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
        // Merge transitively with every overlapping-or-adjacent rectangle.
        // Merging only the first match leaves chains such as A<->new<->B as
        // two GPU transfers even though their union is one dirty region.
        let mut merged = r;
        let mut i = 0;
        while i < self.count {
            if overlaps_or_adjacent(self.rects[i], merged) {
                merged = union_rect(self.rects[i], merged);
                self.count -= 1;
                self.rects[i] = self.rects[self.count];
                // The larger union may now touch an earlier rectangle. Start
                // over; capacity is eight, so this stays tightly bounded.
                i = 0;
            } else {
                i += 1;
            }
        }
        if self.count < 8 {
            self.rects[self.count] = merged;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_touching_chain_into_one_transfer_region() {
        let mut dirty = DirtyList::new();
        dirty.mark(Rect::new(0, 0, 10, 10));
        dirty.mark(Rect::new(20, 0, 10, 10));
        dirty.mark(Rect::new(10, 0, 10, 10));

        assert_eq!(dirty.count, 1);
        assert_eq!(dirty.rects[0], Rect::new(0, 0, 30, 10));
        assert!(!dirty.full);
    }

    #[test]
    fn keeps_separated_regions_separate() {
        let mut dirty = DirtyList::new();
        dirty.mark(Rect::new(0, 0, 8, 8));
        dirty.mark(Rect::new(100, 100, 8, 8));

        assert_eq!(dirty.count, 2);
        assert!(!dirty.full);
    }
}
