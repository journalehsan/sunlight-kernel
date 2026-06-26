use alloc::vec::Vec;
use crate::geom::{Point, Rect};

/// Vertical box layout — arranges children top-to-bottom.
#[derive(Debug, Clone, Copy)]
pub struct VBox {
    pub rect: Rect,
    pub spacing: u32,
}

impl VBox {
    pub fn new(rect: Rect) -> Self {
        Self { rect, spacing: 4 }
    }

    pub fn with_spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Compute rects for children given each child's height.
    /// Children are top-aligned within the parent rect.
    pub fn layout(&self, child_heights: &[u32]) -> Vec<Rect> {
        let mut y = self.rect.y;
        let mut rects = Vec::with_capacity(child_heights.len());
        for &h in child_heights {
            let r = Rect::new(self.rect.x, y, self.rect.w, h);
            rects.push(r);
            y += h as i32 + self.spacing as i32;
        }
        rects
    }

    /// Divide available height equally among `n` children.
    pub fn layout_equal(&self, n: usize) -> Vec<Rect> {
        if n == 0 {
            return Vec::new();
        }
        let total_spacing = self.spacing.saturating_mul(n.saturating_sub(1) as u32);
        let child_h = (self.rect.h.saturating_sub(total_spacing)) / n as u32;
        let heights = alloc::vec![child_h; n];
        self.layout(&heights)
    }

    /// Which child index contains `(x, y)`?
    /// `child_sizes` is `(width, height)` pairs.
    pub fn hit_test(&self, x: i32, y: i32, child_sizes: &[(u32, u32)]) -> Option<usize> {
        let mut cy = self.rect.y;
        for (i, &(w, h)) in child_sizes.iter().enumerate() {
            let r = Rect::new(self.rect.x, cy, self.rect.w.min(w), h);
            if r.contains(Point::new(x, y)) {
                return Some(i);
            }
            cy += h as i32 + self.spacing as i32;
        }
        None
    }
}

/// Horizontal box layout — arranges children left-to-right.
#[derive(Debug, Clone, Copy)]
pub struct HBox {
    pub rect: Rect,
    pub spacing: u32,
}

impl HBox {
    pub fn new(rect: Rect) -> Self {
        Self { rect, spacing: 4 }
    }

    pub fn with_spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Compute rects for children given each child's width.
    /// Children are left-aligned within the parent rect.
    pub fn layout(&self, child_widths: &[u32]) -> Vec<Rect> {
        let mut x = self.rect.x;
        let mut rects = Vec::with_capacity(child_widths.len());
        for &w in child_widths {
            let r = Rect::new(x, self.rect.y, w, self.rect.h);
            rects.push(r);
            x += w as i32 + self.spacing as i32;
        }
        rects
    }

    /// Divide available width equally among `n` children.
    pub fn layout_equal(&self, n: usize) -> Vec<Rect> {
        if n == 0 {
            return Vec::new();
        }
        let total_spacing = self.spacing.saturating_mul(n.saturating_sub(1) as u32);
        let child_w = (self.rect.w.saturating_sub(total_spacing)) / n as u32;
        let widths = alloc::vec![child_w; n];
        self.layout(&widths)
    }

    /// Which child index contains `(x, y)`?
    /// `child_sizes` is `(width, height)` pairs.
    pub fn hit_test(&self, x: i32, y: i32, child_sizes: &[(u32, u32)]) -> Option<usize> {
        let mut cx = self.rect.x;
        for (i, &(w, h)) in child_sizes.iter().enumerate() {
            let r = Rect::new(cx, self.rect.y, w, self.rect.h.min(h));
            if r.contains(Point::new(x, y)) {
                return Some(i);
            }
            cx += w as i32 + self.spacing as i32;
        }
        None
    }
}
