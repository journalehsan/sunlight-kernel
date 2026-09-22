//! Standard Run-As account prompt. Passwords never return to Settings.
use alloc::string::String;
use sun_font::{FontRole, TextStyle, Typography};
use sunlight_ipc::accounts::{self as api, GrantScope, GroupRequest, Operation};
use sunlight_ui::{
    request_close,
    widgets::{Button, SecretInput},
    App, Canvas, Event, Point, Rect, Theme, Window, WindowConfig, WindowDecoration,
};
struct Prompt {
    request: GroupRequest,
    name: String,
    initial: String,
    avatar_id: u32,
    reason: String,
    password: SecretInput,
    status: &'static str,
    complete: bool,
}
fn label(c: &mut Canvas, t: &Theme, r: Rect, s: &str) {
    if r.x >= c.width as i32 || r.y >= c.height as i32 || r.w == 0 || r.h == 0 {
        return;
    }
    let mut sub = c.sub_canvas(r);
    sun_font::draw_text_vcenter(
        &mut sub,
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
impl Prompt {
    fn authenticate(&mut self) {
        let grant = api::authorize_group(self.request, self.password.value());
        self.password.clear();
        match grant.and_then(|g| api::mutate_group(self.request, g)) {
            Ok(()) => {
                self.complete = true;
                self.status = "Group updated. You can close this window.";
            }
            Err(e) => self.status = e.message(),
        }
    }
}
impl App for Prompt {
    fn view(&mut self, c: &mut Canvas, t: &Theme) {
        label(c, t, Rect::new(16, 12, 388, 28), "Authentication Required");
        let avatar = Rect::new(16, 42, 32, 32);
        c.fill_rounded_rect(
            avatar,
            16,
            if self.avatar_id & 1 == 0 {
                t.accent
            } else {
                t.accent.darken(30)
            },
        );
        label(c, t, Rect::new(26, 42, 22, 32), &self.initial);
        label(c, t, Rect::new(60, 46, 344, 24), &self.name);
        label(c, t, Rect::new(16, 76, 388, 24), &self.reason);
        label(
            c,
            t,
            Rect::new(16, 104, 388, 24),
            "Authenticate to change group settings.",
        );
        label(c, t, Rect::new(16, 134, 100, 28), "Password:");
        let field = Rect::new(112, 134, 292, 30);
        c.fill_rounded_rect(field, 4, t.accent.darken(50));
        let masked = "•"
            .repeat(core::str::from_utf8(self.password.value()).map_or(0, |s| s.chars().count()));
        label(c, t, field, &masked);
        label(c, t, Rect::new(16, 172, 388, 36), self.status);
        button(
            c,
            t,
            Rect::new(16, 220, 100, 32),
            if self.complete { "Close" } else { "Cancel" },
        );
        if !self.complete {
            button(c, t, Rect::new(270, 220, 134, 32), "Authenticate");
        }
    }
    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Key(c) if !self.complete => self.password.key(c),
            Event::KeyPress {
                keycode: 0x1c,
                pressed: true,
                ..
            } if !self.complete => self.authenticate(),
            Event::KeyPress {
                keycode: 0x01,
                pressed: true,
                ..
            } => {
                self.password.clear();
                request_close();
            }
            Event::FocusChanged { focused: false } => self.password.clear(),
            Event::Click { x, y } => {
                let p = Point::new(x, y);
                if Rect::new(16, 220, 100, 32).contains(p) {
                    self.password.clear();
                    request_close();
                } else if !self.complete && Rect::new(270, 220, 134, 32).contains(p) {
                    self.authenticate();
                }
            }
            _ => return false,
        }
        true
    }
}
pub fn run(args: &[&str]) -> u64 {
    if args.len() != 8 {
        return 2;
    }
    let op = match args[1] {
        "--group-create" => Operation::GroupCreate,
        "--group-add" => Operation::GroupAddMember,
        "--group-remove" => Operation::GroupRemoveMember,
        "--group-delete" => Operation::GroupDelete,
        _ => return 2,
    };
    let (Ok(gid), Ok(member), Ok(session), Ok(generation)) = (
        args[2].parse::<u32>(),
        args[3].parse::<u32>(),
        args[4].parse::<u64>(),
        args[5].parse::<u64>(),
    ) else {
        return 2;
    };
    let Ok(s) = api::snapshot() else {
        return 1;
    };
    if (s.session_id, s.generation) != (session, generation) {
        return 1;
    }
    let Some(user) = s.current() else {
        return 1;
    };
    let Ok(revision) = args[7].parse::<u64>() else {
        return 2;
    };
    if revision != s.revision {
        return 1;
    }
    let mut name = [0u8; 32];
    if args[6].len() > 31 {
        return 2;
    }
    name[..args[6].len()].copy_from_slice(args[6].as_bytes());
    let group = if op == Operation::GroupCreate {
        args[6]
    } else {
        s.groups()
            .iter()
            .find(|g| g.gid == gid)
            .map_or("Unknown group", |g| g.name())
    };
    let member_name = s
        .users()
        .iter()
        .find(|u| u.uid == member)
        .map_or("", |u| u.login());
    let reason = match op {
        Operation::GroupCreate => alloc::format!("Create group {group}"),
        Operation::GroupDelete => alloc::format!("Delete group {group}"),
        Operation::GroupAddMember => alloc::format!("Add {member_name} to {group}"),
        Operation::GroupRemoveMember => alloc::format!("Remove {member_name} from {group}"),
        _ => return 2,
    };
    let mut app = Prompt {
        request: GroupRequest {
            scope: GrantScope {
                revision,
                operation: op,
                target: gid,
                member,
                session,
                session_generation: generation,
            },
            name,
        },
        name: String::from(user.name()),
        initial: String::from(user.initial()),
        avatar_id: user.avatar_id(),
        reason,
        password: SecretInput::new(),
        status: "",
        complete: false,
    };
    let Some(mut window) = Window::connect_with_flags(
        WindowConfig {
            width: 420,
            height: 270,
            title: "Run-As — Authentication",
            decoration: WindowDecoration::CompactClose,
        },
        1 | (1 << 5),
    ) else {
        return 1;
    };
    window.run(&mut app);
    app.password.clear();
    if app.complete {
        0
    } else {
        1
    }
}
