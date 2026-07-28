use alloc::string::String;
use alloc::vec::Vec;
use sunlight_ui::{Canvas, Color, Rect};
use sun_font::{draw_text, FontRole, TextStyle};

use crate::character::OwlCharacter;

pub struct ConversationPage {
    pub messages: Vec<Message>,
    pub owl: OwlCharacter,
}

#[derive(Debug, Clone)]
pub enum Message {
    User(String),
    Assistant(String),
    SystemStatus(String),
    Clarification(ClarificationData),
    Confirmation(ConfirmationData),
    ActionProgress(ActionProgressData),
    ActionResult(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct ClarificationData {
    pub question: String,
    pub candidates: Vec<(u64, String)>,
}

#[derive(Debug, Clone)]
pub struct ConfirmationData {
    pub description: String,
    pub target: String,
    pub level: u8,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct ActionProgressData {
    pub label: String,
}

impl ConversationPage {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            owl: OwlCharacter::new(),
        }
    }

    pub fn draw(&mut self, canvas: &mut Canvas, rect: Rect) {
        let style = TextStyle::new(FontRole::UiMedium, Color::rgb(255, 255, 255));
        draw_text(canvas, "Conversation", rect.x + 20, rect.y + 20, &style);
        
        let mut y = rect.y + 60;
        for msg in &self.messages {
            match msg {
                Message::User(text) => {
                    let s = TextStyle::new(FontRole::UiMedium, Color::rgb(200, 255, 200));
                    draw_text(canvas, text, rect.x + 40, y, &s);
                    y += 30;
                }
                Message::Assistant(text) => {
                    let s = TextStyle::new(FontRole::UiMedium, Color::rgb(200, 200, 255));
                    draw_text(canvas, text, rect.x + 20, y, &s);
                    y += 30;
                }
                Message::SystemStatus(text) | Message::ActionResult(text) => {
                    let s = TextStyle::new(FontRole::UiSmall, Color::rgb(150, 150, 150));
                    draw_text(canvas, text, rect.x + 20, y, &s);
                    y += 25;
                }
                Message::ActionProgress(data) => {
                    let s = TextStyle::new(FontRole::UiSmall, Color::rgb(255, 200, 100));
                    draw_text(canvas, &data.label, rect.x + 20, y, &s);
                    y += 25;
                }
                Message::Error(text) => {
                    let s = TextStyle::new(FontRole::UiMedium, Color::rgb(255, 100, 100));
                    draw_text(canvas, text, rect.x + 20, y, &s);
                    y += 30;
                }
                Message::Clarification(data) => {
                    let s = TextStyle::new(FontRole::UiMedium, Color::rgb(255, 255, 200));
                    draw_text(canvas, &data.question, rect.x + 20, y, &s);
                    y += 30;
                    for (_, label) in &data.candidates {
                        let cs = TextStyle::new(FontRole::UiSmall, Color::rgb(200, 200, 200));
                        draw_text(canvas, label, rect.x + 40, y, &cs);
                        y += 25;
                    }
                }
                Message::Confirmation(data) => {
                    let s = TextStyle::new(FontRole::UiMedium, Color::rgb(255, 200, 200));
                    draw_text(canvas, &data.description, rect.x + 20, y, &s);
                    y += 30;
                    let cs = TextStyle::new(FontRole::UiSmall, Color::rgb(200, 200, 200));
                    draw_text(canvas, &data.reason, rect.x + 20, y, &cs);
                    y += 25;
                }
            }
        }
        
        let input_y = rect.y + rect.h as i32 - 40;
        canvas.fill_rect(
            Rect::new(rect.x + 10, input_y, rect.w - 20, 30),
            Color::rgb(50, 50, 55),
        );
        let placeholder = TextStyle::new(FontRole::UiSmall, Color::rgb(150, 150, 150));
        draw_text(canvas, "Type a message...", rect.x + 20, input_y + 8, &placeholder);

        self.owl.update();
        let owl_rect = Rect::new(rect.x + rect.w as i32 - 60, rect.y + rect.h as i32 - 100, 50, 50);
        self.owl.draw(canvas, owl_rect);
    }
}
