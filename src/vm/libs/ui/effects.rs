//! Pixel effects for `lib/ui.nyk`: gaussian blur, which tiny-skia lacks,
//! and the backdrop blur of `blurRegion`, which blurs what is under a
//! rectangle and mixes it back through the shape and clip coverage.
//!
//! Everything works on the premultiplied RGBA pixels of a `Pixmap`. Wide
//! blurs run on a copy reduced 2-8x, which leaves nothing a gaussian of that
//! width would keep; the copy is read straight from the source and only the
//! pixels that are written are scaled back up.

use std::cell::RefCell;

use tiny_skia::{Mask, Pixmap};

use super::clip::{Bounds, Clip, scale255};
use super::images::scale_pixel;

/// The largest standard deviation, in physical pixels, a blur accepts.
pub(super) const MAX_SIGMA: f32 = 256.0;

/// Radii of three successive box blurs that together approximate a gaussian
/// of standard deviation `sigma`.
fn box_radii(sigma: f32) -> [usize; 3] {
    let ideal = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let mut lower = ideal.floor() as i32;
    if lower % 2 == 0 {
        lower -= 1;
    }
    let lower = lower.max(1);
    let upper = lower + 2;
    let l = lower as f32;
    let switch =
        ((12.0 * sigma * sigma - 3.0 * l * l - 12.0 * l - 9.0) / (-4.0 * l - 4.0)).round() as i32;
    let radius = |i: i32| {
        let size = if i < switch { lower } else { upper };
        ((size - 1) / 2) as usize
    };
    [radius(0), radius(1), radius(2)]
}

/// `(sum + window / 2) / window` as a multiply and shift, exact for the
/// sums a box of `window` bytes can reach (`window` below 4096).
#[inline(always)]
fn average(sum: u32, half: u32, magic: u64) -> u8 {
    (((sum + half) as u64 * magic) >> 32) as u8
}

fn magic(window: u32) -> u64 {
    (1u64 << 32).div_ceil(window as u64)
}

/// One horizontal box blur of radius `r` over `height` rows of `width`
/// pixels, reading `src` and writing `dst`. Edges repeat their outermost
/// pixel. The four channels run together, and only the first and last
/// `r + 1` pixels of a row need their reads clamped to it.
fn box_rows(src: &[u8], dst: &mut [u8], width: usize, height: usize, r: usize) {
    if width == 0 {
        return;
    }
    let window = (2 * r + 1) as u32;
    let (half, magic) = (window / 2, magic(window));
    let last = width - 1;
    for (line, out) in src
        .chunks_exact(width * 4)
        .zip(dst.chunks_exact_mut(width * 4))
        .take(height)
    {
        let px = |i: usize| &line[i * 4..i * 4 + 4];
        let mut sum = [0u32; 4];
        for (s, v) in sum.iter_mut().zip(px(0)) {
            *s = *v as u32 * (r as u32 + 1);
        }
        for i in 1..=r {
            for (s, v) in sum.iter_mut().zip(px(i.min(last))) {
                *s += *v as u32;
            }
        }
        for (i, o) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            for (o, s) in o.iter_mut().zip(&sum) {
                *o = average(*s, half, magic);
            }
            let (add, drop) = (px((i + r + 1).min(last)), px(i.saturating_sub(r)));
            for ((s, a), d) in sum.iter_mut().zip(add).zip(drop) {
                *s = *s + *a as u32 - *d as u32;
            }
        }
    }
}

/// One vertical box blur of radius `r`, swept down the rows with a running
/// sum per byte of a row, so every read and write runs along memory.
fn box_columns(src: &[u8], dst: &mut [u8], width: usize, height: usize, r: usize) {
    if width == 0 || height == 0 {
        return;
    }
    let row = width * 4;
    let window = (2 * r + 1) as u32;
    let (half, magic) = (window / 2, magic(window));
    let last = height - 1;
    let line = |y: usize| &src[y * row..(y + 1) * row];
    let mut sum: Vec<u32> = line(0).iter().map(|v| *v as u32 * (r as u32 + 1)).collect();
    for i in 1..=r {
        for (s, v) in sum.iter_mut().zip(line(i.min(last))) {
            *s += *v as u32;
        }
    }
    for (y, out) in dst.chunks_exact_mut(row).take(height).enumerate() {
        for (o, s) in out.iter_mut().zip(&sum) {
            *o = average(*s, half, magic);
        }
        let (add, drop) = (line((y + r + 1).min(last)), line(y.saturating_sub(r)));
        for ((s, a), d) in sum.iter_mut().zip(add).zip(drop) {
            *s = *s + *a as u32 - *d as u32;
        }
    }
}

