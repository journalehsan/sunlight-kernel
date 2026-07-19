//! SunlightOS Control Panel — System Preferences.
//!
//! Grid of settings cards:
//!   • Mouse — pointer sensitivity slider (1-10) + acceleration toggle.
//!   • Monitor — current screen resolution (read-only).
//!   • Wallpaper — desktop background picker.
//!   • Notifications — DND toggle.
//!   • About This Computer — hardware, memory, graphics, runtime snapshot.
//!   • About SunlightOS — OS, kernel, and core component identity.
//!
//! Direct page launch: `control-panel --page <name>` where name is one of
//! wallpaper, about-computer, about-os (also accepted: about-sunlightos).
//!
//! Icons: SunlightOS icon theme (Breeze-inspired, TGA format).

#![no_std]
#![no_main]

extern crate alloc;

mod about;
mod clipboard;
mod sysinfo;

use alloc::vec::Vec;
use core::alloc::GlobalAlloc;
use core::cmp;
use sun_font::{self, FontRole, TextStyle, Typography};
use sunlight_ipc::{
    debug_log, ipc_call,
    launch_trace::{self, LaunchSource, LaunchTrace},
    nameserver_lookup, notification_dnd_enabled, notification_set_dnd, process_yield,
    show_notification, CapabilityToken, IpcMsg, NotificationKind, ProcessExit, SgpMsg,
};
use sunlight_libc::crt0;
use sunlight_ui::{
    image::{draw_mono_icon, MonoIcon, TgaImage},
    request_close,
    widgets::{Button, ButtonState, Checkbox, Label, Panel, Slider},
    App, Canvas, Color, Event, HBox, Point, Rect, Theme, VBox, Window, WindowConfig,
};
use sunlight_wallpaper::{
    is_supported_wallpaper, load_desktop_config, save_desktop_config, scan_wallpapers,
    DesktopConfig, WallpaperEntry,
};

use about::{AboutAction, AboutPageState};
use sysinfo::{FixedStr, SystemInfoSnapshot};

// ---------------------------------------------------------------------------
// Icon theme assets (embedded at compile time)
// ---------------------------------------------------------------------------

static ICON_MOUSE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/input-mouse.tga");
static ICON_MONITOR_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/video-display.tga");
static ICON_WALLPAPER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/preferences-desktop-wallpaper.tga");
static ICON_SETTINGS_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/preferences-system.tga");
static ICON_COMPUTER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/computer.tga");
static ICON_ABOUT_OS_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/about.tga");
static ICON_PREFS_MONO_RAW: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icons/preferences-symbolic.raw"));
static ICON_SYM_DND_ON_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icons/do_not_disturb_on.tga"));
static ICON_SYM_DND_OFF_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icons/do_not_disturb_off.tga"));
static ICON_SYM_NOTIFICATIONS_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icons/notifications.tga"));
static ICON_SUNLIGHT_LOGO_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icons/sunlightos-logo.tga"));

const ICON_PREFS_MONO: MonoIcon<'static> = MonoIcon::new(16, 16, ICON_PREFS_MONO_RAW);

// Tall enough to show the complete About SunlightOS details without scrolling.
const WIN_W: u32 = 500;
const WIN_H: u32 = 560;
const FP_ONE: i32 = 65536;
const WALLPAPER_PREVIEW_W: u32 = 240;
const WALLPAPER_PREVIEW_H: u32 = 112;
const WALLPAPER_PREVIEW_MAX_BYTES: usize = 8 * 1024 * 1024;

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
    Wallpaper,
    Notifications,
    AboutComputer,
    AboutOs,
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
    wallpaper_status: [u8; 96],
    wallpaper_status_len: usize,
    /// TGA icon for the Mouse settings card / page.
    icon_mouse: Option<TgaImage>,
    /// TGA icon for the Monitor settings card / page.
    icon_monitor: Option<TgaImage>,
    icon_wallpaper: Option<TgaImage>,
    icon_notifications: Option<TgaImage>,
    icon_dnd_on: Option<TgaImage>,
    icon_dnd: Option<TgaImage>,
    icon_computer: Option<TgaImage>,
    icon_about_os: Option<TgaImage>,
    icon_logo: Option<TgaImage>,
    wallpaper_items: Vec<WallpaperEntry>,
    wallpaper_config: DesktopConfig,
    wallpaper_selected: usize,
    wallpaper_preview: Option<TgaImage>,
    wallpaper_pending_item: Option<usize>,
    sysinfo: SystemInfoSnapshot,
    about: AboutPageState,
}

