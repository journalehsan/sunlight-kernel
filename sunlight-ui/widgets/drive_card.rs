//! `DriveCard` — reusable drive/volume presentation widget.
//!
//! Displays a single drive or volume as a card (grid) or row (list). The
//! widget is pure display: no scanning, no IPC, no filesystem access. All data
//! is supplied by the caller.
//!
//! # Visual layout — Card mode (grid)
//!
//! ```text
//!  +-------------------------------+
//!  |        [icon 48×48]           |
//!  |        Primary Drive          |
//!  |        /mnt/usb               |
//!  |        ext4                   |
//!  |  [===========-----------]     |
//!  |  240 GiB used    160 GiB free |
//!  +-------------------------------+
//! ```
//!
//! # Visual layout — Row mode (list)
//!
//! ```text
//!  +--------------------------------------------------+
//!  | [icon 36x36]  Primary Drive   /mnt/usb  ext4    |
//!  |               [===========-----------]           |
//!  |               240 GiB used    160 GiB free       |
//!  +--------------------------------------------------+
//! ```
//!
//! # Example
//!
//! ```ignore
//! use sunlight_ui::widgets::{DriveCard, DriveCardState, DriveCardLayout};
//! use sunlight_ui::geom::Rect;
//!
//! // Grid card, with capacity
//! DriveCard::new(Rect::new(10, 10, DriveCard::CARD_W, DriveCard::CARD_H), "System")
//!     .with_mount_path("/")
//!     .with_fs_type("ext4")
//!     .with_usage(0.62)
//!     .with_capacity("400 GiB", "248 GiB", "152 GiB")
//!     .with_state(DriveCardState::Hovered)
//!     .draw(&mut canvas, &theme);
//!
//! // List row (no capacity data)
//! DriveCard::new(Rect::new(0, 60, 480, DriveCard::ROW_H), "USB Drive")
//!     .with_layout(DriveCardLayout::Row)
//!     .with_mount_path("/mnt/usb")
//!     .draw(&mut canvas, &theme);
//! ```

use crate::geom::{Point, Rect};
use crate::image::TgaImage;
use crate::paint::Canvas;
use crate::theme::Theme;

// ── Layout ─────────────────────────────────────────────────────────────────

/// Render layout for a [`DriveCard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveCardLayout {
    /// Icon centered at top, info stacked below. Best for 2–4 column grids.
    Card,
    /// Icon on the left, info columns to the right. Best for full-width lists.
    Row,
}

// ── State ───────────────────────────────────────────────────────────────────

/// Visual/interactive state for a [`DriveCard`].
///
/// Caller owns all state; the widget only paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveCardState {
    /// Idle, no interaction.
    Normal,
    /// Pointer is over the card.
    Hovered,
    /// Card is the currently-active/selected drive.
    Selected,
    /// Drive is unavailable or unmounted.
    Disabled,
}

impl DriveCardState {
    #[inline]
    pub const fn is_interactive(self) -> bool {
        !matches!(self, DriveCardState::Disabled)
    }
}

// ── Widget ──────────────────────────────────────────────────────────────────

/// Drive/volume card widget — pure display.
///
/// Accepts pre-formatted strings for capacity fields; the widget does no
/// number formatting. Pass values like `"248 GiB"` or `"60%"` directly.
pub struct DriveCard<'a> {
    pub rect: Rect,
    /// Display name shown as the primary label.
    pub name: &'a str,
    /// Optional TGA icon. Nearest-neighbour scaled to the icon area.
    pub icon: Option<&'a TgaImage>,
    /// Optional mount point or path (e.g. `"/"`, `"/mnt/usb"`).
    pub mount_path: Option<&'a str>,
    /// Optional filesystem type label (e.g. `"ext4"`, `"FAT32"`).
    pub fs_type: Option<&'a str>,
    /// Pre-formatted total capacity string (e.g. `"400 GiB"`).
    pub total: Option<&'a str>,
    /// Pre-formatted used capacity string (e.g. `"248 GiB"`).
    pub used: Option<&'a str>,
    /// Pre-formatted free capacity string (e.g. `"152 GiB"`).
    pub free: Option<&'a str>,
    /// Fill ratio for the usage bar, `0.0..=1.0`. Requires either
    /// [`usage_ratio`](Self) or capacity strings — pass `None` to hide the bar.
    pub usage_ratio: Option<f32>,
    pub layout: DriveCardLayout,
    pub state: DriveCardState,
}