/// Blurs `data` (`width` x `height` premultiplied pixels) in place: three box
/// blurs per axis, so the cost does not depend on the radius. Every pass
/// averages all four channels alike, so a channel never ends up above its
/// alpha.
fn blur_full(data: &mut [u8], width: usize, height: usize, sigma: f32) {
    let mut scratch = vec![0u8; data.len()];
    for r in box_radii(sigma) {
        if r == 0 {
            continue;
        }
        box_rows(data, &mut scratch, width, height, r);
        box_columns(&scratch, data, width, height, r);
    }
}

/// The factor a blur of `sigma` pixels may shrink its input by first: wide
/// blurs leave no detail a quarter- or eighth-size copy would lose.
fn reduction(sigma: f32) -> usize {
    if sigma >= 24.0 {
        8
    } else if sigma >= 12.0 {
        4
    } else if sigma >= 4.0 {
        2
    } else {
        1
    }
}

/// A copy of the `region` of an image (`stride` bytes per row) reduced by
/// `factor`: each small pixel averages the block it covers. Returns the
/// pixels and their width and height.
fn shrink(src: &[u8], stride: usize, region: Bounds, factor: usize) -> (Vec<u8>, usize, usize) {
    let (w, h) = (region.width(), region.height());
    let (sw, sh) = (w.div_ceil(factor), h.div_ceil(factor));
    let mut small = vec![0u8; sw * sh * 4];
    if factor == 1 {
        for (y, out) in small.chunks_exact_mut(w * 4).enumerate() {
            let from = (region.y0 as usize + y) * stride + region.x0 as usize * 4;
            out.copy_from_slice(&src[from..from + w * 4]);
        }
        return (small, w, h);
    }
    let mut acc = vec![0u32; sw * 4];
    for (sy, out) in small.chunks_exact_mut(sw * 4).enumerate() {
        acc.fill(0);
        let (y0, y1) = (sy * factor, ((sy + 1) * factor).min(h));
        for y in y0..y1 {
            let from = (region.y0 as usize + y) * stride + region.x0 as usize * 4;
            let line = &src[from..from + w * 4];
            for (block, a) in line.chunks(factor * 4).zip(acc.as_chunks_mut::<4>().0) {
                for px in block.as_chunks::<4>().0 {
                    a[0] += px[0] as u32;
                    a[1] += px[1] as u32;
                    a[2] += px[2] as u32;
                    a[3] += px[3] as u32;
                }
            }
        }
        let rows = (y1 - y0) as u32;
        for (sx, (o, a)) in out
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(acc.as_chunks::<4>().0)
            .enumerate()
        {
            let columns = (((sx + 1) * factor).min(w) - sx * factor) as u32;
            let count = rows * columns;
            for (o, a) in o.iter_mut().zip(a) {
                *o = ((a + count / 2) / count) as u8;
            }
        }
    }
    (small, sw, sh)
}

/// Where output pixel `i` reads a reduced line: the two small pixels around
/// its centre and the weight (of 256) of the second one.
fn taps(len: usize, small_len: usize, factor: usize) -> Vec<(u32, u32, u32)> {
    let f = factor as f32;
    let top = (small_len - 1) as f32;
    (0..len)
        .map(|i| {
            let at = ((i as f32 + 0.5) / f - 0.5).clamp(0.0, top);
            let first = at.floor() as usize;
            let second = (first + 1).min(small_len - 1);
            let weight = ((at - first as f32) * 256.0).round() as u32;
            (first as u32, second as u32, weight)
        })
        .collect()
}

/// Scales a reduced image back up with bilinear filtering, one output row
/// at a time. Linear weights shared by every channel keep the result
/// premultiplied.
struct Grow<'a> {
    small: &'a [u8],
    width: usize,
    columns: Vec<(u32, u32, u32)>,
    rows: Vec<(u32, u32, u32)>,
    /// The two small rows mixed for the current output row, times 256.
    line: Vec<u32>,
}

