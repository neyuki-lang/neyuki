//! Pixel effects for `lib/ui.nyk`: gaussian blur, which tiny-skia lacks,
//! and the helpers that composite a blurred copy back through clip masks.
//!
//! Everything works on the premultiplied RGBA pixels of a `Pixmap`.

use tiny_skia::{Mask, Pixmap, PremultipliedColorU8};

/// The largest standard deviation, in physical pixels, a blur accepts.
pub(super) const MAX_SIGMA: f32 = 256.0;

/// How much of a pixel a clip mask lets through, given the `coverage` it
/// already has (0 to 255). Pixels outside the mask are clipped away.
pub(super) fn coverage_in(clip: &Mask, x: i32, y: i32, coverage: u32) -> u32 {
    let (width, height) = (clip.width() as i32, clip.height() as i32);
    if x < 0 || y < 0 || x >= width || y >= height {
        return 0;
    }
    (coverage * clip.data()[(y * width + x) as usize] as u32 + 127) / 255
}

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

/// One box blur of radius `r` over `lines` lines of `len` pixels, reading
/// `src` and writing `dst`. Pixel `i` of line `n` starts at byte
/// `n * line_stride + i * step`. Edges repeat their outermost pixel.
///
/// The four channels run together, and only the first and last `r + 1`
/// pixels of a line need their reads clamped to it.
fn box_lines(
    src: &[u8],
    dst: &mut [u8],
    lines: usize,
    len: usize,
    line_stride: usize,
    step: usize,
    r: usize,
) {
    if len == 0 {
        return;
    }
    let window = (2 * r + 1) as u32;
    let half = window / 2;
    let last = len - 1;
    for line in 0..lines {
        let base = line * line_stride;
        let pixel = |i: usize| base + i * step;
        let first = pixel(0);
        let mut sum = [0u32; 4];
        for (c, s) in sum.iter_mut().enumerate() {
            *s = src[first + c] as u32 * (r as u32 + 1);
        }
        for i in 1..=r {
            let at = pixel(i.min(last));
            for (c, s) in sum.iter_mut().enumerate() {
                *s += src[at + c] as u32;
            }
        }
        for i in 0..len {
            let out = pixel(i);
            for (c, s) in sum.iter().enumerate() {
                dst[out + c] = ((s + half) / window) as u8;
            }
            let add = pixel((i + r + 1).min(last));
            let drop = pixel(i.saturating_sub(r));
            for (c, s) in sum.iter_mut().enumerate() {
                *s = *s + src[add + c] as u32 - src[drop + c] as u32;
            }
        }
    }
}

