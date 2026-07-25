use sunlight_ipc::{
    ipc_call, nameserver_lookup, process_yield, CapabilityToken, IpcMsg, MezzoMsg, SgpMsg,
    LOCK_SESSION_USERNAME_MAX,
};
use sunlight_uac::auth::{authenticate_password_for_session, MAX_PASSWORD_LEN};
use sunlight_ui::{request_close, App, Canvas, Color, Event, Rect, Theme, Window, WindowConfig};

const SECURE_FULLSCREEN_FLAGS: u64 = (3 << 2) | (1 << 4) | (1 << 5) | (100 << 6);
const LOCK_PRESENTER_ENTRY_MAGIC: u64 = 0x4C4F_434B_5052_4553;

pub fn requested(argc: u64, argv: *const *const u8) -> bool {
    if argc == LOCK_PRESENTER_ENTRY_MAGIC {
        return true;
    }
    if argc == 0 || argv.is_null() {
        return false;
    }
    let pointer = unsafe { *argv };
    if pointer.is_null() {
        return false;
    }
    let mut length = 0usize;
    while length < 64 && unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    let bytes = unsafe { core::slice::from_raw_parts(pointer, length) };
    bytes.ends_with(b"vortex-lock-presenter")
}

struct LockPresenter {
    mezzo: CapabilityToken,
    generation: u64,
    safe_mode: bool,
    session_uid: u32,
    session_gid: u32,
    username: [u8; LOCK_SESSION_USERNAME_MAX],
    username_len: usize,
    password: [u8; MAX_PASSWORD_LEN],
    password_len: usize,
    message: &'static str,
}

impl LockPresenter {
    fn clear_password(&mut self) {
        for byte in &mut self.password[..self.password_len] {
            unsafe { core::ptr::write_volatile(byte, 0) };
        }
        self.password_len = 0;
    }

    fn authenticate(&mut self) {
        let result = authenticate_password_for_session(
            &self.username[..self.username_len],
            &self.password[..self.password_len],
        );
        self.clear_password();
        let Some(success) = result else {
            self.message = "Authentication failed";
            return;
        };
        if success.uid != self.session_uid || success.gid != self.session_gid {
            self.message = "Authentication failed";
            return;
        }
        self.message = "Unlocking...";
        let reply = ipc_call(
            self.mezzo,
            IpcMsg::with_label(MezzoMsg::AUTHENTICATE)
                .word(0, self.generation)
                .with_cap(0, success.session_grant),
        );
        if reply.label == MezzoMsg::REPLY && reply.words[0] == 0 {
            request_close();
        } else {
            self.message = "Unlock rejected";
        }
    }
}

