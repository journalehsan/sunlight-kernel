//! SunlightOS Control Panel — Day 22/23 first version.
//!
//! Grid of settings cards. Currently:
//!   • Mouse — pointer sensitivity slider (1-10) + acceleration toggle.
//!     Sends SET_MOUSE_SETTINGS to display_server on Apply.
//!   • Monitor — shows current screen resolution; mode switching is TODO.
//!   • Window Behavior — future placeholder for titlebar double-click policy
//!     and workspace-related preferences.
//!
//! Icons: SunlightOS icon theme (Breeze-inspired, TGA format).
//!   Cards display the preferences-desktop-mouse and preferences-desktop-display
//!   icons from /var/sunlightos/icons/SunlightOS (embedded at compile time).
//!
//! TODO: Persist settings via sunlight-kv once persistent config is available.
//! TODO: Monitor mode switching (VMware/QEMU virtual display mode-set).

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, ipc_call,
    launch_trace::{self, LaunchSource, LaunchTrace},
    nameserver_lookup, process_yield, show_notification, CapabilityToken, IpcMsg, NotificationKind,
    ProcessExit, SgpMsg,
};
use sunlight_ui::{
    image::TgaImage,
    request_close,
    widgets::{Button, ButtonState, Checkbox, Label, Panel, Slider},
    App, Canvas, Color, Event, HBox, Point, Rect, Theme, VBox, Window, WindowConfig,
};

// ---------------------------------------------------------------------------
// Icon theme assets (embedded at compile time)
// ---------------------------------------------------------------------------

static ICON_MOUSE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/input-mouse.tga");
static ICON_MONITOR_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/video-display.tga");
static ICON_SETTINGS_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/preferences-system.tga");

const WIN_W: u32 = 500;
const WIN_H: u32 = 330;
const FP_ONE: i32 = 65536;

// Map slider value 1-10 to a pointer sensitivity fixed-point multiplier.
// Slider 5 → 1.0× (FP_ONE).  Range: ~0.6× (1) … ~1.8× (10).
fn slider_to_fp(v: u32) -> i32 {
    FP_ONE + (v.clamp(1, 10) as i32 - 5) * (FP_ONE / 5)
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Grid,
    Mouse,
    Monitor,
}

struct ControlPanelApp {
    page: Page,
    display_ep: Option<CapabilityToken>,
    screen_w: u32,
    screen_h: u32,
    sens_slider: Slider,
    accel_cb: Checkbox<'static>,
    status_msg: [u8; 40],
    status_len: usize,
    /// TGA icon for the Mouse settings card / page.
    icon_mouse: Option<TgaImage>,
    /// TGA icon for the Monitor settings card / page.
    icon_monitor: Option<TgaImage>,
}

impl ControlPanelApp {
    fn new(display_ep: Option<CapabilityToken>, screen_w: u32, screen_h: u32) -> Self {
        let sens_slider = Slider::horizontal(Rect::default())
            .with_range(1, 10)
            .with_value(5);
        let mut accel_cb = Checkbox::new(Rect::default(), "Enable pointer acceleration");
        accel_cb.checked = true;
        Self {
            page: Page::Grid,
            display_ep,
            screen_w,
            screen_h,
            sens_slider,
            accel_cb,
            status_msg: [0u8; 40],
            status_len: 0,
            icon_mouse: TgaImage::parse(ICON_MOUSE_TGA).ok(),
            icon_monitor: TgaImage::parse(ICON_MONITOR_TGA).ok(),
        }
    }

    fn set_status(&mut self, msg: &[u8]) {
        let n = msg.len().min(self.status_msg.len());
        self.status_msg[..n].copy_from_slice(&msg[..n]);
        self.status_len = n;
    }

    fn status_str(&self) -> &str {
        core::str::from_utf8(&self.status_msg[..self.status_len]).unwrap_or("")
    }

    fn apply_mouse_settings(&mut self) {
        let Some(ep) = self.display_ep else {
            self.set_status(b"Display server unavailable");
            return;
        };
        let sens_fp = slider_to_fp(self.sens_slider.value);
        let accel = self.accel_cb.checked as u64;
        let reply = ipc_call(
            ep,
            IpcMsg::with_label(SgpMsg::SET_MOUSE_SETTINGS)
                .word(0, sens_fp as u64)
                .word(1, accel),
        );
        if reply.label == SgpMsg::REPLY {
            self.set_status(b"Settings applied");
            let _ = show_notification(
                NotificationKind::Info,
                "Control Panel",
                "Mouse settings applied",
                3000,
            );
        } else {
            self.set_status(b"Apply failed");
        }
    }

