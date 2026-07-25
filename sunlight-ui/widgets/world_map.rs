//! Monochrome interactive world map (equirectangular projection).
//!
//! ## Projection
//!
//! Equirectangular (plate carrée), longitude ∈ `[-180, 180]`, latitude ∈
//! `[-90, 90]`:
//!
//! ```text
//! x = map_x + (lon + 180) / 360 * map_w
//! y = map_y + ( 90 - lat) / 180 * map_h
//!
//! lon = (x - map_x) / map_w * 360 - 180
//! lat = 90 - (y - map_y) / map_h * 180
//! ```
//!
//! The drawable map rect letterboxes inside the widget bounds to preserve the
//! 2∶1 equirectangular aspect ratio (never stretches geography).
//!
//! ## Silhouette origin
//!
//! No project-owned world-outline asset existed. `WORLD_MAP_BITS` is a
//! deliberately simplified 120×60 monochrome silhouette authored for this
//! widget (stylized continent blobs + Antarctica). It is **not** derived from
//! the colorful timezone design reference and carries no labels, legend, or
//! city data.
//!
//! ## Scope
//!
//! This widget has no timezone names, city search, NTP regions, or settings
//! logic — only geometry, markers, and coordinate hit-testing.

use crate::event::Event;
use crate::geom::{Point, Rect, Size};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

/// Embedded silhouette width (columns).
pub const WORLD_MAP_W: u32 = 120;
/// Embedded silhouette height (rows). Aspect is exactly 2∶1.
pub const WORLD_MAP_H: u32 = 60;

