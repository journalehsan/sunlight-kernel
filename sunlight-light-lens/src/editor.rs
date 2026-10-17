//! Bounded, non-destructive CPU editor. History contains parameters, never pixels.
use sunlight_ui::image::RgbaImage;
use sunlight_ui::{Point, Rect};

// SIMG/TGA need one retained ARGB allocation. Allow bundled 1312x816 photos
// and full-HD images while keeping the original at or below 8 MiB.
pub const MAX_PIXELS: u32 = 2 * 1024 * 1024;
// PNG/JPEG use additional full-image scratch buffers in the shared decoder.
pub const MAX_COMPRESSED_PIXELS: u32 = 1024 * 1024;
pub const MAX_STEPS: usize = 16;
const HISTORY: usize = 32;

pub fn dimensions(w: u32, h: u32) -> Result<(), &'static str> {
    if w == 0 || h == 0 || w > 8192 || h > 8192 || w.checked_mul(h).is_none_or(|n| n > MAX_PIXELS) {
        Err("Image limit: 8192 per side, 2,097,152 pixels.")
    } else {
        Ok(())
    }
}

/// Validate headers before invoking an allocating decoder. Keep the existing
/// PNG/JPEG working-memory limit while accepting larger single-buffer codecs.
pub fn inspect_input(bytes: &[u8]) -> Result<(u32, u32), &'static str> {
    let (w, h) =
        sun_img::inspect_dimensions(bytes).map_err(|_| "Unsupported or malformed image header.")?;
    dimensions(w, h)?;
    if matches!(
        sun_img::detect_format(bytes),
        sun_img::ImageFormat::Png | sun_img::ImageFormat::Jpeg
    ) && w * h > MAX_COMPRESSED_PIXELS
    {
        return Err("PNG/JPEG decode limit: 1,048,576 pixels (scratch memory).");
    }
    Ok((w, h))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Geometry {
    RotateRight,
    RotateLeft,
    FlipH,
    FlipV,
    Crop(Rect),
    Resize(u32, u32),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Step {
    op: Geometry,
    before: (u32, u32),
}
const EMPTY_STEP: Step = Step {
    op: Geometry::FlipH,
    before: (0, 0),
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Effect {
    #[default]
    None,
    Grayscale,
    Sepia,
    Invert,
    Blur,
    Sharpen,
    Edges,
}
impl Effect {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Grayscale => "Grayscale",
            Self::Sepia => "Sepia",
            Self::Invert => "Invert",
            Self::Blur => "Blur",
            Self::Sharpen => "Sharpen",
            Self::Edges => "Edges",
        }
    }
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::Grayscale,
            Self::Grayscale => Self::Sepia,
            Self::Sepia => Self::Invert,
            Self::Invert => Self::Blur,
            Self::Blur => Self::Sharpen,
            Self::Sharpen => Self::Edges,
            Self::Edges => Self::None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub size: (u32, u32),
    steps: [Step; MAX_STEPS],
    count: usize,
    /// brightness, contrast, saturation: -100..100; gamma: -100..100 => 0.25..4.
    pub adjust: [i16; 4],
    pub effect: Effect,
}
impl State {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            size: (w, h),
            steps: [EMPTY_STEP; MAX_STEPS],
            count: 0,
            adjust: [0; 4],
            effect: Effect::None,
        }
    }
    pub fn geometry(&mut self, op: Geometry) -> Result<(), &'static str> {
        if self.count == MAX_STEPS {
            return Err("Geometry limit reached (16). Reset to start again.");
        }
        let (w, h) = self.size;
        let next = match op {
            Geometry::RotateLeft | Geometry::RotateRight => (h, w),
            Geometry::FlipH | Geometry::FlipV => (w, h),
            Geometry::Resize(w, h) => (w, h),
            Geometry::Crop(r) => {
                if r.x < 0
                    || r.y < 0
                    || r.w == 0
                    || r.h == 0
                    || r.x as u64 + r.w as u64 > w as u64
                    || r.y as u64 + r.h as u64 > h as u64
                {
                    return Err("Select a valid crop inside the image.");
                }
                (r.w, r.h)
            }
        };
        dimensions(next.0, next.1)?;
        self.steps[self.count] = Step {
            op,
            before: self.size,
        };
        self.count += 1;
        self.size = next;
        Ok(())
    }
    fn source_point(&self, mut x: u32, mut y: u32) -> (u32, u32) {
        for step in self.steps[..self.count].iter().rev() {
            let (w, h) = step.before;
            (x, y) = match step.op {
                Geometry::RotateRight => (y, h - 1 - x),
                Geometry::RotateLeft => (w - 1 - y, x),
                Geometry::FlipH => (w - 1 - x, y),
                Geometry::FlipV => (x, h - 1 - y),
                Geometry::Crop(r) => (x + r.x as u32, y + r.y as u32),
                Geometry::Resize(nw, nh) => (
                    (x as u64 * w as u64 / nw as u64) as u32,
                    (y as u64 * h as u64 / nh as u64) as u32,
                ),
            };
        }
        (x, y)
    }
}

