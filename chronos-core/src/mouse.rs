//! Per-runtime polling DOS mouse and overflow-safe Mode 13h viewport mapping.
//!
//! The reset range deliberately follows the conventional Mode 13h mouse
//! convention of 0..639 horizontally and 0..199 vertically. Guests wanting
//! one logical coordinate per framebuffer pixel should select 0..319 x
//! 0..199 with INT 33h functions 0007h and 0008h.

use crate::{CpuState, Runtime, Trap, VGA_HEIGHT, VGA_WIDTH};

pub const DOS_MOUSE_DEFAULT_MAX_X: u16 = 639;
pub const DOS_MOUSE_DEFAULT_MAX_Y: u16 = 199;
pub const DOS_MOUSE_BUTTON_COUNT: u16 = 3;
pub const DOS_MOUSE_ERROR_INVALID_ARGUMENT: u16 = 0x0001;
pub const DOS_MOUSE_ERROR_UNSUPPORTED: u16 = 0xffff;
pub const INT33_TRACKED_FUNCTION_COUNT: usize = 9;

/// Initial polling-only INT 33h dispatcher. Successful supported functions
/// clear CF. Reversed ranges return CF=1/AX=0001h, and deliberately unsupported
/// functions return CF=1/AX=FFFFh; Chronos therefore never claims broader
/// Microsoft Mouse Driver compatibility than it implements.
pub(crate) fn dispatch(runtime: &mut Runtime) -> Result<(), Trap> {
    let function = runtime.cpu.ax;
    runtime.mouse.record_int33_function(function);
    match function {
        0x0000 => {
            runtime.mouse.reset();
            if runtime.mouse.installed() {
                runtime.cpu.ax = 0xffff;
                runtime.cpu.bx = runtime.mouse.button_count();
            } else {
                runtime.cpu.ax = 0;
                runtime.cpu.bx = 0;
            }
            runtime.cpu.flags &= !CpuState::FLAG_CF;
        }
        0x0001 => {
            runtime.mouse.show();
            runtime.cpu.flags &= !CpuState::FLAG_CF;
        }
        0x0002 => {
            runtime.mouse.hide();
            runtime.cpu.flags &= !CpuState::FLAG_CF;
        }
        0x0003 => {
            let (x, y) = runtime.mouse.position();
            let buttons = runtime.mouse.buttons().bits();
            runtime.cpu.bx = buttons;
            runtime.cpu.cx = x;
            runtime.cpu.dx = y;
            runtime.mouse.record_state_query(buttons, x, y);
            runtime.cpu.flags &= !CpuState::FLAG_CF;
        }
        0x0004 => {
            runtime.mouse.set_position(runtime.cpu.cx, runtime.cpu.dx);
            runtime.cpu.flags &= !CpuState::FLAG_CF;
        }
        0x0007 => {
            let result = runtime
                .mouse
                .set_horizontal_range(runtime.cpu.cx, runtime.cpu.dx);
            finish_range(runtime, result);
        }
        0x0008 => {
            let result = runtime
                .mouse
                .set_vertical_range(runtime.cpu.cx, runtime.cpu.dx);
            finish_range(runtime, result);
        }
        _ => {
            runtime.cpu.ax = DOS_MOUSE_ERROR_UNSUPPORTED;
            runtime.cpu.flags |= CpuState::FLAG_CF;
        }
    }
    Ok(())
}

