//! Button widget.

use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hovered,
    Pressed,
    Disabled,
}

pub struct Button<'a> {
    pub rect: Rect,
    pub label: &'a str,
    pub state: ButtonState,
    /// If true, render with accent fill (primary action).
    /// If false, render as ghost/secondary button.
    pub primary: bool,
}

impl<'a> Button<'a> {
    pub fn new(rect: Rect, label: &'a str) -> Self {
        Self {
            rect,
            label,
            state: ButtonState::Normal,
            primary: true,
        }
    }

    pub fn secondary(rect: Rect, label: &'a str) -> Self {
        Self {
            rect,
            label,
            state: ButtonState::Normal,
            primary: false,
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let bg = match (self.primary, self.state) {
            (_, ButtonState::Disabled) => theme.panel_alt,
            (true, ButtonState::Normal) => theme.accent,
            (true, ButtonState::Hovered) => theme.accent_hover,
            (true, ButtonState::Pressed) => theme.accent.darken(40),
            (false, ButtonState::Normal) => theme.panel,
            (false, ButtonState::Hovered) => theme.panel_alt,
            (false, ButtonState::Pressed) => theme.border,
        };

        let text_color = match self.state {
            ButtonState::Disabled => theme.text_dim,
            _ if self.primary => theme.bg,
            _ => theme.text,
        };

        canvas.fill_rect(self.rect, bg);

        // Border for secondary buttons
        if !self.primary {
            canvas.draw_rect(self.rect, theme.border);
        }

        // Accent underline for active/hovered primary
        if self.primary && self.state == ButtonState::Hovered {
            canvas.hbar(
                self.rect.x,
                self.rect.bottom() - 2,
                self.rect.w,
                2,
                theme.accent_hover.lighten(60),
            );
        }

        canvas.draw_text_centered(self.rect, self.label, text_color);
    }

    pub fn hit_test(&self, x: i32, y: i32) -> bool {
        self.state != ButtonState::Disabled && self.rect.contains(crate::geom::Point::new(x, y))
    }
}
