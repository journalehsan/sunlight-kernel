//! Reusable scroll container and scrollbar abstraction.
//!
//! Provides [`ScrollState`] for persistent offset tracking and
//! [`draw_scrollbar`] for rendering.  Callers own the clip and
//! hit-testing; this module supplies pure geometry helpers.

use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

/// Width of the vertical scrollbar track and thumb (pixels).
pub const SCROLLBAR_WIDTH: u32 = 10;
/// Minimum thumb height to keep the scrollbar usable on very large content.
pub const SCROLLBAR_MIN_THUMB_H: u32 = 24;
/// Gap from top and bottom of the track to the first/last thumb position.
pub const SCROLLBAR_TRACK_PAD: u32 = 2;

/// Scrollbar visibility policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollPolicy {
    /// Show only when content overflows the viewport.
    Auto,
    /// Always show the scrollbar (thumb may fill the entire track).
    Always,
}

/// Scroll direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDirection {
    Vertical,
    #[allow(dead_code)]
    Horizontal,
    #[allow(dead_code)]
    Both,
}

/// Persistent scroll state for one scrollable region.
///
/// All offsets are clamped to `[0, max_offset]`.  When content is
/// smaller than the viewport the offset is forced to zero.
#[derive(Clone, Copy, Debug)]
pub struct ScrollState {
    /// Current vertical scroll offset in pixels.  Always >= 0.
    pub offset_y: i32,
    /// Current horizontal scroll offset in pixels.  Always >= 0.
    pub offset_x: i32,

    /// Viewport dimensions (the visible clip area).
    pub viewport_w: u32,
    pub viewport_h: u32,

    /// Total content dimensions.
    pub content_w: u32,
    pub content_h: u32,

    /// True while the scrollbar thumb is being actively dragged.
    pub dragging: bool,
    /// True while the pointer hovers the scrollbar track or thumb.
    pub hovered: bool,
    /// True while this scrollable region owns keyboard scrolling focus.
    pub focused: bool,

    /// Offset of the drag anchor relative to the thumb top, in track space.
    drag_anchor_y: i32,
    /// Saved scroll offset at drag start.
    drag_start_offset_y: i32,
}

impl ScrollState {
    pub const fn new() -> Self {
        Self {
            offset_y: 0,
            offset_x: 0,
            viewport_w: 0,
            viewport_h: 0,
            content_w: 0,
            content_h: 0,
            dragging: false,
            hovered: false,
            focused: false,
            drag_anchor_y: 0,
            drag_start_offset_y: 0,
        }
    }

    // ── Geometry queries ────────────────────────────────────────────────

    /// Maximum valid vertical offset.  Zero or positive.
    pub fn max_offset_y(&self) -> i32 {
        if self.viewport_h == 0 {
            return 0;
        }
        (self.content_h.saturating_sub(self.viewport_h) as i32).max(0)
    }

    /// Maximum valid horizontal offset.  Zero or positive.
    pub fn max_offset_x(&self) -> i32 {
        if self.viewport_w == 0 {
            return 0;
        }
        (self.content_w.saturating_sub(self.viewport_w) as i32).max(0)
    }

    /// True when content exceeds the viewport vertically.
    pub fn can_scroll_y(&self) -> bool {
        self.viewport_h > 0 && self.content_h > self.viewport_h
    }

    /// True when content exceeds the viewport horizontally.
    pub fn can_scroll_x(&self) -> bool {
        self.viewport_w > 0 && self.content_w > self.viewport_w
    }

    // ── Clamping ────────────────────────────────────────────────────────

    /// Clamp both offsets into `[0, max]`.  Call after resizing or
    /// changing content size.
    pub fn clamp(&mut self) {
        self.offset_y = self.offset_y.clamp(0, self.max_offset_y());
        self.offset_x = self.offset_x.clamp(0, self.max_offset_x());
    }

    /// Set viewport + content geometry together, then clamp.
    pub fn set_geometry(
        &mut self,
        viewport_w: u32,
        viewport_h: u32,
        content_w: u32,
        content_h: u32,
    ) {
        self.viewport_w = viewport_w;
        self.viewport_h = viewport_h;
        self.content_w = content_w;
        self.content_h = content_h;
        self.clamp();
    }

    /// Set viewport size and clamp, keeping existing content size.
    pub fn set_viewport(&mut self, w: u32, h: u32) {
        self.viewport_w = w;
        self.viewport_h = h;
        self.clamp();
    }

    // ── Offset mutation ─────────────────────────────────────────────────

