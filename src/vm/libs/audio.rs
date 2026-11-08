//! Sound playback backing `@neyuki/audio`.
//!
//! `rodio` supplies two things: the output device (through `cpal`: WASAPI on
//! Windows, Core Audio on macOS, ALSA on Linux) with a mixer in front of it,
//! and the decoders (Symphonia: WAV, FLAC, Ogg Vorbis, MP3, AAC). Everything a
//! script controls while a sound plays is done here instead, in one source
//! per voice: resampling to the device rate (which is also how `speed`
//! works), panning, volume, fades, looping, pausing and seeking.
//!
//! A voice emits frames at exactly the mixer's rate and channel count, so the
//! mixer only adds voices together. Scripts talk to a playing voice through
//! atomics in a shared [`Control`], so neither thread ever waits on the
//! other. A null output with no device, mixed only when `__audio_render`
//! asks, lets the tests check exactly what was played.
//!
//! Sounds come in two kinds. A *loaded* sound is decoded to samples up front:
//! any number of voices can play it at once and seeking is instant. A
//! *streamed* sound keeps the encoded bytes and each voice decodes its own
//! copy while it plays, which keeps a long piece of music to the size of its
//! file. Seeking a streamed voice opens a fresh decoder at the new position
//! on the script's thread and hands it over, so the audio thread never
//! parses or seeks a file itself (except to start a looping stream over).

#[cfg(feature = "audio")]
pub(crate) const NATIVES: &[crate::vm::value::NativeDef] = real::NATIVES;

#[cfg(not(feature = "audio"))]
pub(crate) const NATIVES: &[crate::vm::value::NativeDef] = stub::NATIVES;

#[cfg(not(feature = "audio"))]
mod stub {
    use crate::vm::machine::VM;
    use crate::vm::value::{NativeDef, Value};

    const UNSUPPORTED: &str =
        "this build of Neyuki has no audio support (built without the `audio` feature)";

    fn available(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        Ok(vec![Value::Bool(false)])
    }

    fn unsupported(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        Err(UNSUPPORTED.to_string())
    }

    macro_rules! unsupported {
        ($($name:literal),* $(,)?) => {
            &[
                NativeDef { name: "__audio_available", func: available },
                $(NativeDef { name: $name, func: unsupported },)*
            ]
        };
    }

    pub(crate) const NATIVES: &[NativeDef] = unsupported!(
        "__audio_load",
        "__audio_decode",
        "__audio_tone",
        "__audio_samples",
        "__audio_release",
        "__audio_play",
        "__audio_voice_get",
        "__audio_voice_set",
        "__audio_voice_stop",
        "__audio_voice_seek",
        "__audio_volume",
        "__audio_stop_all",
        "__audio_output",
        "__audio_devices",
        "__audio_set_device",
        "__audio_open_null",
        "__audio_render",
    );
}

#[cfg(feature = "audio")]
mod real {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::io::Cursor;
    use std::num::NonZero;
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    use rodio::mixer::{Mixer, MixerSource};
    use rodio::source::SeekError;
    use rodio::{ChannelCount, Decoder, DeviceSinkBuilder, MixerDeviceSink, SampleRate, Source};

    use crate::vm::libs::args::{array, bool_arg, int_arg, number, number_arg, str_arg};
    use crate::vm::machine::VM;
    use crate::vm::value::{NativeDef, Value};

    pub(crate) const NATIVES: &[NativeDef] = &[
        NativeDef {
            name: "__audio_available",
            func: builtin_available,
        },
        NativeDef {
            name: "__audio_load",
            func: builtin_load,
        },
        NativeDef {
            name: "__audio_decode",
            func: builtin_decode,
        },
        NativeDef {
            name: "__audio_tone",
            func: builtin_tone,
        },
        NativeDef {
            name: "__audio_samples",
            func: builtin_samples,
        },
        NativeDef {
            name: "__audio_release",
            func: builtin_release,
        },
        NativeDef {
            name: "__audio_play",
            func: builtin_play,
        },
        NativeDef {
            name: "__audio_voice_get",
            func: builtin_voice_get,
        },
        NativeDef {
            name: "__audio_voice_set",
            func: builtin_voice_set,
        },
        NativeDef {
            name: "__audio_voice_stop",
            func: builtin_voice_stop,
        },
        NativeDef {
            name: "__audio_voice_seek",
            func: builtin_voice_seek,
        },
        NativeDef {
            name: "__audio_volume",
            func: builtin_volume,
        },
        NativeDef {
            name: "__audio_stop_all",
            func: builtin_stop_all,
        },
        NativeDef {
            name: "__audio_output",
            func: builtin_output,
        },
        NativeDef {
            name: "__audio_devices",
            func: builtin_devices,
        },
        NativeDef {
            name: "__audio_set_device",
            func: builtin_set_device,
        },
        NativeDef {
            name: "__audio_open_null",
            func: builtin_open_null,
        },
        NativeDef {
            name: "__audio_render",
            func: builtin_render,
        },
    ];

    /// Channels a sound may have. Beyond 7.1 nothing maps them sensibly.
    const MAX_CHANNELS: usize = 8;

