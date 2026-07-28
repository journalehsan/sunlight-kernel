use sunlight_ipc::{
    ipc_call, nameserver_lookup, process_yield, CapabilityToken, IpcMsg, MezzoMsg, SessionAction,
    SessionMsg, SgpMsg, LOCK_SESSION_USERNAME_MAX, SESSION_ENDPOINT,
};
use sunlight_uac::auth::{authenticate_password_for_session, MAX_PASSWORD_LEN};
use sunlight_ui::image::{decode_simg, RgbaImage};
use sunlight_ui::widgets::{SolarClockSnapshot, SolarClockWidget};
use sunlight_ui::{
    request_close, App, Canvas, Color, Event, Point, Rect, Theme, Window, WindowConfig,
};

const SECURE_FULLSCREEN_FLAGS: u64 = (3 << 2) | (1 << 4) | (1 << 5) | (100 << 6);
const LOCK_PRESENTER_ENTRY_MAGIC: u64 = 0x4C4F_434B_5052_4553;

/// Same asset as the TTY login screen (`services/tty_server` → `login_bg_simg`).
/// Used for both glance (clock) and password surfaces so lock matches login.
/// SIMG v2 (sub+lz4) — see `docs/SIMG_V2.md`.
static LOGIN_BG_SIMG: &[u8] = include_bytes!("../../../docs/images/sunlight-login-background.simg");

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

#[derive(Clone, Copy, PartialEq, Eq)]
enum PresenterState {
    Glance,
    Login,
    Authenticating,
    Error,
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
    state: PresenterState,
    message: &'static str,
    notif_count: usize,
    wallpaper: Option<RgbaImage>,
    width: u32,
    height: u32,
    last_second: u8,
}

fn current_time() -> (u16, u8, u8, u8, u8, u8) {
    let mut tz_buf = [0u8; 48];
    let mut tz_len = 0;
    crate::query_local_full(&mut tz_buf, &mut tz_len).unwrap_or((2026, 7, 25, 12, 0, 0))
}

fn month_name(mon: u8) -> &'static str {
    match mon {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "",
    }
}

impl LockPresenter {
    fn clear_password(&mut self) {
        for byte in &mut self.password[..self.password_len] {
            unsafe { core::ptr::write_volatile(byte, 0) };
        }
        self.password_len = 0;
    }

    fn authenticate(&mut self) {
        self.state = PresenterState::Authenticating;
        self.message = "Verifying credentials...";
        let result = authenticate_password_for_session(
            &self.username[..self.username_len],
            &self.password[..self.password_len],
        );
        self.clear_password();
        let Some(success) = result else {
            self.message = "Incorrect password. Try again.";
            self.state = PresenterState::Error;
            return;
        };
        if success.uid != self.session_uid || success.gid != self.session_gid {
            self.message = "Incorrect password. Try again.";
            self.state = PresenterState::Error;
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
            // Keep sessiond state in sync with mezzo unlock (Start Menu / Super+L).
            if let Some(sessiond) = nameserver_lookup(SESSION_ENDPOINT) {
                let list = ipc_call(
                    sessiond,
                    IpcMsg::with_label(SessionMsg::SESSION_LIST).word(0, 0),
                );
                if list.label == SessionMsg::REPLY {
                    let _ = ipc_call(
                        sessiond,
                        IpcMsg::with_label(SessionMsg::SESSION_ACTION)
                            .word(0, list.words[0])
                            .word(1, list.words[1])
                            .word(2, SessionAction::UnlockCompleted as u64),
                    );
                }
            }
            request_close();
        } else {
            self.message = "Unlock rejected. Try again.";
            self.state = PresenterState::Error;
        }
    }

    fn ui_rects(&self) -> Option<(Rect, Rect, Rect)> {
        if self.width == 0 || self.height == 0 {
            return None;
        }
        let panel = Rect::new(
            (self.width as i32 / 2).saturating_sub(210),
            (self.height as i32 / 2).saturating_sub(130),
            420,
            260,
        );
        let cancel_btn = Rect::new(panel.x + 36, panel.y + 190, 160, 42);
        let unlock_btn = Rect::new(panel.x + 224, panel.y + 190, 160, 42);
        Some((panel, cancel_btn, unlock_btn))
    }

