//! Geometry primitives: `Rect`, `Point`, `Size`.

/// 2-D point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    #[inline]
    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy)
    }
}

/// Width × height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub w: u32,
    pub h: u32,
}

impl Size {
    #[inline]
    pub const fn new(w: u32, h: u32) -> Self {
        Self { w, h }
    }
}

/// Axis-aligned rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    #[inline]
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    #[inline]
    pub fn from_point_size(p: Point, s: Size) -> Self {
        Self::new(p.x, p.y, s.w, s.h)
    }

    #[inline]
    pub fn right(self) -> i32 {
        self.x + self.w as i32
    }

    #[inline]
    pub fn bottom(self) -> i32 {
        self.y + self.h as i32
    }

    #[inline]
    pub fn contains(self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    /// Shrink all sides by `amount`.
    #[inline]
    pub fn inset(self, amount: i32) -> Self {
        let a2 = amount * 2;
        Self::new(
            self.x + amount,
            self.y + amount,
            (self.w as i32 - a2).max(0) as u32,
            (self.h as i32 - a2).max(0) as u32,
        )
    }

    /// Intersection of two rects — returns `None` if they don't overlap.
    pub fn intersect(self, other: Rect) -> Option<Rect> {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = self.right().min(other.right());
        let y2 = self.bottom().min(other.bottom());
        if x2 > x1 && y2 > y1 {
            Some(Rect::new(x1, y1, (x2 - x1) as u32, (y2 - y1) as u32))
        } else {
            None
        }
    }

    /// Translate by a point offset.
    #[inline]
    pub fn translate(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    #[inline]
    pub fn size(self) -> Size {
        Size::new(self.w, self.h)
    }

    #[inline]
    pub fn origin(self) -> Point {
        Point::new(self.x, self.y)
    }
}

// ---------------------------------------------------------------------------
// Grid layout — 10-block row system
// ---------------------------------------------------------------------------

/// A single grid row that divides its width into 10 equal blocks.
///
/// Each column claims 1–10 blocks of space. Gaps between columns are taken
/// off the top before the block unit is computed, so block widths are always
/// consistent regardless of how many columns share the row.
///
/// # Example
///
/// ```ignore
/// // A row rect 600px wide, two columns: 3-blocks + 7-blocks
/// let row_rect = Rect::new(0, 40, 600, 28);
/// let grid_row = GridRow::new(row_rect).with_gap(8);
/// for (i, col_rect) in grid_row.layout(&[3, 7]).enumerate() {
///     // col 0: x=0,  w=172  (3 * (600-8)/10  = 3*59 = 177 — exact depends on integer div)
///     // col 1: x=180, w=408
/// }
/// ```
///
/// # Notes on breakpoints
///
/// TODO: future versions may support responsive breakpoints (e.g. collapse
/// columns on narrow screens). Not implemented — keep the API here stable.
#[derive(Debug, Clone, Copy)]
pub struct GridRow {
    pub rect: Rect,
    /// Pixel gap between adjacent columns (default 4).
    pub gap: u32,
}

impl GridRow {
    pub const fn new(rect: Rect) -> Self {
        Self { rect, gap: 4 }
    }

    pub const fn with_gap(mut self, gap: u32) -> Self {
        self.gap = gap;
        self
    }

    /// Return an iterator that yields one [`Rect`] per entry in `blocks`.
    ///
    /// Each entry in `blocks` is the number of 1/10-width units that column
    /// should occupy (clamped to 1–10). Blocks exceeding 10 in total simply
    /// cause columns to overflow the row — callers are responsible for keeping
    /// the sum ≤ 10.
    pub fn layout<'a>(&self, blocks: &'a [u32]) -> GridColIter<'a> {
        let col_count = blocks.len() as u32;
        let total_gap = if col_count > 1 {
            self.gap * (col_count - 1)
        } else {
            0
        };
        let available = self.rect.w.saturating_sub(total_gap);
        // Integer block unit; the last column can absorb rounding slack if
        // the caller uses blocks that exactly sum to 10.
        let block_unit = available / 10;
        GridColIter {
            rect: self.rect,
            blocks,
            block_unit,
            gap: self.gap,
            index: 0,
            cursor: self.rect.x,
        }
    }
}

/// Iterator produced by [`GridRow::layout`].
#[derive(Debug, Clone)]
pub struct GridColIter<'a> {
    rect: Rect,
    blocks: &'a [u32],
    block_unit: u32,
    gap: u32,
    index: usize,
    cursor: i32,
}

impl<'a> Iterator for GridColIter<'a> {
    type Item = Rect;

    fn next(&mut self) -> Option<Self::Item> {
        let &b = self.blocks.get(self.index)?;
        let blocks = b.clamp(1, 10);
        let w = blocks * self.block_unit;
        let r = Rect::new(self.cursor, self.rect.y, w, self.rect.h);
        self.index += 1;
        self.cursor += w as i32 + self.gap as i32;
        Some(r)
    }
}

// ---------------------------------------------------------------------------
// Classic axis-based layouts (VBox / HBox) — unchanged
// ---------------------------------------------------------------------------

/// Vertical box layout iterator.
#[derive(Debug, Clone, Copy)]
pub struct VBox {
    pub rect: Rect,
    pub spacing: u32,
}

impl VBox {
    pub const fn new(rect: Rect) -> Self {
        Self { rect, spacing: 4 }
    }

    pub const fn with_spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn layout<'a>(self, heights: &'a [u32]) -> AxisLayout<'a> {
        AxisLayout::vertical(self.rect, self.spacing, heights)
    }
}

/// Horizontal box layout iterator.
#[derive(Debug, Clone, Copy)]
pub struct HBox {
    pub rect: Rect,
    pub spacing: u32,
}

impl HBox {
    pub const fn new(rect: Rect) -> Self {
        Self { rect, spacing: 4 }
    }

    pub const fn with_spacing(mut self, spacing: u32) -> Self {
        self.spacing = spacing;
        self
    }

    pub fn layout<'a>(self, widths: &'a [u32]) -> AxisLayout<'a> {
        AxisLayout::horizontal(self.rect, self.spacing, widths)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AxisLayout<'a> {
    rect: Rect,
    spacing: u32,
    dims: &'a [u32],
    index: usize,
    cursor: i32,
    vertical: bool,
}

impl<'a> AxisLayout<'a> {
    const fn vertical(rect: Rect, spacing: u32, dims: &'a [u32]) -> Self {
        Self {
            rect,
            spacing,
            dims,
            index: 0,
            cursor: rect.y,
            vertical: true,
        }
    }

    const fn horizontal(rect: Rect, spacing: u32, dims: &'a [u32]) -> Self {
        Self {
            rect,
            spacing,
            dims,
            index: 0,
            cursor: rect.x,
            vertical: false,
        }
    }
}

impl<'a> Iterator for AxisLayout<'a> {
    type Item = Rect;

    fn next(&mut self) -> Option<Self::Item> {
        let dim = *self.dims.get(self.index)?;
        let rect = if self.vertical {
            Rect::new(self.rect.x, self.cursor, self.rect.w, dim)
        } else {
            Rect::new(self.cursor, self.rect.y, dim, self.rect.h)
        };
        self.index += 1;
        self.cursor += dim as i32 + self.spacing as i32;
        Some(rect)
    }
}
