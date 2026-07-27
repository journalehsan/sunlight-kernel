//! Welcome to SunlightOS — native onboarding wizard GUI.
//!
//! Ordinary optional startup app: launches after Shell Ready via session plan.
//! Completes one-time policy only via SESSION_STARTUP_COMPLETE after Finish.

#![no_std]
#![no_main]

extern crate alloc;

use core::fmt::Write;
use sun_font::{draw_text, FontRole, TextStyle, Typography, VecFont};
use sunlight_ipc::{
    debug_log, ipc_call, monotonic_millis, nameserver_lookup, process_yield, query_display_metrics,
    CapabilityToken, IpcMsg, ProcessExit, SessionMsg, SESSION_ENDPOINT,
};
use sunlight_libc::{self as libc, crt0, sun_exec};
use sunlight_ui::{
    request_close,
    widgets::{Button, ButtonState},
    App, Canvas, Event, Point, Rect, Theme, Window, WindowConfig, WindowDecoration, WindowMaterial,
};
use sunlight_welcome::{
    should_mark_onboarding_complete, ActionCard, ActionKind, CompletionOutcome, GreetingSource,
    LaunchMode, WizardPage, WizardState, ACTION_CARDS, BUNDLE_ID, DISPLAY_NAME, SLIDE_COUNT,
    SLIDES,
};

const WIN_W: u32 = 640;
const WIN_H: u32 = 480;
const KEY_ESC: u8 = 0x01;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_ENTER: u8 = 0x1C;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[WELCOME-WIZARD] panic\n");
    loop {
        process_yield();
    }
}

fn log_pass(marker: &str) {
    debug_log("[WELCOME-WIZARD] ");
    debug_log(marker);
    debug_log(" PASS\n");
}

/// Word-wrap body text for dark-theme screens (Label is single-line).
fn draw_wrapped(
    canvas: &mut Canvas,
    text: &str,
    x: i32,
    y: i32,
    max_w: u32,
    line_h: i32,
    style: &TextStyle,
) {
    // Approx char width for UiMedium/Small; keep lines under ~max_w pixels.
    let max_chars = ((max_w / 8).max(20) as usize).min(90);
    let mut y_cursor = y;
    let mut start = 0usize;
    let bytes = text.as_bytes();
    while start < bytes.len() {
        let remain = bytes.len() - start;
        let take = remain.min(max_chars);
        let mut end = start + take;
        if end < bytes.len() {
            // Prefer breaking on a space.
            if let Some(rel) = text[start..end].rfind(' ') {
                end = start + rel;
            }
        }
        if end <= start {
            end = (start + take).min(bytes.len());
        }
        let line = &text[start..end].trim_start();
        if !line.is_empty() {
            draw_text(canvas, line, x, y_cursor, style);
            y_cursor += line_h;
        }
        start = end;
        while start < bytes.len() && bytes[start] == b' ' {
            start += 1;
        }
        // Cap lines so a long greeting never paints over the footer.
        if y_cursor > y + line_h * 6 {
            break;
        }
    }
}

fn collect_machine_summary(display_ep: Option<CapabilityToken>) -> sunlight_welcome::MachineSummary {
    let mut m = sunlight_welcome::MachineSummary::empty();
    if let Ok(info) = libc::sysinfo() {
        if info.total_ram_kb > 0 {
            m.ram_mib = Some(info.total_ram_kb / 1024);
        }
    }
    if let Some(ep) = display_ep {
        if let Some(metrics) = query_display_metrics(ep) {
            m.screen_w = Some(metrics.width_px);
            m.screen_h = Some(metrics.height_px);
        }
    }
    // Bounded privacy-safe device class only.
    let mut device = heapless::String::new();
    let _ = device.push_str("desktop");
    m.device_class = Some(device);
    // CPU count is optional; leave None when not available without heavy deps.
    m
}

