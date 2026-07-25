//! Sunlight-style circular clock without traditional hands.
//!
//! The parent supplies a plain [`SolarClockSnapshot`]; this widget never
//! queries RTC, Time Service, Timezone Service, or NTP. Progressive
//! sun-ray markers encode the second; circular tracks encode minute and
//! hour progress. Center digital time uses [`DigitalNumberWidget`].

use crate::geom::{Rect, Size};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};
use crate::widgets::digital_number::{DigitalAlign, DigitalNumberWidget};

/// Fixed-point scale for precomputed unit vectors (≈1.0).
const RAY_SCALE: i32 = 1024;

/// Unit vectors for sixty rays: index 0 at 12 o'clock, advancing clockwise.
/// Each entry is `(dx, dy)` in Q10 fixed point (`/ 1024`).
///
/// Generated once offline; paint only scales and plots — no runtime trig.
pub static RAY_UNIT: [(i16, i16); 60] = [
    (0, -1024),
    (107, -1018),
    (213, -1002),
    (316, -974),
    (416, -935),
    (512, -887),
    (602, -828),
    (685, -761),
    (761, -685),
    (828, -602),
    (887, -512),
    (935, -416),
    (974, -316),
    (1002, -213),
    (1018, -107),
    (1024, 0),
    (1018, 107),
    (1002, 213),
    (974, 316),
    (935, 416),
    (887, 512),
    (828, 602),
    (761, 685),
    (685, 761),
    (602, 828),
    (512, 887),
    (416, 935),
    (316, 974),
    (213, 1002),
    (107, 1018),
    (0, 1024),
    (-107, 1018),
    (-213, 1002),
    (-316, 974),
    (-416, 935),
    (-512, 887),
    (-602, 828),
    (-685, 761),
    (-761, 685),
    (-828, 602),
    (-887, 512),
    (-935, 416),
    (-974, 316),
    (-1002, 213),
    (-1018, 107),
    (-1024, 0),
    (-1018, -107),
    (-1002, -213),
    (-974, -316),
    (-935, -416),
    (-887, -512),
    (-828, -602),
    (-761, -685),
    (-685, -761),
    (-602, -828),
    (-512, -887),
    (-416, -935),
    (-316, -974),
    (-213, -1002),
    (-107, -1018),
];

/// Major markers at 12, 3, 6, 9 (indices 0, 15, 30, 45).
#[inline]
pub fn is_major_ray(index: usize) -> bool {
    index % 15 == 0
}

/// Caller-owned clock input. Completely replace on NTP/timezone jumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolarClockSnapshot {
    /// Hour in 0..=23 (display uses 12-hour cycle for the hour track).
    pub hour: u8,
    /// Minute in 0..=59.
    pub minute: u8,
    /// Second in 0..=59.
    pub second: u8,
}

impl SolarClockSnapshot {
    pub fn new(hour: u8, minute: u8, second: u8) -> Self {
        Self {
            hour: hour.min(23),
            minute: minute.min(59),
            second: second.min(59),
        }
    }

    /// Clamp fields into valid ranges.
    pub fn normalized(self) -> Self {
        Self::new(self.hour, self.minute, self.second)
    }
}

/// Which dynamic layers differ between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SolarClockDirty {
    pub second_rays: bool,
    pub minute_track: bool,
    pub hour_track: bool,
    pub digital: bool,
    pub date: bool,
}

impl SolarClockDirty {
    pub fn any(self) -> bool {
        self.second_rays || self.minute_track || self.hour_track || self.digital || self.date
    }

    pub fn full() -> Self {
        Self {
            second_rays: true,
            minute_track: true,
            hour_track: true,
            digital: true,
            date: true,
        }
    }
}

/// Number of sun rays that should be lit for `second` (rays `0..=second`).
#[inline]
pub fn active_second_rays(second: u8) -> u8 {
    second.min(59).saturating_add(1)
}

/// Minute progress in `0.0..1.0` (includes second fraction).
#[inline]
pub fn minute_progress(minute: u8, second: u8) -> f32 {
    let m = minute.min(59) as f32;
    let s = second.min(59) as f32;
    (m * 60.0 + s) / 3600.0
}

/// Hour progress over a 12-hour cycle in `0.0..1.0` (includes minute fraction).
#[inline]
pub fn hour_progress_12(hour: u8, minute: u8) -> f32 {
    let h = (hour % 12) as f32;
    let m = minute.min(59) as f32;
    (h * 60.0 + m) / (12.0 * 60.0)
}

