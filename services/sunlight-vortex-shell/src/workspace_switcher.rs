//! Compact Workspace Switcher — shell-owned overlay state.
//!
//! Exposes the four existing display-server workspaces (IDs 1..=4). This module
//! browses a static snapshot. Activation goes through the compositor's
//! existing window activation path.
//!
//! Visual widgets live in `sunlight_ui::widgets::workspace_switcher` and only
//! consume plain view models built here.

use alloc::vec::Vec;
use sunlight_ipc::{ipc_call_timeout, sgp::SgpMsg, shm_create, shm_free, CapabilityToken};
use sunlight_ui::{
    image::TgaImage,
    widgets::{
        WorkspaceCardState, WorkspaceCardView, WorkspaceSwitcherLayout, WorkspaceSwitcherPanel,
        WORKSPACE_CARD_COUNT, WORKSPACE_ICON_SLOTS,
    },
    Canvas, Color, Event, Point, Rect, Theme,
};

use crate::{AppId, AppLaunchState, DockAppState, ShellWindowType, WindowSnapshot};

/// Default titles when the compositor has no workspace names (current phase).
pub(crate) const WORKSPACE_TITLES: [&str; WORKSPACE_CARD_COUNT] =
    ["Workspace 1", "Workspace 2", "Workspace 3", "Workspace 4"];

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
    ActivateWindow(u64),
}

struct PreviewPixels {
    w: u32,
    h: u32,
    pixels: Vec<u32>,
}

struct PreviewWindow {
    id: u64,
    workspace: u8,
    geometry: Rect,
    title: [u8; SgpMsg::WORKSPACE_PREVIEW_TITLE_BYTES],
    icon: Option<TgaImage>,
    image: Option<PreviewPixels>,
}

fn fit_rect(area: Rect, w: u32, h: u32) -> Rect {
    if w == 0 || h == 0 || area.w == 0 || area.h == 0 {
        return Rect::default();
    }
    let (w, h) = if u64::from(w) * u64::from(area.h) >= u64::from(h) * u64::from(area.w) {
        (
            area.w,
            (u64::from(h) * u64::from(area.w) / u64::from(w)).max(1) as u32,
        )
    } else {
        (
            (u64::from(w) * u64::from(area.h) / u64::from(h)).max(1) as u32,
            area.h,
        )
    };
    Rect::new(
        area.x + ((area.w - w) / 2) as i32,
        area.y + ((area.h - h) / 2) as i32,
        w,
        h,
    )
}

fn miniature_geometry(outer: Rect, desktop: Rect, miniature: Rect) -> Option<Rect> {
    let clipped = outer.intersect(desktop)?;
    let x = |v: i32| (i64::from(v) * i64::from(miniature.w) / i64::from(desktop.w)) as i32;
    let y = |v: i32| (i64::from(v) * i64::from(miniature.h) / i64::from(desktop.h)) as i32;
    let (left, top) = (x(clipped.x), y(clipped.y));
    Some(Rect::new(
        miniature.x + left,
        miniature.y + top,
        (x(clipped.right()) - left).max(1) as u32,
        (y(clipped.bottom()) - top).max(1) as u32,
    ))
}

fn read_preview(
    ep: CapabilityToken,
    window: &mut PreviewWindow,
    buf: *mut u8,
    cap: CapabilityToken,
) -> Result<(), ()> {
    let reply = ipc_call_timeout(
        ep,
        sunlight_ipc::IpcMsg::with_label(SgpMsg::WORKSPACE_PREVIEW)
            .word(0, window.id)
            .with_cap(0, cap),
        crate::DISPLAY_IPC_TIMEOUT_MS,
    )
    .map_err(|_| ())?;
    if reply.label != SgpMsg::REPLY || reply.words[0] != window.id {
        return Ok(());
    }
    let xy = reply.words[1];
    let wh = reply.words[2];
    window.geometry = Rect::new(xy as i32, (xy >> 32) as i32, wh as u32, (wh >> 32) as u32);
    let (w, h) = (reply.words[3] as u32, (reply.words[3] >> 32) as u32);
    unsafe {
        core::ptr::copy_nonoverlapping(buf, window.title.as_mut_ptr(), window.title.len());
    }
    if (1..=SgpMsg::WORKSPACE_PREVIEW_MAX_W).contains(&w)
        && (1..=SgpMsg::WORKSPACE_PREVIEW_MAX_H).contains(&h)
    {
        let pixels = unsafe {
            core::slice::from_raw_parts(
                buf.add(SgpMsg::WORKSPACE_PREVIEW_TITLE_BYTES).cast::<u32>(),
                (w * h) as usize,
            )
        };
        window.image = Some(PreviewPixels {
            w,
            h,
            pixels: pixels.to_vec(),
        });
    }
    Ok(())
}

