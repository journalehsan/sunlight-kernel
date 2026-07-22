//! Compact Workspace Switcher — shell-owned overlay state.
//!
//! Exposes the four existing display-server workspaces (IDs 1..=4). This module
//! does **not** create a second workspace model. Activation always goes through
//! the shell's existing `switch_workspace` path (`SgpMsg::SET_WORKSPACE`).
//!
//! Visual widgets live in `sunlight_ui::widgets::workspace_switcher` and only
//! consume plain view models built here.

use sunlight_ui::{
    image::TgaImage,
    widgets::{
        WorkspaceCardState, WorkspaceCardView, WorkspaceSwitcherLayout, WorkspaceSwitcherPanel,
        WORKSPACE_CARD_COUNT, WORKSPACE_ICON_SLOTS,
    },
    Canvas, Event, Point, Theme,
};

use crate::{AppId, AppLaunchState, DockAppState, ShellWindowType, WindowSnapshot};

/// Default titles when the compositor has no workspace names (current phase).
pub(crate) const WORKSPACE_TITLES: [&str; WORKSPACE_CARD_COUNT] = [
    "Workspace 1",
    "Workspace 2",
    "Workspace 3",
    "Workspace 4",
];

const KEY_ESC: u8 = 1;
const KEY_1: u8 = 0x02;
const KEY_2: u8 = 0x03;
const KEY_3: u8 = 0x04;
const KEY_4: u8 = 0x05;
const KEY_ENTER: u8 = 0x1C;
const KEY_HOME: u8 = 0x47;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_END: u8 = 0x4F;
const KEY_SPACE: u8 = 0x39;

/// Bounded identity for icon resolution (not a second workspace model).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WorkspaceAppIcon {
    App(AppId),
    /// Window counted but no known AppId — use generic icon.
    Generic,
}

/// Per-workspace summary derived from authoritative LIST_WINDOWS snapshots.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct WorkspaceCardSummary {
    pub workspace_id: u8,
    pub window_count: u32,
    pub icons: [Option<WorkspaceAppIcon>; WORKSPACE_ICON_SLOTS],
    pub icon_len: u8,
    /// Additional unique app identities beyond the three icon slots.
    pub overflow: u32,
    pub empty: bool,
}

impl WorkspaceCardSummary {
    pub(crate) const fn empty(workspace_id: u8) -> Self {
        Self {
            workspace_id,
            window_count: 0,
            icons: [None; WORKSPACE_ICON_SLOTS],
            icon_len: 0,
            overflow: 0,
            empty: true,
        }
    }
}

/// Result of a switcher interaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum WorkspaceSwitcherAction {
    None,
    Close,
    /// Request activation of workspace id `1..=4` via the shell path.
    Activate(u8),
}

/// Session-local Workspace Switcher state. Bounded; no timers or heap growth.
pub(crate) struct WorkspaceSwitcherState {
    open: bool,
    /// Keyboard focus index into cards (`0..4`).
    focus_index: usize,
    hover: Option<usize>,
    cards: [WorkspaceCardSummary; WORKSPACE_CARD_COUNT],
    /// Short failure status (`"Switch failed"`) or empty.
    status: [u8; 24],
    status_len: u8,
}