pub struct Editor {
    pub state: State,
    original: State,
    saved: State,
    undo: [State; HISTORY],
    redo: [State; HISTORY],
    undo_len: usize,
    redo_len: usize,
}
impl Editor {
    pub fn new(w: u32, h: u32) -> Self {
        let state = State::new(w, h);
        Self {
            state,
            original: state,
            saved: state,
            undo: [state; HISTORY],
            redo: [state; HISTORY],
            undo_len: 0,
            redo_len: 0,
        }
    }
    pub fn dirty(&self) -> bool {
        self.state != self.saved
    }
    pub fn mark_saved(&mut self) {
        self.saved = self.state;
    }
    pub fn can_undo(&self) -> bool {
        self.undo_len != 0
    }
    pub fn can_redo(&self) -> bool {
        self.redo_len != 0
    }
    fn push(list: &mut [State; HISTORY], len: &mut usize, state: State) {
        if *len == HISTORY {
            list.copy_within(1.., 0);
            *len -= 1;
        }
        list[*len] = state;
        *len += 1;
    }
    /// Record once on slider release, even if hundreds of previews were drawn.
    pub fn commit(&mut self, before: State) {
        if before != self.state {
            Self::push(&mut self.undo, &mut self.undo_len, before);
            self.redo_len = 0;
        }
    }
    pub fn geometry(&mut self, op: Geometry) -> Result<(), &'static str> {
        let before = self.state;
        self.state.geometry(op)?;
        self.commit(before);
        Ok(())
    }
    pub fn undo(&mut self) {
        if self.undo_len > 0 {
            Self::push(&mut self.redo, &mut self.redo_len, self.state);
            self.undo_len -= 1;
            self.state = self.undo[self.undo_len];
        }
    }
    pub fn redo(&mut self) {
        if self.redo_len > 0 {
            Self::push(&mut self.undo, &mut self.undo_len, self.state);
            self.redo_len -= 1;
            self.state = self.redo[self.redo_len];
        }
    }
    pub fn reset(&mut self) {
        let before = self.state;
        self.state = self.original;
        self.commit(before);
    }
}