    /// Add delta_y to the vertical offset.  Positive = scroll down.
    pub fn scroll_by(&mut self, dy: i32) -> bool {
        let previous = self.offset_y;
        self.offset_y = (self.offset_y + dy).clamp(0, self.max_offset_y());
        self.offset_y != previous
    }

    /// Set absolute vertical offset, clamped.
    pub fn scroll_to_y(&mut self, y: i32) -> bool {
        let previous = self.offset_y;
        self.offset_y = y.clamp(0, self.max_offset_y());
        self.offset_y != previous
    }

    /// Apply a wheel delta using `pixels_per_step` for a conventional detent.
    /// Small HID deltas are treated as one step; fixed-point deltas use
    /// 120 units per step.
    pub fn scroll_by_wheel(&mut self, delta: i16, pixels_per_step: i32) -> bool {
        if delta == 0 || pixels_per_step == 0 {
            return false;
        }
        let steps = if delta.unsigned_abs() >= 120 {
            delta as i32 / 120
        } else {
            delta.signum() as i32
        };
        self.scroll_by(steps.saturating_mul(pixels_per_step))
    }

    /// Page down by approximately one viewport.
    pub fn page_down(&mut self) -> bool {
        let step = (self.viewport_h as i32).max(1);
        self.scroll_by(step)
    }

    /// Page up by approximately one viewport.
    pub fn page_up(&mut self) -> bool {
        let step = (self.viewport_h as i32).max(1);
        self.scroll_by(-step)
    }

    /// Scroll to make `item_y` (top edge, content-space) visible.
    /// Moves as little as possible.  Returns true if offset changed.
    pub fn ensure_visible(&mut self, item_y: i32, item_h: u32) -> bool {
        let prev = self.offset_y;
        let item_bottom = item_y + item_h as i32;
        let view_bottom = self.offset_y + self.viewport_h as i32;

        if item_y < self.offset_y {
            self.offset_y = item_y.max(0);
        } else if item_bottom > view_bottom {
            self.offset_y = (item_bottom - self.viewport_h as i32).max(0);
        }
        self.clamp();
        prev != self.offset_y
    }

    // ── Scrollbar geometry ──────────────────────────────────────────────

    /// Track rect for a vertical scrollbar positioned along the right
    /// edge of `region`.
    pub fn track_rect(&self, region: Rect) -> Rect {
        Rect::new(
            region.right() - SCROLLBAR_WIDTH as i32,
            region.y,
            SCROLLBAR_WIDTH,
            region.h,
        )
    }

    /// Thumb rect within the given track, or None if content fits.
    pub fn thumb_rect(&self, track: Rect) -> Option<Rect> {
        if !self.can_scroll_y() {
            return None;
        }
        let track_h = track.h.saturating_sub(SCROLLBAR_TRACK_PAD * 2);
        if track_h == 0 {
            return None;
        }
        // thumb_h = track_h * viewport_h / content_h (integer + rounding)
        let thumb_h = ((track_h as u64 * self.viewport_h as u64 / self.content_h as u64) as u32)
            .max(SCROLLBAR_MIN_THUMB_H)
            .min(track_h);
        let max_travel = track_h - thumb_h;
        let max_offset = self.max_offset_y().max(1) as u32;
        // travel = offset * max_travel / max_offset (integer + rounding)
        let travel = if max_travel > 0 && max_offset > 0 {
            ((self.offset_y as u64 * max_travel as u64 + max_offset as u64 / 2) / max_offset as u64)
                as u32
        } else {
            0
        };
        Some(Rect::new(
            track.x,
            track.y + SCROLLBAR_TRACK_PAD as i32 + travel as i32,
            SCROLLBAR_WIDTH,
            thumb_h,
        ))
    }

    // ── Drag handling ───────────────────────────────────────────────────

    /// Begin dragging the scrollbar thumb.  `pointer_y` is the
    /// absolute y coordinate of the pointer at drag start.
    pub fn start_drag(&mut self, track: Rect, pointer_y: i32) -> bool {
        if let Some(thumb) = self.thumb_rect(track) {
            self.dragging = true;
            self.drag_start_offset_y = self.offset_y;
            self.drag_anchor_y = pointer_y - thumb.y;
            return true;
        }
        false
    }

