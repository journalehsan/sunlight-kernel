use sunlight_ui::{Point, Rect};

use super::tabs::DeveloperToolTab;

pub const DEFAULT_DEVTOOLS_PANEL_H: u32 = 220;
pub const MIN_DEVTOOLS_PANEL_H: u32 = 140;
pub const MIN_MAIN_CONTENT_H: u32 = 160;
pub const RESIZE_HANDLE_H: u32 = 10;
pub const RESIZE_GAP: i32 = 4;
pub const TAB_BAR_H: u32 = 30;
pub const CLOSE_BUTTON_W: u32 = 68;

#[derive(Debug, Clone)]
pub struct DeveloperPanelState {
    pub open: bool,
    pub active_tab: DeveloperToolTab,
    pub height: u32,
    resize_drag_offset: Option<i32>,
}

impl Default for DeveloperPanelState {
    fn default() -> Self {
        Self {
            open: true,
            active_tab: DeveloperToolTab::Console,
            height: DEFAULT_DEVTOOLS_PANEL_H,
            resize_drag_offset: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeveloperPanelLayout {
    pub main_rect: Rect,
    pub resize_handle_rect: Option<Rect>,
    pub panel_rect: Option<Rect>,
    pub tab_bar_rect: Option<Rect>,
    pub close_button_rect: Option<Rect>,
    pub content_rect: Option<Rect>,
}

impl DeveloperPanelState {
    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.resize_drag_offset = None;
    }

    pub fn set_active_tab(&mut self, active_tab: DeveloperToolTab) {
        self.active_tab = active_tab;
    }

    pub fn is_resizing(&self) -> bool {
        self.resize_drag_offset.is_some()
    }

    pub fn finish_resize(&mut self) {
        self.resize_drag_offset = None;
    }

    pub fn compute_layout(&mut self, available_rect: Rect) -> DeveloperPanelLayout {
        if !self.open || available_rect.h <= MIN_MAIN_CONTENT_H {
            self.height = self.height.min(available_rect.h);
            return DeveloperPanelLayout {
                main_rect: available_rect,
                resize_handle_rect: None,
                panel_rect: None,
                tab_bar_rect: None,
                close_button_rect: None,
                content_rect: None,
            };
        }

        let chrome_h = RESIZE_HANDLE_H as i32 + RESIZE_GAP * 2;
        let max_panel_h = (available_rect.h as i32 - MIN_MAIN_CONTENT_H as i32 - chrome_h)
            .max(MIN_DEVTOOLS_PANEL_H as i32) as u32;
        self.height = self.height.clamp(MIN_DEVTOOLS_PANEL_H, max_panel_h);

        let main_h = available_rect
            .h
            .saturating_sub(self.height)
            .saturating_sub(RESIZE_HANDLE_H)
            .saturating_sub((RESIZE_GAP.max(0) as u32) * 2);
        let main_rect = Rect::new(available_rect.x, available_rect.y, available_rect.w, main_h);

        let resize_handle_y = main_rect.bottom() + RESIZE_GAP;
        let resize_handle_rect = Rect::new(
            available_rect.x,
            resize_handle_y,
            available_rect.w,
            RESIZE_HANDLE_H,
        );
        let panel_y = resize_handle_rect.bottom() + RESIZE_GAP;
        let panel_rect = Rect::new(available_rect.x, panel_y, available_rect.w, self.height);
        let close_button_rect = Rect::new(
            panel_rect.right() - CLOSE_BUTTON_W as i32 - 6,
            panel_rect.y + 3,
            CLOSE_BUTTON_W,
            TAB_BAR_H.saturating_sub(6),
        );
        let tab_bar_rect = Rect::new(
            panel_rect.x,
            panel_rect.y,
            panel_rect.w.saturating_sub(CLOSE_BUTTON_W + 12),
            TAB_BAR_H,
        );
        let content_rect = Rect::new(
            panel_rect.x,
            panel_rect.y + TAB_BAR_H as i32,
            panel_rect.w,
            panel_rect.h.saturating_sub(TAB_BAR_H),
        );

        DeveloperPanelLayout {
            main_rect,
            resize_handle_rect: Some(resize_handle_rect),
            panel_rect: Some(panel_rect),
            tab_bar_rect: Some(tab_bar_rect),
            close_button_rect: Some(close_button_rect),
            content_rect: Some(content_rect),
        }
    }

    pub fn begin_resize(&mut self, point: Point, layout: DeveloperPanelLayout) -> bool {
        let Some(handle_rect) = layout.resize_handle_rect else {
            return false;
        };
        if !handle_rect.contains(point) {
            return false;
        }
        self.resize_drag_offset = Some(point.y - handle_rect.y);
        true
    }

    pub fn update_resize(&mut self, pointer_y: i32, available_rect: Rect) -> bool {
        let Some(offset) = self.resize_drag_offset else {
            return false;
        };
        let bottom = available_rect.bottom();
        let new_handle_y = pointer_y - offset;
        let new_panel_top = new_handle_y + RESIZE_HANDLE_H as i32 + RESIZE_GAP;
        let new_height = bottom.saturating_sub(new_panel_top).max(0) as u32;
        let chrome_h = RESIZE_HANDLE_H as i32 + RESIZE_GAP * 2;
        let max_panel_h = (available_rect.h as i32 - MIN_MAIN_CONTENT_H as i32 - chrome_h)
            .max(MIN_DEVTOOLS_PANEL_H as i32) as u32;
        let clamped = new_height.clamp(MIN_DEVTOOLS_PANEL_H, max_panel_h);
        if clamped != self.height {
            self.height = clamped;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_panel_releases_space() {
        let mut panel = DeveloperPanelState::default();
        let area = Rect::new(0, 50, 1000, 600);
        let open_layout = panel.compute_layout(area);
        assert!(open_layout.panel_rect.is_some());
        panel.close();
        let closed_layout = panel.compute_layout(area);
        assert!(closed_layout.panel_rect.is_none());
        assert_eq!(closed_layout.main_rect, area);
        assert!(open_layout.main_rect.h < closed_layout.main_rect.h);
    }
}