/// Lookup rebuilt only when adjustment parameters change, never per source pixel.
pub struct Sampler {
    state: State,
    lut: [u8; 256],
}
impl Sampler {
    /// Render at most `out.len()/4` pixels for cooperative export, without
    /// allocating a full output image. Returns the number of BGRA bytes.
    pub fn write_bgra(&self, image: &RgbaImage, start: u32, out: &mut [u8]) -> usize {
        let (w, h) = self.state.size;
        let count = (w.saturating_mul(h).saturating_sub(start) as usize).min(out.len() / 4);
        for i in 0..count {
            let position = start + i as u32;
            out[i * 4..i * 4 + 4]
                .copy_from_slice(&self.pixel(image, position % w, position / w).to_le_bytes());
        }
        count * 4
    }
    pub fn new(state: State) -> Self {
        let mut lut = [0; 256];
        let gamma = libm::powf(4.0, state.adjust[3] as f32 / 100.0);
        for (i, v) in lut.iter_mut().enumerate() {
            let c = (((i as i32 - 128) * (100 + state.adjust[1] as i32) / 100)
                + 128
                + state.adjust[0] as i32 * 255 / 100)
                .clamp(0, 255);
            *v = if state.adjust[3] == 0 {
                c as u8
            } else {
                (libm::powf(c as f32 / 255.0, 1.0 / gamma) * 255.0 + 0.5) as u8
            };
        }
        Self { state, lut }
    }
    /// Geometry, saturation, filters and viewport changes can reuse the LUT.
    pub fn update(&mut self, state: State) {
        if [
            self.state.adjust[0],
            self.state.adjust[1],
            self.state.adjust[3],
        ] == [state.adjust[0], state.adjust[1], state.adjust[3]]
        {
            self.state = state;
        } else {
            *self = Self::new(state);
        }
    }
    fn base(&self, image: &RgbaImage, x: u32, y: u32) -> u32 {
        let (sx, sy) = self.state.source_point(x, y);
        let p = image.pixel(sx, sy);
        let r = self.lut[((p >> 16) & 255) as usize] as i32;
        let g = self.lut[((p >> 8) & 255) as usize] as i32;
        let b = self.lut[(p & 255) as usize] as i32;
        let l = (77 * r + 150 * g + 29 * b) / 256;
        let s = 100 + self.state.adjust[2] as i32;
        pack(
            p,
            l + (r - l) * s / 100,
            l + (g - l) * s / 100,
            l + (b - l) * s / 100,
        )
    }
    pub fn pixel(&self, image: &RgbaImage, x: u32, y: u32) -> u32 {
        let x = x.min(self.state.size.0 - 1);
        let y = y.min(self.state.size.1 - 1);
        let p = self.base(image, x, y);
        let r = ((p >> 16) & 255) as i32;
        let g = ((p >> 8) & 255) as i32;
        let b = (p & 255) as i32;
        match self.state.effect {
            Effect::None => p,
            Effect::Grayscale => {
                let l = (77 * r + 150 * g + 29 * b) / 256;
                pack(p, l, l, l)
            }
            Effect::Sepia => pack(
                p,
                (393 * r + 769 * g + 189 * b) / 1000,
                (349 * r + 686 * g + 168 * b) / 1000,
                (272 * r + 534 * g + 131 * b) / 1000,
            ),
            Effect::Invert => p ^ 0x00ffffff,
            effect => {
                let mut sum = [0i32; 3];
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let nx = (x as i32 + dx).clamp(0, self.state.size.0 as i32 - 1) as u32;
                        let ny = (y as i32 + dy).clamp(0, self.state.size.1 as i32 - 1) as u32;
                        let q = self.base(image, nx, ny);
                        for (i, shift) in [16, 8, 0].iter().enumerate() {
                            sum[i] += ((q >> shift) & 255) as i32;
                        }
                    }
                }
                let rgb = [r, g, b];
                let mut out = [0; 3];
                for i in 0..3 {
                    out[i] = match effect {
                        Effect::Blur => sum[i] / 9,
                        Effect::Sharpen => rgb[i] * 2 - sum[i] / 9,
                        _ => (rgb[i] * 9 - sum[i]).abs(),
                    };
                }
                pack(p, out[0], out[1], out[2])
            }
        }
    }
}
fn pack(alpha: u32, r: i32, g: i32, b: i32) -> u32 {
    (alpha & 0xff000000)
        | ((r.clamp(0, 255) as u32) << 16)
        | ((g.clamp(0, 255) as u32) << 8)
        | b.clamp(0, 255) as u32
}

/// Exact integer fit, including very wide or very tall images.
pub fn fit_rect(area: Rect, size: (u32, u32)) -> Option<Rect> {
    let (w, h) = size;
    if w == 0 || h == 0 || area.w == 0 || area.h == 0 {
        return None;
    }
    let (dw, dh) = if area.w as u64 * h as u64 <= area.h as u64 * w as u64 {
        (
            area.w,
            ((area.w as u64 * h as u64 / w as u64) as u32)
                .max(1)
                .min(area.h),
        )
    } else {
        (
            ((area.h as u64 * w as u64 / h as u64) as u32)
                .max(1)
                .min(area.w),
            area.h,
        )
    };
    Some(Rect::new(
        area.x + ((area.w - dw) / 2) as i32,
        area.y + ((area.h - dh) / 2) as i32,
        dw,
        dh,
    ))
}