/// Format `HH:MM` into a fixed 5-byte buffer (always zero-padded).
pub fn format_hhmm(hour: u8, minute: u8, out: &mut [u8; 5]) {
    let h = hour.min(23);
    let m = minute.min(59);
    out[0] = b'0' + h / 10;
    out[1] = b'0' + h % 10;
    out[2] = b':';
    out[3] = b'0' + m / 10;
    out[4] = b'0' + m % 10;
}

/// Diff two snapshots into dirty layer flags (for partial invalidation).
pub fn snapshot_dirty(prev: SolarClockSnapshot, next: SolarClockSnapshot) -> SolarClockDirty {
    let prev = prev.normalized();
    let next = next.normalized();
    if prev == next {
        return SolarClockDirty::default();
    }
    SolarClockDirty {
        second_rays: prev.second != next.second,
        minute_track: prev.minute != next.minute || prev.second != next.second,
        hour_track: prev.hour != next.hour || prev.minute != next.minute,
        digital: prev.hour != next.hour || prev.minute != next.minute,
        date: false,
    }
}

/// Approximate axis-aligned dirty rect for a single ray (parent may union).
pub fn ray_dirty_rect(layout: &SolarClockLayout, ray_index: usize) -> Rect {
    let i = ray_index % 60;
    let (dx, dy) = RAY_UNIT[i];
    let major = is_major_ray(i);
    let len = if major {
        layout.major_ray_len
    } else {
        layout.minor_ray_len
    } as i32;
    let inner = layout.ray_inner_r as i32;
    let outer = inner + len;
    let cx = layout.cx;
    let cy = layout.cy;
    // Endpoints of the ray segment.
    let x0 = cx + (dx as i32 * inner) / RAY_SCALE;
    let y0 = cy + (dy as i32 * inner) / RAY_SCALE;
    let x1 = cx + (dx as i32 * outer) / RAY_SCALE;
    let y1 = cy + (dy as i32 * outer) / RAY_SCALE;
    let pad = if major { 3 } else { 2 };
    let min_x = x0.min(x1) - pad;
    let min_y = y0.min(y1) - pad;
    let max_x = x0.max(x1) + pad;
    let max_y = y0.max(y1) + pad;
    Rect::new(
        min_x,
        min_y,
        (max_x - min_x).max(1) as u32,
        (max_y - min_y).max(1) as u32,
    )
}

/// Geometry derived from the widget rect (letterboxed circle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SolarClockLayout {
    pub bounds: Rect,
    pub cx: i32,
    pub cy: i32,
    pub radius: u32,
    pub ray_inner_r: u32,
    pub major_ray_len: u32,
    pub minor_ray_len: u32,
    pub minute_track_r: u32,
    pub hour_track_r: u32,
    pub digital: Rect,
    pub date: Rect,
}

impl SolarClockLayout {
    pub fn compute(bounds: Rect) -> Self {
        let side = bounds.w.min(bounds.h);
        let radius = side / 2;
        let cx = bounds.x + bounds.w as i32 / 2;
        let cy = bounds.y + bounds.h as i32 / 2;

        let major_ray_len = (radius / 7).max(4).min(18);
        let minor_ray_len = (major_ray_len * 2 / 3).max(3);
        let ray_band = major_ray_len + 2;
        let ray_inner_r = radius.saturating_sub(ray_band);

        let minute_track_r = ray_inner_r.saturating_sub((radius / 16).max(2));
        let hour_track_r = minute_track_r.saturating_sub((radius / 12).max(3));

        // Digital HH:MM band in the center.
        let dig_h = (radius / 4).max(14).min(36);
        let dig_w = (dig_h * 5 * 3 / 5) + 16; // rough 5 cells
        let dig_w = dig_w.min(radius.saturating_mul(5) / 4);
        let digital = Rect::new(cx - dig_w as i32 / 2, cy - dig_h as i32 / 2 - 2, dig_w, dig_h);

        let date_h = 14u32;
        let date = Rect::new(
            cx - (radius as i32 * 3 / 5),
            digital.bottom() + 2,
            (radius * 6 / 5).max(40),
            date_h,
        );

        Self {
            bounds,
            cx,
            cy,
            radius,
            ray_inner_r,
            major_ray_len,
            minor_ray_len,
            minute_track_r,
            hour_track_r,
            digital,
            date,
        }
    }
}

/// Circular monochrome solar clock.
pub struct SolarClockWidget<'a> {
    pub rect: Rect,
    pub snapshot: SolarClockSnapshot,
    /// Optional date line under digital time (caller-owned).
    pub date_text: Option<&'a str>,
    /// Override accent for active rays / tracks.
    pub accent: Option<Color>,
    /// Inactive ray / track color.
    pub muted: Option<Color>,
    /// Face fill. `None` → panel.
    pub face: Option<Color>,
}