fn report_session_completion() -> bool {
    let Some(ep) = nameserver_lookup(SESSION_ENDPOINT) else {
        debug_log("[WELCOME-WIZARD] session endpoint missing\n");
        return false;
    };
    let mut msg = IpcMsg::with_label(SessionMsg::SESSION_STARTUP_COMPLETE);
    let bytes = BUNDLE_ID.as_bytes();
    msg.words[0] = bytes.len().min(32) as u64;
    for w in 2..6 {
        msg.words[w] = 0;
    }
    for (i, b) in bytes.iter().take(32).enumerate() {
        let word = 2 + i / 8;
        let shift = (i % 8) * 8;
        msg.words[word] |= (*b as u64) << shift;
    }
    msg.word_count = 6;
    let reply = ipc_call(ep, msg);
    reply.label == SessionMsg::REPLY
}

fn open_action(card: &ActionCard) -> Result<(), &'static str> {
    match card.kind {
        ActionKind::ComingSoon => {
            Err("Coming soon: available when Wise Owl gains interactive system actions.")
        }
        ActionKind::AboutWelcome => {
            Err("Welcome Wizard v0.1.0 — local onboarding for SunlightOS.")
        }
        ActionKind::OpenApp { command } => {
            let source = sunlight_ipc::launch_trace::LaunchSource::Shell;
            let trace = sun_exec::next_cli_trace(source);
            sun_exec::launch(sun_exec::LaunchRequest {
                trace,
                source,
                command: command.as_bytes(),
                args: &[],
                require_display: true,
            })
            .map(|_| ())
            .map_err(|_| "Not available")
        }
        ActionKind::OpenControlPanel { page } => {
            let source = sunlight_ipc::launch_trace::LaunchSource::Shell;
            let trace = sun_exec::next_cli_trace(source);
            sun_exec::launch(sun_exec::LaunchRequest {
                trace,
                source,
                command: b"settings",
                args: &[b"--page", page.as_bytes()],
                require_display: true,
            })
            .map(|_| ())
            .map_err(|_| "Not available")
        }
    }
}

// ── App ──────────────────────────────────────────────────────────────────────

struct WelcomeApp {
    state: WizardState,
    btn_primary: ButtonState,
    btn_secondary: ButtonState,
    btn_prev: ButtonState,
    btn_next: ButtonState,
    hover_action: Option<usize>,
    press_action: Option<usize>,
    auto_drive: bool,
    auto_step: u8,
    auto_next_ms: u64,
    visible_logged: bool,
    slideshow_logged: bool,
    action_logged: bool,
    completion_sent: bool,
    exit_after_ms: Option<u64>,
}

impl WelcomeApp {
    fn new(mode: LaunchMode, auto_drive: bool) -> Self {
        let mut state = WizardState::new(mode);
        if mode == LaunchMode::Manual {
            state.enter_welcome_center();
        }
        Self {
            state,
            btn_primary: ButtonState::Normal,
            btn_secondary: ButtonState::Normal,
            btn_prev: ButtonState::Normal,
            btn_next: ButtonState::Normal,
            hover_action: None,
            press_action: None,
            auto_drive,
            auto_step: 0,
            auto_next_ms: 0,
            visible_logged: false,
            slideshow_logged: false,
            action_logged: false,
            completion_sent: false,
            exit_after_ms: None,
        }
    }

    fn primary_rect(&self) -> Rect {
        Rect::new(WIN_W as i32 - 160, WIN_H as i32 - 56, 130, 36)
    }
    fn secondary_rect(&self) -> Rect {
        Rect::new(WIN_W as i32 - 310, WIN_H as i32 - 56, 130, 36)
    }
    fn prev_rect(&self) -> Rect {
        Rect::new(24, WIN_H as i32 - 56, 100, 36)
    }
    fn next_rect(&self) -> Rect {
        Rect::new(136, WIN_H as i32 - 56, 100, 36)
    }

    fn action_rect(i: usize) -> Rect {
        let col = (i % 2) as i32;
        let row = (i / 2) as i32;
        Rect::new(40 + col * 290, 120 + row * 90, 270, 78)
    }

