use sunlight_ui::{Canvas, Color, Rect};
use sun_font::{draw_text, FontRole, TextStyle};

pub struct PrivacyPage {
}

impl PrivacyPage {
    pub fn new() -> Self {
        Self {}
    }

    pub fn draw(&mut self, canvas: &mut Canvas, rect: Rect) {
        let style = TextStyle::new(FontRole::UiMedium, Color::rgb(255, 255, 255));
        draw_text(canvas, "Privacy", rect.x + 20, rect.y + 20, &style);
        
        let style_small = TextStyle::new(FontRole::UiSmall, Color::rgb(180, 180, 180));
        draw_text(
            canvas,
            "Wise Owl stores bounded action receipts.",
            rect.x + 20,
            rect.y + 60,
            &style_small,
        );
        draw_text(
            canvas,
            "Wise Owl does not store confirmation secrets.",
            rect.x + 20,
            rect.y + 90,
            &style_small,
        );
        draw_text(
            canvas,
            "Wise Owl does not use action receipts as future authorization.",
            rect.x + 20,
            rect.y + 120,
            &style_small,
        );
    }
}