impl<'a> Grow<'a> {
    /// Output columns `x0..x1` and rows `y0..y1` of an image of `w` x `h`
    /// that `small` (`sw` x `sh`) is the reduction of by `factor`.
    #[allow(clippy::too_many_arguments)]
    fn new(
        small: &'a [u8],
        (sw, sh): (usize, usize),
        (w, h): (usize, usize),
        factor: usize,
        (x0, x1): (usize, usize),
        (y0, y1): (usize, usize),
    ) -> Grow<'a> {
        let columns = taps(w, sw, factor)[x0..x1].to_vec();
        let rows = taps(h, sh, factor)[y0..y1].to_vec();
        Grow {
            small,
            width: sw,
            columns,
            rows,
            line: vec![0; sw * 4],
        }
    }

    /// Output row `i` (counted from `y0`), calling `put` with each column
    /// (counted from `x0`) and its pixel.
    #[inline]
    fn row(&mut self, i: usize, mut put: impl FnMut(usize, u32)) {
        let (top, bottom, weight) = self.rows[i];
        let row = self.width * 4;
        let a = &self.small[top as usize * row..(top as usize + 1) * row];
        let b = &self.small[bottom as usize * row..(bottom as usize + 1) * row];
        for ((l, a), b) in self.line.iter_mut().zip(a).zip(b) {
            *l = *a as u32 * (256 - weight) + *b as u32 * weight;
        }
        for (x, &(left, right, weight)) in self.columns.iter().enumerate() {
            let (p, q) = (left as usize * 4, right as usize * 4);
            let mut out = [0u8; 4];
            for (c, o) in out.iter_mut().enumerate() {
                let v = self.line[p + c] * (256 - weight) + self.line[q + c] * weight;
                *o = ((v + 32768) >> 16) as u8;
            }
            put(x, u32::from_le_bytes(out));
        }
    }
}

/// Blurs the pixmap in place with a gaussian of standard deviation `sigma`
/// pixels. Three box blurs per axis keep the cost independent of the radius;
/// wide blurs run on a reduced copy that is scaled back up with bilinear
/// filtering, which costs a fraction and looks the same.
pub(super) fn blur(pixmap: &mut Pixmap, sigma: f32) {
    if sigma.is_nan() || sigma <= 0.3 {
        return;
    }
    let sigma = sigma.min(MAX_SIGMA);
    let (width, height) = (pixmap.width() as usize, pixmap.height() as usize);
    let factor = reduction(sigma);
    if factor == 1 || width < factor * 2 || height < factor * 2 {
        blur_full(pixmap.data_mut(), width, height, sigma);
        return;
    }
    let region = Bounds::surface(pixmap.width(), pixmap.height());
    let (mut small, sw, sh) = shrink(pixmap.data(), width * 4, region, factor);
    blur_full(&mut small, sw, sh, sigma / factor as f32);
    let mut grow = Grow::new(
        &small,
        (sw, sh),
        (width, height),
        factor,
        (0, width),
        (0, height),
    );
    for (y, out) in pixmap.data_mut().chunks_exact_mut(width * 4).enumerate() {
        let (out, _) = out.as_chunks_mut::<4>();
        grow.row(y, |x, pixel| out[x] = pixel.to_le_bytes());
    }
}

/// The backdrop blur of `blurRegion`: blurs what `target` holds in `region`
/// (the rectangle plus a margin for its edges to see their surroundings)
/// and mixes the result back into the `inner` rectangle, through the
/// `cover` coverage (inner-sized; everything when absent) and the clip.
/// Only the pixels mixed back are scaled up from the reduced copy, and a
/// blur of the same pixels as a recent one copies its result. `cover`
/// comes with a number that tells covers apart (the corner radius).
///
/// The backdrop is usually still while the panels over it are redrawn.
pub(super) fn blur_behind(
    target: &mut Pixmap,
    region: Bounds,
    inner: Bounds,
    sigma: f32,
    cover: Option<(&Mask, u32)>,
    clip: Option<&Clip>,
) {
    let surface = Bounds::surface(target.width(), target.height());
    let region = region.intersect(&surface);
    let out = super::clip::visible(
        clip,
        inner.intersect(&region),
        target.width(),
        target.height(),
    );
    if out.is_empty() || sigma.is_nan() || sigma <= 0.3 {
        return;
    }
    let sigma = sigma.min(MAX_SIGMA);
    let stride = target.width() as usize * 4;
    // A clip mask would have to be part of the key; only plain clips are
    // remembered.
    let key = clip.is_none_or(|c| c.mask.is_none()).then(|| BackdropKey {
        pixels: hash_region(target.data(), stride, region),
        region,
        inner,
        out,
        sigma: sigma.to_bits(),
        cover: cover.map_or(u32::MAX, |(_, id)| id),
    });
    if let Some(key) = &key
        && BACKDROPS.with(|cache| cache.borrow_mut().restore(key, target.data_mut(), stride))
    {
        return;
    }
    mix_blurred(
        target,
        region,
        inner,
        out,
        sigma,
        cover.map(|(m, _)| m),
        clip,
    );
    if let Some(key) = key {
        BACKDROPS.with(|cache| cache.borrow_mut().keep(key, target.data(), stride));
    }
}

