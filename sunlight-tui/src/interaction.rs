//! Allocation-free pointer interaction shared by TTY/TUI applications.
//!
//! Coordinates are framebuffer pixels. Applications describe the same widget
//! rectangles they render, then feed normalized relative pointer reports into
//! [`PointerSurface`]. The tracker owns hover and press capture; keyboard focus
//! remains application state and changes only when the application accepts a
//! completed click.

const FP_SHIFT: u32 = 16;
const CURSOR_W: usize = 10;
const CURSOR_H: usize = 15;
const CURSOR_PIXELS: usize = CURSOR_W * CURSOR_H;
const CURSOR_WHITE: u32 = 0x00ff_ffff;
const CURSOR_BLACK: u32 = 0x0000_0000;
const CURSOR_TRANSPARENT: u8 = 0;
const CURSOR_OUTLINE: u8 = 1;
pub const PRIMARY_BUTTON: u8 = 0x01;

const ARROW_CURSOR: [u8; CURSOR_PIXELS] = [
    1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 0, 0, 0, 0, 0, 0, 0, 1, 2,
    2, 2, 0, 0, 0, 0, 0, 0, 1, 2, 2, 2, 2, 0, 0, 0, 0, 0, 1, 2, 2, 2, 2, 2, 0, 0, 0, 0, 1, 2, 2, 2,
    2, 2, 2, 0, 0, 0, 1, 2, 2, 2, 2, 2, 2, 2, 0, 0, 1, 2, 2, 2, 2, 1, 1, 1, 1, 0, 1, 2, 2, 1, 2, 2,
    0, 0, 0, 0, 1, 2, 1, 0, 1, 2, 2, 0, 0, 0, 1, 1, 0, 0, 1, 2, 2, 0, 0, 0, 1, 0, 0, 0, 0, 1, 2, 2,
    0, 0, 0, 0, 0, 0, 0, 1, 2, 2, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 0,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x
            && point.y >= self.y
            && point.x < self.x.saturating_add(self.w)
            && point.y < self.y.saturating_add(self.h)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidgetId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetKind {
    Button,
    TextInput,
    Selectable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Widget {
    pub id: WidgetId,
    pub bounds: Rect,
    pub kind: WidgetKind,
    pub visible: bool,
    pub enabled: bool,
}

impl Widget {
    pub const fn new(id: WidgetId, bounds: Rect, kind: WidgetKind) -> Self {
        Self {
            id,
            bounds,
            kind,
            visible: true,
            enabled: true,
        }
    }

    pub const fn unavailable(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub const fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }
}

/// Hit test in visual order: the last widget is the topmost widget.
pub fn hit_test(widgets: &[Widget], point: Point) -> Option<WidgetId> {
    widgets
        .iter()
        .rev()
        .find(|widget| widget.visible && widget.enabled && widget.bounds.contains(point))
        .map(|widget| widget.id)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PointerOutcome {
    pub moved: bool,
    pub left: Option<WidgetId>,
    pub entered: Option<WidgetId>,
    pub down: Option<WidgetId>,
    pub up: Option<WidgetId>,
    pub clicked: Option<WidgetId>,
    pub ignored_duplicate: bool,
}

impl PointerOutcome {
    pub fn interaction_changed(self) -> bool {
        self.left.is_some()
            || self.entered.is_some()
            || self.down.is_some()
            || self.up.is_some()
            || self.clicked.is_some()
    }
}

/// Pointer position, button capture, and saved-under cursor pixels for one TUI
/// surface. Movement is retained in Q16.16 form so a future global fractional
/// scale can be applied without losing repeated sub-pixel deltas.
pub struct PointerSurface {
    x_fp: i64,
    y_fp: i64,
    width: u32,
    height: u32,
    initialized: bool,
    active: bool,
    buttons: u8,
    suppress_buttons_until_release: bool,
    hovered: Option<WidgetId>,
    pressed: Option<WidgetId>,
    last_generation: u32,
    saved_x: u32,
    saved_y: u32,
    saved_bg: [u32; CURSOR_PIXELS],
    saved_mask: [bool; CURSOR_PIXELS],
    has_saved: bool,
}

impl PointerSurface {
    pub const fn new() -> Self {
        Self {
            x_fp: 0,
            y_fp: 0,
            width: 0,
            height: 0,
            initialized: false,
            active: true,
            buttons: 0,
            suppress_buttons_until_release: false,
            hovered: None,
            pressed: None,
            last_generation: 0,
            saved_x: 0,
            saved_y: 0,
            saved_bg: [0; CURSOR_PIXELS],
            saved_mask: [false; CURSOR_PIXELS],
            has_saved: false,
        }
    }

    pub fn position(&self) -> Point {
        Point {
            x: self.pixel_x(),
            y: self.pixel_y(),
        }
    }

    pub const fn hovered(&self) -> Option<WidgetId> {
        self.hovered
    }

    pub const fn pressed(&self) -> Option<WidgetId> {
        self.pressed
    }

    pub const fn is_active(&self) -> bool {
        self.active
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        if !self.initialized && width > 0 && height > 0 {
            self.x_fp = i64::from(width / 2) << FP_SHIFT;
            self.y_fp = i64::from(height / 2) << FP_SHIFT;
            self.initialized = true;
        }
        self.clamp_position();
    }

    /// Restore a TUI surface after another session owned pointer input. A held
    /// physical button is ignored until a release report prevents a synthetic
    /// click on the newly active surface.
    pub fn activate(&mut self, width: u32, height: u32) {
        self.resize(width, height);
        self.active = true;
        self.clear_interaction();
        self.suppress_buttons_until_release = true;
    }

    pub fn deactivate(&mut self) {
        self.active = false;
        self.clear_interaction();
    }

    pub fn clear_interaction(&mut self) {
        self.buttons = 0;
        self.hovered = None;
        self.pressed = None;
        self.suppress_buttons_until_release = false;
    }

    pub fn handle_report(
        &mut self,
        dx: i16,
        dy: i16,
        buttons: u8,
        generation: u32,
        width: u32,
        height: u32,
        widgets: &[Widget],
    ) -> PointerOutcome {
        self.resize(width, height);
        if !self.active {
            return PointerOutcome::default();
        }
        if generation != 0 && generation == self.last_generation {
            return PointerOutcome {
                ignored_duplicate: true,
                ..PointerOutcome::default()
            };
        }
        if generation != 0 {
            self.last_generation = generation;
        }

        let before = self.position();
        self.x_fp = self.x_fp.saturating_add(i64::from(dx) << FP_SHIFT);
        self.y_fp = self.y_fp.saturating_add(i64::from(dy) << FP_SHIFT);
        self.clamp_position();
        let point = self.position();
        let mut outcome = PointerOutcome {
            moved: point != before,
            ..PointerOutcome::default()
        };

        let next_hover = hit_test(widgets, point);
        if next_hover != self.hovered {
            outcome.left = self.hovered;
            outcome.entered = next_hover;
            self.hovered = next_hover;
        }

        let supported_buttons = buttons & PRIMARY_BUTTON;
        if self.suppress_buttons_until_release {
            self.buttons = 0;
            self.pressed = None;
            if supported_buttons == 0 {
                self.suppress_buttons_until_release = false;
            }
            return outcome;
        }

        let was_primary_down = self.buttons & PRIMARY_BUTTON != 0;
        let primary_down = supported_buttons != 0;
        if !was_primary_down && primary_down {
            self.pressed = next_hover;
            outcome.down = next_hover;
        } else if was_primary_down && !primary_down {
            outcome.up = self.pressed;
            if self.pressed.is_some() && self.pressed == next_hover {
                outcome.clicked = self.pressed;
            }
            self.pressed = None;
        }
        self.buttons = supported_buttons;
        outcome
    }

    fn pixel_x(&self) -> u32 {
        if self.width == 0 {
            0
        } else {
            (self.x_fp >> FP_SHIFT).clamp(0, i64::from(self.width - 1)) as u32
        }
    }

    fn pixel_y(&self) -> u32 {
        if self.height == 0 {
            0
        } else {
            (self.y_fp >> FP_SHIFT).clamp(0, i64::from(self.height - 1)) as u32
        }
    }

    fn clamp_position(&mut self) {
        let max_x = i64::from(self.width.saturating_sub(1)) << FP_SHIFT;
        let max_y = i64::from(self.height.saturating_sub(1)) << FP_SHIFT;
        self.x_fp = self.x_fp.clamp(0, max_x);
        self.y_fp = self.y_fp.clamp(0, max_y);
    }

    /// Restore only pixels covered by the previous pointer sprite.
    ///
    /// # Safety
    /// `fb_addr` and `pitch_bytes` must describe a writable XRGB framebuffer.
    pub unsafe fn erase_overlay(
        &mut self,
        fb_addr: *mut u32,
        width: u32,
        height: u32,
        pitch_bytes: u32,
    ) {
        if !self.has_saved || fb_addr.is_null() || width == 0 || height == 0 || pitch_bytes == 0 {
            return;
        }
        let stride = pitch_bytes as usize / core::mem::size_of::<u32>();
        for row in 0..CURSOR_H {
            let y = self.saved_y.saturating_add(row as u32);
            if y >= height {
                continue;
            }
            for col in 0..CURSOR_W {
                let index = row * CURSOR_W + col;
                if !self.saved_mask[index] {
                    continue;
                }
                let x = self.saved_x.saturating_add(col as u32);
                if x < width {
                    fb_addr
                        .add(y as usize * stride + x as usize)
                        .write_volatile(self.saved_bg[index]);
                }
            }
        }
        self.has_saved = false;
    }

    /// Composite the existing white/black pointer above TUI contents.
    ///
    /// # Safety
    /// `fb_addr` and `pitch_bytes` must describe a writable XRGB framebuffer.
    pub unsafe fn draw_overlay(
        &mut self,
        fb_addr: *mut u32,
        width: u32,
        height: u32,
        pitch_bytes: u32,
    ) {
        self.resize(width, height);
        if !self.active || fb_addr.is_null() || width == 0 || height == 0 || pitch_bytes == 0 {
            return;
        }
        let stride = pitch_bytes as usize / core::mem::size_of::<u32>();
        let point = self.position();
        self.saved_x = point.x;
        self.saved_y = point.y;
        self.saved_mask = [false; CURSOR_PIXELS];
        for row in 0..CURSOR_H {
            let y = self.saved_y.saturating_add(row as u32);
            if y >= height {
                continue;
            }
            for col in 0..CURSOR_W {
                let index = row * CURSOR_W + col;
                let cursor_pixel = ARROW_CURSOR[index];
                if cursor_pixel == CURSOR_TRANSPARENT {
                    continue;
                }
                let x = self.saved_x.saturating_add(col as u32);
                if x >= width {
                    continue;
                }
                let pixel = fb_addr.add(y as usize * stride + x as usize);
                self.saved_bg[index] = pixel.read_volatile();
                pixel.write_volatile(if cursor_pixel == CURSOR_OUTLINE {
                    CURSOR_BLACK
                } else {
                    CURSOR_WHITE
                });
                self.saved_mask[index] = true;
            }
        }
        self.has_saved = true;
    }
}

impl Default for PointerSurface {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUTTON: Widget = Widget::new(WidgetId(1), Rect::new(10, 10, 20, 20), WidgetKind::Button);

    fn pointer_at_origin() -> PointerSurface {
        let mut pointer = PointerSurface::new();
        pointer.resize(100, 80);
        pointer.x_fp = 0;
        pointer.y_fp = 0;
        pointer
    }

    #[test]
    fn relative_motion_has_screen_direction_and_clamps_without_wrapping() {
        let mut pointer = pointer_at_origin();
        pointer.handle_report(7, 9, 0, 1, 100, 80, &[]);
        assert_eq!(pointer.position(), Point { x: 7, y: 9 });
        pointer.handle_report(-20, -20, 0, 2, 100, 80, &[]);
        assert_eq!(pointer.position(), Point { x: 0, y: 0 });
        pointer.handle_report(i16::MAX, i16::MAX, 0, 3, 100, 80, &[]);
        assert_eq!(pointer.position(), Point { x: 99, y: 79 });
        pointer.handle_report(i16::MAX, i16::MAX, 0, 4, 100, 80, &[]);
        assert_eq!(pointer.position(), Point { x: 99, y: 79 });
    }

    #[test]
    fn repeated_small_deltas_are_retained() {
        let mut pointer = pointer_at_origin();
        for generation in 1..=5 {
            pointer.handle_report(1, 1, 0, generation, 100, 80, &[]);
        }
        assert_eq!(pointer.position(), Point { x: 5, y: 5 });
    }

    #[test]
    fn hit_testing_ignores_unavailable_widgets_and_prefers_topmost() {
        let bottom = Widget::new(WidgetId(1), Rect::new(0, 0, 20, 20), WidgetKind::Button);
        let top = Widget::new(WidgetId(2), Rect::new(5, 5, 20, 20), WidgetKind::Button);
        assert_eq!(
            hit_test(&[bottom, top], Point { x: 10, y: 10 }),
            Some(WidgetId(2))
        );
        assert_eq!(
            hit_test(&[bottom, top.hidden()], Point { x: 10, y: 10 }),
            Some(WidgetId(1))
        );
        assert_eq!(
            hit_test(&[bottom.unavailable()], Point { x: 10, y: 10 }),
            None
        );
        assert_eq!(hit_test(&[bottom], Point { x: 20, y: 20 }), None);
    }

    #[test]
    fn click_requires_press_and_release_on_same_widget_once() {
        let mut pointer = pointer_at_origin();
        pointer.handle_report(15, 15, 0, 1, 100, 80, &[BUTTON]);
        assert_eq!(
            pointer
                .handle_report(0, 0, PRIMARY_BUTTON, 2, 100, 80, &[BUTTON])
                .down,
            Some(WidgetId(1))
        );
        assert_eq!(
            pointer
                .handle_report(0, 0, 0, 3, 100, 80, &[BUTTON])
                .clicked,
            Some(WidgetId(1))
        );
        assert_eq!(
            pointer
                .handle_report(0, 0, 0, 4, 100, 80, &[BUTTON])
                .clicked,
            None
        );
    }

    #[test]
    fn drag_out_release_and_release_without_press_do_not_click() {
        let mut pointer = pointer_at_origin();
        pointer.handle_report(15, 15, PRIMARY_BUTTON, 1, 100, 80, &[BUTTON]);
        assert_eq!(
            pointer
                .handle_report(30, 0, 0, 2, 100, 80, &[BUTTON])
                .clicked,
            None
        );
        assert_eq!(
            pointer
                .handle_report(-30, 0, 0, 3, 100, 80, &[BUTTON])
                .clicked,
            None
        );
    }

    #[test]
    fn duplicate_generation_cannot_double_activate() {
        let mut pointer = pointer_at_origin();
        pointer.handle_report(15, 15, PRIMARY_BUTTON, 10, 100, 80, &[BUTTON]);
        let release = pointer.handle_report(0, 0, 0, 11, 100, 80, &[BUTTON]);
        assert_eq!(release.clicked, Some(WidgetId(1)));
        let duplicate = pointer.handle_report(0, 0, 0, 11, 100, 80, &[BUTTON]);
        assert!(duplicate.ignored_duplicate);
        assert_eq!(duplicate.clicked, None);
    }

    #[test]
    fn session_switch_clears_capture_and_suppresses_held_button() {
        let mut pointer = pointer_at_origin();
        pointer.handle_report(15, 15, PRIMARY_BUTTON, 1, 100, 80, &[BUTTON]);
        pointer.deactivate();
        pointer.activate(100, 80);
        assert_eq!(
            pointer
                .handle_report(0, 0, PRIMARY_BUTTON, 2, 100, 80, &[BUTTON])
                .down,
            None
        );
        pointer.handle_report(0, 0, 0, 3, 100, 80, &[BUTTON]);
        assert_eq!(
            pointer
                .handle_report(0, 0, PRIMARY_BUTTON, 4, 100, 80, &[BUTTON])
                .down,
            Some(WidgetId(1))
        );
    }
}
