//! Image decoding, resampling and placement for `@neyuki/ui`.
//!
//! Everything here is plain data in, plain data out: [`decode`] turns the
//! bytes of a PNG, JPEG, WebP, GIF or BMP file into premultiplied frames
//! without touching the interpreter or the thread-locals in `ui.rs`, so it
//! can later run off the interpreter thread. The format comes from the
//! file's magic bytes, never from its name.
//!
//! Animated GIF, WebP and PNG files are decoded up front into full-canvas
//! frames (the decoders composite and dispose for us), which costs memory but
//! makes drawing any frame as cheap as drawing a still. [`Limits`] bounds that
//! memory before and during decoding.
//!
//! Drawing only needs a source [`Pixmap`] per frame, so an off-screen canvas
//! can reuse [`place`], [`resample`] and [`draw`] unchanged.

use std::io::Cursor;

use image::codecs::bmp::BmpDecoder;
use image::codecs::gif::GifDecoder;
use image::codecs::jpeg::JpegDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::metadata::{LoopCount, Orientation};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat};
use tiny_skia::{
    ColorU8, FillRule, FilterQuality, IntSize, Paint, Pattern, Pixmap, PixmapPaint, PixmapRef,
    SpreadMode, Transform,
};

/// The largest image accepted unless `maxPixels` says otherwise: 64
/// megapixels, comfortably above an 8K frame.
pub(super) const DEFAULT_MAX_PIXELS: u64 = 64_000_000;

/// The most memory the decoded frames of one image may take unless
/// `maxBytes` says otherwise. A long animated GIF at full HD reaches this
/// after about 60 frames.
pub(super) const DEFAULT_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Frame delays at or below this many seconds are shown for
/// [`DEFAULT_FRAME_DELAY`] instead, as browsers do: many GIFs in the wild say
/// 0 or 10 ms and were made expecting that.
const MIN_FRAME_DELAY: f64 = 0.010;
const DEFAULT_FRAME_DELAY: f64 = 0.100;

/// Upper bounds on what one image may decode to.
#[derive(Clone, Copy, Debug)]
pub(super) struct Limits {
    pub max_pixels: u64,
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_pixels: DEFAULT_MAX_PIXELS,
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// A decoded image: every frame at the full canvas size, premultiplied and
/// ready to draw. Stills have one frame and a zero delay.
pub(super) struct Decoded {
    pub frames: Vec<Pixmap>,
    /// Seconds each frame stays on screen, already clamped.
    pub delays: Vec<f64>,
    /// How many times the animation plays; 0 is forever.
    pub loop_count: u32,
}

impl std::fmt::Debug for Decoded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoded")
            .field("frames", &self.frames.len())
            .field("delays", &self.delays)
            .field("loop_count", &self.loop_count)
            .finish()
    }
}

/// Decodes a PNG, JPEG, WebP, GIF or BMP image, picking the format from the
/// leading bytes. Errors are bare reasons; the caller names the source.
pub(super) fn decode(bytes: &[u8], limits: Limits) -> Result<Decoded, String> {
    let format = image::guess_format(bytes).map_err(|_| {
        "unrecognized image format (expected PNG, JPEG, WebP, GIF or BMP)".to_string()
    })?;
    let cursor = Cursor::new(bytes);
    match format {
        ImageFormat::Png => {
            let mut decoder = PngDecoder::new(cursor).map_err(describe(limits))?;
            if decoder.is_apng().map_err(describe(limits))? {
                let (width, height) = decoder.dimensions();
                check_size(width, height, limits)?;
                decoder
                    .set_limits(image_limits(limits))
                    .map_err(describe(limits))?;
                decode_animation(
                    width,
                    height,
                    decoder.apng().map_err(describe(limits))?,
                    limits,
                )
            } else {
                decode_still(decoder, limits)
            }
        }
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(cursor).map_err(describe(limits))?;
            let (width, height) = decoder.dimensions();
            check_size(width, height, limits)?;
            decoder
                .set_limits(image_limits(limits))
                .map_err(describe(limits))?;
            decode_animation(width, height, decoder, limits)
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(cursor).map_err(describe(limits))?;
            if decoder.has_animation() {
                let (width, height) = decoder.dimensions();
                check_size(width, height, limits)?;
                decoder
                    .set_limits(image_limits(limits))
                    .map_err(describe(limits))?;
                decode_animation(width, height, decoder, limits)
            } else {
                decode_still(decoder, limits)
            }
        }
        ImageFormat::Jpeg => {
            decode_still(JpegDecoder::new(cursor).map_err(describe(limits))?, limits)
        }
        ImageFormat::Bmp => {
            decode_still(BmpDecoder::new(cursor).map_err(describe(limits))?, limits)
        }
        other => Err(format!(
            "unsupported image format {} (expected PNG, JPEG, WebP, GIF or BMP)",
            other
                .extensions_str()
                .first()
                .unwrap_or(&"?")
                .to_uppercase()
        )),
    }
}

