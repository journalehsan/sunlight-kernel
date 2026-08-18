//! Compact glyph-only button built on the shared UI-symbol and theme systems.

use crate::font::UiSymbol;
use crate::geom::{Point, Rect};
use crate::paint::Canvas;
use crate::theme::Theme;

use super::button::ButtonState;

pub struct IconButton {
    pub rect: Rect,
    pub symbol: UiSymbol,
    pub state: ButtonState,
    pub primary: bool,
    pub focused: bool,
}

impl IconButton {
    pub const fn new(rect: Rect, symbol: UiSymbol) -> Self {
        Self {
            rect,
            symbol,
            state: ButtonState::Normal,
            primary: false,
            focused: false,
        }
    }

    pub const fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }

    pub const fn with_state(mut self, state: ButtonState) -> Self {
        self.state = state;
        self
    }

    pub const fn with_focus(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        if self.rect.w == 0 || self.rect.h == 0 {
            return;
        }
        let bg = match (self.primary, self.state) {
            (_, ButtonState::Disabled) => theme.panel_alt,
            (true, ButtonState::Normal) => theme.accent,
            (true, ButtonState::Hovered) => theme.accent_hover,
            (true, ButtonState::Pressed) => theme.accent.darken(42),
            (false, ButtonState::Normal) => theme.panel,
            (false, ButtonState::Hovered) => theme.panel_alt,
            (false, ButtonState::Pressed) => theme.chrome.control_pressed,
        };
        let glyph = match self.state {
            ButtonState::Disabled => theme.icon_disabled,
            _ if self.primary => theme.text_on_accent,
            _ => theme.icon_foreground,
        };
        let radius = (self.rect.w.min(self.rect.h) / 2).min(12);
        let border = if self.focused {
            theme.accent_hover
        } else if self.primary {
            theme.accent
        } else {
            theme.border
        };
        canvas.fill_rounded_rect_with_border(self.rect, radius, bg, border, 1);
        canvas.draw_ui_symbol_centered(self.rect, self.symbol, glyph);
    }

    pub fn hit_test(&self, x: i32, y: i32) -> bool {
        self.state != ButtonState::Disabled && self.rect.contains(Point::new(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_icon_button_does_not_hit() {
        let button = IconButton::new(Rect::new(4, 5, 36, 36), UiSymbol::Play)
            .with_state(ButtonState::Disabled);
        assert!(!button.hit_test(10, 10));
    }

    #[test]
    fn enabled_icon_button_uses_full_hit_area() {
        let button = IconButton::new(Rect::new(4, 5, 36, 36), UiSymbol::Play);
        assert!(button.hit_test(4, 5));
        assert!(button.hit_test(39, 40));
        assert!(!button.hit_test(40, 41));
    }
}