impl WorkspaceSwitcherState {
    pub(crate) const fn new() -> Self {
        Self {
            open: false,
            focus_index: 0,
            hover: None,
            cards: [
                WorkspaceCardSummary::empty(1),
                WorkspaceCardSummary::empty(2),
                WorkspaceCardSummary::empty(3),
                WorkspaceCardSummary::empty(4),
            ],
            status: [0; 24],
            status_len: 0,
        }
    }

    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) const fn focus_index(&self) -> usize {
        self.focus_index
    }

    pub(crate) fn cards(&self) -> &[WorkspaceCardSummary; WORKSPACE_CARD_COUNT] {
        &self.cards
    }

    pub(crate) fn open(&mut self, active_workspace: u8) {
        self.open = true;
        self.focus_index = workspace_id_to_index(active_workspace);
        self.hover = None;
        self.clear_status();
    }

    pub(crate) fn close(&mut self) -> bool {
        let was = self.open;
        self.open = false;
        self.hover = None;
        self.clear_status();
        was
    }

    pub(crate) fn set_status(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let n = bytes.len().min(self.status.len());
        self.status[..n].copy_from_slice(&bytes[..n]);
        self.status_len = n as u8;
    }

    pub(crate) fn clear_status(&mut self) {
        self.status_len = 0;
    }

    fn status_str(&self) -> Option<&str> {
        if self.status_len == 0 {
            None
        } else {
            core::str::from_utf8(&self.status[..self.status_len as usize]).ok()
        }
    }

    /// Replace card summaries from authoritative window + app state.
    /// Returns true when any visible summary field changed.
    pub(crate) fn observe_summaries(
        &mut self,
        summaries: [WorkspaceCardSummary; WORKSPACE_CARD_COUNT],
    ) -> bool {
        if self.cards == summaries {
            return false;
        }
        self.cards = summaries;
        true
    }

    pub(crate) fn layout(
        &self,
        screen_w: u32,
        screen_h: u32,
        top_inset: i32,
        dock_top_y: i32,
    ) -> WorkspaceSwitcherLayout {
        WorkspaceSwitcherLayout::compute(screen_w, screen_h, top_inset, dock_top_y)
    }

    pub(crate) fn contains(
        &self,
        point: Point,
        screen_w: u32,
        screen_h: u32,
        top_inset: i32,
        dock_top_y: i32,
    ) -> bool {
        self.open
            && self
                .layout(screen_w, screen_h, top_inset, dock_top_y)
                .contains(point)
    }

    pub(crate) fn handle_event(
        &mut self,
        event: Event,
        screen_w: u32,
        screen_h: u32,
        top_inset: i32,
        dock_top_y: i32,
    ) -> (bool, WorkspaceSwitcherAction) {
        if !self.open {
            return (false, WorkspaceSwitcherAction::None);
        }
        let layout = self.layout(screen_w, screen_h, top_inset, dock_top_y);
        match event {
            Event::Click { x, y } => {
                let p = Point::new(x, y);
                if !layout.contains(p) {
                    return (true, WorkspaceSwitcherAction::Close);
                }
                if let Some(idx) = layout.card_index_at(p) {
                    self.focus_index = idx;
                    let ws = index_to_workspace_id(idx);
                    return (true, WorkspaceSwitcherAction::Activate(ws));
                }
                (true, WorkspaceSwitcherAction::None)
            }
            Event::MouseMove { x, y, .. } => {
                let p = Point::new(x, y);
                let next = layout.card_index_at(p);
                if self.hover != next {
                    self.hover = next;
                    return (true, WorkspaceSwitcherAction::None);
                }
                (false, WorkspaceSwitcherAction::None)
            }
            Event::MouseDown { x, y, .. } => {
                let p = Point::new(x, y);
                if !layout.contains(p) {
                    // Outside press: close without activating underlying target
                    // from this press (shell sets suppress_next_click).
                    return (true, WorkspaceSwitcherAction::Close);
                }
                if let Some(idx) = layout.card_index_at(p) {
                    self.focus_index = idx;
                    return (true, WorkspaceSwitcherAction::None);
                }
                (true, WorkspaceSwitcherAction::None)
            }
            Event::Key('\x1b') => (true, WorkspaceSwitcherAction::Close),
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } => self.handle_keypress(keycode),
            _ => (false, WorkspaceSwitcherAction::None),
        }
    }

    fn handle_keypress(&mut self, keycode: u8) -> (bool, WorkspaceSwitcherAction) {
        match keycode {
            KEY_ESC => (true, WorkspaceSwitcherAction::Close),
            KEY_LEFT => {
                if self.focus_index > 0 {
                    self.focus_index -= 1;
                }
                (true, WorkspaceSwitcherAction::None)
            }
            KEY_RIGHT => {
                if self.focus_index + 1 < WORKSPACE_CARD_COUNT {
                    self.focus_index += 1;
                }
                (true, WorkspaceSwitcherAction::None)
            }
            KEY_HOME => {
                self.focus_index = 0;
                (true, WorkspaceSwitcherAction::None)
            }
            KEY_END => {
                self.focus_index = WORKSPACE_CARD_COUNT - 1;
                (true, WorkspaceSwitcherAction::None)
            }
            KEY_ENTER | KEY_SPACE => {
                let ws = index_to_workspace_id(self.focus_index);
                (true, WorkspaceSwitcherAction::Activate(ws))
            }
            KEY_1 => (true, WorkspaceSwitcherAction::Activate(1)),
            KEY_2 => (true, WorkspaceSwitcherAction::Activate(2)),
            KEY_3 => (true, WorkspaceSwitcherAction::Activate(3)),
            KEY_4 => (true, WorkspaceSwitcherAction::Activate(4)),
            _ => (false, WorkspaceSwitcherAction::None),
        }
    }

    /// Draw using shell-resolved icon images for each card slot.
    pub(crate) fn view(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        screen_w: u32,
        screen_h: u32,
        top_inset: i32,
        dock_top_y: i32,
        icon_images: &[[Option<TgaImage>; WORKSPACE_ICON_SLOTS]; WORKSPACE_CARD_COUNT],
        generic_icon: Option<TgaImage>,
        active_workspace: u8,
    ) {
        if !self.open {
            return;
        }
        let layout = self.layout(screen_w, screen_h, top_inset, dock_top_y);

        // Build temporary view models with icon references.
        let mut icon_refs: [[Option<&TgaImage>; WORKSPACE_ICON_SLOTS]; WORKSPACE_CARD_COUNT] =
            [[None; WORKSPACE_ICON_SLOTS]; WORKSPACE_CARD_COUNT];
        for (ci, row) in icon_images.iter().enumerate() {
            for (si, img) in row.iter().enumerate() {
                icon_refs[ci][si] = img.as_ref();
            }
        }

        let mut views = [WorkspaceCardView {
            id: 1,
            title: WORKSPACE_TITLES[0],
            window_count: 0,
            empty: true,
            icons: &[],
            overflow: 0,
            state: WorkspaceCardState::default(),
        }; WORKSPACE_CARD_COUNT];

        for i in 0..WORKSPACE_CARD_COUNT {
            let summary = self.cards[i];
            let icon_slice = &icon_refs[i][..summary.icon_len as usize];
            views[i] = WorkspaceCardView {
                id: summary.workspace_id,
                title: WORKSPACE_TITLES[i],
                window_count: summary.window_count,
                empty: summary.empty,
                icons: icon_slice,
                overflow: summary.overflow,
                state: WorkspaceCardState {
                    active: summary.workspace_id == active_workspace,
                    focused: self.focus_index == i,
                    hovered: self.hover == Some(i),
                },
            };
        }

        WorkspaceSwitcherPanel {
            layout,
            cards: &views,
            generic_icon: generic_icon.as_ref(),
            status: self.status_str(),
        }
        .draw(canvas, theme);
    }
}

