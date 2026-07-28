use sunlight_ui::{Canvas, Color, Point, Rect};
use sun_font::{draw_text, FontRole, TextStyle};

use crate::{activity, conversation, health, privacy};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Conversation,
    Activity,
    Health,
    Privacy,
}

pub struct UiState {
    pub current_page: Page,
    pub width: u32,
    pub height: u32,
    pub conversation_page: conversation::ConversationPage,
    pub activity_page: activity::ActivityPage,
    pub health_page: health::HealthPage,
    pub privacy_page: privacy::PrivacyPage,
}

impl UiState {
    pub fn new(w: u32, h: u32) -> Self {
        Self {
            current_page: Page::Conversation,
            width: w,
            height: h,
            conversation_page: conversation::ConversationPage::new(),
            activity_page: activity::ActivityPage::new(),
            health_page: health::HealthPage::new(),
            privacy_page: privacy::PrivacyPage::new(),
        }
    }

    pub fn draw(&mut self, canvas: &mut Canvas) {
        canvas.fill_rect(
            Rect::new(0, 0, self.width, self.height),
            Color::rgb(30, 30, 35),
        );
        
        self.draw_navigation(canvas);
        
        let content_rect = Rect::new(120, 0, self.width - 120, self.height);
        
        match self.current_page {
            Page::Conversation => self.conversation_page.draw(canvas, content_rect),
            Page::Activity => self.activity_page.draw(canvas, content_rect),
            Page::Health => self.health_page.draw(canvas, content_rect),
            Page::Privacy => self.privacy_page.draw(canvas, content_rect),
        }
    }

    fn draw_navigation(&self, canvas: &mut Canvas) {
        let rail_w = 120;
        canvas.fill_rect(Rect::new(0, 0, rail_w, self.height), Color::rgb(45, 45, 50));
        
        let style = TextStyle::new(FontRole::UiMedium, Color::rgb(255, 255, 255));
        
        draw_text(canvas, "Wise Owl", 10, 20, &style);
        
        let pages = [
            ("Conversation", Page::Conversation),
            ("Activity", Page::Activity),
            ("Health", Page::Health),
            ("Privacy", Page::Privacy),
        ];
        
        let mut y = 60;
        for (label, page) in pages {
            let color = if self.current_page == page {
                Color::rgb(100, 150, 255)
            } else {
                Color::rgb(200, 200, 200)
            };
            let item_style = TextStyle::new(FontRole::UiSmall, color);
            draw_text(canvas, label, 10, y, &item_style);
            y += 40;
        }
    }

    pub fn handle_click(&mut self, x: i32, y: i32) -> bool {
        if !Rect::new(0, 0, 120, self.height).contains(Point::new(x, y)) {
            return false;
        }

        let page = match y {
            52..=91 => Some(Page::Conversation),
            92..=131 => Some(Page::Activity),
            132..=171 => Some(Page::Health),
            172..=211 => Some(Page::Privacy),
            _ => None,
        };
        if let Some(page) = page {
            let changed = self.current_page != page;
            self.current_page = page;
            changed
        } else {
            false
        }
    }
}