/// Turns a decoder error into a message, naming the limit when the decoder
/// stopped at it.
fn describe(limits: Limits) -> impl Fn(image::ImageError) -> String {
    move |err| match err {
        image::ImageError::Limits(_) => too_many_bytes(limits),
        other => other.to_string(),
    }
}

fn image_limits(limits: Limits) -> image::Limits {
    let mut result = image::Limits::default();
    result.max_alloc = Some(limits.max_bytes);
    result
}

/// Rejects an image whose canvas alone breaks a limit, before anything is
/// allocated for it. Returns the bytes one frame takes.
pub(super) fn check_size(width: u32, height: u32, limits: Limits) -> Result<u64, String> {
    if width == 0 || height == 0 {
        return Err("image is empty".to_string());
    }
    let pixels = width as u64 * height as u64;
    if pixels > limits.max_pixels {
        return Err(format!(
            "image is {}x{} ({} pixels), over the limit of {} pixels (maxPixels)",
            width, height, pixels, limits.max_pixels
        ));
    }
    let bytes = pixels * 4;
    if bytes > limits.max_bytes {
        return Err(too_many_bytes(limits));
    }
    Ok(bytes)
}

fn too_many_bytes(limits: Limits) -> String {
    format!(
        "decoded image needs more than {} bytes (maxBytes)",
        limits.max_bytes
    )
}

fn decode_still(mut decoder: impl ImageDecoder, limits: Limits) -> Result<Decoded, String> {
    let (width, height) = decoder.dimensions();
    check_size(width, height, limits)?;
    decoder
        .set_limits(image_limits(limits))
        .map_err(describe(limits))?;
    // Photos straight from a phone are often stored sideways with an EXIF
    // note saying which way is up.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(describe(limits))?;
    image.apply_orientation(orientation);
    let rgba = image.into_rgba8();
    let (width, height) = rgba.dimensions();
    let frame = rgba_to_pixmap(rgba.into_raw(), width, height)
        .ok_or_else(|| "image is too large to decode".to_string())?;
    Ok(Decoded {
        frames: vec![frame],
        delays: vec![0.0],
        loop_count: 0,
    })
}

fn decode_animation<'a>(
    width: u32,
    height: u32,
    decoder: impl AnimationDecoder<'a>,
    limits: Limits,
) -> Result<Decoded, String> {
    let frame_bytes = check_size(width, height, limits)?;
    let loop_count = match decoder.loop_count() {
        LoopCount::Infinite => 0,
        LoopCount::Finite(n) => n.get(),
    };
    let mut frames = Vec::new();
    let mut delays = Vec::new();
    let mut total = 0u64;
    for frame in decoder.into_frames() {
        let frame = frame.map_err(describe(limits))?;
        total += frame_bytes;
        if total > limits.max_bytes {
            return Err(too_many_bytes(limits));
        }
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        delays.push(frame_delay(numerator as f64 / denominator.max(1) as f64));
        let buffer = frame.into_buffer();
        if buffer.dimensions() != (width, height) {
            return Err("animation frame does not match the canvas size".to_string());
        }
        frames.push(
            rgba_to_pixmap(buffer.into_raw(), width, height)
                .ok_or_else(|| "image is too large to decode".to_string())?,
        );
    }
    if frames.is_empty() {
        return Err("image has no frames".to_string());
    }
    Ok(Decoded {
        frames,
        delays,
        loop_count,
    })
}