    /// The most samples a loaded sound may decode to (1 GiB of `f32`, about
    /// 50 minutes of 48 kHz stereo). Anything longer should be streamed.
    const MAX_SAMPLES: usize = 1 << 28;

    /// How fast volume, pan and pausing glide to a new value, so a change
    /// does not click: a one-pole filter with about this time constant.
    const SMOOTHING_SECONDS: f32 = 0.005;

    /// Frames between position updates from the audio thread.
    const POSITION_INTERVAL: u32 = 256;

    // SHARED STATE -------------------------------------------------------

    /// An `f32` that the script and audio threads can both reach.
    struct AtomicF32(AtomicU32);

    impl AtomicF32 {
        const fn new(value: f32) -> Self {
            AtomicF32(AtomicU32::new(value.to_bits()))
        }

        fn get(&self) -> f32 {
            f32::from_bits(self.0.load(Ordering::Relaxed))
        }

        fn set(&self, value: f32) {
            self.0.store(value.to_bits(), Ordering::Relaxed);
        }
    }

    /// The volume every voice is multiplied by.
    static MASTER: AtomicF32 = AtomicF32::new(1.0);

    /// What a script can change about a playing voice, and what the audio
    /// thread reports back.
    struct Control {
        volume: AtomicF32,
        pan: AtomicF32,
        speed: AtomicF32,
        paused: AtomicBool,
        looping: AtomicBool,
        /// NaN while playing; the fade-out in seconds once `stop` is asked.
        stop: AtomicF32,
        /// Set by the audio thread when the voice has ended (or was dropped
        /// with its output), and never cleared.
        done: AtomicBool,
        /// Seconds into the sound, as an `f64`'s bits.
        position: AtomicU64,
        /// A jump the audio thread makes on its next frame.
        seek: Mutex<Option<Seek>>,
        has_seek: AtomicBool,
    }

    impl Control {
        fn position(&self) -> f64 {
            f64::from_bits(self.position.load(Ordering::Relaxed))
        }

        fn set_position(&self, seconds: f64) {
            self.position.store(seconds.to_bits(), Ordering::Relaxed);
        }

        fn request_seek(&self, seek: Seek) {
            *self.seek.lock().unwrap_or_else(|e| e.into_inner()) = Some(seek);
            self.has_seek.store(true, Ordering::Release);
        }

        fn request_stop(&self, fade: f32) {
            if self.stop.get().is_nan() || fade < self.stop.get() {
                self.stop.set(fade.max(0.0));
            }
        }
    }

    enum Seek {
        /// A loaded sound jumps straight to a frame.
        Frame(f64),
        /// A streamed sound gets a decoder already wound to its frame.
        Stream(Box<Stream>),
    }

    // SOUNDS -------------------------------------------------------------

    type Bytes = Arc<[u8]>;
    type StreamDecoder = Decoder<Cursor<Bytes>>;

    /// Decoded samples, interleaved.
    struct Pcm {
        samples: Vec<f32>,
        channels: usize,
        rate: u32,
    }

    impl Pcm {
        fn frames(&self) -> usize {
            self.samples.len() / self.channels
        }
    }

    /// The encoded bytes of a streamed sound and what its header said.
    struct Encoded {
        bytes: Bytes,
        channels: usize,
        rate: u32,
        duration: Option<f64>,
    }

    #[derive(Clone)]
    enum Sound {
        Loaded(Arc<Pcm>),
        Streamed(Arc<Encoded>),
    }

    impl Sound {
        fn channels(&self) -> usize {
            match self {
                Sound::Loaded(pcm) => pcm.channels,
                Sound::Streamed(encoded) => encoded.channels,
            }
        }

        fn rate(&self) -> u32 {
            match self {
                Sound::Loaded(pcm) => pcm.rate,
                Sound::Streamed(encoded) => encoded.rate,
            }
        }

        fn duration(&self) -> Option<f64> {
            match self {
                Sound::Loaded(pcm) => Some(pcm.frames() as f64 / pcm.rate as f64),
                Sound::Streamed(encoded) => encoded.duration,
            }
        }

        /// What `__audio_load` and friends return for it.
        fn describe(&self, id: u64) -> Vec<Value> {
            vec![
                Value::Int(id as i64),
                self.duration().map_or(Value::Nil, Value::Float),
                Value::Int(self.channels() as i64),
                Value::Int(self.rate() as i64),
                Value::Bool(matches!(self, Sound::Streamed(_))),
            ]
        }
    }

    fn decoder(bytes: &Bytes) -> Result<StreamDecoder, String> {
        // The length makes the stream seekable and lets Symphonia work out
        // the duration of formats that do not state it (MP3, Vorbis).
        let decoder = Decoder::builder()
            .with_data(Cursor::new(bytes.clone()))
            .with_byte_len(bytes.len() as u64)
            .build()
            .map_err(|err| match err {
                rodio::decoder::DecoderError::UnrecognizedFormat => {
                    "unrecognized audio format (WAV, FLAC, Ogg Vorbis, MP3 and AAC are supported)"
                        .to_string()
                }
                other => format!("cannot decode audio: {}", other),
            })?;
        let channels = decoder.channels().get() as usize;
        if channels > MAX_CHANNELS {
            return Err(format!(
                "sounds with {} channels are not supported (at most {})",
                channels, MAX_CHANNELS
            ));
        }
        Ok(decoder)
    }