    fn view_glance(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        width: u32,
        height: u32,
        y: u16,
        mon: u8,
        d: u8,
        h: u8,
        mi: u8,
        s: u8,
    ) {
        let cx = width as i32 / 2;
        let cy = height as i32 / 2;

        // Solar clock in center-top
        let snap = SolarClockSnapshot::new(h, mi, s);
        let date_str = alloc::format!("{} {}, {}", month_name(mon), d, y);
        let clock_rect = Rect::new(cx - 130, cy - 180, 260, 260);
        let clock = SolarClockWidget::new(clock_rect, snap).with_date(&date_str);
        clock.draw(canvas, theme);

        // Privacy-safe notifications summary card
        let notif_rect = Rect::new(cx - 160, cy + 110, 320, 48);
        canvas.fill_rounded_rect(notif_rect, 12, theme.panel);
        canvas.stroke_rounded_rect(notif_rect, 12, 1, theme.border);
        let notif_str = if self.notif_count == 0 {
            "No new notifications".into()
        } else {
            alloc::format!(
                "{} new notification{} • Hidden while locked",
                self.notif_count,
                if self.notif_count == 1 { "" } else { "s" }
            )
        };
        sun_font::draw_text_centered(
            canvas,
            notif_rect,
            &notif_str,
            &sun_font::TextStyle::new(sun_font::FontRole::UiRegular, theme.text),
        );

        // Inert media status card
        let media_rect = Rect::new(cx - 160, cy + 170, 320, 48);
        canvas.fill_rounded_rect(media_rect, 12, theme.panel);
        canvas.stroke_rounded_rect(media_rect, 12, 1, theme.border);
        sun_font::draw_text_centered(
            canvas,
            media_rect,
            "Media playback paused",
            &sun_font::TextStyle::new(sun_font::FontRole::UiRegular, theme.text_dim),
        );

        // Bottom instruction hint — glance stays until Enter/Space only.
        let bottom_rect = Rect::new(0, height as i32 - 48, width, 32);
        sun_font::draw_text_centered(
            canvas,
            bottom_rect,
            "Press Enter or Space to unlock",
            &sun_font::TextStyle::new(sun_font::FontRole::UiSmall, theme.text_dim),
        );
    }

    /// Intentional dismiss of the glance (clock) surface into the password panel.
    /// Mouse motion, clicks, and other keys must not steal the lock design.
    fn glance_to_login_input(event: &Event) -> bool {
        const KEY_ENTER: u8 = 0x1C;
        const KEY_SPACE: u8 = 0x39;
        match event {
            Event::Key('\n') | Event::Key('\r') | Event::Key(' ') => true,
            Event::KeyPress {
                keycode: KEY_ENTER | KEY_SPACE,
                pressed: true,
                ..
            } => true,
            _ => false,
        }
    }

    fn view_login(&self, canvas: &mut Canvas, theme: &Theme) {
        let Some((panel, cancel_btn, unlock_btn)) = self.ui_rects() else {
            return;
        };
        canvas.fill_rounded_rect(panel, 18, theme.panel);
        canvas.stroke_rounded_rect(panel, 18, 1, theme.border);

        let title_text = if self.safe_mode {
            "Safe Recovery Login"
        } else {
            "Unlock SunlightOS"
        };
        sun_font::draw_text(
            canvas,
            title_text,
            panel.x + 36,
            panel.y + 28,
            &sun_font::TextStyle::new(sun_font::FontRole::UiTitle, theme.text),
        );

        let username = core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("user");
        sun_font::draw_text(
            canvas,
            username,
            panel.x + 36,
            panel.y + 68,
            &sun_font::TextStyle::new(sun_font::FontRole::UiMedium, theme.text_dim),
        );

        let field = Rect::new(panel.x + 36, panel.y + 100, 348, 44);
        canvas.fill_rounded_rect(field, 8, theme.panel_alt);
        let border_color = if self.state == PresenterState::Error {
            theme.warn
        } else {
            theme.accent
        };
        canvas.stroke_rounded_rect(field, 8, 1, border_color);

        if self.password_len == 0 {
            sun_font::draw_text_vcenter(
                canvas,
                "Password",
                field.x + 14,
                field.y,
                field.h,
                &sun_font::TextStyle::new(sun_font::FontRole::UiRegular, theme.text_dim),
            );
        } else {
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
        }

        let msg_color = if self.state == PresenterState::Error {
            theme.warn
        } else {
            theme.text_dim
        };
        sun_font::draw_text(
            canvas,
            self.message,
            panel.x + 36,
            panel.y + 158,
            &sun_font::TextStyle::new(sun_font::FontRole::UiSmall, msg_color),
        );

        if self.safe_mode {
            sun_font::draw_text_right(
                canvas,
                Rect::new(panel.x + 36, panel.y + 158, 348, 20),
                "[Safe Mode]",
                &sun_font::TextStyle::new(sun_font::FontRole::UiSmall, theme.warn),
                0,
            );
        }

        // Buttons
        canvas.fill_rounded_rect(cancel_btn, 8, theme.panel_alt);
        canvas.stroke_rounded_rect(cancel_btn, 8, 1, theme.border);
        sun_font::draw_text_centered(
            canvas,
            cancel_btn,
            "Cancel",
            &sun_font::TextStyle::new(sun_font::FontRole::UiRegular, theme.text),
        );

        let unlock_fill = if self.password_len > 0 && self.state != PresenterState::Authenticating {
            theme.accent
        } else {
            theme.panel_alt
        };
        canvas.fill_rounded_rect(unlock_btn, 8, unlock_fill);
        canvas.stroke_rounded_rect(unlock_btn, 8, 1, theme.border);
        sun_font::draw_text_centered(
            canvas,
            unlock_btn,
            "Unlock",
            &sun_font::TextStyle::new(sun_font::FontRole::UiRegular, theme.text),
        );
    }
}

