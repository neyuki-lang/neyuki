//! Clip regions for `@neyuki/ui`.
//!
//! A clip is a rectangle of whole pixels plus, for rounded and path clips,
//! a coverage mask. Plain rectangles, which is what scroll views and lists
//! push every frame, need no mask at all: pushing one is a few comparisons
//! and drawing inside one is as fast as drawing unclipped. The compositing
//! loops in `images` and `effects` only ever visit the rows and columns
//! inside [`Clip::bounds`], and read the mask (when there is one) there.
//!
//! tiny-skia wants a mask the size of the surface. [`Clip::for_skia`] hands
//! it none when the shape lies inside the bounds, the clip's own mask when
//! that is enough, and builds an exact one only for shapes that cross the
//! edge.

use std::cell::OnceCell;
use std::rc::Rc;

use tiny_skia::{FillRule, Mask, Path, Transform};

/// A rectangle of whole pixels, `x0..x1` by `y0..y1`. Empty when either
/// range is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Bounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds {
        x0: 0,
        y0: 0,
        x1: 0,
        y1: 0,
    };

    pub fn surface(width: u32, height: u32) -> Bounds {
        Bounds {
            x0: 0,
            y0: 0,
            x1: width.min(i32::MAX as u32) as i32,
            y1: height.min(i32::MAX as u32) as i32,
        }
    }

    pub fn from_xywh(x: i32, y: i32, w: u32, h: u32) -> Bounds {
        let clamp = |v: i64| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        Bounds {
            x0: x,
            y0: y,
            x1: clamp(x as i64 + w as i64),
            y1: clamp(y as i64 + h as i64),
        }
    }

    /// The pixels a shape spanning `left..right` by `top..bottom` (physical,
    /// fractional) touches, grown by `pad` on every side.
    pub fn covering(left: f32, top: f32, right: f32, bottom: f32, pad: f32) -> Bounds {
        let clamp = |v: f32| v.clamp(-(1 << 30) as f32, (1 << 30) as f32) as i32;
        if !(left <= right && top <= bottom) {
            return Bounds::EMPTY;
        }
        Bounds {
            x0: clamp((left - pad).floor()),
            y0: clamp((top - pad).floor()),
            x1: clamp((right + pad).ceil()),
            y1: clamp((bottom + pad).ceil()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.x1 <= self.x0 || self.y1 <= self.y0
    }

    pub fn width(&self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }

    pub fn height(&self) -> usize {
        (self.y1 - self.y0).max(0) as usize
    }

    pub fn intersect(&self, other: &Bounds) -> Bounds {
        let out = Bounds {
            x0: self.x0.max(other.x0),
            y0: self.y0.max(other.y0),
            x1: self.x1.min(other.x1),
            y1: self.y1.min(other.y1),
        };
        if out.is_empty() { Bounds::EMPTY } else { out }
    }

    /// The smallest rectangle holding both; an empty side counts as nothing.
    pub fn union(&self, other: &Bounds) -> Bounds {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        Bounds {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    pub fn contains(&self, other: &Bounds) -> bool {
        other.x0 >= self.x0 && other.y0 >= self.y0 && other.x1 <= self.x1 && other.y1 <= self.y1
    }
}

/// What a tiny-skia draw should do about the clip.
pub(super) enum Skia<'a> {
    /// The shape lies wholly outside the clip: draw nothing.
    Skip,
    /// Draw with this mask (none: unclipped).
    Draw(Option<&'a Mask>),
}

/// One entry of a target's clip stack, already intersected with the clips
/// below it.
pub(super) struct Clip {
    /// Drawing is limited to these pixels, which lie inside the surface.
    pub bounds: Bounds,
    /// The coverage inside `bounds`, unless all of it is let through. The
    /// mask is the size of the surface and may let through pixels outside
    /// `bounds` (a rectangle pushed inside a rounded clip shares the
    /// rounded clip's mask); nothing reads it there.
    pub mask: Option<Rc<Mask>>,
    /// An exact surface-sized mask of this clip, made the first time a
    /// tiny-skia draw crosses the edge of `bounds`.
    exact: OnceCell<Mask>,
}

impl Clip {
    fn new(bounds: Bounds, mask: Option<Rc<Mask>>) -> Clip {
        Clip {
            bounds,
            mask,
            exact: OnceCell::new(),
        }
    }

    /// A rectangle clip inside `parent` (or the `width` x `height`
    /// surface). Its edges are rounded to whole pixels, as browsers do with
    /// rectangular clips, so it needs no mask of its own.
    pub fn rect(
        width: u32,
        height: u32,
        parent: Option<&Clip>,
        (left, top, right, bottom): (f32, f32, f32, f32),
    ) -> Clip {
        let surface = Bounds::surface(width, height);
        let below = parent.map_or(surface, |clip| clip.bounds);
        let snapped = if left <= right && top <= bottom {
            let round = |v: f32| v.round().clamp(-(1 << 30) as f32, (1 << 30) as f32) as i32;
            Bounds {
                x0: round(left),
                y0: round(top),
                x1: round(right),
                y1: round(bottom),
            }
        } else {
            Bounds::EMPTY
        };
        let bounds = snapped.intersect(&below);
        let mask = parent.and_then(|clip| clip.mask.clone());
        Clip::new(bounds, if bounds.is_empty() { None } else { mask })
    }

    /// A clip to the inside of `path` (drawn through `transform`, with
    /// anti-aliased edges) inside `parent`. `None` for the path clips
    /// everything away.
    pub fn path(
        width: u32,
        height: u32,
        parent: Option<&Clip>,
        path: Option<&Path>,
        transform: Transform,
    ) -> Option<Clip> {
        let surface = Bounds::surface(width, height);
        let below = parent.map_or(surface, |clip| clip.bounds);
        let Some(path) = path else {
            return Some(Clip::new(Bounds::EMPTY, None));
        };
        let shape = path.clone().transform(transform).map(|p| p.bounds());
        let bounds = match shape {
            Some(rect) => {
                Bounds::covering(rect.left(), rect.top(), rect.right(), rect.bottom(), 1.0)
                    .intersect(&below)
            }
            None => Bounds::EMPTY,
        };
        if bounds.is_empty() {
            return Some(Clip::new(Bounds::EMPTY, None));
        }
        let mut mask = Mask::new(width, height)?;
        mask.fill_path(path, FillRule::Winding, true, transform);
        if let Some(below) = parent.and_then(|clip| clip.mask.as_deref()) {
            let (data, under) = (mask.data_mut(), below.data());
            let stride = width as usize;
            for y in bounds.y0 as usize..bounds.y1 as usize {
                let (from, to) = (
                    y * stride + bounds.x0 as usize,
                    y * stride + bounds.x1 as usize,
                );
                for (a, b) in data[from..to].iter_mut().zip(&under[from..to]) {
                    *a = scale255(*a as u32, *b as u32) as u8;
                }
            }
        }
        Some(Clip::new(bounds, Some(Rc::new(mask))))
    }

    /// The coverage row `y` of `x0..x1` gets from the mask, if there is one.
    /// The range must lie inside `bounds`.
    #[inline]
    pub fn row(&self, y: i32, x0: i32, x1: i32) -> Option<&[u8]> {
        self.mask.as_deref().map(|mask| {
            let start = y as usize * mask.width() as usize;
            &mask.data()[start + x0 as usize..start + x1 as usize]
        })
    }

    /// How a tiny-skia draw that touches `area` must clip itself.
    pub fn for_skia(&self, area: Bounds, width: u32, height: u32) -> Skia<'_> {
        let area = area.intersect(&Bounds::surface(width, height));
        if area.intersect(&self.bounds).is_empty() {
            return Skia::Skip;
        }
        if self.bounds.contains(&area) {
            return Skia::Draw(self.mask.as_deref());
        }
        match self.exact(width, height) {
            Some(mask) => Skia::Draw(Some(mask)),
            None => Skia::Skip,
        }
    }

    fn exact(&self, width: u32, height: u32) -> Option<&Mask> {
        if self.exact.get().is_none() {
            let mut mask = Mask::new(width, height)?;
            let stride = width as usize;
            let (x0, x1) = (self.bounds.x0 as usize, self.bounds.x1 as usize);
            let data = mask.data_mut();
            for y in self.bounds.y0 as usize..self.bounds.y1 as usize {
                let row = &mut data[y * stride + x0..y * stride + x1];
                match self.mask.as_deref() {
                    Some(own) => row.copy_from_slice(&own.data()[y * stride + x0..y * stride + x1]),
                    None => row.fill(255),
                }
            }
            let _ = self.exact.set(mask);
        }
        self.exact.get()
    }
}

/// How a draw touching `area` is clipped by an optional clip: the tiny-skia
/// counterpart of [`Clip::for_skia`] for an unclipped target.
pub(super) fn skia_clip(clip: Option<&Clip>, area: Bounds, width: u32, height: u32) -> Skia<'_> {
    match clip {
        Some(clip) => clip.for_skia(area, width, height),
        None if area.intersect(&Bounds::surface(width, height)).is_empty() => Skia::Skip,
        None => Skia::Draw(None),
    }
}

/// The pixels a draw at `area` may change: the part inside the clip and
/// the surface.
pub(super) fn visible(clip: Option<&Clip>, area: Bounds, width: u32, height: u32) -> Bounds {
    let area = area.intersect(&Bounds::surface(width, height));
    match clip {
        Some(clip) => area.intersect(&clip.bounds),
        None => area,
    }
}

/// `x * a / 255`, rounded, for bytes.
#[inline]
pub(super) fn scale255(x: u32, a: u32) -> u32 {
    let t = x * a + 128;
    (t + (t >> 8)) >> 8
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiny_skia::PathBuilder;

    #[test]
    fn bounds_intersect_and_union() {
        let a = Bounds::from_xywh(0, 0, 10, 10);
        let b = Bounds::from_xywh(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Bounds::from_xywh(5, 5, 5, 5));
        assert_eq!(a.union(&b), Bounds::from_xywh(0, 0, 15, 15));
        assert!(a.intersect(&Bounds::from_xywh(20, 20, 1, 1)).is_empty());
        assert_eq!(Bounds::EMPTY.union(&a), a);
        assert!(a.contains(&Bounds::from_xywh(2, 2, 3, 3)));
        assert!(!a.contains(&b));
    }

    #[test]
    fn rect_clips_snap_and_nest_without_masks() {
        let outer = Clip::rect(100, 100, None, (10.4, 10.6, 50.2, 60.0));
        assert_eq!(outer.bounds, Bounds::from_xywh(10, 11, 40, 49));
        assert!(outer.mask.is_none());
        let inner = Clip::rect(100, 100, Some(&outer), (0.0, 0.0, 30.0, 200.0));
        assert_eq!(inner.bounds, Bounds::from_xywh(10, 11, 20, 49));
        let off = Clip::rect(100, 100, Some(&outer), (70.0, 70.0, 80.0, 80.0));
        assert!(off.bounds.is_empty());
        // Off the surface is clipped to it.
        let wide = Clip::rect(100, 100, None, (-10.0, -10.0, 500.0, 500.0));
        assert_eq!(wide.bounds, Bounds::surface(100, 100));
    }

    #[test]
    fn path_clips_carry_a_mask_inside_their_parent() {
        let mut pb = PathBuilder::new();
        pb.push_circle(50.0, 50.0, 20.0);
        let circle = pb.finish().unwrap();
        let clip = Clip::path(100, 100, None, Some(&circle), Transform::identity()).unwrap();
        let mask = clip.mask.as_deref().unwrap();
        assert_eq!(mask.data()[50 * 100 + 50], 255);
        assert_eq!(mask.data()[50 * 100 + 25], 0);
        assert!(clip.bounds.contains(&Bounds::from_xywh(30, 30, 40, 40)));

        let half = Clip::rect(100, 100, None, (0.0, 0.0, 50.0, 100.0));
        let nested =
            Clip::path(100, 100, Some(&half), Some(&circle), Transform::identity()).unwrap();
        assert!(nested.bounds.x1 <= 50);
        let gone = Clip::path(100, 100, None, None, Transform::identity()).unwrap();
        assert!(gone.bounds.is_empty());
    }

    #[test]
    fn skia_gets_a_mask_only_across_the_edge() {
        let clip = Clip::rect(100, 100, None, (10.0, 10.0, 90.0, 90.0));
        assert!(matches!(
            clip.for_skia(Bounds::from_xywh(20, 20, 10, 10), 100, 100),
            Skia::Draw(None)
        ));
        assert!(matches!(
            clip.for_skia(Bounds::from_xywh(95, 95, 3, 3), 100, 100),
            Skia::Skip
        ));
        match clip.for_skia(Bounds::from_xywh(0, 0, 50, 50), 100, 100) {
            Skia::Draw(Some(mask)) => {
                assert_eq!(mask.data()[5 * 100 + 5], 0);
                assert_eq!(mask.data()[50 * 100 + 50], 255);
            }
            _ => panic!("expected an exact mask"),
        }
    }
}