impl<'a> SolarClockWidget<'a> {
    pub fn new(rect: Rect, snapshot: SolarClockSnapshot) -> Self {
        Self {
            rect,
            snapshot: snapshot.normalized(),
            date_text: None,
            accent: None,
            muted: None,
            face: None,
        }
    }

    pub fn with_date(mut self, text: &'a str) -> Self {
        self.date_text = Some(text);
        self
    }

    pub fn with_snapshot(mut self, snapshot: SolarClockSnapshot) -> Self {
        self.snapshot = snapshot.normalized();
        self
    }

    pub fn set_snapshot(&mut self, snapshot: SolarClockSnapshot) {
        self.snapshot = snapshot.normalized();
    }

    pub fn layout(&self) -> SolarClockLayout {
        SolarClockLayout::compute(self.rect)
    }

    /// Preferred square size for a given outer diameter.
    pub fn preferred_size(diameter: u32) -> Size {
        Size::new(diameter, diameter)
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let layout = self.layout();
        if layout.radius < 8 {
            return;
        }

        let accent = self.accent.unwrap_or(theme.accent);
        let muted = self
            .muted
            .unwrap_or_else(|| theme.border.lighten(20));
        let face = self.face.unwrap_or(theme.panel);
        let edge = theme.border;

        // Static face.
        fill_circle(canvas, layout.cx, layout.cy, layout.radius as i32, face);
        stroke_circle(canvas, layout.cx, layout.cy, layout.radius as i32, 2, edge);

        // Inactive minute / hour tracks (static ring).
        stroke_circle(
            canvas,
            layout.cx,
            layout.cy,
            layout.minute_track_r as i32,
            1,
            muted.darken(10),
        );
        stroke_circle(
            canvas,
            layout.cx,
            layout.cy,
            layout.hour_track_r as i32,
            2,
            muted.darken(20),
        );

        // All sixty rays: inactive first, then active overwrite.
        let active = active_second_rays(self.snapshot.second) as usize;
        for i in 0..60 {
            let color = if i < active { accent } else { muted };
            draw_ray(canvas, &layout, i, color);
        }

        // Minute progress arc (thin), hour progress arc (thicker).
        let min_p = minute_progress(self.snapshot.minute, self.snapshot.second);
        let hour_p = hour_progress_12(self.snapshot.hour, self.snapshot.minute);
        draw_progress_arc(
            canvas,
            layout.cx,
            layout.cy,
            layout.minute_track_r as i32,
            min_p,
            2,
            accent,
        );
        draw_progress_arc(
            canvas,
            layout.cx,
            layout.cy,
            layout.hour_track_r as i32,
            hour_p,
            3,
            accent.lighten(30),
        );

        // Digital HH:MM via DigitalNumberWidget.
        let mut hhmm = [0u8; 5];
        format_hhmm(self.snapshot.hour, self.snapshot.minute, &mut hhmm);
        let hhmm_str = core::str::from_utf8(&hhmm).unwrap_or("00:00");

        let dig_h = layout.digital.h.max(12);
        let dig_w = (dig_h * 3 / 5).max(6);
        let mut digital = DigitalNumberWidget::new(layout.digital)
            .with_digit_size(dig_w, dig_h)
            .with_spacing((dig_w / 5).max(1))
            .with_max_chars(5)
            .with_align(DigitalAlign::Center)
            .with_colors(theme.text, theme.panel_alt.lighten(12));
        let _ = digital.set_value_str(hhmm_str);
        digital.draw(canvas, theme);

        if let Some(date) = self.date_text {
            if !date.is_empty() {
                canvas.draw_text_centered(layout.date, date, theme.text_dim);
            }
        }
    }
}

fn draw_ray(canvas: &mut Canvas, layout: &SolarClockLayout, index: usize, color: Color) {
    let (dx, dy) = RAY_UNIT[index % 60];
    let major = is_major_ray(index);
    let len = if major {
        layout.major_ray_len
    } else {
        layout.minor_ray_len
    } as i32;
    let thick = if major { 2u32 } else { 1u32 };
    let inner = layout.ray_inner_r as i32;
    let outer = inner + len;
    let x0 = layout.cx + (dx as i32 * inner) / RAY_SCALE;
    let y0 = layout.cy + (dy as i32 * inner) / RAY_SCALE;
    let x1 = layout.cx + (dx as i32 * outer) / RAY_SCALE;
    let y1 = layout.cy + (dy as i32 * outer) / RAY_SCALE;
    draw_line_thick(canvas, x0, y0, x1, y1, thick, color);
}

