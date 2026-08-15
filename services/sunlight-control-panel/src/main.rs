//! SunlightOS Control Panel — System Preferences.
//!
//! Grid of settings cards:
//!   • Mouse — pointer sensitivity slider (1-10) + acceleration toggle.
//!   • Monitor — current screen resolution (read-only).
//!   • Wallpaper — desktop background picker.
//!   • Notifications — DND toggle.
//!   • About This Computer — hardware, memory, graphics, runtime snapshot.
//!   • About SunlightOS — OS, kernel, and core component identity.
//!   • Network — interface snapshot.
//!   • Power & Thermal — powerd modes + thermald sensors/cooling.
//!   • Date & Time — solar clock, NTP sync, timezone catalog/map.
//!   • Sound — output device, master volume, test tone.
//!
//! Direct page launch: `control-panel --page <name>` where name is one of
//! wallpaper, about-computer, about-os, network, power-thermal, date-time, sound.
//!
//! Icons: SunlightOS icon theme (Breeze-inspired, TGA format).

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]

extern crate alloc;

mod about;
mod clipboard;
mod date_time;
mod network;
mod power_thermal;
mod session;
mod sound;
mod sysinfo;

use alloc::vec::Vec;
use core::cmp;
use sun_font::{self, FontRole, TextStyle, Typography};
use sunlight_ipc::{
    attach_display_mode_dialog, begin_display_mode_change, confirm_display_mode_change, debug_log,
    ipc_call, ipc_call_timeout,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_lookup, nameserver_lookup_timeout, notification_dnd_enabled,
    notification_set_dnd, process_yield, query_display_mode, query_display_mode_capabilities,
    revert_display_mode_change, show_notification, CapabilityToken, DisplayMode,
    DisplayModeCapabilities, DisplayModeManagement, DisplayModeTransaction, IpcMsg,
    NotificationKind, ProcessExit, ScreenBackend, SgpMsg, WiseOwlLifecycleMsg,
    DEFAULT_MODE_PREVIEW_TIMEOUT_MS, WISEOWL_CONTROL_PANEL_LIFECYCLE_ENDPOINT,
};
use sunlight_libc::{crt0, sun_exec::ControlPanelPage};
use sunlight_ui::{
    image::{decode_simg, draw_mono_icon, MonoIcon, RgbaImage, TgaImage},
    request_close,
    widgets::{Button, ButtonState, Checkbox, Label, Panel, Slider},
    App, AxisSizing, Canvas, Color, Column, Event, HBox, LayoutBox, LayoutInvalidation,
    MaterialPalette, Point, Rect, Size, Sizing, Theme, VBox, Window, WindowConfig,
    WindowDecoration, WindowEvent, WindowMaterial,
};
use sunlight_wallpaper::{
    is_supported_wallpaper, load_desktop_config, save_desktop_config, scan_wallpapers,
    DesktopConfig, WallpaperEntry,
};

use about::{AboutAction, AboutPageState};
use date_time::{DateTimeAction, DateTimePageState};
use network::{NetworkAction, NetworkPageState};
use power_thermal::{PowerThermalAction, PowerThermalPageState};
use session::{SessionAction, SessionPageState};
use sound::{SoundAction, SoundPageState};
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
static ICON_NETWORK_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/network-card.tga");
static ICON_POWER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/battery.tga");
static ICON_DATETIME_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/preferences-system-time.tga");
static ICON_SESSION_TGA: &[u8] = include_bytes!(
    "../../../docs/icons/SunlightOS/apps/48/preferences-system-session-services.tga"
);
static ICON_SOUND_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/audio-card.tga");

const ICON_PREFS_MONO: MonoIcon<'static> = MonoIcon::new(16, 16, ICON_PREFS_MONO_RAW);

// Tall enough to show the complete About SunlightOS details without scrolling.
const WIN_W: u32 = 500;
const WIN_H: u32 = 560;
const PAGE_HEADER_H: u32 = 44;
const DISPLAY_DIALOG_W: u32 = 420;
const DISPLAY_DIALOG_H: u32 = 190;
const KEY_ESC: u8 = 0x01;
const KEY_ENTER: u8 = 0x1C;
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
    Network,
    PowerThermal,
    DateTime,
    LoginSession,
    Sound,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayConfirmationResult {
    Kept,
    Reverted,
    TimedOut,
    Closed,
    Failed,
}

struct DisplayConfirmationApp {
    display_ep: CapabilityToken,
    transaction: DisplayModeTransaction,
    previous_mode: DisplayMode,
    result: Option<DisplayConfirmationResult>,
    keep_focused: bool,
    last_seconds: u32,
}

impl DisplayConfirmationApp {
    fn new(
        display_ep: CapabilityToken,
        transaction: DisplayModeTransaction,
        previous_mode: DisplayMode,
    ) -> Self {
        Self {
            display_ep,
            transaction,
            previous_mode,
            result: None,
            keep_focused: true,
            last_seconds: u32::MAX,
        }
    }

    fn seconds_remaining(&self) -> u32 {
        (self
            .transaction
            .deadline_ms
            .saturating_sub(monotonic_millis())
            .saturating_add(999)
            / 1000) as u32
    }

    fn keep(&mut self) {
        self.result = Some(
            if confirm_display_mode_change(self.display_ep, self.transaction.token) {
                DisplayConfirmationResult::Kept
            } else {
                DisplayConfirmationResult::TimedOut
            },
        );
        request_close();
    }

    fn revert(&mut self) {
        self.result = Some(
            if revert_display_mode_change(self.display_ep, self.transaction.token) {
                DisplayConfirmationResult::Reverted
            } else {
                DisplayConfirmationResult::TimedOut
            },
        );
        request_close();
    }

    fn button_rects() -> (Rect, Rect) {
        (
            Rect::new(22, DISPLAY_DIALOG_H as i32 - 54, 110, 30),
            Rect::new(
                DISPLAY_DIALOG_W as i32 - 162,
                DISPLAY_DIALOG_H as i32 - 54,
                140,
                30,
            ),
        )
    }
}

