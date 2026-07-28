use sunlight_ui::{Canvas, Color, Rect};
use sun_font::{draw_text, FontRole, TextStyle};

pub struct HealthPage {
}

impl HealthPage {
    pub fn new() -> Self {
        Self {}
    }

    pub fn draw(&mut self, canvas: &mut Canvas, rect: Rect) {
        let style = TextStyle::new(FontRole::UiMedium, Color::rgb(255, 255, 255));
        draw_text(canvas, "Health", rect.x + 20, rect.y + 20, &style);
        
        let style_small = TextStyle::new(FontRole::UiSmall, Color::rgb(180, 180, 180));
        draw_text(
            canvas,
            "Component health metrics will appear here.",
            rect.x + 20,
            rect.y + 60,
            &style_small,
        );
    }
}
