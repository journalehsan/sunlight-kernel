use sunlight_ui::{Canvas, Color, Rect};
use sun_font::{draw_text, FontRole, TextStyle};

pub struct ActivityPage {
}

impl ActivityPage {
    pub fn new() -> Self {
        Self {}
    }

    pub fn draw(&mut self, canvas: &mut Canvas, rect: Rect) {
        let style = TextStyle::new(FontRole::UiMedium, Color::rgb(255, 255, 255));
        draw_text(canvas, "Activity", rect.x + 20, rect.y + 20, &style);
        
        let style_small = TextStyle::new(FontRole::UiSmall, Color::rgb(180, 180, 180));
        draw_text(
            canvas,
            "Action receipts will appear here.",
            rect.x + 20,
            rect.y + 60,
            &style_small,
        );
    }
}