/// The blur and mix behind [`blur_behind`], writing the `out` pixels.
fn mix_blurred(
    target: &mut Pixmap,
    region: Bounds,
    inner: Bounds,
    out: Bounds,
    sigma: f32,
    cover: Option<&Mask>,
    clip: Option<&Clip>,
) {
    let (w, h) = (region.width(), region.height());
    let mut factor = reduction(sigma);
    if w < factor * 2 || h < factor * 2 {
        factor = 1;
    }
    let stride = target.width() as usize * 4;
    let (mut small, sw, sh) = shrink(target.data(), stride, region, factor);
    blur_full(&mut small, sw, sh, sigma / factor as f32);

    let (rx, ry) = (region.x0 as usize, region.y0 as usize);
    let (x0, x1) = (out.x0 as usize - rx, out.x1 as usize - rx);
    let (y0, y1) = (out.y0 as usize - ry, out.y1 as usize - ry);
    let mut grow =
        (factor > 1).then(|| Grow::new(&small, (sw, sh), (w, h), factor, (x0, x1), (y0, y1)));
    let cover_width = cover.map_or(0, |m| m.width() as usize);
    let data = target.data_mut();
    for (i, y) in (out.y0..out.y1).enumerate() {
        let start = y as usize * stride + out.x0 as usize * 4;
        let (row, _) = data[start..start + out.width() * 4].as_chunks_mut::<4>();
        let clip_row = clip.and_then(|c| c.row(y, out.x0, out.x1));
        let cover_row = cover.map(|m| {
            let at = (y - inner.y0) as usize * cover_width + (out.x0 - inner.x0) as usize;
            &m.data()[at..at + out.width()]
        });
        let mut put = |x: usize, s: u32| {
            let mut k = 255;
            if let Some(cover) = cover_row {
                k = cover[x] as u32;
            }
            if let Some(clip) = clip_row {
                k = scale255(k, clip[x] as u32);
            }
            match k {
                0 => {}
                255 => row[x] = s.to_le_bytes(),
                _ => {
                    let d = u32::from_le_bytes(row[x]);
                    row[x] = (scale_pixel(d, 255 - k) + scale_pixel(s, k)).to_le_bytes();
                }
            }
        };
        match grow.as_mut() {
            Some(grow) => grow.row(i, put),
            None => {
                let at = (y0 + i) * sw * 4;
                let line = &small[at + x0 * 4..at + x1 * 4];
                for (x, px) in line.as_chunks::<4>().0.iter().enumerate() {
                    put(x, u32::from_le_bytes(*px));
                }
            }
        }
    }
}

/// A 64-bit hash of the pixels of `region`, four independent lanes so the
/// multiplies overlap. Reading a region is a fraction of blurring it.
fn hash_region(data: &[u8], stride: usize, region: Bounds) -> u64 {
    const K: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut lanes = [K, K.rotate_left(17), K.rotate_left(31), K.rotate_left(47)];
    let width = region.width() * 4;
    for y in region.y0 as usize..region.y1 as usize {
        let from = y * stride + region.x0 as usize * 4;
        let row = &data[from..from + width];
        let (words, rest) = row.as_chunks::<32>();
        for chunk in words {
            for (lane, word) in lanes.iter_mut().zip(chunk.as_chunks::<8>().0) {
                *lane = (*lane ^ u64::from_le_bytes(*word))
                    .wrapping_mul(K)
                    .rotate_left(29);
            }
        }
        for (i, byte) in rest.iter().enumerate() {
            lanes[i & 3] = (lanes[i & 3] ^ *byte as u64).wrapping_mul(K);
        }
        lanes[0] ^= y as u64;
    }
    lanes
        .iter()
        .fold(0u64, |h, lane| (h ^ lane).wrapping_mul(K).rotate_left(23))
}