    // -----------------------------------------------------------------------
    // Grid page
    // -----------------------------------------------------------------------

    fn card_rects(&self) -> (Rect, Rect) {
        let card_w = 180u32;
        let card_h = 180u32;
        let gap = 24i32;
        let start_x = (WIN_W as i32 - (card_w * 2) as i32 - gap) / 2;
        let card_y = 56i32;
        let r1 = Rect::new(start_x, card_y, card_w, card_h);
        let r2 = Rect::new(start_x + card_w as i32 + gap, card_y, card_w, card_h);
        (r1, r2)
    }

    fn draw_card(
        canvas: &mut Canvas,
        theme: &Theme,
        rect: Rect,
        icon_color: Color,
        label: &str,
        sublabel: &str,
        tga_icon: Option<TgaImage>,
    ) {
        canvas.fill_rect(rect, theme.panel);
        canvas.draw_rect(rect, theme.border);

        let ix = rect.x + rect.w as i32 / 2 - 24;
        let iy = rect.y + 22;
        let icon_rect = Rect::new(ix, iy, 48, 48);

        if let Some(tga) = tga_icon {
            // Draw TGA icon with alpha compositing over the panel background.
            canvas.draw_tga_icon(&tga, icon_rect);
        } else {
            // Fallback: solid color square + inner highlight.
            canvas.fill_rect(icon_rect, icon_color);
            let inner = Rect::new(ix + 12, iy + 12, 24, 24);
            canvas.fill_rect(inner, theme.bg);
        }

        let label_rect = Rect::new(rect.x + 8, rect.bottom() - 42, rect.w - 16, 18);
        let sub_rect = Rect::new(rect.x + 8, rect.bottom() - 24, rect.w - 16, 14);
        Label::new(label_rect, label).draw(canvas, theme);
        Label::new(sub_rect, sublabel).draw(canvas, theme);
    }

    fn draw_grid(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        // Header bar
        let header = Rect::new(0, 0, WIN_W, 44);
        canvas.fill_rect(header, theme.panel);
        canvas.draw_rect(Rect::new(0, 43, WIN_W, 1), theme.border);

        // Header: settings icon + title
        if let Ok(tga) = TgaImage::parse(ICON_SETTINGS_TGA) {
            canvas.draw_tga_icon(&tga, Rect::new(8, 6, 32, 32));
        }
        Label::new(Rect::new(46, 12, WIN_W - 58, 20), "System Preferences").draw(canvas, theme);

        let (c1, c2) = self.card_rects();
        Self::draw_card(
            canvas,
            theme,
            c1,
            theme.accent,
            "Mouse",
            "Pointer & Acceleration",
            self.icon_mouse,
        );
        Self::draw_card(
            canvas,
            theme,
            c2,
            Color(0xFF_50_88_CC),
            "Monitor",
            "Resolution & Display",
            self.icon_monitor,
        );
    }

