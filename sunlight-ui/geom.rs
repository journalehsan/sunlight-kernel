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
