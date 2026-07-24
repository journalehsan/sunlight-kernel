//! Compact Workspace Switcher panel and card primitives.
//!
//! Pure view/layout widgets: shells supply bounded view models. No compositor
//! or process queries live here. Live workspace thumbnails are intentionally
//! out of scope; cards leave a stable content band for a future preview slot.

use crate::{
    geom::{Point, Rect},
    image::TgaImage,
    material::{Material, MaterialPalette, SurfaceRole},
    paint::Canvas,
    theme::{Color, Theme},
};

/// Fixed workspace count for the current shell phase.
pub const WORKSPACE_CARD_COUNT: usize = 4;
/// Maximum application icons drawn inside one card.
pub const WORKSPACE_ICON_SLOTS: usize = 3;

/// Interactive / visual flags for a workspace card.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WorkspaceCardState {
    /// Card represents the currently active workspace.
    pub active: bool,
    /// Keyboard focus is on this card.
    pub focused: bool,
    /// Pointer is over this card.
    pub hovered: bool,
}

/// Bounded plain view model consumed by [`WorkspaceCard`].
///
/// Strings and icon references are owned by the caller for the duration of
/// the draw. Icons are optional so missing assets fall back to a neutral
/// glyph drawn by the shell or toolkit.
#[derive(Debug, Clone, Copy)]
pub struct WorkspaceCardView<'a> {
    /// Workspace id in `1..=4`.
    pub id: u8,
    /// Display name (for example `"Workspace 1"`).
    pub title: &'a str,
    /// Real normal application-window count for this workspace.
    pub window_count: u32,
    /// True when no normal application windows exist.
    pub empty: bool,
    /// Up to [`WORKSPACE_ICON_SLOTS`] unique app icons.
    pub icons: &'a [Option<&'a TgaImage>],
    /// Overflow applications beyond the icon slots (`0` hides `+N`).
    pub overflow: u32,
    pub state: WorkspaceCardState,
}

/// Layout for a compact horizontal row of workspace cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceSwitcherLayout {
    pub panel: Rect,
    pub cards: [Rect; WORKSPACE_CARD_COUNT],
}

impl WorkspaceSwitcherLayout {
    /// Preferred panel size targets at 1× (logical pixels).
    pub const PANEL_W_MIN: u32 = 640;
    pub const PANEL_W_MAX: u32 = 820;
    pub const PANEL_H_MIN: u32 = 130;
    pub const PANEL_H_MAX: u32 = 180;
    pub const CARD_W_MIN: u32 = 130;
    pub const CARD_W_MAX: u32 = 175;
    pub const CARD_H_MIN: u32 = 90;
    pub const CARD_H_MAX: u32 = 125;
    pub const PANEL_PAD: i32 = 14;
    pub const CARD_GAP: i32 = 10;
    /// Gap between the panel bottom and the top of the bottom taskbar/dock.
    pub const DOCK_GAP: i32 = 12;
    pub const PANEL_RADIUS: u32 = 12;
    pub const CARD_RADIUS: u32 = 10;

    /// Compute a centered panel immediately above the bottom dock.
    ///
    /// `dock_top_y` is the top edge of the bottom taskbar/dock cluster.
    /// Placement is clamped inside the usable desktop area below `top_inset`.
    pub fn compute(screen_w: u32, screen_h: u32, top_inset: i32, dock_top_y: i32) -> Self {
        let usable_w = screen_w.saturating_sub(24);
        let panel_w = Self::PANEL_W_MAX
            .min(usable_w)
            .max(Self::PANEL_W_MIN.min(usable_w));

        let inner_w = panel_w.saturating_sub((Self::PANEL_PAD * 2) as u32);
        let gaps = (WORKSPACE_CARD_COUNT.saturating_sub(1) as u32) * Self::CARD_GAP as u32;
        let card_w = ((inner_w.saturating_sub(gaps)) / WORKSPACE_CARD_COUNT as u32)
            .clamp(Self::CARD_W_MIN.min(inner_w / 4), Self::CARD_W_MAX);

        let card_h = Self::CARD_H_MAX
            .min(Self::CARD_H_MIN.saturating_add(card_w.saturating_sub(Self::CARD_W_MIN) / 3))
            .clamp(Self::CARD_H_MIN, Self::CARD_H_MAX);

        let panel_h = (card_h as i32 + Self::PANEL_PAD * 2)
            .clamp(Self::PANEL_H_MIN as i32, Self::PANEL_H_MAX as i32) as u32;

        // Prefer sitting just above the dock; clamp so the panel stays on-screen
        // and below the top inset (top panel / reserved chrome).
        let preferred_y = dock_top_y
            .saturating_sub(Self::DOCK_GAP)
            .saturating_sub(panel_h as i32);
        let min_y = top_inset.max(4);
        let max_y = (screen_h as i32)
            .saturating_sub(panel_h as i32)
            .saturating_sub(4)
            .max(min_y);
        let panel_y = preferred_y.clamp(min_y, max_y);

        let panel_x = ((screen_w as i32 - panel_w as i32) / 2).max(4);
        // Final horizontal clamp so the right edge stays inside the framebuffer.
        let panel_x = panel_x.min(
            (screen_w as i32)
                .saturating_sub(panel_w as i32)
                .saturating_sub(4)
                .max(4),
        );

        let panel = Rect::new(panel_x, panel_y, panel_w, panel_h);

        let total_cards_w = card_w * WORKSPACE_CARD_COUNT as u32 + gaps;
        let row_x = panel.x + (panel.w as i32 - total_cards_w as i32) / 2;
        let row_y = panel.y + (panel.h as i32 - card_h as i32) / 2;

        let mut cards = [Rect::new(0, 0, 0, 0); WORKSPACE_CARD_COUNT];
        let mut x = row_x;
        for card in cards.iter_mut() {
            *card = Rect::new(x, row_y, card_w, card_h);
            x += card_w as i32 + Self::CARD_GAP;
        }

        Self { panel, cards }
    }