/// Row-major bit-packed silhouette (`WORLD_MAP_W` bits per row, MSB first).
///
/// Origin: simplified SunlightOS-authored outline (see module docs). 900 bytes.
pub static WORLD_MAP_BITS: [u8; 900] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x0F, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x1F, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x1F, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x1F, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x3F, 0xE0, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x0F, 0xFE, 0x1F, 0xC0, 0x00, 0x00, 0x1F, 0xFF, 0xF8, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x0F, 0xFF, 0xFF, 0xDF, 0xC0, 0x01, 0x00, 0x9F, 0xFF, 0xFF, 0xFF, 0xF8, 0x00, 0x00,
    0x00, 0x1F, 0xFF, 0xFF, 0xFF, 0xC0, 0x03, 0x9F, 0xDF, 0xFF, 0xFF, 0xFF, 0xFC, 0x00, 0x00,
    0x00, 0x1F, 0xFF, 0xFF, 0xFF, 0x80, 0x07, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00,
    0x00, 0x1F, 0xFF, 0xFF, 0xFE, 0x00, 0x03, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xC0, 0x00,
    0x00, 0x1F, 0xFF, 0xFF, 0xFF, 0x00, 0x03, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xE0, 0x00,
    0x00, 0x3F, 0xFF, 0xFF, 0xFF, 0x80, 0x03, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF8, 0x00,
    0x00, 0x3F, 0xFF, 0xFF, 0xFF, 0x80, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE, 0x00,
    0x00, 0x3F, 0xFF, 0xFF, 0xFF, 0xC0, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFC, 0x00,
    0x00, 0x3F, 0xFF, 0xFF, 0xFF, 0xE0, 0x00, 0x00, 0x3F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFC, 0x00,
    0x00, 0x3F, 0xFF, 0xFF, 0xFF, 0xC0, 0x00, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFC, 0x00,
    0x00, 0x1F, 0xFF, 0xFF, 0xFF, 0xC0, 0x03, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE, 0x00,
    0x00, 0x1F, 0xFF, 0xFF, 0xFF, 0xC0, 0x07, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFC, 0x00,
    0x00, 0x0F, 0xFF, 0xFF, 0xFF, 0x80, 0x07, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFC, 0x00,
    0x00, 0x0F, 0xFF, 0xFF, 0xFF, 0x80, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF8, 0x00,
    0x00, 0x07, 0xFF, 0xFF, 0xFF, 0x80, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF8, 0x00,
    0x00, 0x07, 0xFF, 0xFF, 0xFE, 0x00, 0x0F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xF8, 0x00,
    0x00, 0x03, 0xFF, 0xFF, 0xF8, 0x00, 0x1F, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF, 0xF8, 0x00,
    0x00, 0x00, 0xFF, 0xFF, 0xE0, 0x00, 0x1F, 0xFF, 0xFF, 0x1F, 0xFF, 0xFF, 0xFF, 0xF8, 0x00,
    0x00, 0x00, 0x3F, 0xFF, 0x80, 0x00, 0x3F, 0xFF, 0xFF, 0x0F, 0xFF, 0xFF, 0xFF, 0xF8, 0x00,
    0x00, 0x00, 0x0F, 0xF0, 0x00, 0x00, 0x3F, 0xFF, 0xFF, 0x03, 0xFF, 0xFF, 0xFF, 0x80, 0x00,
    0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x3F, 0xFF, 0xFF, 0x03, 0xFF, 0xFF, 0xFC, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x1F, 0x80, 0x00, 0x3F, 0xFF, 0xFF, 0x01, 0xFD, 0xFF, 0xE0, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x3F, 0xF8, 0x00, 0x3F, 0xFF, 0xFE, 0x01, 0xFC, 0x07, 0xFF, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x3F, 0xF8, 0x00, 0x1F, 0xFF, 0xFE, 0x01, 0xFC, 0x1F, 0xFF, 0xC0, 0x00,
    0x00, 0x00, 0x00, 0x7F, 0xF8, 0x00, 0x1F, 0xFF, 0xFE, 0x00, 0xF8, 0x3F, 0xFF, 0xE0, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xF8, 0x00, 0x0F, 0xFF, 0xFE, 0x00, 0xF8, 0x1F, 0xFF, 0xC0, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xF8, 0x00, 0x0F, 0xFF, 0xFE, 0x00, 0x20, 0x07, 0xFF, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xFC, 0x00, 0x0F, 0xFF, 0xFE, 0x80, 0x00, 0x00, 0x22, 0x00, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xFC, 0x00, 0x07, 0xFF, 0xFF, 0xC0, 0x00, 0x00, 0x3F, 0xE0, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xFC, 0x00, 0x07, 0xFF, 0xF9, 0xC0, 0x00, 0x00, 0xFF, 0xF8, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xFC, 0x00, 0x03, 0xFF, 0xF3, 0xE0, 0x00, 0x01, 0xFF, 0xFC, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xFC, 0x00, 0x00, 0xFF, 0xE1, 0xC0, 0x00, 0x01, 0xFF, 0xFC, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xFE, 0x00, 0x00, 0x3F, 0x81, 0xC0, 0x00, 0x03, 0xFF, 0xFE, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xFC, 0x00, 0x00, 0x0F, 0x00, 0x80, 0x00, 0x01, 0xFF, 0xFC, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xFC, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0xFF, 0xFC, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xF8, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xF8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x3F, 0xE0, 0x00,
    0x00, 0x00, 0x00, 0xFF, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x7F, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x3F, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x3F, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
];

/// Geographic coordinate in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoCoord {
    /// Longitude degrees, normalized to `[-180, 180]`.
    pub lon: f32,
    /// Latitude degrees, clamped to `[-90, 90]`.
    pub lat: f32,
}

impl GeoCoord {
    pub const fn new(lon: f32, lat: f32) -> Self {
        Self { lon, lat }
    }

    /// Wrap longitude into `[-180, 180]` and clamp latitude to `[-90, 90]`.
    pub fn normalized(self) -> Self {
        Self {
            lon: wrap_lon(self.lon),
            lat: self.lat.clamp(-90.0, 90.0),
        }
    }
}

