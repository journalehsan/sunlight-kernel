//! Users & Groups: untrusted presentation client. No account files or auth logic.
use alloc::{format, string::String};
use sun_font::{FontRole, TextStyle, Typography};
use sunlight_ipc::accounts::{self as api, Snapshot};
use sunlight_ui::{
    widgets::Button, AxisSizing, Canvas, Column, Event, LayoutBox, Point, Rect, Sizing, Theme,
};

fn label(canvas: &mut Canvas, theme: &Theme, r: Rect, text: &str) {
    if r.x >= canvas.width as i32 || r.y >= canvas.height as i32 || r.w == 0 || r.h == 0 {
        return;
    }
    let mut clipped = canvas.sub_canvas(r);
    sun_font::draw_text_vcenter(
        &mut clipped,
        text,
        0,
        0,
        r.h,
        &TextStyle::new(FontRole::UiRegular, theme.text),
    );
}
fn message(canvas: &mut Canvas, theme: &Theme, r: Rect, text: &str) {
    let mut line = String::new();
    let mut y = r.y;
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            String::from(word)
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && sun_font::measure_text(&candidate, FontRole::UiRegular).w > r.w {
            if y + 18 <= r.bottom() {
                label(canvas, theme, Rect::new(r.x, y, r.w, 18), &line);
            }
            y += 18;
            line.clear();
            line.push_str(word);
        } else {
            line = candidate;
        }
    }
    if y + 18 <= r.bottom() {
        label(canvas, theme, Rect::new(r.x, y, r.w, 18), &line);
    }
}
fn button(canvas: &mut Canvas, theme: &Theme, r: Rect, text: &str) {
    Button::new(r, text)
        .with_font(&Typography::UI_MEDIUM)
        .draw(canvas, theme);
}

use sunlight_ui::widgets::SecretInput as SecretField;