impl PreviewWindow {
    fn draw(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        generic: Option<&TgaImage>,
        miniature: bool,
    ) {
        if rect.w == 0 || rect.h == 0 {
            return;
        }
        canvas.fill_rect(rect, theme.panel_alt);
        if let Some(image) = &self.image {
            let image_rect = fit_rect(rect.inset(if miniature { 0 } else { 2 }), image.w, image.h);
            for y in 0..image_rect.h {
                let sy = y * image.h / image_rect.h;
                for x in 0..image_rect.w {
                    let sx = x * image.w / image_rect.w;
                    canvas.put_pixel(
                        image_rect.x + x as i32,
                        image_rect.y + y as i32,
                        Color(image.pixels[(sy * image.w + sx) as usize]),
                    );
                }
            }
        } else if let Some(icon) = self.icon.as_ref().or(generic) {
            let size = rect.w.min(rect.h).min(if miniature { 8 } else { 44 });
            canvas.draw_tga_icon(
                icon,
                Rect::new(
                    rect.x + ((rect.w - size) / 2) as i32,
                    rect.y + ((rect.h - size) / 2) as i32,
                    size,
                    size,
                ),
            );
        }
        canvas.draw_rect(rect, theme.border);
    }
}

/// Session-local Workspace Switcher state. Bounded; no timers or heap growth.
pub(crate) struct WorkspaceSwitcherState {
    open: bool,
    /// Keyboard focus index into cards (`0..4`).
    focus_index: usize,
    hover: Option<usize>,
    cards: [WorkspaceCardSummary; WORKSPACE_CARD_COUNT],
    windows: Vec<PreviewWindow>,
    /// Short failure status (`"Switch failed"`) or empty.
    status: [u8; 24],
    status_len: u8,
}