    fn update_grid(&mut self, event: Event) -> bool {
        if let Event::Click { x, y } = event {
            let pt = Point::new(x, y);
            let (c1, c2) = self.card_rects();
            if c1.contains(pt) {
                self.page = Page::Mouse;
                self.status_len = 0;
                return true;
            }
            if c2.contains(pt) {
                self.page = Page::Monitor;
                return true;
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Mouse settings page
    // -----------------------------------------------------------------------

    fn draw_mouse_page(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        let content = Rect::new(12, 12, WIN_W - 24, WIN_H - 24);
        Panel::with_title(content, "Mouse").draw(canvas, theme);

        let inner = Rect::new(
            content.x + 14,
            content.y + 32,
            content.w - 28,
            content.h - 46,
        );

        let heights = [16u32, 28, 24, 18, 28];
        let mut rows = VBox::new(inner).with_spacing(14).layout(&heights);
        let desc_row = rows.next().unwrap_or_default();
        let slider_row = rows.next().unwrap_or_default();
        let accel_row = rows.next().unwrap_or_default();
        let status_row = rows.next().unwrap_or_default();
        let actions_row = rows.next().unwrap_or_default();

        Label::new(desc_row, "Adjust pointer speed and acceleration.").draw(canvas, theme);

        // Slider row
        let sc_widths = [110u32, slider_row.w.saturating_sub(180), 60];
        let mut sc = HBox::new(slider_row).with_spacing(8).layout(&sc_widths);
        let slabel_r = sc.next().unwrap_or_default();
        let sslider_r = sc.next().unwrap_or_default();
        let shint_r = sc.next().unwrap_or_default();

        Label::new(slabel_r, "Pointer Speed:").draw(canvas, theme);
        self.sens_slider.rect = sslider_r;
        self.sens_slider.draw(canvas, theme);

        let hint = match self.sens_slider.value {
            1 => "Slowest",
            2 | 3 => "Slow",
            4 => "Moderate",
            5 => "Default",
            6 | 7 => "Fast",
            8 | 9 => "Faster",
            _ => "Fastest",
        };
        Label::new(shint_r, hint).draw(canvas, theme);

        self.accel_cb.rect = accel_row;
        self.accel_cb.draw(canvas, theme);

        if self.status_len > 0 {
            Label::new(status_row, self.status_str()).draw(canvas, theme);
        }

        // Buttons
        let back_r = Rect::new(actions_row.x, actions_row.y, 80, actions_row.h);
        let apply_r = Rect::new(actions_row.right() - 80, actions_row.y, 80, actions_row.h);

        let mut back = Button::secondary(back_r, "Back");
        back.state = ButtonState::Normal;
        back.draw(canvas, theme);

        let mut apply = Button::new(apply_r, "Apply");
        apply.state = ButtonState::Normal;
        apply.draw(canvas, theme);
    }

    fn mouse_action_rects(&self) -> (Rect, Rect) {
        let content = Rect::new(12, 12, WIN_W - 24, WIN_H - 24);
        let inner = Rect::new(
            content.x + 14,
            content.y + 32,
            content.w - 28,
            content.h - 46,
        );
        let heights = [16u32, 28, 24, 18, 28];
        let mut rows = VBox::new(inner).with_spacing(14).layout(&heights);
        for _ in 0..4 {
            rows.next();
        }
        let actions_row = rows.next().unwrap_or_default();
        let back_r = Rect::new(actions_row.x, actions_row.y, 80, actions_row.h);
        let apply_r = Rect::new(actions_row.right() - 80, actions_row.y, 80, actions_row.h);
        (back_r, apply_r)
    }

    fn update_mouse_page(&mut self, event: Event) -> bool {
        let slider_changed = self.sens_slider.update(event);
        let accel_changed = self.accel_cb.update(event);
        if slider_changed || accel_changed {
            return true;
        }
        if let Event::Click { x, y } = event {
            let pt = Point::new(x, y);
            let (back_r, apply_r) = self.mouse_action_rects();
            if back_r.contains(pt) {
                self.page = Page::Grid;
                self.status_len = 0;
                return true;
            }
            if apply_r.contains(pt) {
                self.apply_mouse_settings();
                return true;
            }
        }
        false
    }

    // -----------------------------------------------------------------------
    // Monitor page
    // -----------------------------------------------------------------------

    fn draw_monitor_page(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        let content = Rect::new(12, 12, WIN_W - 24, WIN_H - 24);
        Panel::with_title(content, "Monitor").draw(canvas, theme);

        let inner = Rect::new(
            content.x + 14,
            content.y + 32,
            content.w - 28,
            content.h - 46,
        );
        let heights = [16u32, 18, 4, 18, 18, 18, 28];
        let mut rows = VBox::new(inner).with_spacing(8).layout(&heights);
        let desc_row = rows.next().unwrap_or_default();
        let cur_row = rows.next().unwrap_or_default();
        let _ = rows.next(); // spacer
        let opt1_row = rows.next().unwrap_or_default();
        let opt2_row = rows.next().unwrap_or_default();
        let opt3_row = rows.next().unwrap_or_default();
        let actions_row = rows.next().unwrap_or_default();

        Label::new(desc_row, "Screen resolution settings.").draw(canvas, theme);

        // Format current resolution
        let mut cur_buf = [0u8; 48];
        let cur_str = fmt_cur_res(self.screen_w, self.screen_h, &mut cur_buf);
        Label::new(cur_row, cur_str).draw(canvas, theme);

        // Placeholder resolution options (disabled)
        Label::new(opt1_row, "  [ ] 800 x 600    (TODO: mode switching)").draw(canvas, theme);
        Label::new(opt2_row, "  [ ] 1024 x 768   (TODO: mode switching)").draw(canvas, theme);
        Label::new(opt3_row, "  [ ] 1280 x 1024  (TODO: mode switching)").draw(canvas, theme);

        let back_r = Rect::new(actions_row.x, actions_row.y, 80, actions_row.h);
        let mut back = Button::secondary(back_r, "Back");
        back.state = ButtonState::Normal;
        back.draw(canvas, theme);
    }

    fn monitor_back_rect(&self) -> Rect {
        let content = Rect::new(12, 12, WIN_W - 24, WIN_H - 24);
        let inner = Rect::new(
            content.x + 14,
            content.y + 32,
            content.w - 28,
            content.h - 46,
        );
        let heights = [16u32, 18, 4, 18, 18, 18, 28];
        let mut rows = VBox::new(inner).with_spacing(8).layout(&heights);
        for _ in 0..6 {
            rows.next();
        }
        let actions_row = rows.next().unwrap_or_default();
        Rect::new(actions_row.x, actions_row.y, 80, actions_row.h)
    }

    fn update_monitor_page(&mut self, event: Event) -> bool {
        if let Event::Click { x, y } = event {
            let pt = Point::new(x, y);
            if self.monitor_back_rect().contains(pt) {
                self.page = Page::Grid;
                return true;
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// App trait impl
// ---------------------------------------------------------------------------

impl App for ControlPanelApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        match self.page {
            Page::Grid => self.draw_grid(canvas, theme),
            Page::Mouse => self.draw_mouse_page(canvas, theme),
            Page::Monitor => self.draw_monitor_page(canvas, theme),
        }
    }

    fn update(&mut self, event: Event) -> bool {
        if let Event::KeyPress {
            keycode: 0x10,
            pressed: true,
            ctrl: true,
            ..
        } = event
        {
            request_close();
            return true;
        }
        match self.page {
            Page::Grid => self.update_grid(event),
            Page::Mouse => self.update_mouse_page(event),
            Page::Monitor => self.update_monitor_page(event),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_u32_into(buf: &mut [u8], pos: &mut usize, n: u32) {
    let mut tmp = [0u8; 10];
    let mut i = 10usize;
    let mut n = n;
    loop {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &b in &tmp[i..] {
        if *pos < buf.len() {
            buf[*pos] = b;
            *pos += 1;
        }
    }
}

fn fmt_cur_res<'a>(w: u32, h: u32, buf: &'a mut [u8; 48]) -> &'a str {
    let prefix = b"Current: ";
    let mut pos = 0usize;
    for &b in prefix {
        buf[pos] = b;
        pos += 1;
    }
    write_u32_into(buf, &mut pos, w);
    if pos + 3 < buf.len() {
        buf[pos] = b' ';
        buf[pos + 1] = b'x';
        buf[pos + 2] = b' ';
        pos += 3;
    }
    write_u32_into(buf, &mut pos, h);
    core::str::from_utf8(&buf[..pos]).unwrap_or("???")
}

// ---------------------------------------------------------------------------
// Allocator + panic
// ---------------------------------------------------------------------------

struct NoAlloc;
unsafe impl core::alloc::GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}
#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[CONTROL-PANEL] panic\n");
    loop {
        process_yield();
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=control-panel",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );
    debug_log("[CONTROL-PANEL] starting\n");

    let display_ep = nameserver_lookup("display_server");
    let (screen_w, screen_h) = if let Some(ep) = display_ep {
        let reply = ipc_call(ep, IpcMsg::with_label(SgpMsg::GET_SCREEN_INFO));
        if reply.label == SgpMsg::REPLY {
            let packed = reply.words[0];
            let w = (packed & 0xFFFF_FFFF) as u32;
            let h = (packed >> 32) as u32;
            (w.max(640), h.max(480))
        } else {
            (1024, 768)
        }
    } else {
        (1024, 768)
    };

    let app = ControlPanelApp::new(display_ep, screen_w, screen_h);
    let mut app = app;

    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "System Preferences",
        decoration: sunlight_ui::WindowDecoration::Normal,
    }) {
        Some(w) => w,
        None => loop {
            process_yield();
        },
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}