#[inline]
pub(crate) fn workspace_id_to_index(ws: u8) -> usize {
    if (1..=WORKSPACE_CARD_COUNT as u8).contains(&ws) {
        (ws - 1) as usize
    } else {
        0
    }
}

#[inline]
pub(crate) fn index_to_workspace_id(index: usize) -> u8 {
    (index.min(WORKSPACE_CARD_COUNT - 1) + 1) as u8
}

/// True for windows that should appear in Workspace Switcher counts/icons.
pub(crate) fn is_switcher_window(window: &WindowSnapshot) -> bool {
    if window.hidden {
        return false;
    }
    // Only normal application windows — never panels, desktop, widgets, or dialogs.
    matches!(window.window_type, ShellWindowType::Normal)
}

/// Map a window owner pid to a known shell app when the generation-aware
/// registry / dock state already tracks it. Never invents identity from PID
/// alone when the registry has no live association.
pub(crate) fn app_icon_for_window(
    window: &WindowSnapshot,
    apps: &[DockAppState],
) -> WorkspaceAppIcon {
    for app in apps {
        if app.pid == Some(window.owner_pid)
            && matches!(
                app.state,
                AppLaunchState::Running
                    | AppLaunchState::Minimized
                    | AppLaunchState::Launching
                    | AppLaunchState::Closing
            )
        {
            return WorkspaceAppIcon::App(app.app_id);
        }
        if app.main_window_id == Some(window.id)
            && matches!(
                app.state,
                AppLaunchState::Running
                    | AppLaunchState::Minimized
                    | AppLaunchState::Launching
                    | AppLaunchState::Closing
            )
        {
            return WorkspaceAppIcon::App(app.app_id);
        }
    }
    WorkspaceAppIcon::Generic
}

