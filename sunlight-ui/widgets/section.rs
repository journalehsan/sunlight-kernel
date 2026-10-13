//! Section — a semantic, filled/hit-testable rectangular region.
//!
//! Unlike a structural [`Panel`](super::panel::Panel), a section owns its
//! rectangle and reports whether a point falls inside it. Toolbars and status
//! bars compose sections for buttons, dropdowns, sliders, and labels so app
//! code can hit-test and highlight them without re-implementing geometry.

use crate::font::VecText;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionState {
    Normal,
    Hovered,
    Pressed,
    Active,
    Disabled,
}

pub struct Section<'a> {
    pub rect: Rect,
    pub label: &'a str,
    pub state: SectionState,
    /// Primary sections use the accent fill; secondary use the panel fill.
    pub primary: bool,
    font: Option<&'a dyn VecText>,
}

impl<'a> Section<'a> {
    pub const fn new(rect: Rect, label: &'a str) -> Self {
        Self {
            rect,
            label,
            state: SectionState::Normal,
            primary: false,
            font: None,
        }
    }

    pub const fn primary(rect: Rect, label: &'a str) -> Self {
        Self {
            rect,
            label,
            state: SectionState::Normal,
            primary: true,
            font: None,
        }
    }

    pub fn with_state(mut self, state: SectionState) -> Self {
        self.state = state;
        self
    }

    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    pub fn contains(&self, point: Point) -> bool {
        self.rect.contains(point)
    }

    /// Fill color for the current state, resolved through the theme.
    pub fn fill_color(&self, theme: &Theme) -> Color {
        match (self.primary, self.state) {
            (_, SectionState::Disabled) => theme.panel_alt,
            (_, SectionState::Active) => theme.accent,
            (true, SectionState::Normal) => theme.accent,
            (true, SectionState::Hovered) => theme.accent_hover,
            (true, SectionState::Pressed) => theme.accent.darken(40),
            (false, SectionState::Normal) => theme.panel,
            (false, SectionState::Hovered) => theme.panel_alt,
            (false, SectionState::Pressed) => theme.border,
        }
    }

    fn text_color(&self, theme: &Theme) -> Color {
        match self.state {
            SectionState::Disabled => theme.text_muted,
            SectionState::Active => theme.text_on_accent,
            _ if self.primary => theme.text_on_accent,
            _ => theme.text,
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.rect.w == 0 || self.rect.h == 0 {
            return;
        }
        canvas.fill_rect(self.rect, self.fill_color(theme));
        if self.state == SectionState::Normal || self.state == SectionState::Disabled {
            canvas.draw_rect(self.rect, theme.border);
        }
        let color = self.text_color(theme);
        if let Some(font) = self.font {
            font.draw_vcenter(
                canvas,
                self.label,
                self.rect.x + 8,
                self.rect.y,
                self.rect.h,
                color,
            );
        } else {
            let ty = self.rect.y + (self.rect.h as i32 - 10) / 2;
            canvas.draw_text(self.rect.x + 8, ty, self.label, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_reports_containment() {
        let section = Section::new(Rect::new(10, 20, 40, 16), "Prev");
        assert!(section.contains(Point::new(10, 20)));
        assert!(section.contains(Point::new(49, 35)));
        assert!(!section.contains(Point::new(50, 20)));
        assert!(!section.contains(Point::new(10, 36)));
    }

    #[test]
    fn state_selects_distinct_fills() {
        let theme = Theme::sunlight_dark();
        let make = |state| Section::new(Rect::new(0, 0, 10, 10), "").with_state(state);
        let normal = make(SectionState::Normal);
        let hovered = make(SectionState::Hovered);
        let pressed = make(SectionState::Pressed);
        assert_ne!(normal.fill_color(&theme), hovered.fill_color(&theme));
        assert_ne!(hovered.fill_color(&theme), pressed.fill_color(&theme));
    }
}
