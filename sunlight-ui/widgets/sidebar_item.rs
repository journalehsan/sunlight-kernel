//! `SidebarItem` — reusable navigation item for left-sidebar lists.
//!
//! A modern, lightweight evolution of the Explorer sidebar card: compact,
//! dark, clean, with a rounded hover/selected surface, a 32x32 TGA icon, and
//! an accent pill on the left of the active item.
//!
//! # Visual layout
//!
//! ```text
//!  +------------------------------------+
//!  |  +------------------------------+  |
//!  |  |[icon 32x32]  Title      (3) |  |   <- rounded inner card
//!  |  |              hint / path    |  |
//!  |  +------------------------------+  |
//!  +------------------------------------+
//! ```
//!
//! - The whole row sits on the sidebar background; only the inner **card**
//!   gets a rounded fill for hover/selected/focus states, giving a modern
//!   "floating" look without full-width separators.
//! - A 3-px accent bar is drawn at the left of the card when selected.
//! - A 1-px focus ring (rounded outline) is drawn around the card when
//!   focused (keyboard navigation), without disturbing the card fill.
//!
//! # Purity contract
//!
//! `SidebarItem` is **pure display**. It performs no IPC, no filesystem
//! probing, and no network access. All selection / hover tracking is owned by
//! the caller — this widget only paints itself and answers `hit_test`.
//!
//! # Example
//!
//! ```ignore
//! use sunlight_ui::widgets::{SidebarItem, SidebarState};
//! use sunlight_ui::geom::{Rect, VBox};
//! use sunlight_ui::paint::Canvas;
//! use sunlight_ui::Theme;
//!
//! // Build a 220px sidebar column of 48px rows.
//! let column = Rect::new(0, 0, 220, 480);
//! let vbox   = VBox::new(column).with_spacing(4);
//! let labels = ["Home", "Documents", "Downloads"];
//! let heights = [SidebarItem::HEIGHT; 3];
//! let selected = 1usize;
//!
//! for (i, row) in vbox.layout(&heights).enumerate() {
//!     let state = if i == selected {
//!         SidebarState::Selected
//!     } else {
//!         SidebarState::Normal
//!     };
//!     SidebarItem::new(row, labels[i])
//!         .with_state(state)
//!         .draw(&mut canvas, &theme);
//! }
//!
//! // On Event::Click at (x, y):
//! // let clicked = items.iter().position(|it| it.hit_test(x, y));
//! ```
//!
//! # Icon handling
//!
//! Pass an `Option<&TgaImage>` via [`SidebarItem::with_icon`]. The image is
//! scaled to [`SidebarItem::ICON_SIZE`]×[…] on blit by the
//! existing zero-alloc [`Canvas::draw_tga_icon`] — so any native source size
//! (16, 32, 48, 64, 256 ...) works. **32x32 is the recommended source size**
//! for sidebar use, since it needs no upscaling and stays crisp.
//!
//! Load icons once at startup and store them in app state (the widget only
//! borrows the static bytes behind [`TgaImage`]):
//!
//! ```ignore
//! use sunlight_ui::image::TgaImage;
//!
//! // Embedded via include_bytes! at build time (same pattern as the wallpaper).
//! const ICON_HOME: &[u8] =
//!     include_bytes!("/var/sunlightos/icons/SunlightOS/places/32/user-home.tga");
//!
//! let home_icon = TgaImage::parse(ICON_HOME).ok();
//! SidebarItem::new(rect, "Home").with_icon(home_icon.as_ref().unwrap());
//! ```
//!
//! # Group headers
//!
//! Use [`SidebarGroupHeader`] to add a category label above a set of items.
//! It renders a short all-caps section label — no expand/collapse logic (add
//! that in your app state if needed).

use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::image::TgaImage;
use crate::paint::Canvas;
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// State enum
// ---------------------------------------------------------------------------

/// Visual/interactive state for a [`SidebarItem`].
///
/// The caller owns the notion of "which item is selected/hovered/focused";
/// the widget only consumes one of these values to pick colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarState {
    /// Idle, no interaction.
    Normal,
    /// Pointer is over the item (caller tracks this from pointer-move events).
    Hovered,
    /// Item is the currently-active destination.
    /// Renders a rounded accent-tinted fill plus a left accent bar.
    Selected,
    /// Keyboard focus without activation (accessibility / arrow-key nav).
    /// Renders a focus ring around the card.
    Focused,
    /// Greyed out and non-clickable. `hit_test` returns `false`.
    Disabled,
}