/// What a backdrop blur's result depends on: the pixels it read and every
/// rectangle and number that shaped it.
#[derive(PartialEq, Eq)]
struct BackdropKey {
    pixels: u64,
    region: Bounds,
    inner: Bounds,
    out: Bounds,
    sigma: u32,
    cover: u32,
}

/// Recent backdrop blurs with the pixels they produced. Glass panels over
/// a still background blur the same pixels every frame; with this they
/// cost a hash of what is under them and a copy.
#[derive(Default)]
struct Backdrops {
    entries: Vec<(BackdropKey, Vec<u8>)>,
}

/// How many results are kept, and how many bytes they may hold.
const BACKDROPS_KEPT: usize = 12;
const BACKDROPS_BYTES: usize = 48 * 1024 * 1024;

impl Backdrops {
    /// Writes the result remembered for `key` into the target. Returns
    /// whether there was one.
    fn restore(&mut self, key: &BackdropKey, data: &mut [u8], stride: usize) -> bool {
        let Some(at) = self.entries.iter().position(|(k, _)| k == key) else {
            return false;
        };
        let entry = self.entries.remove(at);
        let out = key.out;
        let width = out.width() * 4;
        for (y, row) in (out.y0 as usize..out.y1 as usize).zip(entry.1.chunks_exact(width)) {
            let from = y * stride + out.x0 as usize * 4;
            data[from..from + width].copy_from_slice(row);
        }
        self.entries.insert(0, entry);
        true
    }

    /// Remembers the `out` pixels the target now holds for `key`.
    fn keep(&mut self, key: BackdropKey, data: &[u8], stride: usize) {
        let out = key.out;
        let width = out.width() * 4;
        if width * out.height() > BACKDROPS_BYTES / 4 {
            return;
        }
        let mut pixels = Vec::with_capacity(width * out.height());
        for y in out.y0 as usize..out.y1 as usize {
            let from = y * stride + out.x0 as usize * 4;
            pixels.extend_from_slice(&data[from..from + width]);
        }
        self.entries.insert(0, (key, pixels));
        self.entries.truncate(BACKDROPS_KEPT);
        let mut bytes = 0;
        self.entries.retain(|(_, pixels)| {
            bytes += pixels.len();
            bytes <= BACKDROPS_BYTES
        });
    }
}