#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub zoom: u32,
    pub fit: bool,
    pub pan: Point,
}
impl Default for Viewport {
    fn default() -> Self {
        Self {
            zoom: 1000,
            fit: true,
            pan: Point::new(0, 0),
        }
    }
}
impl Viewport {
    pub fn rect(&self, area: Rect, size: (u32, u32)) -> Rect {
        let (w, h) = size;
        if area.w == 0 || area.h == 0 || w == 0 || h == 0 {
            return Rect::new(area.x, area.y, 0, 0);
        }
        if self.fit {
            return fit_rect(area, size).unwrap();
        }
        let z = self.zoom;
        let dw = (w as u64 * z as u64 / 1000).max(1) as u32;
        let dh = (h as u64 * z as u64 / 1000).max(1) as u32;
        Rect::new(
            area.x + (area.w as i32 - dw as i32) / 2 + self.pan.x,
            area.y + (area.h as i32 - dh as i32) / 2 + self.pan.y,
            dw,
            dh,
        )
    }
    pub fn fit_zoom(area: Rect, size: (u32, u32)) -> u32 {
        if size.0 == 0 || size.1 == 0 {
            return 1000;
        }
        ((area.w as u64 * 1000 / size.0 as u64).min(area.h as u64 * 1000 / size.1 as u64) as u32)
            .max(1)
    }
    pub fn clamp(&mut self, area: Rect, size: (u32, u32)) {
        if self.fit {
            self.pan = Point::new(0, 0);
            return;
        }
        let r = Self {
            pan: Point::new(0, 0),
            ..*self
        }
        .rect(area, size);
        self.pan.x = if r.w > area.w {
            self.pan.x.clamp(area.right() - r.right(), area.x - r.x)
        } else {
            0
        };
        self.pan.y = if r.h > area.h {
            self.pan.y.clamp(area.bottom() - r.bottom(), area.y - r.y)
        } else {
            0
        };
    }
    pub fn zoom_at(&mut self, area: Rect, size: (u32, u32), cursor: Point, increase: bool) {
        let z = if self.fit {
            Self::fit_zoom(area, size)
        } else {
            self.zoom
        };
        let zoom = if increase {
            z.saturating_mul(5) / 4
        } else {
            z * 4 / 5
        };
        self.zoom_to(area, size, cursor, zoom);
    }
    /// Wheel deltas use 120 units per detent. Fractional detents make
    /// proportionally smaller steps; no animation or periodic zoom work.
    pub fn wheel_zoom(&mut self, area: Rect, size: (u32, u32), cursor: Point, delta: i16) -> bool {
        if delta == 0 {
            return false;
        }
        let z = if self.fit {
            Self::fit_zoom(area, size)
        } else {
            self.zoom
        };
        let scale = libm::powf(1.25, -(delta.clamp(-1200, 1200) as f32) / 120.0);
        let zoom = ((z as f32 * scale + 0.5) as u32).clamp(10, 16000);
        if zoom == z && !self.fit {
            return false;
        }
        self.zoom_to(area, size, cursor, zoom);
        true
    }
    fn zoom_to(&mut self, area: Rect, size: (u32, u32), cursor: Point, zoom: u32) {
        let old = self.rect(area, size);
        if old.w == 0 || old.h == 0 {
            return;
        }
        self.zoom = zoom.clamp(10, 16000);
        self.fit = false;
        self.pan = Point::new(0, 0);
        let new = self.rect(area, size);
        self.pan.x =
            cursor.x - new.x - ((cursor.x - old.x) as i64 * new.w as i64 / old.w as i64) as i32;
        self.pan.y =
            cursor.y - new.y - ((cursor.y - old.y) as i64 * new.h as i64 / old.h as i64) as i32;
        self.clamp(area, size);
    }
}
pub fn image_point(dst: Rect, size: (u32, u32), p: Point) -> Point {
    if dst.w == 0 || dst.h == 0 {
        return Point::new(0, 0);
    }
    Point::new(
        (((p.x - dst.x) as i64 * size.0 as i64 / dst.w as i64).clamp(0, size.0 as i64)) as i32,
        (((p.y - dst.y) as i64 * size.1 as i64 / dst.h as i64).clamp(0, size.1 as i64)) as i32,
    )
}
/// Drag endpoints are pixel boundaries; fixed ratios stay inside both drag extents.
pub fn crop_rect(a: Point, b: Point, ratio: usize) -> Rect {
    let mut w = (b.x - a.x).unsigned_abs();
    let mut h = (b.y - a.y).unsigned_abs();
    if let Some((rw, rh)) = match ratio {
        1 => Some((1u32, 1u32)),
        2 => Some((4, 3)),
        3 => Some((16, 9)),
        _ => None,
    } {
        let units = (w / rw).min(h / rh);
        w = units * rw;
        h = units * rh;
    }
    Rect::new(
        if b.x < a.x { a.x - w as i32 } else { a.x },
        if b.y < a.y { a.y - h as i32 } else { a.y },
        w,
        h,
    )
}

/// Original comparison is letterboxed in the edited display rectangle. This
/// preserves the viewport without interpreting edited coordinates as source ones.
pub fn comparison_rect(edited: Rect, original: (u32, u32)) -> Rect {
    fit_rect(edited, original).unwrap_or(Rect::new(edited.x, edited.y, 0, 0))
}

pub fn valid_crop(r: Rect, size: (u32, u32)) -> bool {
    r.w >= size.0.min(4)
        && r.h >= size.1.min(4)
        && r.w != 0
        && r.h != 0
        && r.x >= 0
        && r.y >= 0
        && r.x as u64 + r.w as u64 <= size.0 as u64
        && r.y as u64 + r.h as u64 <= size.1 as u64
}

