//! Login & Session Control Panel page — Startup Apps configuration.
//!
//! All mutations go through sunlight-sessiond IPC. This page never launches
//! applications and never edits the immutable system session manifest.

use core::fmt::Write;

use sun_font::{self, FontRole, TextStyle, Typography};
use sunlight_ipc::{
    ipc_call, nameserver_lookup, IpcMsg, SessionMsg, SESSION_ENDPOINT,
};
use sunlight_ui::{
    widgets::{Button, ButtonState},
    Canvas, Color, Event, MaterialPalette, Point, Rect, Theme,
};

fn draw_button(canvas: &mut Canvas, theme: &Theme, button: Button<'_>) {
    button.with_font(&Typography::UI_MEDIUM).draw(canvas, theme);
}

fn draw_text(
    canvas: &mut Canvas,
    rect: Rect,
    text: &str,
    theme: &Theme,
    role: FontRole,
    color: Color,
) {
    let _ = theme;
    sun_font::draw_text_vcenter(
        canvas,
        text,
        rect.x,
        rect.y,
        rect.h,
        &TextStyle::new(role, color),
    );
}

use crate::sysinfo::FixedStr;

const KEY_ESC: u8 = 0x01;
const MAX_ROWS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SessionAction {
    None,
    Back,
}

#[derive(Clone, Copy)]
struct EntryRow {
    app_id: FixedStr<32>,
    enabled: bool,
    policy: u8,
    order: u16,
}

pub struct SessionPageState {
    revision: u64,
    entry_count: u16,
    entries: [EntryRow; MAX_ROWS],
    selected: usize,
    status: FixedStr<80>,
    add_mode: bool,
    eligible_count: u16,
    eligible: [FixedStr<32>; MAX_ROWS],
    dirty_note: bool,
}

impl SessionPageState {
    pub fn new() -> Self {
        Self {
            revision: 0,
            entry_count: 0,
            entries: [EntryRow {
                app_id: FixedStr::empty(),
                enabled: false,
                policy: 1,
                order: 0,
            }; MAX_ROWS],
            selected: 0,
            status: FixedStr::empty(),
            add_mode: false,
            eligible_count: 0,
            eligible: [FixedStr::empty(); MAX_ROWS],
            dirty_note: false,
        }
    }

    pub fn activate(&mut self) -> bool {
        self.refresh();
        true
    }

    fn ep() -> Option<sunlight_ipc::CapabilityToken> {
        nameserver_lookup(SESSION_ENDPOINT)
    }

    fn pack_app(msg: &mut IpcMsg, app_id: &str) {
        let bytes = app_id.as_bytes();
        msg.words[2] = 0;
        msg.words[3] = 0;
        for (i, b) in bytes.iter().take(16).enumerate() {
            if i < 8 {
                msg.words[2] |= (*b as u64) << (i * 8);
            } else {
                msg.words[3] |= (*b as u64) << ((i - 8) * 8);
            }
        }
    }

    fn unpack_app(msg: &IpcMsg, len: usize) -> FixedStr<32> {
        let mut out = FixedStr::empty();
        let mut buf = [0u8; 32];
        let len = len.min(16);
        for i in 0..len {
            let b = if i < 8 {
                ((msg.words[2] >> (i * 8)) & 0xff) as u8
            } else {
                ((msg.words[3] >> ((i - 8) * 8)) & 0xff) as u8
            };
            if b == 0 {
                break;
            }
            buf[i] = b;
        }
        if let Ok(s) = core::str::from_utf8(&buf[..len]) {
            out.set(s);
        }
        out
    }