/// Build four card summaries from the authoritative window list.
///
/// - Window count = true normal-window count per workspace.
/// - Icon list shows each unique app identity at most once (bounded to 3).
/// - Overflow is unique apps beyond the three slots, not residual windows.
/// - Closed windows are simply absent from `windows` (caller rebuilds).
pub(crate) fn build_workspace_summaries(
    windows: &[WindowSnapshot],
    apps: &[DockAppState],
) -> [WorkspaceCardSummary; WORKSPACE_CARD_COUNT] {
    let mut out = [
        WorkspaceCardSummary::empty(1),
        WorkspaceCardSummary::empty(2),
        WorkspaceCardSummary::empty(3),
        WorkspaceCardSummary::empty(4),
    ];

    // Unique apps per workspace (bounded buffer for overflow accounting).
    const MAX_UNIQUE: usize = 16;
    let mut unique: [[Option<WorkspaceAppIcon>; MAX_UNIQUE]; WORKSPACE_CARD_COUNT] =
        [[None; MAX_UNIQUE]; WORKSPACE_CARD_COUNT];
    let mut unique_len = [0usize; WORKSPACE_CARD_COUNT];

    for window in windows {
        if !is_switcher_window(window) {
            continue;
        }
        let ws = window.workspace_id as u8;
        if !(1..=4).contains(&ws) {
            continue;
        }
        let idx = (ws - 1) as usize;
        out[idx].window_count = out[idx].window_count.saturating_add(1);

        let icon = app_icon_for_window(window, apps);
        let len = unique_len[idx];
        let already = unique[idx][..len].iter().any(|slot| *slot == Some(icon));
        if !already && len < MAX_UNIQUE {
            unique[idx][len] = Some(icon);
            unique_len[idx] = len + 1;
        }
    }

    for i in 0..WORKSPACE_CARD_COUNT {
        let len = unique_len[i];
        out[i].empty = out[i].window_count == 0;
        let show = len.min(WORKSPACE_ICON_SLOTS);
        out[i].icon_len = show as u8;
        for s in 0..show {
            out[i].icons[s] = unique[i][s];
        }
        out[i].overflow = len.saturating_sub(WORKSPACE_ICON_SLOTS) as u32;
    }

    out
}