    pub fn card_index_at(&self, point: Point) -> Option<usize> {
        self.cards
            .iter()
            .enumerate()
            .find(|(_, rect)| rect.contains(point))
            .map(|(i, _)| i)
    }

    pub fn contains(&self, point: Point) -> bool {
        self.panel.contains(point)
    }
}

/// Soft ambient contact shadow under a rounded panel (no Solar Focus Glow).
pub fn draw_panel_ambient_shadow(canvas: &mut Canvas, panel: Rect, radius: u32) {
    let shadow = Rect::new(
        panel.x.saturating_add(2),
        panel.y.saturating_add(4),
        panel.w,
        panel.h,
    );
    // Quiet multi-pass shadow — bounded, no allocation.
    let layers = [
        (6i32, Color::rgba(0, 0, 0, 18)),
        (3, Color::rgba(0, 0, 0, 28)),
        (1, Color::rgba(0, 0, 0, 40)),
    ];
    for (expand, color) in layers {
        let r = Rect::new(
            shadow.x - expand,
            shadow.y - expand / 2,
            shadow.w.saturating_add((expand * 2) as u32),
            shadow.h.saturating_add(expand as u32),
        );
        let rad = radius.saturating_add(expand as u32);
        canvas.blend_rounded_rect(r, rad, color);
    }
}

/// Compact `+N` overflow badge for icon stacks.
pub struct BoundedOverflowBadge<'a> {
    pub rect: Rect,
    pub label: &'a str,
}

impl<'a> BoundedOverflowBadge<'a> {
    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.label.is_empty() || self.rect.w == 0 || self.rect.h == 0 {
            return;
        }
        canvas.fill_rounded_rect(self.rect, 6, theme.panel_alt);
        canvas.stroke_rounded_rect(self.rect, 6, 1, theme.border);
        canvas.draw_text_centered(self.rect, self.label, theme.text_dim);
    }
}

/// Horizontal stack of up to three application icons plus optional `+N`.
pub struct AppIconStack<'a> {
    pub rect: Rect,
    pub icons: &'a [Option<&'a TgaImage>],
    pub overflow: u32,
    pub generic: Option<&'a TgaImage>,
}

impl<'a> AppIconStack<'a> {
    pub const ICON: u32 = 22;
    pub const GAP: i32 = 6;

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let mut x = self.rect.x;
        let y = self.rect.y + (self.rect.h as i32 - Self::ICON as i32) / 2;
        let mut drawn = 0usize;
        for slot in self.icons.iter().take(WORKSPACE_ICON_SLOTS) {
            let icon_rect = Rect::new(x, y, Self::ICON, Self::ICON);
            match *slot {
                Some(img) => {
                    canvas.draw_tga_icon(img, icon_rect);
                    drawn += 1;
                }
                None => {
                    if let Some(generic) = self.generic {
                        canvas.draw_tga_icon(generic, icon_rect);
                    } else {
                        // Neutral fallback square when no generic asset is supplied.
                        canvas.fill_rounded_rect(icon_rect, 4, theme.panel_alt);
                        canvas.stroke_rounded_rect(icon_rect, 4, 1, theme.border);
                    }
                    drawn += 1;
                }
            }
            x += Self::ICON as i32 + Self::GAP;
        }

