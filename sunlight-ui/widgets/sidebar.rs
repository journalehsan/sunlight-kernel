//! Compact reusable primitives for system-widget surfaces.
//!
//! These widgets deliberately receive only view data and geometry.  Shells and
//! applications own service access, event routing, and state persistence.

use crate::{
    geom::{Point, Rect},
    material::MaterialPalette,
    paint::Canvas,
    theme::Theme,
};

/// A compact segmented control for a small, fixed set of labels.
pub struct SegmentedTabs<'a> {
    pub rect: Rect,
    pub labels: &'a [&'a str],
    pub selected: usize,
    pub focused: bool,
    pub hovered: Option<usize>,
}

impl<'a> SegmentedTabs<'a> {
    pub const fn new(rect: Rect, labels: &'a [&'a str], selected: usize) -> Self {
        Self {
            rect,
            labels,
            selected,
            focused: false,
            hovered: None,
        }
    }

    pub fn item_rect(&self, index: usize) -> Rect {
        let count = self.labels.len().max(1) as u32;
        let base_w = self.rect.w / count;
        let x = self
            .rect
            .x
            .saturating_add((index as u32).saturating_mul(base_w) as i32);
        let width = if index + 1 == self.labels.len() {
            self.rect.right().saturating_sub(x).max(0) as u32
        } else {
            base_w
        };
        Rect::new(x, self.rect.y, width, self.rect.h)
    }

    pub fn hit_test(&self, point: Point) -> Option<usize> {
        if !self.rect.contains(point) {
            return None;
        }
        self.labels
            .iter()
            .enumerate()
            .find(|(index, _)| self.item_rect(*index).contains(point))
            .map(|(index, _)| index)
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let materials = MaterialPalette::new(theme);
        canvas.fill_material(
            self.rect,
            materials.tinted_content.with_radius(7).without_border(),
        );
        canvas.stroke_rounded_rect(
            self.rect,
            7,
            if self.focused { 2 } else { 1 },
            if self.focused {
                theme.accent
            } else {
                theme.border
            },
        );
        for (index, label) in self.labels.iter().enumerate() {
            let rect = self.item_rect(index).inset(2);
            let selected = index == self.selected;
            let hovered = self.hovered == Some(index);
            if selected {
                canvas.fill_rounded_rect(rect, 5, theme.accent.darken(35));
            } else if hovered {
                canvas.fill_rounded_rect(rect, 5, theme.panel_alt.lighten(10));
            }
            canvas.draw_text_centered(
                rect,
                label,
                if selected { theme.text } else { theme.text_dim },
            );
        }
    }
}

/// Dense card surface for bounded system-widget content.
pub struct WidgetCard<'a> {
    pub rect: Rect,
    pub title: &'a str,
    pub badge: Option<&'a str>,
    pub focused: bool,
}

impl<'a> WidgetCard<'a> {
    pub const fn new(rect: Rect, title: &'a str) -> Self {
        Self {
            rect,
            title,
            badge: None,
            focused: false,
        }
    }

    pub const fn with_badge(mut self, badge: &'a str) -> Self {
        self.badge = Some(badge);
        self
    }

    pub const fn with_focus(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn content_rect(&self) -> Rect {
        Rect::new(
            self.rect.x.saturating_add(12),
            self.rect.y.saturating_add(30),
            self.rect.w.saturating_sub(24),
            self.rect.h.saturating_sub(40),
        )
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        self.draw_chrome(canvas, theme);
        canvas.draw_text(self.rect.x + 12, self.rect.y + 10, self.title, theme.text);
        if let Some(badge) = self.badge {
            let width = (badge.len() as u32)
                .saturating_mul(6)
                .saturating_add(12)
                .min(self.rect.w.saturating_sub(24));
            let badge_rect = Rect::new(
                self.rect.right() - width as i32 - 10,
                self.rect.y + 7,
                width,
                16,
            );
            canvas.fill_rounded_rect(badge_rect, 5, theme.panel_alt);
            canvas.draw_text_centered(badge_rect, badge, theme.text_dim);
        }
    }

    /// Draws only the shared card surface. Compositions that own a richer text
    /// system can render titles and badges separately without duplicating the
    /// card material.
    pub fn draw_chrome(&self, canvas: &mut Canvas, theme: &Theme) {
        let materials = MaterialPalette::new(theme);
        canvas.fill_material(self.rect, materials.card_glass);
        if self.focused {
            canvas.stroke_rounded_rect(self.rect, 8, 2, theme.accent);
        }
    }
}

/// A zero-allocation utilization bar expressed in basis points.
pub struct MetricBar {
    pub rect: Rect,
    pub value_bp: u16,
}

impl MetricBar {
    pub const fn new(rect: Rect, value_bp: u16) -> Self {
        Self { rect, value_bp }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rounded_rect(self.rect, 4, theme.bg);
        canvas.stroke_rounded_rect(self.rect, 4, 1, theme.border);
        let inner = self.rect.inset(2);
        let fill_w = inner.w.saturating_mul(self.value_bp.min(10_000) as u32) / 10_000;
        if fill_w > 0 {
            canvas.fill_rounded_rect(
                Rect::new(inner.x, inner.y, fill_w, inner.h),
                2,
                theme.accent,
            );
        }
    }
}

/// Compact two-state (or small multi-state) unit selector.
pub struct UnitToggle<'a> {
    pub rect: Rect,
    pub labels: &'a [&'a str],
    pub selected: usize,
    pub focused: bool,
}

