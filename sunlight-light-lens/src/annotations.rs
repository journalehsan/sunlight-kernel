//! Bounded vector geometry. Shared objects keep history independent of image size.
use crate::editor::Geometry;
use alloc::{rc::Rc, string::String, vec, vec::Vec};
use sunlight_ui::{Canvas, Color};

pub const MAX_LAYERS: usize = 8;
pub const MAX_OBJECTS: usize = 64;
pub const MAX_POINTS: usize = 256;
pub const MAX_TEXT: usize = 128;
pub const MAX_TEXT_OBJECTS: usize = 16;
pub const MAX_TEXT_MASK: usize = 32 * 1024;
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct P {
    pub x: f32,
    pub y: f32,
}
impl P {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform(pub [f32; 6]);
impl Default for Transform {
    fn default() -> Self {
        Self([1., 0., 0., 1., 0., 0.])
    }
}
impl Transform {
    pub fn point(self, p: P) -> P {
        let [a, b, c, d, e, f] = self.0;
        P::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }
    pub fn inverse(self, p: P) -> P {
        let [a, b, c, d, e, f] = self.0;
        let det = a * d - b * c;
        P::new(
            (d * (p.x - e) - c * (p.y - f)) / det,
            (-b * (p.x - e) + a * (p.y - f)) / det,
        )
    }
    pub fn then(self, next: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = next.0;
        Self([
            g * a + i * b,
            h * a + j * b,
            g * c + i * d,
            h * c + j * d,
            g * e + i * f + k,
            h * e + j * f + l,
        ])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Arrow,
    Line,
    Rectangle,
    Ellipse,
    Text,
    Highlighter,
}
impl Tool {
    pub const ALL: [Self; 8] = [
        Self::Select,
        Self::Pen,
        Self::Arrow,
        Self::Line,
        Self::Rectangle,
        Self::Ellipse,
        Self::Text,
        Self::Highlighter,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Select => "Select / Move",
            Self::Pen => "Pen",
            Self::Arrow => "Arrow",
            Self::Line => "Line",
            Self::Rectangle => "Rectangle",
            Self::Ellipse => "Ellipse",
            Self::Text => "Text",
            Self::Highlighter => "Highlighter",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub color: u32,
    pub fill: Option<u32>,
    pub width: u16,
    pub opacity: u8,
    pub font_size: u16,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            color: 0xffffb52e,
            fill: None,
            width: 3,
            opacity: 255,
            font_size: 26,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct TextMask {
    pub text: String,
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
}
impl TextMask {
    pub fn new(text: &str) -> Result<Self, &'static str> {
        if text.is_empty() || text.len() > MAX_TEXT || text.chars().any(char::is_control) {
            return Err("Text: 1-128 UTF-8 bytes, single line.");
        }
        let role = sun_font::FontRole::UiRegular;
        let size = sun_font::measure_text(text, role);
        let w = size.w.max(1);
        let h = sun_font::line_height(role).max(1);
        if w > 2048 || h > 64 || w.checked_mul(h).is_none_or(|n| n as usize > MAX_TEXT_MASK) {
            return Err("Text raster limit exceeded.");
        }
        let mut pixels = vec![0xff000000; (w * h) as usize];
        sun_font::draw_text(
            &mut Canvas::new(&mut pixels, w, w, h),
            text,
            0,
            0,
            &sun_font::TextStyle::new(role, Color::rgb(255, 255, 255)),
        );
        Ok(Self {
            text: String::from(text),
            w,
            h,
            pixels: pixels.into_iter().map(|p| (p & 255) as u8).collect(),
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub id: u64,
    pub tool: Tool,
    pub a: P,
    pub b: P,
    pub points: Rc<Vec<P>>,
    pub text: Option<Rc<TextMask>>,
    pub style: Style,
    pub transform: Transform,
}
impl Object {
    pub fn new(tool: Tool, a: P, style: Style) -> Self {
        Self {
            id: 0,
            tool,
            a,
            b: a,
            points: Rc::new(vec![a]),
            text: None,
            style,
            transform: Transform::default(),
        }
    }
    pub fn add_point(&mut self, p: P) {
        let last = *self.points.last().unwrap_or(&self.a);
        if (p.x - last.x) * (p.x - last.x) + (p.y - last.y) * (p.y - last.y) >= 1. {
            let points = Rc::make_mut(&mut self.points);
            if points.len() == MAX_POINTS {
                let mut i = 0;
                points.retain(|_| {
                    let keep = i % 2 == 0;
                    i += 1;
                    keep
                });
            }
            points.push(p);
        }
        self.b = p;
    }
    pub fn valid(&self) -> bool {
        match self.tool {
            Tool::Text => self.text.is_some(),
            Tool::Pen | Tool::Highlighter => self.points.len() > 1,
            Tool::Select => false,
            Tool::Rectangle | Tool::Ellipse => {
                (self.a.x - self.b.x).abs() >= 1. && (self.a.y - self.b.y).abs() >= 1.
            }
            _ => (self.a.x - self.b.x).abs() + (self.a.y - self.b.y).abs() >= 1.,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let [a, b, c, d, _, _] = self.transform.0;
        let finite = |p: P| {
            p.x.is_finite()
                && p.y.is_finite()
                && p.x.abs() <= 10_000_000.
                && p.y.abs() <= 10_000_000.
        };
        if !self.valid()
            || !finite(self.a)
            || !finite(self.b)
            || self.points.len() > MAX_POINTS
            || !self.points.iter().copied().all(finite)
            || !(1..=128).contains(&self.style.width)
            || !(1..=128).contains(&self.style.font_size)
            || !self
                .transform
                .0
                .iter()
                .all(|v| v.is_finite() && v.abs() <= 1_000_000.)
            || (a * d - b * c).abs() < 1e-10
        {
            return Err("Object geometry or transform limit exceeded.");
        }
        let (lo, hi) = self.bounds();
        if !finite(lo) || !finite(hi) {
            return Err("Annotation extent limit exceeded.");
        }
        Ok(())
    }
    pub fn local_bounds(&self) -> (P, P) {
        if let Some(text) = &self.text {
            let scale = self.style.font_size as f32 / 13.;
            return (
                self.a,
                P::new(
                    self.a.x + text.w as f32 * scale,
                    self.a.y + text.h as f32 * scale,
                ),
            );
        }
        let mut lo = P::new(self.a.x.min(self.b.x), self.a.y.min(self.b.y));
        let mut hi = P::new(self.a.x.max(self.b.x), self.a.y.max(self.b.y));
        if matches!(self.tool, Tool::Pen | Tool::Highlighter) {
            for p in self.points.iter() {
                lo.x = lo.x.min(p.x);
                lo.y = lo.y.min(p.y);
                hi.x = hi.x.max(p.x);
                hi.y = hi.y.max(p.y);
            }
        }
        let pad = if self.tool == Tool::Arrow {
            self.style.width as f32 * 4. + 8.
        } else {
            self.style.width as f32 / 2. + 1.
        };
        (
            P::new(lo.x - pad, lo.y - pad),
            P::new(hi.x + pad, hi.y + pad),
        )
    }
    pub fn bounds(&self) -> (P, P) {
        let (a, b) = self.local_bounds();
        let corners = [a, P::new(a.x, b.y), b, P::new(b.x, a.y)].map(|p| self.transform.point(p));
        let mut lo = corners[0];
        let mut hi = lo;
        for p in corners {
            lo.x = lo.x.min(p.x);
            lo.y = lo.y.min(p.y);
            hi.x = hi.x.max(p.x);
            hi.y = hi.y.max(p.y);
        }
        (lo, hi)
    }
    pub fn translate(&mut self, dx: f32, dy: f32) {
        self.transform = self.transform.then(Transform([1., 0., 0., 1., dx, dy]));
    }
    pub fn resize(&mut self, target: P) {
        let (a, b) = self.bounds();
        let sx = ((target.x - a.x) / (b.x - a.x).max(1.)).clamp(0.05, 20.);
        let sy = ((target.y - a.y) / (b.y - a.y).max(1.)).clamp(0.05, 20.);
        self.transform = self.transform.then(Transform([
            sx,
            0.,
            0.,
            sy,
            a.x * (1. - sx),
            a.y * (1. - sy),
        ]));
    }
    fn coverage(&self, p: P, tolerance: f32) -> (u32, u8) {
        let q = self.transform.inverse(p);
        let width = self.style.width as f32 / 2. + tolerance;
        let (a, b) = (self.a, self.b);
        let (x0, x1, y0, y1) = (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y));
        let mut fill = false;
        let hit = match self.tool {
            Tool::Pen | Tool::Highlighter => {
                self.points.windows(2).any(|s| near(q, s[0], s[1], width))
            }
            Tool::Line => near(q, a, b, width),
            Tool::Arrow => {
                let dx = b.x - a.x;
                let dy = b.y - a.y;
                let len = libm::sqrtf(dx * dx + dy * dy).max(1.);
                let head = (self.style.width as f32 * 3. + 6.).min(len);
                let ux = dx / len;
                let uy = dy / len;
                near(q, a, b, width)
                    || near(
                        q,
                        b,
                        P::new(
                            b.x - ux * head - uy * head * 0.5,
                            b.y - uy * head + ux * head * 0.5,
                        ),
                        width,
                    )
                    || near(
                        q,
                        b,
                        P::new(
                            b.x - ux * head + uy * head * 0.5,
                            b.y - uy * head - ux * head * 0.5,
                        ),
                        width,
                    )
            }
            Tool::Rectangle => {
                fill = q.x > x0 + width && q.x < x1 - width && q.y > y0 + width && q.y < y1 - width;
                q.x >= x0 - width && q.x <= x1 + width && q.y >= y0 - width && q.y <= y1 + width
            }
            Tool::Ellipse => {
                let rx = ((x1 - x0) / 2.).max(0.5);
                let ry = ((y1 - y0) / 2.).max(0.5);
                let dx = (q.x - (x0 + x1) / 2.) / rx;
                let dy = (q.y - (y0 + y1) / 2.) / ry;
                let r = libm::sqrtf(dx * dx + dy * dy);
                let band = width / rx.min(ry);
                fill = r < 1. - band;
                r <= 1. + band
            }
            Tool::Text => {
                if let Some(text) = &self.text {
                    let scale = self.style.font_size as f32 / 13.;
                    let x = ((q.x - a.x) / scale) as i32;
                    let y = ((q.y - a.y) / scale) as i32;
                    if q.x >= a.x
                        && q.y >= a.y
                        && x >= 0
                        && y >= 0
                        && x < text.w as i32
                        && y < text.h as i32
                    {
                        return (
                            self.style.color,
                            text.pixels[(y as u32 * text.w + x as u32) as usize],
                        );
                    }
                }
                false
            }
            Tool::Select => false,
        };
        if !hit {
            (0, 0)
        } else if fill {
            self.style.fill.map(|c| (c, 255)).unwrap_or((0, 0))
        } else {
            (self.style.color, 255)
        }
    }
    pub fn hit(&self, p: P, tolerance: f32) -> bool {
        if self.tool == Tool::Text {
            let (a, b) = self.local_bounds();
            let q = self.transform.inverse(p);
            return q.x >= a.x && q.x <= b.x && q.y >= a.y && q.y <= b.y;
        }
        let [a, b, c, d, _, _] = self.transform.0;
        let det = (a * d - b * c).abs().max(1e-10);
        let tolerance = tolerance * (a.abs() + b.abs()).max(c.abs() + d.abs()) / det;
        self.coverage(p, tolerance).1 > 0
    }
    pub fn pixel(&self, p: P, opacity: u8) -> u32 {
        let (c, mask) = self.coverage(p, 0.);
        let alpha = (mask as u32 * self.style.opacity as u32 * opacity as u32 + 32512) / 65025;
        (c & 0xffffff) | (alpha << 24)
    }
}
fn near(p: P, a: P, b: P, r: f32) -> bool {
    if p.x < a.x.min(b.x) - r
        || p.x > a.x.max(b.x) + r
        || p.y < a.y.min(b.y) - r
        || p.y > a.y.max(b.y) + r
    {
        return false;
    }
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy).max(0.001)).clamp(0., 1.);
    let x = p.x - a.x - t * dx;
    let y = p.y - a.y - t * dy;
    x * x + y * y <= r * r
}
/// Straight ARGB source-over, including transparent destinations.
pub fn over(dst: u32, src: u32) -> u32 {
    let sa = src >> 24;
    if sa == 0 {
        return dst;
    }
    let da = dst >> 24;
    let alpha = sa * 255 + da * (255 - sa);
    let mut out = ((alpha + 127) / 255) << 24;
    for shift in [0, 8, 16] {
        let c = (((src >> shift) & 255) * sa * 255
            + ((dst >> shift) & 255) * da * (255 - sa)
            + alpha / 2)
            / alpha.max(1);
        out |= c << shift;
    }
    out
}
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: u8,
    pub objects: Vec<Rc<Object>>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub layers: Vec<Layer>,
    pub active: u64,
    next_id: u64,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            layers: vec![Layer {
                id: 0,
                name: String::from("Background"),
                visible: true,
                locked: true,
                opacity: 255,
                objects: Vec::new(),
            }],
            active: 0,
            next_id: 1,
        }
    }
}
impl Scene {
    pub(crate) fn id_floor(&mut self, next: u64) {
        self.next_id = self.next_id.max(next);
    }
    pub(crate) fn next_id(&self) -> u64 {
        self.next_id
    }
    fn id(&mut self) -> Result<u64, &'static str> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("Document ID limit reached.")?;
        Ok(id)
    }
    pub fn layer(&self) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == self.active)
    }
    pub fn layer_mut(&mut self) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == self.active)
    }
    pub fn writable(&self) -> bool {
        self.layer()
            .is_some_and(|l| l.id != 0 && l.visible && !l.locked)
    }
    pub fn count(&self) -> usize {
        self.layers.iter().map(|l| l.objects.len()).sum()
    }
    pub fn add_layer(&mut self) -> Result<u64, &'static str> {
        if self.layers.len() >= MAX_LAYERS {
            return Err("Layer limit: 8 including background.");
        }
        let id = self.id()?;
        self.layers.push(Layer {
            id,
            name: alloc::format!("Annotation {id}"),
            visible: true,
            locked: false,
            opacity: 255,
            objects: Vec::new(),
        });
        self.active = id;
        Ok(id)
    }
    pub fn remove_layer(&mut self) -> Result<(), &'static str> {
        if self.active == 0 {
            return Err("Background is protected.");
        }
        let i = self
            .layers
            .iter()
            .position(|l| l.id == self.active)
            .ok_or("Select a layer.")?;
        if self.layers[i].locked {
            return Err("Unlock layer first.");
        }
        self.layers.remove(i);
        self.active = self.layers[i.saturating_sub(1)].id;
        Ok(())
    }
    pub fn reorder(&mut self, up: bool) {
        if let Some(i) = self.layers.iter().position(|l| l.id == self.active) {
            let next = if up {
                (i + 1).min(self.layers.len() - 1)
            } else {
                i.saturating_sub(1)
            };
            if i > 0 && next > 0 {
                self.layers.swap(i, next);
            }
        }
    }
    pub fn insert(&mut self, mut object: Object) -> Result<u64, &'static str> {
        if !self.writable() {
            return Err("Add or select a visible, unlocked annotation layer.");
        }
        object.validate()?;
        if object.tool == Tool::Text
            && self
                .layers
                .iter()
                .flat_map(|l| &l.objects)
                .filter(|o| o.tool == Tool::Text)
                .count()
                >= MAX_TEXT_OBJECTS
        {
            return Err("Text limit: 16 objects per document.");
        }
        if self.count() >= MAX_OBJECTS {
            return Err("Object limit: 64 per document.");
        }
        let id = self.id()?;
        object.id = id;
        self.layer_mut().unwrap().objects.push(Rc::new(object));
        Ok(id)
    }
    pub fn object(&self, id: u64) -> Option<&Object> {
        self.layers
            .iter()
            .flat_map(|l| l.objects.iter())
            .find(|o| o.id == id)
            .map(|o| o.as_ref())
    }
    pub fn replace(&mut self, object: Object) -> Result<(), &'static str> {
        object.validate()?;
        for l in &mut self.layers {
            if let Some(i) = l.objects.iter().position(|o| o.id == object.id) {
                if l.locked || !l.visible {
                    return Err("Layer is hidden or locked.");
                }
                l.objects[i] = Rc::new(object);
                return Ok(());
            }
        }
        Err("Select an object.")
    }
    pub fn delete(&mut self, id: u64) -> Result<(), &'static str> {
        for l in &mut self.layers {
            if l.objects.iter().any(|o| o.id == id) {
                if l.locked || !l.visible {
                    return Err("Layer is hidden or locked.");
                }
                l.objects.retain(|o| o.id != id);
                return Ok(());
            }
        }
        Err("Select an object.")
    }
    pub fn hit(&self, p: P, tolerance: f32) -> Option<(u64, u64)> {
        for l in self
            .layers
            .iter()
            .rev()
            .filter(|l| l.visible && !l.locked && l.opacity > 0)
        {
            for o in l.objects.iter().rev() {
                if o.style.opacity > 0 && o.hit(p, tolerance) {
                    return Some((l.id, o.id));
                }
            }
        }
        None
    }
    /// Names, locks and active selection do not affect raster output.
    pub fn same_pixels(&self, other: &Self) -> bool {
        let a = &self.layers[0];
        let b = &other.layers[0];
        if a.visible != b.visible || a.opacity != b.opacity {
            return false;
        }
        let visible = |l: &&Layer| l.id != 0 && l.visible && l.opacity > 0 && !l.objects.is_empty();
        let mut a = self.layers.iter().filter(visible);
        let mut b = other.layers.iter().filter(visible);
        loop {
            match (a.next(), b.next()) {
                (None, None) => return true,
                (Some(a), Some(b)) if a.opacity == b.opacity && a.objects == b.objects => {}
                _ => return false,
            }
        }
    }
    pub fn has_annotations(&self) -> bool {
        self.count() > 0
    }
    pub fn transform(&mut self, op: Geometry, size: (u32, u32)) -> Result<(), &'static str> {
        let (w, h) = (size.0 as f32, size.1 as f32);
        let t = match op {
            Geometry::RotateRight => Transform([0., 1., -1., 0., h, 0.]),
            Geometry::RotateLeft => Transform([0., -1., 1., 0., 0., w]),
            Geometry::FlipH => Transform([-1., 0., 0., 1., w, 0.]),
            Geometry::FlipV => Transform([1., 0., 0., -1., 0., h]),
            Geometry::Crop(r) => Transform([1., 0., 0., 1., -r.x as f32, -r.y as f32]),
            Geometry::Resize(nw, nh) => Transform([nw as f32 / w, 0., 0., nh as f32 / h, 0., 0.]),
        };
        for l in &mut self.layers {
            for o in &mut l.objects {
                let o = Rc::make_mut(o);
                o.transform = o.transform.then(t);
                o.validate()?;
            }
        }
        Ok(())
    }
    pub fn pixel(&self, base: u32, p: P) -> u32 {
        let bg = &self.layers[0];
        let mut out = if bg.visible {
            (base & 0xffffff) | ((((base >> 24) * bg.opacity as u32 + 127) / 255) << 24)
        } else {
            0
        };
        for l in self
            .layers
            .iter()
            .skip(1)
            .filter(|l| l.visible && l.opacity > 0)
        {
            let mut group = 0;
            for o in &l.objects {
                group = over(group, o.pixel(p, 255));
            }
            out = over(out, with_opacity(group, l.opacity));
        }
        out
    }
}