    /// Update the offset during an active thumb drag.
    /// `pointer_y` is the current pointer y coordinate.
    pub fn update_drag(&mut self, track: Rect, pointer_y: i32) -> bool {
        if !self.dragging {
            return false;
        }
        let previous = self.offset_y;
        let track_h = track.h.saturating_sub(SCROLLBAR_TRACK_PAD * 2);
        if track_h == 0 {
            return false;
        }
        let thumb_h = self
            .thumb_rect(track)
            .map(|t| t.h)
            .unwrap_or(SCROLLBAR_MIN_THUMB_H);
        let max_travel = track_h.saturating_sub(thumb_h);
        if max_travel == 0 {
            self.offset_y = 0;
            return self.offset_y != previous;
        }
        let local_y = (pointer_y - (track.y + SCROLLBAR_TRACK_PAD as i32) - self.drag_anchor_y)
            .clamp(0, max_travel as i32);
        let max_offset = self.max_offset_y();
        if max_offset == 0 {
            self.offset_y = 0;
            return self.offset_y != previous;
        }
        // offset = local_y * max_offset / max_travel (integer + rounding)
        self.offset_y = ((local_y as u64 * max_offset as u64 + max_travel as u64 / 2)
            / max_travel as u64)
            .min(max_offset as u64) as i32;
        self.clamp();
        self.offset_y != previous
    }

    /// End the current drag.
    pub fn end_drag(&mut self) -> bool {
        let was_dragging = self.dragging;
        self.dragging = false;
        self.drag_anchor_y = 0;
        was_dragging
    }

    /// Handle a track click: page-scroll toward the clicked position.
    /// Returns true if the click was on the track (not on the thumb).
    pub fn handle_track_click(&mut self, track: Rect, pointer_y: i32) -> bool {
        let Some(thumb) = self.thumb_rect(track) else {
            return false;
        };
        if thumb.contains(Point::new(track.x, pointer_y)) {
            return false; // clicked on thumb — caller handles drag
        }
        // Click above thumb -> page up, below thumb -> page down.
        if pointer_y < thumb.y {
            self.page_up();
        } else if pointer_y > thumb.bottom() {
            self.page_down();
        }
        true
    }
}

/// Draw a vertical scrollbar within `region` using `state`.
///
/// The scrollbar is drawn along the right edge of `region`.
/// When `policy` is [`ScrollPolicy::Auto`] and content fits the
/// viewport, nothing is drawn.
pub fn draw_scrollbar(
    canvas: &mut Canvas,
    theme: &Theme,
    region: Rect,
    state: &ScrollState,
    policy: ScrollPolicy,
) {
    if policy == ScrollPolicy::Auto && !state.can_scroll_y() {
        return;
    }
    let track = state.track_rect(region);
    if track.w == 0 || track.h == 0 {
        return;
    }

    let Some(thumb) = state.thumb_rect(track) else {
        return;
    };

    // Thumb colour based on state
    let thumb_color = if state.dragging {
        theme.accent
    } else if state.hovered {
        theme.accent.darken(64)
    } else if state.focused {
        theme.accent.darken(96)
    } else {
        theme.chrome.card_bg.lighten(48)
    };

    // Subtle hover track background
    if state.hovered || state.dragging || state.focused {
        canvas.blend_rect(track, theme.border);
    }

    let radius = (SCROLLBAR_WIDTH / 2).min(3) as u32;
    canvas.fill_rounded_rect(thumb, radius, thumb_color);
}