impl SidebarState {
    /// Returns `true` for any state in which the user may trigger the item.
    #[inline]
    pub const fn is_interactive(self) -> bool {
        !matches!(self, SidebarState::Disabled)
    }
}

// ---------------------------------------------------------------------------
// SidebarItem
// ---------------------------------------------------------------------------

/// A compact navigation row for a vertical sidebar.
///
/// Pure display: no IPC, no filesystem, no network. All state is supplied by
/// the caller via [`SidebarState`].
pub struct SidebarItem<'a> {
    pub rect: Rect,
    /// Primary label (e.g. "Documents").
    pub label: &'a str,
    /// Optional secondary hint rendered below the label in dim color.
    /// Useful for item counts, shortcut hints, or path overviews.
    pub hint: Option<&'a str>,
    /// Optional short badge shown at the right edge (e.g. an unread count).
    /// Rendered inside an accent-tinted pill for readability.
    pub badge: Option<&'a str>,
    /// Optional TGA icon. Any native size — bilinear-scaled (premultiplied)
    /// to [`ICON_SIZE`](Self::ICON_SIZE) on blit; 1:1 uses the fast path.
    /// Pass `None` for text rows.
    pub icon: Option<&'a TgaImage>,
    pub state: SidebarState,
    /// Vector font for label rendering. Falls back to the 5×7 bitmap font when `None`.
    pub font: Option<&'a dyn VecText>,
}

impl<'a> SidebarItem<'a> {
    /// Recommended item height; use with [`VBox`](crate::geom::VBox).
    /// Comfortably fits a 32x32 icon plus a two-line title/hint block.
    pub const HEIGHT: u32 = 48;

    /// Icon render size (scaled from the TGA's native size).
    /// 32x32 is the recommended source resolution for sidebar icons.
    pub const ICON_SIZE: u32 = 32;

    /// Inset of the floating "card" from the row edges (all four sides).
    /// Gives the modern breathing-room look between adjacent items.
    const CARD_INSET: i32 = 3;
    /// Corner radius of the card background and focus ring.
    const CARD_RADIUS: u32 = 8;
    /// Inner horizontal padding: icon/text offset from the card's left edge.
    /// The accent bar is drawn within this space — text never shifts.
    const PAD_H: i32 = 10;
    /// Gap between the right edge of the icon and the start of the text column.
    const ICON_TEXT_GAP: i32 = 10;
    /// Width of the accent bar painted on the left edge of a selected item.
    /// Thin by design — the fill is restrained; the bar carries the accent.
    const ACCENT_BAR_W: u32 = 2;
    /// Right margin reserved for an optional badge.
    const BADGE_MARGIN: i32 = 10;

    pub fn new(rect: Rect, label: &'a str) -> Self {
        Self {
            rect,
            label,
            hint: None,
            badge: None,
            icon: None,
            state: SidebarState::Normal,
            font: None,
        }
    }

    /// Optional second line shown in dim text below the title.
    pub fn with_hint(mut self, hint: &'a str) -> Self {
        self.hint = Some(hint);
        self
    }

    /// Optional short right-aligned badge (e.g. an unread count).
    pub fn with_badge(mut self, badge: &'a str) -> Self {
        self.badge = Some(badge);
        self
    }

