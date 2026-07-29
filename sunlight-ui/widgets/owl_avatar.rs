//! Animated Wise Owl avatar widget.

use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwlAvatarState {
    Idle,
    Listening,
    Thinking,
    Clarification,
    Confirmation,
    Acting,
    Observing,
    Success,
    Warning,
    Offline,
}

pub struct OwlAvatar {
    pub rect: Rect,
    pub state: OwlAvatarState,
    pub tick: u64,
}

impl OwlAvatar {
    pub const fn new(rect: Rect) -> Self {
        Self {
            rect,
            state: OwlAvatarState::Idle,
            tick: 0,
        }
    }

    pub const fn with_state(mut self, state: OwlAvatarState) -> Self {
        self.state = state;
        self
    }

    pub fn advance(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let center_x = self.rect.x + self.rect.w as i32 / 2;
        let center_y = self.rect.y + self.rect.h as i32 / 2;
        let body = Rect::new(center_x - 20, center_y - 20, 40, 40);
        canvas.fill_rounded_rect(body, 12, self.body_color(theme));
        canvas.stroke_rounded_rect(body, 12, 1, theme.border);

        let eye_color = if self.state == OwlAvatarState::Offline {
            theme.icon_disabled
        } else {
            theme.text
        };
        let (left_eye_y, right_eye_y) = self.eye_positions(center_y);
        canvas.fill_rounded_rect(Rect::new(center_x - 12, left_eye_y, 8, 8), 4, eye_color);
        canvas.fill_rounded_rect(Rect::new(center_x + 4, right_eye_y, 8, 8), 4, eye_color);
        canvas.fill_rounded_rect(Rect::new(center_x - 4, center_y + 3, 8, 6), 3, theme.accent);
    }

    fn body_color(&self, theme: &Theme) -> Color {
        match self.state {
            OwlAvatarState::Offline => theme.icon_disabled,
            OwlAvatarState::Warning => theme.warn,
            OwlAvatarState::Success => theme.ok,
            OwlAvatarState::Confirmation => theme.panel.mix(theme.danger, 42),
            OwlAvatarState::Clarification => theme.panel.mix(theme.warn, 38),
            OwlAvatarState::Acting => theme.panel.mix(theme.accent, 42),
            _ => theme.panel_alt.lighten(18),
        }
    }

    fn eye_positions(&self, center_y: i32) -> (i32, i32) {
        let mut left_eye_y = center_y - 10;
        let mut right_eye_y = center_y - 10;

        match self.state {
            OwlAvatarState::Idle if self.tick % 100 < 5 => {
                left_eye_y = center_y - 5;
                right_eye_y = center_y - 5;
            }
            OwlAvatarState::Listening => {
                left_eye_y -= 2;
                right_eye_y += 2;
            }
            OwlAvatarState::Thinking => {
                let offset = (self.tick % 20) as i32 - 10;
                left_eye_y += offset / 5;
                right_eye_y += offset / 5;
            }
            _ => {}
        }

        (left_eye_y, right_eye_y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatar_draws_every_state() {
        let mut pixels = [0u32; 64 * 64];
        let mut canvas = Canvas::new(&mut pixels, 64, 64, 64);
        let theme = Theme::sunlight_dark();
        let states = [
            OwlAvatarState::Idle,
            OwlAvatarState::Listening,
            OwlAvatarState::Thinking,
            OwlAvatarState::Clarification,
            OwlAvatarState::Confirmation,
            OwlAvatarState::Acting,
            OwlAvatarState::Observing,
            OwlAvatarState::Success,
            OwlAvatarState::Warning,
            OwlAvatarState::Offline,
        ];

        for state in states {
            OwlAvatar::new(Rect::new(8, 8, 48, 48))
                .with_state(state)
                .draw(&mut canvas, &theme);
        }
    }
}