impl App for LockPresenter {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.width = canvas.width;
        self.height = canvas.height;
        let width = canvas.width;
        let height = canvas.height;

        // Background is shared for Glance + Login (same image as TTY login).
        if self.safe_mode {
            canvas.fill_rect(Rect::new(0, 0, width, height), Color(0x00101014));
        } else if let Some(ref bg) = self.wallpaper {
            canvas.draw_rgba_cover(bg);
        } else {
            canvas.fill_rect(Rect::new(0, 0, width, height), theme.bg);
        }

        // Draw top status strip in all views
        let top_bar = Rect::new(0, 0, width, 32);
        canvas.fill_rect(top_bar, Color(0x80000000));
        let title_text = if self.safe_mode {
            "SunlightOS • Safe Recovery Mode"
        } else {
            "SunlightOS • Locked"
        };
        sun_font::draw_text_vcenter(
            canvas,
            title_text,
            16,
            0,
            32,
            &sun_font::TextStyle::new(sun_font::FontRole::UiRegular, theme.text_dim),
        );

        let (y, mon, d, h, mi, s) = current_time();
        self.last_second = s;
        let time_str = alloc::format!("{:02}:{:02}", h, mi);
        sun_font::draw_text_right(
            canvas,
            top_bar,
            &time_str,
            &sun_font::TextStyle::new(sun_font::FontRole::UiRegular, theme.text_dim),
            16,
        );

