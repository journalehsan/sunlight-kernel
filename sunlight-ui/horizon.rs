//! Sunlight Horizon window controls — compositor-owned vector chrome glyphs.
//!
//! Physical order on the **right** side of the titlebar (stable in LTR and RTL):
//!
//!     Pin  |  Minimize   Maximize/Restore   Close
//!
//! Glyphs are pure geometry derived from the button rectangle — no font
//! baseline dependency. Hit targets are the full button rectangles.

use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

/// Horizon control kinds in left-to-right physical order within the strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HorizonControl {
    Pin = 0,
    Minimize = 1,
    Maximize = 2,
    Restore = 3,
    Close = 4,
}

/// Visual interaction state for a single control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HorizonControlState {
    /// Quiet: glyph only, no backplate.
    Rest,
    /// Full button backplate (neutral glass or close terracotta).
    Hover,
    /// Distinct from hover (darker backplate).
    Pressed,
    /// Keyboard focus ring.
    Focused,
}

/// Metrics for the control strip (resolution-independent via fixed logical px).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HorizonMetrics {
    pub button_size: u32,
    pub button_spacing: u32,
    /// Extra gap between Pin and the standard three controls.
    pub pin_gap: u32,
    pub radius: u32,
}

impl Default for HorizonMetrics {
    fn default() -> Self {
        Self {
            button_size: 20,
            button_spacing: 4,
            pin_gap: 8,
            radius: 5,
        }
    }
}

/// Canonical physical order for a normal decorated window (left → right).
pub const HORIZON_ORDER_LTR: [HorizonControl; 4] = [
    HorizonControl::Pin,
    HorizonControl::Minimize,
    HorizonControl::Maximize,
    HorizonControl::Close,
];

/// Physical order is intentionally identical under RTL locales.
pub const HORIZON_ORDER_RTL: [HorizonControl; 4] = HORIZON_ORDER_LTR;

/// Colors for Horizon controls. Derived from theme with Sunlight accents.
#[derive(Debug, Clone, Copy)]
pub struct HorizonPalette {
    pub icon_rest: Color,
    pub icon_active_window: Color,
    pub icon_inactive_window: Color,
    pub hover_fill: Color,
    pub pressed_fill: Color,
    pub close_hover_fill: Color,
    pub close_pressed_fill: Color,
    pub pin_active: Color,
    pub focus_ring: Color,
    pub divider: Color,
}

impl HorizonPalette {
    pub fn from_theme(theme: &Theme) -> Self {
        Self {
            icon_rest: theme.icon_muted,
            icon_active_window: theme.icon_foreground,
            icon_inactive_window: theme.icon_disabled.lighten(40),
            // Neutral glass highlight
            hover_fill: Color::rgba(0x3A, 0x3A, 0x4A, 200),
            pressed_fill: Color::rgba(0x28, 0x28, 0x34, 230),
            // Restrained dark terracotta / red for close
            close_hover_fill: Color::rgba(0x8A, 0x3A, 0x32, 220),
            close_pressed_fill: Color::rgba(0x6E, 0x2A, 0x24, 235),
            pin_active: theme.accent, // Sunlight orange
            focus_ring: theme.accent_hover,
            divider: theme.border.lighten(12),
        }
    }
}