/// Seconds a frame with a delay of `ms` milliseconds is shown for.
pub(super) fn frame_delay(ms: f64) -> f64 {
    let seconds = ms / 1000.0;
    if !seconds.is_finite() || seconds <= MIN_FRAME_DELAY {
        DEFAULT_FRAME_DELAY
    } else {
        seconds
    }
}

/// Wraps straight RGBA bytes as a pixmap, premultiplying in place.
pub(super) fn rgba_to_pixmap(mut rgba: Vec<u8>, width: u32, height: u32) -> Option<Pixmap> {
    if rgba.len() as u64 != width as u64 * height as u64 * 4 {
        return None;
    }
    let (pixels, _) = rgba.as_chunks_mut::<4>();
    for pixel in pixels {
        if pixel[3] != 255 {
            let c = ColorU8::from_rgba(pixel[0], pixel[1], pixel[2], pixel[3]).premultiply();
            pixel.copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
    }
    Pixmap::from_vec(rgba, IntSize::from_wh(width, height)?)
}

pub(super) fn is_opaque(pixmap: &Pixmap) -> bool {
    pixmap.pixels().iter().all(|pixel| pixel.alpha() == 255)
}

// PLACEMENT ----------------------------------------------------------------

/// How an image is fitted into the box it is drawn in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Fit {
    /// Stretch to the box.
    Fill,
    /// Scale to fit inside the box, centered, keeping the aspect ratio.
    Contain,
    /// Scale to cover the box, keeping the aspect ratio and cropping the
    /// overflow equally on both sides.
    Cover,
}

impl Fit {
    pub fn parse(name: &str) -> Result<Fit, String> {
        match name {
            "fill" => Ok(Fit::Fill),
            "contain" => Ok(Fit::Contain),
            "cover" => Ok(Fit::Cover),
            _ => Err(format!(
                "fit must be \"fill\", \"contain\" or \"cover\", got \"{}\"",
                name
            )),
        }
    }
}

/// A rectangle as `(x, y, width, height)`.
pub(super) type RectF = (f32, f32, f32, f32);

/// Which part of an `image_w` × `image_h` image is shown, and where, when it
/// is drawn into the box `(x, y, box_w, box_h)`. Returns the source rectangle
/// in image pixels and the destination in the box's units.
pub(super) fn place(image_w: f32, image_h: f32, target: RectF, fit: Fit) -> (RectF, RectF) {
    let (x, y, box_w, box_h) = target;
    let full = (0.0, 0.0, image_w, image_h);
    match fit {
        Fit::Fill => (full, target),
        Fit::Contain => {
            let s = (box_w / image_w).min(box_h / image_h);
            let (w, h) = (image_w * s, image_h * s);
            (full, (x + (box_w - w) / 2.0, y + (box_h - h) / 2.0, w, h))
        }
        Fit::Cover => {
            let s = (box_w / image_w).max(box_h / image_h);
            let (w, h) = (box_w / s, box_h / s);
            (((image_w - w) / 2.0, (image_h - h) / 2.0, w, h), target)
        }
    }
}

/// Rounds a destination rectangle (in physical pixels) to whole pixels, so
/// a prepared copy can be blitted without filtering and neighbouring images
/// meet without seams. Returns `(left, top, width, height)`.
pub(super) fn snap(dest: RectF) -> (i32, i32, u32, u32) {
    let clamp = |v: f32| v.round().clamp(-(1 << 24) as f32, (1 << 24) as f32) as i64;
    let (left, top) = (clamp(dest.0), clamp(dest.1));
    let (right, bottom) = (clamp(dest.0 + dest.2), clamp(dest.1 + dest.3));
    (
        left as i32,
        top as i32,
        (right - left).max(0) as u32,
        (bottom - top).max(0) as u32,
    )
}

// RESAMPLING ---------------------------------------------------------------

/// Filter taps for one output pixel: the first source index and a weight
/// for it and each index after it.
type Taps = (usize, Vec<f32>);