/// Optional marker supplied by the caller (no city metadata).
#[derive(Debug, Clone, Copy)]
pub struct MapMarker {
    pub coord: GeoCoord,
    /// Hit radius in widget pixels (default 6 when 0).
    pub hit_radius: u32,
}

/// Result of a map click / hit test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MapHit {
    /// Click fell outside the letterboxed map rectangle.
    Outside,
    /// Click mapped to a geographic coordinate (and optional marker index).
    Inside {
        coord: GeoCoord,
        marker_index: Option<usize>,
    },
}

/// Layout of the letterboxed equirectangular map inside the widget rect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldMapLayout {
    pub bounds: Rect,
    /// Actual map content rectangle (2∶1, centered).
    pub map: Rect,
}

impl WorldMapLayout {
    /// Letterbox `bounds` to a 2∶1 content rect (equirectangular).
    pub fn compute(bounds: Rect) -> Self {
        if bounds.w == 0 || bounds.h == 0 {
            return Self {
                bounds,
                map: Rect::new(bounds.x, bounds.y, 0, 0),
            };
        }
        // Target aspect = 2:1 (width:height).
        let bw = bounds.w as i64;
        let bh = bounds.h as i64;
        let (mw, mh) = if bw >= bh * 2 {
            // Wider than 2:1 — pillarbox.
            let h = bounds.h;
            let w = h.saturating_mul(2);
            (w, h)
        } else {
            // Taller than 2:1 — letterbox.
            let w = bounds.w;
            let h = w / 2;
            (w, h.max(1))
        };
        let ox = bounds.x + (bounds.w as i32 - mw as i32) / 2;
        let oy = bounds.y + (bounds.h as i32 - mh as i32) / 2;
        Self {
            bounds,
            map: Rect::new(ox, oy, mw, mh),
        }
    }
}

/// Wrap longitude into `[-180, 180]`.
pub fn wrap_lon(lon: f32) -> f32 {
    let mut x = lon;
    // Reduce large values without depending on rem_euclid edge cases for NaN.
    if !x.is_finite() {
        return 0.0;
    }
    while x > 180.0 {
        x -= 360.0;
    }
    while x < -180.0 {
        x += 360.0;
    }
    x
}

/// Convert geographic coordinates to a widget-local pixel inside `map`.
///
/// Returns `None` if `map` has zero area.
pub fn geo_to_point(map: Rect, coord: GeoCoord) -> Option<Point> {
    if map.w == 0 || map.h == 0 {
        return None;
    }
    let c = coord.normalized();
    let fx = (c.lon + 180.0) / 360.0;
    let fy = (90.0 - c.lat) / 180.0;
    let x = map.x + (fx * map.w as f32) as i32;
    let y = map.y + (fy * map.h as f32) as i32;
    // Clamp to inclusive-left exclusive-right style interior.
    let x = x.clamp(map.x, map.right().saturating_sub(1));
    let y = y.clamp(map.y, map.bottom().saturating_sub(1));
    Some(Point::new(x, y))
}

/// Convert a widget-local pixel to geographic coordinates.
///
/// Returns `None` when `p` is outside `map` (including letterbox gutters).
pub fn point_to_geo(map: Rect, p: Point) -> Option<GeoCoord> {
    if map.w == 0 || map.h == 0 || !map.contains(p) {
        return None;
    }
    let fx = (p.x - map.x) as f32 / map.w as f32;
    let fy = (p.y - map.y) as f32 / map.h as f32;
    let lon = fx * 360.0 - 180.0;
    let lat = 90.0 - fy * 180.0;
    Some(GeoCoord::new(lon, lat).normalized())
}

/// Sample the embedded silhouette at normalized map UVs in `[0, 1]`.
pub fn land_at_uv(u: f32, v: f32) -> bool {
    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
        return false;
    }
    let x = (u * WORLD_MAP_W as f32) as u32;
    let y = (v * WORLD_MAP_H as f32) as u32;
    land_at_texel(x.min(WORLD_MAP_W - 1), y.min(WORLD_MAP_H - 1))
}