impl ControlPanelApp {
    fn new(
        display_ep: Option<CapabilityToken>,
        screen_w: u32,
        screen_h: u32,
        initial_page: Page,
    ) -> Self {
        let sens_slider = Slider::horizontal(Rect::default())
            .with_range(1, 10)
            .with_value(5);
        let mut accel_cb = Checkbox::new(Rect::default(), "Enable pointer acceleration");
        accel_cb.checked = true;
        let wallpaper_config = load_desktop_config();
        let wallpaper_items = scan_wallpapers(&wallpaper_config.wallpaper);
        let wallpaper_selected = wallpaper_items
            .iter()
            .position(|it| it.selected)
            .unwrap_or(0);
        Self {
            page: initial_page,
            display_ep,
            screen_w,
            screen_h,
            sens_slider,
            accel_cb,
            status_msg: [0u8; 40],
            status_len: 0,
            wallpaper_status: [0u8; 96],
            wallpaper_status_len: 0,
            icon_mouse: TgaImage::parse(ICON_MOUSE_TGA).ok(),
            icon_monitor: TgaImage::parse(ICON_MONITOR_TGA).ok(),
            icon_wallpaper: TgaImage::parse(ICON_WALLPAPER_TGA).ok(),
            icon_notifications: TgaImage::parse(ICON_SYM_NOTIFICATIONS_TGA).ok(),
            icon_dnd_on: TgaImage::parse(ICON_SYM_DND_ON_TGA).ok(),
            icon_dnd: TgaImage::parse(ICON_SYM_DND_OFF_TGA).ok(),
            icon_computer: TgaImage::parse(ICON_COMPUTER_TGA).ok(),
            icon_about_os: TgaImage::parse(ICON_ABOUT_OS_TGA).ok(),
            icon_logo: TgaImage::parse(ICON_SUNLIGHT_LOGO_TGA).ok(),
            wallpaper_items,
            wallpaper_config,
            wallpaper_selected,
            wallpaper_preview: None,
            wallpaper_pending_item: None,
            sysinfo: SystemInfoSnapshot::collect(display_ep, screen_w, screen_h),
            about: AboutPageState::new(),
        }
    }

    fn refresh_sysinfo(&mut self) {
        self.sysinfo = SystemInfoSnapshot::collect(self.display_ep, self.screen_w, self.screen_h);
    }

    fn set_status(&mut self, msg: &[u8]) {
        let n = msg.len().min(self.status_msg.len());
        self.status_msg[..n].copy_from_slice(&msg[..n]);
        self.status_len = n;
    }

    fn status_str(&self) -> &str {
        core::str::from_utf8(&self.status_msg[..self.status_len]).unwrap_or("")
    }

    fn draw_label(canvas: &mut Canvas, rect: Rect, text: &str, theme: &Theme, role: FontRole) {
        sun_font::draw_text_vcenter(
            canvas,
            text,
            rect.x,
            rect.y,
            rect.h,
            &TextStyle::new(role, theme.text),
        );
    }

    fn draw_dim_label(canvas: &mut Canvas, rect: Rect, text: &str, theme: &Theme, role: FontRole) {
        sun_font::draw_text_vcenter(
            canvas,
            text,
            rect.x,
            rect.y,
            rect.h,
            &TextStyle::new(role, theme.text_dim),
        );
    }