pub struct AccountsPage {
    pub snapshot: Option<Snapshot>,
    status: &'static str,
    groups: bool,
    selected: usize,
    offset: usize,
    password_open: bool,
    secrets: [SecretField; 3],
    focus: usize,
    next_refresh: u64,
    member: usize,
    create_group: bool,
    group_name: sunlight_ui::widgets::TextInput<'static, 32>,
    profile_name: sunlight_ui::widgets::TextInput<'static, 48>,
    profile_uid: Option<u32>,
    profile_revision: u64,
    elevation_pid: Option<u64>,
}
impl AccountsPage {
    pub fn new() -> Self {
        Self {
            snapshot: None,
            status: "Loading accounts...",
            groups: false,
            selected: 0,
            offset: 0,
            password_open: false,
            secrets: [SecretField::new(), SecretField::new(), SecretField::new()],
            focus: 0,
            next_refresh: 0,
            member: 0,
            create_group: false,
            group_name: sunlight_ui::widgets::TextInput::new(Rect::new(0, 0, 0, 0)),
            profile_name: sunlight_ui::widgets::TextInput::new(Rect::new(0, 0, 0, 0)),
            profile_uid: None,
            profile_revision: 0,
            elevation_pid: None,
        }
    }
    pub fn refresh(&mut self, select_current: bool) -> bool {
        let changed = self.apply_snapshot(api::snapshot(), select_current);
        self.next_refresh = sunlight_ipc::monotonic_millis().saturating_add(5000);
        changed
    }
    fn apply_snapshot(
        &mut self,
        result: Result<Snapshot, api::Error>,
        select_current: bool,
    ) -> bool {
        let old = self.snapshot;
        match result {
            Ok(s) => {
                let changed = old.is_some_and(|o| {
                    (o.session_id, o.generation, o.current_uid)
                        != (s.session_id, s.generation, s.current_uid)
                });
                if changed {
                    self.cancel();
                    self.status = "Session changed. Account information refreshed.";
                } else if self.status == "Loading accounts..." || old.is_none() {
                    self.status = "";
                }
                if select_current || changed {
                    self.groups = false;
                    self.selected = s
                        .users()
                        .iter()
                        .position(|u| u.uid == s.current_uid)
                        .unwrap_or(0);
                    self.offset = self.selected;
                }
                self.snapshot = Some(s);
                self.selected = self.selected.min(self.count().saturating_sub(1));
                self.offset = self.offset.min(self.count().saturating_sub(1));
                self.load_profile();
            }
            Err(e) => {
                self.cancel();
                self.snapshot = None;
                self.status = e.message();
            }
        }
        old != self.snapshot
    }
    fn load_profile(&mut self) {
        if let Some(s) = self.snapshot.as_ref() {
            if let Some(user) = s.users().get(self.selected) {
                if self.profile_uid != Some(user.uid) || !self.profile_name.active {
                    self.profile_uid = Some(user.uid);
                    self.profile_name.set_text(user.name());
                    self.profile_revision = s.revision;
                }
            }
        }
    }
    fn save_profile(&mut self) {
        if let Some(mut s) = self.snapshot {
            s.revision = self.profile_revision;
            let result = api::update_own_profile(&s, self.profile_name.value());
            self.profile_name.active = false;
            self.refresh(false);
            self.status = match result {
                Ok(()) => "Display name updated",
                Err(e) => e.message(),
            };
        }
    }
    pub fn activate(&mut self) {
        self.refresh(true);
    }
    pub fn clear_secrets(&mut self) {
        self.cancel();
    }
    fn cancel(&mut self) {
        self.create_group = false;
        self.password_open = false;
        for s in &mut self.secrets {
            s.clear();
        }
    }
    pub fn identity(&self) -> Option<&api::User> {
        self.snapshot.as_ref()?.current()
    }
    pub fn draw_identity(&self, canvas: &mut Canvas, theme: &Theme, r: Rect) {
        let avatar = Rect::new(r.x, r.y + 4, 32, 32);
        let user = self.identity();
        let tint = user.map_or(theme.accent, |u| {
            if u.avatar_id() & 1 == 0 {
                theme.accent
            } else {
                theme.accent.darken(30)
            }
        });
        canvas.fill_rounded_rect(avatar, 16, tint);
        label(
            canvas,
            theme,
            Rect::new(avatar.x + 10, avatar.y, 22, 32),
            user.map_or("?", |u| u.initial()),
        );
        label(
            canvas,
            theme,
            Rect::new(r.x + 42, r.y, r.w.saturating_sub(42), 22),
            user.map_or("Account unavailable", |u| u.name()),
        );
        label(
            canvas,
            theme,
            Rect::new(r.x + 42, r.y + 22, r.w.saturating_sub(42), 18),
            user.map_or("Refresh in Users & Groups", |u| u.login()),
        );
    }
    fn layout(w: u32, h: u32) -> [Rect; 5] {
        let mut slots = [
            LayoutBox::new(Rect::new(0, 0, 0, 40))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(40))),
            LayoutBox::new(Rect::new(0, 0, 0, 38))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(38))),
            LayoutBox::new(Rect::new(0, 0, 0, 0))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Flex(1))),
            LayoutBox::new(Rect::new(0, 0, 0, 128))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(160))),
            LayoutBox::new(Rect::new(0, 0, 0, 40))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fixed(40))),
        ];
        let _ = Column::new(Rect::new(12, 8, w.saturating_sub(24), h.saturating_sub(16)))
            .arrange(&mut slots);
        core::array::from_fn(|i| slots[i].bounds())
    }
    fn count(&self) -> usize {
        self.snapshot.as_ref().map_or(0, |s| {
            if self.groups {
                s.groups().len()
            } else {
                s.users().len()
            }
        })
    }
    fn row_count(r: Rect) -> usize {
        (r.h / 48).max(1) as usize
    }
    pub fn draw(&mut self, canvas: &mut Canvas, theme: &Theme, w: u32, h: u32) {
        let [head, tabs, list, details, status] = Self::layout(w, h);
        button(canvas, theme, Rect::new(head.x, head.y, 60, 30), "Back");
        label(
            canvas,
            theme,
            Rect::new(head.x + 72, head.y, head.w.saturating_sub(172), 30),
            "Users & Groups",
        );
        button(
            canvas,
            theme,
            Rect::new(head.right() - 86, head.y, 86, 30),
            "Refresh",
        );
        button(
            canvas,
            theme,
            Rect::new(tabs.x, tabs.y, 90, 30),
            if self.groups { "Users" } else { "[Users]" },
        );
        button(
            canvas,
            theme,
            Rect::new(tabs.x + 100, tabs.y, 100, 30),
            if self.groups { "[Groups]" } else { "Groups" },
        );
        button(
            canvas,
            theme,
            Rect::new(tabs.right() - 72, tabs.y, 32, 30),
            "<",
        );
        button(
            canvas,
            theme,
            Rect::new(tabs.right() - 34, tabs.y, 32, 30),
            ">",
        );
        if let Some(s) = self.snapshot.as_ref() {
            let visible_list = if list.x >= canvas.width as i32 || list.y >= canvas.height as i32 {
                Rect::new(0, 0, 0, 0)
            } else {
                list
            };
            let mut clipped = canvas.sub_canvas(visible_list);
            for index in self.offset..self.count().min(self.offset + Self::row_count(list)) {
                let r = Rect::new(0, ((index - self.offset) * 48) as i32, list.w, 46);
                if index == self.selected {
                    clipped.fill_rounded_rect(r, 6, theme.accent.darken(65));
                }
                let title = if self.groups {
                    let g = &s.groups()[index];
                    format!("{}   ·   GID {}", g.name(), g.gid)
                } else {
                    let u = &s.users()[index];
                    String::from(u.name())
                };
                label(
                    &mut clipped,
                    theme,
                    Rect::new(
                        if self.groups { 8 } else { 48 },
                        r.y,
                        r.w.saturating_sub(if self.groups { 16 } else { 56 }),
                        24,
                    ),
                    &title,
                );
                if !self.groups {
                    let u = &s.users()[index];
                    let avatar = Rect::new(8, r.y + 6, 32, 32);
                    clipped.fill_rounded_rect(
                        avatar,
                        16,
                        if u.avatar_id() & 1 == 0 {
                            theme.accent
                        } else {
                            theme.accent.darken(30)
                        },
                    );
                    label(
                        &mut clipped,
                        theme,
                        Rect::new(18, r.y + 6, 22, 32),
                        u.initial(),
                    );
                    label(
                        &mut clipped,
                        theme,
                        Rect::new(48, r.y + 24, r.w.saturating_sub(56), 20),
                        &format!(
                            "{}  ·  UID {}  ·  {}",
                            u.login(),
                            u.uid,
                            if u.is_admin() {
                                "Administrator"
                            } else {
                                "Standard"
                            }
                        ),
                    );
                }
            }
            if self.groups {
                if let Some(g) = s.groups().get(self.selected) {
                    let mut members = String::from("Members: ");
                    for (i, u) in s.users().iter().enumerate() {
                        if g.members & (1 << i) != 0 {
                            if members.len() > 9 {
                                members.push_str(", ");
                            }
                            members.push_str(u.login());
                        }
                    }
                    if members.len() == 9 {
                        members.push_str("None");
                    }
                    label(
                        canvas,
                        theme,
                        Rect::new(details.x, details.y, details.w, 28),
                        &members,
                    );
                    if s.policy & 1 != 0 {
                        let user = s.users().get(self.member % s.users().len());
                        button(
                            canvas,
                            theme,
                            Rect::new(details.x, details.y + 32, details.w.min(220), 28),
                            &user.map_or_else(
                                || String::from("Select member"),
                                |u| {
                                    format!(
                                        "{} · {}  >",
                                        u.login(),
                                        if g.members & (1 << (self.member % s.users().len())) != 0 {
                                            "member"
                                        } else {
                                            "not a member"
                                        }
                                    )
                                },
                            ),
                        );
                        if g.gid >= 1000 {
                            button(
                                canvas,
                                theme,
                                Rect::new(details.x, details.y + 66, 80, 30),
                                "Add...",
                            );
                            button(
                                canvas,
                                theme,
                                Rect::new(details.x + 88, details.y + 66, 100, 30),
                                "Remove...",
                            );
                            if g.members == 0 {
                                button(
                                    canvas,
                                    theme,
                                    Rect::new(details.x + 196, details.y + 66, 90, 30),
                                    "Delete...",
                                );
                            }
                        } else {
                            label(
                                canvas,
                                theme,
                                Rect::new(details.x, details.y + 66, details.w, 30),
                                "Protected system group",
                            );
                        }
                        button(
                            canvas,
                            theme,
                            Rect::new(details.x, details.y + 102, 160, 30),
                            "Create Group...",
                        );
                    } else {
                        label(
                            canvas,
                            theme,
                            Rect::new(details.x, details.y + 32, details.w, 28),
                            "Administrator authentication is required to edit groups.",
                        );
                    }
                }
            } else if let Some(u) = s.users().get(self.selected) {
                let primary = s
                    .groups()
                    .iter()
                    .find(|g| g.gid == u.gid)
                    .map(|g| g.name())
                    .unwrap_or("Unlisted group");
                if u.uid == s.current_uid {
                    self.profile_name.rect =
                        Rect::new(details.x, details.y, details.w.saturating_sub(86), 30);
                    self.profile_name.draw(canvas, theme);
                    button(
                        canvas,
                        theme,
                        Rect::new(details.right() - 78, details.y, 78, 30),
                        "Save name",
                    );
                    button(
                        canvas,
                        theme,
                        Rect::new(details.x, details.y + 70, 180, 32),
                        "Change Password...",
                    );
                } else {
                    label(
                        canvas,
                        theme,
                        Rect::new(details.x, details.y, details.w, 28),
                        u.name(),
                    );
                }
                label(
                    canvas,
                    theme,
                    Rect::new(details.x, details.y + 36, details.w, 28),
                    &format!("Primary group: {} · GID {}", primary, u.gid),
                );
                label(
                    canvas,
                    theme,
                    Rect::new(details.x, details.y + 112, details.w, 24),
                    "Account creation and deletion are not available yet.",
                );
            }
        }
        message(canvas, theme, status, self.status);
        if self.create_group {
            let r = Self::dialog(w, h);
            canvas.fill_rounded_rect(r, 8, theme.accent.darken(85));
            label(
                canvas,
                theme,
                Rect::new(r.x + 16, r.y + 16, r.w - 32, 28),
                "Create Group",
            );
            label(
                canvas,
                theme,
                Rect::new(r.x + 16, r.y + 52, r.w - 32, 28),
                "Name (lowercase letters, digits, - or _):",
            );
            self.group_name.draw(canvas, theme);
            label(
                canvas,
                theme,
                Rect::new(r.x + 16, r.y + 130, r.w - 32, 44),
                self.status,
            );
            button(
                canvas,
                theme,
                Rect::new(r.x + 16, r.bottom() - 44, 100, 30),
                "Cancel",
            );
            button(
                canvas,
                theme,
                Rect::new(r.right() - 156, r.bottom() - 44, 140, 30),
                "Authenticate...",
            );
        }
        if self.password_open {
            self.draw_password(canvas, theme, w, h);
        }
    }
    fn dialog(w: u32, h: u32) -> Rect {
        let width = w.saturating_sub(24).min(420);
        let height = h.saturating_sub(24).min(312);
        Rect::new(
            ((w - width) / 2) as i32,
            ((h - height) / 2) as i32,
            width,
            height,
        )
    }
    fn fields(r: Rect) -> [Rect; 3] {
        core::array::from_fn(|i| {
            Rect::new(
                r.x + 16,
                r.y + 64 + i as i32 * 52,
                r.w.saturating_sub(32),
                28,
            )
        })
    }
    fn draw_password(&self, canvas: &mut Canvas, theme: &Theme, w: u32, h: u32) {
        let r = Self::dialog(w, h);
        canvas.fill_rounded_rect(r, 8, theme.accent.darken(85));
        label(
            canvas,
            theme,
            Rect::new(r.x + 16, r.y + 10, r.w.saturating_sub(32), 24),
            "Change your password",
        );
        let fields = Self::fields(r);
        for (i, title) in ["Current password", "New password", "Confirm new password"]
            .iter()
            .enumerate()
        {
            let f = fields[i];
            label(canvas, theme, Rect::new(f.x, f.y - 20, f.w, 20), title);
            canvas.fill_rounded_rect(
                f,
                4,
                if self.focus == i {
                    theme.accent.darken(40)
                } else {
                    theme.accent.darken(65)
                },
            );
            let masked = "•".repeat(
                core::str::from_utf8(self.secrets[i].value()).map_or(0, |s| s.chars().count()),
            );
            label(canvas, theme, f, &masked);
        }
        message(
            canvas,
            theme,
            Rect::new(r.x + 16, r.y + 210, r.w.saturating_sub(32), 44),
            self.status,
        );
        button(
            canvas,
            theme,
            Rect::new(r.x + 16, r.bottom() - 44, 100, 30),
            "Cancel",
        );
        button(
            canvas,
            theme,
            Rect::new(r.right() - 116, r.bottom() - 44, 100, 30),
            "Change",
        );
    }
    fn submit(&mut self) {
        if !api::password_matches(self.secrets[1].value(), self.secrets[2].value()) {
            self.status = "New passwords must match and be non-empty";
            for s in &mut self.secrets {
                s.clear();
            }
            self.focus = 0;
            return;
        }
        let result = self
            .snapshot
            .as_ref()
            .ok_or(api::Error::SessionChanged)
            .and_then(|s| {
                api::change_own_password(s, self.secrets[0].value(), self.secrets[1].value())
            });
        self.cancel();
        self.refresh(false);
        self.status = match result {
            Ok(()) => "Password successfully changed",
            Err(e) => e.message(),
        };
    }
    fn run_group_action(&mut self, op: api::Operation) {
        if self.elevation_pid.is_some() {
            self.status = "Complete or cancel the open Run-As prompt first.";
            return;
        }
        let Some(s) = self.snapshot.as_ref() else {
            return;
        };
        let Some(g) = s.groups().get(self.selected) else {
            return;
        };
        let gid = if op == api::Operation::GroupCreate {
            s.groups()
                .iter()
                .map(|g| g.gid)
                .chain(s.users().iter().map(|u| u.gid))
                .max()
                .unwrap_or(999)
                .max(999)
                .saturating_add(1)
        } else {
            g.gid
        };
        let member = s
            .users()
            .get(self.member % s.users().len())
            .map_or(0, |u| u.uid);
        let flag = match op {
            api::Operation::GroupCreate => "--group-create",
            api::Operation::GroupAddMember => "--group-add",
            api::Operation::GroupRemoveMember => "--group-remove",
            api::Operation::GroupDelete => "--group-delete",
            _ => return,
        };
        let gid = format!("{gid}");
        let member = format!("{member}");
        let session = format!("{}", s.session_id);
        let generation = format!("{}", s.generation);
        let revision = format!("{}", s.revision);
        let name = if op == api::Operation::GroupCreate {
            self.group_name.value()
        } else {
            ""
        };
        self.status = match sunlight_libc::spawn(
            b"/bin/runas",
            &[
                b"runas",
                flag.as_bytes(),
                gid.as_bytes(),
                member.as_bytes(),
                session.as_bytes(),
                generation.as_bytes(),
                name.as_bytes(),
                revision.as_bytes(),
            ],
            None,
        ) {
            Ok(pid) => {
                self.elevation_pid = Some(pid);
                "Complete authentication in Run-As."
            }
            Err(_) => "Run-As unavailable. No group changes were made.",
        };
        self.create_group = false;
    }

    /// Returns true only for Back; all other input stays on this page.
    pub fn update(&mut self, event: Event, w: u32, h: u32) -> bool {
        if self.create_group {
            let r = Self::dialog(w, h);
            self.group_name.rect = Rect::new(r.x + 16, r.y + 90, r.w.saturating_sub(32), 30);
            match event {
                Event::KeyPress {
                    keycode: 0x01,
                    pressed: true,
                    ..
                } => self.create_group = false,
                Event::Click { x, y } => {
                    let p = Point::new(x, y);
                    if Rect::new(r.x + 16, r.bottom() - 44, 100, 30).contains(p) {
                        self.create_group = false;
                    } else if Rect::new(r.right() - 156, r.bottom() - 44, 140, 30).contains(p) {
                        self.run_group_action(api::Operation::GroupCreate);
                    } else {
                        self.group_name.update(event);
                    }
                }
                _ => {
                    self.group_name.update(event);
                }
            }
            return false;
        }
        if self.password_open {
            match event {
                Event::FocusChanged { focused: false } => {
                    self.cancel();
                    self.status = "Password change cancelled";
                }
                Event::KeyPress {
                    keycode: 0x01,
                    pressed: true,
                    ..
                } => {
                    self.cancel();
                    self.status = "Password change cancelled";
                }
                Event::KeyPress {
                    keycode: 0x0f,
                    pressed: true,
                    ..
                } => self.focus = (self.focus + 1) % 3,
                Event::KeyPress {
                    keycode: 0x1c,
                    pressed: true,
                    ..
                } => self.submit(),
                Event::Key(c) => self.secrets[self.focus].key(c),
                Event::Click { x, y } => {
                    let p = Point::new(x, y);
                    let r = Self::dialog(w, h);
                    for (i, f) in Self::fields(r).iter().enumerate() {
                        if f.contains(p) {
                            self.focus = i;
                        }
                    }
                    if Rect::new(r.x + 16, r.bottom() - 44, 100, 30).contains(p) {
                        self.cancel();
                        self.status = "Password change cancelled";
                    } else if Rect::new(r.right() - 116, r.bottom() - 44, 100, 30).contains(p) {
                        self.submit();
                    }
                }
                Event::Tick if sunlight_ipc::monotonic_millis() >= self.next_refresh => {
                    self.refresh(false);
                }
                _ => {}
            }
            return false;
        }
        let [head, tabs, list, details, _] = Self::layout(w, h);
        match event {
            Event::KeyPress {
                keycode: 0x01,
                pressed: true,
                ..
            } => return true,
            Event::Click { x, y } => {
                let p = Point::new(x, y);
                if Rect::new(head.x, head.y, 60, 30).contains(p) {
                    return true;
                }
                if Rect::new(head.right() - 86, head.y, 86, 30).contains(p) {
                    self.refresh(false);
                }
                if Rect::new(tabs.x, tabs.y, 90, 30).contains(p) {
                    self.groups = false;
                    self.selected = 0;
                    self.offset = 0;
                }
                if Rect::new(tabs.x + 100, tabs.y, 100, 30).contains(p) {
                    self.groups = true;
                    self.selected = 0;
                    self.offset = 0;
                }
                if Rect::new(tabs.right() - 72, tabs.y, 32, 30).contains(p) {
                    self.offset = self.offset.saturating_sub(Self::row_count(list));
                }
                if Rect::new(tabs.right() - 34, tabs.y, 32, 30).contains(p) {
                    self.offset =
                        (self.offset + Self::row_count(list)).min(self.count().saturating_sub(1));
                }
                if list.contains(p) {
                    self.selected = (self.offset + (y - list.y) as usize / 48)
                        .min(self.count().saturating_sub(1));
                    self.profile_name.active = false;
                    self.load_profile();
                }
                if !self.groups
                    && Rect::new(details.right() - 78, details.y, 78, 30).contains(p)
                    && self.snapshot.as_ref().is_some_and(|s| {
                        s.users()
                            .get(self.selected)
                            .is_some_and(|u| u.uid == s.current_uid)
                    })
                {
                    self.save_profile();
                }
                if self.groups && self.snapshot.as_ref().is_some_and(|s| s.policy & 1 != 0) {
                    if Rect::new(details.x, details.y + 32, details.w.min(220), 28).contains(p) {
                        self.member = self.member.wrapping_add(1);
                    }
                    let mutable = self
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.groups().get(self.selected))
                        .is_some_and(|g| g.gid >= 1000);
                    if mutable && Rect::new(details.x, details.y + 66, 80, 30).contains(p) {
                        self.run_group_action(api::Operation::GroupAddMember);
                    }
                    if mutable && Rect::new(details.x + 88, details.y + 66, 100, 30).contains(p) {
                        self.run_group_action(api::Operation::GroupRemoveMember);
                    }
                    if mutable
                        && self
                            .snapshot
                            .as_ref()
                            .and_then(|s| s.groups().get(self.selected))
                            .is_some_and(|g| g.members == 0)
                        && Rect::new(details.x + 196, details.y + 66, 90, 30).contains(p)
                    {
                        self.run_group_action(api::Operation::GroupDelete);
                    }
                    if Rect::new(details.x, details.y + 102, 160, 30).contains(p) {
                        self.create_group = true;
                        self.group_name.set_text("");
                        self.group_name.active = true;
                        let r = Self::dialog(w, h);
                        self.group_name.rect =
                            Rect::new(r.x + 16, r.y + 90, r.w.saturating_sub(32), 30);
                    }
                }
                if !self.groups
                    && Rect::new(details.x, details.y + 70, 180, 32).contains(p)
                    && self.snapshot.as_ref().is_some_and(|s| {
                        s.users()
                            .get(self.selected)
                            .is_some_and(|u| u.uid == s.current_uid)
                    })
                {
                    self.cancel();
                    self.password_open = true;
                    self.focus = 0;
                    self.status = "";
                }
            }
            _ => {}
        }
        if !self.groups
            && !self.password_open
            && self.snapshot.as_ref().is_some_and(|s| {
                s.users()
                    .get(self.selected)
                    .is_some_and(|u| u.uid == s.current_uid)
            })
        {
            self.profile_name.update(event);
        }
        false
    }
    pub fn poll(&mut self) -> bool {
        if let Some(pid) = self.elevation_pid {
            match sunlight_libc::try_waitpid(pid) {
                Ok(None) => {}
                Ok(Some(code)) => {
                    self.elevation_pid = None;
                    self.refresh(false);
                    self.status = if code == 0 {
                        "Group updated"
                    } else {
                        "Group update cancelled or failed"
                    };
                    return true;
                }
                Err(_) => {
                    self.elevation_pid = None;
                    self.status = "Run-As unavailable. Refresh to check group state.";
                    return true;
                }
            }
        }
        if sunlight_ipc::monotonic_millis() >= self.next_refresh {
            self.refresh(false);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model() -> Snapshot {
        let mut s = Snapshot::EMPTY;
        s.session_id = 2;
        s.current_uid = 42;
        s.user_count = 1;
        s.users[0].uid = 42;
        s.users[0].username[..3].copy_from_slice(b"ada");
        s.users[0].display_name[..3].copy_from_slice(b"Ada");
        s
    }
    #[test]
    fn service_loss_and_session_change_clear_secrets_and_stale_identity() {
        let mut page = AccountsPage::new();
        page.apply_snapshot(Ok(model()), true);
        page.password_open = true;
        page.secrets[0].key('x');
        page.apply_snapshot(Err(api::Error::Unavailable), false);
        assert!(page.snapshot.is_none());
        assert!(!page.password_open);
        assert!(page.secrets[0].value().is_empty());
        page.apply_snapshot(Ok(model()), true);
        page.password_open = true;
        page.secrets[0].key('x');
        let mut next = model();
        next.session_id += 1;
        page.apply_snapshot(Ok(next), false);
        assert!(!page.password_open);
        assert!(page.secrets[0].value().is_empty());
        assert_eq!(page.snapshot.unwrap().session_id, next.session_id);
    }

    #[test]
    fn preferences_header_uses_session_metadata() {
        let mut page = AccountsPage::new();
        page.snapshot = Some(model());
        assert_eq!(page.identity().unwrap().name(), "Ada");
        page.snapshot.as_mut().unwrap().users[0].display_name[..3].copy_from_slice(b"New");
        assert_eq!(page.identity().unwrap().name(), "New");
    }
    #[test]
    fn cancel_and_focus_loss_clear_all_password_fields() {
        for event in [
            Event::FocusChanged { focused: false },
            Event::KeyPress {
                keycode: 0x01,
                pressed: true,
                shift: false,
                ctrl: false,
                alt: false,
                super_key: false,
            },
        ] {
            let mut page = AccountsPage::new();
            page.password_open = true;
            for field in &mut page.secrets {
                field.key('x');
            }
            page.update(event, 500, 560);
            assert!(!page.password_open);
            assert!(page.secrets.iter().all(|s| s.value().is_empty()));
        }
    }
    #[test]
    fn mismatched_confirmation_is_rejected_before_ipc() {
        let mut page = AccountsPage::new();
        page.password_open = true;
        page.secrets[0].key('a');
        page.secrets[1].key('b');
        page.secrets[2].key('c');
        page.submit();
        assert_eq!(page.status, "New passwords must match and be non-empty");
        assert!(page.secrets.iter().all(|s| s.value().is_empty()));
    }
    #[test]
    fn responsive_page_and_password_dialog_render_within_client() {
        for (w, h) in [(320, 400), (500, 560), (900, 700)] {
            let mut pixels = alloc::vec![0u32;w as usize*h as usize];
            let mut c = Canvas::new(&mut pixels, w, w, h);
            let mut page = AccountsPage::new();
            page.snapshot = Some(model());
            page.draw(&mut c, &Theme::default(), w, h);
            page.password_open = true;
            page.draw(&mut c, &Theme::default(), w, h);
            let slots = AccountsPage::layout(w, h);
            for r in slots {
                assert!(r.right() <= w as i32);
                assert!(r.bottom() <= h as i32);
            }
        }
    }
}