fn finish_range(runtime: &mut Runtime, result: Result<(), MouseRangeError>) {
    if result.is_ok() {
        runtime.cpu.flags &= !CpuState::FLAG_CF;
    } else {
        runtime.cpu.ax = DOS_MOUSE_ERROR_INVALID_ARGUMENT;
        runtime.cpu.flags |= CpuState::FLAG_CF;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseButtons(u16);

impl MouseButtons {
    pub const LEFT: u16 = 1 << 0;
    pub const RIGHT: u16 = 1 << 1;
    pub const MIDDLE: u16 = 1 << 2;
    pub const SUPPORTED_MASK: u16 = Self::LEFT | Self::RIGHT | Self::MIDDLE;

    pub const fn bits(self) -> u16 {
        self.0
    }

    fn set(&mut self, button: u8, pressed: bool) -> bool {
        if button >= DOS_MOUSE_BUTTON_COUNT as u8 {
            return false;
        }
        let mask = 1u16 << button;
        let old = self.0;
        if pressed {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
        old != self.0
    }

    fn clear(&mut self) -> bool {
        let changed = self.0 != 0;
        self.0 = 0;
        changed
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseViewport {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl MouseViewport {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(self, x: i32, y: i32) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        let Some(right) = i64::from(self.x).checked_add(i64::from(self.width)) else {
            return false;
        };
        let Some(bottom) = i64::from(self.y).checked_add(i64::from(self.height)) else {
            return false;
        };
        let x = i64::from(x);
        let y = i64::from(y);
        x >= i64::from(self.x) && x < right && y >= i64::from(self.y) && y < bottom
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseRangeError {
    Reversed,
}

#[derive(Clone, Debug)]
pub struct DosMouse {
    installed: bool,
    button_count: u16,
    x: u16,
    y: u16,
    min_x: u16,
    max_x: u16,
    min_y: u16,
    max_y: u16,
    buttons: MouseButtons,
    /// Counter convention: reset=-1 (hidden), first Show=0 (visible), nested
    /// Shows make it positive, and Hides balance back below zero.
    visibility_counter: i16,
    cursor_visible: bool,
    generation: u64,
    overlay_generation: u64,
    focused: bool,
    pointer_inside: bool,
    captured: bool,
    int33_state_query_count: u64,
    int33_function_counts: [u64; INT33_TRACKED_FUNCTION_COUNT],
    last_state_query: (u16, u16, u16),
}

impl DosMouse {
    pub fn new(installed: bool) -> Self {
        let mut mouse = Self {
            installed,
            button_count: if installed { DOS_MOUSE_BUTTON_COUNT } else { 0 },
            x: 0,
            y: 0,
            min_x: 0,
            max_x: DOS_MOUSE_DEFAULT_MAX_X,
            min_y: 0,
            max_y: DOS_MOUSE_DEFAULT_MAX_Y,
            buttons: MouseButtons::default(),
            visibility_counter: -1,
            cursor_visible: false,
            generation: 0,
            overlay_generation: 0,
            focused: true,
            pointer_inside: false,
            captured: false,
            int33_state_query_count: 0,
            int33_function_counts: [0; INT33_TRACKED_FUNCTION_COUNT],
            last_state_query: (0, 0, 0),
        };
        mouse.reset_state(false);
        mouse
    }

    pub fn reset(&mut self) {
        self.reset_state(true);
    }

    fn reset_state(&mut self, advance_generation: bool) {
        self.button_count = if self.installed {
            DOS_MOUSE_BUTTON_COUNT
        } else {
            0
        };
        self.min_x = 0;
        self.max_x = DOS_MOUSE_DEFAULT_MAX_X;
        self.min_y = 0;
        self.max_y = DOS_MOUSE_DEFAULT_MAX_Y;
        self.x = DOS_MOUSE_DEFAULT_MAX_X / 2;
        self.y = DOS_MOUSE_DEFAULT_MAX_Y / 2;
        self.buttons = MouseButtons::default();
        self.visibility_counter = -1;
        self.cursor_visible = false;
        self.pointer_inside = false;
        self.captured = false;
        if advance_generation {
            self.generation = self.generation.wrapping_add(1);
            self.overlay_generation = self.overlay_generation.wrapping_add(1);
        }
    }

    pub const fn installed(&self) -> bool {
        self.installed
    }

    pub const fn button_count(&self) -> u16 {
        self.button_count
    }

    pub const fn position(&self) -> (u16, u16) {
        (self.x, self.y)
    }

    pub const fn ranges(&self) -> (u16, u16, u16, u16) {
        (self.min_x, self.max_x, self.min_y, self.max_y)
    }

    pub const fn buttons(&self) -> MouseButtons {
        self.buttons
    }

    pub const fn visibility_counter(&self) -> i16 {
        self.visibility_counter
    }

    pub const fn cursor_visible(&self) -> bool {
        self.installed && self.cursor_visible
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn overlay_generation(&self) -> u64 {
        self.overlay_generation
    }

    pub const fn focused(&self) -> bool {
        self.focused
    }

    pub const fn pointer_inside(&self) -> bool {
        self.pointer_inside
    }

    pub const fn captured(&self) -> bool {
        self.captured
    }

    pub const fn int33_state_query_count(&self) -> u64 {
        self.int33_state_query_count
    }

    pub fn int33_function_count(&self, function: u16) -> u64 {
        self.int33_function_counts
            .get(function as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Last BX/CX/DX returned by INT 33h AX=0003.
    pub const fn last_state_query(&self) -> (u16, u16, u16) {
        self.last_state_query
    }

    pub(crate) fn record_state_query(&mut self, buttons: u16, x: u16, y: u16) {
        self.int33_state_query_count = self.int33_state_query_count.wrapping_add(1);
        self.last_state_query = (buttons, x, y);
    }

    fn record_int33_function(&mut self, function: u16) {
        if let Some(count) = self.int33_function_counts.get_mut(function as usize) {
            *count = count.wrapping_add(1);
        }
    }

    pub fn show(&mut self) {
        let was_visible = self.cursor_visible;
        self.visibility_counter = self.visibility_counter.saturating_add(1);
        self.cursor_visible = self.visibility_counter >= 0;
        if self.cursor_visible != was_visible {
            self.overlay_generation = self.overlay_generation.wrapping_add(1);
        }
    }

    pub fn hide(&mut self) {
        let was_visible = self.cursor_visible;
        self.visibility_counter = self.visibility_counter.saturating_sub(1);
        self.cursor_visible = self.visibility_counter >= 0;
        if self.cursor_visible != was_visible {
            self.overlay_generation = self.overlay_generation.wrapping_add(1);
        }
    }

    pub fn set_position(&mut self, x: u16, y: u16) {
        let x = x.clamp(self.min_x, self.max_x);
        let y = y.clamp(self.min_y, self.max_y);
        self.update_position(x, y);
    }

    pub fn set_horizontal_range(&mut self, min: u16, max: u16) -> Result<(), MouseRangeError> {
        if min > max {
            return Err(MouseRangeError::Reversed);
        }
        let before = (self.min_x, self.max_x, self.x);
        self.min_x = min;
        self.max_x = max;
        self.x = self.x.clamp(min, max);
        if before != (self.min_x, self.max_x, self.x) {
            self.generation = self.generation.wrapping_add(1);
            self.overlay_generation = self.overlay_generation.wrapping_add(1);
        }
        Ok(())
    }

    pub fn set_vertical_range(&mut self, min: u16, max: u16) -> Result<(), MouseRangeError> {
        if min > max {
            return Err(MouseRangeError::Reversed);
        }
        let before = (self.min_y, self.max_y, self.y);
        self.min_y = min;
        self.max_y = max;
        self.y = self.y.clamp(min, max);
        if before != (self.min_y, self.max_y, self.y) {
            self.generation = self.generation.wrapping_add(1);
            self.overlay_generation = self.overlay_generation.wrapping_add(1);
        }
        Ok(())
    }

    pub fn focus_changed(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if !focused {
            self.pointer_inside = false;
            self.release_capture_and_buttons();
        }
    }

    pub fn pointer_left(&mut self) {
        if !self.captured {
            self.pointer_inside = false;
        }
    }

    pub fn pointer_delivery_lost(&mut self) {
        self.pointer_inside = false;
        self.release_capture_and_buttons();
    }

    pub fn native_motion(&mut self, viewport: MouseViewport, x: i32, y: i32) -> bool {
        if !self.installed || !self.focused {
            return false;
        }
        let inside = viewport.contains(x, y);
        self.pointer_inside = inside;
        if !inside && !self.captured {
            return false;
        }
        let Some((logical_x, logical_y)) = self.map_native(viewport, x, y, self.captured) else {
            return false;
        };
        self.update_position(logical_x, logical_y)
    }

    pub fn native_button(
        &mut self,
        viewport: MouseViewport,
        x: i32,
        y: i32,
        button: u8,
        pressed: bool,
    ) -> bool {
        if !self.installed || !self.focused || button >= DOS_MOUSE_BUTTON_COUNT as u8 {
            return false;
        }
        let inside = viewport.contains(x, y);
        self.pointer_inside = inside;
        if pressed && !inside && !self.captured {
            return false;
        }
        if !pressed && !inside && !self.captured {
            return false;
        }
        let mut changed = self.native_motion(viewport, x, y);
        if self.buttons.set(button, pressed) {
            self.generation = self.generation.wrapping_add(1);
            changed = true;
        }
        if pressed && inside {
            self.captured = true;
        } else if !pressed && self.buttons.bits() == 0 {
            self.captured = false;
        }
        changed
    }

    pub fn leave_graphics_mode(&mut self) {
        self.pointer_inside = false;
        self.release_capture_and_buttons();
        let was_visible = self.cursor_visible;
        self.visibility_counter = -1;
        self.cursor_visible = false;
        if was_visible {
            self.overlay_generation = self.overlay_generation.wrapping_add(1);
        }
    }

    pub fn framebuffer_position(&self) -> (u16, u16) {
        (
            logical_to_pixel(self.x, self.min_x, self.max_x, VGA_WIDTH as u16 - 1),
            logical_to_pixel(self.y, self.min_y, self.max_y, VGA_HEIGHT as u16 - 1),
        )
    }

    fn release_capture_and_buttons(&mut self) {
        self.captured = false;
        if self.buttons.clear() {
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn update_position(&mut self, x: u16, y: u16) -> bool {
        if (self.x, self.y) == (x, y) {
            return false;
        }
        self.x = x;
        self.y = y;
        self.generation = self.generation.wrapping_add(1);
        self.overlay_generation = self.overlay_generation.wrapping_add(1);
        true
    }

    fn map_native(
        &self,
        viewport: MouseViewport,
        x: i32,
        y: i32,
        clamp: bool,
    ) -> Option<(u16, u16)> {
        let (viewport_x, viewport_y) = viewport_offset(viewport, x, y, clamp)?;
        Some((
            viewport_to_logical(viewport_x, viewport.width, self.min_x, self.max_x),
            viewport_to_logical(viewport_y, viewport.height, self.min_y, self.max_y),
        ))
    }
}

impl Default for DosMouse {
    fn default() -> Self {
        Self::new(true)
    }
}

pub fn viewport_pixel(viewport: MouseViewport, x: i32, y: i32, clamp: bool) -> Option<(u32, u32)> {
    let (local_x, local_y) = viewport_offset(viewport, x, y, clamp)?;
    let pixel_x = local_x
        .checked_mul(VGA_WIDTH as u64)?
        .checked_div(u64::from(viewport.width))?
        .min(VGA_WIDTH as u64 - 1) as u32;
    let pixel_y = local_y
        .checked_mul(VGA_HEIGHT as u64)?
        .checked_div(u64::from(viewport.height))?
        .min(VGA_HEIGHT as u64 - 1) as u32;
    Some((pixel_x, pixel_y))
}

fn viewport_offset(viewport: MouseViewport, x: i32, y: i32, clamp: bool) -> Option<(u64, u64)> {
    if viewport.width == 0 || viewport.height == 0 {
        return None;
    }
    if !clamp && !viewport.contains(x, y) {
        return None;
    }
    let origin_x = i64::from(viewport.x);
    let origin_y = i64::from(viewport.y);
    let max_native_x = origin_x
        .checked_add(i64::from(viewport.width))?
        .checked_sub(1)?;
    let max_native_y = origin_y
        .checked_add(i64::from(viewport.height))?
        .checked_sub(1)?;
    let local_x = i64::from(x)
        .clamp(origin_x, max_native_x)
        .checked_sub(origin_x)? as u64;
    let local_y = i64::from(y)
        .clamp(origin_y, max_native_y)
        .checked_sub(origin_y)? as u64;
    Some((local_x, local_y))
}

fn viewport_to_logical(offset: u64, extent: u32, min: u16, max: u16) -> u16 {
    if min == max || extent <= 1 {
        return min;
    }
    if offset >= u64::from(extent - 1) {
        return max;
    }
    let values = u64::from(max - min) + 1;
    (u64::from(min) + offset * values / u64::from(extent)).min(u64::from(max)) as u16
}

fn logical_to_pixel(value: u16, min: u16, max: u16, pixel_max: u16) -> u16 {
    if min == max {
        return 0;
    }
    let value = value.clamp(min, max) - min;
    ((u32::from(value) * u32::from(pixel_max)) / u32::from(max - min)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_and_visibility_follow_documented_convention() {
        let mut mouse = DosMouse::default();
        mouse.set_horizontal_range(4, 9).unwrap();
        mouse.set_position(9, 150);
        mouse.show();
        mouse.native_button(MouseViewport::new(0, 0, 320, 200), 9, 9, 0, true);
        mouse.reset();
        assert!(mouse.installed());
        assert_eq!(mouse.button_count(), 3);
        assert_eq!(mouse.ranges(), (0, 639, 0, 199));
        assert_eq!(mouse.position(), (319, 99));
        assert_eq!(mouse.buttons().bits(), 0);
        assert_eq!(mouse.visibility_counter(), -1);
        assert!(!mouse.cursor_visible());

        mouse.show();
        assert_eq!(mouse.visibility_counter(), 0);
        assert!(mouse.cursor_visible());
        mouse.show();
        mouse.hide();
        assert!(mouse.cursor_visible());
        mouse.hide();
        assert!(!mouse.cursor_visible());
        for _ in 0..40_000 {
            mouse.hide();
        }
        assert_eq!(mouse.visibility_counter(), i16::MIN);
    }

    #[test]
    fn ranges_validate_clamp_and_allow_zero_width() {
        let mut mouse = DosMouse::default();
        mouse.set_position(600, 190);
        assert_eq!(
            mouse.set_horizontal_range(319, 0),
            Err(MouseRangeError::Reversed)
        );
        assert_eq!(mouse.ranges(), (0, 639, 0, 199));
        mouse.set_horizontal_range(0, 319).unwrap();
        mouse.set_vertical_range(12, 12).unwrap();
        assert_eq!(mouse.position(), (319, 12));
        assert_eq!(mouse.framebuffer_position(), (319, 0));
    }

    #[test]
    fn viewport_edges_centers_scales_and_letterboxes_map_exactly() {
        for scale in 1..=3 {
            let viewport = MouseViewport::new(17, 29, 320 * scale, 200 * scale);
            let mut mouse = DosMouse::default();
            mouse.set_horizontal_range(0, 319).unwrap();
            mouse.set_vertical_range(0, 199).unwrap();
            assert!(mouse.native_motion(viewport, 17, 29));
            assert_eq!(mouse.position(), (0, 0));
            mouse.native_motion(viewport, viewport.x + viewport.width as i32 - 1, 29);
            assert_eq!(mouse.position(), (319, 0));
            mouse.native_motion(viewport, 17, viewport.y + viewport.height as i32 - 1);
            assert_eq!(mouse.position(), (0, 199));
            mouse.native_motion(
                viewport,
                viewport.x + (160 * scale) as i32,
                viewport.y + (100 * scale) as i32,
            );
            assert_eq!(mouse.position(), (160, 100));
        }
    }

    #[test]
    fn text_cell_centers_map_to_the_same_guest_cells() {
        let viewport = MouseViewport::new(17, 29, 80 * 9, 25 * 16);
        let mut mouse = DosMouse::default();
        mouse.set_horizontal_range(0, 79).unwrap();
        mouse.set_vertical_range(0, 24).unwrap();

        for (column, row) in [(0, 0), (31, 3), (39, 7), (79, 24)] {
            mouse.native_motion(
                viewport,
                viewport.x + column * 9 + 4,
                viewport.y + row * 16 + 8,
            );
            assert_eq!(mouse.position(), (column as u16, row as u16));
        }
    }

    #[test]
    fn outside_motion_is_ignored_until_capture_then_clamped_on_every_edge() {
        let viewport = MouseViewport::new(100, 70, 640, 400);
        let mut mouse = DosMouse::default();
        mouse.set_horizontal_range(0, 319).unwrap();
        mouse.set_vertical_range(0, 199).unwrap();
        assert!(!mouse.native_motion(viewport, 99, 70));
        assert_eq!(mouse.position(), (319, 99));
        mouse.native_button(viewport, 100, 70, 0, true);
        assert!(mouse.captured());
        mouse.native_motion(viewport, i32::MIN, i32::MIN);
        assert_eq!(mouse.position(), (0, 0));
        mouse.native_motion(viewport, i32::MAX, i32::MAX);
        assert_eq!(mouse.position(), (319, 199));
        mouse.native_button(viewport, i32::MAX, i32::MAX, 0, false);
        assert!(!mouse.captured());
        assert_eq!(mouse.buttons().bits(), 0);
    }

    #[test]
    fn focus_loss_releases_buttons_capture_without_moving() {
        let viewport = MouseViewport::new(0, 0, 320, 200);
        let mut mouse = DosMouse::default();
        mouse.native_button(viewport, 20, 30, 0, true);
        let position = mouse.position();
        mouse.focus_changed(false);
        assert_eq!(mouse.position(), position);
        assert_eq!(mouse.buttons().bits(), 0);
        assert!(!mouse.captured());
        assert!(!mouse.native_motion(viewport, 100, 100));
    }

    #[test]
    fn separate_devices_are_isolated() {
        let mut first = DosMouse::default();
        let mut second = DosMouse::default();
        first.set_horizontal_range(0, 319).unwrap();
        first.set_position(12, 34);
        second.reset();
        assert_eq!(first.ranges(), (0, 319, 0, 199));
        assert_eq!(first.position(), (12, 34));
        assert_eq!(second.ranges(), (0, 639, 0, 199));
        assert_eq!(second.position(), (319, 99));
    }

    #[test]
    fn malformed_or_odd_viewports_cannot_divide_by_zero_or_overflow() {
        assert_eq!(
            viewport_pixel(MouseViewport::new(0, 0, 0, 200), 0, 0, true),
            None
        );
        assert_eq!(
            viewport_pixel(MouseViewport::new(0, 0, 320, 0), 0, 0, true),
            None
        );
        let viewport = MouseViewport::new(i32::MAX - 10, i32::MAX - 10, 999, 777);
        let point = viewport_pixel(viewport, i32::MAX, i32::MAX, true).unwrap();
        assert!(point.0 < VGA_WIDTH as u32 && point.1 < VGA_HEIGHT as u32);
    }

    #[test]
    fn buttons_focus_and_resize_preserve_coherent_latest_state() {
        let first = MouseViewport::new(21, 33, 960, 600);
        let resized = MouseViewport::new(7, 9, 1280, 800);
        let mut mouse = DosMouse::default();
        mouse.set_horizontal_range(0, 319).unwrap();
        mouse.set_vertical_range(0, 199).unwrap();
        mouse.native_motion(first, first.x + 480, first.y + 300);
        assert_eq!(mouse.position(), (160, 100));
        assert_eq!(
            mouse.position(),
            (160, 100),
            "resize alone cannot mutate guest state"
        );
        mouse.native_motion(resized, resized.x + 640, resized.y + 400);
        assert_eq!(mouse.position(), (160, 100));

        mouse.native_button(resized, resized.x + 10, resized.y + 10, 0, true);
        mouse.native_button(resized, resized.x + 10, resized.y + 10, 1, true);
        mouse.native_button(resized, resized.x + 10, resized.y + 10, 2, true);
        assert_eq!(mouse.buttons().bits(), MouseButtons::SUPPORTED_MASK);
        mouse.native_button(resized, resized.x + 10, resized.y + 10, 0, false);
        assert_eq!(
            mouse.buttons().bits(),
            MouseButtons::RIGHT | MouseButtons::MIDDLE
        );
        mouse.focus_changed(false);
        assert_eq!(mouse.buttons().bits(), 0);
        assert!(!mouse.captured());
    }
}
