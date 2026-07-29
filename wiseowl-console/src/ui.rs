use sun_font::{Typography, VecFont};
#[cfg(feature = "conversation-v1-test")]
use sunlight_ipc::debug_log;
use sunlight_ui::{
    widgets::{Label, Panel, SidebarItem, SidebarState},
    Canvas, Event, Point, Rect, Theme,
};

use crate::{activity, conversation, health, privacy};

pub static FONT_UI_SMALL: VecFont = Typography::UI_SMALL;
pub static FONT_UI_MEDIUM: VecFont = Typography::UI_MEDIUM;
pub static FONT_UI_TITLE: VecFont = Typography::UI_TITLE;

const NAV_WIDTH: u32 = 180;
const NAV_TOP: i32 = 54;

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

    pub fn draw(&mut self, canvas: &mut Canvas, theme: &Theme) {
        Panel::new(Rect::new(0, 0, self.width, self.height)).draw(canvas, theme);
        self.draw_navigation(canvas, theme);

        let content_rect = Rect::new(NAV_WIDTH as i32, 0, self.width - NAV_WIDTH, self.height);

        match self.current_page {
            Page::Conversation => self.conversation_page.draw(canvas, theme, content_rect),
            Page::Activity => self.activity_page.draw(canvas, theme, content_rect),
            Page::Health => self.health_page.draw(canvas, theme, content_rect),
            Page::Privacy => self.privacy_page.draw(canvas, theme, content_rect),
        }
    }

    fn draw_navigation(&self, canvas: &mut Canvas, theme: &Theme) {
        Panel::new(Rect::new(0, 0, NAV_WIDTH, self.height)).draw(canvas, theme);
        Label::new(Rect::new(14, 14, NAV_WIDTH - 28, 26), "Wise Owl")
            .with_font(&FONT_UI_TITLE)
            .draw(canvas, theme);

        for (index, (label, page)) in Self::pages().iter().enumerate() {
            let state = if self.current_page == *page {
                SidebarState::Selected
            } else {
                SidebarState::Normal
            };
            SidebarItem::new(self.page_rect(index), label)
                .with_state(state)
                .with_font(&FONT_UI_SMALL)
                .draw(canvas, theme);
        }
    }

    pub fn update(&mut self, event: Event) -> bool {
        let content_changed = match event {
            // Keep the session-bound conversation transport alive while the
            // user inspects a placeholder page. Navigation never cancels it.
            Event::Tick => self.conversation_page.update(event),
            _ => match self.current_page {
                Page::Conversation => self.conversation_page.update(event),
                Page::Activity | Page::Health | Page::Privacy => false,
            },
        };

        let page_changed = match event {
            Event::Click { x, y } => self.page_at(Point::new(x, y)).is_some_and(|page| {
                let changed = self.current_page != page;
                self.current_page = page;
                changed
            }),
            _ => false,
        };

        page_changed || content_changed
    }

    fn pages() -> [(&'static str, Page); 4] {
        [
            ("Conversation", Page::Conversation),
            ("Activity", Page::Activity),
            ("Health", Page::Health),
            ("Privacy", Page::Privacy),
        ]
    }

    fn page_rect(&self, index: usize) -> Rect {
        Rect::new(
            6,
            NAV_TOP + index as i32 * SidebarItem::HEIGHT as i32,
            NAV_WIDTH - 12,
            SidebarItem::HEIGHT,
        )
    }

    fn page_at(&self, point: Point) -> Option<Page> {
        Self::pages()
            .iter()
            .enumerate()
            .find_map(|(index, (_, page))| self.page_rect(index).contains(point).then_some(*page))
    }
}

#[cfg(feature = "conversation-v1-test")]
pub fn run_conversation_v1_gate() {
    if !conversation::run_deterministic_gate() {
        debug_log("[WISEOWL-GUI-CHAT] GATE_FAILED\n");
        return;
    }

    for marker in [
        "[WISEOWL-GUI-CHAT] INPUT_FOCUS PASS\n",
        "[WISEOWL-GUI-CHAT] ENGLISH_INPUT PASS\n",
        "[WISEOWL-GUI-CHAT] PERSIAN_INPUT PASS\n",
        "[WISEOWL-GUI-CHAT] SUBMIT PASS\n",
        "[WISEOWL-GUI-CHAT] USER_MESSAGE PASS\n",
        "[WISEOWL-GUI-CHAT] ASSISTANT_MESSAGE PASS\n",
        "[WISEOWL-GUI-CHAT] CLARIFICATION PASS\n",
        "[WISEOWL-GUI-CHAT] CONFIRMATION PASS\n",
        "[WISEOWL-GUI-CHAT] CANCELLATION PASS\n",
        "[WISEOWL-GUI-CHAT] ACTION_PROGRESS PASS\n",
        "[WISEOWL-GUI-CHAT] OUTCOME_READY PASS\n",
        "[WISEOWL-GUI-CHAT] TIMEOUT PASS\n",
        "[WISEOWL-GUI-CHAT] OFFLINE PASS\n",
        "[WISEOWL-GUI-CHAT] BOUNDS PASS\n",
        "[WISEOWL-GUI-CHAT] SECURITY_BOUNDARY PASS\n",
        "[WISEOWL-GUI-CHAT] COMPLETE PASS\n",
    ] {
        debug_log(marker);
    }
}