        if self.overflow > 0 && drawn > 0 {
            let mut buf = [0u8; 8];
            let label = format_overflow(self.overflow, &mut buf);
            let badge_w = 22u32.max((label.len() as u32).saturating_mul(6).saturating_add(8));
            let badge = Rect::new(
                x,
                y + (Self::ICON as i32 - 16) / 2,
                badge_w.min(self.rect.right().saturating_sub(x).max(0) as u32),
                16,
            );
            BoundedOverflowBadge { rect: badge, label }.draw(canvas, theme);
        }
    }
}

fn format_overflow(n: u32, buf: &mut [u8; 8]) -> &str {
    buf[0] = b'+';
    let v = n.min(99);
    if v >= 10 {
        buf[1] = b'0' + (v / 10) as u8;
        buf[2] = b'0' + (v % 10) as u8;
        core::str::from_utf8(&buf[..3]).unwrap_or("+N")
    } else {
        buf[1] = b'0' + v as u8;
        core::str::from_utf8(&buf[..2]).unwrap_or("+N")
    }
}

/// Single workspace selection card.
pub struct WorkspaceCard<'a> {
    pub rect: Rect,
    pub view: WorkspaceCardView<'a>,
    pub generic_icon: Option<&'a TgaImage>,
}

impl<'a> WorkspaceCard<'a> {
    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let materials = MaterialPalette::new(theme);
        let radius = WorkspaceSwitcherLayout::CARD_RADIUS;

        // Card glass — denser than the outer overlay for readable content.
        let mut mat = materials.card_glass.with_radius(radius);
        if self.view.state.hovered && !self.view.state.active {
            mat = Material::tinted(theme.panel_alt, 245)
                .with_noise(2)
                .with_border(theme.border)
                .with_radius(radius);
        }
        canvas.fill_material(self.rect, mat);

        // Active workspace: restrained orange rim (not a full orange fill).
        let border = if self.view.state.active {
            theme.accent
        } else if self.view.state.focused {
            theme.accent.mix(theme.border, 96)
        } else if self.view.state.hovered {
            theme.border.lighten(24)
        } else {
            theme.border
        };
        let thickness = if self.view.state.active || self.view.state.focused {
            2
        } else {
            1
        };
        canvas.stroke_rounded_rect(self.rect, radius, thickness, border);

        // Keyboard focus ring — distinguishable from active rim.
        if self.view.state.focused && !self.view.state.active {
            let ring = self.rect.inset(2);
            canvas.stroke_rounded_rect(ring, radius.saturating_sub(2), 1, theme.accent.darken(40));
        }

        let pad = 10i32;
        let title_color = if self.view.state.active {
            theme.text
        } else {
            theme.text_dim
        };

        // Number + title row.
        let mut num_buf = [0u8; 4];
        let num = format_workspace_number(self.view.id, &mut num_buf);
        canvas.draw_text(self.rect.x + pad, self.rect.y + 8, num, title_color);
        canvas.draw_text(
            self.rect.x + pad + 14,
            self.rect.y + 8,
            self.view.title,
            title_color,
        );

        if self.view.state.active {
            let badge_w = 40u32;
            let badge = Rect::new(
                self.rect.right() - badge_w as i32 - pad,
                self.rect.y + 6,
                badge_w,
                14,
            );
            canvas.fill_rounded_rect(badge, 5, theme.panel_alt);
            canvas.draw_text_centered(badge, "Active", theme.accent);
        }

        // Reserved preview band (empty for MVP — no live thumbnails).
        let preview = Rect::new(
            self.rect.x + pad,
            self.rect.y + 28,
            self.rect.w.saturating_sub((pad * 2) as u32),
            28,
        );
        canvas.fill_rounded_rect(preview, 6, theme.panel.darken(8));
        canvas.stroke_rounded_rect(preview, 6, 1, theme.border.darken(10));

        // Footer: Empty / count + icon stack.
        let footer = Rect::new(
            self.rect.x + pad,
            self.rect.bottom() - 30,
            self.rect.w.saturating_sub((pad * 2) as u32),
            24,
        );
        if self.view.empty {
            canvas.draw_text_centered(footer, "Empty", theme.text_muted);
        } else {
            let mut count_buf = [0u8; 12];
            let count_label = format_window_count(self.view.window_count, &mut count_buf);
            canvas.draw_text(footer.x, footer.y + 4, count_label, theme.text_muted);

            let icons_rect = Rect::new(
                footer.x + 36,
                footer.y,
                footer.w.saturating_sub(36),
                footer.h,
            );
            AppIconStack {
                rect: icons_rect,
                icons: self.view.icons,
                overflow: self.view.overflow,
                generic: self.generic_icon,
            }
            .draw(canvas, theme);
        }
    }
}