        match self.state {
            PresenterState::Glance => {
                self.view_glance(canvas, theme, width, height, y, mon, d, h, mi, s);
            }
            PresenterState::Login | PresenterState::Authenticating | PresenterState::Error => {
                self.view_login(canvas, theme);
            }
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match self.state {
            PresenterState::Glance => {
                if Self::glance_to_login_input(&event) {
                    self.state = PresenterState::Login;
                    self.message = "Enter password and press Return";
                    return true;
                }
                match event {
                    Event::Tick => {
                        let (_, _, _, _, _, s) = current_time();
                        if s != self.last_second {
                            self.last_second = s;
                            true
                        } else {
                            false
                        }
                    }
                    // Swallow other input so motion/clicks/random keys never
                    // force the password panel over the clock glance.
                    _ => false,
                }
            }
            PresenterState::Login | PresenterState::Error => {
                if self.state == PresenterState::Error {
                    match event {
                        Event::Key(_) | Event::Click { .. } | Event::MouseDown { .. } => {
                            self.state = PresenterState::Login;
                            self.message = "Enter password and press Return";
                        }
                        _ => {}
                    }
                }

                match event {
                    Event::Key('\u{1b}') => {
                        self.clear_password();
                        self.state = PresenterState::Glance;
                        self.message = "Enter password and press Return";
                        true
                    }
                    Event::Key('\n') | Event::Key('\r') if self.password_len != 0 => {
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
                    Event::Key(character)
                        if character.is_ascii() && !character.is_ascii_control() =>
                    {
                        if self.password_len < self.password.len() {
                            self.password[self.password_len] = character as u8;
                            self.password_len += 1;
                            self.message = "Enter password and press Return";
                            true
                        } else {
                            false
                        }
                    }
                    Event::Click { x, y } | Event::MouseDown { x, y, .. } => {
                        let pt = Point::new(x, y);
                        if let Some((panel, cancel_btn, unlock_btn)) = self.ui_rects() {
                            if cancel_btn.contains(pt) {
                                self.clear_password();
                                self.state = PresenterState::Glance;
                                self.message = "Enter password and press Return";
                                return true;
                            } else if unlock_btn.contains(pt) {
                                if self.password_len != 0 {
                                    self.authenticate();
                                    return true;
                                }
                            } else if !panel.contains(pt) {
                                self.clear_password();
                                self.state = PresenterState::Glance;
                                self.message = "Enter password and press Return";
                                return true;
                            }
                        }
                        false
                    }
                    Event::Tick => {
                        let (_, _, _, _, _, s) = current_time();
                        if s != self.last_second {
                            self.last_second = s;
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                }
            }
            PresenterState::Authenticating => false,
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
        sunlight_ipc::debug_log("[LOCK] PRESENTER_HELLO rejected\n");
        sunlight_ipc::ProcessExit::exit(1);
    }
    // Register-IPC layout from mezzo::presenter_hello_reply:
    // word0 = generation | (safe_mode << 63)
    // word1 = uid | (gid << 32)
    // word2/3 = username bytes [0..16]
    let generation = hello.words[0] & !(1u64 << 63);
    let safe_mode = hello.words[0] >> 63 != 0;
    let session_uid = hello.words[1] as u32;
    let session_gid = (hello.words[1] >> 32) as u32;
    let display_authority = hello.caps[0];
    let mut username = [0u8; LOCK_SESSION_USERNAME_MAX];
    username[0..8].copy_from_slice(&hello.words[2].to_le_bytes());
    username[8..16].copy_from_slice(&hello.words[3].to_le_bytes());
    let username_len = username
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(16.min(username.len()));
    if username_len == 0 || generation == 0 {
        sunlight_ipc::debug_log("[LOCK] PRESENTER_HELLO missing session identity\n");
        sunlight_ipc::ProcessExit::exit(1);
    }
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
        sunlight_ipc::debug_log("[LOCK] LOCK_REGISTER_PRESENTER rejected\n");
        sunlight_ipc::ProcessExit::exit(1);
    }
    sunlight_ipc::debug_log("[LOCK] presenter registered with display\n");
    let notif_count = crate::active_notification_count();
    let wallpaper = match decode_simg(LOGIN_BG_SIMG) {
        Ok(img) => {
            sunlight_ipc::debug_log("[LOCK] login background loaded (simg-v2)\n");
            Some(img)
        }
        Err(_) => {
            sunlight_ipc::debug_log("[LOCK] login background decode failed; solid fallback\n");
            None
        }
    };
    let mut presenter = LockPresenter {
        mezzo,
        generation,
        safe_mode,
        session_uid,
        session_gid,
        username,
        username_len,
        password: [0; MAX_PASSWORD_LEN],
        password_len: 0,
        state: PresenterState::Glance,
        message: "Enter password and press Return",
        notif_count,
        wallpaper,
        width: metrics.width_px,
        height: metrics.height_px,
        last_second: 255,
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
        sunlight_ipc::debug_log("[LOCK] LOCK_PRESENTER_READY rejected\n");
        sunlight_ipc::ProcessExit::exit(1);
    }
    let acknowledged = ipc_call(
        mezzo,
        IpcMsg::with_label(MezzoMsg::PRESENTER_READY)
            .word(0, generation)
            .word(1, window.id()),
    );
    if acknowledged.label != MezzoMsg::REPLY {
        sunlight_ipc::debug_log("[LOCK] Mezzo PRESENTER_READY rejected\n");
        sunlight_ipc::ProcessExit::exit(1);
    }
    sunlight_ipc::debug_log("[LOCK] presenter ready\n");
    window.run(&mut presenter);
    presenter.clear_password();
    sunlight_ipc::ProcessExit::exit(0)
}