    /// Optional TGA icon, scaled to [`ICON_SIZE`](Self::ICON_SIZE).
    pub fn with_icon(mut self, icon: &'a TgaImage) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Set the interactive/visual state of this item.
    pub fn with_state(mut self, state: SidebarState) -> Self {
        self.state = state;
        self
    }

    /// Convenience: set state to [`SidebarState::Focused`].
    pub fn with_focused(self) -> Self {
        self.with_state(SidebarState::Focused)
    }

    /// Convenience: set state to [`SidebarState::Selected`].
    pub fn selected(self) -> Self {
        self.with_state(SidebarState::Selected)
    }

    /// Attach a vector font renderer; falls back to 5×7 bitmap when `None`.
    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    /// The inner "card" rect — inset from [`rect`](Self::rect) on all sides.
    /// Exposed so callers can align sibling widgets (e.g. custom badges).
    pub fn card_rect(&self) -> Rect {
        self.rect.inset(Self::CARD_INSET)
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let card = self.card_rect();

        // ── Card background by state ──────────────────────────────────────
        // Selected uses the shared restrained chrome selection (same family as
        // list rows).  Identity is the thin left accent bar — not a saturated
        // orange slab across the whole item.
        let bg = match self.state {
            SidebarState::Selected => theme.chrome.selection,
            SidebarState::Hovered => theme.panel_alt.lighten(8),
            SidebarState::Focused => theme.panel_alt,
            _ => theme.panel,
        };
        canvas.fill_rounded_rect(card, Self::CARD_RADIUS, bg);

        // ── Left accent pill for the selected item ────────────────────────
        if self.state == SidebarState::Selected {
            let bar_h = card.h.saturating_sub(8).max(8);
            let bar = Rect::new(
                card.x + 3,
                card.y + (card.h.saturating_sub(bar_h) as i32) / 2,
                Self::ACCENT_BAR_W,
                bar_h,
            );
            canvas.fill_rounded_rect(bar, Self::ACCENT_BAR_W, theme.accent);
        }

        // ── Focus ring (keyboard focus) ───────────────────────────────────
        if self.state == SidebarState::Focused {
            canvas.stroke_rounded_rect(card, Self::CARD_RADIUS, 1, theme.accent_hover);
        }

        // ── Hovered: faint top highlight line for depth ───────────────────
        if self.state == SidebarState::Hovered {
            canvas.hbar(
                card.x + Self::CARD_RADIUS as i32,
                card.y,
                card.w.saturating_sub(2 * Self::CARD_RADIUS) as u32,
                1,
                theme.panel_alt.lighten(28),
            );
        }

        // ── Content placement ─────────────────────────────────────────────
        // Content starts after PAD_H (accent bar fits inside that padding).
        let content_x = card.x + Self::PAD_H;

        let text_x = if let Some(icon) = self.icon {
            let icon_y = card.y + (card.h as i32 - Self::ICON_SIZE as i32) / 2;
            canvas.draw_tga_icon(
                icon,
                Rect::new(content_x, icon_y, Self::ICON_SIZE, Self::ICON_SIZE),
            );
            content_x + Self::ICON_SIZE as i32 + Self::ICON_TEXT_GAP
        } else {
            content_x
        };

        // ── Text colors ───────────────────────────────────────────────────
        let label_color = if self.state == SidebarState::Disabled {
            theme.text_dim
        } else {
            theme.text
        };

        // ── Title + hint vertical placement ───────────────────────────────
        let font_h = self.font.map_or(crate::paint::font::GLYPH_H as i32, |f| {
            f.line_height() as i32
        });
        let (title_y, hint_y) = if self.hint.is_some() {
            // Two lines: center the pair vertically inside the card height.
            let block_h = font_h * 2 + 3;
            let top = card.y + (card.h as i32 - block_h) / 2;
            (top, Some(top + font_h + 3))
        } else {
            (card.y + (card.h as i32 - font_h) / 2, None)
        };

        if let Some(f) = self.font {
            f.draw(canvas, self.label, text_x, title_y, label_color);
            if let (Some(h), Some(hy)) = (self.hint, hint_y) {
                f.draw(canvas, h, text_x, hy, theme.text_dim);
            }
        } else {
            canvas.draw_text(text_x, title_y, self.label, label_color);
            if let (Some(h), Some(hy)) = (self.hint, hint_y) {
                canvas.draw_text(text_x, hy, h, theme.text_dim);
            }
        }

        // ── Badge (right-aligned pill) ────────────────────────────────────
        if let Some(b) = self.badge {
            let w = if let Some(f) = self.font {
                f.measure_w(b) as i32
            } else {
                (b.len() as i32) * (crate::paint::font::GLYPH_W as i32 + 1)
            };
            let bx = card.right() - Self::BADGE_MARGIN - w;
            let by = card.y + (card.h as i32 - font_h) / 2;
            let pill = Rect::new(bx - 4, by - 2, w as u32 + 8, font_h as u32 + 4);
            canvas.fill_rounded_rect(pill, 6, theme.accent.darken(160));
            if let Some(f) = self.font {
                f.draw(canvas, b, bx, by, theme.accent.lighten(20));
            } else {
                canvas.draw_text(bx, by, b, theme.accent.lighten(20));
            }
        }
    }

    /// Returns `true` if `(x, y)` falls inside this item and it is interactive.
    ///
    /// Disabled items never report hits. Use this in your event handler on a
    /// click/pointer event to detect selection — the widget performs no
    /// navigation itself.
    pub fn hit_test(&self, x: i32, y: i32) -> bool {
        self.state.is_interactive() && self.rect.contains(Point::new(x, y))
    }
}

// ---------------------------------------------------------------------------
// SidebarGroupHeader
// ---------------------------------------------------------------------------

/// A thin section-label row for grouping sidebar items.
///
/// Renders a short all-caps label in dim text — no expand/collapse behaviour.
/// Add that in your app state and conditionally skip drawing the grouped items.
///
/// # Example
///
/// ```ignore
/// let header = SidebarGroupHeader::new(header_rect, "PLACES");
/// header.draw(&mut canvas, &theme);
/// ```
pub struct SidebarGroupHeader<'a> {
    pub rect: Rect,
    pub label: &'a str,
}

