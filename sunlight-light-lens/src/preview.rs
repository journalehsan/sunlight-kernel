//! One bounded viewport cache. No decoded-image or history snapshots.
use crate::editor::{Sampler, State};
use alloc::vec::Vec;
use sunlight_ui::{image::RgbaImage, Rect};

pub const MAX_CACHE_PIXELS: usize = 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    state: State,
    source: usize,
    dst: Rect,
    clip: Rect,
    step: u32,
}
#[derive(Default)]
pub struct PreviewCache {
    key: Option<Key>,
    pixels: Vec<u32>,
    sampler: Option<Sampler>,
    pub sampled_pixels: u64,
}
impl PreviewCache {
    /// Returns false if the bounded cache cannot hold the viewport; callers
    /// can render directly. Failed allocations never discard the source image.
    pub fn prepare(
        &mut self,
        image: &RgbaImage,
        state: State,
        dst: Rect,
        clip: Rect,
        interactive: bool,
    ) -> bool {
        let step = if interactive { 2 } else { 1 };
        let key = Key {
            state,
            source: image.pixels.as_ptr() as usize,
            dst,
            clip,
            step,
        };
        if self.key == Some(key) {
            return true;
        }
        self.key = None;
        if dst.w == 0 || dst.h == 0 {
            return false;
        }
        let nx = clip.w.div_ceil(step);
        let ny = clip.h.div_ceil(step);
        let Some(len) = (nx as usize).checked_mul(ny as usize) else {
            return false;
        };
        if len > MAX_CACHE_PIXELS {
            return false;
        }
        if self
            .pixels
            .try_reserve_exact(len.saturating_sub(self.pixels.len()))
            .is_err()
        {
            return false;
        }
        self.pixels.resize(len, 0);
        if let Some(sampler) = self.sampler.as_mut() {
            sampler.update(state);
        } else {
            self.sampler = Some(Sampler::new(state));
        }
        let sampler = self.sampler.as_ref().unwrap();
        for y in 0..ny {
            let sy = ((clip.y as i64 + (y * step) as i64 - dst.y as i64) * state.size.1 as i64
                / dst.h as i64) as u32;
            for x in 0..nx {
                let sx = ((clip.x as i64 + (x * step) as i64 - dst.x as i64) * state.size.0 as i64
                    / dst.w as i64) as u32;
                self.pixels[(y * nx + x) as usize] = sampler.pixel(image, sx, sy);
            }
        }
        self.sampled_pixels += len as u64;
        self.key = Some(key);
        true
    }
    pub fn pixel(&self, x: u32, y: u32) -> u32 {
        let key = self.key.unwrap();
        let nx = key.clip.w.div_ceil(key.step);
        let cx = (x as i32 - key.clip.x) as u32 / key.step;
        let cy = (y as i32 - key.clip.y) as u32 / key.step;
        self.pixels[(cy * nx + cx) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    #[test]
    fn cache_reuses_pixels_and_invalidates_for_edits_geometry_view_and_quality() {
        let image = RgbaImage {
            width: 100,
            height: 100,
            pixels: vec![0xff102030; 10000],
        };
        let mut state = State::new(100, 100);
        let dst = Rect::new(-20, -10, 120, 120);
        let clip = Rect::new(0, 0, 100, 100);
        let mut c = PreviewCache::default();
        assert!(c.prepare(&image, state, dst, clip, false));
        assert_eq!(c.sampled_pixels, 10000);
        let capacity = c.pixels.capacity();
        assert_eq!(c.pixel(0, 0), 0xff102030);
        for _ in 0..20 {
            assert!(c.prepare(&image, state, dst, clip, false));
        }
        assert_eq!(c.sampled_pixels, 10000);
        state.adjust[0] = 30;
        assert!(c.prepare(&image, state, dst, clip, true));
        assert_eq!(c.sampled_pixels, 12500);
        assert_ne!(c.pixel(0, 0), 0xff102030);
        assert!(c.prepare(&image, state, dst, clip, false));
        assert_eq!(c.sampled_pixels, 22500);
        assert_eq!(c.pixels.capacity(), capacity);
        state.geometry(crate::editor::Geometry::FlipH).unwrap();
        assert!(c.prepare(&image, state, dst, clip, false));
        assert_eq!(c.sampled_pixels, 32500);
        assert!(c.prepare(&image, state, Rect::new(-19, -10, 120, 120), clip, false));
        assert_eq!(c.sampled_pixels, 42500);
        assert!(c.prepare(&image, state, dst, Rect::new(0, 0, 90, 100), false));
        assert_eq!(c.sampled_pixels, 51500);
        let replacement = RgbaImage {
            width: 100,
            height: 100,
            pixels: vec![0xffffffff; 10000],
        };
        assert!(c.prepare(&replacement, state, dst, clip, false));
        assert_eq!(c.sampled_pixels, 61500);
        assert_eq!(c.pixel(0, 0), 0xffffffff);
        std::println!("cache evidence: 20 unchanged redraws added 0 samples; 100x100 drag preview used 2500 vs 10000 full-quality samples");
    }
    #[test]
    fn cache_is_bounded_and_allows_direct_fallback() {
        let image = RgbaImage {
            width: 1,
            height: 1,
            pixels: vec![0xffffffff],
        };
        let state = State::new(1, 1);
        let mut c = PreviewCache::default();
        let r = Rect::new(0, 0, 1024, 1024);
        assert!(c.prepare(&image, state, r, r, false));
        assert_eq!(c.pixels.capacity(), MAX_CACHE_PIXELS);
        let r = Rect::new(0, 0, 1025, 1024);
        assert!(!c.prepare(&image, state, r, r, false));
        assert_eq!(c.pixels.capacity(), MAX_CACHE_PIXELS);
        assert!(c.key.is_none());
    }
}