impl App for LockPresenter {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let width = canvas.width;
        let height = canvas.height;
        canvas.fill_rect(
            Rect::new(0, 0, width, height),
            if self.safe_mode {
                Color(0x00101014)
            } else {
                theme.bg
            },
        );
        let panel = Rect::new(
            (width as i32 / 2).saturating_sub(210),
            (height as i32 / 2).saturating_sub(120),
            420,
            240,
        );
        canvas.fill_rounded_rect(panel, 18, theme.panel);
        canvas.stroke_rounded_rect(panel, 18, 1, theme.border);
        sun_font::draw_text(
            canvas,
            "SunlightOS is locked",
            panel.x + 36,
            panel.y + 32,
            &sun_font::TextStyle::new(sun_font::FontRole::UiTitle, theme.text),
        );
        let username = core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("user");
        sun_font::draw_text(
            canvas,
            username,
            panel.x + 36,
            panel.y + 76,
            &sun_font::TextStyle::new(sun_font::FontRole::UiMedium, theme.text_dim),
        );
        let field = Rect::new(panel.x + 36, panel.y + 108, 348, 44);
        canvas.fill_rounded_rect(field, 8, theme.panel_alt);
        canvas.stroke_rounded_rect(field, 8, 1, theme.accent);
        let mut masked = alloc::string::String::new();
        for _ in 0..self.password_len {
            let _ = masked.push('•');
        }
        sun_font::draw_text_vcenter(
            canvas,
            &masked,
            field.x + 14,
            field.y,
            field.h,
            &sun_font::TextStyle::new(sun_font::FontRole::UiRegular, theme.text),
        );
        sun_font::draw_text(
            canvas,
            self.message,
            panel.x + 36,
            panel.y + 172,
            &sun_font::TextStyle::new(sun_font::FontRole::UiSmall, theme.text_dim),
        );
        if self.safe_mode {
            sun_font::draw_text(
                canvas,
                "Safe lock presenter",
                panel.x + 36,
                panel.y + 202,
                &sun_font::TextStyle::new(sun_font::FontRole::UiSmall, theme.warn),
            );
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Key('\n') if self.password_len != 0 => {
                self.authenticate();
                true
            }
            Event::Key('\u{8}') => {
                if self.password_len != 0 {
                    self.password_len -= 1;
                    self.password[self.password_len] = 0;
                    true
                } else {
                    false
                }
            }
            Event::Key(character) if character.is_ascii() && !character.is_ascii_control() => {
                if self.password_len < self.password.len() {
                    self.password[self.password_len] = character as u8;
                    self.password_len += 1;
                    self.message = "Enter password and press Return";
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }
}

pub fn run() -> ! {
    let mezzo = loop {
        if let Some(capability) = nameserver_lookup("mezzo") {
            break capability;
        }
        process_yield();
    };
    let hello = ipc_call(mezzo, IpcMsg::with_label(MezzoMsg::PRESENTER_HELLO));
    if hello.label != MezzoMsg::REPLY {
        sunlight_ipc::ProcessExit::exit(1);
    }
    let generation = hello.words[0];
    let safe_mode = hello.words[1] != 0;
    let display_authority = hello.caps[0];
    let display = nameserver_lookup("display_server").unwrap_or(CapabilityToken::INVALID);
    let metrics = sunlight_ipc::query_display_metrics(display)
        .unwrap_or_else(sunlight_ipc::DisplayMetrics::safe_fallback);
    let mut window = loop {
        if let Some(window) = Window::connect(WindowConfig {
            width: metrics.width_px,
            height: metrics.height_px,
            title: "Vortex Lock Screen",
            decoration: sunlight_ui::WindowDecoration::HiddenOverlay,
        }) {
            break window;
        }
        process_yield();
    };
    window.configure_flags(SECURE_FULLSCREEN_FLAGS);
    let register = ipc_call(
        display,
        IpcMsg::with_label(SgpMsg::LOCK_REGISTER_PRESENTER)
            .word(0, generation)
            .word(1, window.id())
            .with_cap(0, display_authority),
    );
    if register.label != SgpMsg::REPLY {
        sunlight_ipc::ProcessExit::exit(1);
    }
    let mut username = [0u8; LOCK_SESSION_USERNAME_MAX];
    for word in 0..4 {
        username[word * 8..word * 8 + 8].copy_from_slice(&hello.words[4 + word].to_le_bytes());
    }
    let username_len = username
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(username.len());
    if username_len == 0 {
        sunlight_ipc::ProcessExit::exit(1);
    }
    let mut presenter = LockPresenter {
        mezzo,
        generation,
        safe_mode,
        session_uid: hello.words[2] as u32,
        session_gid: hello.words[3] as u32,
        username,
        username_len,
        password: [0; MAX_PASSWORD_LEN],
        password_len: 0,
        message: "Enter password and press Return",
    };
    {
        let theme = Theme::sunlight_dark();
        let mut canvas = window.canvas();
        presenter.view(&mut canvas, &theme);
        window.commit();
    }
    let ready = ipc_call(
        display,
        IpcMsg::with_label(SgpMsg::LOCK_PRESENTER_READY)
            .word(0, generation)
            .with_cap(0, display_authority),
    );
    if ready.label != SgpMsg::REPLY {
        sunlight_ipc::ProcessExit::exit(1);
    }
    let acknowledged = ipc_call(
        mezzo,
        IpcMsg::with_label(MezzoMsg::PRESENTER_READY)
            .word(0, generation)
            .word(1, window.id()),
    );
    if acknowledged.label != MezzoMsg::REPLY {
        sunlight_ipc::ProcessExit::exit(1);
    }
    window.run(&mut presenter);
    presenter.clear_password();
    sunlight_ipc::ProcessExit::exit(0)
}