#[inline]
pub fn land_at_texel(x: u32, y: u32) -> bool {
    if x >= WORLD_MAP_W || y >= WORLD_MAP_H {
        return false;
    }
    let bit_index = y * WORLD_MAP_W + x;
    let byte = WORLD_MAP_BITS[(bit_index / 8) as usize];
    let bit = 7 - (bit_index % 8);
    (byte >> bit) & 1 != 0
}

/// Hit-test markers nearest-first within their hit radii.
pub fn hit_test_markers(map: Rect, p: Point, markers: &[MapMarker]) -> Option<usize> {
    let mut best: Option<(usize, i32)> = None;
    for (i, m) in markers.iter().enumerate() {
        let Some(mp) = geo_to_point(map, m.coord) else {
            continue;
        };
        let r = m.hit_radius.max(4) as i32;
        let dx = p.x - mp.x;
        let dy = p.y - mp.y;
        let d2 = dx * dx + dy * dy;
        if d2 <= r * r {
            if best.map(|(_, bd)| d2 < bd).unwrap_or(true) {
                best = Some((i, d2));
            }
        }
    }
    best.map(|(i, _)| i)
}

/// Monochrome interactive world map widget.
pub struct WorldMapWidget<'a> {
    pub rect: Rect,
    /// Optional grid (off by default).
    pub show_grid: bool,
    /// Caller-owned markers.
    pub markers: &'a [MapMarker],
    /// Optional selected coordinate (orange accent).
    pub selected: Option<GeoCoord>,
    /// Optional hover coordinate.
    pub hover: Option<GeoCoord>,
    /// Land fill color. `None` → muted monochrome from theme.
    pub land: Option<Color>,
    /// Ocean / background. `None` → transparent skip (panel shows through) or dark.
    pub ocean: Option<Color>,
    /// Fill the widget rect with ocean color before drawing land.
    pub fill_background: bool,
}