/// Draw an arc from 12 o'clock clockwise covering `progress` of a full circle.
fn draw_progress_arc(
    canvas: &mut Canvas,
    cx: i32,
    cy: i32,
    radius: i32,
    progress: f32,
    thickness: u32,
    color: Color,
) {
    if radius <= 0 || progress <= 0.0 {
        return;
    }
    let progress = if progress > 1.0 { 1.0 } else { progress };
    // 60 samples match ray table — reuse unit vectors (no libm ceil/floor).
    // steps = ceil(progress * 60) via integer math on milli-units.
    let milli = (progress * 1000.0) as u32;
    let steps = ((milli * 60 + 999) / 1000).clamp(1, 60) as usize;
    for i in 0..steps {
        let (dx, dy) = RAY_UNIT[i.min(59)];
        let x = cx + (dx as i32 * radius) / RAY_SCALE;
        let y = cy + (dy as i32 * radius) / RAY_SCALE;
        let t = thickness.max(1) as i32;
        for by in 0..t {
            for bx in 0..t {
                canvas.put_pixel(x + bx - t / 2, y + by - t / 2, color);
            }
        }
        // Connect consecutive samples with a short stroke for continuity.
        if i > 0 {
            let (pdx, pdy) = RAY_UNIT[i - 1];
            let px = cx + (pdx as i32 * radius) / RAY_SCALE;
            let py = cy + (pdy as i32 * radius) / RAY_SCALE;
            draw_line_thick(canvas, px, py, x, y, thickness, color);
        }
    }
}