    fn decode(bytes: Bytes, stream: bool) -> Result<Sound, String> {
        let decoder = decoder(&bytes)?;
        let channels = decoder.channels().get() as usize;
        let rate = decoder.sample_rate().get();
        if stream {
            let duration = decoder.total_duration().map(|d| d.as_secs_f64());
            return Ok(Sound::Streamed(Arc::new(Encoded {
                bytes,
                channels,
                rate,
                duration,
            })));
        }
        let mut samples = Vec::new();
        for sample in decoder {
            if samples.len() >= MAX_SAMPLES {
                return Err("sound is too long to load; load it with `stream = true`".to_string());
            }
            samples.push(sample);
        }
        samples.truncate(samples.len() - samples.len() % channels);
        Ok(Sound::Loaded(Arc::new(Pcm {
            samples,
            channels,
            rate,
        })))
    }

    thread_local! {
        static SOUNDS: RefCell<HashMap<u64, Sound>> = RefCell::new(HashMap::new());
        static VOICES: RefCell<HashMap<u64, VoiceEntry>> = RefCell::new(HashMap::new());
        static NEXT_ID: Cell<u64> = const { Cell::new(1) };
        static OUTPUT: RefCell<Option<Output>> = const { RefCell::new(None) };
        /// The device `setDevice` chose, or `None` for the system default.
        static DEVICE: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    fn next_id() -> u64 {
        NEXT_ID.with(|next| {
            let id = next.get();
            next.set(id + 1);
            id
        })
    }

    fn register(sound: Sound) -> Vec<Value> {
        let id = next_id();
        let described = sound.describe(id);
        SOUNDS.with(|sounds| sounds.borrow_mut().insert(id, sound));
        described
    }

    fn sound_arg(args: &[Value], index: usize) -> Result<Sound, String> {
        let id = int_arg(args, index, "sound")?;
        SOUNDS
            .with(|sounds| sounds.borrow().get(&(id as u64)).cloned())
            .ok_or_else(|| "sound was released".to_string())
    }

    // OUTPUT -------------------------------------------------------------

    enum Sink {
        /// Only held: dropping it closes the device stream.
        #[allow(dead_code)]
        Device(MixerDeviceSink),
        /// No device: the mix is pulled by `__audio_render` instead, which
        /// is how the tests hear what was played.
        Null(Box<MixerSource>),
    }

    struct Output {
        sink: Sink,
        mixer: Mixer,
        name: String,
        channels: usize,
        rate: u32,
    }

    fn ignore_stream_error(_: rodio::CpalError) {}

    fn device_name(device: &rodio::Device) -> Option<String> {
        device.description().ok().map(|d| d.name().to_string())
    }

    fn open_device(name: Option<&str>) -> Result<Output, String> {
        let host = rodio::cpal::default_host();
        let device = match name {
            None => host
                .default_output_device()
                .ok_or_else(|| "no audio output device".to_string())?,
            Some(name) => host
                .output_devices()
                .map_err(|err| format!("cannot list audio devices: {}", err))?
                .find(|device| device_name(device).as_deref() == Some(name))
                .ok_or_else(|| format!("no audio output device named `{}`", name))?,
        };
        let label = device_name(&device).unwrap_or_else(|| "unknown".to_string());
        let mut sink = DeviceSinkBuilder::from_device(device)
            .map_err(|err| format!("cannot open audio device `{}`: {}", label, err))?
            .with_error_callback(ignore_stream_error as fn(rodio::CpalError))
            .open_sink_or_fallback()
            .map_err(|err| format!("cannot open audio device `{}`: {}", label, err))?;
        sink.log_on_drop(false);
        let config = *sink.config();
        Ok(Output {
            mixer: sink.mixer().clone(),
            sink: Sink::Device(sink),
            name: label,
            channels: config.channel_count().get() as usize,
            rate: config.sample_rate().get(),
        })
    }

    /// Runs `f` with the output, opening the chosen device on first use.
    fn with_output<T>(f: impl FnOnce(&mut Output) -> Result<T, String>) -> Result<T, String> {
        OUTPUT.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                let name = DEVICE.with(|device| device.borrow().clone());
                *slot = Some(open_device(name.as_deref())?);
            }
            f(slot.as_mut().expect("opened above"))
        })
    }

    /// Replaces the output. Every voice of the old one ends with it.
    fn replace_output(output: Option<Output>) {
        let old = OUTPUT.with(|cell| std::mem::replace(&mut *cell.borrow_mut(), output));
        drop(old);
        VOICES.with(|voices| {
            for entry in voices.borrow().values() {
                entry.control.done.store(true, Ordering::Relaxed);
            }
            voices.borrow_mut().clear();
        });
    }

    // VOICES -------------------------------------------------------------

    struct VoiceEntry {
        control: Arc<Control>,
        sound: Sound,
    }

    /// A streamed sound being decoded, one frame ahead of where it plays so
    /// it can be interpolated.
    struct Stream {
        encoded: Arc<Encoded>,
        decoder: StreamDecoder,
        /// The frames at `base` and `base + 1`.
        prev: [f32; MAX_CHANNELS],
        next: [f32; MAX_CHANNELS],
        base: u64,
        /// Set once the decoder has run out: the frames in the sound.
        length: Option<u64>,
        /// The decoder ran out and the voice does not loop.
        ended: bool,
    }

    impl Stream {
        /// A stream positioned at `frame`, decoded up to there.
        fn at(encoded: &Arc<Encoded>, frame: u64) -> Result<Stream, String> {
            let mut stream = Stream {
                encoded: encoded.clone(),
                decoder: decoder(&encoded.bytes)?,
                prev: [0.0; MAX_CHANNELS],
                next: [0.0; MAX_CHANNELS],
                base: 0,
                length: None,
                ended: false,
            };
            let seconds = frame as f64 / encoded.rate as f64;
            let sought = frame == 0
                || stream
                    .decoder
                    .try_seek(Duration::from_secs_f64(seconds))
                    .is_ok();
            // A file the decoder cannot seek in is decoded up to the frame.
            let skip = (frame as usize).saturating_mul(encoded.channels);
            if !sought {
                let skipped = stream.decoder.by_ref().take(skip).count();
                if skipped < skip {
                    stream.ended = true;
                    stream.length = Some((skipped / encoded.channels) as u64);
                }
            }
            stream.base = frame;
            stream.prev = stream.read(frame, false);
            stream.next = stream.read(frame + 1, false);
            Ok(stream)
        }

        /// Frame `index`, the one after the last read; silence once the
        /// sound has ended. With `looping` an exhausted decoder starts over,
        /// and `index` keeps counting up past the end.
        fn read(&mut self, index: u64, looping: bool) -> [f32; MAX_CHANNELS] {
            let channels = self.encoded.channels;
            let mut frame = [0.0; MAX_CHANNELS];
            if self.ended {
                return frame;
            }
            let mut filled = 0;
            let mut restarted = false;
            while filled < channels {
                match self.decoder.next() {
                    Some(sample) => {
                        frame[filled] = sample;
                        filled += 1;
                    }
                    None if looping && !restarted && filled == 0 => {
                        restarted = true;
                        self.length.get_or_insert(index);
                        match decoder(&self.encoded.bytes) {
                            Ok(decoder) => self.decoder = decoder,
                            Err(_) => break,
                        }
                    }
                    None => break,
                }
            }
            if filled < channels {
                self.ended = true;
                self.length.get_or_insert(index);
            }
            frame
        }
    }

    enum Input {
        Loaded(Arc<Pcm>),
        Streamed(Box<Stream>),
    }

    /// One playing sound, as a `rodio` source at the output's own rate and
    /// channel count.
    struct Voice {
        control: Arc<Control>,
        input: Input,
        source_channels: usize,
        source_rate: f64,
        out_channels: usize,
        out_rate: f64,
        /// Position in source frames, fractional.
        pos: f64,
        /// The output frame being handed out sample by sample.
        frame: [f32; MAX_CHANNELS],
        cursor: usize,
        /// Smoothed values, gliding towards what the control says.
        volume: f32,
        pan: f32,
        running: f32,
        smoothing: f32,
        /// The fade envelope: fade-in, then fade-out on `stop`.
        envelope: f32,
        envelope_step: f32,
        stopping: bool,
        since_report: u32,
        finished: bool,
    }

    impl Voice {
        fn report_position(&self) {
            let frames = match &self.input {
                Input::Loaded(_) => self.pos,
                Input::Streamed(stream) => match stream.length {
                    Some(length) if length > 0 => self.pos % length as f64,
                    _ => self.pos,
                },
            };
            self.control.set_position(frames / self.source_rate);
        }

        fn finish(&mut self) {
            self.finished = true;
            self.report_position();
            self.control.done.store(true, Ordering::Release);
        }

        fn apply_seek(&mut self) {
            let seek = self
                .control
                .seek
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            self.control.has_seek.store(false, Ordering::Release);
            match (seek, &mut self.input) {
                (Some(Seek::Frame(frame)), Input::Loaded(_)) => self.pos = frame,
                (Some(Seek::Stream(stream)), Input::Streamed(current)) => {
                    self.pos = stream.base as f64;
                    *current = stream;
                }
                _ => {}
            }
        }

        /// The source frame at the current position, interpolated, or
        /// `None` once the sound is over.
        fn sample_frame(&mut self, looping: bool, out: &mut [f32; MAX_CHANNELS]) -> bool {
            let channels = self.source_channels;
            match &mut self.input {
                Input::Loaded(pcm) => {
                    let frames = pcm.frames();
                    if frames == 0 {
                        return false;
                    }
                    if self.pos >= frames as f64 {
                        if !looping {
                            return false;
                        }
                        self.pos %= frames as f64;
                    }
                    let index = self.pos as usize;
                    let t = (self.pos - index as f64) as f32;
                    let next = if index + 1 < frames {
                        Some(index + 1)
                    } else if looping {
                        Some(0)
                    } else {
                        None
                    };
                    let a = &pcm.samples[index * channels..index * channels + channels];
                    for c in 0..channels {
                        let b = next.map_or(0.0, |n| pcm.samples[n * channels + c]);
                        out[c] = a[c] + (b - a[c]) * t;
                    }
                    true
                }
                Input::Streamed(stream) => {
                    let target = self.pos as u64;
                    while stream.base < target {
                        stream.prev = stream.next;
                        stream.next = stream.read(stream.base + 2, looping);
                        stream.base += 1;
                    }
                    if stream.ended && stream.length.is_some_and(|length| stream.base >= length) {
                        return false;
                    }
                    let t = (self.pos - stream.base as f64) as f32;
                    for ((o, a), b) in out
                        .iter_mut()
                        .zip(&stream.prev)
                        .zip(&stream.next)
                        .take(channels)
                    {
                        *o = a + (b - a) * t;
                    }
                    true
                }
            }
        }

        /// Works out the next output frame into `self.frame`.
        fn render_frame(&mut self) -> bool {
            if self.control.has_seek.load(Ordering::Acquire) {
                self.apply_seek();
            }
            let control = &*self.control;
            let stop = control.stop.get();
            let paused = control.paused.load(Ordering::Relaxed);
            let target_volume = control.volume.get() * MASTER.get();
            let target_pan = control.pan.get();
            let looping = control.looping.load(Ordering::Relaxed);
            let speed = control.speed.get() as f64;

            if !stop.is_nan() && !self.stopping {
                self.stopping = true;
                if stop <= 0.0 || self.envelope <= 0.0 {
                    return false;
                }
                self.envelope_step = -self.envelope / (stop * self.out_rate as f32);
            }

            let k = self.smoothing;
            self.running += ((if paused { 0.0 } else { 1.0 }) - self.running) * k;
            if paused && self.running < 1e-4 {
                self.running = 0.0;
                self.frame = [0.0; MAX_CHANNELS];
                return true;
            }
            self.volume += (target_volume - self.volume) * k;
            self.pan += (target_pan - self.pan) * k;

            let mut source = [0.0; MAX_CHANNELS];
            if !self.sample_frame(looping, &mut source) {
                return false;
            }

            self.envelope = (self.envelope + self.envelope_step).clamp(0.0, 1.0);
            if self.stopping && self.envelope <= 0.0 {
                return false;
            }
            let gain = self.volume * self.envelope * self.running;
            let left = (1.0 - self.pan).min(1.0) * gain;
            let right = (1.0 + self.pan).min(1.0) * gain;
            spread(
                &source[..self.source_channels],
                &mut self.frame[..self.out_channels],
                left,
                right,
                gain,
            );

            self.pos += speed * self.source_rate / self.out_rate;
            self.since_report += 1;
            if self.since_report >= POSITION_INTERVAL {
                self.since_report = 0;
                self.report_position();
            }
            true
        }
    }

    /// Maps a source frame onto the output's channels: mono and stereo are
    /// panned onto the first two, further channels pass through, and a mono
    /// output gets the average of left and right.
    fn spread(source: &[f32], out: &mut [f32], left: f32, right: f32, gain: f32) {
        out.fill(0.0);
        match (source.len(), out.len()) {
            (_, 1) => {
                let mono = if source.len() == 1 {
                    source[0]
                } else {
                    (source[0] + source[1]) * 0.5
                };
                out[0] = mono * gain;
            }
            (1, _) => {
                out[0] = source[0] * left;
                out[1] = source[0] * right;
            }
            (n, m) => {
                out[0] = source[0] * left;
                out[1] = source[1] * right;
                for c in 2..n.min(m) {
                    out[c] = source[c] * gain;
                }
            }
        }
    }

    impl Drop for Voice {
        fn drop(&mut self) {
            self.control.done.store(true, Ordering::Release);
        }
    }

    impl Iterator for Voice {
        type Item = f32;

        fn next(&mut self) -> Option<f32> {
            if self.finished {
                return None;
            }
            if self.cursor == 0 && !self.render_frame() {
                self.finish();
                return None;
            }
            let sample = self.frame[self.cursor];
            self.cursor = (self.cursor + 1) % self.out_channels;
            Some(sample)
        }
    }

    impl Source for Voice {
        fn current_span_len(&self) -> Option<usize> {
            None
        }

        fn channels(&self) -> ChannelCount {
            NonZero::new(self.out_channels as u16).expect("output has channels")
        }

        fn sample_rate(&self) -> SampleRate {
            NonZero::new(self.out_rate as u32).expect("output has a rate")
        }

        fn total_duration(&self) -> Option<Duration> {
            None
        }

        fn try_seek(&mut self, _: Duration) -> Result<(), SeekError> {
            Err(SeekError::NotSupported {
                underlying_source: "neyuki audio voice",
            })
        }
    }

    fn sweep_finished_voices() {
        VOICES.with(|voices| {
            voices
                .borrow_mut()
                .retain(|_, entry| !entry.control.done.load(Ordering::Acquire))
        });
    }

    fn voice(id: i64) -> Option<(Arc<Control>, Sound)> {
        VOICES.with(|voices| {
            voices
                .borrow()
                .get(&(id as u64))
                .map(|entry| (entry.control.clone(), entry.sound.clone()))
        })
    }

    /// Where `seconds` lands in a sound, as a source frame.
    fn seek_for(sound: &Sound, seconds: f64, looping: bool) -> Result<Seek, String> {
        let frame = (seconds * sound.rate() as f64).max(0.0);
        match sound {
            Sound::Loaded(pcm) => {
                let frames = pcm.frames() as f64;
                let frame = if looping && frames > 0.0 {
                    frame % frames
                } else {
                    frame.min(frames)
                };
                Ok(Seek::Frame(frame))
            }
            Sound::Streamed(encoded) => {
                let mut frame = frame as u64;
                if let (true, Some(duration)) = (looping, encoded.duration) {
                    let length = (duration * encoded.rate as f64) as u64;
                    if length > 0 {
                        frame %= length;
                    }
                }
                Ok(Seek::Stream(Box::new(Stream::at(encoded, frame)?)))
            }
        }
    }

    // NATIVES ------------------------------------------------------------

    fn builtin_available(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        Ok(vec![Value::Bool(with_output(|_| Ok(())).is_ok())])
    }

    fn read_file(path: &str) -> Result<Bytes, String> {
        crate::app::read(path)
            .unwrap_or_else(|| std::fs::read(path))
            .map(Bytes::from)
            .map_err(|err| format!("cannot open {}: {}", path, err))
    }

    /// `__audio_load(path, stream)`: id, duration, channels, rate, streamed.
    fn builtin_load(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let path = str_arg(args, 0, "path")?;
        let stream = bool_arg(args, 1, "stream")?;
        let sound = decode(read_file(path)?, stream).map_err(|err| format!("{}: {}", path, err))?;
        Ok(register(sound))
    }

    /// `__audio_decode(data, stream)`, from a buffer or string.
    fn builtin_decode(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let bytes: Bytes = match args.first() {
            Some(Value::Buffer(buffer)) => Bytes::from(buffer.borrow().data.as_slice()),
            Some(Value::String(s)) => Bytes::from(s.0.str.as_bytes()),
            None | Some(Value::Nil) => return Err("data must be provided".to_string()),
            Some(other) => {
                return Err(format!(
                    "data must be a buffer or string, got {}",
                    other.type_name()
                ));
            }
        };
        let stream = bool_arg(args, 1, "stream")?;
        Ok(register(decode(bytes, stream)?))
    }

    fn rate_arg(args: &[Value], index: usize) -> Result<u32, String> {
        let rate = int_arg(args, index, "sampleRate")?;
        if !(1..=384_000).contains(&rate) {
            return Err("sampleRate must be between 1 and 384000".to_string());
        }
        Ok(rate as u32)
    }

    /// `__audio_tone(wave, frequency, seconds, rate, volume)`.
    fn builtin_tone(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let wave = str_arg(args, 0, "wave")?;
        let frequency = number_arg(args, 1, "frequency")?;
        let seconds = number_arg(args, 2, "seconds")?;
        let rate = rate_arg(args, 3)?;
        let volume = number_arg(args, 4, "volume")? as f32;
        let frames = (seconds * rate as f64).round();
        if !(0.0..=MAX_SAMPLES as f64).contains(&frames) {
            return Err("seconds is out of range".to_string());
        }
        let frames = frames as usize;
        // A couple of milliseconds of ramp at each end, so the tone starts
        // and stops without a click.
        let ramp = ((rate as f64 * 0.002) as usize).clamp(1, (frames / 2).max(1));
        let mut seed: u32 = 0x9e37_79b9;
        let mut samples = Vec::with_capacity(frames);
        for i in 0..frames {
            let phase = (i as f64 * frequency / rate as f64).fract() as f32;
            let value = match wave {
                "sine" => (phase * std::f32::consts::TAU).sin(),
                "square" => {
                    if phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                "saw" => 2.0 * phase - 1.0,
                "triangle" => 1.0 - 4.0 * (phase - 0.5).abs(),
                "noise" => {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
                }
                other => return Err(format!("unknown wave `{}`", other)),
            };
            let edge = (i.min(frames - 1 - i) as f32 / ramp as f32).min(1.0);
            samples.push(value * volume * edge);
        }
        Ok(register(Sound::Loaded(Arc::new(Pcm {
            samples,
            channels: 1,
            rate,
        }))))
    }

    /// `__audio_samples(list, rate, channels)`: a sound from numbers.
    fn builtin_samples(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let list = match args.first() {
            Some(Value::Table(t)) => t.clone(),
            None | Some(Value::Nil) => return Err("samples must be provided".to_string()),
            Some(other) => {
                return Err(format!(
                    "samples must be a list of numbers, got {}",
                    other.type_name()
                ));
            }
        };
        let rate = rate_arg(args, 1)?;
        let channels = int_arg(args, 2, "channels")?;
        if !(1..=MAX_CHANNELS as i64).contains(&channels) {
            return Err(format!("channels must be between 1 and {}", MAX_CHANNELS));
        }
        let channels = channels as usize;
        let list = list.borrow();
        if list.array.len() > MAX_SAMPLES {
            return Err("too many samples".to_string());
        }
        if list.array.len() % channels != 0 {
            return Err(format!(
                "the number of samples must be a multiple of channels ({})",
                channels
            ));
        }
        let samples = list
            .array
            .iter()
            .enumerate()
            .map(|(i, value)| {
                number(value)
                    .map(|n| n as f32)
                    .map_err(|_| format!("samples[{}] must be a number", i + 1))
            })
            .collect::<Result<Vec<f32>, String>>()?;
        Ok(register(Sound::Loaded(Arc::new(Pcm {
            samples,
            channels,
            rate,
        }))))
    }

    fn builtin_release(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = int_arg(args, 0, "sound")?;
        SOUNDS.with(|sounds| sounds.borrow_mut().remove(&(id as u64)));
        Ok(vec![])
    }

    fn float_arg(args: &[Value], index: usize, name: &str) -> Result<f32, String> {
        number_arg(args, index, name).map(|n| n as f32)
    }

    /// `__audio_play(sound, volume, pan, speed, loop, fadeIn, start, paused)`.
    fn builtin_play(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let sound = sound_arg(args, 0)?;
        let volume = float_arg(args, 1, "volume")?;
        let pan = float_arg(args, 2, "pan")?;
        let speed = float_arg(args, 3, "speed")?;
        let looping = bool_arg(args, 4, "loop")?;
        let fade_in = float_arg(args, 5, "fadeIn")?;
        let start = number_arg(args, 6, "start")?;
        let paused = bool_arg(args, 7, "paused")?;

        sweep_finished_voices();
        let (pos, input) = match seek_for(&sound, start, looping)? {
            Seek::Frame(frame) => match &sound {
                Sound::Loaded(pcm) => (frame, Input::Loaded(pcm.clone())),
                Sound::Streamed(_) => unreachable!("a streamed sound seeks by decoder"),
            },
            Seek::Stream(stream) => (stream.base as f64, Input::Streamed(stream)),
        };
        let control = Arc::new(Control {
            volume: AtomicF32::new(volume),
            pan: AtomicF32::new(pan),
            speed: AtomicF32::new(speed),
            paused: AtomicBool::new(paused),
            looping: AtomicBool::new(looping),
            stop: AtomicF32::new(f32::NAN),
            done: AtomicBool::new(false),
            position: AtomicU64::new((pos / sound.rate() as f64).to_bits()),
            seek: Mutex::new(None),
            has_seek: AtomicBool::new(false),
        });
        with_output(|output| {
            let out_rate = output.rate as f64;
            let voice = Voice {
                control: control.clone(),
                input,
                source_channels: sound.channels(),
                source_rate: sound.rate() as f64,
                out_channels: output.channels,
                out_rate,
                pos,
                frame: [0.0; MAX_CHANNELS],
                cursor: 0,
                volume: volume * MASTER.get(),
                pan,
                running: if paused { 0.0 } else { 1.0 },
                smoothing: 1.0 - (-1.0 / (SMOOTHING_SECONDS * out_rate as f32)).exp(),
                envelope: if fade_in > 0.0 { 0.0 } else { 1.0 },
                envelope_step: if fade_in > 0.0 {
                    1.0 / (fade_in * out_rate as f32)
                } else {
                    0.0
                },
                stopping: false,
                since_report: 0,
                finished: false,
            };
            output.mixer.add(voice);
            Ok(())
        })?;
        let id = next_id();
        VOICES.with(|voices| {
            voices
                .borrow_mut()
                .insert(id, VoiceEntry { control, sound })
        });
        Ok(vec![Value::Int(id as i64)])
    }

    /// `__audio_voice_get(voice)`: done, paused, position, volume, pan,
    /// speed, loop. A voice that is gone reports done.
    fn builtin_voice_get(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = int_arg(args, 0, "voice")?;
        let Some((control, _)) = voice(id) else {
            return Ok(vec![Value::Bool(true)]);
        };
        Ok(vec![
            Value::Bool(control.done.load(Ordering::Acquire)),
            Value::Bool(control.paused.load(Ordering::Relaxed)),
            Value::Float(control.position()),
            Value::Float(control.volume.get() as f64),
            Value::Float(control.pan.get() as f64),
            Value::Float(control.speed.get() as f64),
            Value::Bool(control.looping.load(Ordering::Relaxed)),
        ])
    }

    /// `__audio_voice_set(voice, field, value)`.
    fn builtin_voice_set(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = int_arg(args, 0, "voice")?;
        let field = str_arg(args, 1, "field")?;
        let Some((control, _)) = voice(id) else {
            return Ok(vec![]);
        };
        match field {
            "volume" => control.volume.set(float_arg(args, 2, "volume")?),
            "pan" => control.pan.set(float_arg(args, 2, "pan")?),
            "speed" => control.speed.set(float_arg(args, 2, "speed")?),
            "loop" => control
                .looping
                .store(bool_arg(args, 2, "loop")?, Ordering::Relaxed),
            "paused" => control
                .paused
                .store(bool_arg(args, 2, "paused")?, Ordering::Relaxed),
            other => return Err(format!("unknown voice field `{}`", other)),
        }
        Ok(vec![])
    }

    /// `__audio_voice_stop(voice, fade)`.
    fn builtin_voice_stop(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = int_arg(args, 0, "voice")?;
        let fade = float_arg(args, 1, "fade")?;
        if let Some((control, _)) = voice(id) {
            control.request_stop(fade);
        }
        Ok(vec![])
    }

    /// `__audio_voice_seek(voice, seconds)`.
    fn builtin_voice_seek(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let id = int_arg(args, 0, "voice")?;
        let seconds = number_arg(args, 1, "seconds")?;
        let Some((control, sound)) = voice(id) else {
            return Ok(vec![]);
        };
        let seek = seek_for(&sound, seconds, control.looping.load(Ordering::Relaxed))?;
        let frame = match &seek {
            Seek::Frame(frame) => *frame,
            Seek::Stream(stream) => stream.base as f64,
        };
        control.request_seek(seek);
        control.set_position(frame / sound.rate() as f64);
        Ok(vec![])
    }

    /// `__audio_volume(volume?)`: sets the master volume when given, and
    /// returns it.
    fn builtin_volume(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        if !matches!(args.first(), None | Some(Value::Nil)) {
            MASTER.set(float_arg(args, 0, "volume")?);
        }
        Ok(vec![Value::Float(MASTER.get() as f64)])
    }

    /// `__audio_stop_all(fade)`.
    fn builtin_stop_all(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let fade = float_arg(args, 0, "fade")?;
        VOICES.with(|voices| {
            for entry in voices.borrow().values() {
                entry.control.request_stop(fade);
            }
        });
        Ok(vec![])
    }

    /// `__audio_output()`: name, rate, channels of the open output.
    fn builtin_output(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        with_output(|output| {
            Ok(vec![
                Value::str(&output.name),
                Value::Int(output.rate as i64),
                Value::Int(output.channels as i64),
            ])
        })
    }

    fn builtin_devices(_vm: &mut VM, _args: &[Value]) -> Result<Vec<Value>, String> {
        let host = rodio::cpal::default_host();
        let mut names = Vec::new();
        if let Ok(devices) = host.output_devices() {
            for device in devices {
                if let Some(name) = device_name(&device).filter(|name| !names.contains(name)) {
                    names.push(name);
                }
            }
        }
        Ok(vec![array(
            names.iter().map(|name| Value::str(name)).collect(),
        )])
    }

    /// `__audio_set_device(name?)`: switches to a device by name, or back to
    /// the system default. Playing voices stop.
    fn builtin_set_device(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let name = match args.first() {
            None | Some(Value::Nil) => None,
            _ => Some(str_arg(args, 0, "name")?.to_string()),
        };
        let output = open_device(name.as_deref())?;
        DEVICE.with(|device| *device.borrow_mut() = name);
        replace_output(Some(output));
        Ok(vec![])
    }

    /// `__audio_open_null(rate, channels)`: an output with no device, mixed
    /// only when `__audio_render` asks. For tests.
    fn builtin_open_null(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let rate = rate_arg(args, 0)?;
        let channels = int_arg(args, 1, "channels")?;
        if !(1..=MAX_CHANNELS as i64).contains(&channels) {
            return Err(format!("channels must be between 1 and {}", MAX_CHANNELS));
        }
        let (mixer, source) = rodio::mixer::mixer(
            NonZero::new(channels as u16).expect("checked above"),
            NonZero::new(rate).expect("checked above"),
        );
        replace_output(Some(Output {
            sink: Sink::Null(Box::new(source)),
            mixer,
            name: "null".to_string(),
            channels: channels as usize,
            rate,
        }));
        Ok(vec![])
    }

    /// `__audio_render(frames)`: pulls that many frames from the null
    /// output, interleaved.
    fn builtin_render(_vm: &mut VM, args: &[Value]) -> Result<Vec<Value>, String> {
        let frames = int_arg(args, 0, "frames")?;
        if !(0..=1 << 22).contains(&frames) {
            return Err("frames is out of range".to_string());
        }
        OUTPUT.with(|cell| {
            let mut slot = cell.borrow_mut();
            let Some(Output {
                sink: Sink::Null(source),
                channels,
                ..
            }) = slot.as_mut()
            else {
                return Err("render needs the null output".to_string());
            };
            let count = frames as usize * *channels;
            let samples = (0..count)
                .map(|_| Value::Float(source.next().unwrap_or(0.0) as f64))
                .collect();
            Ok(vec![array(samples)])
        })
    }
}