/// The weights of a triangle (bilinear) filter mapping `len` source pixels
/// starting at `start` onto `out_len` output pixels. When shrinking, the
/// filter widens to cover every source pixel that falls in an output pixel,
/// so downscaling averages instead of skipping (no aliasing or shimmer).
fn filter_taps(src_len: u32, start: f32, len: f32, out_len: u32) -> Vec<Taps> {
    let scale = len / out_len as f32;
    let support = scale.max(1.0);
    let lo = (start.floor().max(0.0) as usize).min(src_len.max(1) as usize - 1);
    let hi = ((start + len).ceil() as usize)
        .min(src_len as usize)
        .max(lo + 1);
    (0..out_len)
        .map(|i| {
            let center = start + (i as f32 + 0.5) * scale;
            let first = ((center - support).floor().max(lo as f32) as usize).min(hi - 1);
            let last = ((center + support).ceil() as usize).clamp(first + 1, hi);
            let mut weights: Vec<f32> = (first..last)
                .map(|j| (1.0 - ((j as f32 + 0.5 - center) / support).abs()).max(0.0))
                .collect();
            let sum: f32 = weights.iter().sum();
            if sum > 0.0 {
                weights.iter_mut().for_each(|w| *w /= sum);
                (first, weights)
            } else {
                // Only reachable for degenerate crops; take the nearest.
                let nearest = (center.floor().max(0.0) as usize).clamp(lo, hi - 1);
                (nearest, vec![1.0])
            }
        })
        .collect()
}

/// Resamples the `src` rectangle of `source` (fractional coordinates
/// allowed) to `width` × `height` pixels with a separable triangle filter.
/// Pixels stay premultiplied throughout, which keeps transparent edges
/// from bleeding dark fringes.
pub(super) fn resample(source: &Pixmap, src: RectF, width: u32, height: u32) -> Option<Pixmap> {
    if width == 0 || height == 0 || src.2 <= 0.0 || src.3 <= 0.0 {
        return None;
    }
    let columns = filter_taps(source.width(), src.0, src.2, width);
    let rows = filter_taps(source.height(), src.1, src.3, height);
    let first_row = rows.iter().map(|(first, _)| *first).min()?;
    let last_row = rows.iter().map(|(first, w)| first + w.len()).max()?;
    let stride = source.width() as usize * 4;
    let data = source.data();

    // Horizontal pass over just the source rows the vertical pass reads.
    let out_stride = width as usize * 4;
    let mut middle = vec![0f32; (last_row - first_row) * out_stride];
    for (r, row) in (first_row..last_row).enumerate() {
        let line = &data[row * stride..(row + 1) * stride];
        let target = &mut middle[r * out_stride..(r + 1) * out_stride];
        for (x, (first, weights)) in columns.iter().enumerate() {
            let mut acc = [0f32; 4];
            for (k, w) in weights.iter().enumerate() {
                let p = &line[(first + k) * 4..(first + k) * 4 + 4];
                for (a, v) in acc.iter_mut().zip(p) {
                    *a += *v as f32 * w;
                }
            }
            target[x * 4..x * 4 + 4].copy_from_slice(&acc);
        }
    }

    // Vertical pass into the output.
    let mut out = Pixmap::new(width, height)?;
    let out_data = out.data_mut();
    for (y, (first, weights)) in rows.iter().enumerate() {
        let target = &mut out_data[y * out_stride..(y + 1) * out_stride];
        for x in 0..width as usize {
            let mut acc = [0f32; 4];
            for (k, w) in weights.iter().enumerate() {
                let base = (first + k - first_row) * out_stride + x * 4;
                for (a, v) in acc.iter_mut().zip(&middle[base..base + 4]) {
                    *a += v * w;
                }
            }
            let alpha = acc[3].round().clamp(0.0, 255.0);
            let pixel = &mut target[x * 4..x * 4 + 4];
            // Rounding must not leave a channel above alpha, which would
            // not be a valid premultiplied color.
            for (out, value) in pixel.iter_mut().zip(&acc[..3]) {
                *out = value.round().clamp(0.0, alpha) as u8;
            }
            pixel[3] = alpha as u8;
        }
    }
    Some(out)
}

// DRAWING ------------------------------------------------------------------