/// Move or resize an existing selection. Edge bits: left=1, right=2,
/// top=4, bottom=8; 16 moves. Fixed-ratio edges keep the other axis centered.
pub fn drag_crop(
    r: Rect,
    start: Point,
    end: Point,
    edges: u8,
    size: (u32, u32),
    ratio: usize,
) -> Rect {
    if r.w == 0
        || r.h == 0
        || r.x < 0
        || r.y < 0
        || r.x as u64 + r.w as u64 > size.0 as u64
        || r.y as u64 + r.h as u64 > size.1 as u64
    {
        return r;
    }
    let (w, h) = (size.0 as i32, size.1 as i32);
    let dx = end.x - start.x;
    let dy = end.y - start.y;
    if edges == 16 {
        return Rect::new(
            (r.x + dx).clamp(0, w - r.w as i32),
            (r.y + dy).clamp(0, h - r.h as i32),
            r.w,
            r.h,
        );
    }
    let mut l = r.x;
    let mut t = r.y;
    let mut right = r.right();
    let mut bottom = r.bottom();
    let min_w = r.w.min(size.0.min(4)).max(1) as i32;
    let min_h = r.h.min(size.1.min(4)).max(1) as i32;
    if edges & 1 != 0 {
        l = (l + dx).clamp(0, right - min_w);
    }
    if edges & 2 != 0 {
        right = (right + dx).clamp(l + min_w, w);
    }
    if edges & 4 != 0 {
        t = (t + dy).clamp(0, bottom - min_h);
    }
    if edges & 8 != 0 {
        bottom = (bottom + dy).clamp(t + min_h, h);
    }
    let (rw, rh) = match ratio {
        1 => (1, 1),
        2 => (4, 3),
        3 => (16, 9),
        _ => return Rect::new(l, t, (right - l) as u32, (bottom - t) as u32),
    };
    let horizontal = edges & 3 != 0;
    let vertical = edges & 12 != 0;
    let cx = r.x + r.w as i32 / 2;
    let cy = r.y + r.h as i32 / 2;
    let max_w = if horizontal {
        right - l
    } else {
        2 * cx.min(w - cx)
    };
    let max_h = if vertical {
        bottom - t
    } else {
        2 * cy.min(h - cy)
    };
    let units = (max_w / rw).min(max_h / rh);
    if units < 1 {
        return r;
    }
    let nw = units * rw;
    let nh = units * rh;
    let x = if edges & 1 != 0 {
        right - nw
    } else if horizontal {
        l
    } else {
        cx - nw / 2
    };
    let y = if edges & 4 != 0 {
        bottom - nh
    } else if vertical {
        t
    } else {
        cy - nh / 2
    };
    let next = Rect::new(x, y, nw as u32, nh as u32);
    if valid_crop(next, size) {
        next
    } else {
        r
    }
}