impl<'a> UnitToggle<'a> {
    pub const fn new(rect: Rect, labels: &'a [&'a str], selected: usize) -> Self {
        Self {
            rect,
            labels,
            selected,
            focused: false,
        }
    }

    pub const fn with_focus(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn item_rect(&self, index: usize) -> Rect {
        let count = self.labels.len().max(1) as u32;
        let width = self.rect.w / count;
        let x = self
            .rect
            .x
            .saturating_add((index as u32).saturating_mul(width) as i32);
        let item_w = if index + 1 == self.labels.len() {
            self.rect.right().saturating_sub(x).max(0) as u32
        } else {
            width
        };
        Rect::new(x, self.rect.y, item_w, self.rect.h)
    }

    pub fn hit_test(&self, point: Point) -> Option<usize> {
        self.labels
            .iter()
            .enumerate()
            .find(|(index, _)| self.item_rect(*index).contains(point))
            .map(|(index, _)| index)
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        self.draw_chrome(canvas, theme);
        for (index, label) in self.labels.iter().enumerate() {
            let rect = self.item_rect(index).inset(2);
            canvas.draw_text_centered(
                rect,
                label,
                if index == self.selected {
                    theme.text
                } else {
                    theme.text_dim
                },
            );
        }
    }

    /// Draws only the selector surface and selected state.
    pub fn draw_chrome(&self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rounded_rect(self.rect, 6, theme.panel_alt);
        canvas.stroke_rounded_rect(
            self.rect,
            6,
            if self.focused { 2 } else { 1 },
            if self.focused {
                theme.accent
            } else {
                theme.border
            },
        );
        for (index, _) in self.labels.iter().enumerate() {
            let rect = self.item_rect(index).inset(2);
            if index == self.selected {
                canvas.fill_rounded_rect(rect, 4, theme.accent.darken(30));
            }
        }
    }
}

/// One bounded article row in a compact feed.
pub struct ArticleListItem<'a> {
    pub rect: Rect,
    pub title: &'a str,
    pub summary: &'a str,
    pub meta: &'a str,
    pub hovered: bool,
    pub focused: bool,
}

impl<'a> ArticleListItem<'a> {
    pub const fn new(rect: Rect, title: &'a str, summary: &'a str, meta: &'a str) -> Self {
        Self {
            rect,
            title,
            summary,
            meta,
            hovered: false,
            focused: false,
        }
    }

    pub const fn with_interaction(mut self, hovered: bool, focused: bool) -> Self {
        self.hovered = hovered;
        self.focused = focused;
        self
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        self.draw_chrome(canvas, theme);
        canvas.draw_text(self.rect.x + 8, self.rect.y + 6, self.title, theme.text);
        canvas.draw_text(
            self.rect.x + 8,
            self.rect.y + 20,
            self.summary,
            theme.text_dim,
        );
        canvas.draw_text(
            self.rect.x + 8,
            self.rect.y + 34,
            self.meta,
            theme.text_muted,
        );
    }

    /// Draws only the reusable hover and focus affordance.
    pub fn draw_chrome(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.hovered || self.focused {
            canvas.fill_rounded_rect(self.rect, 6, theme.panel_alt.lighten(8));
        }
        if self.focused {
            canvas.stroke_rounded_rect(self.rect, 6, 2, theme.accent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SegmentedTabs;
    use crate::geom::{Point, Rect};

    #[test]
    fn segmented_tabs_hit_test_uses_physical_left_to_right_geometry() {
        let tabs = SegmentedTabs::new(Rect::new(10, 10, 120, 24), &["A", "B"], 0);
        assert_eq!(tabs.hit_test(Point::new(20, 20)), Some(0));
        assert_eq!(tabs.hit_test(Point::new(100, 20)), Some(1));
    }
}
