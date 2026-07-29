use sunlight_ui::{
    widgets::{Label, Panel},
    Canvas, Rect, Theme,
};

use crate::ui::{FONT_UI_MEDIUM, FONT_UI_SMALL};

pub struct HealthPage {}

impl HealthPage {
    pub fn new() -> Self {
        Self {}
    }

    pub fn draw(&mut self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "Health").with_font(&FONT_UI_MEDIUM);
        panel.draw(canvas, theme);
        Label::new(
            panel.content_rect().inset(18),
            "Component health metrics will appear here.",
        )
        .dim()
        .with_font(&FONT_UI_SMALL)
        .draw(canvas, theme);
    }
}
