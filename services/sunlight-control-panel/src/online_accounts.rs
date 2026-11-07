//! Token-free Online Accounts client: only metadata and high-level broker commands.
use sun_font::{FontRole, TextStyle, Typography};
use sunlight_ipc::{accounts::text, online_accounts as api, *};
use sunlight_ui::{widgets::Button, Canvas, Color, Event, Point, Rect, Theme};

fn call(msg: IpcMsg) -> Result<IpcMsg, &'static str> {
    let ep = nameserver_lookup_timeout("sunlight-account-broker", 500)
        .ok_or("Account broker unavailable. Try Refresh.")?;
    let r =
        ipc_call_timeout(ep, msg, 1500).map_err(|_| "Account broker timed out. Try Refresh.")?;
    if r.label != api::REPLY {
        return Err("Invalid account broker response.");
    }
    match r.words[0] {
        0 => Ok(r),
        1 => Err("Account management denied for this application."),
        2 => {
            Err("Provider not configured. Install a Microsoft Client ID or use a mock test image.")
        }
        7 => Err("Live-session account storage unavailable. Try Refresh."),
        _ => Err("Could not start authorization. Check network and provider configuration."),
    }
}
fn label(c: &mut Canvas, t: &Theme, r: Rect, s: &str) {
    let mut clipped = c.sub_canvas(r);
    sun_font::draw_text_vcenter(
        &mut clipped,
        s,
        0,
        0,
        r.h,
        &TextStyle::new(FontRole::UiRegular, t.text),
    );
}
fn button(c: &mut Canvas, t: &Theme, r: Rect, s: &str) {
    Button::new(r, s)
        .with_font(&Typography::UI_MEDIUM)
        .draw(c, t);
}
fn microsoft(c: &mut Canvas, x: i32, y: i32) {
    for (dx, dy, color) in [
        (0, 0, Color::rgb(242, 80, 34)),
        (15, 0, Color::rgb(127, 186, 0)),
        (0, 15, Color::rgb(0, 164, 239)),
        (15, 15, Color::rgb(255, 185, 0)),
    ] {
        c.draw_rect(Rect::new(x + dx, y + dy, 13, 13), color);
    }
}
fn outcome(status: u64) -> &'static str {
    match status {
        1 => "Account connected in this live session.",
        2 => "Sign-in rejected: callback state mismatch. Try again.",
        3 => "Sign-in timed out. Check your network and try again.",
        4 => "Sign-in cancelled. No account was added.",
        5 => "The provider rejected sign-in or permission was declined.",
        7 => "Could not save account to live-session storage. Try again.",
        _ => "Network or callback failure. Check connectivity and try again.",
    }
}