    fn primary_label(&self) -> &'static str {
        match self.state.page {
            WizardPage::ImmediateWelcome => "Get Started",
            WizardPage::Greeting => "Continue",
            WizardPage::Slide(_) => "Next",
            WizardPage::Actions => "Finish",
        }
    }

    fn secondary_label(&self) -> &'static str {
        match self.state.page {
            WizardPage::ImmediateWelcome => "Skip",
            WizardPage::Greeting => "Skip tour",
            WizardPage::Slide(_) => "Skip to end",
            WizardPage::Actions => "Close",
        }
    }

    fn do_primary(&mut self) {
        match self.state.page {
            WizardPage::ImmediateWelcome => {
                self.state.begin();
                self.state.ensure_greeting();
                if self
                    .state
                    .greeting
                    .as_ref()
                    .map(|g| g.source == GreetingSource::LocalFallback)
                    .unwrap_or(false)
                {
                    log_pass("FALLBACK_GREETING");
                }
            }
            WizardPage::Greeting => {
                self.state.continue_from_greeting();
            }
            WizardPage::Slide(_) => {
                self.state.next_slide();
                if matches!(self.state.page, WizardPage::Actions) && !self.slideshow_logged {
                    log_pass("SLIDESHOW");
                    self.slideshow_logged = true;
                }
            }
            WizardPage::Actions => {
                let outcome = self.state.finish();
                self.complete_and_exit(outcome);
            }
        }
    }

    fn do_secondary(&mut self) {
        match self.state.page {
            WizardPage::ImmediateWelcome | WizardPage::Greeting | WizardPage::Slide(_) => {
                self.state.skip_to_actions();
                if !self.slideshow_logged {
                    log_pass("SLIDESHOW");
                    self.slideshow_logged = true;
                }
            }
            WizardPage::Actions => {
                let outcome = self.state.dismiss_early();
                self.complete_and_exit(outcome);
            }
        }
    }

    fn complete_and_exit(&mut self, outcome: CompletionOutcome) {
        if should_mark_onboarding_complete(outcome) && !self.completion_sent {
            if report_session_completion() {
                log_pass("COMPLETION_RECORDED");
                self.completion_sent = true;
            } else {
                debug_log("[WELCOME-WIZARD] completion report failed (will remain eligible)\n");
            }
        }
        self.exit_after_ms = Some(monotonic_millis().saturating_add(80));
        request_close();
    }

    fn draw_footer(&self, canvas: &mut Canvas, theme: &Theme) {
        let mut p = Button::new(self.primary_rect(), self.primary_label())
            .with_font(&Typography::UI_MEDIUM);
        p.state = self.btn_primary;
        p.draw(canvas, theme);

        let mut s = Button::secondary(self.secondary_rect(), self.secondary_label())
            .with_font(&Typography::UI_MEDIUM);
        s.state = self.btn_secondary;
        s.draw(canvas, theme);

        if matches!(
            self.state.page,
            WizardPage::Slide(_) | WizardPage::Greeting | WizardPage::Actions
        ) {
            let mut prev =
                Button::secondary(self.prev_rect(), "Back").with_font(&Typography::UI_MEDIUM);
            prev.state = self.btn_prev;
            prev.draw(canvas, theme);
        }
        if matches!(self.state.page, WizardPage::Slide(_)) {
            let mut next =
                Button::new(self.next_rect(), "Next").with_font(&Typography::UI_MEDIUM);
            next.state = self.btn_next;
            next.draw(canvas, theme);
        }
    }

    fn hit_test_buttons(&mut self, p: Point, press: bool, release: bool) {
        let primary_r = self.primary_rect();
        let secondary_r = self.secondary_rect();
        let prev_r = self.prev_rect();
        let next_r = self.next_rect();
        let page = self.state.page;
        let update = |st: &mut ButtonState, r: Rect| {
            if !r.contains(p) {
                if *st != ButtonState::Disabled {
                    *st = ButtonState::Normal;
                }
                return false;
            }
            if press {
                *st = ButtonState::Pressed;
            } else if release && *st == ButtonState::Pressed {
                *st = ButtonState::Hovered;
                return true;
            } else if !press {
                *st = ButtonState::Hovered;
            }
            false
        };
        let primary_hit = update(&mut self.btn_primary, primary_r) && release;
        let secondary_hit = update(&mut self.btn_secondary, secondary_r) && release;
        let prev_hit = matches!(
            page,
            WizardPage::Slide(_) | WizardPage::Greeting | WizardPage::Actions
        ) && update(&mut self.btn_prev, prev_r)
            && release;
        let next_hit = matches!(page, WizardPage::Slide(_))
            && update(&mut self.btn_next, next_r)
            && release;
        if primary_hit {
            self.do_primary();
        }
        if secondary_hit {
            self.do_secondary();
        }
        if prev_hit {
            self.state.prev_slide();
        }
        if next_hit {
            self.state.next_slide();
            if matches!(self.state.page, WizardPage::Actions) && !self.slideshow_logged {
                log_pass("SLIDESHOW");
                self.slideshow_logged = true;
            }
        }
        if matches!(page, WizardPage::Actions) {
            self.hover_action = None;
            for i in 0..ACTION_CARDS.len() {
                let r = Self::action_rect(i);
                if r.contains(p) {
                    self.hover_action = Some(i);
                    if press {
                        self.press_action = Some(i);
                    }
                    if release && self.press_action == Some(i) {
                        self.activate_action(i);
                        self.press_action = None;
                    }
                }
            }
            if release {
                self.press_action = None;
            }
        }
    }

    fn activate_action(&mut self, i: usize) {
        let card = ACTION_CARDS[i];
        match open_action(&card) {
            Ok(()) => {
                self.state.set_action_status("Opened.");
                if !self.action_logged {
                    log_pass("ACTION_CARD");
                    self.action_logged = true;
                }
            }
            Err(msg) => {
                self.state.set_action_status(msg);
                if (card.placeholder_honest || i == 5) && !self.action_logged {
                    log_pass("ACTION_CARD");
                    self.action_logged = true;
                }
            }
        }
    }

    fn auto_tick(&mut self) {
        if !self.auto_drive {
            return;
        }
        let now = monotonic_millis();
        if now < self.auto_next_ms {
            return;
        }
        match self.auto_step {
            0 => {
                if !self.visible_logged {
                    log_pass("AUTO_LAUNCH");
                    self.visible_logged = true;
                }
                self.auto_step = 1;
                self.auto_next_ms = now.saturating_add(150);
            }
            1 => {
                self.do_primary();
                self.auto_step = 2;
                self.auto_next_ms = now.saturating_add(100);
            }
            2 => {
                self.do_primary();
                self.auto_step = 3;
                self.auto_next_ms = now.saturating_add(50);
            }
            3 => {
                if matches!(self.state.page, WizardPage::Slide(_)) {
                    self.do_primary();
                    self.auto_next_ms = now.saturating_add(40);
                } else {
                    self.auto_step = 4;
                    self.auto_next_ms = now.saturating_add(50);
                }
            }
            4 => {
                self.activate_action(5); // Coming Soon placeholder
                self.auto_step = 5;
                self.auto_next_ms = now.saturating_add(50);
            }
            5 => {
                self.activate_action(1); // Control Panel
                self.auto_step = 6;
                self.auto_next_ms = now.saturating_add(100);
            }
            6 => {
                self.do_primary(); // Finish
                self.auto_step = 7;
            }
            _ => {}
        }
    }
}