/// Apply layer opacity once to its composited contents, including overlaps.
pub fn with_opacity(color: u32, opacity: u8) -> u32 {
    (color & 0xffffff) | ((((color >> 24) * opacity as u32 + 127) / 255) << 24)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::{Editor, Sampler};
    use sunlight_ui::{image::RgbaImage, Rect};
    fn rect(color: u32) -> Object {
        let mut o = Object::new(
            Tool::Rectangle,
            P::new(2., 3.),
            Style {
                color,
                fill: Some(color),
                width: 1,
                ..Style::default()
            },
        );
        o.b = P::new(7., 8.);
        o
    }
    fn scene() -> Scene {
        let mut s = Scene::default();
        s.add_layer().unwrap();
        s.insert(rect(0xffff0000)).unwrap();
        s
    }
    #[test]
    fn layers_have_stable_ids_protect_background_and_reorder() {
        let mut s = scene();
        let first = s.active;
        let second = s.add_layer().unwrap();
        s.insert(rect(0xff0000ff)).unwrap();
        let p = P::new(4.5, 5.5);
        assert_eq!(s.pixel(0, p), 0xff0000ff);
        s.reorder(false);
        assert_eq!(s.layers[1].id, second);
        assert_eq!(s.pixel(0, p), 0xffff0000);
        s.remove_layer().unwrap();
        assert_eq!(s.active, 0);
        assert_eq!(s.layers[1].id, first);
        assert!(s.remove_layer().is_err());
        s.reorder(true);
        assert_eq!(s.layers[0].id, 0);
        while s.layers.len() < MAX_LAYERS {
            s.add_layer().unwrap();
        }
        assert!(s.add_layer().is_err());
        let mut overflow = Scene::default();
        overflow.next_id = u64::MAX;
        assert!(overflow.add_layer().is_err());
        assert_eq!(overflow.layers.len(), 1);
        let mut e = Editor::new(10, 10);
        e.edit_scene(|s| {
            s.layers[0].visible = false;
            s.layers[0].opacity = 128;
            Ok(())
        })
        .unwrap();
        e.reset();
        assert!(e.scene.layers[0].visible);
        assert_eq!(e.scene.layers[0].opacity, 255);
        e.undo();
        assert!(!e.scene.layers[0].visible);
    }
    #[test]
    fn visibility_locking_opacity_and_object_z_order_control_selection() {
        let mut s = scene();
        let p = P::new(4.5, 5.5);
        let lower = s.hit(p, 0.).unwrap();
        let top = s.insert(rect(0xff00ff00)).unwrap();
        assert_eq!(s.hit(p, 0.).unwrap().1, top);
        s.delete(top).unwrap();
        assert_eq!(s.hit(p, 0.), Some(lower));
        s.layer_mut().unwrap().locked = true;
        assert_eq!(s.hit(p, 0.), None);
        assert!(s.delete(lower.1).is_err());
        assert!(s.insert(rect(0)).is_err());
        assert!(s.remove_layer().is_err());
        assert_eq!(s.pixel(0, p), 0xffff0000);
        s.layer_mut().unwrap().locked = false;
        s.layer_mut().unwrap().visible = false;
        assert_eq!(s.pixel(0, p), 0);
        assert_eq!(s.hit(p, 0.), None);
        s.layer_mut().unwrap().visible = true;
        s.layer_mut().unwrap().opacity = 0;
        assert_eq!(s.hit(p, 0.), None);
    }
    #[test]
    fn object_motion_resizing_and_hit_testing_use_transformed_geometry() {
        let mut o = rect(0xffff0000);
        o.translate(10., 20.);
        assert!(!o.hit(P::new(4., 5.), 0.));
        assert!(o.hit(P::new(14., 25.), 0.));
        let (a, b) = o.bounds();
        o.resize(P::new(a.x + (b.x - a.x) * 2., a.y + (b.y - a.y) * 3.));
        let (na, nb) = o.bounds();
        assert_eq!(na, a);
        assert!((nb.x - a.x - (b.x - a.x) * 2.).abs() < 0.01);
        assert!((nb.y - a.y - (b.y - a.y) * 3.).abs() < 0.01);
        let mut line = Object::new(Tool::Line, P::new(3., 3.), Style::default());
        line.b = P::new(8., 3.);
        line.transform = Transform([2., 0., 0., 2., 10., 10.]);
        assert!(line.hit(P::new(20., 17.), 1.));
        assert!(!line.hit(P::new(20., 25.), 1.));
    }
    #[test]
    fn all_image_geometry_transforms_keep_annotations_on_matching_content() {
        let image = RgbaImage {
            width: 10,
            height: 12,
            pixels: (0..120).map(|n| 0xff000000 | n).collect(),
        };
        for (op, expected) in [
            (Geometry::RotateRight, P::new(6.5, 4.5)),
            (Geometry::RotateLeft, P::new(5.5, 5.5)),
            (Geometry::FlipH, P::new(5.5, 5.5)),
            (Geometry::FlipV, P::new(4.5, 6.5)),
            (Geometry::Crop(Rect::new(2, 3, 6, 7)), P::new(2.5, 2.5)),
            (Geometry::Resize(20, 36), P::new(9., 16.5)),
        ] {
            let mut e = Editor::new(10, 12);
            e.edit_scene(|s| {
                *s = scene();
                Ok(())
            })
            .unwrap();
            let id = e.scene.layers[1].objects[0].id;
            e.geometry(op).unwrap();
            let o = e.scene.object(id).unwrap();
            assert_eq!(o.transform.point(P::new(4.5, 5.5)), expected);
            assert_eq!(e.scene.hit(expected, 0.).unwrap().1, id);
            let sampler = Sampler::new(e.state);
            assert_eq!(
                sampler.pixel(&image, expected.x as u32, expected.y as u32),
                image.pixels[54]
            );
            assert_eq!(e.scene.pixel(0, expected), 0xffff0000);
            e.undo();
            assert_eq!(e.state.size, (10, 12));
            assert_eq!(e.scene.object(id).unwrap().transform, Transform::default());
            e.redo();
            assert_eq!(
                e.scene
                    .object(id)
                    .unwrap()
                    .transform
                    .point(P::new(4.5, 5.5)),
                expected
            );
        }
    }
    #[test]
    fn unified_history_checkpoints_styles_layers_geometry_and_branching() {
        let mut e = Editor::new(20, 20);
        e.edit_scene(|s| {
            s.add_layer()?;
            s.insert(rect(0xffff0000))?;
            Ok(())
        })
        .unwrap();
        let created = e.scene.clone();
        let id = created.layers[1].objects[0].id;
        e.mark_saved();
        let saved = e.scene.clone();
        e.activate(0);
        assert!(!e.dirty());
        e.activate(created.active);
        let mut moved = e.scene.object(id).unwrap().clone();
        moved.translate(2., 4.);
        moved.style.opacity = 64;
        e.edit_scene(|s| s.replace(moved)).unwrap();
        let changed = e.scene.clone();
        assert!(e.dirty());
        e.undo();
        assert_eq!(e.scene.layers, saved.layers);
        assert!(!e.dirty());
        e.redo();
        assert_eq!(e.scene, changed);
        e.edit_scene(|s| {
            s.layer_mut().unwrap().visible = false;
            s.layer_mut().unwrap().opacity = 128;
            Ok(())
        })
        .unwrap();
        e.undo();
        assert_eq!(e.scene, changed);
        e.edit_scene(|s| s.delete(id)).unwrap();
        assert!(!e.can_redo());
        e.undo();
        assert!(e.scene.object(id).is_some());
        e.reset();
        assert!(!e.scene.has_annotations());
        e.undo();
        assert_eq!(e.scene, changed);
        let before = e.scene.clone();
        assert!(e.geometry(Geometry::Resize(0, 1)).is_err());
        assert_eq!(e.scene, before);
    }
    #[test]
    fn straight_alpha_and_layer_group_opacity_do_not_accumulate_overlaps() {
        assert_eq!(over(0, 0x80ff0000), 0x80ff0000);
        assert_eq!(over(0xff0000ff, 0x80ff0000), 0xff80007f);
        assert_eq!(over(0x12345678, 0), 0x12345678);
        let mut s = scene();
        s.insert(rect(0xffff0000)).unwrap();
        s.layer_mut().unwrap().opacity = 128;
        assert_eq!(s.pixel(0xff0000ff, P::new(4.5, 5.5)), 0xff80007f);
        s.layers[0].visible = false;
        assert_eq!(s.pixel(0xffffffff, P::new(4.5, 5.5)), 0x80ff0000);
    }
    #[test]
    fn highlighter_alpha_is_applied_once_per_stroke_and_arrows_point_both_directions() {
        let mut o = Object::new(
            Tool::Highlighter,
            P::new(2., 5.),
            Style {
                opacity: 96,
                width: 4,
                ..Style::default()
            },
        );
        o.add_point(P::new(8., 5.));
        o.add_point(P::new(2., 5.));
        assert_eq!(o.pixel(P::new(4., 5.), 255) >> 24, 96);
        for (a, b, head) in [
            (P::new(2., 10.), P::new(20., 10.), P::new(12., 14.)),
            (P::new(20., 10.), P::new(2., 10.), P::new(10., 6.)),
        ] {
            let mut arrow = Object::new(
                Tool::Arrow,
                a,
                Style {
                    width: 1,
                    ..Style::default()
                },
            );
            arrow.b = b;
            assert!(arrow.pixel(head, 255) >> 24 > 0);
        }
    }
    #[test]
    fn point_storage_keeps_the_endpoint_and_caps_document_allocations() {
        let mut o = Object::new(Tool::Pen, P::new(0., 0.), Style::default());
        for n in 1..10000 {
            o.add_point(P::new(n as f32, 1.));
        }
        assert!(o.points.len() <= MAX_POINTS);
        assert_eq!(o.points.last().unwrap().x, 9999.);
        let mut s = scene();
        while s.count() < MAX_OBJECTS {
            s.insert(o.clone()).unwrap();
        }
        assert!(s.insert(o).is_err());
        let mut e = Editor::new(20, 20);
        e.edit_scene(|s| {
            *s = scene();
            Ok(())
        })
        .unwrap();
        let id = e.scene.layers[1].objects[0].id;
        let shared = e.scene.object(id).unwrap().points.clone();
        for n in 0..100 {
            let mut o = e.scene.object(id).unwrap().clone();
            o.style.opacity = n;
            e.edit_scene(|s| s.replace(o)).unwrap();
        }
        assert!(Rc::ptr_eq(&shared, &e.scene.object(id).unwrap().points));
        let mut n = 0;
        while e.can_undo() {
            e.undo();
            n += 1;
        }
        assert_eq!(n, 32);
    }
    #[test]
    fn text_uses_native_glyphs_and_rejects_malformed_or_unbounded_data() {
        for bad in ["", "a\nb", "\0", &"x".repeat(MAX_TEXT + 1)] {
            assert!(TextMask::new(bad).is_err());
        }
        let mask = Rc::new(TextMask::new("Light Lens").unwrap());
        assert!(mask.pixels.iter().any(|a| *a > 0));
        assert!(mask.pixels.len() <= 2048 * 64);
        let mut o = Object::new(Tool::Text, P::new(2., 3.), Style::default());
        o.text = Some(mask.clone());
        assert!(o.validate().is_ok());
        let mut s = Scene::default();
        s.add_layer().unwrap();
        for _ in 0..MAX_TEXT_OBJECTS {
            s.insert(o.clone()).unwrap();
        }
        assert!(s.insert(o.clone()).is_err());
        assert!(o.hit(P::new(3., 4.), 0.));
        o.translate(4., 6.);
        assert!(Rc::ptr_eq(&mask, o.text.as_ref().unwrap()));
        let mut bad = rect(0);
        bad.transform.0[0] = f32::NAN;
        assert!(bad.validate().is_err());
        bad.transform = Transform([0.; 6]);
        assert!(bad.validate().is_err());
        bad = rect(0);
        bad.style.width = 0;
        assert!(bad.validate().is_err());
        bad = rect(0);
        bad.b = bad.a;
        assert!(!bad.valid());
    }
    #[test]
    fn histogram_channels_include_visible_composite_and_alpha() {
        let image = RgbaImage {
            width: 10,
            height: 12,
            pixels: vec![0xff0000ff; 120],
        };
        let mut s = scene();
        let h = crate::editor::Histogram::compute_scene(
            &image,
            crate::editor::State::new(10, 12),
            Some(&s),
        );
        assert!(h.channels[0][255] > 0);
        assert!(h.channels[2][255] > 0);
        s.layers[1].visible = false;
        let hidden = crate::editor::Histogram::compute_scene(
            &image,
            crate::editor::State::new(10, 12),
            Some(&s),
        );
        assert_eq!(hidden.channels[0][255], 0);
        assert_eq!(hidden.channels[2][255], 120 * 255);
    }
    #[test]
    fn ids_are_not_reused_after_undo_branches_or_reset() {
        let mut e = Editor::new(10, 10);
        let mut first = 0;
        e.edit_scene(|s| {
            first = s.add_layer()?;
            Ok(())
        })
        .unwrap();
        e.undo();
        let mut second = 0;
        e.edit_scene(|s| {
            second = s.add_layer()?;
            Ok(())
        })
        .unwrap();
        assert!(second > first);
        e.reset();
        let mut third = 0;
        e.edit_scene(|s| {
            third = s.add_layer()?;
            Ok(())
        })
        .unwrap();
        assert!(third > second);
    }
}