impl<'a> DriveCard<'a> {
    // ── Recommended dimensions ────────────────────────────────────────────

    /// Recommended card width for grid/Card layout.
    pub const CARD_W: u32 = 160;
    /// Recommended card height for Card layout with full capacity display.
    pub const CARD_H: u32 = 148;
    /// Recommended row height for Row/list layout.
    pub const ROW_H: u32 = 60;

    // ── Internal constants ────────────────────────────────────────────────

    const CARD_RADIUS: u32 = 10;
    const CARD_INSET: i32 = 3;
    const CARD_ICON_SIZE: u32 = 48;
    const ROW_ICON_SIZE: u32 = 36;
    const PAD: i32 = 10;
    const BAR_H: u32 = 6;
    const ACCENT_BAR_W: u32 = 3;

    // ── Constructors & builders ───────────────────────────────────────────

    pub fn new(rect: Rect, name: &'a str) -> Self {
        Self {
            rect,
            name,
            icon: None,
            mount_path: None,
            fs_type: None,
            total: None,
            used: None,
            free: None,
            usage_ratio: None,
            layout: DriveCardLayout::Card,
            state: DriveCardState::Normal,
        }
    }

    pub fn with_icon(mut self, icon: &'a TgaImage) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_mount_path(mut self, path: &'a str) -> Self {
        self.mount_path = Some(path);
        self
    }

    pub fn with_fs_type(mut self, fs: &'a str) -> Self {
        self.fs_type = Some(fs);
        self
    }

    /// Attach pre-formatted capacity strings. Pass all three or none.
    pub fn with_capacity(mut self, total: &'a str, used: &'a str, free: &'a str) -> Self {
        self.total = Some(total);
        self.used = Some(used);
        self.free = Some(free);
        self
    }

    /// Set the usage fill ratio (`0.0`–`1.0`). Draws a color-coded progress
    /// bar — green / amber / red depending on fill level.
    pub fn with_usage(mut self, ratio: f32) -> Self {
        self.usage_ratio = Some(ratio.clamp(0.0, 1.0));
        self
    }

    pub fn with_layout(mut self, layout: DriveCardLayout) -> Self {
        self.layout = layout;
        self
    }

    pub fn with_state(mut self, state: DriveCardState) -> Self {
        self.state = state;
        self
    }

    pub fn selected(self) -> Self {
        self.with_state(DriveCardState::Selected)
    }

    // ── Draw ─────────────────────────────────────────────────────────────

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let card = self.rect.inset(Self::CARD_INSET);

        // ── Card background ───────────────────────────────────────────────
        let bg = match self.state {
            DriveCardState::Selected => theme.accent.darken(148),
            DriveCardState::Hovered => theme.panel_alt.lighten(10),
            DriveCardState::Disabled => theme.panel.darken(20),
            DriveCardState::Normal => theme.panel,
        };
        canvas.fill_rounded_rect(card, Self::CARD_RADIUS, bg);

        // ── Border (accent when selected, subtle otherwise) ───────────────
        let border_color = if self.state == DriveCardState::Selected {
            theme.accent.darken(60)
        } else {
            theme.border
        };
        canvas.stroke_rounded_rect(card, Self::CARD_RADIUS, 1, border_color);

        // ── Selected: left accent pill ────────────────────────────────────
        if self.state == DriveCardState::Selected {
            let bar_h = card.h.saturating_sub(2 * Self::CARD_RADIUS);
            if bar_h > 0 {
                let bar = Rect::new(
                    card.x + 2,
                    card.y + Self::CARD_RADIUS as i32,
                    Self::ACCENT_BAR_W,
                    bar_h,
                );
                canvas.fill_rounded_rect(bar, Self::ACCENT_BAR_W, theme.accent);
            }
        }