/// Reject stale generation pairs when a source provides them.
///
/// Current LIST_WINDOWS reports generation 0; this helper still enforces the
/// contract for future event sources and unit tests.
pub(crate) fn generation_matches(expected: u64, observed: u64) -> bool {
    if expected == 0 || observed == 0 {
        // Zero means "unknown" on either side — do not reject.
        return true;
    }
    expected == observed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ShellWindowState, ShellWindowType, WindowSnapshot};

    fn win(id: u64, pid: u64, ws: u64, wtype: ShellWindowType, hidden: bool) -> WindowSnapshot {
        WindowSnapshot {
            id,
            owner_pid: pid,
            state: ShellWindowState::Normal,
            window_type: wtype,
            workspace_id: ws,
            hidden,
            rolled_up: false,
            title: [0; 16],
        }
    }

    fn app(id: AppId, pid: Option<u64>, main_window_id: Option<u64>) -> DockAppState {
        let mut a = DockAppState::new(id, "Test", id);
        a.pid = pid;
        a.main_window_id = main_window_id;
        a.state = AppLaunchState::Running;
        a
    }

    #[test]
    fn exactly_four_workspace_ids() {
        let summaries = build_workspace_summaries(&[], &[]);
        assert_eq!(summaries.len(), 4);
        for (i, s) in summaries.iter().enumerate() {
            assert_eq!(s.workspace_id, (i + 1) as u8);
            assert!(s.empty);
            assert_eq!(s.window_count, 0);
        }
    }

    #[test]
    fn panels_and_overlays_excluded_from_counts() {
        let windows = [
            win(1, 10, 1, ShellWindowType::Desktop, false),
            win(2, 11, 1, ShellWindowType::Widget, false),
            win(3, 12, 1, ShellWindowType::Dialog, false),
            win(4, 13, 1, ShellWindowType::Normal, false),
            win(5, 14, 1, ShellWindowType::Normal, true),
        ];
        let summaries = build_workspace_summaries(&windows, &[]);
        assert_eq!(summaries[0].window_count, 1);
        assert!(!summaries[0].empty);
    }

    #[test]
    fn multi_window_same_app_keeps_window_count_and_one_icon() {
        let windows = [
            win(1, 42, 2, ShellWindowType::Normal, false),
            win(2, 42, 2, ShellWindowType::Normal, false),
            win(3, 42, 2, ShellWindowType::Normal, false),
        ];
        let apps = [app(AppId::Terminal, Some(42), Some(1))];
        let summaries = build_workspace_summaries(&windows, &apps);
        assert_eq!(summaries[1].window_count, 3);
        assert_eq!(summaries[1].icon_len, 1);
        assert_eq!(
            summaries[1].icons[0],
            Some(WorkspaceAppIcon::App(AppId::Terminal))
        );
        assert_eq!(summaries[1].overflow, 0);
    }

    #[test]
    fn icon_overflow_produces_plus_n() {
        let windows = [
            win(1, 1, 1, ShellWindowType::Normal, false),
            win(2, 2, 1, ShellWindowType::Normal, false),
            win(3, 3, 1, ShellWindowType::Normal, false),
            win(4, 4, 1, ShellWindowType::Normal, false),
            win(5, 5, 1, ShellWindowType::Normal, false),
        ];
        let apps = [
            app(AppId::Terminal, Some(1), Some(1)),
            app(AppId::Files, Some(2), Some(2)),
            app(AppId::Calculator, Some(3), Some(3)),
            app(AppId::Settings, Some(4), Some(4)),
            app(AppId::Tasks, Some(5), Some(5)),
        ];
        let summaries = build_workspace_summaries(&windows, &apps);
        assert_eq!(summaries[0].window_count, 5);
        assert_eq!(summaries[0].icon_len, 3);
        assert_eq!(summaries[0].overflow, 2);
    }

    #[test]
    fn closed_windows_disappear() {
        let open = [win(1, 9, 3, ShellWindowType::Normal, false)];
        let apps = [app(AppId::Files, Some(9), Some(1))];
        let with = build_workspace_summaries(&open, &apps);
        assert_eq!(with[2].window_count, 1);
        let without = build_workspace_summaries(&[], &apps);
        assert_eq!(without[2].window_count, 0);
        assert!(without[2].empty);
        assert_eq!(without[2].icon_len, 0);
    }

    #[test]
    fn missing_identity_uses_generic_and_still_counts() {
        let windows = [win(9, 999, 4, ShellWindowType::Normal, false)];
        let summaries = build_workspace_summaries(&windows, &[]);
        assert_eq!(summaries[3].window_count, 1);
        assert_eq!(summaries[3].icons[0], Some(WorkspaceAppIcon::Generic));
    }

    #[test]
    fn stale_generation_rejected_when_both_nonzero() {
        assert!(!generation_matches(3, 4));
        assert!(generation_matches(0, 4));
        assert!(generation_matches(3, 0));
        assert!(generation_matches(7, 7));
    }

    #[test]
    fn open_selects_active_workspace_card() {
        let mut sw = WorkspaceSwitcherState::new();
        sw.open(3);
        assert!(sw.is_open());
        assert_eq!(sw.focus_index(), 2);
        sw.open(1);
        assert_eq!(sw.focus_index(), 0);
    }

    #[test]
    fn escape_and_outside_click_close() {
        let mut sw = WorkspaceSwitcherState::new();
        sw.open(1);
        let (dirty, action) = sw.handle_event(Event::key('\x1b'), 1366, 768, 48, 716);
        assert!(dirty);
        assert_eq!(action, WorkspaceSwitcherAction::Close);

        sw.open(2);
        let layout = sw.layout(1366, 768, 48, 716);
        let outside = Event::click(0, 0);
        let (_, action) = sw.handle_event(outside, 1366, 768, 48, 716);
        assert_eq!(action, WorkspaceSwitcherAction::Close);
        // panel itself should be non-empty for hit tests
        assert!(layout.panel.w >= 640 || layout.panel.w > 0);
    }

    #[test]
    fn number_keys_and_arrows_navigate() {
        let mut sw = WorkspaceSwitcherState::new();
        sw.open(2);
        assert_eq!(sw.focus_index(), 1);

        let (_, a) = sw.handle_event(
            Event::key_press(KEY_RIGHT, true, false, false, false, false),
            1366,
            768,
            48,
            716,
        );
        assert_eq!(a, WorkspaceSwitcherAction::None);
        assert_eq!(sw.focus_index(), 2);

        let (_, a) = sw.handle_event(
            Event::key_press(KEY_END, true, false, false, false, false),
            1366,
            768,
            48,
            716,
        );
        assert_eq!(a, WorkspaceSwitcherAction::None);
        assert_eq!(sw.focus_index(), 3);

        let (_, a) = sw.handle_event(
            Event::key_press(KEY_1, true, false, false, false, false),
            1366,
            768,
            48,
            716,
        );
        assert_eq!(a, WorkspaceSwitcherAction::Activate(1));

        let (_, a) = sw.handle_event(
            Event::key_press(KEY_ENTER, true, false, false, false, false),
            1366,
            768,
            48,
            716,
        );
        assert_eq!(a, WorkspaceSwitcherAction::Activate(index_to_workspace_id(sw.focus_index())));
    }

    #[test]
    fn selecting_card_activates_existing_id() {
        let mut sw = WorkspaceSwitcherState::new();
        sw.open(1);
        let layout = sw.layout(1366, 768, 48, 716);
        let c = layout.cards[3];
        let (_, action) = sw.handle_event(
            Event::click(c.x + 2, c.y + 2),
            1366,
            768,
            48,
            716,
        );
        assert_eq!(action, WorkspaceSwitcherAction::Activate(4));
    }

    #[test]
    fn repeated_open_close_keeps_fixed_card_storage() {
        let mut sw = WorkspaceSwitcherState::new();
        for i in 0..64 {
            sw.open(((i % 4) + 1) as u8);
            let _ = sw.observe_summaries(build_workspace_summaries(&[], &[]));
            assert!(sw.close());
            assert!(!sw.is_open());
        }
        // Still exactly four empty cards — no growth.
        assert_eq!(sw.cards().len(), 4);
    }
}