/// At most 4096 stratified samples, always taken from edited output pixels.
/// Transparent pixels do not contribute; partial alpha weights coverage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Histogram {
    pub bins: [u32; 256],
    pub samples: u32,
}
impl Histogram {
    pub fn compute(image: &RgbaImage, state: State) -> Self {
        let mut out = Self {
            bins: [0; 256],
            samples: 0,
        };
        let (w, h) = state.size;
        if w == 0 || h == 0 {
            return out;
        }
        let sampler = Sampler::new(state);
        let nx = w.min(64);
        let ny = h.min(64);
        for gy in 0..ny {
            let y = ((2 * gy + 1) as u64 * h as u64 / (2 * ny) as u64) as u32;
            for gx in 0..nx {
                let x = ((2 * gx + 1) as u64 * w as u64 / (2 * nx) as u64) as u32;
                let p = sampler.pixel(image, x, y);
                let l = (77 * ((p >> 16) & 255) + 150 * ((p >> 8) & 255) + 29 * (p & 255)) / 256;
                out.bins[l as usize] += p >> 24;
                out.samples += 1;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn image() -> RgbaImage {
        RgbaImage {
            width: 3,
            height: 2,
            pixels: vec![
                0x10102030, 0x20405060, 0x30708090, 0x40a0b0c0, 0x50d0e0f0, 0x60ffffff,
            ],
        }
    }
    fn pixels(s: State) -> alloc::vec::Vec<u32> {
        let i = image();
        let sampler = Sampler::new(s);
        (0..s.size.1)
            .flat_map(|y| (0..s.size.0).map(move |x| (x, y)))
            .map(|(x, y)| sampler.pixel(&i, x, y))
            .collect()
    }
    #[test]
    fn original_comparison_letterboxes_without_reusing_crop_coordinates() {
        assert_eq!(
            comparison_rect(Rect::new(-50, 20, 200, 400), (600, 300)),
            Rect::new(-50, 170, 200, 100)
        );
        assert_eq!(
            comparison_rect(Rect::new(3, 4, 0, 0), (1, 1)),
            Rect::new(3, 4, 0, 0)
        );
    }
    #[test]
    fn crop_handles_stay_bounded_and_enforce_ratios() {
        let r = Rect::new(30, 30, 32, 18);
        for ratio in 0..=3 {
            for edges in [1, 2, 4, 8, 5, 6, 9, 10, 16] {
                for end in [
                    Point::new(-200, -200),
                    Point::new(90, 90),
                    Point::new(1000, 1000),
                ] {
                    let next = drag_crop(r, Point::new(40, 40), end, edges, (100, 100), ratio);
                    assert!(
                        next.x >= 0 && next.y >= 0 && next.right() <= 100 && next.bottom() <= 100
                    );
                    assert!(next.w > 0 && next.h > 0);
                    // An impossible ratio resize retains the previous selection.
                    if edges != 16 && next != r && ratio != 0 {
                        let (rw, rh) = [(1, 1), (1, 1), (4, 3), (16, 9)][ratio];
                        assert_eq!(next.w * rh, next.h * rw);
                    }
                }
            }
        }
        assert_eq!(
            drag_crop(
                r,
                Point::new(0, 0),
                Point::new(-100, 100),
                16,
                (100, 100),
                0
            ),
            Rect::new(0, 82, 32, 18)
        );
        assert!(!valid_crop(Rect::new(0, 0, 1, 1), (100, 100)));
        assert!(valid_crop(Rect::new(0, 0, 1, 1), (1, 1)));
    }
    #[test]
    fn histogram_samples_edited_geometry_effects_and_alpha() {
        let image = RgbaImage {
            width: 3,
            height: 1,
            pixels: vec![0xff000000, 0x80ffffff, 0x00ffffff],
        };
        let mut state = State::new(3, 1);
        let h = Histogram::compute(&image, state);
        assert_eq!(h.samples, 3);
        assert_eq!(h.bins[0], 255);
        assert_eq!(h.bins[255], 128);
        assert_eq!(h.bins.iter().sum::<u32>(), 383);
        state
            .geometry(Geometry::Crop(Rect::new(0, 0, 1, 1)))
            .unwrap();
        state.effect = Effect::Invert;
        let h = Histogram::compute(&image, state);
        assert_eq!(h.bins[255], 255);
        assert_eq!(h.bins[0], 0);
        state.geometry(Geometry::Resize(2048, 1024)).unwrap();
        let h = Histogram::compute(&image, state);
        assert_eq!(h.samples, 4096);
        assert_eq!(h.bins[255], 4096 * 255);
    }
    #[test]
    fn wheel_fractional_delta_anchor_and_limits() {
        let area = Rect::new(0, 0, 400, 300);
        let size = (800, 600);
        let p = Point::new(210, 160);
        let mut view = Viewport {
            fit: false,
            ..Viewport::default()
        };
        let before = image_point(view.rect(area, size), size, p);
        assert!(view.wheel_zoom(area, size, p, -30));
        assert!(view.zoom > 1000 && view.zoom < 1250);
        let after = image_point(view.rect(area, size), size, p);
        assert!((before.x - after.x).abs() <= 1 && (before.y - after.y).abs() <= 1);
        assert!(view.wheel_zoom(area, size, p, 30));
        assert!((view.zoom as i32 - 1000).abs() <= 1);
        view.zoom = 16000;
        assert!(!view.wheel_zoom(area, size, p, -120));
        view.zoom = 10;
        assert!(!view.wheel_zoom(area, size, p, 120));
        view.pan = Point::new(500, -500);
        view.clamp(area, size);
        assert_eq!(view.pan, Point::new(0, 0));
    }
    #[test]
    fn checkpoints_new_branches_noops_and_history_memory_bound() {
        let mut e = Editor::new(100, 100);
        e.geometry(Geometry::Crop(Rect::new(10, 10, 40, 40)))
            .unwrap();
        e.geometry(Geometry::RotateRight).unwrap();
        e.geometry(Geometry::FlipH).unwrap();
        e.geometry(Geometry::Resize(80, 80)).unwrap();
        e.mark_saved();
        let saved = e.state;
        for _ in 0..4 {
            e.undo();
        }
        for _ in 0..4 {
            e.redo();
        }
        assert_eq!(e.state, saved);
        assert!(!e.dirty());
        e.undo();
        let b = e.state;
        e.commit(b);
        assert!(e.can_redo());
        e.state.adjust[0] = 1;
        e.commit(b);
        assert!(!e.can_redo());
        assert!(e.dirty());
        for n in 0..100 {
            let b = e.state;
            e.state.adjust[1] = n;
            e.commit(b);
        }
        assert_eq!(e.undo_len, HISTORY);
        assert!(core::mem::size_of::<Editor>() < 64 * 1024);
    }
    #[test]
    fn streaming_export_roundtrips_geometry_adjustments_and_alpha() {
        let image = image();
        let mut state = State::new(3, 2);
        state.geometry(Geometry::RotateRight).unwrap();
        state
            .geometry(Geometry::Crop(Rect::new(0, 1, 2, 2)))
            .unwrap();
        state.geometry(Geometry::Resize(4, 4)).unwrap();
        state.adjust = [10, 20, -10, 25];
        state.effect = Effect::Sepia;
        let sampler = Sampler::new(state);
        let mut bgra = alloc::vec::Vec::new();
        let mut position = 0;
        let mut chunk = [0; 12];
        loop {
            let n = sampler.write_bgra(&image, position, &mut chunk);
            if n == 0 {
                break;
            }
            bgra.extend_from_slice(&chunk[..n]);
            position += (n / 4) as u32;
        }
        assert_eq!(position, 16);
        for header in [
            sun_img::simg_v2::encode_raw_header(4, 4).unwrap().to_vec(),
            sun_img::encode_tga_rgba32_header(4, 4).unwrap().to_vec(),
        ] {
            let mut file = header;
            file.extend_from_slice(&bgra);
            let decoded = sun_img::decode_image(&file).unwrap();
            assert_eq!((decoded.width, decoded.height), state.size);
            for y in 0..4 {
                for x in 0..4 {
                    let p = sampler.pixel(&image, x, y);
                    let i = ((y * 4 + x) * 4) as usize;
                    assert_eq!(
                        &decoded.pixels[i..i + 4],
                        &[(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]
                    );
                }
            }
        }
        assert!(sun_img::simg_v2::encode_raw_header(0, 1).is_err());
        assert!(sun_img::encode_tga_rgba32_header(65536, 1).is_err());
    }
    #[test]
    fn rotation_flip_and_alpha() {
        let original = image().pixels;
        let mut s = State::new(3, 2);
        s.geometry(Geometry::RotateRight).unwrap();
        assert_eq!(s.size, (2, 3));
        assert_eq!(
            pixels(s),
            vec![
                original[3],
                original[0],
                original[4],
                original[1],
                original[5],
                original[2]
            ]
        );
        s.geometry(Geometry::RotateLeft).unwrap();
        assert_eq!(pixels(s), original);
        let mut s = State::new(3, 2);
        s.geometry(Geometry::FlipH).unwrap();
        assert_eq!(
            pixels(s),
            vec![
                original[2],
                original[1],
                original[0],
                original[5],
                original[4],
                original[3]
            ]
        );
        s.geometry(Geometry::FlipV).unwrap();
        assert_eq!(
            pixels(s),
            original.into_iter().rev().collect::<alloc::vec::Vec<_>>()
        );
    }
    #[test]
    fn neutral_adjustments_and_effect_alpha() {
        let mut s = State::new(3, 2);
        assert_eq!(pixels(s), image().pixels);
        for effect in [
            Effect::None,
            Effect::Grayscale,
            Effect::Sepia,
            Effect::Invert,
            Effect::Blur,
            Effect::Sharpen,
            Effect::Edges,
        ] {
            s.effect = effect;
            s.adjust = [20, 30, -50, 40];
            for (a, b) in pixels(s).iter().zip(image().pixels) {
                assert_eq!(a >> 24, b >> 24);
            }
        }
        s = State::new(3, 2);
        s.adjust[0] = 100;
        assert_eq!(pixels(s)[0] & 0xffffff, 0xffffff);
        s.adjust = [0, 0, -100, 0];
        let p = pixels(s)[0];
        assert_eq!((p >> 16) & 255, p & 255);
        s.adjust = [0, 0, 0, 50];
        assert!(pixels(s)[0] & 255 > 0x30);
    }
    #[test]
    fn crop_under_zoom_pan_and_ratios() {
        let dst = Rect::new(-100, 40, 600, 400);
        let size = (300, 200);
        let a = image_point(dst, size, Point::new(0, 80));
        let b = image_point(dst, size, Point::new(200, 240));
        assert_eq!(crop_rect(a, b, 0), Rect::new(50, 20, 100, 80));
        assert_eq!(crop_rect(b, a, 1), Rect::new(70, 20, 80, 80));
        for ratio in 1..=3 {
            let r = crop_rect(Point::new(0, 0), Point::new(111, 89), ratio);
            let (w, h) = match ratio {
                1 => (1, 1),
                2 => (4, 3),
                _ => (16, 9),
            };
            assert_eq!(r.w * h, r.h * w);
            assert!(r.w <= 111 && r.h <= 89);
        }
        let mut s = State::new(3, 2);
        s.geometry(Geometry::Crop(Rect::new(1, 0, 2, 1))).unwrap();
        assert_eq!(pixels(s), image().pixels[1..3]);
    }
    #[test]
    fn history_and_transactions() {
        let mut e = Editor::new(3, 2);
        let before = e.state;
        e.state.adjust[0] = 20;
        e.state.adjust[0] = 30;
        e.commit(before);
        e.mark_saved();
        e.geometry(Geometry::RotateRight).unwrap();
        assert!(e.dirty());
        e.undo();
        assert!(!e.dirty());
        assert_eq!(e.state.adjust[0], 30);
        e.undo();
        assert_eq!(e.state, before);
        e.redo();
        assert_eq!(e.state.adjust[0], 30);
        e.reset();
        assert_eq!(e.state, before);
        e.undo();
        assert_eq!(e.state.adjust[0], 30);
        for n in 0..100 {
            let b = e.state;
            e.state.adjust[0] = n;
            e.commit(b);
        }
        assert_eq!(e.undo_len, HISTORY);
    }
    #[test]
    fn input_limits_account_for_codec_scratch_and_allow_full_hd_geometry() {
        assert_eq!(dimensions(1312, 816), Ok(()));
        assert_eq!(dimensions(1920, 1080), Ok(()));
        assert_eq!(dimensions(2048, 1024), Ok(()));
        let mut png = alloc::vec![0;33];
        png[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        png[8..16].copy_from_slice(b"\0\0\0\rIHDR");
        png[16..20].copy_from_slice(&1312u32.to_be_bytes());
        png[20..24].copy_from_slice(&816u32.to_be_bytes());
        assert!(inspect_input(&png).unwrap_err().contains("scratch memory"));
        let mut resized = State::new(1312, 816);
        resized.geometry(Geometry::Resize(1920, 1080)).unwrap();
        assert_eq!(resized.size, (1920, 1080));
    }
    #[test]
    fn validation_and_resize() {
        for (w, h) in [(0, 1), (1, 0), (u32::MAX, 2), (8192, 8192), (2049, 1024)] {
            assert!(dimensions(w, h).is_err());
        }
        let mut s = State::new(3, 2);
        let before = s;
        assert!(s.geometry(Geometry::Crop(Rect::new(2, 0, 2, 1))).is_err());
        assert_eq!(s, before);
        s.geometry(Geometry::Resize(6, 4)).unwrap();
        let p = pixels(s);
        assert_eq!(p[0], p[1]);
        assert_eq!(p[0], p[6]);
    }
    #[test]
    fn pan_can_reach_both_edges_with_odd_overflow() {
        let area = Rect::new(7, 9, 400, 300);
        let size = (401, 301);
        let mut v = Viewport {
            fit: false,
            pan: Point::new(-100, -100),
            ..Viewport::default()
        };
        v.clamp(area, size);
        let r = v.rect(area, size);
        assert_eq!(r.right(), area.right());
        assert_eq!(r.bottom(), area.bottom());
        v.pan = Point::new(100, 100);
        v.clamp(area, size);
        let r = v.rect(area, size);
        assert_eq!(r.x, area.x);
        assert_eq!(r.y, area.y);
    }
    #[test]
    fn viewport_anchor_and_bounds() {
        let area = Rect::new(0, 0, 400, 300);
        let size = (800, 600);
        let mut v = Viewport {
            fit: false,
            ..Viewport::default()
        };
        let p = Point::new(230, 180);
        let a = image_point(v.rect(area, size), size, p);
        v.zoom_at(area, size, p, true);
        let b = image_point(v.rect(area, size), size, p);
        assert!((a.x - b.x).abs() <= 1 && (a.y - b.y).abs() <= 1);
        v.pan = Point::new(i32::MAX, i32::MIN);
        v.clamp(area, size);
        let r = v.rect(area, size);
        assert!(r.x <= area.x && r.right() >= area.right());
        assert!(r.y <= area.y && r.bottom() >= area.bottom());
    }
}