    fn draw_button(canvas: &mut Canvas, theme: &Theme, button: Button<'_>) {
        button.with_font(&Typography::UI_MEDIUM).draw(canvas, theme);
    }

    fn wallpaper_status_str(&self) -> &str {
        core::str::from_utf8(&self.wallpaper_status[..self.wallpaper_status_len]).unwrap_or("")
    }

    fn set_wallpaper_status(&mut self, msg: &str) {
        let bytes = msg.as_bytes();
        let n = bytes.len().min(self.wallpaper_status.len());
        self.wallpaper_status[..n].copy_from_slice(&bytes[..n]);
        self.wallpaper_status_len = n;
    }

    fn refresh_wallpaper_preview(&mut self) {
        if self.wallpaper_items.is_empty() {
            self.set_wallpaper_status("No wallpapers found");
            return;
        }
        let path = self.wallpaper_items[self.wallpaper_selected]
            .preview_path
            .clone();
        let Some(bytes) = read_preview_wallpaper_bytes(path.as_bytes()) else {
            // Keep the previous valid preview (if any) and surface the problem.
            self.set_wallpaper_status("Selected wallpaper missing");
            return;
        };
        if !is_supported_wallpaper(bytes) {
            self.set_wallpaper_status("Unsupported or corrupt wallpaper");
            return;
        }
        match TgaImage::parse(bytes) {
            Ok(img) => {
                self.wallpaper_preview = Some(img);
                self.wallpaper_status_len = 0;
            }
            Err(_) => {
                self.wallpaper_preview = None;
                self.set_wallpaper_status("Unsupported or corrupt wallpaper");
            }
        }
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

    fn card_rects(&self) -> [Rect; 6] {
        let card_w = 136u32;
        let card_h = 110u32;
        let gap = 14i32;
        let start_x = (WIN_W as i32 - (card_w * 3) as i32 - gap * 2) / 2;
        let card_y = 52i32;
        let row2_y = card_y + card_h as i32 + 12;
        [
            Rect::new(start_x, card_y, card_w, card_h),
            Rect::new(start_x + card_w as i32 + gap, card_y, card_w, card_h),
            Rect::new(start_x + (card_w as i32 + gap) * 2, card_y, card_w, card_h),
            Rect::new(start_x, row2_y, card_w, card_h),
            Rect::new(start_x + card_w as i32 + gap, row2_y, card_w, card_h),
            Rect::new(start_x + (card_w as i32 + gap) * 2, row2_y, card_w, card_h),
        ]
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
        Self::draw_label(canvas, label_rect, label, theme, FontRole::UiMedium);
        Self::draw_dim_label(canvas, sub_rect, sublabel, theme, FontRole::UiSmall);
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
        let _ = draw_mono_icon(
            canvas,
            &ICON_PREFS_MONO,
            Point::new(18, 14),
            theme.icon_foreground,
        );
        Self::draw_label(
            canvas,
            Rect::new(46, 12, WIN_W - 58, 20),
            "System Preferences",
            theme,
            FontRole::UiTitle,
        );

        let cards = self.card_rects();
        Self::draw_card(
            canvas,
            theme,
            cards[0],
            theme.icon_foreground,
            "Mouse",
            "Pointer & Acceleration",
            self.icon_mouse,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[1],
            theme.icon_muted,
            "Monitor",
            "Resolution & Display",
            self.icon_monitor,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[2],
            theme.accent,
            "Wallpaper",
            "Desktop Background",
            self.icon_wallpaper,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[3],
            theme.icon_foreground,
            "Notifications",
            "History & DND",
            self.icon_notifications,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[4],
            theme.icon_foreground,
            "About Computer",
            "Hardware, memory, GPU",
            self.icon_computer,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[5],
            theme.accent,
            "About SunlightOS",
            "OS, kernel, and build",
            self.icon_about_os.or(self.icon_logo),
        );
    }

    fn update_grid(&mut self, event: Event) -> bool {
        if let Event::Click { x, y } = event {
            let pt = Point::new(x, y);
            let cards = self.card_rects();
            if cards[0].contains(pt) {
                self.page = Page::Mouse;
                self.status_len = 0;
                return true;
            }
            if cards[1].contains(pt) {
                self.page = Page::Monitor;
                return true;
            }
            if cards[2].contains(pt) {
                self.page = Page::Wallpaper;
                self.refresh_wallpaper_preview();
                return true;
            }
            if cards[3].contains(pt) {
                self.page = Page::Notifications;
                self.status_len = 0;
                return true;
            }
            if cards[4].contains(pt) {
                self.page = Page::AboutComputer;
                self.about = AboutPageState::new();
                self.refresh_sysinfo();
                return true;
            }
            if cards[5].contains(pt) {
                self.page = Page::AboutOs;
                self.about = AboutPageState::new();
                self.refresh_sysinfo();
                return true;
            }
        }
        false
    }

    fn handle_about_action(&mut self, action: AboutAction, computer_page: bool) -> bool {
        match action {
            AboutAction::None => false,
            AboutAction::Back => {
                self.page = Page::Grid;
                self.about.clear_status();
                true
            }
            AboutAction::Refresh => {
                self.refresh_sysinfo();
                self.about.set_status("Information refreshed");
                true
            }
            AboutAction::Copy => {
                if computer_page {
                    let mut buf = FixedStr::<1024>::empty();
                    self.sysinfo.copy_computer_summary(&mut buf);
                    match clipboard::set_clipboard_text(buf.as_str().as_bytes()) {
                        Ok(()) => {
                            self.about.set_status("Summary copied");
                            let _ = show_notification(
                                NotificationKind::Info,
                                "Control Panel",
                                "Summary copied to clipboard",
                                2500,
                            );
                        }
                        Err(msg) => self.about.set_status(msg),
                    }
                } else {
                    let mut buf = FixedStr::<1536>::empty();
                    self.sysinfo.copy_system_report(&mut buf);
                    match clipboard::set_clipboard_text(buf.as_str().as_bytes()) {
                        Ok(()) => {
                            self.about.set_status("System report copied");
                            let _ = show_notification(
                                NotificationKind::Info,
                                "Control Panel",
                                "System report copied to clipboard",
                                2500,
                            );
                        }
                        Err(msg) => self.about.set_status(msg),
                    }
                }
                true
            }
            AboutAction::NavigateComputer => {
                self.page = Page::AboutComputer;
                self.about = AboutPageState::new();
                self.refresh_sysinfo();
                true
            }
            AboutAction::NavigateOs => {
                self.page = Page::AboutOs;
                self.about = AboutPageState::new();
                self.refresh_sysinfo();
                true
            }
        }
    }

    fn update_about_computer_page(&mut self, event: Event) -> bool {
        let action = about::update_computer_page(event, WIN_W, WIN_H, &mut self.about);
        if action == AboutAction::None {
            // Still repaint on scroll keypresses.
            if let Event::KeyPress { pressed: true, .. } = event {
                return true;
            }
            return false;
        }
        self.handle_about_action(action, true)
    }

    fn update_about_os_page(&mut self, event: Event) -> bool {
        let action = about::update_os_page(event, WIN_W, WIN_H, &mut self.about);
        if action == AboutAction::None {
            if let Event::KeyPress { pressed: true, .. } = event {
                return true;
            }
            return false;
        }
        self.handle_about_action(action, false)
    }

    fn notification_back_rect() -> Rect {
        Rect::new(28, WIN_H as i32 - 44, 80, 28)
    }

    fn notification_dnd_rect() -> Rect {
        Rect::new(WIN_W as i32 - 148, WIN_H as i32 - 44, 120, 28)
    }

    fn draw_notifications_page(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        let content = Rect::new(12, 12, WIN_W - 24, WIN_H - 24);
        Panel::new(content).draw(canvas, theme);
        let title_bar = Rect::new(content.x, content.y, content.w, 28);
        canvas.fill_rect(title_bar, theme.panel_alt);
        canvas.draw_rect(
            Rect::new(title_bar.x, title_bar.bottom(), title_bar.w, 1),
            theme.border,
        );
        sun_font::draw_text_vcenter(
            canvas,
            "Notifications",
            title_bar.x + 12,
            title_bar.y,
            title_bar.h,
            &TextStyle::new(FontRole::UiTitle, theme.accent),
        );
        let dnd = notification_dnd_enabled();
        if let Some(icon) = self.icon_notifications {
            canvas.draw_tga_icon_tinted(&icon, Rect::new(28, 56, 20, 20), theme.accent);
        }
        Self::draw_label(
            canvas,
            Rect::new(54, 58, WIN_W - 82, 18),
            "Open Notification Center from the top-right bell in Vortex Shell.",
            theme,
            FontRole::UiSmall,
        );
        let dnd_icon = if dnd { self.icon_dnd_on } else { self.icon_dnd };
        if let Some(icon) = dnd_icon {
            canvas.draw_tga_icon_tinted(
                &icon,
                Rect::new(28, 86, 20, 20),
                if dnd { theme.warn } else { theme.icon_muted },
            );
        }
        Self::draw_label(
            canvas,
            Rect::new(54, 88, WIN_W - 82, 18),
            if dnd { "DND is On." } else { "DND is Off." },
            theme,
            FontRole::UiMedium,
        );
        Self::draw_dim_label(
            canvas,
            Rect::new(28, 116, WIN_W - 56, 18),
            "When DND is on, notifications are saved to history but popups are hidden.",
            theme,
            FontRole::UiSmall,
        );

        let mut back = Button::secondary(Self::notification_back_rect(), "Back");
        back.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, back);

        let mut dnd_btn = Button::new(
            Self::notification_dnd_rect(),
            if dnd { "Turn DND Off" } else { "Turn DND On" },
        );
        dnd_btn.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, dnd_btn);
    }

    fn update_notifications_page(&mut self, event: Event) -> bool {
        if let Event::Click { x, y } = event {
            let pt = Point::new(x, y);
            if Self::notification_back_rect().contains(pt) {
                self.page = Page::Grid;
                return true;
            }
            if Self::notification_dnd_rect().contains(pt) {
                let _ = notification_set_dnd(!notification_dnd_enabled());
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

        Label::new(desc_row, "Display resolution (read-only).").draw(canvas, theme);

        // Format current resolution
        let mut cur_buf = [0u8; 48];
        let cur_str = fmt_cur_res(self.screen_w, self.screen_h, &mut cur_buf);
        Label::new(cur_row, cur_str).draw(canvas, theme);

        Label::new(opt1_row, "Runtime mode switching is not available yet.").draw(canvas, theme);
        Label::new(
            opt2_row,
            "Use host VM settings or ./tools/runs.sh --resolution.",
        )
        .draw(canvas, theme);
        Label::new(opt3_row, " ").draw(canvas, theme);

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

    fn wallpaper_preview_rect() -> Rect {
        Rect::new(
            ((WIN_W - WALLPAPER_PREVIEW_W) / 2) as i32,
            46,
            WALLPAPER_PREVIEW_W,
            WALLPAPER_PREVIEW_H,
        )
    }

    /// 3-column wrapping grid tile for wallpaper `idx` (rows of 3,3,1 for 7 items).
    fn wallpaper_item_rect(idx: usize) -> Rect {
        const COLS: i32 = 3;
        const TILE_W: i32 = 102;
        const TILE_H: i32 = 34;
        const HGAP: i32 = 10;
        const VGAP: i32 = 5;
        const GRID_Y: i32 = 166;
        let total_w = COLS * TILE_W + (COLS - 1) * HGAP;
        let start_x = (WIN_W as i32 - total_w) / 2;
        let col = (idx as i32) % COLS;
        let row = (idx as i32) / COLS;
        Rect::new(
            start_x + col * (TILE_W + HGAP),
            GRID_Y + row * (TILE_H + VGAP),
            TILE_W as u32,
            TILE_H as u32,
        )
    }

    fn wallpaper_back_rect() -> Rect {
        Rect::new(28, WIN_H as i32 - 44, 80, 28)
    }

    fn wallpaper_refresh_rect() -> Rect {
        Rect::new((WIN_W as i32 - 80) / 2, WIN_H as i32 - 44, 80, 28)
    }

    fn wallpaper_apply_rect() -> Rect {
        Rect::new(WIN_W as i32 - 108, WIN_H as i32 - 44, 80, 28)
    }

    fn draw_wallpaper_page(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        let content = Rect::new(12, 12, WIN_W - 24, WIN_H - 24);
        Panel::with_title(content, "Wallpaper").draw(canvas, theme);

        let preview = Self::wallpaper_preview_rect();
        canvas.fill_rect(preview, theme.panel_alt);
        canvas.draw_rect(preview, theme.border);
        if let Some(img) = self.wallpaper_preview {
            let dst = fit_image_rect(img.width, img.height, preview.inset(4));
            canvas.draw_tga_icon(&img, dst);
        } else {
            Label::new(
                Rect::new(preview.x + 8, preview.y + 8, preview.w - 16, preview.h - 16),
                "Preview unavailable",
            )
            .draw(canvas, theme);
        }

        for idx in 0..self.wallpaper_items.len() {
            let item = &self.wallpaper_items[idx];
            let r = Self::wallpaper_item_rect(idx);
            let fill = if idx == self.wallpaper_selected {
                theme.accent.darken(140)
            } else {
                theme.panel
            };
            let border = if idx == self.wallpaper_selected {
                theme.accent
            } else {
                theme.border
            };
            canvas.fill_rounded_rect(r, 6, fill);
            canvas.stroke_rounded_rect(r, 6, 1, border);
            Label::new(Rect::new(r.x + 8, r.y + 9, r.w - 16, 16), &item.label).draw(canvas, theme);
        }

        if self.wallpaper_status_len > 0 {
            Label::new(
                Rect::new(28, 270, WIN_W - 56, 14),
                self.wallpaper_status_str(),
            )
            .draw(canvas, theme);
        }

        let mut back = Button::secondary(Self::wallpaper_back_rect(), "Back");
        back.state = ButtonState::Normal;
        back.draw(canvas, theme);

        let mut refresh = Button::secondary(Self::wallpaper_refresh_rect(), "Refresh");
        refresh.state = ButtonState::Normal;
        refresh.draw(canvas, theme);

        let mut apply = Button::new(Self::wallpaper_apply_rect(), "Apply");
        apply.state = ButtonState::Normal;
        apply.draw(canvas, theme);
    }

    fn apply_wallpaper(&mut self) {
        if self.wallpaper_items.is_empty() {
            self.set_wallpaper_status("No wallpapers found");
            return;
        }
        self.wallpaper_config.wallpaper = self.wallpaper_items[self.wallpaper_selected]
            .apply_path
            .clone();
        match save_desktop_config(&self.wallpaper_config) {
            Ok(()) => {
                for idx in 0..self.wallpaper_items.len() {
                    self.wallpaper_items[idx].selected = idx == self.wallpaper_selected;
                }
                self.set_wallpaper_status("Wallpaper applied");
                let _ = show_notification(
                    NotificationKind::Info,
                    "Control Panel",
                    "Wallpaper applied",
                    3000,
                );
            }
            Err(_) => self.set_wallpaper_status("Config write failed"),
        }
    }

    /// Rescan the wallpaper directory and rebuild the grid, preserving the
    /// current selection when possible (matched by `apply_path`).
    fn refresh_wallpaper_list(&mut self) {
        let prev_apply = self
            .wallpaper_items
            .get(self.wallpaper_selected)
            .map(|item| item.apply_path.clone());
        self.wallpaper_items = scan_wallpapers(&self.wallpaper_config.wallpaper);
        self.wallpaper_selected = prev_apply
            .and_then(|p| {
                self.wallpaper_items
                    .iter()
                    .position(|item| item.apply_path == p)
            })
            .unwrap_or_else(|| {
                self.wallpaper_items
                    .iter()
                    .position(|item| item.selected)
                    .unwrap_or(0)
            });
        self.refresh_wallpaper_preview();
        if self.wallpaper_items.is_empty() {
            self.set_wallpaper_status("No wallpapers found");
        } else {
            self.set_wallpaper_status("Wallpaper list refreshed");
        }
    }

    fn wallpaper_item_at(pt: Point, len: usize) -> Option<usize> {
        (0..len).find(|&idx| Self::wallpaper_item_rect(idx).contains(pt))
    }

    fn select_wallpaper(&mut self, idx: usize) {
        if idx >= self.wallpaper_items.len() {
            return;
        }
        self.wallpaper_selected = idx;
        self.refresh_wallpaper_preview();
    }

    fn update_wallpaper_page(&mut self, event: Event) -> bool {
        match event {
            Event::MouseDown { x, y, button: 0 } => {
                let pt = Point::new(x, y);
                self.wallpaper_pending_item =
                    Self::wallpaper_item_at(pt, self.wallpaper_items.len());
                if let Some(idx) = self.wallpaper_pending_item {
                    self.select_wallpaper(idx);
                    return true;
                }
            }
            Event::MouseUp { x, y, button: 0 } | Event::Click { x, y } => {
                let pt = Point::new(x, y);
                if Self::wallpaper_back_rect().contains(pt) {
                    self.wallpaper_pending_item = None;
                    self.page = Page::Grid;
                    return true;
                }
                if Self::wallpaper_apply_rect().contains(pt) {
                    self.wallpaper_pending_item = None;
                    self.apply_wallpaper();
                    return true;
                }
                if Self::wallpaper_refresh_rect().contains(pt) {
                    self.wallpaper_pending_item = None;
                    self.refresh_wallpaper_list();
                    return true;
                }
                if let Some(idx) = Self::wallpaper_item_at(pt, self.wallpaper_items.len()) {
                    if self
                        .wallpaper_pending_item
                        .map(|pending| pending == idx)
                        .unwrap_or(true)
                    {
                        self.select_wallpaper(idx);
                    }
                    self.wallpaper_pending_item = None;
                    return true;
                }
                self.wallpaper_pending_item = None;
            }
            _ => {}
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
            Page::Wallpaper => self.draw_wallpaper_page(canvas, theme),
            Page::Notifications => self.draw_notifications_page(canvas, theme),
            Page::AboutComputer => about::draw_computer_page(
                canvas,
                theme,
                WIN_W,
                WIN_H,
                &self.sysinfo,
                &self.about,
                self.icon_computer,
            ),
            Page::AboutOs => about::draw_os_page(
                canvas,
                theme,
                WIN_W,
                WIN_H,
                &self.sysinfo,
                &self.about,
                self.icon_logo.or(self.icon_about_os),
            ),
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
            Page::Wallpaper => self.update_wallpaper_page(event),
            Page::Notifications => self.update_notifications_page(event),
            Page::AboutComputer => self.update_about_computer_page(event),
            Page::AboutOs => self.update_about_os_page(event),
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

fn read_preview_wallpaper_bytes(path: &[u8]) -> Option<&'static [u8]> {
    use sunlight_libc as libc;

    static mut PREVIEW_BUF: [u8; WALLPAPER_PREVIEW_MAX_BYTES] = [0u8; WALLPAPER_PREVIEW_MAX_BYTES];

    let fd = libc::open(path).ok()?;
    let mut len = 0usize;
    loop {
        let remaining = WALLPAPER_PREVIEW_MAX_BYTES.saturating_sub(len);
        if remaining == 0 {
            break;
        }
        let take = remaining.min(4096);
        let chunk = unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(PREVIEW_BUF).cast::<u8>().add(len),
                take,
            )
        };
        let n = match libc::read(fd, chunk) {
            Ok(n) => n,
            Err(libc::sys::Errno::Again) => continue,
            Err(_) => {
                let _ = libc::close(fd);
                return None;
            }
        };
        if n == 0 {
            break;
        }
        len = len.saturating_add(n);
    }
    let _ = libc::close(fd);
    Some(unsafe { core::slice::from_raw_parts(core::ptr::addr_of!(PREVIEW_BUF).cast::<u8>(), len) })
}

fn fit_image_rect(img_w: u32, img_h: u32, bounds: Rect) -> Rect {
    if img_w == 0 || img_h == 0 || bounds.w == 0 || bounds.h == 0 {
        return bounds;
    }
    let by_width_h = (bounds.w as u64)
        .saturating_mul(img_h as u64)
        .checked_div(img_w as u64)
        .unwrap_or(bounds.h as u64);
    let (draw_w, draw_h) = if by_width_h <= bounds.h as u64 {
        (bounds.w, by_width_h as u32)
    } else {
        let w = (bounds.h as u64)
            .saturating_mul(img_w as u64)
            .checked_div(img_h as u64)
            .unwrap_or(bounds.w as u64);
        (cmp::min(w as u32, bounds.w), bounds.h)
    };
    Rect::new(
        bounds.x + (bounds.w.saturating_sub(draw_w) / 2) as i32,
        bounds.y + (bounds.h.saturating_sub(draw_h) / 2) as i32,
        draw_w,
        draw_h,
    )
}

// ---------------------------------------------------------------------------
// Allocator + panic
// ---------------------------------------------------------------------------

struct BumpAllocator;
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 8 * 1024 * 1024] = [0; 8 * 1024 * 1024];
        static mut NEXT: usize = 0;
        let aligned = (NEXT + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _: *mut u8, _: core::alloc::Layout) {}
}
#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

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
    let (screen_w, screen_h) = display_ep
        .and_then(sunlight_ipc::query_display_metrics)
        .map(|m| (m.width_px, m.height_px))
        .unwrap_or((sunlight_ipc::SAFE_FALLBACK_W, sunlight_ipc::SAFE_FALLBACK_H));

    let initial_page = parse_initial_page(argc, argv);
    let app = ControlPanelApp::new(display_ep, screen_w, screen_h, initial_page);
    let mut app = app;
    if app.page == Page::Wallpaper {
        app.refresh_wallpaper_preview();
    }
    if matches!(app.page, Page::AboutComputer | Page::AboutOs) {
        app.refresh_sysinfo();
    }

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

fn parse_initial_page(argc: u64, argv: *const *const u8) -> Page {
    let mut raw = [core::ptr::null::<u8>(); 8];
    let count = unsafe { crt0::collect_raw_args(argc, argv, &mut raw) };
    let mut i = 1usize;
    while i < count {
        let len = unsafe { crt0::cstr_len(raw[i], 48) };
        if len == 0 {
            i += 1;
            continue;
        }
        let bytes = unsafe { core::slice::from_raw_parts(raw[i], len) };
        if bytes == b"--page" && i + 1 < count {
            let next_len = unsafe { crt0::cstr_len(raw[i + 1], 48) };
            let next = unsafe { core::slice::from_raw_parts(raw[i + 1], next_len) };
            match next {
                b"wallpaper" => return Page::Wallpaper,
                b"about-computer" | b"computer" => return Page::AboutComputer,
                b"about-os" | b"about-sunlightos" | b"about" => return Page::AboutOs,
                _ => {}
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    Page::Grid
}
