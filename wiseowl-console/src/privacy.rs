use sunlight_ui::{
    widgets::{Label, Panel},
    Canvas, Rect, Theme,
};

use crate::ui::{FONT_UI_MEDIUM, FONT_UI_SMALL};

pub struct PrivacyPage {}

impl PrivacyPage {
    pub fn new() -> Self {
        Self {}
    }

    pub fn draw(&mut self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "Privacy").with_font(&FONT_UI_MEDIUM);
        panel.draw(canvas, theme);
        let content = panel.content_rect().inset(18);
        let statements = [
            "Wise Owl stores bounded action receipts.",
            "Wise Owl does not store confirmation secrets.",
            "Wise Owl does not use action receipts as future authorization.",
        ];

        for (index, statement) in statements.iter().enumerate() {
            Label::new(
                Rect::new(content.x, content.y + index as i32 * 30, content.w, 22),
                statement,
            )
            .dim()
            .with_font(&FONT_UI_SMALL)
            .draw(canvas, theme);
        }
    }
}