        // ── Hovered: subtle top sheen ─────────────────────────────────────
        if self.state == DriveCardState::Hovered {
            canvas.hbar(
                card.x + Self::CARD_RADIUS as i32,
                card.y,
                card.w.saturating_sub(2 * Self::CARD_RADIUS),
                1,
                theme.panel_alt.lighten(30),
            );
        }

        match self.layout {
            DriveCardLayout::Card => self.draw_card_body(canvas, theme, card),
            DriveCardLayout::Row => self.draw_row_body(canvas, theme, card),
        }
    }

    // ── Card (grid) body layout ───────────────────────────────────────────

    fn draw_card_body(&self, canvas: &mut Canvas, theme: &Theme, card: Rect) {
        let font_h = crate::paint::font::GLYPH_H as i32;
        let icon_size = Self::CARD_ICON_SIZE;

        // Icon — centered horizontally, top-padded
        let icon_x = card.x + (card.w as i32 - icon_size as i32) / 2;
        let icon_y = card.y + Self::PAD;
        self.draw_icon_or_placeholder(canvas, theme, icon_x, icon_y, icon_size);

        let text_color = if self.state == DriveCardState::Disabled {
            theme.text_dim
        } else {
            theme.text
        };

        let mut cy = icon_y + icon_size as i32 + 6;

        // Drive name — centered
        draw_centered(canvas, card, cy, self.name, text_color);
        cy += font_h + 3;

        // Mount path — centered, dim
        if let Some(path) = self.mount_path {
            draw_centered(canvas, card, cy, path, theme.text_dim);
            cy += font_h + 2;
        }

        // FS type — centered, dimmer
        if let Some(fs) = self.fs_type {
            draw_centered(canvas, card, cy, fs, theme.text_dim.darken(20));
            cy += font_h + 5;
        }

        // Usage bar
        if let Some(ratio) = self.usage_ratio {
            cy = self.draw_usage_bar(canvas, theme, card, cy, ratio);
        }

        // Capacity row: "used" left-aligned, "free" right-aligned
        self.draw_capacity_row(canvas, theme, card, cy);
    }

    // ── Row (list) body layout ────────────────────────────────────────────

    fn draw_row_body(&self, canvas: &mut Canvas, theme: &Theme, card: Rect) {
        let font_h = crate::paint::font::GLYPH_H as i32;
        let icon_size = Self::ROW_ICON_SIZE;

        let icon_x = card.x + Self::PAD;
        let icon_y = card.y + (card.h as i32 - icon_size as i32) / 2;
        self.draw_icon_or_placeholder(canvas, theme, icon_x, icon_y, icon_size);

        let text_color = if self.state == DriveCardState::Disabled {
            theme.text_dim
        } else {
            theme.text
        };

        // Text block starts after icon
        let tx = icon_x + icon_size as i32 + Self::PAD;
        let text_area = Rect::new(tx, card.y, (card.right() - tx).max(0) as u32, card.h);

        let has_bar = self.usage_ratio.is_some();
        let has_cap = self.used.is_some() || self.total.is_some();
        let info_lines = has_bar as i32 + has_cap as i32;
        let total_text_h = if info_lines > 0 {
            // name + separator line + bar/cap lines
            font_h * 2 + 4 + info_lines * (font_h + 2)
        } else {
            font_h * 2 + 3
        };

        let mut cy = card.y + (card.h as i32 - total_text_h) / 2;

        // Name
        canvas.draw_text(text_area.x, cy, self.name, text_color);
        cy += font_h + 3;

        // Mount path and fs type on one line separated by space
        {
            let mut lx = text_area.x;
            if let Some(path) = self.mount_path {
                lx = canvas.draw_text(lx, cy, path, theme.text_dim);
                lx += 4;
            }
            if let Some(fs) = self.fs_type {
                // Dot separator
                lx = canvas.draw_text(lx, cy, "· ", theme.text_dim.darken(30));
                canvas.draw_text(lx, cy, fs, theme.text_dim.darken(20));
            }
            cy += font_h + 4;
        }

        // Usage bar
        if let Some(ratio) = self.usage_ratio {
            cy = self.draw_usage_bar(canvas, theme, text_area, cy, ratio);
        }

        // Capacity row
        self.draw_capacity_row(canvas, theme, text_area, cy);
    }

    // ── Shared helpers ────────────────────────────────────────────────────

    fn draw_icon_or_placeholder(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        x: i32,
        y: i32,
        size: u32,
    ) {
        let rect = Rect::new(x, y, size, size);
        if let Some(icon) = self.icon {
            canvas.draw_tga_icon(icon, rect);
        } else {
            // Placeholder: rounded square with a stylised "HDD" label
            canvas.fill_rounded_rect(rect, 8, theme.panel_alt);
            canvas.stroke_rounded_rect(rect, 8, 1, theme.border);
            let font_h = crate::paint::font::GLYPH_H as i32;
            let label = "HDD";
            let tw = Canvas::measure_text(label) as i32;
            let lx = x + (size as i32 - tw) / 2;
            let ly = y + (size as i32 - font_h) / 2;
            canvas.draw_text(lx, ly, label, theme.text_dim);
        }
    }

    /// Draw the usage progress bar at `cy`, returns new `cy` below the bar.
    fn draw_usage_bar(&self, canvas: &mut Canvas, theme: &Theme, area: Rect, cy: i32, ratio: f32) -> i32 {
        let bar_x = area.x;
        let bar_w = area.w;
        let bar_rect = Rect::new(bar_x, cy, bar_w, Self::BAR_H);

        // Track
        canvas.fill_rounded_rect(bar_rect, Self::BAR_H / 2, theme.panel_alt);

        // Fill — color encodes severity
        let fill_w = ((bar_w as f32) * ratio) as u32;
        if fill_w > 0 {
            let fill_color = if ratio > 0.85 {
                theme.danger
            } else if ratio > 0.65 {
                theme.warn
            } else {
                theme.accent
            };
            let fill_rect = Rect::new(bar_x, cy, fill_w, Self::BAR_H);
            canvas.fill_rounded_rect(fill_rect, Self::BAR_H / 2, fill_color);
        }

        cy + Self::BAR_H as i32 + 3
    }

    /// Draw capacity labels at `cy` (used left, free right).
    fn draw_capacity_row(&self, canvas: &mut Canvas, theme: &Theme, area: Rect, cy: i32) {
        let left_x = area.x;
        let right_edge = area.right();

        match (self.used, self.free) {
            (Some(used), Some(free)) => {
                canvas.draw_text(left_x, cy, used, theme.accent.lighten(20));
                let fw = Canvas::measure_text(free) as i32;
                canvas.draw_text(right_edge - fw, cy, free, theme.text_dim);
            }
            (Some(used), None) => {
                canvas.draw_text(left_x, cy, used, theme.accent.lighten(20));
            }
            (None, Some(free)) => {
                canvas.draw_text(left_x, cy, free, theme.text_dim);
            }
            (None, None) => {
                if let Some(total) = self.total {
                    let tw = Canvas::measure_text(total) as i32;
                    let tx = left_x + (area.w as i32 - tw) / 2;
                    canvas.draw_text(tx.max(left_x), cy, total, theme.text_dim);
                }
            }
        }
    }

    // ── Hit test ──────────────────────────────────────────────────────────

    /// Returns `true` if `(x, y)` is inside this card and it is interactive.
    pub fn hit_test(&self, x: i32, y: i32) -> bool {
        self.state.is_interactive() && self.rect.contains(Point::new(x, y))
    }
}