pub struct OnlineAccounts {
    snapshot: api::Snapshot,
    mode: u64,
    flow: Option<u64>,
    providers: bool,
    status: &'static str,
    next_poll: u64,
    flow_deadline: u64,
    offset: usize,
}
impl OnlineAccounts {
    pub fn new() -> Self {
        Self {
            snapshot: api::Snapshot::EMPTY,
            mode: 0,
            flow: None,
            providers: false,
            status: "Loading Online Accounts...",
            next_poll: 0,
            flow_deadline: 0,
            offset: 0,
        }
    }
    pub fn refresh(&mut self) -> bool {
        let result = (|| {
            self.mode = call(IpcMsg::with_label(api::STATUS))?.words[1];
            let (p, t) = shm_alloc().map_err(|_| "Could not load account metadata.")?;
            unsafe {
                core::ptr::write_bytes(p, 0, 4096);
            }
            let r = call(IpcMsg::with_label(api::LIST_ACCOUNTS).with_cap(0, t));
            if r.is_ok() {
                let s = unsafe { core::ptr::read_unaligned(p.cast::<api::Snapshot>()) };
                if s.count <= api::MAX_ACCOUNTS as u64 {
                    self.snapshot = s;
                } else {
                    let _ = shm_free(t);
                    return Err("Invalid account metadata.");
                }
            }
            let _ = shm_free(t);
            r.map(|_| ())
        })();
        match result {
            Ok(()) => {
                if self.status == "Loading Online Accounts..." {
                    self.status = "";
                }
            }
            Err(e) => {
                self.snapshot = api::Snapshot::EMPTY;
                self.status = e;
            }
        }
        self.next_poll = monotonic_millis() + 1500;
        true
    }
    fn start(&mut self) {
        if self.mode == 0 {
            self.status = "Provider not configured. A Microsoft Client ID is required.";
            return;
        }
        match call(IpcMsg::with_label(api::START)) {
            Ok(r) => {
                self.flow = Some(r.words[1]);
                self.flow_deadline = monotonic_millis() + 135_000;
                self.providers = false;
                self.status = "Finish sign-in in Rappid Rabbit, or Cancel here.";
                debug_log("[ONLINE-ACCOUNTS-UI] authorization started");
            }
            Err(e) => self.status = e,
        }
    }
    fn disconnect(&mut self, index: usize) {
        let Ok((p, t)) = shm_alloc() else {
            self.status = "Could not disconnect account.";
            return;
        };
        unsafe {
            core::ptr::copy_nonoverlapping(self.snapshot.accounts[index].id.as_ptr(), p, 48);
        }
        let r = call(IpcMsg::with_label(api::DISCONNECT).with_cap(0, t));
        let _ = shm_free(t);
        match r {
            Ok(_) => {
                self.status = "Account disconnected. Session data removed.";
                self.refresh();
                debug_log("[ONLINE-ACCOUNTS-UI] disconnected list refreshed");
            }
            Err(e) => self.status = e,
        }
    }
    pub fn tick(&mut self) -> bool {
        if monotonic_millis() < self.next_poll {
            return false;
        }
        self.next_poll = monotonic_millis() + 1000;
        if let Some(id) = self.flow {
            match call(IpcMsg::with_label(api::FLOW_STATUS).word(0, id)) {
                Ok(r) if r.words[1] == 0 => return false,
                Ok(r) => {
                    self.flow = None;
                    self.status = outcome(r.words[1]);
                    self.refresh();
                    debug_log("[ONLINE-ACCOUNTS-UI] authorization result; list refreshed");
                }
                Err(e) => {
                    if monotonic_millis() >= self.flow_deadline {
                        self.flow = None;
                    }
                    self.status = e;
                }
            }
            true
        } else {
            false
        }
    }
    fn visible(h: u32) -> usize {
        (h.saturating_sub(240) / 76).max(1) as usize
    }
    pub fn draw(&self, c: &mut Canvas, t: &Theme, w: u32, h: u32) {
        c.clear_transparent(Rect::new(0, 0, w, h));
        button(c, t, Rect::new(12, 10, 70, 30), "Back");
        label(
            c,
            t,
            Rect::new(96, 10, w.saturating_sub(100), 30),
            "Online Accounts",
        );
        label(
            c,
            t,
            Rect::new(20, 54, w.saturating_sub(40), 24),
            "Accounts are saved in RAM for this live session.",
        );
        label(
            c,
            t,
            Rect::new(20, 82, w.saturating_sub(40), 24),
            match self.mode {
                1 => "Microsoft  |  Mock test mode - provider not configured",
                2 => "Microsoft  |  Provider configured",
                _ => "Microsoft  |  Provider not configured",
            },
        );
        button(
            c,
            t,
            Rect::new(20, 118, 140, 32),
            if self.flow.is_some() {
                "Cancel sign-in"
            } else {
                "Add Account"
            },
        );
        button(c, t, Rect::new(174, 118, 90, 32), "Refresh");
        if self.providers {
            microsoft(c, 24, 174);
            button(
                c,
                t,
                Rect::new(68, 170, 240, 38),
                if self.mode == 1 {
                    "Microsoft (Mock test mode)"
                } else {
                    "Microsoft"
                },
            );
        } else if self.snapshot.count == 0 {
            label(
                c,
                t,
                Rect::new(24, 174, w.saturating_sub(48), 28),
                "No online accounts. Add a Microsoft account to get started.",
            );
        } else {
            for (row, index) in (self.offset..self.snapshot.count as usize)
                .take(Self::visible(h))
                .enumerate()
            {
                let a = &self.snapshot.accounts[index];
                let y = 170 + row as i32 * 76;
                microsoft(c, 24, y + 10);
                label(
                    c,
                    t,
                    Rect::new(68, y, w.saturating_sub(200), 24),
                    "Microsoft",
                );
                label(
                    c,
                    t,
                    Rect::new(68, y + 23, w.saturating_sub(200), 22),
                    text(&a.identifier),
                );
                label(
                    c,
                    t,
                    Rect::new(68, y + 45, w.saturating_sub(200), 22),
                    if a.active == 1 {
                        "Connected / Active"
                    } else {
                        "Expired - reconnect required"
                    },
                );
                button(
                    c,
                    t,
                    Rect::new(w.saturating_sub(124) as i32, y + 14, 108, 32),
                    "Disconnect",
                );
            }
        }
        if self.snapshot.count as usize > Self::visible(h) {
            button(
                c,
                t,
                Rect::new(
                    w.saturating_sub(180) as i32,
                    h.saturating_sub(64) as i32,
                    72,
                    26,
                ),
                "Prev",
            );
            button(
                c,
                t,
                Rect::new(
                    w.saturating_sub(96) as i32,
                    h.saturating_sub(64) as i32,
                    72,
                    26,
                ),
                "Next",
            );
        }
        // Wrap status to the actual panel width so errors remain legible.
        let split = self
            .status
            .char_indices()
            .filter(|(i, c)| {
                *c == ' '
                    && sun_font::measure_text(&self.status[..*i], FontRole::UiRegular).w
                        <= w.saturating_sub(40)
            })
            .last()
            .map_or(self.status.len(), |(i, _)| i);
        label(
            c,
            t,
            Rect::new(20, h.saturating_sub(48) as i32, w.saturating_sub(40), 20),
            &self.status[..split],
        );
        label(
            c,
            t,
            Rect::new(20, h.saturating_sub(27) as i32, w.saturating_sub(40), 20),
            self.status[split..].trim(),
        );
    }
    pub fn update(&mut self, event: Event, w: u32, h: u32) -> bool {
        match event {
            Event::Tick => {
                self.tick();
            }
            Event::Click { x, y } => {
                let p = Point::new(x, y);
                if Rect::new(12, 10, 70, 30).contains(p) {
                    return true;
                }
                if Rect::new(20, 118, 140, 32).contains(p) {
                    if let Some(id) = self.flow {
                        match call(IpcMsg::with_label(api::CANCEL).word(0, id)) {
                            Ok(r) => {
                                self.flow = None;
                                self.status = outcome(r.words[1]);
                                if r.words[1] == 1 {
                                    self.refresh();
                                }
                            }
                            Err(e) => self.status = e,
                        }
                    } else {
                        self.providers = !self.providers;
                    }
                } else if Rect::new(174, 118, 90, 32).contains(p) {
                    self.refresh();
                } else if self.providers && Rect::new(68, 170, 240, 38).contains(p) {
                    self.start();
                } else if !self.providers {
                    for (row, index) in (self.offset..self.snapshot.count as usize)
                        .take(Self::visible(h))
                        .enumerate()
                    {
                        if Rect::new(w.saturating_sub(124) as i32, 184 + row as i32 * 76, 108, 32)
                            .contains(p)
                        {
                            self.disconnect(index);
                            break;
                        }
                    }
                    if Rect::new(
                        w.saturating_sub(180) as i32,
                        h.saturating_sub(64) as i32,
                        72,
                        26,
                    )
                    .contains(p)
                    {
                        self.offset = self.offset.saturating_sub(Self::visible(h));
                    }
                    if Rect::new(
                        w.saturating_sub(96) as i32,
                        h.saturating_sub(64) as i32,
                        72,
                        26,
                    )
                    .contains(p)
                        && self.offset + Self::visible(h) < self.snapshot.count as usize
                    {
                        self.offset += Self::visible(h);
                    }
                }
            }
            _ => {}
        }
        false
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authorization_failures_have_distinct_messages() {
        for (s, word) in [
            (2, "state mismatch"),
            (3, "timed out"),
            (4, "cancelled"),
            (5, "provider rejected"),
            (6, "Network"),
        ] {
            assert!(outcome(s).contains(word));
        }
    }
    #[test]
    fn account_list_paginates_for_small_windows() {
        assert_eq!(OnlineAccounts::visible(240), 1);
        assert!(OnlineAccounts::visible(600) > 1);
    }
}
