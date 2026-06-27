use sunlight_ui::Rect;

/// Fixed-capacity dirty region accumulator.
///
/// Callers mark rects that need repainting. `redraw_scene` reads the list to
/// decide how much of the back buffer to present to the framebuffer.
pub struct DirtyList {
    pub rects: [Rect; 8],
    pub count: usize,
    /// When true the entire screen must be presented (list overflowed or a
    /// whole-screen event occurred).
    pub full: bool,
}

impl DirtyList {
    pub const fn new() -> Self {
        Self {
            rects: [Rect::new(0, 0, 0, 0); 8],
            count: 0,
            full: false,
        }
    }

    pub fn mark_full(&mut self) {
        self.full = true;
        self.count = 0;
    }

    pub fn mark(&mut self, r: Rect) {
        if self.full || r.w == 0 || r.h == 0 {
            return;
        }
        // Merge with any overlapping-or-adjacent existing rect.
        for i in 0..self.count {
            if overlaps_or_adjacent(self.rects[i], r) {
                self.rects[i] = union_rect(self.rects[i], r);
                return;
            }
        }
        if self.count < 8 {
            self.rects[self.count] = r;
            self.count += 1;
        } else {
            self.mark_full();
        }
    }

    pub fn clear(&mut self) {
        self.full = false;
        self.count = 0;
    }

    pub fn needs_full_present(&self) -> bool {
        self.full || self.count == 0
    }
}

fn overlaps_or_adjacent(a: Rect, b: Rect) -> bool {
    a.x <= b.right() && b.x <= a.right() && a.y <= b.bottom() && b.y <= a.bottom()
}

fn union_rect(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = a.right().max(b.right());
    let y1 = a.bottom().max(b.bottom());
    Rect::new(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32)
}