impl<'a> WorldMapWidget<'a> {
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            show_grid: false,
            markers: &[],
            selected: None,
            hover: None,
            land: None,
            ocean: None,
            fill_background: true,
        }
    }

    pub fn with_markers(mut self, markers: &'a [MapMarker]) -> Self {
        self.markers = markers;
        self
    }

    pub fn with_selected(mut self, coord: GeoCoord) -> Self {
        self.selected = Some(coord.normalized());
        self
    }

    pub fn with_hover(mut self, coord: GeoCoord) -> Self {
        self.hover = Some(coord.normalized());
        self
    }

    pub fn with_grid(mut self, on: bool) -> Self {
        self.show_grid = on;
        self
    }

    pub fn layout(&self) -> WorldMapLayout {
        WorldMapLayout::compute(self.rect)
    }

    /// Preferred size preserving 2∶1 aspect for a given width.
    pub fn preferred_size(width: u32) -> Size {
        Size::new(width, (width / 2).max(1))
    }

    pub fn point_to_geo(&self, p: Point) -> Option<GeoCoord> {
        point_to_geo(self.layout().map, p)
    }

    pub fn geo_to_point(&self, coord: GeoCoord) -> Option<Point> {
        geo_to_point(self.layout().map, coord)
    }

    /// Process a pointer event. Returns a [`MapHit`] for clicks inside the map.
    /// Hover updates are reflected via `hover` if the caller assigns the result.
    pub fn handle_event(&mut self, event: Event) -> Option<MapHit> {
        match event {
            Event::Click { x, y } | Event::MouseDown { x, y, button: 0 } => {
                Some(self.hit_test(Point::new(x, y)))
            }
            Event::MouseMove { x, y } => {
                let p = Point::new(x, y);
                self.hover = point_to_geo(self.layout().map, p);
                None
            }
            _ => None,
        }
    }

    pub fn hit_test(&self, p: Point) -> MapHit {
        let layout = self.layout();
        let Some(coord) = point_to_geo(layout.map, p) else {
            return MapHit::Outside;
        };
        let marker_index = hit_test_markers(layout.map, p, self.markers);
        MapHit::Inside {
            coord,
            marker_index,
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let layout = self.layout();
        if layout.map.w == 0 || layout.map.h == 0 {
            return;
        }

        let land = self.land.unwrap_or_else(|| theme.text_dim.darken(20));
        let ocean = self.ocean.unwrap_or(theme.bg);
        let accent = theme.accent;
        let border = theme.border;

        if self.fill_background {
            canvas.fill_rect(self.rect, ocean);
        }

        // Nearest-neighbor scale of the embedded silhouette into the map rect.
        let mw = layout.map.w;
        let mh = layout.map.h;
        for py in 0..mh {
            let v = (py as f32 + 0.5) / mh as f32;
            let ty = ((v * WORLD_MAP_H as f32) as u32).min(WORLD_MAP_H - 1);
            for px in 0..mw {
                let u = (px as f32 + 0.5) / mw as f32;
                let tx = ((u * WORLD_MAP_W as f32) as u32).min(WORLD_MAP_W - 1);
                if land_at_texel(tx, ty) {
                    canvas.put_pixel(layout.map.x + px as i32, layout.map.y + py as i32, land);
                }
            }
        }

        canvas.draw_rect(layout.map, border);

        if self.show_grid {
            draw_grid(canvas, layout.map, theme.border.darken(10));
        }

        // Markers
        for m in self.markers {
            if let Some(pt) = geo_to_point(layout.map, m.coord) {
                fill_marker(canvas, pt, 3, accent);
            }
        }

        if let Some(h) = self.hover {
            if let Some(pt) = geo_to_point(layout.map, h) {
                stroke_marker(canvas, pt, 5, accent.lighten(40));
            }
        }

        if let Some(s) = self.selected {
            if let Some(pt) = geo_to_point(layout.map, s) {
                fill_marker(canvas, pt, 4, accent);
                stroke_marker(canvas, pt, 6, accent.lighten(60));
            }
        }
    }
}

fn draw_grid(canvas: &mut Canvas, map: Rect, color: Color) {
    // Meridians every 30°, parallels every 30°.
    for i in 1..12 {
        let x = map.x + (map.w as i32 * i) / 12;
        canvas.vline(x, map.y, map.h, color);
    }
    for i in 1..6 {
        let y = map.y + (map.h as i32 * i) / 6;
        canvas.hline(map.x, y, map.w, color);
    }
}

fn fill_marker(canvas: &mut Canvas, p: Point, r: i32, color: Color) {
    for dy in -r..=r {
        for dx in -r..=r {
            if dx * dx + dy * dy <= r * r {
                canvas.put_pixel(p.x + dx, p.y + dy, color);
            }
        }
    }
}

fn stroke_marker(canvas: &mut Canvas, p: Point, r: i32, color: Color) {
    for dy in -r..=r {
        for dx in -r..=r {
            let d2 = dx * dx + dy * dy;
            if d2 <= r * r && d2 >= (r - 1) * (r - 1) {
                canvas.put_pixel(p.x + dx, p.y + dy, color);
            }
        }
    }
}