/// Test whether `point` hits the scrollbar track of `region`.
/// Returns `Some(true)` for thumb hit, `Some(false)` for track hit,
/// `None` for miss.
pub fn hit_test_scrollbar(region: Rect, state: &ScrollState, x: i32, y: i32) -> Option<bool> {
    let track = state.track_rect(region);
    if !track.contains(Point::new(x, y)) {
        return None;
    }
    if let Some(thumb) = state.thumb_rect(track) {
        if thumb.contains(Point::new(x, y)) {
            return Some(true); // thumb hit
        }
    }
    Some(false) // track hit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(viewport_h: u32, content_h: u32) -> ScrollState {
        let mut s = ScrollState::new();
        s.set_geometry(200, viewport_h, 200, content_h);
        s
    }

    #[test]
    fn content_smaller_than_viewport_has_zero_offset() {
        let s = state_with(400, 200);
        assert_eq!(s.max_offset_y(), 0);
        assert!(!s.can_scroll_y());
    }

    #[test]
    fn content_equal_to_viewport_has_zero_offset() {
        let s = state_with(400, 400);
        assert_eq!(s.max_offset_y(), 0);
        assert!(!s.can_scroll_y());
    }

    #[test]
    fn content_larger_than_viewport_has_positive_max() {
        let s = state_with(400, 1000);
        assert_eq!(s.max_offset_y(), 600);
        assert!(s.can_scroll_y());
    }

    #[test]
    fn max_offset_does_not_underflow() {
        let s = state_with(1000, 200);
        assert_eq!(s.max_offset_y(), 0);
    }

    #[test]
    fn scroll_by_clamps_at_limits() {
        let mut s = state_with(200, 600);
        s.scroll_by(-100);
        assert_eq!(s.offset_y, 0);
        s.scroll_by(1000);
        assert_eq!(s.offset_y, 400);
    }

    #[test]
    fn scroll_by_positive_and_negative() {
        let mut s = state_with(200, 1000);
        s.scroll_by(300);
        assert_eq!(s.offset_y, 300);
        s.scroll_by(-100);
        assert_eq!(s.offset_y, 200);
    }

    #[test]
    fn wheel_delta_supports_hid_and_fixed_point_steps() {
        let mut s = state_with(200, 1000);
        assert!(s.scroll_by_wheel(1, 30));
        assert_eq!(s.offset_y, 30);
        assert!(s.scroll_by_wheel(240, 30));
        assert_eq!(s.offset_y, 90);
        assert!(s.scroll_by_wheel(-120, 30));
        assert_eq!(s.offset_y, 60);
    }

    #[test]
    fn clamp_after_content_shrinks() {
        let mut s = state_with(200, 600);
        s.scroll_by(300);
        assert_eq!(s.offset_y, 300);
        s.set_geometry(200, 400, 200, 300); // content now smaller than viewport
        assert_eq!(s.offset_y, 0);
    }

    #[test]
    fn clamp_after_viewport_grows() {
        let mut s = state_with(200, 600);
        s.scroll_by(300);
        s.set_viewport(200, 500); // viewport grew, max offset shrinks
        assert_eq!(s.offset_y, 100); // clamped to max_offset = 100
    }

    #[test]
    fn ensure_visible_already_visible_does_nothing() {
        let mut s = state_with(200, 600);
        s.scroll_by(100);
        assert!(!s.ensure_visible(120, 30)); // item at 120 is visible
        assert_eq!(s.offset_y, 100);
    }

    #[test]
    fn ensure_visible_above_viewport() {
        let mut s = state_with(200, 600);
        s.scroll_by(200);
        assert!(s.ensure_visible(50, 30));
        assert_eq!(s.offset_y, 50);
    }

    #[test]
    fn ensure_visible_below_viewport() {
        let mut s = state_with(200, 600);
        assert!(s.ensure_visible(400, 50));
        assert_eq!(s.offset_y, 250); // 400 + 50 - 200 = 250
    }

    #[test]
    fn thumb_size_proportional() {
        // 200 / 600 = 1/3 of track
        let s = state_with(200, 600);
        let region = Rect::new(0, 0, 200, 200);
        let track = s.track_rect(region);
        let thumb = s.thumb_rect(track).unwrap();
        let track_h = track.h.saturating_sub(SCROLLBAR_TRACK_PAD * 2);
        let expected = (track_h as f64 * 200.0 / 600.0).max(SCROLLBAR_MIN_THUMB_H as f64) as u32;
        assert_eq!(thumb.h, expected);
    }

    #[test]
    fn thumb_at_top_when_scrolled_to_top() {
        let s = state_with(200, 600);
        let region = Rect::new(0, 0, 200, 200);
        let track = s.track_rect(region);
        let thumb = s.thumb_rect(track).unwrap();
        assert_eq!(thumb.y, track.y + SCROLLBAR_TRACK_PAD as i32);
    }

    #[test]
    fn thumb_at_bottom_when_scrolled_to_bottom() {
        let mut s = state_with(200, 600);
        s.scroll_to_y(s.max_offset_y());
        let region = Rect::new(0, 0, 200, 200);
        let track = s.track_rect(region);
        let thumb = s.thumb_rect(track).unwrap();
        let track_h = track.h.saturating_sub(SCROLLBAR_TRACK_PAD * 2);
        let max_travel = track_h - thumb.h;
        assert_eq!(
            thumb.y,
            track.y + SCROLLBAR_TRACK_PAD as i32 + max_travel as i32
        );
    }

    #[test]
    fn drag_thumb_to_middle() {
        let mut s = state_with(200, 600);
        let region = Rect::new(0, 0, 200, 200);
        let track = s.track_rect(region);
        let initial_thumb = s.thumb_rect(track).unwrap();

        // Start drag at thumb center
        let pointer_y = initial_thumb.y + initial_thumb.h as i32 / 2;
        s.start_drag(track, pointer_y);
        assert!(s.dragging);

        // Drag to middle of track
        let mid_y = track.y + track.h as i32 / 2;
        s.update_drag(track, mid_y);
        assert!(s.offset_y > 0);
        assert!(s.offset_y < s.max_offset_y());

        s.end_drag();
        assert!(!s.dragging);
    }

    #[test]
    fn drag_thumb_to_top_and_bottom() {
        let mut s = state_with(200, 800);
        let region = Rect::new(0, 0, 200, 200);
        let track = s.track_rect(region);
        let thumb = s.thumb_rect(track).unwrap();

        // Drag to bottom
        s.start_drag(track, thumb.y + thumb.h as i32 / 2);
        s.update_drag(track, track.bottom());
        assert_eq!(s.offset_y, s.max_offset_y());

        // Drag to top
        let thumb = s.thumb_rect(track).unwrap();
        s.end_drag();
        s.start_drag(track, thumb.y + thumb.h as i32 / 2);
        s.update_drag(track, track.y);
        assert_eq!(s.offset_y, 0);
    }

    #[test]
    fn track_click_above_thumb_pages_up() {
        let mut s = state_with(200, 800);
        s.scroll_to_y(s.max_offset_y()); // scroll to bottom: offset = 600
        let region = Rect::new(0, 0, 200, 200);
        let track = s.track_rect(region);

        // Click at top of track (above thumb)
        let top_y = track.y + SCROLLBAR_TRACK_PAD as i32 + 1;
        s.handle_track_click(track, top_y);
        assert!(s.offset_y < 600); // scrolled up
    }

    #[test]
    fn track_click_below_thumb_pages_down() {
        let mut s = state_with(200, 800);
        // offset is 0, thumb at top
        let region = Rect::new(0, 0, 200, 200);
        let track = s.track_rect(region);

        // Click near bottom of track (below thumb)
        let bottom_y = track.bottom() - SCROLLBAR_TRACK_PAD as i32 - 1;
        s.handle_track_click(track, bottom_y);
        assert!(s.offset_y > 0); // scrolled down
    }

    #[test]
    fn zero_height_viewport_is_stable() {
        let mut s = ScrollState::new();
        s.set_geometry(0, 0, 200, 400);
        assert_eq!(s.max_offset_y(), 0);
        s.scroll_by(100);
        assert_eq!(s.offset_y, 0);
    }

    #[test]
    fn very_small_geometry_is_stable() {
        let mut s = ScrollState::new();
        s.set_geometry(1, 10, 1, 100);
        assert_eq!(s.max_offset_y(), 90);
        s.scroll_by(50);
        assert_eq!(s.offset_y, 50);
        let thumb = s.thumb_rect(Rect::new(0, 0, 10, 10)).unwrap();
        assert!(thumb.h >= SCROLLBAR_MIN_THUMB_H || thumb.h <= 10);
    }

    #[test]
    fn independent_states_do_not_interfere() {
        let mut s1 = state_with(200, 1000);
        let mut s2 = state_with(300, 400);
        s1.scroll_by(500);
        s2.scroll_by(50);
        assert_eq!(s1.offset_y, 500);
        assert_eq!(s2.offset_y, 50);
    }

    #[test]
    fn reset_offset_when_setting_geometry() {
        let mut s = state_with(200, 600);
        s.scroll_by(300);
        s.set_geometry(200, 200, 200, 400);
        assert_eq!(s.offset_y, 200); // max_offset = 200
        s.set_geometry(200, 200, 200, 150); // content < viewport
        assert_eq!(s.offset_y, 0);
    }

    #[test]
    fn page_scroll_does_not_underflow() {
        let mut s = state_with(200, 600);
        s.scroll_by(50);
        s.page_up();
        assert_eq!(s.offset_y, 0); // clamped, not negative
    }

    #[test]
    fn hit_test_scrollbar_distinguishes_thumb_from_track() {
        let s = state_with(200, 600);
        let region = Rect::new(0, 0, 200, 200);
        let track = s.track_rect(region);
        let thumb = s.thumb_rect(track).unwrap();

        // Thumb hit
        let thumb_x = thumb.x + thumb.w as i32 / 2;
        let thumb_y = thumb.y + thumb.h as i32 / 2;
        assert_eq!(hit_test_scrollbar(region, &s, thumb_x, thumb_y), Some(true));

        // Track hit (below thumb)
        let track_below = thumb.bottom() + 2;
        if track_below < track.bottom() {
            assert_eq!(
                hit_test_scrollbar(region, &s, thumb_x, track_below),
                Some(false)
            );
        }

        // Miss (outside scrollbar)
        assert_eq!(hit_test_scrollbar(region, &s, 189, 10), None);
    }
}