fn draw_line_thick(
    canvas: &mut Canvas,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: u32,
    color: Color,
) {
    let mut x = x0;
    let mut y = y0;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let brush = thickness.max(1) as i32;
    loop {
        for by in 0..brush {
            for bx in 0..brush {
                canvas.put_pixel(x + bx - brush / 2, y + by - brush / 2, color);
            }
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn fill_circle(canvas: &mut Canvas, cx: i32, cy: i32, radius: i32, color: Color) {
    if radius <= 0 {
        return;
    }
    // Midpoint fill via horizontal spans (faster than per-pixel circle test).
    let mut x = radius;
    let mut y = 0;
    let mut err = 1 - x;
    while x >= y {
        hspan(canvas, cx - x, cy + y, (2 * x + 1) as u32, color);
        hspan(canvas, cx - x, cy - y, (2 * x + 1) as u32, color);
        hspan(canvas, cx - y, cy + x, (2 * y + 1) as u32, color);
        hspan(canvas, cx - y, cy - x, (2 * y + 1) as u32, color);
        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

fn hspan(canvas: &mut Canvas, x: i32, y: i32, len: u32, color: Color) {
    canvas.hline(x, y, len, color);
}

fn stroke_circle(
    canvas: &mut Canvas,
    cx: i32,
    cy: i32,
    radius: i32,
    stroke: u32,
    color: Color,
) {
    if radius <= 0 {
        return;
    }
    for s in 0..stroke.max(1) {
        let r = radius - s as i32;
        if r <= 0 {
            continue;
        }
        let mut x = r;
        let mut y = 0;
        let mut err = 1 - x;
        while x >= y {
            plot8(canvas, cx, cy, x, y, color);
            y += 1;
            if err < 0 {
                err += 2 * y + 1;
            } else {
                x -= 1;
                err += 2 * (y - x) + 1;
            }
        }
    }
}

fn plot8(canvas: &mut Canvas, cx: i32, cy: i32, x: i32, y: i32, color: Color) {
    canvas.put_pixel(cx + x, cy + y, color);
    canvas.put_pixel(cx + y, cy + x, color);
    canvas.put_pixel(cx - y, cy + x, color);
    canvas.put_pixel(cx - x, cy + y, color);
    canvas.put_pixel(cx - x, cy - y, color);
    canvas.put_pixel(cx - y, cy - x, color);
    canvas.put_pixel(cx + y, cy - x, color);
    canvas.put_pixel(cx + x, cy - y, color);
}

// ── Host-side unit tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_ray_activation_counts() {
        assert_eq!(active_second_rays(0), 1);
        assert_eq!(active_second_rays(1), 2);
        assert_eq!(active_second_rays(15), 16);
        assert_eq!(active_second_rays(59), 60);
        assert_eq!(active_second_rays(255), 60);
    }

    #[test]
    fn minute_hour_progress_normalization() {
        assert!((minute_progress(0, 0) - 0.0).abs() < 1e-6);
        assert!((minute_progress(30, 0) - 0.5).abs() < 1e-6);
        assert!((minute_progress(59, 59) - 1.0).abs() < 0.001);

        assert!((hour_progress_12(0, 0) - 0.0).abs() < 1e-6);
        assert!((hour_progress_12(6, 0) - 0.5).abs() < 1e-6);
        assert!((hour_progress_12(12, 0) - 0.0).abs() < 1e-6); // 12 ≡ 0
        assert!((hour_progress_12(18, 0) - 0.5).abs() < 1e-6);
        assert!((hour_progress_12(3, 30) - (3.5 / 12.0)).abs() < 1e-5);
    }

    #[test]
    fn midnight_and_noon_boundaries() {
        let midnight = SolarClockSnapshot::new(0, 0, 0);
        let noon = SolarClockSnapshot::new(12, 0, 0);
        assert_eq!(active_second_rays(midnight.second), 1);
        assert!((minute_progress(midnight.minute, midnight.second) - 0.0).abs() < 1e-6);
        assert!((hour_progress_12(midnight.hour, midnight.minute) - 0.0).abs() < 1e-6);
        assert!((hour_progress_12(noon.hour, noon.minute) - 0.0).abs() < 1e-6);

        let almost = SolarClockSnapshot::new(23, 59, 59);
        assert_eq!(active_second_rays(almost.second), 60);
        assert!((minute_progress(almost.minute, almost.second) - 1.0).abs() < 0.001);
        // 23:59 → 11:59 on 12h track ≈ almost full
        assert!(hour_progress_12(almost.hour, almost.minute) > 0.99);
    }

    #[test]
    fn snapshot_dirty_layers() {
        let a = SolarClockSnapshot::new(10, 20, 30);
        let b = SolarClockSnapshot::new(10, 20, 31);
        let d = snapshot_dirty(a, b);
        assert!(d.second_rays);
        assert!(d.minute_track);
        assert!(!d.hour_track);
        assert!(!d.digital);

        let c = SolarClockSnapshot::new(10, 21, 0);
        let d2 = snapshot_dirty(a, c);
        assert!(d2.second_rays);
        assert!(d2.minute_track);
        assert!(d2.hour_track);
        assert!(d2.digital);

        assert!(!snapshot_dirty(a, a).any());
    }

    #[test]
    fn major_rays_at_cardinals() {
        assert!(is_major_ray(0));
        assert!(is_major_ray(15));
        assert!(is_major_ray(30));
        assert!(is_major_ray(45));
        assert!(!is_major_ray(1));
        assert!(!is_major_ray(14));
    }

    #[test]
    fn format_hhmm_zero_padded() {
        let mut buf = [0u8; 5];
        format_hhmm(3, 5, &mut buf);
        assert_eq!(&buf, b"03:05");
        format_hhmm(23, 59, &mut buf);
        assert_eq!(&buf, b"23:59");
        format_hhmm(0, 0, &mut buf);
        assert_eq!(&buf, b"00:00");
    }

    #[test]
    fn ray_table_has_unit_length() {
        for &(dx, dy) in RAY_UNIT.iter() {
            let len2 = (dx as i32) * (dx as i32) + (dy as i32) * (dy as i32);
            // ≈ 1024²
            assert!((len2 - RAY_SCALE * RAY_SCALE).abs() < 3000, "len2={len2}");
        }
        assert_eq!(RAY_UNIT[0], (0, -1024));
        assert_eq!(RAY_UNIT[15], (1024, 0));
        assert_eq!(RAY_UNIT[30], (0, 1024));
        assert_eq!(RAY_UNIT[45], (-1024, 0));
    }

    #[test]
    fn draw_representative_times_no_panic() {
        let theme = Theme::sunlight_dark();
        let mut pixels = vec![0u32; 200 * 200];
        let times = [
            (0, 0, 0),
            (3, 15, 15),
            (6, 30, 30),
            (9, 45, 45),
            (23, 59, 59),
        ];
        for (h, m, s) in times {
            let mut canvas = Canvas::new(&mut pixels, 200, 200, 200);
            SolarClockWidget::new(
                Rect::new(10, 10, 180, 180),
                SolarClockSnapshot::new(h, m, s),
            )
            .with_date("Mon 1 Jan")
            .draw(&mut canvas, &theme);
        }
        assert!(pixels.iter().any(|&p| p != 0));
    }

    #[test]
    fn layout_is_square_centered() {
        let layout = SolarClockLayout::compute(Rect::new(0, 0, 200, 100));
        assert_eq!(layout.radius, 50);
        assert_eq!(layout.cx, 100);
        assert_eq!(layout.cy, 50);
    }
}