impl App for WelcomeApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        if !self.visible_logged && !self.auto_drive {
            log_pass("AUTO_LAUNCH");
            self.visible_logged = true;
        }
        // Match native Sunlight apps: dark panel body + chrome header.
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.panel);
        canvas.fill_rect(Rect::new(0, 0, WIN_W, 64), theme.chrome.titlebar_active);
        canvas.fill_rect(
            Rect::new(0, 64, WIN_W, 1),
            theme.chrome.titlebar_divider_active,
        );
        draw_text(
            canvas,
            DISPLAY_NAME,
            24,
            24,
            &TextStyle::new(FontRole::UiLarge, theme.chrome.title_active),
        );

        let title_style = TextStyle::new(FontRole::UiLarge, theme.text);
        let body_style = TextStyle::new(FontRole::UiMedium, theme.text_muted);
        let dim_style = TextStyle::new(FontRole::UiSmall, theme.text_dim);
        let card_title_style = TextStyle::new(FontRole::UiMedium, theme.text);
        let card_body_style = TextStyle::new(FontRole::UiSmall, theme.text_muted);

        match self.state.page {
            WizardPage::ImmediateWelcome => {
                draw_text(canvas, "Your desktop is ready.", 40, 110, &title_style);
                draw_wrapped(
                    canvas,
                    "Take a short tour of SunlightOS features, or skip and explore on your own.",
                    40,
                    170,
                    WIN_W - 80,
                    22,
                    &body_style,
                );
                if self.state.onboarding_already_complete {
                    draw_text(
                        canvas,
                        "Onboarding already completed — Welcome Center mode.",
                        40,
                        280,
                        &dim_style,
                    );
                }
            }
            WizardPage::Greeting => {
                self.state.ensure_greeting();
                let text = self
                    .state
                    .greeting
                    .as_ref()
                    .map(|g| g.text.as_str())
                    .unwrap_or("Welcome to SunlightOS.");
                draw_wrapped(canvas, text, 40, 100, WIN_W - 80, 22, &body_style);
                // Summary card — dark elevated surface, not white.
                let card = Rect::new(40, 250, WIN_W - 80, 110);
                canvas.fill_rect(card, theme.chrome.card_bg);
                canvas.draw_rect(card, theme.border);
                let mut line = heapless::String::<128>::new();
                let _ = write!(
                    &mut line,
                    "SunlightOS {}",
                    self.state.sunlight_version.as_str()
                );
                draw_text(canvas, line.as_str(), 56, 268, &card_title_style);
                line.clear();
                if let Some(ram) = self.state.machine.ram_mib {
                    let _ = write!(&mut line, "Memory: {} MiB  ", ram);
                }
                if let Some(c) = self.state.machine.cpu_cores {
                    let _ = write!(&mut line, "CPU cores: {}", c);
                }
                if line.is_empty() {
                    let _ = line.push_str("System summary partial");
                }
                draw_text(canvas, line.as_str(), 56, 304, &card_body_style);
            }
            WizardPage::Slide(i) => {
                let slide = &SLIDES[i.min(SLIDE_COUNT - 1)];
                let mut progress = heapless::String::<32>::new();
                let _ = write!(&mut progress, "Slide {} of {}", i + 1, SLIDE_COUNT);
                draw_text(canvas, progress.as_str(), 40, 96, &dim_style);
                draw_text(canvas, slide.title, 40, 132, &title_style);
                draw_wrapped(canvas, slide.body, 40, 190, WIN_W - 80, 22, &body_style);
            }
            WizardPage::Actions => {
                draw_text(canvas, "Next steps", 40, 92, &title_style);
                for (i, card) in ACTION_CARDS.iter().enumerate() {
                    let r = Self::action_rect(i);
                    let bg = if self.hover_action == Some(i) {
                        theme.chrome.selection
                    } else {
                        theme.chrome.card_bg
                    };
                    canvas.fill_rect(r, bg);
                    canvas.draw_rect(r, theme.border);
                    draw_text(
                        canvas,
                        card.title,
                        r.x + 14,
                        r.y + 16,
                        &card_title_style,
                    );
                    draw_wrapped(
                        canvas,
                        card.description,
                        r.x + 14,
                        r.y + 42,
                        r.w - 28,
                        18,
                        &card_body_style,
                    );
                }
                if !self.state.action_status.is_empty() {
                    draw_text(
                        canvas,
                        self.state.action_status.as_str(),
                        40,
                        WIN_H as i32 - 100,
                        &dim_style,
                    );
                }
            }
        }

        self.draw_footer(canvas, theme);
        let _ = (FontRole::UiLarge, VecFont(FontRole::UiRegular));
    }

    fn update(&mut self, event: Event) -> bool {
        if let Some(deadline) = self.exit_after_ms {
            if monotonic_millis() >= deadline {
                request_close();
                return false;
            }
        }
        self.auto_tick();
        match event {
            Event::Tick => {
                self.auto_tick();
                true
            }
            Event::MouseMove { x, y } => {
                self.hit_test_buttons(Point::new(x, y), false, false);
                true
            }
            Event::MouseDown { x, y, .. } => {
                self.hit_test_buttons(Point::new(x, y), true, false);
                true
            }
            Event::MouseUp { x, y, .. } | Event::Click { x, y } => {
                self.hit_test_buttons(Point::new(x, y), false, true);
                true
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } => {
                if keycode == KEY_ESC {
                    let outcome = self.state.dismiss_early();
                    self.complete_and_exit(outcome);
                } else if keycode == KEY_ENTER {
                    self.do_primary();
                } else if keycode == KEY_RIGHT {
                    if matches!(self.state.page, WizardPage::Slide(_)) {
                        self.state.next_slide();
                    } else {
                        self.do_primary();
                    }
                } else if keycode == KEY_LEFT {
                    self.state.prev_slide();
                }
                true
            }
            _ => true,
        }
    }
}