impl<'a> SidebarGroupHeader<'a> {
    /// Recommended height for a group header row.
    pub const HEIGHT: u32 = 24;

    pub const fn new(rect: Rect, label: &'a str) -> Self {
        Self { rect, label }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(self.rect, theme.bg);

        // Bottom separator — visually separates header from items below.
        canvas.hbar(
            self.rect.x,
            self.rect.bottom() - 1,
            self.rect.w,
            1,
            theme.border,
        );

        // Left-aligned label, vertically centered.
        let font_h = crate::paint::font::GLYPH_H as i32;
        let ty = self.rect.y + (self.rect.h as i32 - font_h) / 2;
        canvas.draw_text(self.rect.x + 8, ty, self.label, theme.text_dim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint::Canvas;
    use crate::theme::Theme;

    #[test]
    fn state_interactive() {
        assert!(SidebarState::Normal.is_interactive());
        assert!(SidebarState::Hovered.is_interactive());
        assert!(SidebarState::Selected.is_interactive());
        assert!(SidebarState::Focused.is_interactive());
        assert!(!SidebarState::Disabled.is_interactive());
    }

    #[test]
    fn hit_test_respects_disabled_and_bounds() {
        let item = SidebarItem::new(Rect::new(0, 0, 200, 48), "Home");
        assert!(item.hit_test(10, 10));
        assert!(!item.hit_test(300, 10)); // outside x
        assert!(!item.hit_test(10, 100)); // outside y

        let disabled =
            SidebarItem::new(Rect::new(0, 0, 200, 48), "Home").with_state(SidebarState::Disabled);
        assert!(!disabled.hit_test(10, 10));
    }

    #[test]
    fn card_rect_is_inset() {
        let item = SidebarItem::new(Rect::new(0, 0, 200, 48), "Home");
        let card = item.card_rect();
        assert_eq!(card.x, 3);
        assert_eq!(card.y, 3);
        assert_eq!(card.w, 194);
        assert_eq!(card.h, 42);
    }

    #[test]
    fn builders_set_fields() {
        let item = SidebarItem::new(Rect::new(0, 0, 10, 10), "X")
            .with_hint("hint")
            .with_badge("3")
            .selected();
        assert_eq!(item.hint, Some("hint"));
        assert_eq!(item.badge, Some("3"));
        assert_eq!(item.state, SidebarState::Selected);
    }

    #[test]
    fn draw_all_states_does_not_panic() {
        // Verify the draw path runs for every state without panicking. Pixel
        // correctness is covered by the paint/draw module tests.
        let theme = Theme::sunlight_dark();
        let mut pixels = [0u32; 220 * 48];
        for &state in &[
            SidebarState::Normal,
            SidebarState::Hovered,
            SidebarState::Selected,
            SidebarState::Focused,
            SidebarState::Disabled,
        ] {
            let mut canvas = Canvas::new(&mut pixels, 220, 220, 48);
            SidebarItem::new(Rect::new(0, 0, 220, 48), "Documents")
                .with_hint("/home/docs")
                .with_badge("9")
                .with_state(state)
                .draw(&mut canvas, &theme);
        }
    }
}