    pub fn refresh(&mut self) {
        let Some(ep) = Self::ep() else {
            self.status.set("Session service unavailable");
            return;
        };
        let summary = ipc_call(ep, IpcMsg::with_label(SessionMsg::SESSION_PROFILE_GET).word(0, 0));
        if summary.label != SessionMsg::REPLY {
            self.status.set("Could not load session profile");
            return;
        }
        self.revision = summary.words[0];
        self.entry_count = (summary.words[2] & 0xffff) as u16;
        for i in 0..MAX_ROWS {
            self.entries[i].app_id.clear();
            self.entries[i].enabled = false;
        }
        for index in 0..self.entry_count.min(MAX_ROWS as u16) {
            let reply = ipc_call(
                ep,
                IpcMsg::with_label(SessionMsg::SESSION_PROFILE_UPDATE)
                    .word(0, 0)
                    .word(1, index as u64),
            );
            if reply.label != SessionMsg::REPLY {
                break;
            }
            let app_len = ((reply.words[1] >> 8) & 0xff) as usize;
            self.entries[index as usize] = EntryRow {
                app_id: Self::unpack_app(&reply, app_len),
                enabled: ((reply.words[1] >> 16) & 1) != 0,
                policy: ((reply.words[1] >> 24) & 0xff) as u8,
                order: ((reply.words[1] >> 32) & 0xffff) as u16,
            };
        }
        self.status.set("Changes apply at your next login.");
    }

    fn mutate(&mut self, op: u64, app_id: &str, policy: u8, direction: u8) {
        let Some(ep) = Self::ep() else {
            self.status.set("Session service unavailable");
            return;
        };
        let mut msg = IpcMsg::with_label(op)
            .word(
                0,
                (app_id.len().min(16) as u64) << 32
                    | ((policy as u64) << 40)
                    | ((direction as u64) << 48),
            )
            .word(1, self.revision);
        Self::pack_app(&mut msg, app_id);
        let reply = ipc_call(ep, msg);
        if reply.label != SessionMsg::REPLY {
            if reply.words[0] == SessionMsg::ERR_PROFILE_REVISION {
                self.status.set("Revision conflict — reloading");
            } else {
                self.status.set("Update failed");
            }
            self.refresh();
            return;
        }
        self.dirty_note = true;
        self.refresh();
        self.status.set("Saved. Changes apply at your next login.");
    }

    fn load_eligible(&mut self) {
        let Some(ep) = Self::ep() else {
            return;
        };
        self.eligible_count = 0;
        for index in 0..MAX_ROWS as u64 {
            let reply = ipc_call(
                ep,
                IpcMsg::with_label(SessionMsg::SESSION_PROFILE_LIST_ELIGIBLE_APPS)
                    .word(0, 0)
                    .word(1, index),
            );
            if reply.label != SessionMsg::REPLY {
                break;
            }
            let total = (reply.words[1] & 0xffff) as u16;
            let configured = ((reply.words[1] >> 32) & 1) != 0;
            if configured {
                if index + 1 >= total as u64 {
                    break;
                }
                continue;
            }
            let app_len = ((reply.words[1] >> 48) & 0xff) as usize;
            if (self.eligible_count as usize) < MAX_ROWS {
                self.eligible[self.eligible_count as usize] = Self::unpack_app(&reply, app_len);
                self.eligible_count = self.eligible_count.saturating_add(1);
            }
            if index + 1 >= total as u64 {
                break;
            }
        }
    }

