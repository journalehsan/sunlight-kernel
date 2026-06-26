//! Geometry primitives: `Rect`, `Point`, `Size`.

/// 2-D point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self { Self { x, y } }

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
    pub const fn new(w: u32, h: u32) -> Self { Self { w, h } }
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
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self { Self { x, y, w, h } }

    #[inline]
    pub fn from_point_size(p: Point, s: Size) -> Self {
        Self::new(p.x, p.y, s.w, s.h)
    }

    #[inline]
    pub fn right(self) -> i32 { self.x + self.w as i32 }

    #[inline]
    pub fn bottom(self) -> i32 { self.y + self.h as i32 }

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
    pub fn size(self) -> Size { Size::new(self.w, self.h) }

    #[inline]
    pub fn origin(self) -> Point { Point::new(self.x, self.y) }
}
