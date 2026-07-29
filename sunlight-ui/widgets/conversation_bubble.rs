//! Conversation bubble widget for assistant, user, and system messages.

use crate::font::VecText;
use crate::geom::Rect;
use crate::paint::Canvas;
use crate::theme::{Color, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationBubbleKind {
    User,
    Assistant,
    System,
    Progress,
    Error,
    Clarification,
    Confirmation,
    Option,
}

pub struct ConversationBubble<'a> {
    pub rect: Rect,
    pub text: &'a str,
    pub detail: Option<&'a str>,
    pub kind: ConversationBubbleKind,
    font: Option<&'a dyn VecText>,
}

impl<'a> ConversationBubble<'a> {
    pub const HEIGHT: u32 = 30;
    pub const DETAIL_HEIGHT: u32 = 48;

    pub const fn new(rect: Rect, text: &'a str, kind: ConversationBubbleKind) -> Self {
        Self {
            rect,
            text,
            detail: None,
            kind,
            font: None,
        }
    }

    pub const fn with_detail(mut self, detail: &'a str) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn with_font(mut self, font: &'a dyn VecText) -> Self {
        self.font = Some(font);
        self
    }

    pub const fn preferred_height(&self) -> u32 {
        if self.detail.is_some() {
            Self::DETAIL_HEIGHT
        } else {
            Self::HEIGHT
        }
    }

    pub fn draw(&self, canvas: &mut Canvas, theme: &Theme) {
        let (background, border, text_color) = self.colors(theme);
        canvas.fill_rounded_rect(self.rect, 7, background);
        canvas.stroke_rounded_rect(self.rect, 7, 1, border);

        let text_rect = self.rect.inset(8);
        if let Some(detail) = self.detail {
            self.draw_text(canvas, self.text, text_rect.x, text_rect.y + 3, text_color);
            self.draw_text(
                canvas,
                detail,
                text_rect.x,
                text_rect.y + 22,
                theme.text_dim,
            );
        } else {
            self.draw_text_vcenter(canvas, self.text, text_rect, text_color);
        }
    }

    fn colors(&self, theme: &Theme) -> (Color, Color, Color) {
        match self.kind {
            ConversationBubbleKind::User => {
                (theme.chrome.selection, theme.accent.darken(100), theme.text)
            }
            ConversationBubbleKind::Assistant => (theme.panel_alt, theme.border, theme.text),
            ConversationBubbleKind::System => (theme.panel, theme.border, theme.text_dim),
            ConversationBubbleKind::Progress => (
                theme.panel.mix(theme.warn, 24),
                theme.warn.darken(100),
                theme.text,
            ),
            ConversationBubbleKind::Error => (
                theme.panel.mix(theme.danger, 32),
                theme.danger.darken(92),
                theme.danger_text,
            ),
            ConversationBubbleKind::Clarification => (
                theme.panel.mix(theme.warn, 20),
                theme.warn.darken(110),
                theme.text,
            ),
            ConversationBubbleKind::Confirmation => (
                theme.panel.mix(theme.danger, 18),
                theme.danger.darken(120),
                theme.text,
            ),
            ConversationBubbleKind::Option => (theme.panel, theme.border, theme.text_muted),
        }
    }

    fn draw_text(&self, canvas: &mut Canvas, text: &str, x: i32, y: i32, color: Color) {
        if let Some(font) = self.font {
            font.draw(canvas, text, x, y, color);
        } else {
            canvas.draw_text(x, y, text, color);
        }
    }

    fn draw_text_vcenter(&self, canvas: &mut Canvas, text: &str, rect: Rect, color: Color) {
        if let Some(font) = self.font {
            font.draw_vcenter(canvas, text, rect.x, rect.y, rect.h, color);
        } else {
            canvas.draw_text_centered(rect, text, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bubbles_draw_with_and_without_detail() {
        let mut pixels = [0u32; 96 * 64];
        let mut canvas = Canvas::new(&mut pixels, 96, 96, 64);
        let theme = Theme::sunlight_dark();

        ConversationBubble::new(
            Rect::new(4, 4, 80, ConversationBubble::HEIGHT),
            "Ready",
            ConversationBubbleKind::Assistant,
        )
        .draw(&mut canvas, &theme);
        ConversationBubble::new(
            Rect::new(4, 38, 80, ConversationBubble::DETAIL_HEIGHT),
            "Confirmation required",
            ConversationBubbleKind::Confirmation,
        )
        .with_detail("Review the requested action.")
        .draw(&mut canvas, &theme);
    }
}
