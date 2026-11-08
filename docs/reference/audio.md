# @neyuki/audio

Sound playback: sound files, generated tones and raw samples, mixed together with per-voice volume, panning, speed, looping, fades, pausing and seeking. Output goes through the platform audio API (WASAPI on Windows, Core Audio on macOS, ALSA on Linux, which also reaches PulseAudio and PipeWire through their ALSA plugins).

A **sound** is something that can be played: a WAV, FLAC, Ogg Vorbis, MP3 or AAC (`.m4a`, `.mp4`) file (or its bytes), a tone, or a list of samples. Playing it starts a **voice**, and the voice is what you pause, stop, pan, speed up or seek in. One sound can have any number of voices at once, so a footstep loaded once can overlap itself.

Sounds play on their own thread: nothing needs calling every frame, and the script carries on while they play. When the script ends they end too, so `voice:wait()` is how a script lets one finish. The output device opens the first time something needs it. Loading and decoding work on a machine with no sound device, and `audio.isAvailable()` tells you whether playing will.

```lua
local audio = require("@neyuki/audio")

local jump = audio.load("sounds/jump.wav")
local music = audio.load("music/theme.ogg", { stream = true })

music:play({ loop = true, volume = 0.6, fadeIn = 2 })
jump:play({ pan = -0.4, speed = 1.1 })

local beep = audio.tone(880, 0.15, { wave = "square", volume = 0.2 }):play()
beep:wait()
```

**Loaded and streamed sounds.** `audio.load` decodes the whole file up front. That costs memory (about 10 MiB a minute for 44.1 kHz stereo), but any number of voices can play it at once and seeking is instant. Use it for effects. With `stream = true` only the file's bytes are kept, and each voice decodes its own copy as it plays, so a long piece of music takes the size of its file. Seeking a streamed voice opens the file again at the new position, so it costs a little more than a loaded sound's jump, but not more the further in it goes. Decoding is done by [Symphonia](https://github.com/pdeljanov/Symphonia).

The module is behind the `audio` cargo feature, which is on by default. On Linux it links ALSA (`libasound2-dev` on Debian and Ubuntu, `alsa-lib-devel` on Fedora), so `--no-default-features` builds leave it out. In such a build `audio.isAvailable()` is `false` and every other function raises an error.

## audio.isAvailable(): boolean

Whether this build has audio and the output device opens. It is `false` on a build without the `audio` feature and on a machine with no sound device. Opens the device if nothing has yet.

## audio.load(path: string, options: table?): Sound

Loads a WAV, FLAC, Ogg Vorbis, MP3 or AAC file from disk or from a built app's files. With `stream = true` it keeps the encoded bytes and decodes while playing, as described above. An unrecognized or damaged file raises an error naming the path.

## audio.decode(data: buffer | string, options: table?): Sound

Like `load`, from the bytes of a sound file in a buffer or string, such as a download. Takes the same `stream` option.

## audio.tone(frequency: number, seconds: number, options: table?): Sound

A generated mono tone. Options are `wave`, `volume` and `sampleRate`. `wave` is `"sine"` (the default), `"square"`, `"saw"`, `"triangle"` or `"noise"`, which ignores the frequency. `volume` defaults to 0.5 and `sampleRate` to 44100. The tone fades in and out over 2 ms so it starts and ends without a click.

## audio.fromSamples(samples: {number}, options: table?): Sound

A sound made from a list of samples from -1 to 1. Options are `sampleRate` (default 44100) and `channels` (default 1, at most 8). With more than one channel the list is interleaved: left, right, left, right, and so on.

## audio.play(sound: Sound, options: table?): Voice

Starts a voice playing `sound` and returns it. It opens the output device on first use and raises an error when there is none. Options:

- `volume`: 1 by default; 0 is silent, values above 1 are louder (up to 16)
- `pan`: -1 (left) to 1 (right), 0 by default
- `speed`: 1 by default; 2 plays twice as fast and an octave up, 0.5 half as fast and an octave down (above 0, up to 16)
- `loop`: start again from the beginning each time the sound ends
- `fadeIn`: seconds to fade in over
- `start`: seconds into the sound to start from
- `paused`: start paused, for `voice:resume()` later

## audio.volume(): number

The master volume every voice is multiplied by, 1 at first.

## audio.setVolume(volume: number): ()

Sets the master volume (0 to 16). Playing voices follow within a few milliseconds.

## audio.stopAll(fade: number?): ()

Stops every voice, fading out over `fade` seconds (default 0).

## audio.output(): table

The open output device as `{ name, sampleRate, channels }`. Opens the device if nothing has yet, and raises an error when there is none.

## audio.devices(): {string}

The names of the output devices, for `setDevice`. Empty when there are none.

## audio.setDevice(name: string?): ()

Switches output to the device called `name`, or back to the system default when `name` is nil. Every playing voice stops, because it belonged to the old device.

## Sound:play(options: table?): Voice

The same as `audio.play(sound, options)`.

## Sound:close(): ()

Frees the samples. Voices already playing the sound carry on to their end, and playing it again raises an error. A sound also has the fields `duration` (seconds, or `nil` in the rare streamed file whose length cannot be worked out), `channels`, `sampleRate`, `streamed` and `path` (`nil` unless it came from `load`).

## Voice:isPlaying(): boolean

Whether the voice is still sounding. It is false once the voice has ended or been stopped, and while it is paused.

## Voice:isPaused(): boolean

Whether the voice is paused.

## Voice:isDone(): boolean

Whether the voice has ended, been stopped or lost its device. A finished voice cannot be restarted; play the sound again instead.

## Voice:wait(): ()

Waits until the voice is done. Inside a task only that task waits (see [task](task.md)); elsewhere the script blocks. A looping voice never finishes by itself, so waiting on one is an error.

## Voice:pause(): ()

Pauses the voice, keeping its position. The sound glides out over a few milliseconds instead of clicking.

## Voice:resume(): ()

Carries on from where `pause` left off.

## Voice:stop(fade: number?): ()

Stops the voice, fading out over `fade` seconds (default 0, at once).

## Voice:position(): number

Seconds into the sound. A looping voice counts from the start again each time round. Once the voice is done, this is where it stopped.

## Voice:seek(seconds: number): ()

Jumps to `seconds` into the sound. It is instant for a loaded sound. A streamed sound reopens its decoder at that point, on the calling thread.

## Voice:volume(): number

The voice's volume.

## Voice:setVolume(volume: number): ()

Sets the voice's volume (0 to 16). Changes glide over a few milliseconds, so they do not click.

## Voice:pan(): number

The voice's pan, from -1 (left) to 1 (right).

## Voice:setPan(pan: number): ()

Moves the voice between left (-1) and right (1). A stereo sound is balanced: panning right turns the left channel down.

## Voice:speed(): number

The voice's playback rate.

## Voice:setSpeed(speed: number): ()

Changes the playback rate, and the pitch with it.

## Voice:isLooping(): boolean

Whether the voice starts again when it reaches the end.

## Voice:setLoop(looping: boolean): ()

Turns looping on or off. With looping off, the voice ends when it next reaches the end of the sound.