fn format_workspace_number(id: u8, buf: &mut [u8; 4]) -> &str {
    if id == 0 || id > 9 {
        buf[0] = b'?';
        return core::str::from_utf8(&buf[..1]).unwrap_or("?");
    }
    buf[0] = b'0' + id;
    core::str::from_utf8(&buf[..1]).unwrap_or("?")
}

fn format_window_count(count: u32, buf: &mut [u8; 12]) -> &str {
    // "N win" compact label.
    let n = count.min(99);
    let mut i = 0usize;
    if n >= 10 {
        buf[i] = b'0' + (n / 10) as u8;
        i += 1;
        buf[i] = b'0' + (n % 10) as u8;
        i += 1;
    } else {
        buf[i] = b'0' + n as u8;
        i += 1;
    }
    let suffix = b" win";
    for &b in suffix {
        if i < buf.len() {
            buf[i] = b;
            i += 1;
        }
    }
    core::str::from_utf8(&buf[..i]).unwrap_or("win")
}

/// Full Workspace Switcher panel: glass surface + four cards in one row.
pub struct WorkspaceSwitcherPanel<'a> {
    pub layout: WorkspaceSwitcherLayout,
    pub cards: &'a [WorkspaceCardView<'a>; WORKSPACE_CARD_COUNT],
    pub generic_icon: Option<&'a TgaImage>,
    pub status: Option<&'a str>,
}

impl<'a> WorkspaceSwitcherPanel<'a> {
    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        draw_panel_ambient_shadow(
            canvas,
            self.layout.panel,
            WorkspaceSwitcherLayout::PANEL_RADIUS,
        );
        canvas.fill_material(
            self.layout.panel,
            Material::for_role(SurfaceRole::SystemOverlay, theme)
                .with_radius(WorkspaceSwitcherLayout::PANEL_RADIUS),
        );

        for (i, view) in self.cards.iter().enumerate() {
            WorkspaceCard {
                rect: self.layout.cards[i],
                view: *view,
                generic_icon: self.generic_icon,
            }
            .draw(canvas, theme);
        }

        if let Some(status) = self.status {
            if !status.is_empty() {
                let status_rect = Rect::new(
                    self.layout.panel.x + 12,
                    self.layout.panel.bottom() - 16,
                    self.layout.panel.w.saturating_sub(24),
                    12,
                );
                canvas.draw_text_centered(status_rect, status, theme.danger_text);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_fits_four_cards_on_thinkpad_and_vmware() {
        // ThinkPad-like and higher VMware resolutions.
        for (w, h) in [(1366u32, 768u32), (1920, 1080), (1280, 800), (1024, 768)] {
            let dock_top = h as i32 - 8 - 44;
            let layout = WorkspaceSwitcherLayout::compute(w, h, 50, dock_top);
            assert!(layout.panel.w <= w);
            assert!(layout.panel.right() <= w as i32);
            assert!(layout.panel.y >= 4);
            assert!(layout.panel.bottom() <= dock_top);
            assert_eq!(layout.cards.len(), 4);
            for card in &layout.cards {
                assert!(card.w >= 80);
                assert!(layout.panel.contains(Point::new(card.x, card.y)));
            }
            // No horizontal scroll: last card inside panel.
            assert!(layout.cards[3].right() <= layout.panel.right());
        }
    }

    #[test]
    fn layout_stays_above_dock_and_below_top() {
        let layout = WorkspaceSwitcherLayout::compute(1366, 768, 48, 716);
        assert!(layout.panel.y >= 48);
        assert!(layout.panel.bottom() + WorkspaceSwitcherLayout::DOCK_GAP <= 716 + 2);
    }

    #[test]
    fn overflow_label_is_bounded() {
        let mut buf = [0u8; 8];
        assert_eq!(format_overflow(1, &mut buf), "+1");
        assert_eq!(format_overflow(12, &mut buf), "+12");
        assert_eq!(format_overflow(150, &mut buf), "+99");
    }

    #[test]
    fn card_hit_test_maps_indices() {
        let layout = WorkspaceSwitcherLayout::compute(1366, 768, 48, 716);
        for i in 0..4 {
            let c = layout.cards[i];
            let p = Point::new(c.x + 4, c.y + 4);
            assert_eq!(layout.card_index_at(p), Some(i));
        }
        assert_eq!(layout.card_index_at(Point::new(0, 0)), None);
    }
}