    pub fn draw(&mut self, canvas: &mut Canvas, theme: &Theme, win_w: u32, win_h: u32) {
        canvas.clear_transparent(Rect::new(0, 0, win_w, win_h));
        draw_text(
            canvas,
            Rect::new(16, 12, win_w - 32, 22),
            "Login & Session",
            theme,
            FontRole::UiTitle,
            theme.text,
        );
        draw_text(
            canvas,
            Rect::new(16, 38, win_w - 32, 18),
            "Startup Apps",
            theme,
            FontRole::UiRegular,
            theme.text,
        );

        // Required components section
        canvas.fill_material(
            Rect::new(12, 64, win_w - 24, 56),
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(8)
                .without_border(),
        );
        draw_text(
            canvas,
            Rect::new(24, 70, win_w - 48, 16),
            "Required Session Components",
            theme,
            FontRole::UiSmall,
            theme.text_dim,
        );
        draw_text(
            canvas,
            Rect::new(24, 90, win_w - 48, 18),
            "Vortex Shell — Required · Always starts · Managed by SunlightOS",
            theme,
            FontRole::UiRegular,
            theme.text,
        );

        // Optional section
        let list_top = 132i32;
        draw_text(
            canvas,
            Rect::new(16, list_top, win_w - 32, 16),
            "Optional Startup Applications",
            theme,
            FontRole::UiSmall,
            theme.text_dim,
        );

        if self.add_mode {
            draw_text(
                canvas,
                Rect::new(16, list_top + 22, win_w - 32, 16),
                "Add Startup App — select an eligible installed application",
                theme,
                FontRole::UiRegular,
                theme.text,
            );
            if self.eligible_count == 0 {
                draw_text(
                    canvas,
                    Rect::new(24, list_top + 48, win_w - 48, 18),
                    "No additional eligible applications.",
                    theme,
                    FontRole::UiRegular,
                    theme.text_dim,
                );
            } else {
                for i in 0..self.eligible_count as usize {
                    let y = list_top + 44 + (i as i32) * 28;
                    if i == self.selected {
                        canvas.fill_rect(Rect::new(16, y, win_w - 32, 26), theme.panel);
                    }
                    draw_text(
                        canvas,
                        Rect::new(24, y + 4, win_w - 48, 18),
                        self.eligible[i].as_str(),
                        theme,
                        FontRole::UiRegular,
                        theme.text,
                    );
                }
            }
        } else if self.entry_count == 0 {
            draw_text(
                canvas,
                Rect::new(24, list_top + 28, win_w - 48, 36),
                "No optional applications start automatically.",
                theme,
                FontRole::UiRegular,
                theme.text_dim,
            );
        } else {
            for i in 0..self.entry_count as usize {
                let y = list_top + 24 + (i as i32) * 36;
                if i == self.selected {
                    canvas.fill_rect(Rect::new(16, y, win_w - 32, 34), theme.panel);
                }
                let e = &self.entries[i];
                let mut buf = FixedStr::<96>::empty();
                let _ = write!(
                    &mut buf,
                    "{}  {}  order={}  {}",
                    e.app_id.as_str(),
                    if e.enabled { "on" } else { "off" },
                    e.order,
                    match e.policy {
                        1 => "every-login",
                        2 => "first-login-only",
                        3 => "after-upgrade",
                        4 => "disabled",
                        _ => "?",
                    }
                );
                draw_text(
                    canvas,
                    Rect::new(24, y + 8, win_w - 48, 18),
                    buf.as_str(),
                    theme,
                    FontRole::UiRegular,
                    theme.text,
                );
            }
        }

        // Status
        draw_text(
            canvas,
            Rect::new(16, win_h as i32 - 72, win_w - 32, 16),
            self.status.as_str(),
            theme,
            FontRole::UiSmall,
            theme.text_dim,
        );
        if self.dirty_note {
            draw_text(
                canvas,
                Rect::new(16, win_h as i32 - 56, win_w - 32, 16),
                "Applies next login",
                theme,
                FontRole::UiSmall,
                theme.accent,
            );
        }

        // Buttons
        let by = win_h as i32 - 40;
        draw_button(
            canvas,
            theme,
            Button::secondary(Rect::new(12, by, 72, 28), "Back"),
        );
        if self.add_mode {
            draw_button(
                canvas,
                theme,
                Button::secondary(Rect::new(92, by, 80, 28), "Cancel"),
            );
            draw_button(canvas, theme, Button::new(Rect::new(180, by, 72, 28), "Add"));
        } else {
            draw_button(canvas, theme, Button::new(Rect::new(92, by, 72, 28), "Add"));
            draw_button(
                canvas,
                theme,
                Button::secondary(Rect::new(172, by, 80, 28), "Remove"),
            );
            draw_button(
                canvas,
                theme,
                Button::secondary(Rect::new(260, by, 80, 28), "Toggle"),
            );
            draw_button(
                canvas,
                theme,
                Button::secondary(Rect::new(348, by, 56, 28), "Up"),
            );
            draw_button(
                canvas,
                theme,
                Button::secondary(Rect::new(412, by, 64, 28), "Down"),
            );
        }
        let _ = ButtonState::Normal;
    }