/// Blurs `data` (`width` x `height` premultiplied pixels) in place: three box
/// blurs per axis, so the cost does not depend on the radius.
fn blur_full(data: &mut [u8], width: usize, height: usize, sigma: f32) {
    let mut scratch = vec![0u8; data.len()];
    for r in box_radii(sigma) {
        if r == 0 {
            continue;
        }
        box_lines(data, &mut scratch, height, width, width * 4, 4, r);
        box_lines(&scratch, data, width, height, 4, width * 4, r);
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
    let (sw, sh) = (width.div_ceil(factor), height.div_ceil(factor));
    let data = pixmap.data_mut();
    // Shrink: each small pixel averages the factor x factor block it covers.
    let mut small = vec![0u8; sw * sh * 4];
    for sy in 0..sh {
        let (y0, y1) = (sy * factor, ((sy + 1) * factor).min(height));
        for sx in 0..sw {
            let (x0, x1) = (sx * factor, ((sx + 1) * factor).min(width));
            let mut acc = [0u32; 4];
            for y in y0..y1 {
                let row = y * width * 4;
                for x in x0..x1 {
                    let at = row + x * 4;
                    for (c, a) in acc.iter_mut().enumerate() {
                        *a += data[at + c] as u32;
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u32;
            let out = (sy * sw + sx) * 4;
            for (c, a) in acc.iter().enumerate() {
                small[out + c] = ((a + count / 2) / count) as u8;
            }
        }
    }
    blur_full(&mut small, sw, sh, sigma / factor as f32);
    // Grow back: bilinear between the centres of the small pixels.
    let f = factor as f32;
    for y in 0..height {
        let fy = ((y as f32 + 0.5) / f - 0.5).clamp(0.0, (sh - 1) as f32);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let ty = fy - y0 as f32;
        for x in 0..width {
            let fx = ((x as f32 + 0.5) / f - 0.5).clamp(0.0, (sw - 1) as f32);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let tx = fx - x0 as f32;
            let (a, b) = ((y0 * sw + x0) * 4, (y0 * sw + x1) * 4);
            let (c, d) = ((y1 * sw + x0) * 4, (y1 * sw + x1) * 4);
            let out = (y * width + x) * 4;
            for k in 0..4 {
                let top = small[a + k] as f32 + (small[b + k] as f32 - small[a + k] as f32) * tx;
                let bottom = small[c + k] as f32 + (small[d + k] as f32 - small[c + k] as f32) * tx;
                data[out + k] = (top + (bottom - top) * ty + 0.5) as u8;
            }
        }
    }
    // Rounding can leave a color channel a hair above its alpha, which is
    // not a valid premultiplied pixel.
    for pixel in data.chunks_exact_mut(4) {
        let alpha = pixel[3];
        for channel in &mut pixel[..3] {
            *channel = (*channel).min(alpha);
        }
    }
}

/// Mixes `source` into `target` with its top-left corner at `(left, top)`:
/// each pixel moves toward the source by `cover` (the same size as the
/// source; everything when absent) times `clip` (the size of the target).
pub(super) fn blend_region(
    target: &mut Pixmap,
    source: &Pixmap,
    left: i32,
    top: i32,
    cover: Option<&Mask>,
    clip: Option<&Mask>,
) {
    let (tw, th) = (target.width() as i32, target.height() as i32);
    let (sw, sh) = (source.width() as i32, source.height() as i32);
    let pixels = target.pixels_mut();
    let from = source.pixels();
    for sy in 0..sh {
        let y = top + sy;
        if y < 0 || y >= th {
            continue;
        }
        for sx in 0..sw {
            let x = left + sx;
            if x < 0 || x >= tw {
                continue;
            }
            let mut coverage = match cover {
                Some(cover) => cover.data()[(sy * sw + sx) as usize] as u32,
                None => 255,
            };
            if let Some(clip) = clip {
                coverage = coverage_in(clip, x, y, coverage);
            }
            if coverage == 0 {
                continue;
            }
            let index = (y * tw + x) as usize;
            let (dst, src) = (pixels[index], from[(sy * sw + sx) as usize]);
            let mix = |d: u8, s: u8| {
                (d as i32 + (s as i32 - d as i32) * coverage as i32 / 255).clamp(0, 255) as u8
            };
            let alpha = mix(dst.alpha(), src.alpha());
            let red = mix(dst.red(), src.red()).min(alpha);
            let green = mix(dst.green(), src.green()).min(alpha);
            let blue = mix(dst.blue(), src.blue()).min(alpha);
            if let Some(pixel) = PremultipliedColorU8::from_rgba(red, green, blue, alpha) {
                pixels[index] = pixel;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn blend_region_follows_cover_and_clip() {
        let mut target = Pixmap::new(4, 1).unwrap();
        target.fill(tiny_skia::Color::BLACK);
        let mut source = Pixmap::new(2, 1).unwrap();
        source.fill(tiny_skia::Color::WHITE);
        // Right half of the source hangs off the target.
        blend_region(&mut target, &source, 3, 0, None, None);
        assert_eq!(target.pixels()[3].red(), 255);
        assert_eq!(target.pixels()[2].red(), 0);

        let mut clip = Mask::new(4, 1).unwrap();
        clip.data_mut().copy_from_slice(&[0, 255, 0, 0]);
        blend_region(&mut target, &source, 0, 0, None, Some(&clip));
        assert_eq!(target.pixels()[0].red(), 0);
        assert_eq!(target.pixels()[1].red(), 255);
        assert_eq!(coverage_in(&clip, 9, 0, 255), 0);
        assert_eq!(coverage_in(&clip, 1, 0, 255), 255);
    }
}