// ── Host-side unit tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_preserves_aspect() {
        // Wide bounds → height limited.
        let l = WorldMapLayout::compute(Rect::new(0, 0, 400, 100));
        assert_eq!(l.map.w, 200);
        assert_eq!(l.map.h, 100);
        assert_eq!(l.map.x, 100);

        // Tall bounds → width limited.
        let l2 = WorldMapLayout::compute(Rect::new(0, 0, 100, 400));
        assert_eq!(l2.map.w, 100);
        assert_eq!(l2.map.h, 50);
        assert_eq!(l2.map.y, 175);
    }

    #[test]
    fn point_outside_map_is_none() {
        let map = Rect::new(50, 50, 200, 100);
        assert!(point_to_geo(map, Point::new(10, 10)).is_none());
        assert!(point_to_geo(map, Point::new(49, 80)).is_none());
        assert!(point_to_geo(map, Point::new(250, 80)).is_none());
        assert!(point_to_geo(map, Point::new(100, 49)).is_none());
    }

    #[test]
    fn equator_prime_meridian_center() {
        let map = Rect::new(0, 0, 360, 180);
        let p = geo_to_point(map, GeoCoord::new(0.0, 0.0)).unwrap();
        // Center of 360×180.
        assert!((p.x - 180).abs() <= 1);
        assert!((p.y - 90).abs() <= 1);

        let g = point_to_geo(map, Point::new(180, 90)).unwrap();
        assert!(g.lon.abs() < 1.0);
        assert!(g.lat.abs() < 1.0);
    }

    #[test]
    fn poles_and_dateline() {
        let map = Rect::new(0, 0, 360, 180);
        let n = geo_to_point(map, GeoCoord::new(0.0, 90.0)).unwrap();
        assert_eq!(n.y, 0);
        let s = geo_to_point(map, GeoCoord::new(0.0, -90.0)).unwrap();
        assert_eq!(s.y, 179);

        let west = geo_to_point(map, GeoCoord::new(-180.0, 0.0)).unwrap();
        assert_eq!(west.x, 0);
        let east = geo_to_point(map, GeoCoord::new(180.0, 0.0)).unwrap();
        // 180° maps to right edge (clamped to w-1).
        assert!(east.x >= 359);
    }

    #[test]
    fn projection_round_trip_tolerance() {
        let map = Rect::new(10, 20, 400, 200);
        let samples = [
            GeoCoord::new(0.0, 0.0),
            GeoCoord::new(-74.0, 40.7),   // NYC-ish
            GeoCoord::new(139.7, 35.7),   // Tokyo-ish
            GeoCoord::new(2.35, 48.85),   // Paris-ish
            GeoCoord::new(151.2, -33.9),  // Sydney-ish
            GeoCoord::new(-58.4, -34.6),  // Buenos Aires-ish
            GeoCoord::new(37.6, 55.75),   // Moscow-ish
        ];
        for c in samples {
            let p = geo_to_point(map, c).unwrap();
            let back = point_to_geo(map, p).unwrap();
            assert!(
                (back.lon - c.normalized().lon).abs() < 1.0,
                "lon round-trip {:?} → {:?}",
                c,
                back
            );
            assert!(
                (back.lat - c.normalized().lat).abs() < 1.0,
                "lat round-trip {:?} → {:?}",
                c,
                back
            );
        }
    }

    #[test]
    fn longitude_wrapping() {
        assert!((wrap_lon(190.0) + 170.0).abs() < 0.01);
        assert!((wrap_lon(-190.0) - 170.0).abs() < 0.01);
        assert!((wrap_lon(0.0)).abs() < 0.01);
        assert!((wrap_lon(540.0) - 180.0).abs() < 0.01 || (wrap_lon(540.0) + 180.0).abs() < 0.01);
    }

    #[test]
    fn outside_click_emits_outside() {
        let w = WorldMapWidget::new(Rect::new(0, 0, 400, 100));
        // Letterboxed map is 200×100 centered at x=100..300.
        match w.hit_test(Point::new(10, 50)) {
            MapHit::Outside => {}
            other => panic!("expected Outside, got {other:?}"),
        }
        match w.hit_test(Point::new(200, 50)) {
            MapHit::Inside { .. } => {}
            other => panic!("expected Inside, got {other:?}"),
        }
    }

    #[test]
    fn marker_hit_testing() {
        let map = Rect::new(0, 0, 360, 180);
        let markers = [
            MapMarker {
                coord: GeoCoord::new(0.0, 0.0),
                hit_radius: 8,
            },
            MapMarker {
                coord: GeoCoord::new(90.0, 0.0),
                hit_radius: 8,
            },
        ];
        let p0 = geo_to_point(map, markers[0].coord).unwrap();
        assert_eq!(hit_test_markers(map, p0, &markers), Some(0));
        let p1 = geo_to_point(map, markers[1].coord).unwrap();
        assert_eq!(hit_test_markers(map, p1, &markers), Some(1));
        assert_eq!(hit_test_markers(map, Point::new(10, 10), &markers), None);
    }

    #[test]
    fn silhouette_has_land_and_ocean() {
        // Antarctica band should be land; mid-ocean should often be empty.
        assert!(land_at_texel(60, 58));
        // Top-left corner is ocean in our silhouette.
        assert!(!land_at_texel(0, 0));
    }

    #[test]
    fn draw_no_panic() {
        let theme = Theme::sunlight_dark();
        let mut pixels = vec![0u32; 320 * 160];
        let mut canvas = Canvas::new(&mut pixels, 320, 320, 160);
        let markers = [MapMarker {
            coord: GeoCoord::new(-74.0, 40.7),
            hit_radius: 6,
        }];
        WorldMapWidget::new(Rect::new(10, 10, 300, 140))
            .with_markers(&markers)
            .with_selected(GeoCoord::new(0.0, 0.0))
            .draw(&mut canvas, &theme);
        assert!(pixels.iter().any(|&p| p != 0));
    }

    /// Optional host geometry raster dump (not QEMU visual proof).
    ///
    /// `SUNLIGHT_WRITE_WIDGET_PREVIEW=1 cargo test -p sunlight-ui \
    ///   --target x86_64-unknown-linux-gnu --features std --lib \
    ///   write_combined_widget_preview -- --exact --nocapture`
    #[test]
    fn write_combined_widget_preview() {
        use crate::widgets::{
            DigitalNumberWidget, SolarClockSnapshot, SolarClockWidget,
        };
        if std::env::var_os("SUNLIGHT_WRITE_WIDGET_PREVIEW").is_none() {
            return;
        }
        let w = 640u32;
        let h = 480u32;
        let mut pixels = vec![0u32; (w * h) as usize];
        let mut canvas = Canvas::new(&mut pixels, w, w, h);
        let theme = Theme::sunlight_dark();
        canvas.fill_rect(Rect::new(0, 0, w, h), theme.bg);

        let mut dig = DigitalNumberWidget::new(Rect::new(20, 20, 200, 40))
            .with_digit_size(16, 28)
            .with_max_chars(5)
            .with_value("04:05");
        dig.draw(&mut canvas, &theme);

        SolarClockWidget::new(
            Rect::new(40, 80, 180, 180),
            SolarClockSnapshot::new(9, 45, 45),
        )
        .with_date("Mon 25 Jul")
        .draw(&mut canvas, &theme);

        let markers = [MapMarker {
            coord: GeoCoord::new(-74.0, 40.7),
            hit_radius: 6,
        }];
        WorldMapWidget::new(Rect::new(240, 80, 360, 180))
            .with_markers(&markers)
            .with_selected(GeoCoord::new(0.0, 0.0))
            .draw(&mut canvas, &theme);

        let path = "target/widget_gallery_host_preview.ppm";
        let mut out = String::with_capacity((w * h * 12) as usize);
        out.push_str(&format!("P3\n{w} {h}\n255\n"));
        for p in &pixels {
            let r = ((p >> 16) & 0xFF) as u8;
            let g = ((p >> 8) & 0xFF) as u8;
            let b = (*p & 0xFF) as u8;
            out.push_str(&format!("{r} {g} {b}\n"));
        }
        std::fs::create_dir_all("target").ok();
        std::fs::write(path, out).expect("write preview ppm");
        eprintln!("host geometry preview written to {path}");
    }
}