/// Composites a prepared (already sized) frame with its top-left corner at
/// whole pixel `(left, top)` of `target`. An opaque frame drawn at full
/// opacity without rounded corners is copied row by row.
pub(super) fn draw(
    target: &mut Pixmap,
    source: PixmapRef,
    opaque: bool,
    left: i32,
    top: i32,
    opacity: f32,
    radius: f32,
) {
    if radius > 0.0 {
        let Some(path) = super::rounded_rect(
            left as f32,
            top as f32,
            source.width() as f32,
            source.height() as f32,
            radius,
        ) else {
            return;
        };
        let paint = Paint {
            shader: Pattern::new(
                source,
                SpreadMode::Pad,
                FilterQuality::Nearest,
                opacity,
                Transform::from_translate(left as f32, top as f32),
            ),
            anti_alias: true,
            ..Paint::default()
        };
        target.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    } else if opaque && opacity >= 1.0 {
        blit(target, source, left, top);
    } else {
        let paint = PixmapPaint {
            opacity,
            quality: FilterQuality::Nearest,
            ..PixmapPaint::default()
        };
        target.draw_pixmap(left, top, source, &paint, Transform::identity(), None);
    }
}

/// Copies `source` over `target` at `(left, top)`, clipped to the target.
fn blit(target: &mut Pixmap, source: PixmapRef, left: i32, top: i32) {
    let (tw, th) = (target.width() as i64, target.height() as i64);
    let (sw, sh) = (source.width() as i64, source.height() as i64);
    let (x0, y0) = ((left as i64).max(0), (top as i64).max(0));
    let (x1, y1) = ((left as i64 + sw).min(tw), (top as i64 + sh).min(th));
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let count = ((x1 - x0) * 4) as usize;
    let src = source.data();
    let dst = target.data_mut();
    for y in y0..y1 {
        let from = (((y - top as i64) * sw + (x0 - left as i64)) * 4) as usize;
        let to = ((y * tw + x0) * 4) as usize;
        dst[to..to + count].copy_from_slice(&src[from..from + count]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JPEG: &[u8] = include_bytes!("../../../../tests/fixtures/quadrants.jpg");
    const GIF: &[u8] = include_bytes!("../../../../tests/fixtures/spin.gif");
    const WEBP: &[u8] = include_bytes!("../../../../tests/fixtures/still.webp");
    const ANIMATED_WEBP: &[u8] = include_bytes!("../../../../tests/fixtures/blink.webp");
    const BMP: &[u8] = include_bytes!("../../../../tests/fixtures/pixels.bmp");
    const PNG: &[u8] = include_bytes!("../../../../tests/fixtures/dot.png");

    fn rgba(pixmap: &Pixmap, x: u32, y: u32) -> (u8, u8, u8, u8) {
        let p = pixmap.pixel(x, y).unwrap();
        (p.red(), p.green(), p.blue(), p.alpha())
    }

    fn size(image: &Decoded) -> (u32, u32) {
        (image.frames[0].width(), image.frames[0].height())
    }

    fn near(actual: (u8, u8, u8, u8), expected: (u8, u8, u8, u8)) -> bool {
        let d = |a: u8, b: u8| (a as i32 - b as i32).abs() <= 8;
        d(actual.0, expected.0)
            && d(actual.1, expected.1)
            && d(actual.2, expected.2)
            && d(actual.3, expected.3)
    }

    #[test]
    fn jpeg_decodes_its_quadrants() {
        let image = decode(JPEG, Limits::default()).unwrap();
        assert_eq!(size(&image), (16, 16));
        assert_eq!(image.frames.len(), 1);
        let frame = &image.frames[0];
        assert!(near(rgba(frame, 3, 3), (255, 0, 0, 255)));
        assert!(near(rgba(frame, 12, 3), (0, 255, 0, 255)));
        assert!(near(rgba(frame, 3, 12), (0, 0, 255, 255)));
        assert!(near(rgba(frame, 12, 12), (255, 255, 255, 255)));
    }

    #[test]
    fn webp_still_keeps_alpha() {
        let image = decode(WEBP, Limits::default()).unwrap();
        assert_eq!((size(&image), image.frames.len()), ((4, 3), 1));
        assert_eq!(rgba(&image.frames[0], 0, 0), (255, 0, 0, 255));
        // Half transparent blue, premultiplied.
        let corner = rgba(&image.frames[0], 3, 2);
        assert_eq!((corner.0, corner.1, corner.3), (0, 0, 128));
        assert!(corner.2 <= 128 && corner.2 >= 126);
        assert!(!is_opaque(&image.frames[0]));
    }

    #[test]
    fn animated_gif_decodes_every_frame_and_delay() {
        let image = decode(GIF, Limits::default()).unwrap();
        assert_eq!(size(&image), (4, 4));
        assert_eq!(image.frames.len(), 3);
        assert_eq!(rgba(&image.frames[0], 1, 1), (255, 0, 0, 255));
        assert_eq!(rgba(&image.frames[1], 1, 1), (0, 255, 0, 255));
        assert_eq!(rgba(&image.frames[2], 1, 1), (0, 0, 255, 255));
        // 50 ms, 0 ms (shown for 100 ms) and 200 ms.
        assert_eq!(image.delays, vec![0.05, 0.1, 0.2]);
        assert_eq!(image.loop_count, 0);
    }

    #[test]
    fn animated_webp_decodes_every_frame_and_loop_count() {
        let image = decode(ANIMATED_WEBP, Limits::default()).unwrap();
        assert_eq!(size(&image), (4, 4));
        assert_eq!(image.frames.len(), 2);
        assert_eq!(rgba(&image.frames[0], 2, 2), (255, 0, 0, 255));
        assert_eq!(rgba(&image.frames[1], 2, 2), (0, 0, 255, 255));
        // 80 ms, then 5 ms (shown for 100 ms).
        assert_eq!(image.delays, vec![0.08, 0.1]);
        assert_eq!(image.loop_count, 3);
    }

    #[test]
    fn bmp_and_png_decode() {
        let bmp = decode(BMP, Limits::default()).unwrap();
        assert_eq!(size(&bmp), (3, 2));
        assert_eq!(rgba(&bmp.frames[0], 0, 0), (255, 0, 0, 255));
        assert_eq!(rgba(&bmp.frames[0], 2, 1), (128, 128, 128, 255));
        let png = decode(PNG, Limits::default()).unwrap();
        assert_eq!(size(&png), (4, 3));
    }

    #[test]
    fn unknown_bytes_are_rejected() {
        let err = decode(b"definitely not an image", Limits::default()).unwrap_err();
        assert!(err.contains("unrecognized image format"), "{}", err);
        // A truncated GIF fails cleanly rather than panicking.
        assert!(decode(&GIF[..20], Limits::default()).is_err());
    }

    #[test]
    fn limits_reject_large_images() {
        let tight = Limits {
            max_pixels: 15,
            max_bytes: DEFAULT_MAX_BYTES,
        };
        let err = decode(GIF, tight).unwrap_err();
        assert!(err.contains("maxPixels"), "{}", err);
        // Each 4x4 frame is 64 bytes, so two fit and the third does not.
        let frames = Limits {
            max_pixels: DEFAULT_MAX_PIXELS,
            max_bytes: 150,
        };
        let err = decode(GIF, frames).unwrap_err();
        assert!(err.contains("maxBytes"), "{}", err);
        assert!(check_size(0, 10, Limits::default()).is_err());
    }

    #[test]
    fn short_delays_are_clamped_like_browsers() {
        assert_eq!(frame_delay(0.0), 0.1);
        assert_eq!(frame_delay(10.0), 0.1);
        assert_eq!(frame_delay(20.0), 0.02);
        assert_eq!(frame_delay(f64::NAN), 0.1);
    }

    #[test]
    fn rgba_images_are_premultiplied() {
        let pixmap = rgba_to_pixmap(vec![255, 255, 255, 128, 0, 0, 0, 0], 2, 1).unwrap();
        let pixels = pixmap.pixels();
        assert_eq!(pixels[0].alpha(), 128);
        assert!(pixels[0].red() <= 128);
        assert_eq!(pixels[1].alpha(), 0);
        assert!(rgba_to_pixmap(vec![0; 7], 2, 1).is_none());
    }

    #[test]
    fn fits_place_images() {
        // A 200x100 image in a 100x100 box.
        let target = (10.0, 20.0, 100.0, 100.0);
        assert_eq!(
            place(200.0, 100.0, target, Fit::Fill),
            ((0.0, 0.0, 200.0, 100.0), target)
        );
        assert_eq!(
            place(200.0, 100.0, target, Fit::Contain),
            ((0.0, 0.0, 200.0, 100.0), (10.0, 45.0, 100.0, 50.0))
        );
        assert_eq!(
            place(200.0, 100.0, target, Fit::Cover),
            ((50.0, 0.0, 100.0, 100.0), target)
        );
        assert!(Fit::parse("stretch").is_err());
        assert_eq!(Fit::parse("cover"), Ok(Fit::Cover));
    }

    #[test]
    fn snapping_keeps_edges_whole() {
        assert_eq!(snap((0.4, 0.6, 10.2, 10.0)), (0, 1, 11, 10));
        assert_eq!(snap((5.0, 5.0, 0.2, 0.2)), (5, 5, 0, 0));
    }

    #[test]
    fn identity_taps_copy_pixels() {
        let taps = filter_taps(4, 0.0, 4.0, 4);
        for (i, (first, weights)) in taps.iter().enumerate() {
            assert_eq!((*first, weights.as_slice()), (i, &[1.0][..]));
        }
        // Halving averages pairs.
        let taps = filter_taps(4, 0.0, 4.0, 2);
        assert_eq!(taps[0].0, 0);
        let sum: f32 = taps[0].1.iter().sum();
        assert!((sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn resampling_averages_and_preserves_solid_colors() {
        // Black and white columns shrink to gray.
        let mut source = Pixmap::new(4, 2).unwrap();
        for (i, pixel) in source.pixels_mut().iter_mut().enumerate() {
            let v = if i % 2 == 0 { 0 } else { 255 };
            *pixel = ColorU8::from_rgba(v, v, v, 255).premultiply();
        }
        let half = resample(&source, (0.0, 0.0, 4.0, 2.0), 2, 1).unwrap();
        let gray = rgba(&half, 0, 0);
        assert!(gray.0 > 100 && gray.0 < 155 && gray.3 == 255, "{:?}", gray);

        // A solid color stays exactly that color at any size.
        let mut solid = Pixmap::new(3, 3).unwrap();
        solid.fill(tiny_skia::Color::from_rgba8(10, 200, 30, 255));
        let big = resample(&solid, (0.0, 0.0, 3.0, 3.0), 17, 5).unwrap();
        assert!(
            big.pixels()
                .iter()
                .all(|p| (p.red(), p.green(), p.blue(), p.alpha()) == (10, 200, 30, 255))
        );
        assert!(is_opaque(&big));

        // An identity resample copies, and a crop picks its region.
        let same = resample(&source, (0.0, 0.0, 4.0, 2.0), 4, 2).unwrap();
        assert_eq!(same.data(), source.data());
        let crop = resample(&source, (1.0, 0.0, 1.0, 1.0), 1, 1).unwrap();
        assert_eq!(rgba(&crop, 0, 0), (255, 255, 255, 255));
    }

    #[test]
    fn opaque_frames_blit_and_clip() {
        let mut target = Pixmap::new(4, 4).unwrap();
        target.fill(tiny_skia::Color::WHITE);
        let mut source = Pixmap::new(2, 2).unwrap();
        source.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        draw(&mut target, source.as_ref(), true, 3, -1, 1.0, 0.0);
        assert_eq!(rgba(&target, 3, 0), (255, 0, 0, 255));
        assert_eq!(rgba(&target, 2, 0), (255, 255, 255, 255));
        assert_eq!(rgba(&target, 3, 1), (255, 255, 255, 255));
        // Off the canvas entirely is a no-op.
        draw(&mut target, source.as_ref(), true, 10, 10, 1.0, 0.0);
        draw(&mut target, source.as_ref(), true, -5, 0, 1.0, 0.0);

        // Half opacity blends with what is underneath.
        draw(&mut target, source.as_ref(), true, 0, 2, 0.5, 0.0);
        let blended = rgba(&target, 0, 2);
        assert!(blended.1 > 100 && blended.1 < 155, "{:?}", blended);

        // Rounded corners leave the corner pixel mostly untouched.
        let mut target = Pixmap::new(20, 20).unwrap();
        target.fill(tiny_skia::Color::WHITE);
        let mut source = Pixmap::new(20, 20).unwrap();
        source.fill(tiny_skia::Color::BLACK);
        draw(&mut target, source.as_ref(), true, 0, 0, 1.0, 8.0);
        assert!(rgba(&target, 0, 0).0 > 200);
        assert_eq!(rgba(&target, 10, 10), (0, 0, 0, 255));
    }
}