impl App for DisplayConfirmationApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(
            Rect::new(0, 0, DISPLAY_DIALOG_W, DISPLAY_DIALOG_H),
            theme.bg,
        );
        let card = Rect::new(10, 10, DISPLAY_DIALOG_W - 20, DISPLAY_DIALOG_H - 20);
        canvas.fill_rounded_rect(card, 10, theme.panel);
        canvas.stroke_rounded_rect(card, 10, 2, theme.accent);
        Label::new(
            Rect::new(26, 28, DISPLAY_DIALOG_W - 52, 24),
            "Keep this display configuration?",
        )
        .with_font(&Typography::UI_MEDIUM)
        .draw(canvas, theme);
        let mut countdown_buf = [0u8; 64];
        Label::new(
            Rect::new(26, 70, DISPLAY_DIALOG_W - 52, 40),
            fmt_countdown(
                self.previous_mode,
                self.seconds_remaining(),
                &mut countdown_buf,
            ),
        )
        .with_font(&Typography::UI_SMALL)
        .draw(canvas, theme);
        let (revert_rect, keep_rect) = Self::button_rects();
        let mut revert = Button::secondary(revert_rect, "Revert").with_font(&Typography::UI_MEDIUM);
        let mut keep = Button::new(keep_rect, "Keep Changes").with_font(&Typography::UI_MEDIUM);
        if self.keep_focused {
            keep.state = ButtonState::Pressed;
        } else {
            revert.state = ButtonState::Pressed;
        }
        revert.draw(canvas, theme);
        keep.draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                let point = Point::new(x, y);
                let (revert_rect, keep_rect) = Self::button_rects();
                if revert_rect.contains(point) {
                    self.revert();
                    return true;
                }
                if keep_rect.contains(point) {
                    self.keep();
                    return true;
                }
                false
            }
            Event::KeyPress {
                keycode: KEY_ESC,
                pressed: true,
                ..
            } => {
                self.revert();
                true
            }
            Event::KeyPress {
                keycode: KEY_ENTER,
                pressed: true,
                ..
            }
            | Event::Key('\n')
                if self.keep_focused =>
            {
                self.keep();
                true
            }
            Event::KeyPress {
                keycode: 0x0F,
                pressed: true,
                ..
            } => {
                self.keep_focused = !self.keep_focused;
                true
            }
            Event::Tick => {
                let seconds = self.seconds_remaining();
                let changed = seconds != self.last_seconds;
                self.last_seconds = seconds;
                changed
            }
            _ => false,
        }
    }

    fn poll_timeout_ms(&self) -> u64 {
        200
    }
}