// ── Module-private helpers ────────────────────────────────────────────────────

fn draw_centered(canvas: &mut Canvas, area: Rect, y: i32, text: &str, color: crate::theme::Color) {
    let tw = Canvas::measure_text(text) as i32;
    let x = area.x + (area.w as i32 - tw) / 2;
    canvas.draw_text(x.max(area.x), y, text, color);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::Canvas;
    use crate::theme::Theme;

    const W: u32 = DriveCard::CARD_W;
    const H: u32 = DriveCard::CARD_H;

    #[test]
    fn state_interactive() {
        assert!(DriveCardState::Normal.is_interactive());
        assert!(DriveCardState::Hovered.is_interactive());
        assert!(DriveCardState::Selected.is_interactive());
        assert!(!DriveCardState::Disabled.is_interactive());
    }

    #[test]
    fn hit_test_bounds_and_disabled() {
        let card = DriveCard::new(Rect::new(0, 0, W, H), "System");
        assert!(card.hit_test(10, 10));
        assert!(!card.hit_test(W as i32 + 10, 10));
        assert!(!card.hit_test(10, H as i32 + 10));

        let disabled = DriveCard::new(Rect::new(0, 0, W, H), "System")
            .with_state(DriveCardState::Disabled);
        assert!(!disabled.hit_test(10, 10));
    }

    #[test]
    fn builders_set_fields() {
        let card = DriveCard::new(Rect::new(0, 0, W, H), "USB")
            .with_mount_path("/mnt/usb")
            .with_fs_type("FAT32")
            .with_capacity("64 GiB", "40 GiB", "24 GiB")
            .with_usage(0.625)
            .selected();
        assert_eq!(card.mount_path, Some("/mnt/usb"));
        assert_eq!(card.fs_type, Some("FAT32"));
        assert_eq!(card.total, Some("64 GiB"));
        assert_eq!(card.used, Some("40 GiB"));
        assert_eq!(card.free, Some("24 GiB"));
        assert!((card.usage_ratio.unwrap() - 0.625).abs() < f32::EPSILON);
        assert_eq!(card.state, DriveCardState::Selected);
    }

    #[test]
    fn draw_card_all_states_no_panic() {
        let theme = Theme::sunlight_dark();
        let mut pixels = vec![0u32; (W * H) as usize];
        for &state in &[
            DriveCardState::Normal,
            DriveCardState::Hovered,
            DriveCardState::Selected,
            DriveCardState::Disabled,
        ] {
            let mut canvas = Canvas::new(&mut pixels, W, W, H);
            DriveCard::new(Rect::new(0, 0, W, H), "System")
                .with_mount_path("/")
                .with_fs_type("ext4")
                .with_capacity("400 GiB", "248 GiB", "152 GiB")
                .with_usage(0.62)
                .with_state(state)
                .draw(&mut canvas, &theme);
        }
    }

    #[test]
    fn draw_card_minimal_no_panic() {
        let theme = Theme::sunlight_dark();
        let mut pixels = vec![0u32; (W * H) as usize];
        let mut canvas = Canvas::new(&mut pixels, W, W, H);
        DriveCard::new(Rect::new(0, 0, W, H), "Drive").draw(&mut canvas, &theme);
    }

    #[test]
    fn draw_row_all_states_no_panic() {
        let rw: u32 = 480;
        let rh = DriveCard::ROW_H;
        let theme = Theme::sunlight_dark();
        let mut pixels = vec![0u32; (rw * rh) as usize];
        for &state in &[
            DriveCardState::Normal,
            DriveCardState::Hovered,
            DriveCardState::Selected,
            DriveCardState::Disabled,
        ] {
            let mut canvas = Canvas::new(&mut pixels, rw, rw, rh);
            DriveCard::new(Rect::new(0, 0, rw, rh), "USB Drive")
                .with_layout(DriveCardLayout::Row)
                .with_mount_path("/mnt/usb")
                .with_fs_type("FAT32")
                .with_usage(0.5)
                .with_capacity("64 GiB", "32 GiB", "32 GiB")
                .with_state(state)
                .draw(&mut canvas, &theme);
        }
    }

    #[test]
    fn usage_clamped() {
        let card = DriveCard::new(Rect::new(0, 0, W, H), "X").with_usage(2.5);
        assert_eq!(card.usage_ratio, Some(1.0));
        let card2 = DriveCard::new(Rect::new(0, 0, W, H), "X").with_usage(-1.0);
        assert_eq!(card2.usage_ratio, Some(0.0));
    }
}