/// Layout rectangles for the normal four-control strip on the right.
///
/// `chrome_w` is the full decoration width including borders.
/// Controls are always on the physical right; `rtl` does not reverse them.
pub fn layout_controls(
    wx: i32,
    wy: i32,
    chrome_w: u32,
    titlebar_h: u32,
    metrics: HorizonMetrics,
    maximized: bool,
    _rtl: bool,
) -> HorizonLayout {
    let btn = metrics.button_size;
    let gap = metrics.button_spacing;
    let pin_gap = metrics.pin_gap;
    // Right padding so the close button is not flush against the edge.
    let right_pad = gap;
    let y = wy + (titlebar_h.saturating_sub(btn) as i32) / 2;

    // From right: Close, Max/Restore, Min, [pin_gap], Pin
    let close_x = wx + chrome_w as i32 - right_pad as i32 - btn as i32;
    let max_x = close_x - gap as i32 - btn as i32;
    let min_x = max_x - gap as i32 - btn as i32;
    let pin_x = min_x - pin_gap as i32 - btn as i32;

    let maximize_kind = if maximized {
        HorizonControl::Restore
    } else {
        HorizonControl::Maximize
    };

    HorizonLayout {
        pin: Rect::new(pin_x, y, btn, btn),
        minimize: Rect::new(min_x, y, btn, btn),
        maximize: Rect::new(max_x, y, btn, btn),
        close: Rect::new(close_x, y, btn, btn),
        maximize_kind,
        // Divider between pin and min
        divider_x: pin_x + btn as i32 + (pin_gap as i32) / 2,
        divider_y0: y + 4,
        divider_y1: y + btn as i32 - 4,
        metrics,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct HorizonLayout {
    pub pin: Rect,
    pub minimize: Rect,
    pub maximize: Rect,
    pub close: Rect,
    pub maximize_kind: HorizonControl,
    pub divider_x: i32,
    pub divider_y0: i32,
    pub divider_y1: i32,
    pub metrics: HorizonMetrics,
}

impl HorizonLayout {
    pub fn rect_for(&self, control: HorizonControl) -> Rect {
        match control {
            HorizonControl::Pin => self.pin,
            HorizonControl::Minimize => self.minimize,
            HorizonControl::Maximize | HorizonControl::Restore => self.maximize,
            HorizonControl::Close => self.close,
        }
    }

    /// Hit-test pointer against control rects. Full rects are hit targets.
    pub fn hit_test(&self, x: i32, y: i32) -> Option<HorizonControl> {
        let p = crate::geom::Point::new(x, y);
        if self.close.contains(p) {
            return Some(HorizonControl::Close);
        }
        if self.maximize.contains(p) {
            return Some(self.maximize_kind);
        }
        if self.minimize.contains(p) {
            return Some(HorizonControl::Minimize);
        }
        if self.pin.contains(p) {
            return Some(HorizonControl::Pin);
        }
        None
    }

    /// Left edge of the control strip (for title text clipping).
    pub fn strip_left(&self) -> i32 {
        self.pin.x - self.metrics.button_spacing as i32
    }
}

fn same_control_slot(a: HorizonControl, b: HorizonControl) -> bool {
    match (a, b) {
        (HorizonControl::Maximize, HorizonControl::Restore)
        | (HorizonControl::Restore, HorizonControl::Maximize) => true,
        _ => a == b,
    }
}

fn resolve_state(
    control: HorizonControl,
    hover: Option<HorizonControl>,
    pressed: Option<HorizonControl>,
    focused: Option<HorizonControl>,
) -> HorizonControlState {
    if pressed.is_some_and(|p| same_control_slot(p, control)) {
        HorizonControlState::Pressed
    } else if focused.is_some_and(|f| same_control_slot(f, control)) {
        HorizonControlState::Focused
    } else if hover.is_some_and(|h| same_control_slot(h, control)) {
        HorizonControlState::Hover
    } else {
        HorizonControlState::Rest
    }
}

/// Draw the pin/min/max/close strip including optional divider.
pub fn draw_control_strip(
    canvas: &mut Canvas<'_>,
    layout: &HorizonLayout,
    palette: &HorizonPalette,
    window_active: bool,
    pin_active: bool,
    hover: Option<HorizonControl>,
    pressed: Option<HorizonControl>,
    focused: Option<HorizonControl>,
) {
    // Subtle vertical divider between pin and the standard three.
    if layout.divider_y1 > layout.divider_y0 {
        canvas.vline(
            layout.divider_x,
            layout.divider_y0,
            (layout.divider_y1 - layout.divider_y0) as u32,
            palette.divider,
        );
    }

    for control in [
        HorizonControl::Pin,
        HorizonControl::Minimize,
        layout.maximize_kind,
        HorizonControl::Close,
    ] {
        let state = resolve_state(control, hover, pressed, focused);
        let accent = control == HorizonControl::Pin && pin_active;
        draw_control(
            canvas,
            layout.rect_for(control),
            control,
            state,
            window_active,
            accent,
            palette,
            layout.metrics.radius,
        );
    }
}

/// Draw a single Horizon control into `rect`.
pub fn draw_control(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    control: HorizonControl,
    state: HorizonControlState,
    window_active: bool,
    accent_active: bool,
    palette: &HorizonPalette,
    radius: u32,
) {
    // Backplate only on hover/pressed/focused — quiet at rest.
    match state {
        HorizonControlState::Rest => {}
        HorizonControlState::Hover => {
            let fill = if control == HorizonControl::Close {
                palette.close_hover_fill
            } else {
                palette.hover_fill
            };
            canvas.blend_rounded_rect(rect, radius, fill);
        }
        HorizonControlState::Pressed => {
            let fill = if control == HorizonControl::Close {
                palette.close_pressed_fill
            } else {
                palette.pressed_fill
            };
            canvas.blend_rounded_rect(rect, radius, fill);
        }
        HorizonControlState::Focused => {
            let fill = palette.hover_fill;
            canvas.blend_rounded_rect(rect, radius, fill);
            canvas.stroke_rounded_rect(rect, radius, 1, palette.focus_ring);
        }
    }

    let mut icon = if window_active {
        palette.icon_active_window
    } else {
        palette.icon_inactive_window
    };
    if control == HorizonControl::Pin && accent_active {
        icon = palette.pin_active;
    }
    if state == HorizonControlState::Hover && control == HorizonControl::Close {
        icon = Color::rgb(0xF5, 0xE6, 0xE4); // lighter on terracotta
    }

    draw_glyph(canvas, rect, control, icon, accent_active, palette);
}

/// Resolution-independent vector glyphs derived from the button rectangle.
pub fn draw_glyph(
    canvas: &mut Canvas<'_>,
    rect: Rect,
    control: HorizonControl,
    color: Color,
    pin_lit: bool,
    palette: &HorizonPalette,
) {
    // Inset content box (~55% of button) for optical centering.
    let pad_x = (rect.w.saturating_mul(22) / 100).max(3) as i32;
    let pad_y = (rect.h.saturating_mul(22) / 100).max(3) as i32;
    let box_r = Rect::new(
        rect.x + pad_x,
        rect.y + pad_y,
        rect.w.saturating_sub((pad_x * 2) as u32).max(4),
        rect.h.saturating_sub((pad_y * 2) as u32).max(4),
    );
    let stroke = stroke_thickness(rect);

    match control {
        HorizonControl::Minimize => {
            // Horizon line in the lower third of the content box.
            let y = box_r.y + (box_r.h as i32 * 2) / 3;
            let x0 = box_r.x + 1;
            let len = box_r.w.saturating_sub(2).max(2);
            for t in 0..stroke {
                canvas.hline(x0, y + t as i32, len, color);
            }
        }
        HorizonControl::Maximize => {
            // Thin outlined frame.
            stroke_rect_inset(canvas, box_r, stroke, color);
        }
        HorizonControl::Restore => {
            // Two slightly offset overlapping frames (echo).
            let dx = (box_r.w / 5).max(2) as i32;
            let dy = (box_r.h / 5).max(2) as i32;
            let back = Rect::new(
                box_r.x + dx,
                box_r.y,
                box_r.w.saturating_sub(dx as u32).max(3),
                box_r.h.saturating_sub(dy as u32).max(3),
            );
            let front = Rect::new(
                box_r.x,
                box_r.y + dy,
                box_r.w.saturating_sub(dx as u32).max(3),
                box_r.h.saturating_sub(dy as u32).max(3),
            );
            stroke_rect_inset(canvas, back, stroke, color);
            // Fill front interior with transparent (overwrite back edges) then stroke.
            // Clear front fill by re-drawing with a hole is hard on XRGB; just stroke both.
            stroke_rect_inset(canvas, front, stroke, color);
        }
        HorizonControl::Close => {
            // Clean diagonal cross.
            let x0 = box_r.x;
            let y0 = box_r.y;
            let x1 = box_r.right() - 1;
            let y1 = box_r.bottom() - 1;
            draw_line_thick(canvas, x0, y0, x1, y1, stroke, color);
            draw_line_thick(canvas, x1, y0, x0, y1, stroke, color);
        }
        HorizonControl::Pin => {
            draw_pin_glyph(canvas, box_r, stroke, color, pin_lit, palette.pin_active);
        }
    }
}

fn stroke_thickness(rect: Rect) -> u32 {
    // ~1px at 20px buttons; scale gently for larger titlebars.
    let s = rect.w.min(rect.h);
    if s >= 28 {
        2
    } else {
        1
    }
}

fn stroke_rect_inset(canvas: &mut Canvas<'_>, r: Rect, stroke: u32, color: Color) {
    if r.w < 2 || r.h < 2 {
        return;
    }
    for t in 0..stroke {
        let rr = Rect::new(
            r.x + t as i32,
            r.y + t as i32,
            r.w.saturating_sub(t * 2).max(1),
            r.h.saturating_sub(t * 2).max(1),
        );
        canvas.draw_rect(rr, color);
    }
}

fn draw_line_thick(
    canvas: &mut Canvas<'_>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    thickness: u32,
    color: Color,
) {
    // Bresenham with a small orthogonal brush for thickness.
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

/// Minimal pin: round head + shaft. Active state may light the center orange.
fn draw_pin_glyph(
    canvas: &mut Canvas<'_>,
    box_r: Rect,
    stroke: u32,
    color: Color,
    lit: bool,
    accent: Color,
) {
    let cx = box_r.x + box_r.w as i32 / 2;
    let head_r = (box_r.w.min(box_r.h) / 4).max(2) as i32;
    let head_cy = box_r.y + head_r + 1;
    // Head outline (circle approximation)
    draw_circle_outline(canvas, cx, head_cy, head_r, stroke, color);
    if lit {
        // Orange center light
        let ir = (head_r - 1).max(1);
        fill_circle(canvas, cx, head_cy, ir, accent);
    }
    // Shaft
    let shaft_top = head_cy + head_r;
    let shaft_bot = box_r.bottom() - 2;
    for t in 0..stroke.max(1) {
        canvas.vline(
            cx - (stroke as i32) / 2 + t as i32,
            shaft_top,
            (shaft_bot - shaft_top).max(1) as u32,
            color,
        );
    }
    // Small base flare
    let base_w = (box_r.w / 3).max(2) as i32;
    canvas.hline(cx - base_w / 2, shaft_bot, base_w as u32, color);
}

fn draw_circle_outline(
    canvas: &mut Canvas<'_>,
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

fn fill_circle(canvas: &mut Canvas<'_>, cx: i32, cy: i32, radius: i32, color: Color) {
    if radius <= 0 {
        return;
    }
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= radius * radius {
                canvas.put_pixel(cx + dx, cy + dy, color);
            }
        }
    }
}

fn plot8(canvas: &mut Canvas<'_>, cx: i32, cy: i32, x: i32, y: i32, color: Color) {
    canvas.put_pixel(cx + x, cy + y, color);
    canvas.put_pixel(cx + y, cy + x, color);
    canvas.put_pixel(cx - y, cy + x, color);
    canvas.put_pixel(cx - x, cy + y, color);
    canvas.put_pixel(cx - x, cy - y, color);
    canvas.put_pixel(cx - y, cy - x, color);
    canvas.put_pixel(cx + y, cy - x, color);
    canvas.put_pixel(cx + x, cy - y, color);
}

/// Draw title text with optional ambient shadow for low-contrast glass only.
pub fn draw_title_text(
    canvas: &mut Canvas<'_>,
    text: &str,
    rect: Rect,
    color: Color,
    ambient_shadow: bool,
) {
    if text.is_empty() {
        return;
    }
    // Use bitmap path via canvas; vertical center inside titlebar strip.
    let ty = rect.y + (rect.h as i32 - 10) / 2;
    let tx = rect.x + 4;
    if ambient_shadow {
        // Single-pixel ambient shadow, bounded to text region.
        canvas.draw_text(tx + 1, ty + 1, text, Color::rgba(0, 0, 0, 140));
    }
    canvas.draw_text(tx, ty, text, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_order_stable_under_rtl() {
        assert_eq!(HORIZON_ORDER_LTR, HORIZON_ORDER_RTL);
        let ltr = layout_controls(0, 0, 400, 32, HorizonMetrics::default(), false, false);
        let rtl = layout_controls(0, 0, 400, 32, HorizonMetrics::default(), false, true);
        assert_eq!(ltr.pin.x, rtl.pin.x);
        assert_eq!(ltr.minimize.x, rtl.minimize.x);
        assert_eq!(ltr.maximize.x, rtl.maximize.x);
        assert_eq!(ltr.close.x, rtl.close.x);
        // Pin left of min left of max left of close
        assert!(ltr.pin.x < ltr.minimize.x);
        assert!(ltr.minimize.x < ltr.maximize.x);
        assert!(ltr.maximize.x < ltr.close.x);
        // Pin separation larger than standard spacing
        let pin_to_min = ltr.minimize.x - (ltr.pin.x + ltr.pin.w as i32);
        let min_to_max = ltr.maximize.x - (ltr.minimize.x + ltr.minimize.w as i32);
        assert!(pin_to_min > min_to_max);
    }

    #[test]
    fn maximize_vs_restore_glyph_selection() {
        let normal = layout_controls(10, 20, 300, 32, HorizonMetrics::default(), false, false);
        let maxed = layout_controls(10, 20, 300, 32, HorizonMetrics::default(), true, false);
        assert_eq!(normal.maximize_kind, HorizonControl::Maximize);
        assert_eq!(maxed.maximize_kind, HorizonControl::Restore);
        // Same hit rect either way
        assert_eq!(normal.maximize, maxed.maximize);
    }

    #[test]
    fn hit_rects_cover_full_buttons_not_glyph_pixels() {
        let layout = layout_controls(0, 0, 400, 32, HorizonMetrics::default(), false, false);
        // Corner of pin button still hits
        assert_eq!(
            layout.hit_test(layout.pin.x, layout.pin.y),
            Some(HorizonControl::Pin)
        );
        assert_eq!(
            layout.hit_test(layout.close.right() - 1, layout.close.bottom() - 1),
            Some(HorizonControl::Close)
        );
        // Gap between pin and minimize is not a control (divider region)
        let gap_x = layout.pin.right() + 1;
        if gap_x < layout.minimize.x {
            assert_eq!(layout.hit_test(gap_x, layout.pin.y + 5), None);
        }
        // Outside strip
        assert_eq!(layout.hit_test(0, 0), None);
    }

    #[test]
    fn control_ordering_matches_horizon_spec() {
        assert_eq!(
            HORIZON_ORDER_LTR,
            [
                HorizonControl::Pin,
                HorizonControl::Minimize,
                HorizonControl::Maximize,
                HorizonControl::Close,
            ]
        );
    }

    #[test]
    fn glyph_stays_inside_button_bounds() {
        // Smoke: drawing into a tiny canvas must not panic; pixels outside
        // the button may not be written by put_pixel bounds checks.
        let mut pixels = [0u32; 40 * 40];
        let mut canvas = Canvas::new(&mut pixels, 40, 40, 40);
        let rect = Rect::new(5, 5, 20, 20);
        let theme = Theme::sunlight_dark();
        let palette = HorizonPalette::from_theme(&theme);
        for control in [
            HorizonControl::Pin,
            HorizonControl::Minimize,
            HorizonControl::Maximize,
            HorizonControl::Restore,
            HorizonControl::Close,
        ] {
            draw_control(
                &mut canvas,
                rect,
                control,
                HorizonControlState::Hover,
                true,
                control == HorizonControl::Pin,
                &palette,
                5,
            );
        }
    }
}