impl WorkspaceSwitcherState {
    pub(crate) const fn new() -> Self {
        Self {
            open: false,
            focus_index: 0,
            windows: Vec::new(),
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
        self.windows.clear();
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

    pub(crate) fn capture(
        &mut self,
        ep: CapabilityToken,
        windows: &[WindowSnapshot],
        apps: &[DockAppState],
    ) {
        self.windows.clear();
        let max_bytes = SgpMsg::WORKSPACE_PREVIEW_TITLE_BYTES
            + (SgpMsg::WORKSPACE_PREVIEW_MAX_W * SgpMsg::WORKSPACE_PREVIEW_MAX_H * 4) as usize;
        let mut scratch = shm_create(max_bytes, 0).ok();
        for window in windows.iter().filter(|w| is_switcher_window(w)) {
            let icon = match app_icon_for_window(window, apps) {
                WorkspaceAppIcon::App(app) => crate::VortexShell::static_icon_for_app(app),
                WorkspaceAppIcon::Generic => None,
            };
            let mut title = [0; SgpMsg::WORKSPACE_PREVIEW_TITLE_BYTES];
            title[..window.title.len()].copy_from_slice(&window.title);
            let mut preview = PreviewWindow {
                id: window.id,
                workspace: window.workspace_id as u8,
                geometry: Rect::new(0, 0, 4, 3),
                title,
                icon,
                image: None,
            };
            if let Some((buf, cap)) = scratch {
                if read_preview(ep, &mut preview, buf, cap).is_err() {
                    let _ = shm_free(cap);
                    scratch = None;
                }
            }
            self.windows.push(preview);
        }
        if let Some((_, cap)) = scratch {
            let _ = shm_free(cap);
        }
    }

    pub(crate) fn remove_closed(&mut self, windows: &[WindowSnapshot]) -> bool {
        let count = self.windows.len();
        self.windows.retain(|preview| {
            windows.iter().any(|win| {
                win.id == preview.id
                    && win.workspace_id == u64::from(preview.workspace)
                    && is_switcher_window(win)
            })
        });
        self.windows.len() != count
    }

    pub(crate) fn picker_rect(&self, w: u32, h: u32, top: i32, dock: i32) -> Option<Rect> {
        if !self.open {
            return None;
        }
        let strip = self.layout(w, h, top, dock).panel;
        let bottom = strip.y - 12;
        let top = top + 8;
        (bottom > top + 52).then(|| Rect::new(strip.x, top, strip.w, (bottom - top) as u32))
    }

    /// Grid cells never intersect. Fit each window into its cell using its
    /// actual outer aspect ratio and reserve a separate line for the title.
    fn tiles(&self, area: Rect) -> impl Iterator<Item = (&PreviewWindow, Rect)> {
        let workspace = index_to_workspace_id(self.focus_index);
        let windows = self
            .windows
            .iter()
            .filter(move |win| win.workspace == workspace);
        let count = windows.clone().count() as u32;
        let cols = (1..=count).find(|cols| cols * cols >= count).unwrap_or(1);
        let rows = count.div_ceil(cols).max(1);
        let cell_w = area.w / cols;
        let cell_h = area.h / rows;
        windows.enumerate().map(move |(index, win)| {
            let index = index as u32;
            let cell = Rect::new(
                area.x + ((index % cols) * cell_w) as i32,
                area.y + ((index / cols) * cell_h) as i32,
                cell_w,
                cell_h,
            );
            (win, cell)
        })
    }

    fn tile_image(tile: Rect, window: &PreviewWindow) -> Rect {
        let padded = tile.inset(4);
        fit_rect(
            Rect::new(padded.x, padded.y, padded.w, padded.h.saturating_sub(14)),
            window.geometry.w.max(1),
            window.geometry.h.max(1),
        )
    }

    fn tile_area(picker: Rect) -> Rect {
        Rect::new(
            picker.x + 10,
            picker.y + 26,
            picker.w.saturating_sub(20),
            picker.h.saturating_sub(34),
        )
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

    /// Panel rectangle published to the compositor so the switcher renders
    /// above application windows instead of inside the desktop layer.
    pub(crate) fn overlay_rect(
        &self,
        screen_w: u32,
        screen_h: u32,
        top_inset: i32,
        dock_top_y: i32,
    ) -> Option<Rect> {
        if !self.open {
            return None;
        }
        let panel = self.layout(screen_w, screen_h, top_inset, dock_top_y).panel;
        Some(
            self.picker_rect(screen_w, screen_h, top_inset, dock_top_y)
                .map_or(panel, |picker| {
                    Rect::new(
                        panel.x,
                        picker.y,
                        panel.w,
                        (panel.bottom() - picker.y) as u32,
                    )
                }),
        )
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
            && (self
                .layout(screen_w, screen_h, top_inset, dock_top_y)
                .contains(point)
                || self
                    .picker_rect(screen_w, screen_h, top_inset, dock_top_y)
                    .is_some_and(|rect| rect.contains(point)))
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
                if !self.contains(p, screen_w, screen_h, top_inset, dock_top_y) {
                    return (true, WorkspaceSwitcherAction::Close);
                }
                if let Some(idx) = layout.card_index_at(p) {
                    self.focus_index = idx;
                }
                if let Some(picker) = self.picker_rect(screen_w, screen_h, top_inset, dock_top_y) {
                    if let Some((win, _)) = self
                        .tiles(Self::tile_area(picker))
                        .find(|(win, cell)| Self::tile_image(*cell, win).contains(p))
                    {
                        return (true, WorkspaceSwitcherAction::ActivateWindow(win.id));
                    }
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
                if !self.contains(p, screen_w, screen_h, top_inset, dock_top_y) {
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
                let action = self
                    .windows
                    .iter()
                    .rev()
                    .find(|win| win.workspace == ws)
                    .map_or(WorkspaceSwitcherAction::None, |win| {
                        WorkspaceSwitcherAction::ActivateWindow(win.id)
                    });
                (true, action)
            }
            KEY_1 | KEY_2 | KEY_3 | KEY_4 => {
                self.focus_index = usize::from(keycode - KEY_1);
                (true, WorkspaceSwitcherAction::None)
            }
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
        let desktop = Rect::new(0, 0, screen_w, screen_h);
        for (i, card) in layout.cards.into_iter().enumerate() {
            let miniature = fit_rect(
                WorkspaceSwitcherLayout::preview_rect(card).inset(2),
                screen_w,
                screen_h,
            );
            for window in self
                .windows
                .iter()
                .filter(|w| usize::from(w.workspace) == i + 1)
            {
                if let Some(rect) = miniature_geometry(window.geometry, desktop, miniature) {
                    window.draw(canvas, theme, rect, generic_icon.as_ref(), true);
                }
            }
        }
        if let Some(picker) = self.picker_rect(screen_w, screen_h, top_inset, dock_top_y) {
            canvas.fill_rounded_rect(picker, 12, theme.panel);
            canvas.stroke_rounded_rect(picker, 12, 1, theme.border);
            canvas.draw_text(
                picker.x + 12,
                picker.y + 10,
                WORKSPACE_TITLES[self.focus_index],
                theme.accent,
            );
            let area = Self::tile_area(picker);
            let mut empty = true;
            for (window, cell) in self.tiles(area) {
                empty = false;
                window.draw(
                    canvas,
                    theme,
                    Self::tile_image(cell, window),
                    generic_icon.as_ref(),
                    false,
                );
                let end = window
                    .title
                    .iter()
                    .position(|b| *b == 0)
                    .unwrap_or(window.title.len());
                let title = core::str::from_utf8(&window.title[..end]).unwrap_or("");
                let title = if title.is_empty() { "Window" } else { title };
                let text: alloc::string::String =
                    title.chars().take((cell.w / 8) as usize).collect();
                canvas.draw_text_centered(
                    Rect::new(cell.x, cell.bottom() - 14, cell.w, 14),
                    &text,
                    theme.text,
                );
            }
            if empty {
                canvas.draw_text_centered(area, "No windows", theme.text_muted);
            }
        }
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
    fn overlay_rect_tracks_panel_and_clears_when_closed() {
        let mut sw = WorkspaceSwitcherState::new();
        assert!(sw.overlay_rect(1366, 768, 48, 716).is_none());
        sw.open(1);
        let overlay = sw.overlay_rect(1366, 768, 48, 716).unwrap();
        assert_eq!(
            overlay.intersect(sw.layout(1366, 768, 48, 716).panel),
            Some(sw.layout(1366, 768, 48, 716).panel)
        );
        let picker = sw.picker_rect(1366, 768, 48, 716).unwrap();
        assert_eq!(overlay.intersect(picker), Some(picker));
        sw.close();
        assert!(sw.overlay_rect(1366, 768, 48, 716).is_none());
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
        assert_eq!(a, WorkspaceSwitcherAction::None);
        assert_eq!(sw.focus_index(), 0);

        let (_, a) = sw.handle_event(
            Event::key_press(KEY_ENTER, true, false, false, false, false),
            1366,
            768,
            48,
            716,
        );
        assert_eq!(a, WorkspaceSwitcherAction::None);
    }

    #[test]
    fn selecting_card_browses_without_activation() {
        let mut sw = WorkspaceSwitcherState::new();
        sw.open(1);
        let layout = sw.layout(1366, 768, 48, 716);
        let c = layout.cards[3];
        let (_, action) = sw.handle_event(Event::click(c.x + 2, c.y + 2), 1366, 768, 48, 716);
        assert_eq!(action, WorkspaceSwitcherAction::None);
        assert_eq!(sw.focus_index(), 3);
    }

    #[test]
    fn picker_layout_and_click_use_browsed_window_identity() {
        let mut sw = WorkspaceSwitcherState::new();
        sw.open(1);
        for (id, w, h) in [(11, 600, 300), (12, 300, 600), (13, 600, 600)] {
            sw.windows.push(PreviewWindow {
                id,
                workspace: 3,
                geometry: Rect::new(20, 30, w, h),
                title: [0; SgpMsg::WORKSPACE_PREVIEW_TITLE_BYTES],
                icon: None,
                image: None,
            });
        }
        let picker = sw.picker_rect(1366, 768, 48, 716).unwrap();
        assert_eq!(
            sw.tiles(WorkspaceSwitcherState::tile_area(picker)).count(),
            0
        );
        let c = sw.layout(1366, 768, 48, 716).cards[2];
        let (_, action) = sw.handle_event(Event::click(c.x + 2, c.y + 2), 1366, 768, 48, 716);
        assert_eq!(action, WorkspaceSwitcherAction::None);
        let tiles: Vec<_> = sw
            .tiles(WorkspaceSwitcherState::tile_area(picker))
            .map(|(win, cell)| (win.id, WorkspaceSwitcherState::tile_image(cell, win)))
            .collect();
        assert_eq!(tiles.len(), 3);
        assert!((tiles[0].1.w as i32 - 2 * tiles[0].1.h as i32).abs() <= 1);
        assert!((tiles[1].1.h as i32 - 2 * tiles[1].1.w as i32).abs() <= 1);
        for (index, (_, tile)) in tiles.iter().enumerate() {
            for (_, other) in &tiles[index + 1..] {
                assert!(tile.intersect(*other).is_none());
            }
        }
        let (id, first) = tiles[0];
        let (_, action) =
            sw.handle_event(Event::click(first.x + 2, first.y + 2), 1366, 768, 48, 716);
        assert_eq!(action, WorkspaceSwitcherAction::ActivateWindow(id));
        assert!(sw.close());
        assert!(sw.windows.is_empty());
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