fn run_display_confirmation_dialog(
    display_ep: CapabilityToken,
    transaction: DisplayModeTransaction,
    previous_mode: DisplayMode,
) -> DisplayConfirmationResult {
    let flags = 1 | (1 << 5) | (95 << 6);
    let Some(mut window) = Window::connect_with_flags(
        WindowConfig {
            width: DISPLAY_DIALOG_W,
            height: DISPLAY_DIALOG_H,
            title: "Display Configuration",
            decoration: WindowDecoration::CompactClose,
        },
        flags,
    ) else {
        let _ = revert_display_mode_change(display_ep, transaction.token);
        debug_log("[DISPLAY-MODE] failed stage=dialog-create error=window-unavailable\n");
        return DisplayConfirmationResult::Failed;
    };
    if !attach_display_mode_dialog(display_ep, transaction.token, window.id()) {
        let _ = revert_display_mode_change(display_ep, transaction.token);
        debug_log("[DISPLAY-MODE] failed stage=dialog-attach error=rejected\n");
        return DisplayConfirmationResult::Failed;
    }
    let mut app = DisplayConfirmationApp::new(display_ep, transaction, previous_mode);
    window.run(&mut app);
    app.result.unwrap_or(DisplayConfirmationResult::Closed)
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
    icon_network: Option<TgaImage>,
    icon_power: Option<TgaImage>,
    icon_datetime: Option<TgaImage>,
    icon_session: Option<TgaImage>,
    icon_sound: Option<TgaImage>,
    wallpaper_items: Vec<WallpaperEntry>,
    wallpaper_config: DesktopConfig,
    wallpaper_selected: usize,
    wallpaper_preview: Option<RgbaImage>,
    wallpaper_pending_item: Option<usize>,
    sysinfo: SystemInfoSnapshot,
    about: AboutPageState,
    display_capabilities: Option<DisplayModeCapabilities>,
    display_modes: Vec<DisplayMode>,
    selected_mode: usize,
    display_transaction: Option<DisplayModeTransaction>,
    display_previous_mode: Option<DisplayMode>,
    display_error: FixedStr<96>,
    network: NetworkPageState,
    power_thermal: PowerThermalPageState,
    date_time: DateTimePageState,
    login_session: SessionPageState,
    sound: SoundPageState,
    client_bounds: Rect,
    layout_invalidation: LayoutInvalidation,
    layout: ControlPanelLayout,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ControlPanelLayout {
    root: Rect,
    header: Rect,
    content: Rect,
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
        let mut accel_cb = Checkbox::new(Rect::default(), "Enable pointer acceleration")
            .with_font(&Typography::UI_REGULAR);
        accel_cb.checked = true;
        let wallpaper_config = load_desktop_config();
        let wallpaper_items = scan_wallpapers(&wallpaper_config.wallpaper);
        let wallpaper_selected = wallpaper_items
            .iter()
            .position(|it| it.selected)
            .unwrap_or(0);
        let mut app = Self {
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
            icon_network: TgaImage::parse(ICON_NETWORK_TGA).ok(),
            icon_power: TgaImage::parse(ICON_POWER_TGA).ok(),
            icon_datetime: TgaImage::parse(ICON_DATETIME_TGA).ok(),
            icon_session: TgaImage::parse(ICON_SESSION_TGA).ok(),
            icon_sound: TgaImage::parse(ICON_SOUND_TGA).ok(),
            wallpaper_items,
            wallpaper_config,
            wallpaper_selected,
            wallpaper_preview: None,
            wallpaper_pending_item: None,
            sysinfo: SystemInfoSnapshot::collect(display_ep, screen_w, screen_h),
            about: AboutPageState::new(),
            display_capabilities: None,
            display_modes: Vec::new(),
            selected_mode: 0,
            display_transaction: None,
            display_previous_mode: None,
            display_error: FixedStr::empty(),
            network: NetworkPageState::new(),
            power_thermal: PowerThermalPageState::new(),
            date_time: DateTimePageState::new(),
            login_session: SessionPageState::new(),
            sound: SoundPageState::new(),
            client_bounds: Rect::new(0, 0, WIN_W, WIN_H),
            layout_invalidation: LayoutInvalidation::new(),
            layout: ControlPanelLayout::default(),
        };
        app.ensure_layout();
        app
    }

    fn compute_layout(root: Rect) -> ControlPanelLayout {
        let mut children = [
            LayoutBox::new(Rect::new(0, 0, 0, PAGE_HEADER_H)).with_sizing(Sizing::new(
                AxisSizing::Fill,
                AxisSizing::Fixed(PAGE_HEADER_H),
            )),
            LayoutBox::new(Rect::new(0, 0, 0, 0))
                .with_sizing(Sizing::new(AxisSizing::Fill, AxisSizing::Fill)),
        ];
        let _ = Column::new(root).arrange(&mut children);
        ControlPanelLayout {
            root,
            header: children[0].bounds(),
            content: children[1].bounds(),
        }
    }

    fn ensure_layout(&mut self) -> bool {
        if !self.layout_invalidation.update(self.client_bounds) {
            return false;
        }
        self.layout = Self::compute_layout(self.client_bounds);
        true
    }

    fn set_client_bounds(&mut self, width: u32, height: u32) -> bool {
        let bounds = Rect::new(0, 0, width, height);
        if bounds == self.client_bounds {
            return false;
        }
        self.client_bounds = bounds;
        self.layout_invalidation.invalidate();
        self.ensure_layout()
    }

    fn win_w(&self) -> u32 {
        self.client_bounds.w
    }

    fn win_h(&self) -> u32 {
        self.client_bounds.h
    }

    fn page_content(&self) -> Rect {
        Rect::new(
            12,
            12,
            self.win_w().saturating_sub(24),
            self.win_h().saturating_sub(24),
        )
    }

    fn refresh_display_modes(&mut self) {
        self.display_modes.clear();
        self.display_transaction = None;
        self.display_previous_mode = None;
        self.display_error.clear();
        let Some(display_ep) = self.display_ep else {
            self.display_capabilities = None;
            self.display_error.set("Display service unavailable");
            return;
        };
        let Some(capabilities) = query_display_mode_capabilities(display_ep) else {
            self.display_capabilities = None;
            self.display_error
                .set("Could not query display capabilities");
            return;
        };
        for index in 0..capabilities.mode_count {
            if let Some(mode) = query_display_mode(display_ep, index) {
                self.display_modes.push(mode);
            }
        }
        self.selected_mode = self
            .display_modes
            .iter()
            .position(|mode| mode.current)
            .unwrap_or(0);
        self.screen_w = capabilities.current_mode.width;
        self.screen_h = capabilities.current_mode.height;
        self.display_capabilities = Some(capabilities);
    }

    fn apply_selected_display_mode(&mut self) {
        if self.display_transaction.is_some() {
            return;
        }
        let Some(display_ep) = self.display_ep else {
            self.display_error.set("Display service unavailable");
            return;
        };
        let Some(mode) = self.display_modes.get(self.selected_mode).copied() else {
            return;
        };
        if mode.current {
            self.display_error.clear();
            return;
        }
        let previous_mode = self
            .display_capabilities
            .map(|capabilities| capabilities.current_mode)
            .unwrap_or(mode);
        debug_log("[DISPLAY-MODE] ui selection old=");
        log_mode(previous_mode.width, previous_mode.height);
        debug_log(" requested=");
        log_mode(mode.width, mode.height);
        debug_log("\n");
        match begin_display_mode_change(
            display_ep,
            mode.width,
            mode.height,
            DEFAULT_MODE_PREVIEW_TIMEOUT_MS,
        ) {
            Some(transaction) => {
                self.display_previous_mode = Some(previous_mode);
                self.screen_w = transaction.applied_mode.width;
                self.screen_h = transaction.applied_mode.height;
                self.display_transaction = Some(transaction);
                self.display_error.clear();
                let result =
                    run_display_confirmation_dialog(display_ep, transaction, previous_mode);
                self.refresh_display_modes();
                match result {
                    DisplayConfirmationResult::Kept => {
                        self.display_error.clear();
                    }
                    DisplayConfirmationResult::Reverted
                    | DisplayConfirmationResult::TimedOut
                    | DisplayConfirmationResult::Closed => {
                        self.display_error
                            .set("Display mode restored to previous setting");
                    }
                    DisplayConfirmationResult::Failed => {
                        self.display_error
                            .set("Could not show display confirmation");
                    }
                }
            }
            None => {
                self.refresh_display_modes();
                self.display_error
                    .set("Display preview failed; previous mode restored");
            }
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
        if !is_supported_wallpaper(&bytes) {
            self.set_wallpaper_status("Unsupported or corrupt wallpaper");
            return;
        }
        match decode_simg(&bytes) {
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

    fn card_rects(&self) -> [Rect; 11] {
        let card_w = 136u32;
        let card_h = 96u32;
        let gap = 14i32;
        let start_x = (self.win_w() as i32 - (card_w * 3) as i32 - gap * 2) / 2;
        let card_y = self.layout.content.y + 8;
        let row2_y = card_y + card_h as i32 + 10;
        let row3_y = row2_y + card_h as i32 + 10;
        let row4_y = row3_y + card_h as i32 + 10;
        [
            Rect::new(start_x, card_y, card_w, card_h),
            Rect::new(start_x + card_w as i32 + gap, card_y, card_w, card_h),
            Rect::new(start_x + (card_w as i32 + gap) * 2, card_y, card_w, card_h),
            Rect::new(start_x, row2_y, card_w, card_h),
            Rect::new(start_x + card_w as i32 + gap, row2_y, card_w, card_h),
            Rect::new(start_x + (card_w as i32 + gap) * 2, row2_y, card_w, card_h),
            Rect::new(start_x, row3_y, card_w, card_h),
            Rect::new(start_x + card_w as i32 + gap, row3_y, card_w, card_h),
            Rect::new(start_x + (card_w as i32 + gap) * 2, row3_y, card_w, card_h),
            Rect::new(start_x, row4_y, card_w, card_h),
            Rect::new(start_x + card_w as i32 + gap, row4_y, card_w, card_h),
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
        canvas.fill_material(
            rect,
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(10)
                .without_border(),
        );
        // Quiet hairline — modern grid without heavy framed tiles.
        canvas.stroke_rounded_rect(rect, 10, 1, theme.panel.lighten(22));

        let ix = rect.x + rect.w as i32 / 2 - 24;
        let iy = rect.y + 22;
        let icon_rect = Rect::new(ix, iy, 48, 48);

        if let Some(tga) = tga_icon {
            // Rounded application icon (AA corner clip + bilinear when scaled).
            canvas.draw_tga_icon_rounded(&tga, icon_rect, 12);
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
        canvas.clear_transparent(self.layout.root);

        // Header bar — denser surface in the same charcoal family as WindowGlass.
        let header = self.layout.header;
        canvas.fill_material(
            header,
            MaterialPalette::new(theme)
                .card_glass
                .with_radius(0)
                .without_border(),
        );
        canvas.draw_rect(
            Rect::new(header.x, header.bottom() - 1, header.w, 1),
            theme.chrome.subtle_border,
        );

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
            Rect::new(46, header.y + 12, header.w.saturating_sub(58), 20),
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
        Self::draw_card(
            canvas,
            theme,
            cards[6],
            theme.accent,
            "Network",
            "Ethernet & Loopback",
            self.icon_network,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[7],
            theme.accent,
            "Power & Thermal",
            "Modes, fans, sensors",
            self.icon_power,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[8],
            theme.accent,
            "Date & Time",
            "Clock, sync, timezone",
            self.icon_datetime,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[9],
            theme.accent,
            "Login & Session",
            "Startup Apps",
            self.icon_session,
        );
        Self::draw_card(
            canvas,
            theme,
            cards[10],
            theme.accent,
            "Sound",
            "Output & Volume",
            self.icon_sound,
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
                self.refresh_display_modes();
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
            if cards[6].contains(pt) {
                self.page = Page::Network;
                return self.network.refresh();
            }
            if cards[7].contains(pt) {
                self.page = Page::PowerThermal;
                return self.power_thermal.refresh();
            }
            if cards[8].contains(pt) {
                self.page = Page::DateTime;
                return self.date_time.activate();
            }
            if cards[9].contains(pt) {
                self.page = Page::LoginSession;
                return self.login_session.activate();
            }
            if cards[10].contains(pt) {
                self.page = Page::Sound;
                return self.sound.refresh();
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
        let action = about::update_computer_page(event, self.win_w(), self.win_h(), &mut self.about);
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
        let action = about::update_os_page(event, self.win_w(), self.win_h(), &mut self.about);
        if action == AboutAction::None {
            if let Event::KeyPress { pressed: true, .. } = event {
                return true;
            }
            return false;
        }
        self.handle_about_action(action, false)
    }

    fn update_network_page(&mut self, event: Event) -> bool {
        if matches!(event, Event::Tick) {
            return self.network.refresh_due() && self.network.refresh();
        }
        match self.network.update(event, self.win_w(), self.win_h()) {
            NetworkAction::None => true,
            NetworkAction::Back => {
                self.page = Page::Grid;
                // Release the bounded snapshot immediately instead of retaining
                // page-local state while the user works elsewhere.
                self.network = NetworkPageState::new();
                true
            }
        }
    }

    fn update_sound_page(&mut self, event: Event) -> bool {
        if matches!(event, Event::Tick) {
            return self.sound.refresh_due() && self.sound.refresh();
        }
        match self.sound.update(event, self.win_w(), self.win_h()) {
            SoundAction::None => true,
            SoundAction::Back => {
                self.page = Page::Grid;
                self.sound = SoundPageState::new();
                true
            }
        }
    }

    fn update_power_thermal_page(&mut self, event: Event) -> bool {
        let (redraw, action) = self.power_thermal.update(event, self.win_w(), self.win_h());
        match action {
            PowerThermalAction::None => redraw,
            PowerThermalAction::Back => {
                self.page = Page::Grid;
                // Drop page-local IPC snapshots when leaving the page.
                self.power_thermal = PowerThermalPageState::new();
                true
            }
        }
    }

    fn update_date_time_page(&mut self, event: Event) -> bool {
        let (redraw, action) = self.date_time.update(event, self.win_w(), self.win_h());
        match action {
            DateTimeAction::None => redraw,
            DateTimeAction::Back => {
                self.page = Page::Grid;
                // Cancel clock/sync deadlines and drop page-local IPC state.
                self.date_time.deactivate();
                true
            }
        }
    }

    fn notification_back_rect(&self) -> Rect {
        Rect::new(28, self.win_h() as i32 - 44, 80, 28)
    }

    fn notification_dnd_rect(&self) -> Rect {
        Rect::new(self.win_w() as i32 - 148, self.win_h() as i32 - 44, 120, 28)
    }

    fn draw_notifications_page(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.clear_transparent(self.layout.root);
        let content = self.page_content();
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
            Rect::new(54, 58, self.win_w().saturating_sub(82), 18),
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
            Rect::new(54, 88, self.win_w().saturating_sub(82), 18),
            if dnd { "DND is On." } else { "DND is Off." },
            theme,
            FontRole::UiMedium,
        );
        Self::draw_dim_label(
            canvas,
            Rect::new(28, 116, self.win_w().saturating_sub(56), 18),
            "When DND is on, notifications are saved to history but popups are hidden.",
            theme,
            FontRole::UiSmall,
        );

        let mut back = Button::secondary(self.notification_back_rect(), "Back");
        back.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, back);

        let mut dnd_btn = Button::new(
            self.notification_dnd_rect(),
            if dnd { "Turn DND Off" } else { "Turn DND On" },
        );
        dnd_btn.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, dnd_btn);
    }

    fn update_notifications_page(&mut self, event: Event) -> bool {
        if let Event::Click { x, y } = event {
            let pt = Point::new(x, y);
            if self.notification_back_rect().contains(pt) {
                self.page = Page::Grid;
                return true;
            }
            if self.notification_dnd_rect().contains(pt) {
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
        canvas.clear_transparent(self.layout.root);

        let content = self.page_content();
        Panel::with_title(content, "Mouse")
            .with_font(&Typography::UI_MEDIUM)
            .draw(canvas, theme);

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

        Label::new(desc_row, "Adjust pointer speed and acceleration.")
            .with_font(&Typography::UI_SMALL)
            .draw(canvas, theme);

        // Slider row
        let sc_widths = [110u32, slider_row.w.saturating_sub(180), 60];
        let mut sc = HBox::new(slider_row).with_spacing(8).layout(&sc_widths);
        let slabel_r = sc.next().unwrap_or_default();
        let sslider_r = sc.next().unwrap_or_default();
        let shint_r = sc.next().unwrap_or_default();

        Label::new(slabel_r, "Pointer Speed:")
            .with_font(&Typography::UI_REGULAR)
            .draw(canvas, theme);
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
        Label::new(shint_r, hint)
            .with_font(&Typography::UI_SMALL)
            .draw(canvas, theme);

        self.accel_cb.rect = accel_row;
        self.accel_cb.draw(canvas, theme);

        if self.status_len > 0 {
            Label::new(status_row, self.status_str())
                .with_font(&Typography::UI_SMALL)
                .draw(canvas, theme);
        }

        // Buttons
        let back_r = Rect::new(actions_row.x, actions_row.y, 80, actions_row.h);
        let apply_r = Rect::new(actions_row.right() - 80, actions_row.y, 80, actions_row.h);

        let mut back = Button::secondary(back_r, "Back");
        back.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, back);

        let mut apply = Button::new(apply_r, "Apply");
        apply.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, apply);
    }

    fn mouse_action_rects(&self) -> (Rect, Rect) {
        let content = self.page_content();
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
        canvas.clear_transparent(self.layout.root);

        let content = self.page_content();
        Panel::with_title(content, "Monitor")
            .with_font(&Typography::UI_MEDIUM)
            .draw(canvas, theme);
        let inner = self.monitor_inner_rect();
        let backend = self
            .display_capabilities
            .map(|capabilities| backend_label(capabilities.backend))
            .unwrap_or("Unavailable");
        Label::new(Rect::new(inner.x, inner.y, inner.w, 18), backend)
            .with_font(&Typography::UI_REGULAR)
            .draw(canvas, theme);
        let mut current_buf = [0u8; 48];
        Label::new(
            Rect::new(inner.x, inner.y + 22, inner.w, 18),
            fmt_cur_res(self.screen_w, self.screen_h, &mut current_buf),
        )
        .with_font(&Typography::UI_REGULAR)
        .draw(canvas, theme);

        let management_text = self
            .display_capabilities
            .map(|capabilities| match capabilities.management {
                DisplayModeManagement::Manual => "Management: Manual",
                DisplayModeManagement::Automatic => "Management: Automatic",
                DisplayModeManagement::ReadOnly => "Management: Read-only",
            })
            .unwrap_or("Management: Unavailable");
        Label::new(
            Rect::new(inner.x, inner.y + 44, inner.w, 18),
            management_text,
        )
        .with_font(&Typography::UI_SMALL)
        .draw(canvas, theme);

        if self.display_modes.is_empty() {
            let reason = self
                .display_capabilities
                .map(|capabilities| capabilities.read_only_reason.message())
                .filter(|reason| !reason.is_empty())
                .unwrap_or(self.display_error.as_str());
            Label::new(Rect::new(inner.x, inner.y + 82, inner.w, 36), reason)
                .with_font(&Typography::UI_SMALL)
                .draw(canvas, theme);
        } else {
            Label::new(Rect::new(inner.x, inner.y + 70, inner.w, 18), "Resolution")
                .with_font(&Typography::UI_MEDIUM)
                .draw(canvas, theme);
            for (index, mode) in self.display_modes.iter().enumerate() {
                let rect = self.monitor_mode_rect(index);
                let selected = index == self.selected_mode;
                canvas.fill_rounded_rect(
                    rect,
                    5,
                    if selected {
                        theme.panel_alt
                    } else {
                        theme.panel
                    },
                );
                canvas.stroke_rounded_rect(
                    rect,
                    5,
                    1,
                    if selected { theme.accent } else { theme.border },
                );
                let mut mode_buf = [0u8; 48];
                let mode_text = fmt_mode(*mode, &mut mode_buf);
                Label::new(
                    Rect::new(rect.x + 10, rect.y, rect.w - 20, rect.h),
                    mode_text,
                )
                .with_font(&Typography::UI_REGULAR)
                .draw(canvas, theme);
            }
        }
        if !self.display_error.as_str().is_empty() {
            Label::new(
                Rect::new(inner.x, inner.bottom() - 54, inner.w - 100, 18),
                self.display_error.as_str(),
            )
            .with_font(&Typography::UI_SMALL)
            .draw(canvas, theme);
        }

        let (back_r, apply_r) = self.monitor_action_rects();
        let mut back = Button::secondary(back_r, "Back");
        back.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, back);
        let can_apply = self
            .display_modes
            .get(self.selected_mode)
            .is_some_and(|mode| !mode.current)
            && self.display_transaction.is_none();
        let mut apply = Button::new(apply_r, "Apply");
        apply.state = if can_apply {
            ButtonState::Normal
        } else {
            ButtonState::Disabled
        };
        Self::draw_button(canvas, theme, apply);
    }

    fn monitor_inner_rect(&self) -> Rect {
        let content = self.page_content();
        Rect::new(
            content.x + 14,
            content.y + 32,
            content.w.saturating_sub(28),
            content.h.saturating_sub(46),
        )
    }

    fn monitor_mode_rect(&self, index: usize) -> Rect {
        let inner = self.monitor_inner_rect();
        let column = index / 7;
        let row = index % 7;
        let gap = 8;
        let width = (inner.w - gap) / 2;
        Rect::new(
            inner.x + column as i32 * (width as i32 + gap as i32),
            inner.y + 94 + row as i32 * 27,
            width,
            24,
        )
    }

    fn monitor_action_rects(&self) -> (Rect, Rect) {
        let inner = self.monitor_inner_rect();
        (
            Rect::new(inner.x, inner.bottom() - 30, 80, 28),
            Rect::new(inner.right() - 80, inner.bottom() - 30, 80, 28),
        )
    }

    fn update_monitor_page(&mut self, event: Event) -> bool {
        if let Event::Click { x, y } = event {
            let pt = Point::new(x, y);
            let (back_rect, apply_rect) = self.monitor_action_rects();
            if back_rect.contains(pt) {
                self.page = Page::Grid;
                return true;
            }
            if apply_rect.contains(pt) {
                self.apply_selected_display_mode();
                return true;
            }
            for index in 0..self.display_modes.len() {
                if self.monitor_mode_rect(index).contains(pt) {
                    self.selected_mode = index;
                    self.display_error.clear();
                    return true;
                }
            }
        }
        false
    }

    fn wallpaper_preview_rect(&self) -> Rect {
        Rect::new(
            ((self.win_w().saturating_sub(WALLPAPER_PREVIEW_W)) / 2) as i32,
            46,
            WALLPAPER_PREVIEW_W,
            WALLPAPER_PREVIEW_H,
        )
    }

    /// 3-column wrapping grid tile for wallpaper `idx` (rows of 3,3,1 for 7 items).
    fn wallpaper_item_rect(&self, idx: usize) -> Rect {
        const COLS: i32 = 3;
        const TILE_W: i32 = 102;
        const TILE_H: i32 = 34;
        const HGAP: i32 = 10;
        const VGAP: i32 = 5;
        const GRID_Y: i32 = 166;
        let total_w = COLS * TILE_W + (COLS - 1) * HGAP;
        let start_x = (self.win_w() as i32 - total_w) / 2;
        let col = (idx as i32) % COLS;
        let row = (idx as i32) / COLS;
        Rect::new(
            start_x + col * (TILE_W + HGAP),
            GRID_Y + row * (TILE_H + VGAP),
            TILE_W as u32,
            TILE_H as u32,
        )
    }

    fn wallpaper_back_rect(&self) -> Rect {
        Rect::new(28, self.win_h() as i32 - 44, 80, 28)
    }

    fn wallpaper_refresh_rect(&self) -> Rect {
        Rect::new((self.win_w() as i32 - 80) / 2, self.win_h() as i32 - 44, 80, 28)
    }

    fn wallpaper_apply_rect(&self) -> Rect {
        Rect::new(self.win_w() as i32 - 108, self.win_h() as i32 - 44, 80, 28)
    }

    fn draw_wallpaper_page(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.clear_transparent(self.layout.root);
        let content = self.page_content();
        Panel::with_title(content, "Wallpaper")
            .with_font(&Typography::UI_MEDIUM)
            .draw(canvas, theme);

        let preview = self.wallpaper_preview_rect();
        canvas.fill_rect(preview, theme.panel_alt);
        canvas.draw_rect(preview, theme.border);
        if let Some(ref img) = self.wallpaper_preview {
            let dst = fit_image_rect(img.width, img.height, preview.inset(4));
            // Force-opaque samples (same fix as Light Lens / Files SIMG gray hole).
            draw_wallpaper_preview_opaque(canvas, img, dst);
        } else {
            Label::new(
                Rect::new(preview.x + 8, preview.y + 8, preview.w - 16, preview.h - 16),
                "Preview unavailable",
            )
            .with_font(&Typography::UI_SMALL)
            .draw(canvas, theme);
        }

        for idx in 0..self.wallpaper_items.len() {
            let item = &self.wallpaper_items[idx];
            let r = self.wallpaper_item_rect(idx);
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
            Label::new(Rect::new(r.x + 8, r.y + 9, r.w - 16, 16), &item.label)
                .with_font(&Typography::UI_REGULAR)
                .draw(canvas, theme);
        }

        if self.wallpaper_status_len > 0 {
            Label::new(
                Rect::new(28, 270, self.win_w().saturating_sub(56), 14),
                self.wallpaper_status_str(),
            )
            .with_font(&Typography::UI_SMALL)
            .draw(canvas, theme);
        }

        let mut back = Button::secondary(self.wallpaper_back_rect(), "Back");
        back.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, back);

        let mut refresh = Button::secondary(self.wallpaper_refresh_rect(), "Refresh");
        refresh.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, refresh);

        let mut apply = Button::new(self.wallpaper_apply_rect(), "Apply");
        apply.state = ButtonState::Normal;
        Self::draw_button(canvas, theme, apply);
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

    fn wallpaper_item_at(&self, pt: Point, len: usize) -> Option<usize> {
        (0..len).find(|&idx| self.wallpaper_item_rect(idx).contains(pt))
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
                    self.wallpaper_item_at(pt, self.wallpaper_items.len());
                if let Some(idx) = self.wallpaper_pending_item {
                    self.select_wallpaper(idx);
                    return true;
                }
            }
            Event::MouseUp { x, y, button: 0 } | Event::Click { x, y } => {
                let pt = Point::new(x, y);
                if self.wallpaper_back_rect().contains(pt) {
                    self.wallpaper_pending_item = None;
                    self.page = Page::Grid;
                    return true;
                }
                if self.wallpaper_apply_rect().contains(pt) {
                    self.wallpaper_pending_item = None;
                    self.apply_wallpaper();
                    return true;
                }
                if self.wallpaper_refresh_rect().contains(pt) {
                    self.wallpaper_pending_item = None;
                    self.refresh_wallpaper_list();
                    return true;
                }
                if let Some(idx) = self.wallpaper_item_at(pt, self.wallpaper_items.len()) {
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
        if self.client_bounds.size() != Size::new(canvas.width, canvas.height) {
            let _ = self.set_client_bounds(canvas.width, canvas.height);
        } else {
            let _ = self.ensure_layout();
        }
        let (win_w, win_h) = (self.win_w(), self.win_h());
        match self.page {
            Page::Grid => self.draw_grid(canvas, theme),
            Page::Mouse => self.draw_mouse_page(canvas, theme),
            Page::Monitor => self.draw_monitor_page(canvas, theme),
            Page::Wallpaper => self.draw_wallpaper_page(canvas, theme),
            Page::Notifications => self.draw_notifications_page(canvas, theme),
            Page::AboutComputer => about::draw_computer_page(
                canvas,
                theme,
                win_w,
                win_h,
                &self.sysinfo,
                &self.about,
                self.icon_computer,
            ),
            Page::AboutOs => about::draw_os_page(
                canvas,
                theme,
                win_w,
                win_h,
                &self.sysinfo,
                &self.about,
                self.icon_logo.or(self.icon_about_os),
            ),
            Page::Network => self.network.draw(canvas, theme, win_w, win_h),
            Page::PowerThermal => self.power_thermal.draw(canvas, theme, win_w, win_h),
            Page::DateTime => self.date_time.draw(canvas, theme, win_w, win_h),
            Page::LoginSession => self.login_session.draw(canvas, theme, win_w, win_h),
            Page::Sound => self.sound.draw(canvas, theme, win_w, win_h),
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
            Page::Network => self.update_network_page(event),
            Page::PowerThermal => self.update_power_thermal_page(event),
            Page::DateTime => self.update_date_time_page(event),
            Page::LoginSession => match self.login_session.update(event, self.win_w(), self.win_h()) {
                SessionAction::None => false,
                SessionAction::Back => {
                    self.page = Page::Grid;
                    true
                }
            },
            Page::Sound => self.update_sound_page(event),
        }
    }

    fn on_ready(&mut self) -> bool {
        if self.page == Page::Monitor {
            self.refresh_display_modes();
            return true;
        }
        if self.page == Page::Network {
            return self.network.refresh();
        }
        if self.page == Page::PowerThermal {
            return self.power_thermal.refresh();
        }
        if self.page == Page::DateTime {
            return self.date_time.activate();
        }
        if self.page == Page::Sound {
            return self.sound.refresh();
        }
        false
    }

    fn window_event(&mut self, event: WindowEvent) -> bool {
        let WindowEvent::Resized { width, height } = event;
        self.set_client_bounds(width, height)
    }

    fn poll_timeout_ms(&self) -> u64 {
        if self.display_transaction.is_some() {
            250
        } else if self.page == Page::PowerThermal {
            500
        } else if self.page == Page::DateTime {
            // Arm Event::Tick so second-ray progress can update once per second
            // without a continuous full-repaint loop.
            250
        } else if self.page == Page::Sound {
            400
        } else {
            200
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

fn log_mode(width: u32, height: u32) {
    let mut buffer = [0u8; 48];
    let text = fmt_cur_res(width, height, &mut buffer);
    debug_log(text);
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

fn backend_label(backend: ScreenBackend) -> &'static str {
    match backend {
        ScreenBackend::VmwareSvga => "Display: VMware SVGA II",
        ScreenBackend::VirtioGpu => "Display: VirtIO GPU",
        ScreenBackend::LimineFramebuffer => "Display: Firmware framebuffer",
        ScreenBackend::Fallback => "Display: Fallback framebuffer",
    }
}

fn fmt_mode<'a>(mode: DisplayMode, buf: &'a mut [u8; 48]) -> &'a str {
    let mut pos = 0usize;
    write_u32_into(buf, &mut pos, mode.width);
    if pos + 3 < buf.len() {
        buf[pos] = b' ';
        buf[pos + 1] = b'x';
        buf[pos + 2] = b' ';
        pos += 3;
    }
    write_u32_into(buf, &mut pos, mode.height);
    let suffix = if mode.current {
        b"  Current".as_slice()
    } else if mode.preferred {
        b"  Recommended".as_slice()
    } else {
        b"".as_slice()
    };
    for &byte in suffix {
        if pos < buf.len() {
            buf[pos] = byte;
            pos += 1;
        }
    }
    core::str::from_utf8(&buf[..pos]).unwrap_or("???")
}

fn fmt_countdown<'a>(previous_mode: DisplayMode, seconds: u32, buf: &'a mut [u8; 64]) -> &'a str {
    let prefix = b"Reverting to ";
    let mut pos = 0usize;
    for &byte in prefix {
        buf[pos] = byte;
        pos += 1;
    }
    write_u32_into(buf, &mut pos, previous_mode.width);
    if pos < buf.len() {
        buf[pos] = b'x';
        pos += 1;
    }
    write_u32_into(buf, &mut pos, previous_mode.height);
    let middle = b" in ";
    for &byte in middle {
        if pos < buf.len() {
            buf[pos] = byte;
            pos += 1;
        }
    }
    write_u32_into(buf, &mut pos, seconds);
    let suffix = if seconds == 1 {
        b" second.".as_slice()
    } else {
        b" seconds.".as_slice()
    };
    for &byte in suffix {
        if pos < buf.len() {
            buf[pos] = byte;
            pos += 1;
        }
    }
    core::str::from_utf8(&buf[..pos]).unwrap_or("Reverting automatically.")
}

fn read_preview_wallpaper_bytes(path: &[u8]) -> Option<alloc::vec::Vec<u8>> {
    use sunlight_libc as libc;

    let fd = libc::open(path).ok()?;
    let reserve = libc::fstat(fd)
        .ok()
        .map(|stat| (stat.size as usize).min(WALLPAPER_PREVIEW_MAX_BYTES))
        .unwrap_or(0);
    let mut out = alloc::vec::Vec::new();
    if out.try_reserve_exact(reserve).is_err() {
        let _ = libc::close(fd);
        return None;
    }
    let mut buf = [0u8; 4096];
    loop {
        let n = match libc::read(fd, &mut buf) {
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
        let take = (WALLPAPER_PREVIEW_MAX_BYTES - out.len()).min(n);
        out.extend_from_slice(&buf[..take]);
        if out.len() >= WALLPAPER_PREVIEW_MAX_BYTES || take < n {
            break;
        }
    }
    let _ = libc::close(fd);
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Fit-blit a decoded wallpaper with forced opaque ARGB (no gray alpha hole).
fn draw_wallpaper_preview_opaque(canvas: &mut Canvas, img: &RgbaImage, dst: Rect) {
    if img.width == 0 || img.height == 0 || dst.w == 0 || dst.h == 0 {
        return;
    }
    let cx0 = dst.x.max(0) as u32;
    let cy0 = dst.y.max(0) as u32;
    let cx1 = dst.right().min(canvas.width as i32).max(0) as u32;
    let cy1 = dst.bottom().min(canvas.height as i32).max(0) as u32;
    if cx0 >= cx1 || cy0 >= cy1 {
        return;
    }
    let dw = dst.w.max(1) as u64;
    let dh = dst.h.max(1) as u64;
    let sw = img.width as u64;
    let sh = img.height as u64;
    for dy in cy0..cy1 {
        let ly = (dy as i32 - dst.y) as u64;
        let sy = ((ly * sh) / dh) as u32;
        let row_off = dy as usize * canvas.stride as usize;
        for dx in cx0..cx1 {
            let lx = (dx as i32 - dst.x) as u64;
            let sx = ((lx * sw) / dw) as u32;
            let idx = row_off + dx as usize;
            if idx < canvas.pixels.len() {
                canvas.pixels[idx] = 0xFF00_0000 | (img.pixel(sx, sy) & 0x00FF_FFFF);
            }
        }
    }
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

#[cfg(not(test))]
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

#[cfg(not(test))]
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

    if let Some(lifecycle) =
        nameserver_lookup_timeout(WISEOWL_CONTROL_PANEL_LIFECYCLE_ENDPOINT, 250)
    {
        let _ = ipc_call_timeout(
            lifecycle,
            IpcMsg::with_label(WiseOwlLifecycleMsg::SOURCE_HELLO)
                .word(0, WiseOwlLifecycleMsg::SOURCE_CONTROL_PANEL),
            250,
        );
    }

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

    let mut window = match Window::connect_with_material(
        WindowConfig {
            width: WIN_W,
            height: WIN_H,
            title: "System Preferences",
            decoration: sunlight_ui::WindowDecoration::Normal,
        },
        WindowMaterial::WindowGlass,
    ) {
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
            match ControlPanelPage::from_cli_id(next) {
                Some(ControlPanelPage::Wallpaper) => return Page::Wallpaper,
                Some(ControlPanelPage::Display) => return Page::Monitor,
                Some(ControlPanelPage::AboutComputer) => return Page::AboutComputer,
                Some(ControlPanelPage::AboutOs) => return Page::AboutOs,
                Some(ControlPanelPage::Network) => return Page::Network,
                Some(ControlPanelPage::PowerThermal) => return Page::PowerThermal,
                Some(ControlPanelPage::DateTime) => return Page::DateTime,
                Some(ControlPanelPage::LoginSession) => return Page::LoginSession,
                Some(ControlPanelPage::Sound) => return Page::Sound,
                None => {}
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    Page::Grid
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_shell_tracks_root_dimensions() {
        let layout = ControlPanelApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H));
        assert_eq!(layout.root, Rect::new(0, 0, WIN_W, WIN_H));
        assert_eq!(layout.header.w, WIN_W);
        assert_eq!(layout.header.h, PAGE_HEADER_H);
        assert_eq!(layout.content.w, WIN_W);
        assert_eq!(layout.content.h, WIN_H - PAGE_HEADER_H);
        assert_eq!(layout.content.y, PAGE_HEADER_H as i32);
        assert_eq!(layout.header.bottom(), layout.content.y);
    }

    #[test]
    fn navigation_cards_stay_centered_while_content_expands() {
        let initial = ControlPanelApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H));
        let wide = ControlPanelApp::compute_layout(Rect::new(0, 0, WIN_W + 200, WIN_H));
        let tall = ControlPanelApp::compute_layout(Rect::new(0, 0, WIN_W, WIN_H + 180));
        assert_eq!(wide.header.w, initial.header.w + 200);
        assert_eq!(wide.content.w, initial.content.w + 200);
        assert_eq!(tall.content.h, initial.content.h + 180);
        assert_eq!(wide.header.h, PAGE_HEADER_H);
        let card_row_w = 136 * 3 + 14 * 2;
        let start_wide = (wide.root.w as i32 - card_row_w) / 2;
        let start_initial = (initial.root.w as i32 - card_row_w) / 2;
        assert!(start_wide > start_initial);
    }

    #[test]
    fn settings_page_content_follows_available_width() {
        let content = |w: u32, h: u32| {
            Rect::new(12, 12, w.saturating_sub(24), h.saturating_sub(24))
        };
        assert_eq!(content(WIN_W, WIN_H).w, WIN_W - 24);
        assert_eq!(content(WIN_W + 160, WIN_H).w, WIN_W + 136);
        assert_eq!(content(WIN_W, WIN_H + 80).h, WIN_H + 56);
        let back_y = |h: u32| h as i32 - 44;
        assert_eq!(back_y(WIN_H + 80), back_y(WIN_H) + 80);
    }

    #[test]
    fn tiny_windows_remain_safe_and_repeated_layout_is_identical() {
        for bounds in [
            Rect::new(0, 0, 0, 0),
            Rect::new(0, 0, 1, 1),
            Rect::new(0, 0, 40, 20),
        ] {
            let layout = ControlPanelApp::compute_layout(bounds);
            assert_eq!(layout.root, bounds);
        }
        let bounds = Rect::new(0, 0, 720, 640);
        assert_eq!(
            ControlPanelApp::compute_layout(bounds),
            ControlPanelApp::compute_layout(bounds)
        );
    }
}