    pub fn update(&mut self, event: Event, win_w: u32, win_h: u32) -> SessionAction {
        let by = win_h as i32 - 40;
        match event {
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } if keycode == KEY_ESC => {
                if self.add_mode {
                    self.add_mode = false;
                    return SessionAction::None;
                }
                return SessionAction::Back;
            }
            Event::Click { x, y } => {
                let pt = Point::new(x, y);
                if Rect::new(12, by, 72, 28).contains(pt) {
                    return SessionAction::Back;
                }
                if self.add_mode {
                    if Rect::new(92, by, 80, 28).contains(pt) {
                        self.add_mode = false;
                        return SessionAction::None;
                    }
                    if Rect::new(180, by, 72, 28).contains(pt)
                        && self.selected < self.eligible_count as usize
                    {
                        let mut owned = FixedStr::<32>::empty();
                        owned.set(self.eligible[self.selected].as_str());
                        self.add_mode = false;
                        self.mutate(SessionMsg::SESSION_PROFILE_ADD_APP, owned.as_str(), 1, 0);
                        return SessionAction::None;
                    }
                    // Select eligible row
                    let list_top = 132i32;
                    for i in 0..self.eligible_count as usize {
                        let y = list_top + 44 + (i as i32) * 28;
                        if Rect::new(16, y, win_w - 32, 26).contains(pt) {
                            self.selected = i;
                            return SessionAction::None;
                        }
                    }
                } else {
                    if Rect::new(92, by, 72, 28).contains(pt) {
                        self.load_eligible();
                        self.add_mode = true;
                        self.selected = 0;
                        return SessionAction::None;
                    }
                    if Rect::new(172, by, 80, 28).contains(pt) && self.entry_count > 0 {
                        let mut owned = FixedStr::<32>::empty();
                        owned.set(
                            self.entries[self.selected.min(self.entry_count as usize - 1)]
                                .app_id
                                .as_str(),
                        );
                        self.mutate(
                            SessionMsg::SESSION_PROFILE_REMOVE_APP,
                            owned.as_str(),
                            0,
                            0,
                        );
                        return SessionAction::None;
                    }
                    if Rect::new(260, by, 80, 28).contains(pt) && self.entry_count > 0 {
                        let idx = self.selected.min(self.entry_count as usize - 1);
                        let enabled = self.entries[idx].enabled;
                        let mut owned = FixedStr::<32>::empty();
                        owned.set(self.entries[idx].app_id.as_str());
                        let op = if enabled {
                            SessionMsg::SESSION_PROFILE_DISABLE_APP
                        } else {
                            SessionMsg::SESSION_PROFILE_ENABLE_APP
                        };
                        self.mutate(op, owned.as_str(), 0, 0);
                        return SessionAction::None;
                    }
                    if Rect::new(348, by, 56, 28).contains(pt) && self.entry_count > 0 {
                        let mut owned = FixedStr::<32>::empty();
                        owned.set(
                            self.entries[self.selected.min(self.entry_count as usize - 1)]
                                .app_id
                                .as_str(),
                        );
                        self.mutate(SessionMsg::SESSION_PROFILE_REORDER, owned.as_str(), 0, 0);
                        return SessionAction::None;
                    }
                    if Rect::new(412, by, 64, 28).contains(pt) && self.entry_count > 0 {
                        let mut owned = FixedStr::<32>::empty();
                        owned.set(
                            self.entries[self.selected.min(self.entry_count as usize - 1)]
                                .app_id
                                .as_str(),
                        );
                        self.mutate(SessionMsg::SESSION_PROFILE_REORDER, owned.as_str(), 0, 1);
                        return SessionAction::None;
                    }
                    let list_top = 132i32;
                    for i in 0..self.entry_count as usize {
                        let y = list_top + 24 + (i as i32) * 36;
                        if Rect::new(16, y, win_w - 32, 34).contains(pt) {
                            self.selected = i;
                            return SessionAction::None;
                        }
                    }
                }
            }
            _ => {}
        }
        SessionAction::None
    }
}