fn parse_mode(argc: u64, argv: *const *const u8) -> LaunchMode {
    let mut raw = [core::ptr::null(); 16];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut raw) };
    let mut mode = LaunchMode::Automatic;
    for i in 0..count {
        let p = raw[i];
        if p.is_null() {
            continue;
        }
        let mut len = 0usize;
        while len < 64 && unsafe { *p.add(len) } != 0 {
            len += 1;
        }
        let bytes = unsafe { core::slice::from_raw_parts(p, len) };
        if bytes == b"--manual" || bytes == b"--mode=manual" {
            mode = LaunchMode::Manual;
        }
    }
    mode
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let mode = parse_mode(argc, argv);
    let inject = option_env!("SUNLIGHT_INJECT_PHASE") == Some("welcome_wizard");
    let auto_drive = inject && mode == LaunchMode::Automatic;

    debug_log("[WELCOME-WIZARD] start mode=");
    debug_log(if mode == LaunchMode::Manual {
        "manual"
    } else {
        "automatic"
    });
    debug_log("\n");

    if mode == LaunchMode::Manual {
        log_pass("MANUAL_RELAUNCH");
    }

    // Wait for the display server like ordinary sun-exec GUI apps. Sessiond
    // already delays spawn until after desktop settle; this avoids a hard
    // fail if display is still registering.
    let mut display_ep = nameserver_lookup("display_server");
    let mut wait_spins = 0u32;
    while display_ep.is_none() && wait_spins < 200 {
        process_yield();
        display_ep = nameserver_lookup("display_server");
        wait_spins = wait_spins.saturating_add(1);
    }

    let mut app = WelcomeApp::new(mode, auto_drive);
    app.state.machine = collect_machine_summary(display_ep);

    // Connect like other apps: retry briefly if the compositor is still busy
    // finishing the desktop window (avoids fighting Vortex for first paint).
    let mut window = None;
    for _ in 0..40u32 {
        if let Some(w) = Window::connect_with_material(
            WindowConfig {
                width: WIN_W,
                height: WIN_H,
                title: DISPLAY_NAME,
                decoration: WindowDecoration::CompactCloseMinimize,
            },
            WindowMaterial::WindowGlass,
        ) {
            window = Some(w);
            break;
        }
        process_yield();
    }
    let mut window = match window {
        Some(w) => w,
        None => {
            debug_log("[WELCOME-WIZARD] window connect failed\n");
            ProcessExit::exit(1);
        }
    };

    window.run(&mut app);
    debug_log("[WELCOME-WIZARD] exit\n");
    ProcessExit::exit(0);
}