thread_local! {
    static BACKDROPS: RefCell<Backdrops> = RefCell::new(Backdrops::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::PremultipliedColorU8;

    fn alpha_at(pixmap: &Pixmap, x: u32, y: u32) -> u8 {
        pixmap.pixels()[(y * pixmap.width() + x) as usize].alpha()
    }

    #[test]
    fn box_radii_grow_with_sigma() {
        assert!(box_radii(1.5).iter().sum::<usize>() > 0);
        let small: usize = box_radii(2.0).iter().sum();
        let large: usize = box_radii(20.0).iter().sum();
        assert!(large > small * 5, "{} vs {}", large, small);
    }

    #[test]
    fn averages_match_division() {
        for window in [1u32, 3, 5, 17, 101, 513, 1025] {
            let (half, magic) = (window / 2, magic(window));
            for sum in (0..=255 * window).step_by(7) {
                assert_eq!(average(sum, half, magic) as u32, (sum + half) / window);
            }
            let top = 255 * window;
            assert_eq!(average(top, half, magic) as u32, (top + half) / window);
        }
    }

    #[test]
    fn blur_keeps_flat_colors_and_spreads_points() {
        let mut flat = Pixmap::new(9, 9).unwrap();
        flat.fill(tiny_skia::Color::from_rgba8(10, 20, 30, 255));
        blur(&mut flat, 3.0);
        assert!(
            flat.pixels()
                .iter()
                .all(|p| p.alpha() == 255 && p.red() == 10)
        );

        let mut dot = Pixmap::new(21, 21).unwrap();
        dot.pixels_mut()[10 * 21 + 10] =
            PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap();
        blur(&mut dot, 2.0);
        let (centre, near, far) = (
            alpha_at(&dot, 10, 10),
            alpha_at(&dot, 12, 10),
            alpha_at(&dot, 0, 0),
        );
        assert!(
            centre < 255 && centre > near && near > far,
            "{} {} {}",
            centre,
            near,
            far
        );
        // No light is lost away from the edges.
        let total: u32 = dot.pixels().iter().map(|p| p.alpha() as u32).sum();
        assert!(total > 150 && total < 330, "{}", total);
    }

    #[test]
    fn wide_blur_on_a_reduced_copy_matches_the_direct_one() {
        // A white square on black, blurred directly and through the reduced
        // copy: the two agree to within a few levels everywhere.
        let mut direct = Pixmap::new(64, 64).unwrap();
        direct.fill(tiny_skia::Color::BLACK);
        for y in 24..40 {
            for x in 24..40 {
                direct.pixels_mut()[y * 64 + x] =
                    PremultipliedColorU8::from_rgba(255, 255, 255, 255).unwrap();
            }
        }
        let mut reduced = direct.clone();
        let (w, h) = (64, 64);
        blur_full(direct.data_mut(), w, h, 6.0);
        blur(&mut reduced, 6.0);
        let worst = direct
            .pixels()
            .iter()
            .zip(reduced.pixels())
            .map(|(a, b)| (a.red() as i32 - b.red() as i32).abs())
            .max()
            .unwrap();
        assert!(worst <= 12, "differs by {}", worst);
        assert!(reduced.pixels().iter().all(|p| p.red() <= p.alpha()));
    }

    #[test]
    fn blurring_behind_writes_only_the_covered_clipped_pixels() {
        // Black left half, white right half; blur the middle strip.
        let mut target = Pixmap::new(64, 16).unwrap();
        for (i, pixel) in target.pixels_mut().iter_mut().enumerate() {
            let v = if i % 64 < 32 { 0 } else { 255 };
            *pixel = PremultipliedColorU8::from_rgba(v, v, v, 255).unwrap();
        }
        let before = target.clone();
        let inner = Bounds::from_xywh(16, 4, 32, 8);
        let region = Bounds::from_xywh(0, 0, 64, 16);
        let clip = Clip::rect(64, 16, None, (0.0, 0.0, 40.0, 16.0));
        blur_behind(&mut target, region, inner, 6.0, None, Some(&clip));
        let red = |p: &Pixmap, x: usize, y: usize| p.pixels()[y * 64 + x].red();
        // Inside: the edge between the halves is softened.
        assert!(red(&target, 31, 8) > 0 && red(&target, 31, 8) < 255);
        // Outside the rectangle or the clip nothing changes.
        assert_eq!(red(&target, 31, 2), red(&before, 31, 2));
        assert_eq!(red(&target, 44, 8), red(&before, 44, 8));
        assert!(target.pixels().iter().all(|p| p.red() <= p.alpha()));

        // A cover mask keeps its zero pixels untouched.
        let mut target = before.clone();
        let mut cover = Mask::new(32, 8).unwrap();
        cover.data_mut()[4 * 32 + 15] = 255;
        blur_behind(&mut target, region, inner, 6.0, Some((&cover, 0)), None);
        assert_ne!(red(&target, 31, 8), red(&before, 31, 8));
        assert_eq!(red(&target, 30, 8), red(&before, 30, 8));
    }

    #[test]
    fn a_repeated_backdrop_blur_is_restored_from_the_cache() {
        let mut target = Pixmap::new(48, 24).unwrap();
        for (i, pixel) in target.pixels_mut().iter_mut().enumerate() {
            let v = ((i * 37) % 251) as u8;
            *pixel = PremultipliedColorU8::from_rgba(v, v / 2, v / 3, 255).unwrap();
        }
        let before = target.clone();
        let region = Bounds::from_xywh(0, 0, 48, 24);
        let inner = Bounds::from_xywh(8, 4, 30, 14);
        let mut first = before.clone();
        blur_behind(&mut first, region, inner, 9.0, None, None);
        let mut second = before.clone();
        blur_behind(&mut second, region, inner, 9.0, None, None);
        assert_eq!(first.data(), second.data());
        // The cached copy matches what blurring computes.
        let mut direct = before.clone();
        mix_blurred(&mut direct, region, inner, inner, 9.0, None, None);
        assert_eq!(first.data(), direct.data());
        // Different pixels under the panel are blurred afresh.
        let mut other = before.clone();
        other.fill(tiny_skia::Color::BLACK);
        blur_behind(&mut other, region, inner, 9.0, None, None);
        assert_eq!(other.pixels()[10 * 48 + 20].red(), 0);
    }
}
