//! Vortex Shell — SunlightOS desktop surface.
//!
//! Renders the wallpaper fullscreen plus two shell panel strips:
//!   • Top bar:    [☀ workspaces]  [App Title / SunlightOS]  [status cluster]
//!   • Bottom bar: [overview|sidebar|settings]  [grid|term|tasks|calc]  [Search…]
//!
//! Top-right status cluster (implemented here):
//!   [power] [network] [battery] HH:MM AM/PM
//!
//! Clock source: "tz" service via TzMsg::GET_LOCAL_TIME (word0 packed y/m/d/h/min/s).
//!   Same path used internally by sunlight-top (telemetry fill_local_time calls tz).
//!   Compact 12h format (e.g. "5:29 AM").
//!
//! Network status source: "networkd" via NetworkdMsg::LIST_INTERFACES + unpack_iface_summary.
//!   Any non-Loopback iface with link in {Up, Carrier} => connected glyph (green).
//!   Only loopback or none => disabled glyph. Direct IPC, not shelling to networkctl.
//!
//! Battery: static placeholder icon. No ACPI queries.
//!   TODO(battery): integrate real battery via powerd context or future battery service.
//!
//! Top-bar power button: icon only, click is a no-op. Session/power actions
//!   (Sleep/Restart/Shut Down) live in the Start Menu footer instead — see
//!   `start_menu.rs` and `docs/GUI/START_MENU.md`. Restart/Shut Down call
//!   `sunlight_libc::power` directly (kernel `PowerCtl` syscall / ACPI);
//!   Sleep has no kernel support yet and just shows a notification.
//!
//! Start Menu: dark, structured app launcher opened via the dock's grid icon.
//!   Search, Pinned/All Apps/Recent sections, footer power actions. See
//!   `start_menu.rs` for the view model/layout and `docs/GUI/START_MENU.md`
//!   for the architecture writeup and known limitations.
//!
//! Update frequency: driven by Window::POLL_TIMEOUT_MS (~200 ms Event::Tick).
//!   Redraw is requested only on visible change (minute rollover or net state flip)
//!   to keep commits minimal. The shell uses double-buffered SHM; full view() on
//!   dirty still yields no visible flicker for small status updates.
//!
//! Constraints observed:
//!   - No battery driver logic.
//!   - No networkd changes.
//!   - sunlight-top left unchanged.
//!   - Existing shell appearance and layout preserved.
//!
//! Deliverables (this file):
//!   changed files: services/sunlight-vortex-shell/src/main.rs
//!   clock source used: "tz" (TzMsg::GET_LOCAL_TIME)
//!   network status source used: "networkd" (LIST_INTERFACES + IfaceSummary)
//!   update frequency: ~200 ms Tick (POLL_TIMEOUT_MS), redraw only on content change
//!   fake battery behavior: static icon (BAT_ROWS), no queries
//!   TODOs: see TODO(battery) marker above.

#![no_std]
#![no_main]

extern crate alloc;

mod start_menu;

use alloc::{string::String, vec::Vec};
use sun_font::{self, draw_text_vcenter, measure_text, FontRole, TextStyle};
use sunlight_calendar::{
    build_selected_day_previews, SelectedDayReminderPreview, SelectedDayTaskPreview,
};
use sunlight_ipc::{
    debug_log, get_time_utc, ipc_call_timeout,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_lookup, nameserver_lookup_timeout, notification_dnd_enabled,
    notification_kv_get_into, notification_kv_put, notification_set_dnd, process_is_alive,
    process_yield, query_display_metrics, shm_alloc, shm_free, shm_map, show_notification,
    unpack_iface_summary, CapabilityToken, DisplayMetrics, InterfaceKind, IpcMsg, LinkState,
    NetworkdMsg, NotificationKind, NotificationPriority, ProcessExit, SgpMsg, TzMsg,
    NOTIFICATION_RECENT_KEY, SAFE_FALLBACK_H, SAFE_FALLBACK_W, SHM_PAGE,
};
use sunlight_libc::{self as libc, sun_exec, sun_open, DirEntry, FT_DIR};
use sunlight_reminders::{
    by_date_list_key as reminder_due_date_list_key, decode_task,
    parse_id_list as parse_task_id_list, reminder_date_list_key, task_key,
};
use sunlight_telemetry::{SystemSnapshot, Telemetry};
use sunlight_ui::{
    image::{
        icon_theme::{self, category as icon_category, name as icon_name},
        mime_icon, TgaImage,
    },
    App, Canvas, Color, Event, Point, Rect, Theme, Window, WindowConfig,
};
use sunlight_wallpaper::{is_supported_wallpaper, load_desktop_config, DesktopConfig};

// ---------------------------------------------------------------------------
// Wallpaper asset
// ---------------------------------------------------------------------------

const FALLBACK_BG: u32 = 0x00121214;
const DESKTOP_CONFIG_PATH: &[u8] = b"/home/root/.config/sunlight/desktop.toml";

// ---------------------------------------------------------------------------
// Icon theme — SunlightOS icon set (Breeze-inspired, 256×256 BGRA TGA type-2)
// Each icon is embedded at compile time; decoded on-demand via TgaImage::pixel_argb.
// ---------------------------------------------------------------------------

// Desktop icons
static ICON_COMPUTER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/computer.tga");
static ICON_HOME_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/places/16/user-home.tga");
static ICON_TRASH_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/places/16/user-trash.tga");
static ICON_FOLDER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/places/16/folder.tga");
static ICON_INODE_DIRECTORY_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/inode-directory.tga");
static ICON_DRIVE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/drive-harddisk.tga");
static ICON_NETWORK_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/network-card.tga");
static ICON_FILE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-x-generic.tga");
static ICON_IMAGE_FILE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/image-x-generic.tga");
static ICON_TEXT_PLAIN_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-plain.tga");
static ICON_TEXT_MARKDOWN_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-markdown.tga");
static ICON_TEXT_RUST_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-rust.tga");
static ICON_APPLICATION_JSON_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/application-json.tga");
static ICON_APPLICATION_EXECUTABLE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/application-x-executable.tga");
static ICON_APPLICATION_OCTET_STREAM_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/application-octet-stream.tga");
static ICON_AUDIO_GENERIC_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/audio-x-generic.tga");
static ICON_VIDEO_GENERIC_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/video-x-generic.tga");
static ICON_UNKNOWN_FILE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/unknown.tga");

// Dock icons
static ICON_TERMINAL_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/utilities-terminal.tga");
static ICON_CALC_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/accessories-calculator.tga");
static ICON_FILES_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/places/16/system-file-manager.tga");
static ICON_SETTINGS_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/preferences-system.tga");
static ICON_CALENDAR_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/org.kde.merkuro.calendar.tga");
static ICON_RUNNER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/system-run.tga");
static ICON_GENERIC_APP_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/applications-system.tga");
static ICON_API_LAB_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/apifox.tga");
static ICON_TASKS_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/ksysguard.tga");
static ICON_BENCH_TGA: &[u8] = include_bytes!("../../../docs/icons/SunlightOS/apps/48/cpu-x.tga");
static ICON_EYES_TGA: &[u8] = include_bytes!("../../../docs/icons/SunlightOS/apps/48/kmag.tga");
static ICON_TEXT_EDITOR_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/kate.tga");
static ICON_WRITER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/libreoffice-writer.tga");
static MENU_NEW_FOLDER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/folder-new.tga");
static MENU_NEW_TEXT_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/document-new.tga");
static MENU_REFRESH_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/view-refresh.tga");
static MENU_SORT_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/sort-name.tga");
static MENU_TERMINAL_TGA: &[u8] = include_bytes!(
    "../../../docs/icons/SunlightOS/actions/scalable/xsi-utilities-terminal-symbolic.tga"
);

// Material Symbols glyphs (rasterized at build time from assets/fonts/material-symbols/
// using the local font. Provides clean vector icons for panel/dock/search at small sizes.
static ICON_SYM_HOME_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_home.tga"));
static ICON_SYM_SEARCH_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_search.tga"));
static ICON_SYM_TERMINAL_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_terminal.tga"));
static ICON_SYM_FOLDER_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_folder.tga"));
static ICON_SYM_CALENDAR_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_calendar_month.tga"));
static ICON_SYM_NOTIF_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_notifications.tga"));
static ICON_SYM_LOGOUT_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_logout.tga"));
static ICON_SYM_LAN_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_lan.tga"));
static ICON_SYM_MENU_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_menu.tga"));
static ICON_SYM_SETTINGS_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_settings.tga"));
static ICON_SYM_EDIT_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_edit.tga"));
static ICON_SYM_CALC_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_calculate.tga"));
static ICON_SYM_PUBLIC_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_public.tga"));
static ICON_SYM_CODE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_code.tga"));
static ICON_SYM_ARTICLE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_article.tga"));
static ICON_SYM_SUNNY_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_sunny.tga"));
static ICON_SYM_START_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_start_menu.tga"));
static ICON_SYM_CLOSE_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_close.tga"));
static ICON_SYM_DND_ON_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_do_not_disturb_on.tga"));
static ICON_SYM_DND_OFF_TGA: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon_do_not_disturb_off.tga"));

/// Theme icons for desktop shortcuts. All fields are `Copy` (TgaImage borrows `&'static [u8]`).
#[derive(Clone, Copy)]
struct DesktopTheme {
    computer: Option<TgaImage>,
    home: Option<TgaImage>,
    trash: Option<TgaImage>,
    folder: Option<TgaImage>,
    inode_directory: Option<TgaImage>,
    drive: Option<TgaImage>,
    network: Option<TgaImage>,
    file: Option<TgaImage>,
    image: Option<TgaImage>,
    text_plain: Option<TgaImage>,
    text_markdown: Option<TgaImage>,
    text_rust: Option<TgaImage>,
    application_json: Option<TgaImage>,
    application_executable: Option<TgaImage>,
    application_octet_stream: Option<TgaImage>,
    audio_generic: Option<TgaImage>,
    video_generic: Option<TgaImage>,
    unknown: Option<TgaImage>,
}

impl DesktopTheme {
    fn load() -> Self {
        Self {
            computer: TgaImage::parse(ICON_COMPUTER_TGA).ok(),
            home: TgaImage::parse(ICON_HOME_TGA).ok(),
            trash: TgaImage::parse(ICON_TRASH_TGA).ok(),
            folder: TgaImage::parse(ICON_FOLDER_TGA).ok(),
            inode_directory: TgaImage::parse(ICON_INODE_DIRECTORY_TGA).ok(),
            drive: TgaImage::parse(ICON_DRIVE_TGA).ok(),
            network: TgaImage::parse(ICON_NETWORK_TGA).ok(),
            file: TgaImage::parse(ICON_FILE_TGA).ok(),
            image: TgaImage::parse(ICON_IMAGE_FILE_TGA).ok(),
            text_plain: TgaImage::parse(ICON_TEXT_PLAIN_TGA).ok(),
            text_markdown: TgaImage::parse(ICON_TEXT_MARKDOWN_TGA).ok(),
            text_rust: TgaImage::parse(ICON_TEXT_RUST_TGA).ok(),
            application_json: TgaImage::parse(ICON_APPLICATION_JSON_TGA).ok(),
            application_executable: TgaImage::parse(ICON_APPLICATION_EXECUTABLE_TGA).ok(),
            application_octet_stream: TgaImage::parse(ICON_APPLICATION_OCTET_STREAM_TGA).ok(),
            audio_generic: TgaImage::parse(ICON_AUDIO_GENERIC_TGA).ok(),
            video_generic: TgaImage::parse(ICON_VIDEO_GENERIC_TGA).ok(),
            unknown: TgaImage::parse(ICON_UNKNOWN_FILE_TGA).ok(),
        }
    }

    fn icon_for(&self, kind: DesktopIconKind) -> Option<TgaImage> {
        match kind {
            DesktopIconKind::Computer => self.computer,
            DesktopIconKind::Home => self.home,
            DesktopIconKind::Trash => self.trash,
            DesktopIconKind::Folder => self.folder,
            DesktopIconKind::Image => self.image,
            DesktopIconKind::File | DesktopIconKind::DesktopEntry => self.file,
            DesktopIconKind::Drive => self.drive,
            DesktopIconKind::Network => self.network,
        }
    }

    fn icon_for_entry(&self, kind: DesktopIconKind, name: &str) -> Option<TgaImage> {
        if matches!(
            kind,
            DesktopIconKind::Computer
                | DesktopIconKind::Home
                | DesktopIconKind::Trash
                | DesktopIconKind::Drive
                | DesktopIconKind::Network
        ) {
            return self.icon_for(kind);
        }
        if kind == DesktopIconKind::Folder {
            return self
                .folder
                .or(self.inode_directory)
                .or(self.application_octet_stream);
        }

        let mime = sunlight_libc::sun_open::mime_from_path(name.as_bytes());
        let mut exact_name = [0u8; mime_icon::MAX_MIME_ICON_NAME];
        let lookup = mime_icon::resolve_file_icon(mime, &mut exact_name);
        if let Some(icon_name) = lookup.exact {
            if let Some(icon) = self.icon_by_name(icon_name) {
                return Some(icon);
            }
        }
        if let Some(icon_name) = lookup.family {
            if let Some(icon) = self.icon_by_name(icon_name) {
                return Some(icon);
            }
        }
        self.icon_by_name(lookup.generic)
            .or(self.icon_by_name(mime_icon::UNKNOWN_ICON))
    }

    fn icon_by_name(&self, name: &str) -> Option<TgaImage> {
        match name {
            "folder" => self.folder,
            "inode-directory" => self.inode_directory,
            "text-plain" => self.text_plain,
            "text-markdown" => self.text_markdown,
            "text-rust" => self.text_rust,
            "text-x-generic" => self.file,
            "image-x-generic" => self.image,
            "application-json" => self.application_json,
            "application-x-executable" => self.application_executable,
            "application-octet-stream" => self.application_octet_stream,
            "audio-x-generic" => self.audio_generic,
            "video-x-generic" => self.video_generic,
            "unknown" => self.unknown,
            _ => None,
        }
    }
}

/// Theme icons for the bottom dock.
#[derive(Clone, Copy)]
struct DockTheme {
    terminal: Option<TgaImage>,
    calendar: Option<TgaImage>,
    calc: Option<TgaImage>,
    files: Option<TgaImage>,
    settings: Option<TgaImage>,
}

impl DockTheme {
    fn load() -> Self {
        Self {
            terminal: TgaImage::parse(ICON_TERMINAL_TGA).ok(),
            calendar: TgaImage::parse(ICON_CALENDAR_TGA).ok(),
            calc: TgaImage::parse(ICON_CALC_TGA).ok(),
            files: TgaImage::parse(ICON_FILES_TGA).ok(),
            settings: TgaImage::parse(ICON_SETTINGS_TGA).ok(),
        }
    }

    /// Return TGA icon for dock slot index (0=grid/launcher, 1=terminal, 2=calendar, 3=calc, 4=files).
    fn dock_icon(&self, idx: usize) -> Option<TgaImage> {
        match idx {
            0 => None, // launcher grid — keep pixel-art
            1 => self.terminal,
            2 => self.calendar,
            3 => self.calc,
            4 => self.files,
            _ => None,
        }
    }

    fn icon_for_app(&self, app_id: AppId) -> Option<TgaImage> {
        match app_id {
            AppId::Terminal => self.terminal,
            AppId::Chronos => self.terminal,
            AppId::Calculator => self.calc,
            AppId::Files => self.files,
            AppId::Settings => self.settings,
            AppId::Calendar => self.calendar,
            // Not rendered in the bottom dock; the Start Menu owns their icons.
            AppId::Tasks
            | AppId::Bench
            | AppId::Eyes
            | AppId::TextEditor
            | AppId::Writer
            | AppId::RappidRabbit
            | AppId::ApiLab => None,
        }
    }
}

/// Small Material Symbols glyph set loaded from build-time rasters.
/// Orange accent applied at draw time for active/system items.
/// Centralized to avoid scattering raw glyph strings or per-site loads.
#[derive(Clone, Copy)]
struct SymbolTheme {
    home: Option<TgaImage>,
    search: Option<TgaImage>,
    terminal: Option<TgaImage>,
    folder: Option<TgaImage>,
    calendar: Option<TgaImage>,
    notifications: Option<TgaImage>,
    logout: Option<TgaImage>,
    lan: Option<TgaImage>,
    menu: Option<TgaImage>,
    settings: Option<TgaImage>,
    edit: Option<TgaImage>,
    calc: Option<TgaImage>,
    public: Option<TgaImage>,
    code: Option<TgaImage>,
    article: Option<TgaImage>,
    sunny: Option<TgaImage>,
    start: Option<TgaImage>,
    close: Option<TgaImage>,
    dnd_on: Option<TgaImage>,
    dnd_off: Option<TgaImage>,
}

impl SymbolTheme {
    fn load() -> Self {
        Self {
            home: TgaImage::parse(ICON_SYM_HOME_TGA).ok(),
            search: TgaImage::parse(ICON_SYM_SEARCH_TGA).ok(),
            terminal: TgaImage::parse(ICON_SYM_TERMINAL_TGA).ok(),
            folder: TgaImage::parse(ICON_SYM_FOLDER_TGA).ok(),
            calendar: TgaImage::parse(ICON_SYM_CALENDAR_TGA).ok(),
            notifications: TgaImage::parse(ICON_SYM_NOTIF_TGA).ok(),
            logout: TgaImage::parse(ICON_SYM_LOGOUT_TGA).ok(),
            lan: TgaImage::parse(ICON_SYM_LAN_TGA).ok(),
            menu: TgaImage::parse(ICON_SYM_MENU_TGA).ok(),
            settings: TgaImage::parse(ICON_SYM_SETTINGS_TGA).ok(),
            edit: TgaImage::parse(ICON_SYM_EDIT_TGA).ok(),
            calc: TgaImage::parse(ICON_SYM_CALC_TGA).ok(),
            public: TgaImage::parse(ICON_SYM_PUBLIC_TGA).ok(),
            code: TgaImage::parse(ICON_SYM_CODE_TGA).ok(),
            article: TgaImage::parse(ICON_SYM_ARTICLE_TGA).ok(),
            sunny: TgaImage::parse(ICON_SYM_SUNNY_TGA).ok(),
            start: TgaImage::parse(ICON_SYM_START_TGA).ok(),
            close: TgaImage::parse(ICON_SYM_CLOSE_TGA).ok(),
            dnd_on: TgaImage::parse(ICON_SYM_DND_ON_TGA).ok(),
            dnd_off: TgaImage::parse(ICON_SYM_DND_OFF_TGA).ok(),
        }
    }

    fn get(&self, name: &str) -> Option<TgaImage> {
        match name {
            "home" => self.home,
            "search" => self.search,
            "terminal" => self.terminal,
            "folder" => self.folder,
            "calendar_month" => self.calendar,
            "notifications" => self.notifications,
            "logout" => self.logout,
            "lan" => self.lan,
            "menu" => self.menu,
            "settings" => self.settings,
            "edit" => self.edit,
            "calculate" => self.calc,
            "public" => self.public,
            "code" => self.code,
            "article" => self.article,
            "sunny" => self.sunny,
            "start" => self.start.or(self.menu),
            "close" => self.close,
            "do_not_disturb_on" => self.dnd_on,
            "do_not_disturb_off" => self.dnd_off,
            _ => None,
        }
    }
}

/// Identifies a launchable SunlightOS app.
///
/// `Terminal`/`Calculator`/`Files`/`Settings` are tracked by both the bottom
/// dock and the Start Menu. Chronos and the remaining apps reuse the same
/// registry/launch machinery but are shown through the Start Menu rather than
/// the fixed 4-icon dock.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppId {
    Terminal,
    Chronos,
    Calculator,
    Files,
    Settings,
    Tasks,
    Bench,
    Eyes,
    TextEditor,
    Writer,
    Calendar,
    RappidRabbit,
    ApiLab,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppLaunchState {
    NotRunning,
    Launching,
    Running,
    Minimized,
    Closing,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DockZone {
    Placeholder,
    App(AppId),
}

#[derive(Clone, Copy)]
pub(crate) struct DockAppState {
    pub(crate) app_id: AppId,
    pub(crate) display_name: &'static str,
    icon: AppId,
    pid: Option<u64>,
    main_window_id: Option<u64>,
    pub(crate) state: AppLaunchState,
    last_launch_id: u64,
    last_launch_source: LaunchSource,
    last_launch_started_at: u64,
    last_click_at: u64,
    launch_error: [u8; 64],
    launch_error_len: usize,
    pub(crate) launch_attempts: u32,
    duplicate_blocks: u32,
}

impl DockAppState {
    const fn new(app_id: AppId, display_name: &'static str, icon: AppId) -> Self {
        Self {
            app_id,
            display_name,
            icon,
            pid: None,
            main_window_id: None,
            state: AppLaunchState::NotRunning,
            last_launch_id: 0,
            last_launch_source: LaunchSource::Unknown,
            last_launch_started_at: 0,
            last_click_at: 0,
            launch_error: [0; 64],
            launch_error_len: 0,
            launch_attempts: 0,
            duplicate_blocks: 0,
        }
    }

    fn clear_error(&mut self) {
        self.launch_error_len = 0;
    }

    fn set_error(&mut self, text: &str) {
        let bytes = text.as_bytes();
        let n = bytes.len().min(self.launch_error.len());
        self.launch_error[..n].copy_from_slice(&bytes[..n]);
        self.launch_error_len = n;
    }

    fn error_str(&self) -> &str {
        core::str::from_utf8(&self.launch_error[..self.launch_error_len]).unwrap_or("")
    }
}

#[derive(Clone, Copy)]
struct WindowSnapshot {
    id: u64,
    owner_pid: u64,
    state: u64,
    window_type: u64,
    workspace_id: u64,
    hidden: bool,
    rolled_up: bool,
    title: [u8; 16],
}

#[derive(Clone)]
enum RunningIcon {
    Static(TgaImage),
    Runtime(Vec<u8>),
    Missing,
}

#[derive(Clone, PartialEq, Eq)]
struct IconOverride {
    app_key: String,
    icon_ref: String,
}

struct RunningAppEntry {
    win_id: u64,
    pid: u64,
    display_name: String,
    cell_w: u32,
    minimized: bool,
    icon_hint: String,
    icon: Option<RunningIcon>,
    last_click_at: u64,
}

#[derive(Clone)]
struct CalendarMiniEvent {
    title: String,
    time: String,
}

impl RunningAppEntry {
    /// Recompute the cell width. Running-app cells are icon-only (the full
    /// title is shown via the hover tooltip), so every cell is the same
    /// icon-only width regardless of minimized state.
    fn refresh_cell_width(&mut self) {
        self.cell_w = RUNNING_CELL_PAD as u32 * 2 + RUNNING_ICON;
    }
}

// ---------------------------------------------------------------------------
// Window geometry
// ---------------------------------------------------------------------------

// Desktop-layer config flags (see app.rs WindowConfig docs).
// bits[1:0]=2 Desktop, bits[3:2]=3 Fullscreen, bit[4]=1 NoChrome → 0x1E
const DESKTOP_LAYER_FLAGS: u64 = 0x1E;

// ---------------------------------------------------------------------------
// Panel geometry constants
// ---------------------------------------------------------------------------

const RADIUS: u32 = 7;
pub(crate) const TOP_H: u32 = 36; // top bar height
pub(crate) const TOP_Y: i32 = 6; // top bar Y offset from screen top
pub(crate) const TOP_PAD: i32 = 8; // horizontal margin from screen edge

pub(crate) const BOT_H: u32 = 44; // bottom cluster height
pub(crate) const BOT_Y_OFF: i32 = 8; // distance from screen bottom to bottom of cluster
const ICON_BTN: u32 = 36; // square size for icon buttons in clusters
const CLUSTER_PAD: i32 = 6; // inner horizontal padding inside clusters
const ICON_GAP: i32 = 4; // gap between icon buttons
                         // Top-right status cluster spacing. Kept as constants so the balance of the
                         // [lan][notif][logout] on the right; workspace indicator (semantic icons) on the
                         // left after the SunlightOS brand. All tunable in one place.
const TOP_ICON_SIZE: u32 = 18; // status icon size in the top bar
const TOP_ICON_GAP: i32 = 8; // gap between top-bar status icons (was 4 — too cramped)
const TOP_RIGHT_PAD: i32 = 8; // right margin of the top-right cluster
const TOP_WS_LEFT_GAP: i32 = 12; // gap between brand and workspace indicator
const WS_INDICATOR_COUNT: usize = 4; // workspaces 1..=4
const WS_BTN_W: u32 = 18; // workspace indicator button width
const WS_BTN_H: u32 = 18; // workspace indicator button height
const WS_ICON_SIZE: u32 = 16; // glyph drawn inside a workspace button
const WS_BTN_GAP: i32 = 4; // gap between workspace indicator buttons
const RUNNING_ICON: u32 = 24; // icon size inside running-app cells
const RUNNING_CELL_PAD: i32 = 6; // inner padding inside running-app cells
const RUNNING_NAME_BUF: usize = 64;
const RUNNING_MINIMIZED_DOT: u32 = 6;
/// How long the pointer must rest on a running-app cell before its title
/// tooltip appears (ms). Keeps the bar calm while sweeping across items.
const RUNNING_TOOLTIP_DELAY_MS: u64 = 300;
const MAX_RUNNING_TRACKED: usize = 32;
const ENABLE_RUNNING_TASKBAR: bool = true;

const SEARCH_W: u32 = 200; // search box width
const SEARCH_H: u32 = 32; // search box height
const MAX_RECENT_APPS: usize = 6; // Start Menu "Recent" section cap
const STATUS_POLL_MS: u64 = 1000;
const TIME_IPC_TIMEOUT_MS: u64 = 250;
const NET_IPC_TIMEOUT_MS: u64 = 50;
const DISPLAY_IPC_TIMEOUT_MS: u64 = 50;
const KV_LOOKUP_TIMEOUT_MS: u64 = 250;
const KV_IPC_TIMEOUT_MS: u64 = 250;
const KV_VALUE: u64 = 0x4B05;
const KV_ERROR: u64 = 0x4BEE;
const KV_GET_SHM2: u64 = 0x4B09;
const CAL_EVENT_PREFIX: &str = "app.calendar.events/";
const CAL_INDEX_BY_DATE_PREFIX: &str = "app.calendar.index/by-date/";
const CAL_EVENT_TITLE_MAX: usize = 48;
const CAL_POPUP_DAYS: usize = 42;
const CAL_POPUP_EVENTS: usize = 8;
const CAL_POPUP_TASKS: usize = 4;
const CAL_POPUP_REMINDERS: usize = 4;
const APP_STATE_POLL_MS: u64 = 250;
const APP_LAUNCH_TIMEOUT_MS: u64 = 30_000;
const APP_PRESS_MS: u64 = 140;
const DESKTOP_CELL_W: u32 = 92;
const DESKTOP_CELL_H: u32 = 88;
const DESKTOP_ICON_SCALE: u32 = 2;
const DESKTOP_LABEL_CHARS: usize = 12;
const MAX_DIR_ENTRIES: usize = 48;
const MENU_W: u32 = 156;
const MENU_ITEM_H: u32 = 22;
const NOTIF_CENTER_W: u32 = 320;
const NOTIF_CENTER_RECENT_LIMIT: usize = 32;
const NOTIF_DISMISS_ZONES_MAX: usize = 32;

static mut KV_CAP_CACHE: CapabilityToken = CapabilityToken::INVALID;

// ---------------------------------------------------------------------------
// Heap (bump allocator — no dynamic allocations used)
// ---------------------------------------------------------------------------

const HEAP_SIZE: usize = 8 * 1024 * 1024;
const WALLPAPER_MAX_BYTES: usize = 8 * 1024 * 1024;
#[repr(align(16))]
struct BumpHeap(core::cell::UnsafeCell<[u8; HEAP_SIZE]>);
unsafe impl Sync for BumpHeap {}
static BUMP_HEAP: BumpHeap = BumpHeap(core::cell::UnsafeCell::new([0u8; HEAP_SIZE]));

/// Bump pointer for `BumpAlloc`, hoisted to module scope (instead of a
/// function-local `static`) so `shell_heap_used_bytes()` below can report
/// live usage for the temporary heap diagnostics (Part A #1/#2).
static HEAP_NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
/// Count of allocations that failed because the bump heap is exhausted.
static HEAP_ALLOC_FAIL_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

struct BumpAlloc;
unsafe impl core::alloc::GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        use core::sync::atomic::Ordering;
        let heap_ptr = BUMP_HEAP.0.get() as *mut u8;
        let cur = HEAP_NEXT.load(Ordering::Relaxed);
        let aligned = (cur + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP_SIZE {
            HEAP_ALLOC_FAIL_COUNT.fetch_add(1, Ordering::Relaxed);
            return core::ptr::null_mut();
        }
        HEAP_NEXT.store(end, Ordering::Relaxed);
        unsafe { heap_ptr.add(aligned) }
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: BumpAlloc = BumpAlloc;

/// Temporary heap diagnostics (see AGENTS task: Part A #1/#2). These expose
/// the no-dealloc bump allocator's live usage so a stress test can confirm
/// `shell_heap_used_bytes()` stops growing after startup/first app
/// discovery, instead of creeping toward `shell_heap_capacity_bytes()`.
fn shell_heap_used_bytes() -> usize {
    HEAP_NEXT.load(core::sync::atomic::Ordering::Relaxed)
}

fn shell_heap_capacity_bytes() -> usize {
    HEAP_SIZE
}

fn shell_heap_alloc_fail_count() -> u64 {
    HEAP_ALLOC_FAIL_COUNT.load(core::sync::atomic::Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Pixel-art icon bitmaps (1 bit per pixel, u16 rows, MSB = leftmost pixel)
// Width is the number of significant bits; stored in the MSBs of each u16.
// All icons are 16×16 pixel fields scaled to fit an ICON_BTN×ICON_BTN cell.
// ---------------------------------------------------------------------------

/// Sun icon — filled circle + 4 cardinal rays (N/S/E/W).
const SUN_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0000000000000000,
    0b0000000100000000, // N ray
    0b0000000000000000,
    0b0000011111000000, // circle top (cols 5-9)
    0b0000111111100000, // circle (cols 4-10)
    0b0000111111100000,
    0b0100111111100100, // W ray + circle + E ray
    0b0000111111100000,
    0b0000111111100000,
    0b0000011111000000, // circle bottom (cols 5-9)
    0b0000000000000000,
    0b0000000100000000, // S ray
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Overview icon — 2×2 grid of rounded squares.
const OVERVIEW_ROWS: [u16; 16] = [
    0b0111101111000000,
    0b0100101001000000,
    0b0100101001000000,
    0b0111101111000000,
    0b0000000000000000,
    0b0111101111000000,
    0b0100101001000000,
    0b0100101001000000,
    0b0111101111000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Sidebar icon — vertical bar on left + content area.
const SIDEBAR_ROWS: [u16; 16] = [
    0b1111111111111100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1001000000000100,
    0b1111111111111100,
];

/// Settings icon — gear / cogwheel approximation.
const SETTINGS_ROWS: [u16; 16] = [
    0b0000011000000000,
    0b0001011010000000,
    0b0001111110000000,
    0b0011100011100000,
    0b0110100010110000,
    0b1101111111011000,
    0b1100111110011000,
    0b1100111110011000,
    0b1101111111011000,
    0b0110100010110000,
    0b0011100011100000,
    0b0001111110000000,
    0b0001011010000000,
    0b0000011000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Launcher grid icon — 3×3 dots.
const GRID_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0110001100011000,
    0b0110001100011000,
    0b0000000000000000,
    0b0000000000000000,
    0b0110001100011000,
    0b0110001100011000,
    0b0000000000000000,
    0b0000000000000000,
    0b0110001100011000,
    0b0110001100011000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Terminal icon — ">_" prompt shape.
const TERMINAL_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0000000000000000,
    0b1100000000000000,
    0b0110000000000000,
    0b0011000000000000,
    0b0110000000000000,
    0b1100000000000000,
    0b0000000000000000,
    0b0000111111100000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Calendar icon — simple page with date squares.
const CALENDAR_ROWS: [u16; 16] = [
    0b0011111111110000,
    0b0010101010100000,
    0b0011111111110000,
    0b0011000000110000,
    0b0011011000110000,
    0b0011000000110000,
    0b0011000000110000,
    0b0011011000110000,
    0b0011000000110000,
    0b0011000000110000,
    0b0011011000110000,
    0b0011000000110000,
    0b0011111111110000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Calculator icon — display + keypad grid.
const CALC_ROWS: [u16; 16] = [
    0b0111111111100000,
    0b0100000000100000,
    0b0100000000100000,
    0b0111111111100000,
    0b0000000000000000,
    0b0110011001100000,
    0b0110011001100000,
    0b0000000000000000,
    0b0110011001100000,
    0b0110011001100000,
    0b0000000000000000,
    0b0110011001100000,
    0b0110011001100000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Power icon — circle with vertical bar (⏻ style).
const POWER_ROWS: [u16; 16] = [
    0b0000011000000000,
    0b0001100110000000,
    0b0011000011000000,
    0b0010000001000000,
    0b0110000001100000,
    0b0100000000100000,
    0b0100011000100000,
    0b0100011000100000,
    0b0100000000100000,
    0b0100000000100000,
    0b0100000000100000,
    0b0010000001000000,
    0b0011000011000000,
    0b0001100110000000,
    0b0000011000000000,
    0b0000000000000000,
];

/// Network connected — simplified jack/plug with signal.
const NET_ON_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0000001111000000,
    0b0000011111100000,
    0b0000110000110000,
    0b0001100000011000,
    0b0001000000001000,
    0b0011000000001100,
    0b0010011111100100,
    0b0010011111100100,
    0b0011000000001100,
    0b0001000000001000,
    0b0001100000011000,
    0b0000110000110000,
    0b0000011111100000,
    0b0000001111000000,
    0b0000000000000000,
];

/// Network disconnected — same shape with X overlay.
const NET_OFF_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b1000001111000001,
    0b0100011111100010,
    0b0010110000110100,
    0b0001100000011000,
    0b1001000000001001,
    0b0111000000001100,
    0b0010011111100100,
    0b0010011111100100,
    0b0111000000001100,
    0b1001000000001001,
    0b0001100000011000,
    0b0010110000110100,
    0b0100011111100010,
    0b1000001111000001,
    0b0000000000000000,
];

/// Battery placeholder — body + terminal nub (static, not live).
const BAT_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0000000000000000,
    0b0011111111110000,
    0b0010000000010000,
    0b0010111011010000,
    0b0010111011010000,
    0b0010111011010000,
    0b0010000000010000,
    0b0011111111110000,
    0b0000011000000000,
    0b0000011000000000,
    0b0000011000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Folder icon.
const FOLDER_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0001111000000000,
    0b0011000111110000,
    0b0110000000011000,
    0b0111111111111000,
    0b0111111111111000,
    0b0111111111111000,
    0b0111111111111000,
    0b0111111111111000,
    0b0111111111111000,
    0b0111111111111000,
    0b0111111111111000,
    0b0111111111111000,
    0b0111111111111000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Generic document icon.
const FILE_ROWS: [u16; 16] = [
    0b0001111111000000,
    0b0001000001100000,
    0b0001000000110000,
    0b0001001111110000,
    0b0001001000010000,
    0b0001001111110000,
    0b0001001000010000,
    0b0001001111110000,
    0b0001001000010000,
    0b0001001111110000,
    0b0001000000010000,
    0b0001000000010000,
    0b0001000000010000,
    0b0001111111110000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Computer icon.
const COMPUTER_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0011111111110000,
    0b0010000000010000,
    0b0010111111010000,
    0b0010100001010000,
    0b0010100001010000,
    0b0010100001010000,
    0b0010111111010000,
    0b0010000000010000,
    0b0011111111110000,
    0b0000011111000000,
    0b0000001110000000,
    0b0000011111000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Home icon.
const HOME_ROWS: [u16; 16] = [
    0b0000001100000000,
    0b0000011110000000,
    0b0000110011000000,
    0b0001100001100000,
    0b0011111111110000,
    0b0011000000110000,
    0b0011000000110000,
    0b0011001100110000,
    0b0011001100110000,
    0b0011000000110000,
    0b0011000000110000,
    0b0011000000110000,
    0b0011111111110000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Trash icon.
const TRASH_ROWS: [u16; 16] = [
    0b0000011111000000,
    0b0001111111110000,
    0b0000011111000000,
    0b0000111111100000,
    0b0000110001100000,
    0b0000110001100000,
    0b0000110001100000,
    0b0000110001100000,
    0b0000110001100000,
    0b0000110001100000,
    0b0000110001100000,
    0b0000111111100000,
    0b0000111111100000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

/// Drive icon.
const DRIVE_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0011111111110000,
    0b0110000000011000,
    0b0110000000011000,
    0b0110000000011000,
    0b0110000000011000,
    0b0110000000011000,
    0b0111111111111000,
    0b0110000000011000,
    0b0110000000011000,
    0b0110000110011000,
    0b0110000110011000,
    0b0111111111111000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
];

// Width of the 16-pixel icon bitmap (used in draw_icon16).
const ICON16_W: u32 = 16;

// ---------------------------------------------------------------------------
// Click zone bookkeeping
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum DesktopIconKind {
    Computer,
    Home,
    Trash,
    Network,
    Drive,
    Folder,
    Image,
    File,
    DesktopEntry,
}

struct DesktopIcon {
    name: String,
    label: String,
    _tooltip: String,
    _action: String,
    kind: DesktopIconKind,
    rect: Rect,
}

const DESKTOP_DOUBLE_CLICK_MS: u64 = 400;

struct DesktopPaths {
    _username: String,
    home_dir: String,
    desktop_dir: String,
    trash_dir: String,
    hostname: String,
}

#[derive(Clone, Copy)]
enum ContextMenuAction {
    NewFolder,
    NewTextFile,
    Refresh,
    SortByName,
    OpenTerminalHere,
    WallpaperSettings,
}

#[derive(Clone, Copy)]
struct MenuItem {
    action: ContextMenuAction,
    rect: Rect,
    icon: Option<TgaImage>,
}

struct ContextMenuState {
    rect: Rect,
    items: [MenuItem; 6],
}

const MENU_LABELS: [(&str, ContextMenuAction); 6] = [
    ("New Folder", ContextMenuAction::NewFolder),
    ("New Text File", ContextMenuAction::NewTextFile),
    ("Refresh", ContextMenuAction::Refresh),
    ("Sort By Name", ContextMenuAction::SortByName),
    ("Open Terminal", ContextMenuAction::OpenTerminalHere),
    ("Wallpaper Settings", ContextMenuAction::WallpaperSettings),
];

// ---------------------------------------------------------------------------
// Desktop marquee-selection state machine
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum DesktopSelectState {
    Idle,
    /// Mouse button is down on empty desktop but hasn't moved past threshold yet.
    Armed {
        anchor: Point,
    },
    /// Past the 4-pixel threshold — rubber-band rectangle is visible.
    Dragging {
        anchor: Point,
        current: Point,
    },
}

// ---------------------------------------------------------------------------
// Shell application state
// ---------------------------------------------------------------------------

struct VortexShell {
    wallpaper: Option<TgaImage>,
    wallpaper_config: DesktopConfig,
    wallpaper_error: bool,
    wallpaper_last_reload_ms: u64,
    display_ep: CapabilityToken,
    desktop_paths: DesktopPaths,
    desktop_icons: Vec<DesktopIcon>,
    screen_w: u32,
    screen_h: u32,
    /// Bounds of each clickable dock button (local coords), plus the action.
    dock_zones: [(Rect, DockZone); 4],
    selected_icons: Vec<usize>,
    last_desktop_click_idx: Option<usize>,
    last_desktop_click_at: u64,
    context_menu: Option<ContextMenuState>,
    selection_state: DesktopSelectState,
    suppress_next_click: bool,
    /// Tracks whether mouse is hovering over a dock icon (index 0..4).
    hover: Option<usize>,
    /// Tracks hover on the settings button in the left cluster.
    settings_hover: bool,
    /// Cached local hour/min for the status clock.
    status_hour: u8,
    status_min: u8,
    // Extended for center date/time + tooltip (populated from tz GET_LOCAL_TIME)
    status_year: u16,
    status_month: u8,
    status_day: u8,
    status_sec: u8,
    // TZ id bytes for tooltip (from /etc/localtime or GET_ZONE best effort)
    tz_id: [u8; 48],
    tz_id_len: usize,
    // Effective locale for formatting (LC_TIME or LANG from /etc/locale.conf)
    locale: [u8; 48],
    locale_len: usize,
    /// Cached "any non-loopback interface up/carrier".
    status_net_up: bool,
    /// Next monotonic deadline for best-effort status polling.
    next_status_poll_ms: u64,
    /// Bounds of the power button for future click handling.
    power_zone: Rect,
    /// Bounds of the settings button in the left cluster.
    settings_zone: Rect,
    /// Bounds of the dock's grid icon — toggles the Start Menu.
    launcher_zone: Rect,
    /// TGA icon theme for desktop shortcuts.
    desktop_theme: DesktopTheme,
    /// TGA icon theme for the bottom dock.
    dock_theme: DockTheme,
    /// Material Symbols glyphs (panel, dock controls, search). Accent color at draw.
    symbols: SymbolTheme,
    /// App registry used for launch/focus/restore behavior. The first four
    /// entries (Terminal/Calculator/Files/Settings) are also shown in the
    /// bottom dock; Tasks/Bench/Eyes/TextEditor/Writer are Start-Menu-only
    /// but share the same launch/state-sync machinery.
    apps: [DockAppState; 13],
    /// Dynamic dock entries for visible non-pinned windows.
    running_apps: Vec<RunningAppEntry>,
    /// User-provided icon overrides loaded from `desktop.toml`.
    icon_overrides: Vec<IconOverride>,
    /// Reused scratch buffer for LIST_WINDOWS snapshots.
    window_snapshots: Vec<WindowSnapshot>,
    /// Clickable bounds for the dynamic running-app strip.
    running_zones: Vec<(Rect, u64)>,
    /// Hovered running-app index, if any.
    running_hover: Option<usize>,
    /// Monotonic timestamp (ms) when the current running-app hover began.
    /// Drives the tooltip open delay (see RUNNING_TOOLTIP_DELAY_MS).
    running_hover_since: Option<u64>,

    // Top panel interactive zones (updated during draw_top_bar for hit testing)
    datetime_zone: Rect,
    net_zone: Rect,
    notif_zone: Rect,
    logout_zone: Rect,
    /// Active workspace (1..=4), mirrored from the display server via snapshot
    /// refresh. Kept runtime-only — never persisted.
    current_workspace: u8,
    /// Bounds of the whole [1][2][3][4] cluster, for hit-testing clicks.
    workspace_zone: Rect,
    /// Per-button click targets inside the workspace indicator.
    workspace_btn_zones: [Rect; WS_INDICATOR_COUNT],

    // Popover / dialog / tooltip state (conservative, no overengineer)
    show_datetime_tooltip: bool,
    show_calendar_popover: bool,
    cal_popup_open_btn: Rect,
    cal_view_month: u8, // 1-12 current view (adjusted by offset)
    cal_view_year: u16,
    cal_selected_day: u8,
    cal_event_days: [bool; CAL_POPUP_DAYS],
    cal_selected_events: Vec<CalendarMiniEvent>,
    cal_selected_tasks: Vec<SelectedDayTaskPreview>,
    cal_selected_reminders: Vec<SelectedDayReminderPreview>,
    cal_last_loaded_key: [u8; 10],
    cal_last_loaded_key_len: usize,
    show_notif_panel: bool,
    notif_dnd_toggle_r: Rect,
    notif_mark_seen_r: Rect,
    notif_dismiss_r: Rect,
    notif_dismiss_zones: Vec<(Rect, String)>,
    show_logout_confirm: bool,

    // Stash for logout dialog button rects (simple, recomputed often)
    logout_cancel_r: Rect,
    logout_confirm_r: Rect,
    /// Optional telemetry snapshot source for process-name fallback.
    telemetry: Option<Telemetry>,
    /// Next monotonic deadline for app/window registry polling.
    next_app_poll_ms: u64,
    /// Next launch trace id assigned by this shell process.
    next_launch_id: u64,
    /// Dark Start Menu overlay — search, pinned/all-apps/recent sections,
    /// power actions. See `start_menu.rs` and `docs/GUI/START_MENU.md`.
    start_menu: start_menu::StartMenuState,
    /// After the Start Menu closes because the user clicked the dock grid
    /// icon, suppress the follow-up `Click` so the menu stays closed.
    suppress_launcher_open: bool,
    /// Session-only most-recently-used app list (newest first, capped),
    /// shown in the Start Menu's "Recent" section. Not persisted across
    /// restarts — falls back to a static "Suggested" set when empty.
    recent_apps: Vec<AppId>,
}

impl VortexShell {
    fn new(display_ep: CapabilityToken) -> Self {
        let wallpaper_config = load_desktop_config();
        let (wallpaper, wallpaper_error) = load_wallpaper_from_config(&wallpaper_config);
        let icon_overrides = load_desktop_icon_overrides();
        let desktop_paths = resolve_desktop_paths();
        ensure_directory(&desktop_paths.desktop_dir);
        if wallpaper.is_some() {
            debug_log("[VORTEX] wallpaper loaded\n");
        } else {
            debug_log("[VORTEX] wallpaper unavailable — using fallback\n");
        }
        let desktop_theme = DesktopTheme::load();
        let dock_theme = DockTheme::load();
        let symbols = SymbolTheme::load();
        let telemetry = if ENABLE_RUNNING_TASKBAR {
            let telemetry = Telemetry::init().ok();
            if telemetry.is_none() {
                debug_log("[VORTEX] telemetry unavailable for running-app names\n");
            }
            telemetry
        } else {
            debug_log("[VORTEX] running taskbar disabled for perf test\n");
            None
        };
        let mut shell = Self {
            wallpaper,
            wallpaper_config,
            wallpaper_error,
            wallpaper_last_reload_ms: monotonic_millis(),
            display_ep,
            desktop_paths,
            desktop_icons: Vec::new(),
            screen_w: SAFE_FALLBACK_W,
            screen_h: SAFE_FALLBACK_H,
            dock_zones: [(Rect::new(0, 0, 0, 0), DockZone::Placeholder); 4],
            selected_icons: Vec::new(),
            last_desktop_click_idx: None,
            last_desktop_click_at: 0,
            context_menu: None,
            selection_state: DesktopSelectState::Idle,
            suppress_next_click: false,
            hover: None,
            settings_hover: false,
            status_hour: 0xff,
            status_min: 0xff,
            status_year: 1970,
            status_month: 1,
            status_day: 1,
            status_sec: 0,
            tz_id: [
                b'U', b'T', b'C', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            tz_id_len: 3,
            locale: [
                b'C', b'.', b'U', b'T', b'F', b'-', b'8', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
            locale_len: 7,
            status_net_up: false,
            next_status_poll_ms: 0,
            power_zone: Rect::new(0, 0, 0, 0),
            settings_zone: Rect::new(0, 0, 0, 0),
            launcher_zone: Rect::new(0, 0, 0, 0),
            desktop_theme,
            dock_theme,
            symbols,
            datetime_zone: Rect::new(0, 0, 0, 0),
            net_zone: Rect::new(0, 0, 0, 0),
            notif_zone: Rect::new(0, 0, 0, 0),
            logout_zone: Rect::new(0, 0, 0, 0),
            current_workspace: 1,
            workspace_zone: Rect::new(0, 0, 0, 0),
            workspace_btn_zones: [Rect::new(0, 0, 0, 0); WS_INDICATOR_COUNT],
            show_datetime_tooltip: false,
            show_calendar_popover: false,
            cal_popup_open_btn: Rect::new(0, 0, 0, 0),
            cal_view_month: 1,
            cal_view_year: 1970,
            cal_selected_day: 1,
            cal_event_days: [false; CAL_POPUP_DAYS],
            cal_selected_events: Vec::new(),
            cal_selected_tasks: Vec::new(),
            cal_selected_reminders: Vec::new(),
            cal_last_loaded_key: [0; 10],
            cal_last_loaded_key_len: 0,
            show_notif_panel: false,
            notif_dnd_toggle_r: Rect::new(0, 0, 0, 0),
            notif_mark_seen_r: Rect::new(0, 0, 0, 0),
            notif_dismiss_r: Rect::new(0, 0, 0, 0),
            notif_dismiss_zones: Vec::new(),
            show_logout_confirm: false,
            logout_cancel_r: Rect::new(0, 0, 0, 0),
            logout_confirm_r: Rect::new(0, 0, 0, 0),
            apps: [
                DockAppState::new(AppId::Terminal, "Sunlight Terminal", AppId::Terminal),
                DockAppState::new(AppId::Chronos, "Chronos", AppId::Chronos),
                DockAppState::new(AppId::Calculator, "Sunlight Calculator", AppId::Calculator),
                DockAppState::new(AppId::Files, "Sunlight Files", AppId::Files),
                DockAppState::new(AppId::Settings, "System Preferences", AppId::Settings),
                // Not shown in the bottom dock — launched/tracked only from the Start Menu.
                DockAppState::new(AppId::Tasks, "Task Manager", AppId::Tasks),
                DockAppState::new(AppId::Bench, "Sunlight Bench", AppId::Bench),
                DockAppState::new(AppId::Eyes, "Eyes", AppId::Eyes),
                DockAppState::new(AppId::TextEditor, "Sunlight Edit", AppId::TextEditor),
                DockAppState::new(AppId::Writer, "Sunlight Writer", AppId::Writer),
                DockAppState::new(AppId::Calendar, "Sunlight Calendar", AppId::Calendar),
                DockAppState::new(AppId::RappidRabbit, "Rappid Rabbit", AppId::RappidRabbit),
                DockAppState::new(AppId::ApiLab, "Sunlight API Lab", AppId::ApiLab),
            ],
            running_apps: Vec::new(),
            icon_overrides,
            window_snapshots: Vec::new(),
            running_zones: Vec::new(),
            running_hover: None,
            running_hover_since: None,
            telemetry,
            next_app_poll_ms: 0,
            next_launch_id: 1,
            start_menu: start_menu::StartMenuState::new(),
            suppress_launcher_open: false,
            recent_apps: Vec::new(),
        };
        shell.reload_desktop_icons();
        shell
    }

    fn maybe_reload_wallpaper(&mut self, now: u64, width: u32, height: u32) {
        let res_changed = self.screen_w != width || self.screen_h != height;
        if !res_changed && now.saturating_sub(self.wallpaper_last_reload_ms) < 1000 {
            return;
        }
        let next_overrides = load_desktop_icon_overrides();
        if next_overrides != self.icon_overrides {
            self.icon_overrides = next_overrides;
            for entry in &mut self.running_apps {
                entry.icon = None;
            }
        }
        let next = load_desktop_config();
        if next == self.wallpaper_config {
            self.wallpaper_last_reload_ms = now;
            return;
        }
        let (wallpaper, wallpaper_error) = load_wallpaper_from_config(&next);
        self.wallpaper = wallpaper;
        self.wallpaper_error = wallpaper_error;
        self.wallpaper_config = next;
        self.wallpaper_last_reload_ms = now;
    }

    fn refresh_status(&mut self) -> bool {
        let mut dirty = false;

        let mut tmp_tz = [0u8; 48];
        let mut tmp_tz_l = 0usize;
        if let Some((y, mon, d, h, mi, s)) = query_local_full(&mut tmp_tz, &mut tmp_tz_l) {
            if mi != self.status_min
                || d != self.status_day
                || mon != self.status_month
                || y != self.status_year
                || self.status_min == 0xff
            {
                self.status_year = y;
                self.status_month = mon;
                self.status_day = d;
                self.status_hour = h;
                self.status_min = mi;
                self.status_sec = s;
                if self.show_calendar_popover && (1..=12).contains(&mon) && y >= 1970 {
                    self.cal_view_month = mon;
                    self.cal_view_year = y;
                }
                if tmp_tz_l > 0 && tmp_tz_l <= 48 {
                    self.tz_id[..tmp_tz_l].copy_from_slice(&tmp_tz[..tmp_tz_l]);
                    self.tz_id_len = tmp_tz_l;
                }
                dirty = true;
            }
        }

        if let Some(net_up) = query_net_up() {
            if net_up != self.status_net_up {
                self.status_net_up = net_up;
                dirty = true;
            }
        }

        // Load locale infrequently (file read is cheap here; 1/min ok)
        if self.status_min % 5 == 0 || self.locale_len == 0 {
            if let Some(loc) = read_locale_effective() {
                let b = loc.as_bytes();
                let n = b.len().min(47);
                self.locale[..n].copy_from_slice(&b[..n]);
                self.locale[n] = 0;
                self.locale_len = n;
                dirty = true;
            }
        }

        dirty
    }

    fn reset_calendar_view_to_today(&mut self) {
        self.cal_view_month = if (1..=12).contains(&self.status_month) {
            self.status_month
        } else {
            1
        };
        self.cal_view_year = if self.status_year >= 1970 {
            self.status_year
        } else {
            1970
        };
        self.cal_selected_day = if self.status_day >= 1
            && self.status_day <= cal_days_in_month(self.cal_view_year, self.cal_view_month)
        {
            self.status_day
        } else {
            1
        };
        self.cal_last_loaded_key_len = 0;
        self.refresh_calendar_popover_data();
    }

    fn refresh_calendar_popover_data(&mut self) {
        let key = format_cal_date(
            self.cal_view_year,
            self.cal_view_month,
            self.cal_selected_day,
        );
        let kb = key.as_bytes();
        if kb.len() == self.cal_last_loaded_key_len
            && self.cal_last_loaded_key[..self.cal_last_loaded_key_len] == *kb
        {
            return;
        }
        self.cal_event_days = [false; CAL_POPUP_DAYS];
        let offset = cal_weekday_sun0(self.cal_view_year, self.cal_view_month, 1);
        let dim = cal_days_in_month(self.cal_view_year, self.cal_view_month);
        for day in 1..=dim {
            let idx = offset + (day as usize - 1);
            if idx < CAL_POPUP_DAYS {
                self.cal_event_days[idx] =
                    calendar_day_has_items(self.cal_view_year, self.cal_view_month, day);
            }
        }
        self.cal_selected_events = load_calendar_events_for_day(
            self.cal_view_year,
            self.cal_view_month,
            self.cal_selected_day,
        );
        let (tasks, reminders) = load_tasks_and_reminders_for_day(
            self.cal_view_year,
            self.cal_view_month,
            self.cal_selected_day,
        );
        self.cal_selected_tasks = tasks;
        self.cal_selected_reminders = reminders;
        self.cal_last_loaded_key_len = kb.len().min(self.cal_last_loaded_key.len());
        self.cal_last_loaded_key[..self.cal_last_loaded_key_len]
            .copy_from_slice(&kb[..self.cal_last_loaded_key_len]);
    }

    fn reload_desktop_icons(&mut self) {
        self.desktop_icons = load_desktop_icons(&self.desktop_paths);
        self.selected_icons
            .retain(|idx| *idx < self.desktop_icons.len());
        self.last_desktop_click_idx = None;
        self.last_desktop_click_at = 0;
        if self.selected_icons.is_empty() {
            self.selection_state = DesktopSelectState::Idle;
        }
    }

    fn app(&self, app_id: AppId) -> &DockAppState {
        self.apps
            .iter()
            .find(|app| app.app_id == app_id)
            .expect("app registry entry missing")
    }

    fn app_mut(&mut self, app_id: AppId) -> &mut DockAppState {
        self.apps
            .iter_mut()
            .find(|app| app.app_id == app_id)
            .expect("app registry entry missing")
    }

    fn app_state_name(state: AppLaunchState) -> &'static str {
        match state {
            AppLaunchState::NotRunning => "NotRunning",
            AppLaunchState::Launching => "Launching",
            AppLaunchState::Running => "Running",
            AppLaunchState::Minimized => "Minimized",
            AppLaunchState::Closing => "Closing",
            AppLaunchState::Failed => "Failed",
        }
    }

    fn app_launch_path(app_id: AppId) -> &'static str {
        match app_id {
            AppId::Terminal => "/bin/sunlight-terminal",
            AppId::Chronos => "/bin/sunlight-chronos",
            AppId::Calculator => "/bin/calculator",
            AppId::Files => "/bin/sunlight-files",
            AppId::Settings => "/bin/control-panel",
            AppId::Tasks => "/bin/sunlight-tasks",
            AppId::Bench => "/bin/sunbench",
            AppId::Eyes => "/bin/eyes",
            AppId::TextEditor => "/bin/sunlight-edit",
            AppId::Writer => "/bin/sunlight-writer",
            AppId::Calendar => "/bin/sunlight-calendar",
            AppId::RappidRabbit => "/bin/rappid-rabbit",
            AppId::ApiLab => "/bin/sunlight-api-lab",
        }
    }

    fn app_launch_command(app_id: AppId) -> &'static [u8] {
        match app_id {
            AppId::Terminal => b"terminal",
            AppId::Chronos => b"chronos",
            AppId::Calculator => b"calculator",
            AppId::Files => b"files",
            AppId::Settings => b"settings",
            AppId::Tasks => b"tasks",
            AppId::Bench => b"bench",
            AppId::Eyes => b"eyes",
            AppId::TextEditor => b"sunlight-edit",
            AppId::Writer => b"sunlight-writer",
            AppId::Calendar => b"calendar",
            AppId::RappidRabbit => b"rappid-rabbit",
            AppId::ApiLab => b"sunlight-api-lab",
        }
    }

    fn app_trace_subject(app_id: AppId) -> &'static str {
        match app_id {
            AppId::Terminal => "app=terminal",
            AppId::Chronos => "app=chronos",
            AppId::Calculator => "app=calculator",
            AppId::Files => "app=files",
            AppId::Settings => "app=control-panel",
            AppId::Tasks => "app=tasks",
            AppId::Bench => "app=bench",
            AppId::Eyes => "app=eyes",
            AppId::TextEditor => "app=sunlight-edit",
            AppId::Writer => "app=sunlight-writer",
            AppId::Calendar => "app=sunlight-calendar",
            AppId::RappidRabbit => "app=rappid-rabbit",
            AppId::ApiLab => "app=sunlight-api-lab",
        }
    }

    fn app_allows_multiple_instances(app_id: AppId) -> bool {
        matches!(app_id, AppId::Calendar | AppId::Chronos)
    }

    fn open_file_via_resolver(&mut self, path: &str, source: LaunchSource) -> bool {
        let trace = self.next_launch_trace(source);
        match sun_open::open_path(trace, source, path.as_bytes()) {
            Ok(result) => {
                debug_log("[VORTEX] open_path ok path=\"");
                debug_log(path);
                debug_log("\" pid=");
                debug_log_u32(result.pid as u32);
                debug_log("\n");
                true
            }
            Err(err) => {
                let (title, body) = open_error_notification(err, path);
                debug_log("[VORTEX] open_path failed path=\"");
                debug_log(path);
                debug_log("\" error=");
                debug_log(body);
                debug_log("\n");
                let _ = show_notification(NotificationKind::Error, title, body, 5000);
                false
            }
        }
    }

    fn select_only_desktop_icon(&mut self, idx: usize) {
        self.selected_icons.clear();
        self.selected_icons.push(idx);
    }

    fn clear_desktop_selection(&mut self) {
        self.selected_icons.clear();
        self.last_desktop_click_idx = None;
        self.last_desktop_click_at = 0;
    }

    fn select_desktop_icons_in_rect(&mut self, rect: Rect) {
        self.selected_icons.clear();
        for (idx, icon) in self.desktop_icons.iter().enumerate() {
            if icon.rect.w == 0 || icon.rect.h == 0 {
                continue;
            }
            if icon.rect.intersect(rect).is_some() {
                self.selected_icons.push(idx);
            }
        }
    }

    fn desktop_selection_rect(&self) -> Option<Rect> {
        let (anchor, current) = match self.selection_state {
            DesktopSelectState::Dragging { anchor, current } => (anchor, current),
            _ => return None,
        };
        let x = anchor.x.min(current.x);
        let y = anchor.y.min(current.y);
        let w = (anchor.x - current.x).unsigned_abs().max(1);
        let h = (anchor.y - current.y).unsigned_abs().max(1);
        Some(Rect::new(x, y, w, h))
    }

    fn launch_desktop_icon(&mut self, idx: usize, now: u64) -> bool {
        let (kind, path) = {
            let Some(icon) = self.desktop_icons.get(idx) else {
                return false;
            };
            (icon.kind, icon._action.clone())
        };
        self.select_only_desktop_icon(idx);
        match kind {
            DesktopIconKind::File | DesktopIconKind::Image | DesktopIconKind::DesktopEntry => {
                self.open_file_via_resolver(&path, LaunchSource::Shortcut)
            }
            DesktopIconKind::Network => {
                self.handle_app_click(AppId::Settings, now, LaunchSource::Shortcut)
            }
            DesktopIconKind::Computer
            | DesktopIconKind::Home
            | DesktopIconKind::Trash
            | DesktopIconKind::Drive
            | DesktopIconKind::Folder => {
                self.handle_app_click(AppId::Files, now, LaunchSource::Shortcut)
            }
        }
    }

    fn handle_desktop_icon_click(&mut self, idx: usize, now: u64) -> bool {
        let is_double_click = self.last_desktop_click_idx == Some(idx)
            && now.saturating_sub(self.last_desktop_click_at) <= DESKTOP_DOUBLE_CLICK_MS;
        self.select_only_desktop_icon(idx);
        self.last_desktop_click_idx = Some(idx);
        self.last_desktop_click_at = now;
        if is_double_click {
            let launched = self.launch_desktop_icon(idx, now);
            self.last_desktop_click_idx = None;
            self.last_desktop_click_at = 0;
            launched
        } else {
            true
        }
    }

    fn arm_desktop_marquee(&mut self, anchor: Point) {
        self.selection_state = DesktopSelectState::Armed { anchor };
        self.suppress_next_click = false;
        self.clear_desktop_selection();
    }

    fn update_desktop_marquee(&mut self, point: Point) {
        const DRAG_THRESHOLD: i32 = 4;
        match self.selection_state {
            DesktopSelectState::Armed { anchor } => {
                let dx = (point.x - anchor.x).abs();
                let dy = (point.y - anchor.y).abs();
                if dx >= DRAG_THRESHOLD || dy >= DRAG_THRESHOLD {
                    self.selection_state = DesktopSelectState::Dragging {
                        anchor,
                        current: point,
                    };
                    if let Some(rect) = self.desktop_selection_rect() {
                        self.select_desktop_icons_in_rect(rect);
                    }
                }
            }
            DesktopSelectState::Dragging { anchor, .. } => {
                self.selection_state = DesktopSelectState::Dragging {
                    anchor,
                    current: point,
                };
                if let Some(rect) = self.desktop_selection_rect() {
                    self.select_desktop_icons_in_rect(rect);
                }
            }
            DesktopSelectState::Idle => {}
        }
    }

    fn end_selection_gesture(&mut self) {
        self.selection_state = DesktopSelectState::Idle;
    }

    fn next_launch_trace(&mut self, source: LaunchSource) -> LaunchTrace {
        let trace = LaunchTrace::new(self.next_launch_id, source, monotonic_millis());
        self.next_launch_id = self.next_launch_id.saturating_add(1);
        trace
    }

    fn register_launch_trace(&self, trace: LaunchTrace, pid: u64, app_id: AppId) {
        let reply = ipc_call_timeout(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::LAUNCH_TRACE)
                .word(0, trace.launch_id)
                .word(1, trace.source as u64)
                .word(2, pid)
                .word(3, trace.requested_at_ms),
            DISPLAY_IPC_TIMEOUT_MS,
        );
        if reply.is_err() {
            debug_log("[VORTEX] launch_trace_register_failed(");
            debug_log(Self::app_trace_subject(app_id));
            debug_log(", pid=");
            debug_log_u32(pid as u32);
            debug_log(")\n");
        }
    }

    fn log_launch_trace(
        trace: LaunchTrace,
        app_id: AppId,
        phase: &str,
        pid: Option<u64>,
        now: u64,
    ) {
        launch_trace::log_phase(trace, Self::app_trace_subject(app_id), phase, pid, now);
    }

    fn dock_zone_app(slot: usize) -> DockZone {
        match slot {
            0 => DockZone::App(AppId::Terminal),
            1 => DockZone::App(AppId::Calendar),
            2 => DockZone::App(AppId::Calculator),
            3 => DockZone::App(AppId::Files),
            _ => DockZone::Placeholder,
        }
    }

    /// True only for pinned apps that already have a dedicated dock icon
    /// (Terminal, Calendar, Calculator, Files). These are excluded from the
    /// dynamic running-apps strip to avoid showing the same app twice.
    /// Start-Menu-only pinned apps (Settings, Tasks, Bench, Eyes, TextEditor,
    /// Writer)
    /// have no dock icon, so they *are* shown in the running strip when their
    /// window is open — otherwise they'd be invisible in the taskbar.
    fn app_pid_has_dock_icon(&self, pid: u64) -> bool {
        self.apps.iter().any(|app| {
            app.pid == Some(pid)
                && matches!(
                    app.app_id,
                    AppId::Terminal | AppId::Calendar | AppId::Calculator | AppId::Files
                )
        })
    }

    fn process_name_hint<'a>(
        window: &WindowSnapshot,
        telem: Option<&'a SystemSnapshot>,
    ) -> Option<&'a str> {
        telem.and_then(|snap| Self::process_name_for_pid(window.owner_pid, snap))
    }

    fn process_name_for_pid<'a>(pid: u64, snap: &'a SystemSnapshot) -> Option<&'a str> {
        snap.procs
            .iter()
            .take(snap.proc_count)
            .find(|proc| proc.pid as u64 == pid)
            .map(|proc| proc.name_str())
    }

    fn running_display_name<'a>(
        &self,
        window: &WindowSnapshot,
        telem: Option<&SystemSnapshot>,
        buf: &'a mut [u8; RUNNING_NAME_BUF],
    ) -> &'a str {
        let title_len = copy_sanitized_ascii(&window.title, buf);
        if title_len > 0 {
            return core::str::from_utf8(&buf[..title_len]).unwrap_or("App");
        }

        if let Some(name) = Self::process_name_hint(window, telem) {
            let name_len = copy_sanitized_ascii(name.as_bytes(), buf);
            if name_len > 0 {
                return core::str::from_utf8(&buf[..name_len]).unwrap_or("App");
            }
        }

        let len = write_fallback_app_name(window.owner_pid, buf);
        core::str::from_utf8(&buf[..len]).unwrap_or("App")
    }

    fn is_rtl_locale(&self) -> bool {
        let locale = core::str::from_utf8(&self.locale[..self.locale_len]).unwrap_or("");
        locale.starts_with("fa") || locale.starts_with("ar") || locale.starts_with("he")
    }

    fn log_app_click(&self, app_id: AppId) {
        let app = self.app(app_id);
        debug_log("[VORTEX] dock_app_click(");
        debug_log(app.display_name);
        debug_log(", ");
        debug_log(Self::app_state_name(app.state));
        if app.state == AppLaunchState::Failed {
            let error = app.error_str();
            if !error.is_empty() {
                debug_log(", ");
                debug_log(error);
            }
        }
        debug_log(", attempts=");
        debug_log_u32(app.launch_attempts);
        debug_log(", blocked=");
        debug_log_u32(app.duplicate_blocks);
        debug_log(")\n");
    }

    fn refresh_window_snapshots(&mut self) -> Option<()> {
        self.window_snapshots.clear();
        let mut idx = 0u64;
        loop {
            let reply = ipc_call_timeout(
                self.display_ep,
                IpcMsg::with_label(SgpMsg::LIST_WINDOWS).word(0, idx),
                DISPLAY_IPC_TIMEOUT_MS,
            )
            .ok()?;
            if reply.label != SgpMsg::REPLY {
                return None;
            }
            if reply.words[0] == 0 {
                break;
            }
            let metadata = reply.words[3];
            // The display server packs the active workspace id into bits 16..23
            // of words[3]. Mirror it so the shell's indicator + taskbar filter
            // follow both Super+1..4 (handled in the compositor) and the
            // clickable indicator (which sends SET_WORKSPACE).
            let active_ws = ((metadata >> 16) & 0xFF) as u8;
            if (1..=4).contains(&active_ws) {
                self.current_workspace = active_ws;
            }
            let mut title = [0u8; 16];
            for i in 0..8usize {
                title[i] = ((reply.words[6] >> (i * 8)) & 0xFF) as u8;
                title[8 + i] = ((reply.words[7] >> (i * 8)) & 0xFF) as u8;
            }
            self.window_snapshots.push(WindowSnapshot {
                id: reply.words[0],
                owner_pid: reply.words[1],
                state: reply.words[2],
                window_type: metadata & 0xFF,
                workspace_id: reply.words[4],
                hidden: reply.words[5] != 0,
                rolled_up: ((metadata >> 8) & 1) != 0,
                title,
            });
            idx = idx.saturating_add(1);
        }
        Some(())
    }

    fn sync_app_registry(&mut self, now: u64, force: bool) -> bool {
        if !force && now < self.next_app_poll_ms {
            return false;
        }
        self.next_app_poll_ms = now.saturating_add(APP_STATE_POLL_MS);

        let prev_ws = self.current_workspace;
        let Some(()) = self.refresh_window_snapshots() else {
            return false;
        };
        let mut windows = core::mem::take(&mut self.window_snapshots);

        // A workspace switch (via Super+1..4 in the compositor) changes no app
        // state, so it wouldn't otherwise mark dirty — but the indicator + taskbar
        // filter must redraw to reflect the new active workspace.
        let mut dirty = self.current_workspace != prev_ws;
        for app in &mut self.apps {
            let prev_state = app.state;
            let prev_window = app.main_window_id;
            let prev_pid = app.pid;
            let maybe_win = app
                .pid
                .and_then(|pid| windows.iter().find(|win| win.owner_pid == pid).copied());

            if let Some(win) = maybe_win {
                app.pid = Some(win.owner_pid);
                app.main_window_id = Some(win.id);
                app.clear_error();
                let minimized = (win.state & 0x3) == 1;
                app.state = if minimized {
                    AppLaunchState::Minimized
                } else {
                    AppLaunchState::Running
                };
                if prev_state == AppLaunchState::Launching {
                    debug_log("[VORTEX] app_launch_window_attached(");
                    debug_log(app.display_name);
                    debug_log(", ");
                    debug_log_u32(win.id as u32);
                    debug_log(")\n");
                    Self::log_launch_trace(
                        LaunchTrace::new(
                            app.last_launch_id,
                            app.last_launch_source,
                            app.last_launch_started_at,
                        ),
                        app.app_id,
                        "dock_state_running",
                        Some(win.owner_pid),
                        now,
                    );
                } else if prev_state == AppLaunchState::Minimized
                    && app.state == AppLaunchState::Running
                {
                    debug_log("[VORTEX] app_restore_minimized(");
                    debug_log(app.display_name);
                    debug_log(", ");
                    debug_log_u32(win.id as u32);
                    debug_log(")\n");
                }
                if app.state != prev_state || app.main_window_id != prev_window {
                    dirty = true;
                }
                continue;
            }

            match app.state {
                AppLaunchState::Launching => {
                    let alive = app.pid.map(process_is_alive).unwrap_or(false);
                    if now.saturating_sub(app.last_launch_started_at) >= APP_LAUNCH_TIMEOUT_MS {
                        app.state = AppLaunchState::Failed;
                        app.set_error("launch timed out");
                        debug_log("[VORTEX] app_launch_timeout(");
                        debug_log(app.display_name);
                        debug_log(")\n");
                        dirty = true;
                    } else if app.pid.is_some() && !alive {
                        app.state = AppLaunchState::Failed;
                        app.set_error("process exited before window");
                        debug_log("[VORTEX] app_launch_failed(");
                        debug_log(app.display_name);
                        debug_log(")\n");
                        dirty = true;
                    }
                }
                AppLaunchState::Running | AppLaunchState::Minimized | AppLaunchState::Closing => {
                    if let Some(pid) = prev_pid {
                        if !process_is_alive(pid) {
                            app.state = AppLaunchState::NotRunning;
                            app.pid = None;
                            app.main_window_id = None;
                            app.clear_error();
                            debug_log("[VORTEX] app_window_closed(");
                            debug_log(app.display_name);
                            debug_log(")\n");
                            dirty = true;
                        } else {
                            app.state = AppLaunchState::Closing;
                            app.main_window_id = None;
                            debug_log("[VORTEX] app_closing_awaiting_exit(");
                            debug_log(app.display_name);
                            debug_log(")\n");
                            dirty = true;
                        }
                    } else {
                        app.state = AppLaunchState::NotRunning;
                        app.main_window_id = None;
                        dirty = true;
                    }
                }
                AppLaunchState::Failed => {
                    if let Some(pid) = prev_pid {
                        if !process_is_alive(pid) {
                            app.state = AppLaunchState::NotRunning;
                            app.pid = None;
                            app.main_window_id = None;
                            app.clear_error();
                            dirty = true;
                        }
                    }
                }
                AppLaunchState::NotRunning => {
                    if let Some(pid) = prev_pid {
                        if process_is_alive(pid) {
                            app.state = AppLaunchState::Failed;
                            app.set_error("window disappeared");
                            app.main_window_id = None;
                        } else {
                            app.pid = None;
                            app.main_window_id = None;
                            app.clear_error();
                        }
                        dirty = true;
                    } else if prev_window.is_some() {
                        app.main_window_id = None;
                        dirty = true;
                    }
                }
            }
        }

        if ENABLE_RUNNING_TASKBAR {
            let telem_snapshot = self.telemetry.as_mut().map(|telemetry| {
                let _ = telemetry.poll();
                *telemetry.snapshot()
            });
            if self.sync_running_apps(&windows, telem_snapshot.as_ref()) {
                dirty = true;
            }
        }

        windows.clear();
        self.window_snapshots = windows;

        dirty
    }

    fn sync_running_apps(
        &mut self,
        windows: &[WindowSnapshot],
        telem: Option<&SystemSnapshot>,
    ) -> bool {
        let mut dirty = false;
        let mut seen = [0u64; MAX_RUNNING_TRACKED];
        let mut seen_len = 0usize;

        for window in windows {
            if window.hidden
                || window.workspace_id != self.current_workspace as u64
                || window.window_type == 2
                || window.window_type == 3
            {
                continue;
            }
            if self.app_pid_has_dock_icon(window.owner_pid) {
                continue;
            }

            if seen_len < seen.len() {
                seen[seen_len] = window.id;
                seen_len += 1;
            }
            let minimized = (window.state & 0x3) == 1 || window.rolled_up;
            let mut name_buf = [0u8; RUNNING_NAME_BUF];
            let display_name = self.running_display_name(window, telem, &mut name_buf);
            let proc_name = Self::process_name_hint(window, telem);

            if let Some(entry) = self
                .running_apps
                .iter_mut()
                .find(|entry| entry.win_id == window.id)
            {
                let pid_changed = entry.pid != window.owner_pid;
                if pid_changed {
                    entry.pid = window.owner_pid;
                    dirty = true;
                }
                if entry.minimized != minimized {
                    entry.minimized = minimized;
                    dirty = true;
                }
                if entry.display_name.as_str() != display_name {
                    entry.display_name.clear();
                    entry.display_name.push_str(display_name);
                    dirty = true;
                }
                let next_icon_hint =
                    build_icon_resolution_key(proc_name, entry.display_name.as_str());
                if pid_changed || entry.icon_hint != next_icon_hint || entry.icon.is_none() {
                    entry.icon_hint = next_icon_hint;
                    entry.icon = resolve_running_icon(
                        proc_name,
                        entry.display_name.as_str(),
                        &self.icon_overrides,
                    );
                }
                continue;
            }

            if self.running_apps.len() >= MAX_RUNNING_TRACKED {
                continue;
            }

            let mut entry = RunningAppEntry {
                win_id: window.id,
                pid: window.owner_pid,
                display_name: String::from(display_name),
                cell_w: 0,
                minimized,
                icon_hint: build_icon_resolution_key(proc_name, display_name),
                icon: resolve_running_icon(proc_name, display_name, &self.icon_overrides),
                last_click_at: 0,
            };
            entry.refresh_cell_width();
            self.running_apps.push(entry);
            dirty = true;
        }

        let before = self.running_apps.len();
        self.running_apps.retain(|entry| {
            let mut i = 0usize;
            while i < seen_len {
                if seen[i] == entry.win_id {
                    return true;
                }
                i += 1;
            }
            false
        });
        if self.running_apps.len() != before {
            dirty = true;
        }

        if self
            .running_hover
            .map_or(false, |idx| idx >= self.running_apps.len())
        {
            self.running_hover = None;
        }
        dirty
    }

    fn launch_app(&mut self, app_id: AppId, now: u64, source: LaunchSource) -> bool {
        let trace = self.next_launch_trace(source);
        Self::log_launch_trace(trace, app_id, "launch_request_received", None, now);

        {
            let app = self.app_mut(app_id);
            app.last_click_at = now;
            app.launch_attempts = app.launch_attempts.saturating_add(1);
            app.last_launch_id = trace.launch_id;
            app.last_launch_source = source;
        }

        let mut clear_stale_pid = false;
        let duplicate_blocked = {
            let app = self.app(app_id);
            if Self::app_allows_multiple_instances(app_id) {
                false
            } else if let Some(pid) = app.pid {
                if process_is_alive(pid) {
                    true
                } else {
                    clear_stale_pid = true;
                    false
                }
            } else if matches!(
                app.state,
                AppLaunchState::Launching | AppLaunchState::Running | AppLaunchState::Minimized
            ) {
                true
            } else if app.state == AppLaunchState::Failed {
                app.pid.map(process_is_alive).unwrap_or(false)
            } else {
                false
            }
        };
        Self::log_launch_trace(trace, app_id, "duplicate_launch_check_done", None, now);
        if duplicate_blocked {
            let app = self.app_mut(app_id);
            app.duplicate_blocks = app.duplicate_blocks.saturating_add(1);
            debug_log("[VORTEX] app_launch_blocked_duplicate(");
            debug_log(app.display_name);
            debug_log(")\n");
            return false;
        }
        if clear_stale_pid {
            let app = self.app_mut(app_id);
            app.pid = None;
            app.main_window_id = None;
            if matches!(
                app.state,
                AppLaunchState::Running
                    | AppLaunchState::Minimized
                    | AppLaunchState::Launching
                    | AppLaunchState::Closing
            ) {
                app.state = AppLaunchState::NotRunning;
                app.clear_error();
            }
        }

        {
            let app = self.app_mut(app_id);
            app.state = AppLaunchState::Launching;
            app.main_window_id = None;
            app.last_launch_started_at = now;
            app.clear_error();
            Self::log_launch_trace(trace, app_id, "dock_state_launching", None, now);
        }

        Self::log_launch_trace(trace, app_id, "spawn_start", None, now);
        let words = [Self::app_launch_command(app_id)];
        match sun_exec::launch_from_words(trace, source, &words, true) {
            Ok(result) => {
                Self::log_launch_trace(
                    trace,
                    app_id,
                    "spawn_returned",
                    Some(result.pid),
                    monotonic_millis(),
                );
                self.register_launch_trace(trace, result.pid, app_id);
                let app = self.app_mut(app_id);
                app.pid = Some(result.pid);
                Self::log_launch_trace(
                    trace,
                    app_id,
                    "process_created",
                    Some(result.pid),
                    monotonic_millis(),
                );
                debug_log("[VORTEX] app_launch_started(");
                debug_log(app.display_name);
                debug_log(", pid=");
                debug_log_u32(result.pid as u32);
                debug_log(")\n");
                true
            }
            Err(err) => {
                Self::log_launch_trace(trace, app_id, "spawn_failed", None, monotonic_millis());
                let app = self.app_mut(app_id);
                app.state = AppLaunchState::Failed;
                app.set_error(launch_error_text(err));
                debug_log("[VORTEX] app_launch_failed(");
                debug_log(app.display_name);
                debug_log(")\n");
                let mut body = String::from("Could not start ");
                body.push_str(Self::app_launch_path(app_id));
                let _ = show_notification(NotificationKind::Error, "Launch failed", &body, 30_000);
                false
            }
        }
    }

    fn activate_app_window(&mut self, app_id: AppId, now: u64) -> bool {
        let window_id = {
            let app = self.app(app_id);
            app.main_window_id
        };
        let Some(win_id) = window_id else {
            return false;
        };

        let reply = ipc_call_timeout(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::ACTIVATE_WINDOW).word(0, win_id),
            DISPLAY_IPC_TIMEOUT_MS,
        );
        if reply.is_err() {
            return false;
        }

        let app = self.app_mut(app_id);
        if app.state == AppLaunchState::Minimized {
            debug_log("[VORTEX] app_restore_minimized(");
            debug_log(app.display_name);
            debug_log(", ");
            debug_log_u32(win_id as u32);
            debug_log(")\n");
        } else {
            debug_log("[VORTEX] app_focus_existing(");
            debug_log(app.display_name);
            debug_log(", ");
            debug_log_u32(win_id as u32);
            debug_log(")\n");
        }
        app.state = AppLaunchState::Running;
        app.last_click_at = now;
        true
    }

    fn activate_running_window(&mut self, win_id: u64, now: u64) -> bool {
        let reply = ipc_call_timeout(
            self.display_ep,
            IpcMsg::with_label(SgpMsg::ACTIVATE_WINDOW).word(0, win_id),
            DISPLAY_IPC_TIMEOUT_MS,
        );
        if reply.is_err() {
            return false;
        }

        if let Some(entry) = self
            .running_apps
            .iter_mut()
            .find(|entry| entry.win_id == win_id)
        {
            entry.minimized = false;
            entry.last_click_at = now;
        }
        true
    }

    fn handle_app_click(&mut self, app_id: AppId, now: u64, source: LaunchSource) -> bool {
        self.sync_app_registry(now, true);
        self.log_app_click(app_id);
        let state = self.app(app_id).state;
        if Self::app_allows_multiple_instances(app_id) {
            return self.launch_app(app_id, now, source);
        }
        match state {
            AppLaunchState::NotRunning | AppLaunchState::Failed | AppLaunchState::Closing => {
                self.launch_app(app_id, now, source)
            }
            AppLaunchState::Launching => {
                let app = self.app_mut(app_id);
                app.duplicate_blocks = app.duplicate_blocks.saturating_add(1);
                debug_log("[VORTEX] app_launch_blocked_duplicate(");
                debug_log(app.display_name);
                debug_log(")\n");
                false
            }
            AppLaunchState::Running | AppLaunchState::Minimized => {
                self.activate_app_window(app_id, now)
            }
        }
    }

    /// Record `app_id` as the most-recently-used app for the Start Menu's
    /// "Recent" section (session-only, not persisted — see
    /// `docs/GUI/START_MENU.md`).
    fn note_recent_app(&mut self, app_id: AppId) {
        self.recent_apps.retain(|id| *id != app_id);
        self.recent_apps.insert(0, app_id);
        self.recent_apps.truncate(MAX_RECENT_APPS);
    }

    /// Interpret what the Start Menu asked for. This is the *only* place
    /// that turns a Start Menu action into IPC/launch/power side effects —
    /// `start_menu.rs` itself stays UI-only.
    fn apply_start_menu_action(&mut self, action: start_menu::StartMenuAction, now: u64) {
        use start_menu::{PowerAction, StartMenuAction};
        match action {
            StartMenuAction::None | StartMenuAction::DismissedOutside { .. } => {}
            StartMenuAction::Launch(app_id) => {
                self.note_recent_app(app_id);
                let _ = self.handle_app_click(app_id, now, LaunchSource::Shell);
            }
            StartMenuAction::Unavailable(name) => {
                let mut body = String::new();
                body.push_str(name);
                body.push_str(" isn't available yet.");
                show_notification(NotificationKind::Info, "Coming soon", &body, 3500);
            }
            StartMenuAction::Power(PowerAction::Sleep) => {
                show_notification(
                    NotificationKind::Info,
                    "Sleep",
                    "Sleep isn't supported on this hardware yet.",
                    3500,
                );
            }
            StartMenuAction::Power(PowerAction::Restart) => {
                debug_log("[VORTEX] start menu: reboot requested\n");
                libc::power::reboot();
            }
            StartMenuAction::Power(PowerAction::Shutdown) => {
                debug_log("[VORTEX] start menu: shutdown requested\n");
                libc::power::shutdown();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing helpers
// ---------------------------------------------------------------------------

/// Draw a 16×16 pixel-art icon scaled 1:1 centred inside `cell`.
fn draw_icon16(canvas: &mut Canvas, cell: Rect, rows: &[u16; 16], color: Color) {
    let ox = cell.x + (cell.w as i32 - ICON16_W as i32) / 2;
    let oy = cell.y + (cell.h as i32 - 16i32) / 2;
    for (row_idx, &row_bits) in rows.iter().enumerate() {
        for col in 0..ICON16_W as usize {
            let bit = (row_bits >> (ICON16_W as usize - 1 - col)) & 1;
            if bit != 0 {
                canvas.put_pixel(ox + col as i32, oy + row_idx as i32, color);
            }
        }
    }
}

/// Draw a 16×16 pixel-art icon scaled up inside `cell`.
fn draw_icon16_scaled(canvas: &mut Canvas, cell: Rect, rows: &[u16; 16], color: Color, scale: u32) {
    let icon_w = ICON16_W * scale;
    let icon_h = 16 * scale;
    let ox = cell.x + (cell.w as i32 - icon_w as i32) / 2;
    let oy = cell.y + (cell.h as i32 - icon_h as i32) / 2;
    for (row_idx, &row_bits) in rows.iter().enumerate() {
        for col in 0..ICON16_W as usize {
            let bit = (row_bits >> (ICON16_W as usize - 1 - col)) & 1;
            if bit != 0 {
                canvas.fill_rect(
                    Rect::new(
                        ox + col as i32 * scale as i32,
                        oy + row_idx as i32 * scale as i32,
                        scale,
                        scale,
                    ),
                    color,
                );
            }
        }
    }
}

/// Draw a panel pill: filled rounded rect with a 1-px border.
fn draw_panel(canvas: &mut Canvas, rect: Rect, fill: Color, border: Color) {
    canvas.fill_rounded_rect(rect, RADIUS, fill);
    canvas.stroke_rounded_rect(rect, RADIUS, 1, border);
}

/// Draw an icon button cell. `highlight` draws it with the accent tint.
fn draw_icon_btn(
    canvas: &mut Canvas,
    cell: Rect,
    rows: &[u16; 16],
    theme: &Theme,
    highlight: bool,
    hover: bool,
) {
    if hover {
        canvas.fill_rounded_rect(cell, 5, theme.panel_alt);
    }
    let icon_color = if highlight {
        theme.accent
    } else if hover {
        theme.text
    } else {
        theme.text_dim
    };
    draw_icon16(canvas, cell, rows, icon_color);
}

fn draw_app_button(
    canvas: &mut Canvas,
    cell: Rect,
    theme: &Theme,
    dock: &DockTheme,
    rows: &[u16; 16],
    app: &DockAppState,
    hovered: bool,
    now: u64,
) {
    let pressed = now.saturating_sub(app.last_click_at) < APP_PRESS_MS;
    let pulse = ((now / 220) & 1) == 0;

    let mut fill = theme.panel;
    let mut border = theme.border;
    let mut icon_color = theme.text_dim;
    let mut bottom_marker = false;

    match app.state {
        AppLaunchState::NotRunning => {
            if hovered {
                fill = theme.accent.darken(78);
                border = theme.accent_hover.darken(24);
                icon_color = theme.text;
            }
        }
        AppLaunchState::Launching => {
            fill = if pulse {
                theme.accent.darken(56)
            } else {
                theme.panel
            };
            border = theme.accent;
            icon_color = if pulse {
                theme.accent_hover
            } else {
                theme.text
            };
        }
        AppLaunchState::Running => {
            fill = theme.accent.darken(68);
            border = theme.accent;
            icon_color = theme.text;
            if hovered {
                fill = theme.accent.darken(58);
            }
        }
        AppLaunchState::Minimized => {
            fill = theme.panel;
            border = theme.border;
            icon_color = theme.text;
            bottom_marker = true;
            if hovered {
                fill = theme.panel_alt;
            }
        }
        AppLaunchState::Closing => {
            fill = if pulse { theme.panel_alt } else { theme.panel };
            border = theme.border;
            icon_color = if pulse { theme.text_dim } else { theme.text };
            if hovered {
                fill = theme.accent.darken(78);
                border = theme.accent_hover.darken(24);
                icon_color = theme.text;
            }
        }
        AppLaunchState::Failed => {
            fill = theme.panel_alt;
            border = theme.warn;
            icon_color = theme.warn;
            if hovered {
                fill = theme.warn.darken(72);
            }
        }
    }

    if pressed {
        fill = theme.accent_hover.darken(35);
        border = theme.accent_hover;
        icon_color = theme.text;
    }

    canvas.fill_rounded_rect(cell, 5, fill);
    canvas.stroke_rounded_rect(cell, 5, 1, border);
    if bottom_marker {
        draw_taskbar_dot(canvas, cell, theme.accent);
    }

    if let Some(tga) = dock.icon_for_app(app.icon) {
        canvas.draw_tga_icon(&tga, cell.inset(2));
    } else {
        draw_icon16(canvas, cell, rows, icon_color);
    }
}

/// Tint a white+alpha TGA glyph to the given color using its alpha mask.
/// Used for Material Symbols so they pick up orange accent at small panel size.
fn draw_tga_tinted_orange(canvas: &mut Canvas, img: &TgaImage, dst: Rect, tint: Color) {
    if img.width == 0 || img.height == 0 {
        return;
    }
    let cx0 = dst.x.max(0) as u32;
    let cy0 = dst.y.max(0) as u32;
    let cx1 = (dst.right() as u32).min(canvas.width);
    let cy1 = (dst.bottom() as u32).min(canvas.height);
    if cx0 >= cx1 || cy0 >= cy1 {
        return;
    }
    let dw = (dst.right() - dst.x.max(0)).max(1) as u32;
    let dh = (dst.bottom() - dst.y.max(0)).max(1) as u32;
    let tr = ((tint.0 >> 16) & 0xFF) as u32;
    let tg = ((tint.0 >> 8) & 0xFF) as u32;
    let tb = (tint.0 & 0xFF) as u32;

    for dy in cy0..cy1 {
        let src_y = (dy - cy0) * img.height / dh;
        for dx in cx0..cx1 {
            let src_x = (dx - cx0) * img.width / dw;
            let argb = img.pixel_argb(src_x, src_y);
            let a = (argb >> 24) as u32;
            if a == 0 {
                continue;
            }
            // Blend tint over dst using src alpha
            let ia = 255 - a;
            // Read dst? Canvas doesn't expose easy read for all; approximate by direct write scaled
            // For small icons we just write the tinted value (overwrites, good for panel bg)
            let r = (tr * a + 0 * ia) / 255; // assume dark panel; simple solid tint
            let g = (tg * a + 0 * ia) / 255;
            let b = (tb * a + 0 * ia) / 255;
            canvas.put_pixel(dx as i32, dy as i32, Color((r << 16) | (g << 8) | b));
        }
    }
}

fn draw_tga_bytes(canvas: &mut Canvas, bytes: &[u8], dst: Rect) {
    if bytes.len() < 18 {
        return;
    }
    if bytes[2] != 2 {
        return;
    }
    let bpp = bytes[16];
    if bpp != 24 && bpp != 32 {
        return;
    }
    let width = u16::from_le_bytes([bytes[12], bytes[13]]) as u32;
    let height = u16::from_le_bytes([bytes[14], bytes[15]]) as u32;
    if width == 0 || height == 0 {
        return;
    }
    let id_len = bytes[0] as usize;
    let cm_len = u16::from_le_bytes([bytes[5], bytes[6]]) as u32;
    let cm_entry_bits = bytes[7] as u32;
    let cm_bytes = if bytes[1] != 0 {
        cm_len * ((cm_entry_bits + 7) / 8)
    } else {
        0
    };
    let data_offset = 18usize
        .saturating_add(id_len)
        .saturating_add(cm_bytes as usize);
    if data_offset >= bytes.len() {
        return;
    }
    let top_down = (bytes[17] & 0x20) != 0;

    let cx0 = dst.x.max(0) as u32;
    let cy0 = dst.y.max(0) as u32;
    let cx1 = (dst.right() as u32).min(canvas.width);
    let cy1 = (dst.bottom() as u32).min(canvas.height);
    if cx0 >= cx1 || cy0 >= cy1 {
        return;
    }
    let dw = (dst.right() - dst.x.max(0)).max(1) as u32;
    let dh = (dst.bottom() - dst.y.max(0)).max(1) as u32;
    let bpp_bytes = (bpp / 8) as u32;
    let row_stride = width.saturating_mul(bpp_bytes);

    for dy in cy0..cy1 {
        let src_y = (dy - cy0) * height / dh;
        let file_row = if top_down {
            src_y
        } else {
            height.saturating_sub(1).saturating_sub(src_y)
        };
        let row_start = data_offset + file_row as usize * row_stride as usize;
        let out_row = dy as usize * canvas.stride as usize;
        for dx in cx0..cx1 {
            let src_x = (dx - cx0) * width / dw;
            let idx = row_start + src_x as usize * bpp_bytes as usize;
            if idx + 2 >= bytes.len() {
                continue;
            }
            let b = bytes[idx] as u32;
            let g = bytes[idx + 1] as u32;
            let r = bytes[idx + 2] as u32;
            let a = if bpp == 32 && idx + 3 < bytes.len() {
                bytes[idx + 3] as u32
            } else {
                0xFF
            };
            if a == 0 {
                continue;
            }
            let px = out_row + dx as usize;
            if px >= canvas.pixels.len() {
                continue;
            }
            if a == 0xFF {
                canvas.pixels[px] = (r << 16) | (g << 8) | b;
            } else {
                let dst_px = canvas.pixels[px];
                let dr = (dst_px >> 16) & 0xFF;
                let dg = (dst_px >> 8) & 0xFF;
                let db = dst_px & 0xFF;
                let ia = 255 - a;
                let nr = (r * a + dr * ia) >> 8;
                let ng = (g * a + dg * ia) >> 8;
                let nb = (b * a + db * ia) >> 8;
                canvas.pixels[px] = (nr << 16) | (ng << 8) | nb;
            }
        }
    }
}

fn draw_generic_app_icon(canvas: &mut Canvas, rect: Rect, fill: Color, border: Color) {
    let body = rect.inset(2);
    canvas.fill_rounded_rect(body, 3, fill);
    canvas.stroke_rounded_rect(body, 3, 1, border);
    let title = Rect::new(body.x + 3, body.y + 3, body.w.saturating_sub(6), 4);
    canvas.fill_rect(title, border);
    let line_color = fill.darken(18);
    for i in 0..3i32 {
        let line_y = body.y + 10 + i * 4;
        canvas.fill_rect(
            Rect::new(body.x + 4, line_y, body.w.saturating_sub(8), 1),
            line_color,
        );
    }
}

fn draw_taskbar_dot(canvas: &mut Canvas, cell: Rect, color: Color) {
    let dot_w = RUNNING_MINIMIZED_DOT;
    let dot_x = cell.x + (cell.w as i32 - dot_w as i32) / 2;
    let dot_y = cell.bottom() - dot_w as i32 - 2;
    canvas.fill_rect(Rect::new(dot_x, dot_y, dot_w, dot_w), color);
}

fn draw_running_app_button(
    canvas: &mut Canvas,
    cell: Rect,
    theme: &Theme,
    entry: &RunningAppEntry,
    hovered: bool,
    now: u64,
) {
    let pressed = now.saturating_sub(entry.last_click_at) < APP_PRESS_MS;
    let mut fill;
    let mut border;
    let mut icon_color = theme.text;

    if entry.minimized {
        fill = theme.panel;
        border = theme.border;
    } else {
        fill = theme.accent.darken(68);
        border = theme.accent;
    }

    if hovered {
        fill = if entry.minimized {
            theme.panel_alt
        } else {
            theme.accent.darken(58)
        };
    }
    if pressed {
        fill = theme.accent_hover.darken(35);
        border = theme.accent_hover;
        icon_color = theme.text;
    }

    canvas.fill_rounded_rect(cell, 5, fill);
    canvas.stroke_rounded_rect(cell, 5, 1, border);
    // Marker invariant: active (non-minimized) entries get a full-width accent
    // bar along the bottom here; minimized entries instead get the accent dot,
    // drawn after the icon near the end of this function.
    if !entry.minimized {
        canvas.fill_rect(
            Rect::new(cell.x + 4, cell.bottom() - 3, cell.w.saturating_sub(8), 2),
            theme.accent,
        );
    }

    // Cells are icon-only; the full title is shown via the hover tooltip.
    let icon_rect = Rect::new(
        cell.x + (cell.w as i32 - RUNNING_ICON as i32) / 2,
        cell.y + (cell.h as i32 - RUNNING_ICON as i32) / 2,
        RUNNING_ICON,
        RUNNING_ICON,
    );
    match &entry.icon {
        Some(RunningIcon::Static(img)) => canvas.draw_tga_icon(img, icon_rect),
        Some(RunningIcon::Runtime(bytes)) => draw_tga_bytes(canvas, bytes, icon_rect),
        Some(RunningIcon::Missing) | None => {
            draw_generic_app_icon(canvas, icon_rect, icon_color.darken(72), border)
        }
    }

    if entry.minimized {
        draw_taskbar_dot(canvas, cell, theme.accent);
    }
}

// ---------------------------------------------------------------------------
// Status cluster: clock (tz), network (networkd), battery (placeholder), power
// ---------------------------------------------------------------------------

/// Query "tz" for full local time + basic zone info. Returns (y,m,d,h,min,s) on success.
/// Also fills a small tz id buffer from reply or fallback.
fn query_local_full(
    out_tz: &mut [u8; 48],
    out_tz_len: &mut usize,
) -> Option<(u16, u8, u8, u8, u8, u8)> {
    let Some(tz) = nameserver_lookup("tz") else {
        return None;
    };
    let Ok(reply) = ipc_call_timeout(
        tz,
        IpcMsg::with_label(TzMsg::GET_LOCAL_TIME),
        TIME_IPC_TIMEOUT_MS,
    ) else {
        return None;
    };
    if reply.label != TzMsg::REPLY {
        return None;
    }
    // word(0): year(u16)<<48 | mon<<40 | day<<32 | h<<24 | min<<16 | s<<8
    let w = reply.words[0];
    let y = ((w >> 48) & 0xffff) as u16;
    let mon = ((w >> 40) & 0xff) as u8;
    let d = ((w >> 32) & 0xff) as u8;
    let h = ((w >> 24) & 0xff) as u8;
    let mi = ((w >> 16) & 0xff) as u8;
    let s = ((w >> 8) & 0xff) as u8;

    // Try GET_ZONE for id (best effort, non fatal)
    *out_tz_len = 3;
    out_tz[..3].copy_from_slice(b"UTC");
    if let Ok(zr) = ipc_call_timeout(tz, IpcMsg::with_label(TzMsg::GET_ZONE), TIME_IPC_TIMEOUT_MS) {
        if zr.label == TzMsg::REPLY {
            // id packed starting word 2
            let mut tmp = [0u8; 48];
            let mut l = 0usize;
            for wi in 2..6 {
                let wd = zr.words[wi];
                for bi in 0..8 {
                    let b = (wd >> (bi * 8)) as u8;
                    if b == 0 {
                        break;
                    }
                    if l < 47 {
                        tmp[l] = b;
                        l += 1;
                    }
                }
            }
            if l > 0 {
                out_tz[..l].copy_from_slice(&tmp[..l]);
                *out_tz_len = l;
            }
        }
    }
    Some((y, mon, d, h, mi, s))
}

/// Query networkd for any non-loopback interface that is Up or Carrier.
/// Returns Some(true/false) on success.
fn query_net_up() -> Option<bool> {
    let Some(netd) = nameserver_lookup("networkd") else {
        return None;
    };
    let mut idx = 0u64;
    loop {
        let Ok(reply) = ipc_call_timeout(
            netd,
            IpcMsg::with_label(NetworkdMsg::LIST_INTERFACES).word(0, idx),
            NET_IPC_TIMEOUT_MS,
        ) else {
            return None;
        };
        let Some(sum) = unpack_iface_summary(&reply) else {
            break;
        };
        if sum.kind != InterfaceKind::Loopback {
            if sum.link == LinkState::Up || sum.link == LinkState::Carrier {
                return Some(true);
            }
        }
        idx += 1;
        if sum.total > 0 && idx as u16 >= sum.total {
            break;
        }
    }
    Some(false)
}

/// Format hour/min (0-23,0-59) into compact "H:MM AM" style in a stack buffer.
/// Returns the length written.
fn format_time_12h(h: u8, m: u8, out: &mut [u8; 8]) -> usize {
    if h > 23 || m > 59 {
        // fallback
        out[..5].copy_from_slice(b"??:??");
        return 5;
    }
    let mut hh = h % 12;
    if hh == 0 {
        hh = 12;
    }
    let am = h < 12;
    // write hour (1 or 2 digits)
    let mut pos = 0usize;
    if hh >= 10 {
        out[pos] = b'0' + (hh / 10);
        pos += 1;
    }
    out[pos] = b'0' + (hh % 10);
    pos += 1;
    out[pos] = b':';
    pos += 1;
    out[pos] = b'0' + (m / 10);
    pos += 1;
    out[pos] = b'0' + (m % 10);
    pos += 1;
    out[pos] = b' ';
    pos += 1;
    if am {
        out[pos] = b'A';
        pos += 1;
        out[pos] = b'M';
        pos += 1;
    } else {
        out[pos] = b'P';
        pos += 1;
        out[pos] = b'M';
        pos += 1;
    }
    pos
}

/// Draw the top-right status cluster. Returns the leftmost x used (for zone calc).
fn draw_status_cluster(
    canvas: &mut Canvas,
    theme: &Theme,
    bar: Rect,
    net_up: bool,
    h: u8,
    m: u8,
) -> i32 {
    // We draw right-to-left: clock | battery | net | power
    let mut x = bar.right() - 12; // right padding inside bar

    // Clock text first (rightmost)
    let mut tbuf = [0u8; 8];
    let tlen = format_time_12h(h, m, &mut tbuf);
    let ts = core::str::from_utf8(&tbuf[..tlen]).unwrap_or("??:??");
    let tw = measure_text(ts, FontRole::UiSmall).w;
    let clock_x = x - tw as i32;
    draw_text_vcenter(
        canvas,
        ts,
        clock_x,
        bar.y,
        bar.h,
        &TextStyle::new(FontRole::UiSmall, theme.text),
    );
    x = clock_x - 8;

    // Battery icon (static placeholder)
    // TODO(battery): replace with live data from powerd/ACPI when available.
    // For now this is a synthetic icon; no driver queries.
    let bat_cell = Rect::new(
        x - ICON_BTN as i32,
        bar.y + (TOP_H as i32 - ICON_BTN as i32) / 2,
        ICON_BTN,
        ICON_BTN,
    );
    draw_icon16(canvas, bat_cell, &BAT_ROWS, theme.text_dim);
    x = bat_cell.x - 4;

    // Network icon
    let net_rows = if net_up { &NET_ON_ROWS } else { &NET_OFF_ROWS };
    let net_cell = Rect::new(
        x - ICON_BTN as i32,
        bar.y + (TOP_H as i32 - ICON_BTN as i32) / 2,
        ICON_BTN,
        ICON_BTN,
    );
    draw_icon16(
        canvas,
        net_cell,
        net_rows,
        if net_up { theme.ok } else { theme.text_dim },
    );
    x = net_cell.x - 4;

    // Power icon (leftmost of cluster; acts as a button zone)
    let pwr_cell = Rect::new(
        x - ICON_BTN as i32,
        bar.y + (TOP_H as i32 - ICON_BTN as i32) / 2,
        ICON_BTN,
        ICON_BTN,
    );
    draw_icon16(canvas, pwr_cell, &POWER_ROWS, theme.warn);

    // Return left edge of power cell for click zone
    pwr_cell.x
}

// ---------------------------------------------------------------------------
// Top bar layout
// ---------------------------------------------------------------------------

fn draw_top_bar(canvas: &mut Canvas, theme: &Theme, screen_w: u32, shell: &mut VortexShell) {
    let bar = Rect::new(TOP_PAD, TOP_Y, screen_w - (TOP_PAD * 2) as u32, TOP_H);
    draw_panel(canvas, bar, theme.panel, theme.border);

    let sym = shell.symbols;

    // ── Left zone: improved 8-ray sun + SunlightOS + quick shortcuts ─────────
    let mut x = bar.x + 8;

    // Sunlight mark: prefer sunny symbol glyph (orange), else fallback 8-ray-ish pixel
    let sun_w = 22u32;
    let sun_cell = Rect::new(x, bar.y + 2, sun_w, TOP_H - 4);
    if let Some(tga) = sym.sunny {
        // Draw white glyph then manually accent tint using simple mask (see helper below)
        draw_tga_tinted_orange(canvas, &tga, sun_cell, theme.accent);
    } else {
        draw_icon16(canvas, sun_cell, &SUN_ROWS, theme.accent);
    }
    x += sun_w as i32 + 4;

    // "SunlightOS" label (small, clean)
    let brand = "SunlightOS";
    draw_text_vcenter(
        canvas,
        brand,
        x,
        bar.y,
        bar.h,
        &TextStyle::new(FontRole::UiSmall, theme.text),
    );
    let brand_w = measure_text(brand, FontRole::UiSmall).w as i32;
    x += brand_w + 8;

    // Workspace indicator (semantic glyphs) — sits on the left right after the
    // brand, replacing the old static shortcut icons. Each button maps to a
    // workspace: home(1) · browser/public(2) · code(3) · office/article(4).
    // Glyphs come from the build-time Material Symbols rasteriser (see build.rs).
    x += TOP_WS_LEFT_GAP;
    let ws_glyphs = [sym.home, sym.public, sym.code, sym.article];
    let ws_btn_y = bar.y + (TOP_H - WS_BTN_H) as i32 / 2;
    let mut bx = x;
    for i in 0..WS_INDICATOR_COUNT {
        let cell = Rect::new(bx, ws_btn_y, WS_BTN_W, WS_BTN_H);
        let active = (i as u8) + 1 == shell.current_workspace;
        if active {
            // Filled accent pill makes the active workspace clearly distinct.
            canvas.fill_rounded_rect(cell, 5, theme.accent);
        }
        if let Some(tga) = ws_glyphs[i] {
            let ic = WS_ICON_SIZE as i32;
            let ic_cell = Rect::new(
                bx + (WS_BTN_W as i32 - ic) / 2,
                ws_btn_y + (WS_BTN_H as i32 - ic) / 2,
                WS_ICON_SIZE,
                WS_ICON_SIZE,
            );
            let tint = if active {
                theme.text_on_accent
            } else {
                theme.text_dim
            };
            draw_tga_tinted_orange(canvas, &tga, ic_cell, tint);
        }
        shell.workspace_btn_zones[i] = cell;
        bx += WS_BTN_W as i32 + WS_BTN_GAP;
    }
    let ws_cluster_w = (WS_INDICATOR_COUNT as i32) * (WS_BTN_W as i32)
        + ((WS_INDICATOR_COUNT as i32) - 1) * WS_BTN_GAP;
    shell.workspace_zone = Rect::new(x, ws_btn_y, ws_cluster_w as u32, WS_BTN_H);

    // ── Center: localized date/time (clickable, hoverable) ───────────────────
    // Use cached full time + locale. Format per spec. Updates on minute change only.
    let center_text = format_center_datetime(
        shell.status_year,
        shell.status_month,
        shell.status_day,
        shell.status_hour,
        shell.status_min,
        core::str::from_utf8(&shell.locale[..shell.locale_len]).unwrap_or("C.UTF-8"),
    );
    let tw = measure_text(&center_text, FontRole::UiMedium).w as i32;
    let cx = bar.x + (bar.w as i32 - tw) / 2;
    let cy = bar.y;
    draw_text_vcenter(
        canvas,
        &center_text,
        cx,
        cy,
        bar.h,
        &TextStyle::new(FontRole::UiMedium, theme.text),
    );
    // clickable/hover rect around the text cluster
    let pad = 6;
    shell.datetime_zone = Rect::new(cx - pad, cy, (tw + pad * 2) as u32, bar.h);

    // ── Right: meaningful indicators (no duplicate time) ─────────────────────
    // [lan] [notif] [logout]   (workspace indicator lives on the left now)
    // Spacing is driven by small constants so the cluster stays balanced.
    let mut rx = bar.right() - TOP_RIGHT_PAD;
    let ic = TOP_ICON_SIZE; // status icon size

    // Logout
    rx -= ic as i32;
    let logout_cell = Rect::new(rx, bar.y + (TOP_H - ic) as i32 / 2, ic, ic);
    if let Some(tga) = sym.logout {
        draw_tga_tinted_orange(canvas, &tga, logout_cell, theme.accent);
    }
    shell.logout_zone = logout_cell;
    rx -= TOP_ICON_GAP;

    // Notifications
    rx -= ic as i32;
    let notif_cell = Rect::new(rx, bar.y + (TOP_H - ic) as i32 / 2, ic, ic);
    if let Some(tga) = sym.notifications {
        draw_tga_tinted_orange(canvas, &tga, notif_cell, theme.text_dim);
    }
    shell.notif_zone = notif_cell;
    rx -= TOP_ICON_GAP;

    // Network (lan)
    rx -= ic as i32;
    let net_cell = Rect::new(rx, bar.y + (TOP_H - ic) as i32 / 2, ic, ic);
    let net_color = if shell.status_net_up {
        theme.accent
    } else {
        theme.text_dim
    };
    if let Some(tga) = sym.lan {
        draw_tga_tinted_orange(canvas, &tga, net_cell, net_color);
    }
    shell.net_zone = net_cell;
}

/// Format the center date/time cluster per spec.
/// en_US.UTF-8-ish: "Wed, Jul 8   12:39 AM"
/// C / C.UTF-8:     "2026-07-08   00:39"
/// Uses sunlight-locale helpers. 12h only for en style in this polish.
fn format_center_datetime(y: u16, mon: u8, d: u8, h: u8, mi: u8, loc: &str) -> String {
    let is_en = loc.to_ascii_lowercase().starts_with("en_us")
        || loc.to_ascii_lowercase().starts_with("en-us");
    if is_en {
        // short names (weekday_name with 0 falls back inside helper)
        let wd = sunlight_locale::weekday_name(0, false, loc);
        let mon_s = sunlight_locale::month_name(mon, false, loc);
        let mut hh = h % 12;
        if hh == 0 {
            hh = 12;
        }
        let ap = if h >= 12 { "PM" } else { "AM" };
        alloc::format!("{}, {} {}   {}:{:02} {}", wd, mon_s, d, hh, mi, ap)
    } else {
        alloc::format!("{:04}-{:02}-{:02}   {:02}:{:02}", y, mon, d, h, mi)
    }
}

fn cal_is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn cal_days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if cal_is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn cal_weekday_sun0(year: u16, month: u8, day: u8) -> usize {
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year as i32;
    let m = month as i32;
    if m < 3 {
        y -= 1;
    }
    ((y + y / 4 - y / 100 + y / 400 + t[(m - 1) as usize] + day as i32) % 7) as usize
}

fn format_cal_date(year: u16, month: u8, day: u8) -> String {
    alloc::format!("{:04}-{:02}-{:02}", year, month, day)
}

fn format_legacy_slash_date(year: u16, month: u8, day: u8) -> String {
    alloc::format!("{:04}/{:02}/{:02}", year, month, day)
}

fn format_event_key(event_id: u64) -> String {
    alloc::format!("{}{:016x}", CAL_EVENT_PREFIX, event_id)
}

fn calendar_index_key(date: &str) -> String {
    let mut key = String::from(CAL_INDEX_BY_DATE_PREFIX);
    key.push_str(date);
    key
}

fn parse_id_list(bytes: &[u8]) -> Vec<u64> {
    let mut ids = Vec::new();
    let Ok(text) = core::str::from_utf8(bytes) else {
        return ids;
    };
    for line in text.lines() {
        if let Some(id) = parse_u64_ascii(line.as_bytes()) {
            if !ids.iter().any(|existing| *existing == id) {
                ids.push(id);
            }
        }
    }
    ids
}

fn parse_u64_ascii(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(value)
}

fn calendar_day_has_events(year: u16, month: u8, day: u8) -> bool {
    let date = format_cal_date(year, month, day);
    kv_get_bytes(&calendar_index_key(&date))
        .map(|bytes| !parse_id_list(&bytes).is_empty())
        .unwrap_or(false)
}

fn calendar_day_has_items(year: u16, month: u8, day: u8) -> bool {
    if calendar_day_has_events(year, month, day) {
        return true;
    }
    let date = format_cal_date(year, month, day);
    let legacy_date = format_legacy_slash_date(year, month, day);
    !load_task_ids_with_legacy(
        &reminder_due_date_list_key(&date),
        &reminder_due_date_list_key(&legacy_date),
    )
    .is_empty()
        || !load_task_ids_with_legacy(
            &reminder_date_list_key(&date),
            &reminder_date_list_key(&legacy_date),
        )
        .is_empty()
}

fn load_calendar_events_for_day(year: u16, month: u8, day: u8) -> Vec<CalendarMiniEvent> {
    let mut events = Vec::new();
    let date = format_cal_date(year, month, day);
    let Some(index_bytes) = kv_get_bytes(&calendar_index_key(&date)) else {
        return events;
    };
    for id in parse_id_list(&index_bytes)
        .into_iter()
        .take(CAL_POPUP_EVENTS)
    {
        if let Some(bytes) = kv_get_bytes(&format_event_key(id)) {
            if let Some(event) = parse_calendar_event_summary(&bytes) {
                events.push(event);
            }
        }
    }
    events
}

fn load_tasks_and_reminders_for_day(
    year: u16,
    month: u8,
    day: u8,
) -> (Vec<SelectedDayTaskPreview>, Vec<SelectedDayReminderPreview>) {
    let date = format_cal_date(year, month, day);
    let legacy_date = format_legacy_slash_date(year, month, day);
    let due_ids = load_task_ids_with_legacy(
        &reminder_due_date_list_key(&date),
        &reminder_due_date_list_key(&legacy_date),
    );
    let reminder_ids = load_task_ids_with_legacy(
        &reminder_date_list_key(&date),
        &reminder_date_list_key(&legacy_date),
    );
    let selected = build_selected_day_previews(
        &date,
        &due_ids,
        &reminder_ids,
        |id| kv_get_bytes(&task_key(id)).and_then(|bytes| decode_task(&bytes)),
        |list_id| sunlight_reminders::default_list_name(list_id).map(String::from),
    );
    (selected.tasks, selected.reminders)
}

fn load_task_ids_with_legacy(canonical_key: &str, legacy_key: &str) -> Vec<u64> {
    kv_get_bytes(canonical_key)
        .and_then(|bytes| parse_task_id_list(&bytes))
        .filter(|ids| !ids.is_empty())
        .or_else(|| kv_get_bytes(legacy_key).and_then(|bytes| parse_task_id_list(&bytes)))
        .unwrap_or_default()
}

fn parse_calendar_event_summary(bytes: &[u8]) -> Option<CalendarMiniEvent> {
    let text = core::str::from_utf8(bytes).ok()?;
    if !text.starts_with("SCAL2\n") {
        return None;
    }
    let mut title = String::new();
    let mut start = String::new();
    let mut all_day = false;
    for line in text.lines().skip(1) {
        let Some(eq) = line.find('=') else { continue };
        let name = &line[..eq];
        let value = unescape_calendar_field(&line[eq + 1..]);
        match name {
            "title" => title = value,
            "start" => start = value,
            "all_day" => all_day = value == "1",
            _ => {}
        }
    }
    if title.is_empty() {
        return None;
    }
    if title.chars().count() > CAL_EVENT_TITLE_MAX {
        title = ellipsize_label(&title, CAL_EVENT_TITLE_MAX);
    }
    let time = if all_day {
        String::from("All day")
    } else if start.is_empty() {
        String::from("No time")
    } else {
        start
    };
    Some(CalendarMiniEvent { title, time })
}

fn unescape_calendar_field(value: &str) -> String {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            out.push(if ch == 'n' { ' ' } else { ch });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    out
}

fn kv_cap() -> Option<CapabilityToken> {
    let cached = unsafe { KV_CAP_CACHE };
    if cached != CapabilityToken::INVALID {
        return Some(cached);
    }
    let cap = nameserver_lookup_timeout("sunlight-kv", KV_LOOKUP_TIMEOUT_MS)?;
    unsafe {
        KV_CAP_CACHE = cap;
    }
    Some(cap)
}

fn kv_get_bytes(key: &str) -> Option<Vec<u8>> {
    if key.len() > SHM_PAGE {
        return None;
    }
    let cap = kv_cap()?;
    let (key_ptr, key_tok) = shm_alloc().ok()?;
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
    }
    let msg = IpcMsg::with_label(KV_GET_SHM2)
        .word(0, key.len() as u64)
        .with_cap(0, key_tok);
    let reply_res = ipc_call_timeout(cap, msg, KV_IPC_TIMEOUT_MS);
    let _ = shm_free(key_tok);
    let reply = reply_res.ok()?;
    if reply.label == KV_ERROR {
        return None;
    }
    if reply.label != KV_VALUE {
        return None;
    }
    let len = (reply.words[0] as usize).min(SHM_PAGE);
    if len == 0 {
        return Some(Vec::new());
    }
    let tok = reply.caps[0];
    if tok == CapabilityToken::INVALID {
        return None;
    }
    let ptr = match shm_map(tok) {
        Ok(ptr) => ptr,
        Err(_) => {
            let _ = shm_free(tok);
            return None;
        }
    };
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(tok);
    Some(bytes)
}

// ---------------------------------------------------------------------------
// Desktop icons and menu
// ---------------------------------------------------------------------------

fn sanitize_ascii(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        match b {
            0 | b'\n' | b'\r' => break,
            0x20..=0x7e => out.push(b as char),
            _ => out.push('?'),
        }
    }
    out
}

fn ellipsize_label(text: &str, max_chars: usize) -> String {
    let chars = text.chars().count();
    if chars <= max_chars {
        return String::from(text);
    }
    let keep = max_chars.saturating_sub(3);
    let mut out = String::with_capacity(max_chars);
    for ch in text.chars().take(keep) {
        out.push(ch);
    }
    out.push('.');
    out.push('.');
    out.push('.');
    out
}

#[derive(Clone, PartialEq, Eq)]
struct NotificationRecord {
    storage_key: String,
    id: String,
    timestamp: String,
    sender_pid: Option<u64>,
    sender_name: String,
    sender_icon: Option<String>,
    owner: String,
    title: String,
    body: String,
    priority: NotificationPriority,
    seen: bool,
    dismissed: bool,
}

fn parse_notification_priority(value: &str) -> NotificationPriority {
    match value {
        "low" => NotificationPriority::Low,
        "high" => NotificationPriority::High,
        "critical" => NotificationPriority::Critical,
        _ => NotificationPriority::Normal,
    }
}

fn unescape_notification_field(value: &str) -> String {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            out.push(match ch {
                'n' => '\n',
                'e' => '=',
                '\\' => '\\',
                other => other,
            });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    out
}

fn append_notification_escaped(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '=' => out.push_str("\\e"),
            _ => out.push(ch),
        }
    }
}

fn append_notification_field(out: &mut String, key: &str, value: &str) {
    out.push_str(key);
    out.push('=');
    append_notification_escaped(out, value);
    out.push('\n');
}

fn notification_priority_name(priority: NotificationPriority) -> &'static str {
    match priority {
        NotificationPriority::Low => "low",
        NotificationPriority::Normal => "normal",
        NotificationPriority::High => "high",
        NotificationPriority::Critical => "critical",
    }
}

fn decode_notification_record(storage_key: &str, bytes: &[u8]) -> Option<NotificationRecord> {
    let text = core::str::from_utf8(bytes).ok()?;
    let mut record = NotificationRecord {
        storage_key: String::from(storage_key),
        id: String::new(),
        timestamp: String::new(),
        sender_pid: None,
        sender_name: String::new(),
        sender_icon: None,
        owner: String::new(),
        title: String::new(),
        body: String::new(),
        priority: NotificationPriority::Normal,
        seen: false,
        dismissed: false,
    };
    for line in text.lines().skip(1) {
        let Some(eq) = line.find('=') else { continue };
        let key = &line[..eq];
        let value = unescape_notification_field(&line[eq + 1..]);
        match key {
            "id" => record.id = value,
            "timestamp" => record.timestamp = value,
            "sender_pid" => record.sender_pid = parse_u64_ascii(value.as_bytes()),
            "sender_name" => record.sender_name = value,
            "sender_icon" => record.sender_icon = if value.is_empty() { None } else { Some(value) },
            "owner" => record.owner = value,
            "title" => record.title = value,
            "body" => record.body = value,
            "priority" => record.priority = parse_notification_priority(&value),
            "seen" => record.seen = value == "1" || value == "true",
            "dismissed" => record.dismissed = value == "1" || value == "true",
            _ => {}
        }
    }
    if record.id.is_empty() || record.title.is_empty() {
        return None;
    }
    if record.sender_name.is_empty() {
        record.sender_name = String::from("Unknown sender");
    }
    if record.owner.is_empty() {
        record.owner = String::from("unknown");
    }
    Some(record)
}

fn encode_notification_record(record: &NotificationRecord) -> Vec<u8> {
    let mut out = String::from("sunlight-notification-v1\n");
    append_notification_field(&mut out, "id", &record.id);
    append_notification_field(&mut out, "timestamp", &record.timestamp);
    append_notification_field(
        &mut out,
        "sender_pid",
        &record
            .sender_pid
            .map(|pid| alloc::format!("{}", pid))
            .unwrap_or_default(),
    );
    append_notification_field(&mut out, "sender_name", &record.sender_name);
    append_notification_field(
        &mut out,
        "sender_icon",
        record.sender_icon.as_deref().unwrap_or(""),
    );
    append_notification_field(&mut out, "owner", &record.owner);
    append_notification_field(&mut out, "title", &record.title);
    append_notification_field(&mut out, "body", &record.body);
    append_notification_field(
        &mut out,
        "priority",
        notification_priority_name(record.priority),
    );
    append_notification_field(&mut out, "seen", if record.seen { "1" } else { "0" });
    append_notification_field(
        &mut out,
        "dismissed",
        if record.dismissed { "1" } else { "0" },
    );
    out.into_bytes()
}

fn notification_history_recent(limit: usize, include_dismissed: bool) -> Vec<NotificationRecord> {
    let mut index = [0u8; SHM_PAGE];
    let Some(index_len) = notification_kv_get_into(NOTIFICATION_RECENT_KEY, &mut index) else {
        return Vec::new();
    };
    let Ok(text) = core::str::from_utf8(&index[..index_len]) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut value = [0u8; SHM_PAGE];
    for key in text.lines().filter(|line| !line.is_empty()) {
        if out.len() >= limit {
            break;
        }
        let Some(value_len) = notification_kv_get_into(key, &mut value) else {
            continue;
        };
        let Some(record) = decode_notification_record(key, &value[..value_len]) else {
            continue;
        };
        if !include_dismissed && record.dismissed {
            continue;
        }
        out.push(record);
    }
    out
}

fn notification_store_record(record: &NotificationRecord) {
    let encoded = encode_notification_record(record);
    let _ = notification_kv_put(&record.storage_key, &encoded);
}

fn notification_set_seen(record: &NotificationRecord, seen: bool) {
    let mut updated = record.clone();
    updated.seen = seen;
    notification_store_record(&updated);
}

fn notification_set_dismissed(record: &NotificationRecord, dismissed: bool) {
    let mut updated = record.clone();
    updated.dismissed = dismissed;
    notification_store_record(&updated);
}

fn notification_group_label(record: &NotificationRecord) -> String {
    if record.sender_name == record.owner {
        record.sender_name.clone()
    } else {
        alloc::format!("{} · {}", record.sender_name, record.owner)
    }
}

fn notification_time_label(record: &NotificationRecord) -> String {
    let Some(timestamp) = parse_u64_ascii(record.timestamp.as_bytes()) else {
        return String::from("recently");
    };
    let now = get_time_utc();
    if timestamp == 0 || now < timestamp {
        return String::from("recently");
    }
    let age = now.saturating_sub(timestamp);
    if age < 60 {
        String::from("just now")
    } else if age < 3_600 {
        alloc::format!("{}m ago", age / 60)
    } else if age < 86_400 {
        alloc::format!("{}h ago", age / 3_600)
    } else {
        alloc::format!("{}d ago", age / 86_400)
    }
}

fn notification_meta(record: &NotificationRecord) -> String {
    notification_time_label(record)
}

fn notification_priority_color(priority: NotificationPriority) -> Color {
    match priority {
        NotificationPriority::Low => Color(0x00667777),
        NotificationPriority::Normal => Color(0x00888899),
        NotificationPriority::High => Color(0x00D6A94A),
        NotificationPriority::Critical => Color(0x00D95F5F),
    }
}

fn join_path(base: &str, leaf: &str) -> String {
    let mut out = String::with_capacity(base.len() + leaf.len() + 1);
    out.push_str(base);
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(leaf.trim_start_matches('/'));
    out
}

fn load_wallpaper_from_config(cfg: &DesktopConfig) -> (Option<TgaImage>, bool) {
    let Some(bytes) = read_wallpaper_bytes(cfg.wallpaper.as_bytes()) else {
        debug_log("[VORTEX] wallpaper config path unreadable\n");
        return (None, true);
    };
    if bytes.is_empty() {
        return (None, true);
    }
    if !is_supported_wallpaper(bytes) {
        debug_log("[VORTEX] wallpaper unsupported or corrupt\n");
        return (None, true);
    }
    match TgaImage::parse(bytes) {
        Ok(img) => (Some(img), false),
        Err(_) => {
            debug_log("[VORTEX] wallpaper parse failed\n");
            (None, true)
        }
    }
}

fn read_wallpaper_bytes(path: &[u8]) -> Option<&'static [u8]> {
    static mut WALLPAPER_BUF: [u8; WALLPAPER_MAX_BYTES] = [0u8; WALLPAPER_MAX_BYTES];

    let fd = libc::open(path).ok()?;
    let mut len = 0usize;
    loop {
        let remaining = WALLPAPER_MAX_BYTES.saturating_sub(len);
        if remaining == 0 {
            break;
        }
        let take = remaining.min(4096);
        let chunk = unsafe {
            core::slice::from_raw_parts_mut(
                core::ptr::addr_of_mut!(WALLPAPER_BUF).cast::<u8>().add(len),
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
    Some(unsafe {
        core::slice::from_raw_parts(core::ptr::addr_of!(WALLPAPER_BUF).cast::<u8>(), len)
    })
}

fn read_file_bytes(path: &[u8], limit: usize) -> Option<Vec<u8>> {
    let fd = libc::open(path).ok()?;
    let reserve = libc::fstat(fd)
        .ok()
        .map(|stat| (stat.size as usize).min(limit))
        .unwrap_or(0);
    let mut out = Vec::with_capacity(reserve);
    let mut buf = [0u8; 128];
    loop {
        let n = match libc::read(fd, &mut buf) {
            Ok(n) => n,
            Err(_) => {
                let _ = libc::close(fd);
                return None;
            }
        };
        if n == 0 {
            break;
        }
        let take = (limit - out.len()).min(n);
        out.extend_from_slice(&buf[..take]);
        if out.len() >= limit || take < n {
            break;
        }
    }
    let _ = libc::close(fd);
    Some(out)
}

fn parse_u32_ascii(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for &b in bytes {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}

fn read_hostname() -> String {
    if let Some(bytes) = read_file_bytes(b"/etc/hostname", 128) {
        let host = sanitize_ascii(&bytes);
        if !host.is_empty() {
            return host;
        }
    }
    String::from("sunlight")
}

/// Read /etc/locale.conf (or fallback) and return effective LC_TIME/LANG string.
/// Uses sunlight-locale parser for correct fallback chain. Safe if file missing.
fn read_locale_effective() -> Option<String> {
    let data = read_file_bytes(b"/etc/locale.conf", 1024).unwrap_or_default();
    let cfg = sunlight_locale::parse_locale_conf(&data);
    let eff = cfg.lc_time();
    if eff.is_empty() {
        None
    } else {
        Some(alloc::string::String::from(eff))
    }
}

fn root_desktop_paths(hostname: String) -> DesktopPaths {
    DesktopPaths {
        _username: String::from("root"),
        home_dir: String::from("/root"),
        desktop_dir: String::from("/root/Desktop"),
        trash_dir: String::from("/root/.local/share/Trash"),
        hostname,
    }
}

fn lookup_user_by_uid(uid: u32) -> Option<(String, String)> {
    let bytes = read_file_bytes(b"/etc/passwd", 2048)?;
    for line in bytes.split(|&b| b == b'\n') {
        if line.is_empty() || line[0] == b'#' {
            continue;
        }
        let mut parts = line.split(|&b| b == b':');
        let username = parts.next()?;
        let _passwd = parts.next()?;
        let uid_field = parts.next()?;
        let _gid = parts.next()?;
        let _comment = parts.next()?;
        let home = parts.next()?;
        if parse_u32_ascii(uid_field)? != uid {
            continue;
        }
        let uname = sanitize_ascii(username);
        if uname.is_empty() {
            return None;
        }
        let home_dir = {
            let parsed = sanitize_ascii(home);
            if parsed.is_empty() {
                let mut h = String::from("/home/");
                h.push_str(&uname);
                h
            } else {
                parsed
            }
        };
        return Some((uname, home_dir));
    }
    None
}

fn resolve_desktop_paths() -> DesktopPaths {
    let hostname = read_hostname();
    let uid = libc::getuid() as u32;
    if uid == 0 {
        return root_desktop_paths(hostname);
    }
    if let Some((username, home_dir)) = lookup_user_by_uid(uid) {
        let desktop_dir = join_path(&home_dir, "Desktop");
        let trash_dir = join_path(&home_dir, ".local/share/Trash");
        return DesktopPaths {
            _username: username,
            home_dir,
            desktop_dir,
            trash_dir,
            hostname,
        };
    }
    debug_log("[VORTEX] TODO(user): desktop path fallback to /root/Desktop\n");
    root_desktop_paths(hostname)
}

fn ensure_directory(path: &str) {
    if libc::stat(path.as_bytes()).is_ok() {
        return;
    }
    if libc::mkdir_recursive(path.as_bytes()).is_err() {
        debug_log("[VORTEX] desktop dir create failed\n");
    }
}

fn make_desktop_icon(
    name: String,
    tooltip: &str,
    action: String,
    kind: DesktopIconKind,
) -> DesktopIcon {
    let label = ellipsize_label(&name, DESKTOP_LABEL_CHARS);
    DesktopIcon {
        name,
        label,
        _tooltip: String::from(tooltip),
        _action: action,
        kind,
        rect: Rect::new(0, 0, 0, 0),
    }
}

fn maybe_add_drive_icon(icons: &mut Vec<DesktopIcon>, path: &str, display_name: &str) {
    if libc::stat(path.as_bytes()).is_ok() {
        icons.push(make_desktop_icon(
            String::from(display_name),
            "Mounted drive",
            String::from(path),
            DesktopIconKind::Drive,
        ));
    }
}

fn load_drive_icons() -> Vec<DesktopIcon> {
    let mut icons = Vec::new();
    maybe_add_drive_icon(&mut icons, "/boot", "boot");
    let mut entries = [DirEntry::zeroed(); MAX_DIR_ENTRIES];
    if let Ok(count) = libc::read_dir(b"/mnt", &mut entries) {
        for entry in entries.iter().take(count) {
            if entry.file_type != FT_DIR {
                continue;
            }
            let name = sanitize_ascii(entry.name_bytes());
            if name.is_empty() {
                continue;
            }
            let path = join_path("/mnt", &name);
            icons.push(make_desktop_icon(
                name,
                "Mounted drive",
                path,
                DesktopIconKind::Drive,
            ));
        }
    }
    icons.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    icons
}

fn load_desktop_dir_icons(desktop_dir: &str) -> Vec<DesktopIcon> {
    let mut icons = Vec::new();
    let mut entries = [DirEntry::zeroed(); MAX_DIR_ENTRIES];
    if let Ok(count) = libc::read_dir(desktop_dir.as_bytes(), &mut entries) {
        for entry in entries.iter().take(count) {
            let name = sanitize_ascii(entry.name_bytes());
            if name.is_empty() {
                continue;
            }
            let path = join_path(desktop_dir, &name);
            let kind = if entry.file_type == FT_DIR {
                DesktopIconKind::Folder
            } else if is_supported_image_name(&name) {
                DesktopIconKind::Image
            } else if name.ends_with(".desktop") {
                DesktopIconKind::DesktopEntry
            } else {
                DesktopIconKind::File
            };
            icons.push(make_desktop_icon(name, "Desktop entry", path, kind));
        }
    }
    icons.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    icons
}

fn load_desktop_icons(paths: &DesktopPaths) -> Vec<DesktopIcon> {
    let mut icons = Vec::new();
    icons.push(make_desktop_icon(
        paths.hostname.clone(),
        "Computer",
        String::from("computer:///"),
        DesktopIconKind::Computer,
    ));
    icons.push(make_desktop_icon(
        String::from("Home"),
        "Home folder",
        paths.home_dir.clone(),
        DesktopIconKind::Home,
    ));
    icons.push(make_desktop_icon(
        String::from("Trash"),
        "Trash",
        paths.trash_dir.clone(),
        DesktopIconKind::Trash,
    ));
    icons.push(make_desktop_icon(
        String::from("Network"),
        "Network locations",
        String::from("network:///"),
        DesktopIconKind::Network,
    ));
    icons.extend(load_drive_icons());
    icons.extend(load_desktop_dir_icons(&paths.desktop_dir));
    icons
}

fn desktop_area(screen_w: u32, screen_h: u32) -> Rect {
    let x = TOP_PAD + 10;
    let y = TOP_Y + TOP_H as i32 + 14;
    let bottom = bot_y(screen_h) - 10;
    Rect::new(
        x,
        y,
        (screen_w as i32 - x - TOP_PAD - 10).max(0) as u32,
        (bottom - y).max(0) as u32,
    )
}

fn layout_desktop_icons(icons: &mut [DesktopIcon], area: Rect) {
    let rows = ((area.h / DESKTOP_CELL_H).max(1)) as usize;
    let cols = ((area.w / DESKTOP_CELL_W).max(1)) as usize;
    for (i, icon) in icons.iter_mut().enumerate() {
        let col = i / rows;
        let row = i % rows;
        if col >= cols {
            icon.rect = Rect::new(-1024, -1024, 0, 0);
            continue;
        }
        icon.rect = Rect::new(
            area.x + col as i32 * DESKTOP_CELL_W as i32,
            area.y + row as i32 * DESKTOP_CELL_H as i32,
            DESKTOP_CELL_W,
            DESKTOP_CELL_H,
        );
    }
}

fn desktop_icon_visual(kind: DesktopIconKind, theme: &Theme) -> (&'static [u16; 16], Color) {
    match kind {
        DesktopIconKind::Computer => (&COMPUTER_ROWS, theme.accent),
        DesktopIconKind::Home => (&HOME_ROWS, theme.ok),
        DesktopIconKind::Trash => (&TRASH_ROWS, theme.text_dim),
        DesktopIconKind::Network => (&NET_ON_ROWS, theme.text),
        DesktopIconKind::Drive => (&DRIVE_ROWS, theme.warn),
        DesktopIconKind::Folder => (&FOLDER_ROWS, theme.accent_hover),
        DesktopIconKind::Image => (&FILE_ROWS, theme.accent),
        DesktopIconKind::File | DesktopIconKind::DesktopEntry => (&FILE_ROWS, theme.text),
    }
}

fn is_supported_image_name(name: &str) -> bool {
    name.ends_with(".simg")
        || name.ends_with(".SIMG")
        || name.ends_with(".tga")
        || name.ends_with(".TGA")
}

fn draw_desktop_icons(
    canvas: &mut Canvas,
    theme: &Theme,
    icons: &[DesktopIcon],
    selected: &[usize],
    dt: DesktopTheme,
) {
    for (idx, icon) in icons.iter().enumerate() {
        if icon.rect.w == 0 {
            continue;
        }
        let slot = icon.rect;
        let is_selected = selected.contains(&idx);
        let tile = Rect::new(
            slot.x + 8,
            slot.y + 2,
            slot.w.saturating_sub(16),
            slot.h.saturating_sub(10),
        );
        canvas.fill_rounded_rect(
            tile,
            10,
            if is_selected {
                theme.panel
            } else {
                theme.panel_alt
            },
        );
        canvas.stroke_rounded_rect(
            tile,
            10,
            1,
            if is_selected {
                theme.accent
            } else {
                theme.border
            },
        );
        if is_selected {
            let highlight = slot.inset(4);
            canvas.stroke_rounded_rect(highlight, 8, 1, theme.accent);
        }

        let icon_rect = Rect::new(slot.x + 18, slot.y + 6, 48, 40);

        // Prefer TGA theme icon; fall back to pixel-art if not loaded.
        if let Some(tga) = dt.icon_for_entry(icon.kind, &icon.name) {
            canvas.draw_tga_icon(&tga, icon_rect);
        } else {
            let (rows, color) = desktop_icon_visual(icon.kind, theme);
            draw_icon16_scaled(canvas, icon_rect, rows, color, DESKTOP_ICON_SCALE);
        }

        let icon_color = if is_selected {
            theme.text
        } else {
            theme.text_dim.lighten(90)
        };
        let label_w = measure_text(&icon.label, FontRole::UiSmall).w;
        let label_x = slot.x + (slot.w as i32 - label_w as i32) / 2;
        let label_h = sun_font::line_height(FontRole::UiSmall) + 4;
        let label_rect_y = slot.y + 58;
        canvas.fill_rounded_rect(
            Rect::new(
                slot.x + 12,
                label_rect_y - 2,
                slot.w.saturating_sub(24),
                label_h + 4,
            ),
            6,
            if is_selected {
                theme.panel
            } else {
                theme.panel_alt
            },
        );
        draw_text_vcenter(
            canvas,
            &icon.label,
            label_x,
            label_rect_y,
            label_h,
            &TextStyle::new(FontRole::UiSmall, icon_color),
        );
    }
}

fn draw_desktop_marquee(canvas: &mut Canvas, theme: &Theme, rect: Rect) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let fill = Color::rgba(theme.accent.r(), theme.accent.g(), theme.accent.b(), 48);
    let border = Color::rgba(theme.accent.r(), theme.accent.g(), theme.accent.b(), 180);
    for y in rect.y..rect.bottom() {
        for x in rect.x..rect.right() {
            canvas.blend_pixel(x, y, fill);
        }
    }
    for x in rect.x..rect.right() {
        canvas.blend_pixel(x, rect.y, border);
        canvas.blend_pixel(x, rect.bottom() - 1, border);
    }
    for y in rect.y..rect.bottom() {
        canvas.blend_pixel(rect.x, y, border);
        canvas.blend_pixel(rect.right() - 1, y, border);
    }
}

fn make_context_menu(x: i32, y: i32, screen_w: u32, screen_h: u32) -> ContextMenuState {
    let menu_h = MENU_ITEM_H * MENU_LABELS.len() as u32 + 8;
    let max_x = screen_w as i32 - MENU_W as i32 - 6;
    let max_y = screen_h as i32 - menu_h as i32 - 6;
    let rect = Rect::new(
        x.clamp(6, max_x.max(6)),
        y.clamp(6, max_y.max(6)),
        MENU_W,
        menu_h,
    );
    let mut items = [MenuItem {
        action: ContextMenuAction::Refresh,
        rect: Rect::new(0, 0, 0, 0),
        icon: None,
    }; 6];
    for (i, (_, action)) in MENU_LABELS.iter().enumerate() {
        let icon = match action {
            ContextMenuAction::NewFolder => TgaImage::parse(MENU_NEW_FOLDER_TGA).ok(),
            ContextMenuAction::NewTextFile => TgaImage::parse(MENU_NEW_TEXT_TGA).ok(),
            ContextMenuAction::Refresh => TgaImage::parse(MENU_REFRESH_TGA).ok(),
            ContextMenuAction::SortByName => TgaImage::parse(MENU_SORT_TGA).ok(),
            ContextMenuAction::OpenTerminalHere => TgaImage::parse(MENU_TERMINAL_TGA).ok(),
            ContextMenuAction::WallpaperSettings => TgaImage::parse(ICON_SETTINGS_TGA).ok(),
        };
        items[i] = MenuItem {
            action: *action,
            rect: Rect::new(
                rect.x + 4,
                rect.y + 4 + i as i32 * MENU_ITEM_H as i32,
                MENU_W - 8,
                MENU_ITEM_H,
            ),
            icon,
        };
    }
    ContextMenuState { rect, items }
}

fn draw_context_menu(canvas: &mut Canvas, theme: &Theme, menu: &ContextMenuState) {
    canvas.fill_rounded_rect(menu.rect, 8, theme.panel);
    canvas.stroke_rounded_rect(menu.rect, 8, 1, theme.border);
    for (i, (label, _)) in MENU_LABELS.iter().enumerate() {
        let item = menu.items[i].rect;
        canvas.fill_rect(Rect::new(item.x, item.y, item.w, item.h), theme.panel_alt);
        if i == 0 {
            canvas.fill_rect(Rect::new(item.x, item.y, item.w, 1), theme.border);
        }
        if let Some(icon) = menu.items[i].icon {
            canvas.draw_tga_icon(&icon, Rect::new(item.x + 4, item.y + 2, 16, 16));
        }
        let tw = measure_text(label, FontRole::UiRegular).w;
        let tx = (item.x + 24).min(item.x + item.w as i32 - tw as i32);
        draw_text_vcenter(
            canvas,
            label,
            tx,
            item.y,
            item.h,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
    }
}

fn icon_at(icons: &[DesktopIcon], p: Point) -> Option<usize> {
    icons.iter().position(|icon| icon.rect.contains(p))
}

fn menu_action_at(menu: &ContextMenuState, p: Point) -> Option<ContextMenuAction> {
    menu.items
        .iter()
        .find(|item| item.rect.contains(p))
        .map(|item| item.action)
}

fn create_new_folder(desktop_dir: &str) {
    for n in 0..100u32 {
        let mut name = String::from("New Folder");
        if n > 0 {
            name.push(' ');
            let mut digits = [0u8; 10];
            let len = fmt_u32_ascii(n + 1, &mut digits);
            for &b in &digits[..len] {
                name.push(b as char);
            }
        }
        let path = join_path(desktop_dir, &name);
        if libc::stat(path.as_bytes()).is_ok() {
            continue;
        }
        if libc::mkdir(path.as_bytes(), 0o755).is_err() {
            debug_log("[VORTEX] new folder create failed\n");
        }
        return;
    }
}

fn create_new_text_file(desktop_dir: &str) {
    for n in 0..100u32 {
        let mut name = String::from("New Text File");
        if n > 0 {
            name.push(' ');
            let mut digits = [0u8; 10];
            let len = fmt_u32_ascii(n + 1, &mut digits);
            for &b in &digits[..len] {
                name.push(b as char);
            }
        }
        name.push_str(".txt");
        let path = join_path(desktop_dir, &name);
        if libc::stat(path.as_bytes()).is_ok() {
            continue;
        }
        if libc::create(path.as_bytes()).is_err() {
            debug_log("[VORTEX] new text file create failed\n");
        }
        return;
    }
}

fn fmt_u32_ascii(mut value: u32, out: &mut [u8; 10]) -> usize {
    if value == 0 {
        out[0] = b'0';
        return 1;
    }
    let mut rev = [0u8; 10];
    let mut n = 0usize;
    while value > 0 {
        rev[n] = b'0' + (value % 10) as u8;
        value /= 10;
        n += 1;
    }
    for i in 0..n {
        out[i] = rev[n - 1 - i];
    }
    n
}

fn copy_sanitized_ascii(bytes: &[u8], out: &mut [u8]) -> usize {
    let mut len = 0usize;
    for &b in bytes {
        match b {
            0 | b'\n' | b'\r' => break,
            0x20..=0x7e => {
                if len >= out.len() {
                    break;
                }
                out[len] = b;
                len += 1;
            }
            _ => {
                if len >= out.len() {
                    break;
                }
                out[len] = b'?';
                len += 1;
            }
        }
    }
    len
}

fn write_fallback_app_name(pid: u64, out: &mut [u8; RUNNING_NAME_BUF]) -> usize {
    let prefix = b"App-";
    let mut len = prefix.len();
    out[..len].copy_from_slice(prefix);
    let mut rev = [0u8; 20];
    let mut n = 0usize;
    let mut value = pid;
    if value == 0 {
        rev[n] = b'0';
        n += 1;
    } else {
        while value > 0 && n < rev.len() {
            rev[n] = b'0' + (value % 10) as u8;
            value /= 10;
            n += 1;
        }
    }
    for idx in (0..n).rev() {
        if len >= out.len() {
            break;
        }
        out[len] = rev[idx];
        len += 1;
    }
    len
}

fn normalize_icon_stem(name: &str) -> String {
    let mut stem = name.trim();
    if let Some(rest) = stem.strip_prefix("sunlight-") {
        stem = rest;
    } else if let Some(rest) = stem.strip_prefix("sunlight_") {
        stem = rest;
    }

    let mut out = String::with_capacity(stem.len());
    for ch in stem.chars() {
        let ch = ch.to_ascii_lowercase();
        match ch {
            'a'..='z' | '0'..='9' | '.' | '-' | '_' | '@' => out.push(ch),
            ' ' | '/' | '\\' | ':' | '+' | '=' | ',' | '(' | ')' | '[' | ']' => out.push('-'),
            _ => {}
        }
    }
    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

fn build_icon_resolution_key(proc_name: Option<&str>, display_name: &str) -> String {
    let mut key = normalize_icon_stem(display_name);
    if let Some(proc_name) = proc_name {
        let proc_key = normalize_icon_stem(proc_name);
        if !proc_key.is_empty() {
            if !key.is_empty() {
                key.push('|');
            }
            key.push_str(&proc_key);
        }
    }
    key
}

fn parse_toml_string(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if !trimmed.starts_with('"') {
        let bare = trimmed.split('#').next().unwrap_or("").trim();
        return (!bare.is_empty()).then(|| String::from(bare));
    }

    let mut out = String::new();
    let mut escape = false;
    for ch in trimmed[1..].chars() {
        if escape {
            out.push(match ch {
                'n' => '\n',
                't' => '\t',
                '"' => '"',
                '\\' => '\\',
                other => other,
            });
            escape = false;
            continue;
        }
        match ch {
            '\\' => escape = true,
            '"' => return Some(out),
            other => out.push(other),
        }
    }
    None
}

fn parse_toml_key(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with('"') {
        return parse_toml_string(trimmed);
    }
    Some(String::from(trimmed))
}

fn parse_desktop_icon_overrides(bytes: &[u8]) -> Vec<IconOverride> {
    let mut overrides = Vec::new();
    let Ok(text) = core::str::from_utf8(bytes) else {
        return overrides;
    };
    let mut in_icons = false;
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_icons = matches!(line, "[icons]" | "[taskbar.icons]" | "[desktop.icons]");
            continue;
        }
        if !in_icons {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let Some(key) = parse_toml_key(raw_key) else {
            continue;
        };
        let Some(icon_ref) = parse_toml_string(raw_value) else {
            continue;
        };
        let app_key = normalize_icon_stem(&key);
        if app_key.is_empty() || icon_ref.is_empty() {
            continue;
        }
        if let Some(existing) = overrides.iter_mut().find(|entry| entry.app_key == app_key) {
            existing.icon_ref.clear();
            existing.icon_ref.push_str(&icon_ref);
        } else {
            overrides.push(IconOverride { app_key, icon_ref });
        }
    }
    overrides
}

fn load_desktop_icon_overrides() -> Vec<IconOverride> {
    read_file_bytes(DESKTOP_CONFIG_PATH, 4096)
        .map(|bytes| parse_desktop_icon_overrides(&bytes))
        .unwrap_or_default()
}

fn icon_override_for<'a>(overrides: &'a [IconOverride], name: &str) -> Option<&'a str> {
    let key = normalize_icon_stem(name);
    if key.is_empty() {
        return None;
    }
    overrides
        .iter()
        .find(|entry| entry.app_key == key)
        .map(|entry| entry.icon_ref.as_str())
}

fn resolve_icon_ref(icon_ref: &str) -> Option<RunningIcon> {
    if icon_ref.starts_with('/') {
        return read_file_bytes(icon_ref.as_bytes(), 32 * 1024).map(RunningIcon::Runtime);
    }
    if let Some(bytes) = resolve_icon_bytes(icon_ref) {
        if let Ok(img) = TgaImage::parse(bytes) {
            return Some(RunningIcon::Static(img));
        }
    }
    try_load_runtime_icon(icon_ref).map(RunningIcon::Runtime)
}

fn resolve_icon_bytes(name: &str) -> Option<&'static [u8]> {
    let stem = normalize_icon_stem(name);
    match stem.as_str() {
        "terminal"
        | icon_name::TERMINAL
        | "xterm"
        | "konsole"
        | "alacritty"
        | "sunlight-terminal" => Some(ICON_TERMINAL_TGA),
        "calc" | "calculator" | icon_name::CALCULATOR | "kcalc" | "sunlight-calculator" => {
            Some(ICON_CALC_TGA)
        }
        "files"
        | "file-manager"
        | icon_name::FILE_MANAGER
        | "dolphin"
        | "org.kde.dolphin"
        | "nautilus"
        | "thunar"
        | "sunlight-files" => Some(ICON_FILES_TGA),
        "settings"
        | icon_name::SETTINGS
        | "systemsettings"
        | "system-settings"
        | "sunlight-settings" => Some(ICON_SETTINGS_TGA),
        "folder" | "home" | "folder-home" => Some(ICON_FOLDER_TGA),
        "computer" | "desktop-computer" => Some(ICON_COMPUTER_TGA),
        "drive" | icon_name::DRIVE | "disk" => Some(ICON_DRIVE_TGA),
        "sunlight-writer" | "writer" | "libreoffice-writer" => Some(ICON_WRITER_TGA),
        "text"
        | icon_name::TEXT_GENERIC
        | "editor"
        | "text-editor"
        | "accessories-text-editor"
        | "notes"
        | "sunlight-edit"
        | "sunlight-text"
        | "kate" => Some(ICON_TEXT_EDITOR_TGA),
        "calendar" | "sunlight-calendar" => Some(ICON_CALENDAR_TGA),
        "rappid-rabbit" | "rabbit" | "internet-web-browser" | "web-browser" => {
            Some(ICON_GENERIC_APP_TGA)
        }
        "sunlight-api-lab" | "api-lab" | "apifox" => Some(ICON_API_LAB_TGA),
        "runner" | "run" | "system-run" => Some(ICON_RUNNER_TGA),
        "tasks" | "task-manager" | "ksysguard" | "sunlight-tasks" => Some(ICON_TASKS_TGA),
        "bench" | "sunlight-bench" | "cpu-x" => Some(ICON_BENCH_TGA),
        "eyes" | "kmag" | "sunlight-eyes" => Some(ICON_EYES_TGA),
        "generic-app" | "app" | "application" | "applications-system" | "windows" => {
            Some(ICON_GENERIC_APP_TGA)
        }
        _ => None,
    }
}

fn is_generated_app_name(stem: &str) -> bool {
    stem.strip_prefix("app-")
        .map(|suffix| !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()))
        .unwrap_or(false)
}

fn try_load_runtime_icon(name: &str) -> Option<Vec<u8>> {
    let stem = normalize_icon_stem(name);
    if stem.is_empty() || is_generated_app_name(&stem) {
        return None;
    }

    let mut path = [0u8; 160];
    let candidates = [name, stem.as_str()];
    let categories = [
        icon_category::APPS,
        icon_category::PLACES,
        icon_category::MIMETYPES,
        icon_category::DEVICES,
        icon_category::PREFERENCES,
        icon_category::ACTIONS,
    ];
    let sizes = [48u32, 32, 16];

    for candidate in candidates {
        for &category in &categories {
            for &size in &sizes {
                let len = icon_theme::icon_path(category, size, candidate, &mut path);
                if len == 0 {
                    continue;
                }
                if let Some(bytes) = read_file_bytes(&path[..len], 32 * 1024) {
                    if !bytes.is_empty() {
                        return Some(bytes);
                    }
                }
            }
        }
    }
    None
}

fn resolve_running_icon(
    proc_name: Option<&str>,
    display_name: &str,
    overrides: &[IconOverride],
) -> Option<RunningIcon> {
    for candidate in [proc_name, Some(display_name)].into_iter().flatten() {
        if let Some(icon_ref) = icon_override_for(overrides, candidate) {
            if let Some(icon) = resolve_icon_ref(icon_ref) {
                return Some(icon);
            }
        }
    }

    for candidate in [proc_name, Some(display_name)].into_iter().flatten() {
        if let Some(bytes) = resolve_icon_bytes(candidate) {
            if let Ok(img) = TgaImage::parse(bytes) {
                return Some(RunningIcon::Static(img));
            }
        }
    }

    for candidate in [proc_name, Some(display_name)].into_iter().flatten() {
        if let Some(bytes) = try_load_runtime_icon(candidate) {
            return Some(RunningIcon::Runtime(bytes));
        }
    }

    if let Some(bytes) = resolve_icon_bytes("generic-app") {
        if let Ok(img) = TgaImage::parse(bytes) {
            return Some(RunningIcon::Static(img));
        }
    }
    if let Some(bytes) = try_load_runtime_icon("generic-app") {
        return Some(RunningIcon::Runtime(bytes));
    }
    Some(RunningIcon::Missing)
}

// ---------------------------------------------------------------------------
// Bottom bar layout
// ---------------------------------------------------------------------------

/// Compute y coordinate of the top of the bottom clusters.
pub(crate) fn bot_y(screen_h: u32) -> i32 {
    screen_h as i32 - BOT_Y_OFF - BOT_H as i32
}

fn dock_cluster_width(count: usize) -> u32 {
    if count == 0 {
        return CLUSTER_PAD as u32 * 2;
    }
    CLUSTER_PAD as u32 * 2
        + count as u32 * ICON_BTN
        + (count.saturating_sub(1) as u32) * ICON_GAP as u32
}

/// Draw the bottom-left cluster: overview | sidebar | settings.
/// Returns the settings button rect for click-zone registration.
fn draw_bot_left(
    canvas: &mut Canvas,
    theme: &Theme,
    by: i32,
    dock: &DockTheme,
    settings_app: &DockAppState,
    settings_hover: bool,
    now: u64,
) -> Rect {
    let icons: &[&[u16; 16]] = &[&OVERVIEW_ROWS, &SIDEBAR_ROWS, &SETTINGS_ROWS];
    let cluster_w = dock_cluster_width(icons.len());
    let cluster = Rect::new(TOP_PAD, by, cluster_w, BOT_H);
    draw_panel(canvas, cluster, theme.panel, theme.border);

    let mut cx = cluster.x + CLUSTER_PAD;
    let mut settings_cell = Rect::new(0, 0, 0, 0);
    for (i, rows) in icons.iter().enumerate() {
        let cell = Rect::new(
            cx,
            cluster.y + (BOT_H as i32 - ICON_BTN as i32) / 2,
            ICON_BTN,
            ICON_BTN,
        );
        // Slot 2 = settings icon — use TGA if available.
        if i == 2 {
            settings_cell = cell;
            draw_app_button(
                canvas,
                cell,
                theme,
                dock,
                rows,
                settings_app,
                settings_hover,
                now,
            );
        } else {
            draw_icon_btn(canvas, cell, rows, theme, false, false);
        }
        cx += ICON_BTN as i32 + ICON_GAP;
    }
    settings_cell
}

/// Draw the bottom-center dock and return `(launcher_rect, [terminal, tasks,
/// calc, files])`. `launcher_rect` is the grid icon that toggles the Start
/// Menu; `menu_open` draws it in an active/highlighted state.
fn draw_bot_center(
    canvas: &mut Canvas,
    theme: &Theme,
    by: i32,
    screen_w: u32,
    hover: Option<usize>,
    running_hover: Option<usize>,
    dock: DockTheme,
    sym: SymbolTheme,
    terminal_app: &DockAppState,
    calendar_app: &DockAppState,
    calc_app: &DockAppState,
    files_app: &DockAppState,
    running_apps: &[RunningAppEntry],
    running_zones: &mut Vec<(Rect, u64)>,
    menu_open: bool,
    _rtl: bool,
    now: u64,
) -> (Rect, [Rect; 4]) {
    let icons: &[&[u16; 16]; 5] = &[
        &GRID_ROWS,
        &TERMINAL_ROWS,
        &CALENDAR_ROWS,
        &CALC_ROWS,
        &FOLDER_ROWS,
    ];
    let fixed_w = dock_cluster_width(icons.len());
    let mut running_total_w = 0u32;
    for entry in running_apps {
        running_total_w = running_total_w.saturating_add(entry.cell_w);
    }
    if !running_apps.is_empty() {
        running_total_w = running_total_w.saturating_add(
            ICON_GAP as u32
                + (running_apps.len().saturating_sub(1) as u32) * ICON_GAP as u32
                + CLUSTER_PAD as u32,
        );
    }
    let total_w = fixed_w.saturating_add(running_total_w);
    let min_x = TOP_PAD + dock_cluster_width(3) as i32 + 8;
    let max_x = screen_w as i32 - TOP_PAD - SEARCH_W as i32 - 8 - total_w as i32;
    let cx_start = ((screen_w as i32 - total_w as i32) / 2).clamp(min_x, max_x.max(min_x));
    let cluster = Rect::new(cx_start, by, total_w, BOT_H);
    draw_panel(canvas, cluster, theme.panel, theme.border);

    let mut x = cluster.x + CLUSTER_PAD;
    let mut clickable = [Rect::new(0, 0, 0, 0); 4];
    let mut launcher_rect = Rect::new(0, 0, 0, 0);
    for (i, rows) in icons.iter().enumerate() {
        let cell = Rect::new(
            x,
            cluster.y + (BOT_H as i32 - ICON_BTN as i32) / 2,
            ICON_BTN,
            ICON_BTN,
        );
        let is_hover = hover
            .map(|h| h == i.saturating_sub(1) && i > 0)
            .unwrap_or(false);

        match i {
            0 => {
                launcher_rect = cell;
                if menu_open || is_hover {
                    canvas.fill_rounded_rect(cell, 5, theme.panel_alt);
                }
                if menu_open {
                    canvas.stroke_rounded_rect(cell, 5, 1, theme.accent);
                }
                // Use Material Symbols start/menu glyph in orange accent (rich app icons kept for others)
                if let Some(tga) = sym.start.or(sym.menu) {
                    draw_tga_tinted_orange(canvas, &tga, cell.inset(4), theme.accent);
                } else {
                    draw_icon_btn(canvas, cell, rows, theme, false, false);
                }
            }
            1 => draw_app_button(
                canvas,
                cell,
                theme,
                &dock,
                rows,
                terminal_app,
                is_hover,
                now,
            ),
            2 => draw_app_button(
                canvas,
                cell,
                theme,
                &dock,
                rows,
                calendar_app,
                is_hover,
                now,
            ),
            3 => draw_app_button(canvas, cell, theme, &dock, rows, calc_app, is_hover, now),
            4 => draw_app_button(canvas, cell, theme, &dock, rows, files_app, is_hover, now),
            _ => {
                if let Some(tga) = dock.dock_icon(i) {
                    if is_hover {
                        canvas.fill_rounded_rect(cell, 5, theme.panel_alt);
                    }
                    // Inset 2px for visual padding inside the button cell.
                    let icon_area = cell.inset(2);
                    canvas.draw_tga_icon(&tga, icon_area);
                } else {
                    draw_icon_btn(canvas, cell, rows, theme, false, is_hover);
                }
            }
        }

        // Icon 0 (grid) toggles the Start Menu — see `launcher_rect` above.
        // Icons 1,2,3 are clickable (terminal, tasks, calc); icon 4 is Files.
        if i >= 1 {
            clickable[i - 1] = cell;
        }
        if i + 1 < icons.len() {
            x += ICON_BTN as i32 + ICON_GAP;
        } else {
            x += ICON_BTN as i32;
        }
    }
    if !running_apps.is_empty() {
        x += ICON_GAP as i32;
        for (i, entry) in running_apps.iter().enumerate() {
            let cell_w = entry.cell_w;
            let cell = Rect::new(
                x,
                cluster.y + (BOT_H as i32 - ICON_BTN as i32) / 2,
                cell_w,
                ICON_BTN,
            );
            draw_running_app_button(
                canvas,
                cell,
                theme,
                entry,
                running_hover.map_or(false, |h| h == i),
                now,
            );
            if let Some(zone) = running_zones.get_mut(i) {
                *zone = (cell, entry.win_id);
            } else {
                running_zones.push((cell, entry.win_id));
            }
            if i + 1 < running_apps.len() {
                x += cell_w as i32 + ICON_GAP as i32;
            } else {
                x += cell_w as i32;
            }
        }
    }
    running_zones.truncate(running_apps.len());
    (launcher_rect, clickable)
}

impl VortexShell {
    fn draw_datetime_tooltip(&mut self, canvas: &mut Canvas, theme: &Theme, _cw: u32, _ch: u32) {
        // Build lines with safe fallbacks
        let loc_str = core::str::from_utf8(&self.locale[..self.locale_len.min(47)])
            .unwrap_or("C.UTF-8")
            .trim_end_matches('\0');
        let tz_str = if self.tz_id_len > 0 {
            core::str::from_utf8(&self.tz_id[..self.tz_id_len.min(47)])
                .unwrap_or("UTC")
                .trim_end_matches('\0')
        } else {
            "UTC"
        };
        let tz_disp = if tz_str.eq_ignore_ascii_case("utc") || tz_str.is_empty() {
            "UTC"
        } else {
            tz_str
        };

        // long form using locale helpers
        let dt = sunlight_locale::SimpleDateTime {
            year: self.status_year as i32,
            month: self.status_month,
            day: self.status_day,
            hour: self.status_hour,
            minute: self.status_min,
            second: self.status_sec,
            weekday_iso: 0,
        };
        let long_date = sunlight_locale::format_long_date(&dt, loc_str);
        let long_time = alloc::format!(
            "{:02}:{:02}:{:02} {}",
            self.status_hour,
            self.status_min,
            self.status_sec,
            if self.status_hour >= 12 { "PM" } else { "AM" }
        );
        let line1 = alloc::format!("{} {}", long_date, long_time);

        let l2 = alloc::format!("Timezone: {}", tz_disp);
        let l3 = alloc::format!("Locale: {}", loc_str);
        let l4 = alloc::format!("LC_TIME: {}", loc_str);

        let lines: [&str; 4] = [&line1, &l2, &l3, &l4];

        let mut w = 0u32;
        for &ln in &lines {
            w = w.max(measure_text(ln, FontRole::UiSmall).w);
        }
        w = w.saturating_add(16);
        let h = 18 * 4 + 10;
        let x = (self.datetime_zone.x + self.datetime_zone.w as i32 / 2 - w as i32 / 2).max(8);
        let y = self.datetime_zone.bottom() + 4;

        let r = Rect::new(x, y, w, h as u32);
        canvas.fill_rounded_rect(r, 6, theme.panel);
        canvas.stroke_rounded_rect(r, 6, 1, theme.border);

        let mut ty = y + 6;
        for &ln in &lines {
            draw_text_vcenter(
                canvas,
                ln,
                x + 8,
                ty,
                16,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
            ty += 16;
        }
    }

    /// Draws the full (untruncated) window title above a running-app cell
    /// after the pointer has dwelt on it. Mirrors the datetime tooltip style
    /// but single-line and anchored to the hovered cell rather than the bar.
    fn draw_running_tooltip(&self, canvas: &mut Canvas, theme: &Theme, idx: usize, cw: u32) {
        let Some(&(cell, _win_id)) = self.running_zones.get(idx) else {
            return;
        };
        let Some(entry) = self.running_apps.get(idx) else {
            return;
        };
        let title = entry.display_name.as_str();
        if title.is_empty() {
            return;
        }

        let pad = 4i32;
        let text_w = measure_text(title, FontRole::UiSmall).w as i32;
        let w = text_w + pad * 2;
        let h = 18u32;

        // Centered on the cell, 8px above its top edge.
        let cx = cell.x + cell.w as i32 / 2;
        let mut x = cx - w / 2;
        let y = (cell.y - h as i32 - 8).max(2);

        // Clamp horizontally into the screen so long titles stay on-screen.
        if x < 2 {
            x = 2;
        } else if x + w > cw as i32 - 2 {
            x = cw as i32 - w - 2;
        }

        let r = Rect::new(x, y, w as u32, h);
        canvas.fill_rounded_rect(r, 2, Color::rgb(0x2a, 0x2a, 0x2a));
        canvas.stroke_rounded_rect(r, 2, 1, theme.border);
        draw_text_vcenter(
            canvas,
            title,
            x + pad,
            y,
            h,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );
    }

    fn draw_calendar_popover(&mut self, canvas: &mut Canvas, theme: &Theme, cw: u32, _ch: u32) {
        self.refresh_calendar_popover_data();
        let panel = self.calendar_popover_rect(cw);
        let x = panel.x;
        let y = panel.y;
        let pw = panel.w;
        canvas.fill_rounded_rect(panel, 8, theme.panel);
        canvas.stroke_rounded_rect(panel, 8, 1, theme.border);

        let mon_name = sunlight_locale::month_name(self.cal_view_month, true, "en_US.UTF-8");
        let header = alloc::format!("{} {}", mon_name, self.cal_view_year);
        draw_text_vcenter(
            canvas,
            &header,
            x + 12,
            y + 4,
            20,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        let mut hx = x + 12;
        let cell = 38u32;
        for &wd in &["S", "M", "T", "W", "T", "F", "S"] {
            draw_text_vcenter(
                canvas,
                wd,
                hx + 12,
                y + 26,
                14,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
            hx += cell as i32;
        }

        let grid_y = y + 44;
        let offset = cal_weekday_sun0(self.cal_view_year, self.cal_view_month, 1);
        let dim = cal_days_in_month(self.cal_view_year, self.cal_view_month);
        let today_d =
            if self.cal_view_month == self.status_month && self.cal_view_year == self.status_year {
                self.status_day
            } else {
                0
            };
        for idx in 0..CAL_POPUP_DAYS {
            let row = idx / 7;
            let col = idx % 7;
            let gx = x + 12 + col as i32 * cell as i32;
            let cell_r = Rect::new(gx, grid_y + row as i32 * 22, cell - 4, 20);
            if idx < offset {
                continue;
            }
            let day = (idx - offset + 1) as u8;
            if day > dim {
                continue;
            }
            let is_selected = day == self.cal_selected_day;
            let is_today = day == today_d;
            if is_selected {
                canvas.fill_rounded_rect(cell_r, 4, theme.accent);
            } else if is_today {
                canvas.fill_rounded_rect(cell_r, 4, theme.panel_alt);
                canvas.stroke_rounded_rect(cell_r, 4, 1, theme.accent);
            }
            let s = alloc::format!("{}", day);
            draw_text_vcenter(
                canvas,
                &s,
                gx + 7,
                cell_r.y,
                cell_r.h,
                &TextStyle::new(
                    FontRole::UiSmall,
                    if is_selected {
                        theme.text_on_accent
                    } else {
                        theme.text
                    },
                ),
            );
            if self.cal_event_days[idx] {
                canvas.fill_rounded_rect(
                    Rect::new(cell_r.right() - 8, cell_r.bottom() - 6, 4, 4),
                    2,
                    if is_selected {
                        theme.text_on_accent
                    } else {
                        theme.accent
                    },
                );
            }
        }

        let list_y = y + 184;
        canvas.hbar(x + 12, list_y - 8, pw - 24, 1, theme.border);
        let selected_date = format_cal_date(
            self.cal_view_year,
            self.cal_view_month,
            self.cal_selected_day,
        );
        draw_text_vcenter(
            canvas,
            &selected_date,
            x + 12,
            list_y - 4,
            18,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );

        let mut item_y = list_y + 18;
        draw_text_vcenter(
            canvas,
            "Events",
            x + 12,
            item_y,
            16,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );
        item_y += 16;
        if self.cal_selected_events.is_empty() {
            draw_text_vcenter(
                canvas,
                "No events for this day",
                x + 12,
                item_y,
                18,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
            item_y += 20;
        } else {
            for event in self.cal_selected_events.iter().take(3) {
                let row = Rect::new(x + 12, item_y, pw - 24, 20);
                canvas.fill_rounded_rect(row, 4, theme.panel_alt);
                draw_text_vcenter(
                    canvas,
                    &event.time,
                    row.x + 6,
                    row.y,
                    row.h,
                    &TextStyle::new(FontRole::UiSmall, theme.accent),
                );
                draw_text_vcenter(
                    canvas,
                    &event.title,
                    row.x + 62,
                    row.y,
                    row.h,
                    &TextStyle::new(FontRole::UiSmall, theme.text),
                );
                item_y += 22;
            }
            if self.cal_selected_events.len() > 3 {
                draw_text_vcenter(
                    canvas,
                    "More in Calendar…",
                    x + 12,
                    item_y,
                    18,
                    &TextStyle::new(FontRole::UiSmall, theme.text_muted),
                );
                item_y += 18;
            }
        }

        item_y += 2;
        draw_text_vcenter(
            canvas,
            "Tasks",
            x + 12,
            item_y,
            16,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );
        item_y += 16;
        if self.cal_selected_tasks.is_empty() {
            draw_text_vcenter(
                canvas,
                "No tasks for this day",
                x + 12,
                item_y,
                18,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
            item_y += 20;
        } else {
            for task in self.cal_selected_tasks.iter().take(CAL_POPUP_TASKS) {
                let row = Rect::new(x + 12, item_y, pw - 24, 20);
                canvas.fill_rounded_rect(row, 4, theme.panel_alt);
                let marker = if task.status == sunlight_reminders::TaskStatus::Done {
                    "[x]"
                } else {
                    "[ ]"
                };
                draw_text_vcenter(
                    canvas,
                    marker,
                    row.x + 6,
                    row.y,
                    row.h,
                    &TextStyle::new(FontRole::UiSmall, theme.accent),
                );
                let mut title = task.title.clone();
                if title.chars().count() > 28 {
                    title = ellipsize_label(&title, 28);
                }
                draw_text_vcenter(
                    canvas,
                    &title,
                    row.x + 34,
                    row.y,
                    row.h,
                    &TextStyle::new(FontRole::UiSmall, theme.text),
                );
                if !task.due_time.is_empty() {
                    draw_text_vcenter(
                        canvas,
                        &task.due_time,
                        row.right() - 42,
                        row.y,
                        row.h,
                        &TextStyle::new(FontRole::UiSmall, theme.text_muted),
                    );
                }
                item_y += 22;
            }
        }

        item_y += 2;
        draw_text_vcenter(
            canvas,
            "Reminders",
            x + 12,
            item_y,
            16,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );
        item_y += 16;
        if self.cal_selected_reminders.is_empty() {
            draw_text_vcenter(
                canvas,
                "No reminders",
                x + 12,
                item_y,
                18,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
        } else {
            for reminder in self.cal_selected_reminders.iter().take(CAL_POPUP_REMINDERS) {
                let row = Rect::new(x + 12, item_y, pw - 24, 20);
                canvas.fill_rounded_rect(row, 4, theme.panel_alt);
                let time = if reminder.reminder_time.is_empty() {
                    "--:--"
                } else {
                    reminder.reminder_time.as_str()
                };
                draw_text_vcenter(
                    canvas,
                    time,
                    row.x + 6,
                    row.y,
                    row.h,
                    &TextStyle::new(FontRole::UiSmall, theme.accent),
                );
                let mut title = reminder.title.clone();
                if title.chars().count() > 30 {
                    title = ellipsize_label(&title, 30);
                }
                draw_text_vcenter(
                    canvas,
                    &title,
                    row.x + 52,
                    row.y,
                    row.h,
                    &TextStyle::new(FontRole::UiSmall, theme.text),
                );
                item_y += 22;
            }
        }

        let btn_y = panel.bottom() - 26;
        let btn_r = Rect::new(x + 12, btn_y, pw - 24, 20);
        self.cal_popup_open_btn = btn_r;
        canvas.fill_rounded_rect(btn_r, 4, theme.accent);
        draw_text_vcenter(
            canvas,
            "Open Calendar",
            btn_r.x
                + ((btn_r.w as i32 - measure_text("Open Calendar", FontRole::UiSmall).w as i32)
                    / 2),
            btn_r.y,
            btn_r.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_on_accent),
        );
    }

    fn calendar_popover_rect(&self, cw: u32) -> Rect {
        let pw = 300u32;
        let ph = 310u32;
        let cx = self.datetime_zone.x + (self.datetime_zone.w as i32) / 2;
        let x = (cx - (pw as i32) / 2)
            .max(TOP_PAD)
            .min((cw - pw - TOP_PAD as u32) as i32);
        Rect::new(x, self.datetime_zone.bottom() + 2, pw, ph)
    }

    fn calendar_day_at_point(&self, point: Point, cw: u32) -> Option<u8> {
        let panel = self.calendar_popover_rect(cw);
        let grid_x = panel.x + 12;
        let grid_y = panel.y + 44;
        if point.x < grid_x || point.y < grid_y {
            return None;
        }
        let col = (point.x - grid_x) / 38;
        let row = (point.y - grid_y) / 22;
        if col < 0 || col >= 7 || row < 0 || row >= 6 {
            return None;
        }
        let idx = row as usize * 7 + col as usize;
        let offset = cal_weekday_sun0(self.cal_view_year, self.cal_view_month, 1);
        if idx < offset {
            return None;
        }
        let day = (idx - offset + 1) as u8;
        if day == 0 || day > cal_days_in_month(self.cal_view_year, self.cal_view_month) {
            None
        } else {
            Some(day)
        }
    }

    fn draw_notif_panel(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let pw = NOTIF_CENTER_W.min(canvas.width.saturating_sub(24));
        let ph = canvas.height.saturating_sub(72).clamp(180, 520);
        let x = canvas.width as i32 - pw as i32 - 12;
        let y = TOP_Y + TOP_H as i32 + 8;
        let panel = Rect::new(x, y, pw, ph);
        canvas.fill_rounded_rect(panel, 10, theme.panel);
        canvas.stroke_rounded_rect(panel, 10, 1, theme.border);

        let header_icon_r = Rect::new(panel.x + 14, panel.y + 11, 18, 18);
        if let Some(icon) = self.symbols.notifications {
            canvas.draw_tga_icon_tinted(&icon, header_icon_r, theme.accent);
        }
        draw_text_vcenter(
            canvas,
            "Notifications",
            panel.x + 38,
            panel.y + 10,
            22,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        let dnd_on = notification_dnd_enabled();
        self.notif_dnd_toggle_r = Rect::new(panel.right() - 98, panel.y + 10, 82, 22);
        canvas.fill_rounded_rect(
            self.notif_dnd_toggle_r,
            6,
            if dnd_on { theme.warn } else { theme.panel_alt },
        );
        canvas.stroke_rounded_rect(
            self.notif_dnd_toggle_r,
            6,
            1,
            if dnd_on {
                theme.warn.darken(40)
            } else {
                theme.border
            },
        );
        let dnd_icon_r = Rect::new(
            self.notif_dnd_toggle_r.x + 7,
            self.notif_dnd_toggle_r.y + 3,
            16,
            16,
        );
        let dnd_icon = if dnd_on {
            self.symbols.dnd_on
        } else {
            self.symbols.dnd_off
        };
        if let Some(icon) = dnd_icon {
            canvas.draw_tga_icon_tinted(
                &icon,
                dnd_icon_r,
                if dnd_on { theme.text } else { theme.icon_muted },
            );
        }
        draw_text_vcenter(
            canvas,
            if dnd_on { "DND On" } else { "DND Off" },
            self.notif_dnd_toggle_r.x + 26,
            self.notif_dnd_toggle_r.y,
            self.notif_dnd_toggle_r.h,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );

        let records = notification_history_recent(NOTIF_CENTER_RECENT_LIMIT, false);
        self.notif_mark_seen_r = Rect::new(0, 0, 0, 0);
        self.notif_dismiss_r = Rect::new(0, 0, 0, 0);
        self.notif_dismiss_zones.clear();

        if records.is_empty() {
            draw_text_vcenter(
                canvas,
                "No notifications yet.",
                panel.x + 14,
                panel.y + 52,
                18,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
            return;
        }

        let mut cy = panel.y + 44;
        let bottom = panel.bottom() - 12;
        let mut rendered_groups: Vec<String> = Vec::new();
        for group_record in &records {
            if cy + 42 > bottom {
                draw_text_vcenter(
                    canvas,
                    "More in history...",
                    panel.x + 14,
                    cy,
                    18,
                    &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                );
                break;
            }
            let group = notification_group_label(group_record);
            if rendered_groups.iter().any(|existing| existing == &group) {
                continue;
            }
            rendered_groups.push(group.clone());
            if rendered_groups.len() > 1 {
                cy += 8;
            }
            let group_icon_r = Rect::new(panel.x + 14, cy + 1, 16, 16);
            if let Some(icon) = self.symbols.settings {
                canvas.draw_tga_icon_tinted(&icon, group_icon_r, theme.text_dim.darken(20));
            }
            let label = ellipsize_label(&group, 30);
            draw_text_vcenter(
                canvas,
                &label,
                panel.x + 34,
                cy,
                18,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim.darken(10)),
            );
            cy += 22;

            for record in records
                .iter()
                .filter(|record| notification_group_label(record) == group)
            {
                let card_h = 70u32;
                if cy + card_h as i32 > bottom {
                    break;
                }
                let card = Rect::new(panel.x + 10, cy, panel.w - 20, card_h);
                let fill = if record.seen {
                    theme.panel_alt
                } else {
                    theme.accent.darken(150)
                };
                canvas.fill_rounded_rect(card, 8, fill);
                canvas.stroke_rounded_rect(
                    card,
                    8,
                    1,
                    notification_priority_color(record.priority),
                );
                let close_r = Rect::new(card.right() - 26, card.y + 8, 16, 16);
                if self.notif_dismiss_zones.len() < NOTIF_DISMISS_ZONES_MAX {
                    self.notif_dismiss_zones
                        .push((close_r, record.storage_key.clone()));
                }
                let title = ellipsize_label(&record.title, 30);
                let body = ellipsize_label(&record.body, 44);
                let meta = notification_meta(record);
                draw_text_vcenter(
                    canvas,
                    &title,
                    card.x + 12,
                    card.y + 8,
                    16,
                    &TextStyle::new(FontRole::UiSmall, theme.text),
                );
                draw_text_vcenter(
                    canvas,
                    &body,
                    card.x + 12,
                    card.y + 30,
                    16,
                    &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                );
                draw_text_vcenter(
                    canvas,
                    &meta,
                    card.x + 12,
                    card.y + 50,
                    14,
                    &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                );
                canvas.fill_rounded_rect(close_r, 4, theme.panel);
                canvas.stroke_rounded_rect(close_r, 4, 1, theme.border);
                if let Some(icon) = self.symbols.close {
                    canvas.draw_tga_icon_tinted(&icon, close_r, theme.text_dim);
                } else {
                    draw_text_vcenter(
                        canvas,
                        "x",
                        close_r.x + 5,
                        close_r.y,
                        close_r.h,
                        &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                    );
                }
                cy += card_h as i32 + 10;
            }
        }
    }

    fn draw_logout_confirm(&mut self, canvas: &mut Canvas, theme: &Theme, _cw: u32, _ch: u32) {
        let dw = 240u32;
        let dh = 80u32;
        let x = (self.screen_w / 2) as i32 - (dw / 2) as i32;
        let y = (self.screen_h / 2) as i32 - (dh / 2) as i32;
        let panel = Rect::new(x, y, dw, dh);
        canvas.fill_rounded_rect(panel, 8, theme.panel);
        canvas.stroke_rounded_rect(panel, 8, 1, theme.border);

        draw_text_vcenter(
            canvas,
            "Log out of SunlightOS?",
            x + 12,
            y + 8,
            18,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        let bw = 90u32;
        let bh = 22u32;
        let by = y + dh as i32 - 30;
        let cancel_r = Rect::new(x + 12, by, bw, bh);
        let logout_r = Rect::new(x + dw as i32 - 12 - bw as i32, by, bw, bh);

        canvas.fill_rounded_rect(cancel_r, 4, theme.panel_alt);
        draw_text_vcenter(
            canvas,
            "Cancel",
            cancel_r.x + 8,
            cancel_r.y,
            bh,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );

        canvas.fill_rounded_rect(logout_r, 4, theme.warn);
        draw_text_vcenter(
            canvas,
            "Log Out",
            logout_r.x + 8,
            logout_r.y,
            bh,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );

        self.logout_cancel_r = cancel_r;
        self.logout_confirm_r = logout_r;
    }
} // end impl VortexShell

// (stash fields live in VortexShell)

/// Draw the bottom-right search box.
fn draw_bot_right(canvas: &mut Canvas, theme: &Theme, by: i32, screen_w: u32, sym: SymbolTheme) {
    let sx = screen_w as i32 - TOP_PAD - SEARCH_W as i32;
    let sy = by + (BOT_H as i32 - SEARCH_H as i32) / 2;
    let search_rect = Rect::new(sx, sy, SEARCH_W, SEARCH_H);
    draw_panel(canvas, search_rect, theme.panel_alt, theme.border);

    // Search glyph icon on left
    let ic = 14u32;
    let icell = Rect::new(sx + 6, sy + (SEARCH_H as i32 - ic as i32) / 2, ic, ic);
    if let Some(tga) = sym.search {
        draw_tga_tinted_orange(canvas, &tga, icell, theme.text_dim);
    }

    // Placeholder text (indented for icon)
    let ph = "Search...";
    let ph_w = measure_text(ph, FontRole::UiSmall).w;
    let ph_x = sx + 24;
    draw_text_vcenter(
        canvas,
        ph,
        ph_x,
        search_rect.y,
        search_rect.h,
        &TextStyle::new(FontRole::UiSmall, theme.text_dim),
    );
}

// ---------------------------------------------------------------------------
// App impl
// ---------------------------------------------------------------------------

impl App for VortexShell {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        if self.status_min == 0xff {
            let _ = self.refresh_status();
        }

        let now = monotonic_millis();
        if now >= self.next_app_poll_ms {
            let _ = self.sync_app_registry(now, false);
        }

        let cw = canvas.width;
        let ch = canvas.height;
        self.maybe_reload_wallpaper(now, cw, ch);
        self.screen_w = cw;
        self.screen_h = ch;

        // ── Wallpaper ────────────────────────────────────────────────────────
        canvas.fill_rect(Rect::new(0, 0, cw, ch), Color(FALLBACK_BG));
        if let Some(ref wp) = self.wallpaper {
            canvas.draw_image_cover(wp);
        }

        let desktop_rect = desktop_area(cw, ch);
        layout_desktop_icons(&mut self.desktop_icons, desktop_rect);
        draw_desktop_icons(
            canvas,
            theme,
            &self.desktop_icons,
            &self.selected_icons,
            self.desktop_theme,
        );
        if let Some(rect) = self.desktop_selection_rect() {
            draw_desktop_marquee(canvas, theme, rect);
        }

        // ── Top bar ──────────────────────────────────────────────────────────
        draw_top_bar(canvas, theme, cw, self);
        // power_zone kept for compat (may be unused now that logout is primary)
        self.power_zone = self.logout_zone; // conservative alias for old code paths

        // Date/time tooltip (hover, long info, safe fallbacks)
        if self.show_datetime_tooltip && !self.show_calendar_popover {
            self.draw_datetime_tooltip(canvas, theme, cw, ch);
        }

        // Running-app title tooltip: only after a dwell and never while the
        // user is dragging out a desktop selection (the pointer is busy).
        if self.selection_state == DesktopSelectState::Idle {
            if let Some(idx) = self.running_hover {
                if let Some(start) = self.running_hover_since {
                    if now.saturating_sub(start) >= RUNNING_TOOLTIP_DELAY_MS {
                        self.draw_running_tooltip(canvas, theme, idx, cw);
                    }
                }
            }
        }

        // Calendar popover (small, under center)
        if self.show_calendar_popover {
            self.draw_calendar_popover(canvas, theme, cw, ch);
        }

        // Notif placeholder panel
        if self.show_notif_panel {
            self.draw_notif_panel(canvas, theme);
        }

        // Logout confirm dialog
        if self.show_logout_confirm {
            self.draw_logout_confirm(canvas, theme, cw, ch);
        }

        // ── Bottom panels ────────────────────────────────────────────────────
        let by = bot_y(ch);
        let dock_theme = self.dock_theme;
        let settings_app = *self.app(AppId::Settings);
        let terminal_app = *self.app(AppId::Terminal);
        let calendar_app = *self.app(AppId::Calendar);
        let calc_app = *self.app(AppId::Calculator);
        let files_app = *self.app(AppId::Files);
        self.settings_zone = draw_bot_left(
            canvas,
            theme,
            by,
            &dock_theme,
            &settings_app,
            self.settings_hover,
            now,
        );
        self.running_zones.clear();
        let running_apps: &[RunningAppEntry] = if ENABLE_RUNNING_TASKBAR {
            &self.running_apps
        } else {
            &[]
        };
        let rtl = self.is_rtl_locale();
        let (launcher_rect, dock_cells) = draw_bot_center(
            canvas,
            theme,
            by,
            cw,
            self.hover,
            self.running_hover,
            dock_theme,
            self.symbols,
            &terminal_app,
            &calendar_app,
            &calc_app,
            &files_app,
            running_apps,
            &mut self.running_zones,
            self.start_menu.is_open(),
            rtl,
            now,
        );
        self.launcher_zone = launcher_rect;
        draw_bot_right(canvas, theme, by, cw, self.symbols);

        // Record clickable zones (terminal, tasks, calc, files).
        self.dock_zones = [
            (dock_cells[0], Self::dock_zone_app(0)),
            (dock_cells[1], Self::dock_zone_app(1)), // calendar
            (dock_cells[2], Self::dock_zone_app(2)),
            (dock_cells[3], Self::dock_zone_app(3)),
        ];

        self.start_menu
            .view(canvas, theme, cw, ch, &self.apps, &self.recent_apps, now);

        if let Some(menu) = &self.context_menu {
            draw_context_menu(canvas, theme, menu);
        }
    }

    fn update(&mut self, event: Event) -> bool {
        // The Start Menu owns all interactive input while open (click,
        // mouse move/hover, keyboard search/nav). `Event::Tick` deliberately
        // falls through below so background app-state polling keeps the
        // menu's running-app badges fresh even while it's open.
        if self.start_menu.is_open() {
            match event {
                Event::Click { .. }
                | Event::MouseDown { .. }
                | Event::MouseUp { .. }
                | Event::MouseMove { .. }
                | Event::Key(_)
                | Event::KeyPress { .. } => {
                    let now = monotonic_millis();
                    let (dirty, action) = self.start_menu.handle_event(
                        event,
                        self.screen_w,
                        self.screen_h,
                        &self.recent_apps,
                        now,
                    );
                    if let start_menu::StartMenuAction::DismissedOutside { x, y } = action {
                        if matches!(event, Event::MouseDown { .. }) {
                            self.suppress_next_click = true;
                        }
                        if self.launcher_zone.contains(Point::new(x, y)) {
                            self.suppress_launcher_open = true;
                        }
                    }
                    self.apply_start_menu_action(action, now);
                    return dirty;
                }
                _ => {}
            }
        }
        match event {
            Event::Click { x, y } => {
                let point = Point::new(x, y);
                if self.suppress_next_click {
                    self.suppress_next_click = false;
                    return true;
                }
                // A left-button release reaches us as `Click` (the display
                // library aliases left-up to Click). Finish any marquee gesture
                // armed on MouseDown here so the state machine returns to Idle:
                //   - Dragging: commit the final selection rect and consume the
                //     event — a drag must NOT be treated as a launch click.
                //   - Armed (pressed but never dragged past threshold): this is
                //     a plain click on empty desktop; reset and fall through to
                //     normal click handling below.
                match self.selection_state {
                    DesktopSelectState::Dragging { .. } => {
                        self.update_desktop_marquee(point);
                        self.end_selection_gesture();
                        return true;
                    }
                    DesktopSelectState::Armed { .. } => {
                        self.end_selection_gesture();
                    }
                    DesktopSelectState::Idle => {}
                }
                if let Some(menu) = self.context_menu.take() {
                    if let Some(action) = menu_action_at(&menu, point) {
                        match action {
                            ContextMenuAction::NewFolder => {
                                create_new_folder(&self.desktop_paths.desktop_dir);
                                self.reload_desktop_icons();
                            }
                            ContextMenuAction::NewTextFile => {
                                create_new_text_file(&self.desktop_paths.desktop_dir);
                                self.reload_desktop_icons();
                            }
                            ContextMenuAction::Refresh | ContextMenuAction::SortByName => {
                                self.reload_desktop_icons();
                            }
                            ContextMenuAction::OpenTerminalHere => {
                                let _ = self.handle_app_click(
                                    AppId::Terminal,
                                    monotonic_millis(),
                                    LaunchSource::Shortcut,
                                );
                            }
                            ContextMenuAction::WallpaperSettings => {
                                let trace = self.next_launch_trace(LaunchSource::Shortcut);
                                let request = sun_exec::LaunchRequest {
                                    trace,
                                    source: LaunchSource::Shortcut,
                                    command: b"settings",
                                    args: &[b"--page", b"wallpaper"],
                                    require_display: true,
                                };
                                let _ = sun_exec::launch(request);
                            }
                        }
                        return true;
                    }
                    return true;
                }
                // Handle "Open Calendar" button in popover
                if self.show_calendar_popover && self.cal_popup_open_btn.contains(point) {
                    self.show_calendar_popover = false;
                    return self.handle_app_click(
                        AppId::Calendar,
                        monotonic_millis(),
                        LaunchSource::Shortcut,
                    );
                }
                if self.show_calendar_popover {
                    if let Some(day) = self.calendar_day_at_point(point, self.screen_w) {
                        self.cal_selected_day = day;
                        self.cal_last_loaded_key_len = 0;
                        self.refresh_calendar_popover_data();
                        return true;
                    }
                    if self.calendar_popover_rect(self.screen_w).contains(point) {
                        return true;
                    }
                }
                if self.show_notif_panel {
                    if self.notif_dnd_toggle_r.contains(point) {
                        let next = !notification_dnd_enabled();
                        let _ = notification_set_dnd(next);
                        return true;
                    }
                    for (rect, key) in &self.notif_dismiss_zones {
                        if rect.contains(point) {
                            for record in
                                notification_history_recent(NOTIF_CENTER_RECENT_LIMIT, false)
                            {
                                if record.storage_key == *key {
                                    notification_set_dismissed(&record, true);
                                    break;
                                }
                            }
                            return true;
                        }
                    }
                    if self.notif_mark_seen_r.contains(point) {
                        if let Some(record) =
                            notification_history_recent(NOTIF_CENTER_RECENT_LIMIT, false)
                                .into_iter()
                                .next()
                        {
                            notification_set_seen(&record, true);
                        }
                        return true;
                    }
                    if self.notif_dismiss_r.contains(point) {
                        if let Some(record) =
                            notification_history_recent(NOTIF_CENTER_RECENT_LIMIT, false)
                                .into_iter()
                                .next()
                        {
                            notification_set_dismissed(&record, true);
                        }
                        return true;
                    }
                    let pw = NOTIF_CENTER_W.min(self.screen_w.saturating_sub(24));
                    let ph = self.screen_h.saturating_sub(72).clamp(180, 520);
                    let panel = Rect::new(
                        self.screen_w as i32 - pw as i32 - 12,
                        TOP_Y + TOP_H as i32 + 8,
                        pw,
                        ph,
                    );
                    if panel.contains(point) {
                        for record in notification_history_recent(NOTIF_CENTER_RECENT_LIMIT, false)
                        {
                            if !record.seen {
                                notification_set_seen(&record, true);
                            }
                        }
                        return true;
                    }
                }
                // Close transient panels on outside click (conservative)
                let mut closed = false;
                if self.show_calendar_popover && !self.datetime_zone.contains(point) {
                    self.show_calendar_popover = false;
                    closed = true;
                }
                if self.show_notif_panel {
                    self.show_notif_panel = false;
                    closed = true;
                }
                if self.show_logout_confirm && !/*simple*/ false {
                    // keep open until buttons; but if click far we'll close below in draw click
                }
                if closed {
                    return true;
                }

                // Power button click: no behavior yet. Session/power actions
                // live in the Start Menu footer (see launcher_zone below).
                if self.power_zone.contains(point) {
                    debug_log("[VORTEX] power clicked (no-op; see Start Menu)\n");
                    return false;
                }

                if self.show_logout_confirm {
                    if self.logout_cancel_r.contains(point) {
                        self.show_logout_confirm = false;
                        return true;
                    }
                    if self.logout_confirm_r.contains(point) {
                        self.show_logout_confirm = false;
                        // Safe logout path (no display stop):
                        // close shell windows + user apps, return toward login.
                        // TODO(session): call into sunlight-uac / session manager when ready.
                        debug_log("[VORTEX] logout confirmed (safe stub: close shell context)\n");
                        // For this task we just close overlays and let higher level handle session end.
                        // Do not kill critical services.
                        return true;
                    }
                    // click elsewhere on confirm open: close it
                    self.show_logout_confirm = false;
                    return true;
                }
                if self.launcher_zone.contains(point) {
                    if self.suppress_launcher_open {
                        self.suppress_launcher_open = false;
                        return true;
                    }
                    if self.start_menu.is_open() {
                        self.start_menu.close();
                    } else {
                        self.start_menu.open_menu();
                    }
                    return true;
                }
                if self.settings_zone.contains(point) {
                    return self.handle_app_click(
                        AppId::Settings,
                        monotonic_millis(),
                        LaunchSource::Shortcut,
                    );
                }

                // Center date/time: click toggles calendar popover, hides tooltip
                if self.datetime_zone.contains(point) {
                    self.show_datetime_tooltip = false;
                    self.show_calendar_popover = !self.show_calendar_popover;
                    if self.show_calendar_popover {
                        self.reset_calendar_view_to_today();
                    }
                    return true;
                }

                // Workspace indicator (semantic glyphs on the left) — switch
                // the active workspace. Set it optimistically so the redraw we
                // trigger on return shows the new active color immediately; the
                // next snapshot refresh confirms it from the compositor.
                if self.workspace_zone.contains(point) {
                    for (i, r) in self.workspace_btn_zones.iter().enumerate() {
                        if r.contains(point) {
                            let ws = (i + 1) as u64;
                            self.current_workspace = ws as u8;
                            let _ = ipc_call_timeout(
                                self.display_ep,
                                IpcMsg::with_label(SgpMsg::SET_WORKSPACE).word(0, ws),
                                DISPLAY_IPC_TIMEOUT_MS,
                            );
                            return true;
                        }
                    }
                }
                if self.logout_zone.contains(point) {
                    self.show_logout_confirm = true;
                    self.show_notif_panel = false;
                    self.show_calendar_popover = false;
                    return true;
                }
                if self.notif_zone.contains(point) {
                    self.show_notif_panel = !self.show_notif_panel;
                    self.show_calendar_popover = false;
                    self.show_logout_confirm = false;
                    return true;
                }
                if self.net_zone.contains(point) {
                    // visual only for now (network status shown by color)
                    debug_log("[VORTEX] net indicator clicked (visual)\n");
                    return false;
                }
                if let Some(idx) = icon_at(&self.desktop_icons, point) {
                    return self.handle_desktop_icon_click(idx, monotonic_millis());
                }
                for (rect, zone) in &self.dock_zones {
                    if rect.contains(point) {
                        return match zone {
                            DockZone::App(app_id) => self.handle_app_click(
                                *app_id,
                                monotonic_millis(),
                                LaunchSource::Dock,
                            ),
                            DockZone::Placeholder => false,
                        };
                    }
                }
                for (rect, win_id) in &self.running_zones {
                    if rect.contains(point) {
                        return self.activate_running_window(*win_id, monotonic_millis());
                    }
                }
                let changed = !self.selected_icons.is_empty();
                self.clear_desktop_selection();
                changed
            }
            Event::MouseDown { x, y, button } if button == 0 => {
                let point = Point::new(x, y);
                self.settings_hover = self.settings_zone.contains(point);
                if self.settings_zone.contains(point) {
                    if let Some(app) = self
                        .apps
                        .iter_mut()
                        .find(|app| app.app_id == AppId::Settings)
                    {
                        app.last_click_at = monotonic_millis();
                    }
                    return true;
                }
                for (rect, zone) in &self.dock_zones {
                    if rect.contains(point) {
                        if let DockZone::App(app_id) = zone {
                            if let Some(app) =
                                self.apps.iter_mut().find(|app| app.app_id == *app_id)
                            {
                                app.last_click_at = monotonic_millis();
                            }
                        }
                        return true;
                    }
                }
                for (rect, win_id) in &self.running_zones {
                    if rect.contains(point) {
                        if let Some(entry) = self
                            .running_apps
                            .iter_mut()
                            .find(|entry| entry.win_id == *win_id)
                        {
                            entry.last_click_at = monotonic_millis();
                        }
                        return true;
                    }
                }
                if let Some(idx) = icon_at(&self.desktop_icons, point) {
                    self.select_only_desktop_icon(idx);
                } else {
                    self.arm_desktop_marquee(point);
                }
                true
            }
            Event::MouseDown { x, y, button } if button == 1 => {
                let point = Point::new(x, y);
                self.settings_hover = self.settings_zone.contains(point);
                if self.settings_zone.contains(point) {
                    return true;
                }
                for (rect, zone) in &self.dock_zones {
                    if rect.contains(point) {
                        if let DockZone::App(app_id) = zone {
                            if let Some(app) =
                                self.apps.iter_mut().find(|app| app.app_id == *app_id)
                            {
                                app.last_click_at = monotonic_millis();
                            }
                        }
                        return true;
                    }
                }
                for (rect, win_id) in &self.running_zones {
                    if rect.contains(point) {
                        if let Some(entry) = self
                            .running_apps
                            .iter_mut()
                            .find(|entry| entry.win_id == *win_id)
                        {
                            entry.last_click_at = monotonic_millis();
                        }
                        return true;
                    }
                }
                self.end_selection_gesture();
                if let Some(idx) = icon_at(&self.desktop_icons, point) {
                    self.select_only_desktop_icon(idx);
                } else {
                    self.clear_desktop_selection();
                }
                self.context_menu = Some(make_context_menu(x, y, self.screen_w, self.screen_h));
                true
            }
            Event::Key('\x1b') => {
                let mut did = false;
                if self.show_calendar_popover {
                    self.show_calendar_popover = false;
                    did = true;
                }
                if self.show_notif_panel {
                    self.show_notif_panel = false;
                    did = true;
                }
                if self.show_logout_confirm {
                    self.show_logout_confirm = false;
                    did = true;
                }
                self.show_datetime_tooltip = false;
                did
            }
            Event::KeyPress { keycode: 1, .. } => {
                // rough esc keycode if used
                // fall to above via char if possible; keep simple
                false
            }
            Event::MouseMove { x, y } => {
                if self.selection_state != DesktopSelectState::Idle {
                    self.update_desktop_marquee(Point::new(x, y));
                    return true;
                }
                let prev = self.hover;
                self.hover = None;
                for (i, (rect, _)) in self.dock_zones.iter().enumerate() {
                    if rect.contains(Point::new(x, y)) {
                        self.hover = Some(i);
                        break;
                    }
                }
                let prev_running = self.running_hover;
                self.running_hover = None;
                for (i, (rect, _)) in self.running_zones.iter().enumerate() {
                    if rect.contains(Point::new(x, y)) {
                        self.running_hover = Some(i);
                        break;
                    }
                }
                // Reset the tooltip timer whenever the hovered cell changes
                // (including leaving the strip entirely), so the >300ms dwell
                // only counts continuous rest on one item.
                if self.running_hover != prev_running {
                    self.running_hover_since = self.running_hover.map(|_| monotonic_millis());
                }
                let prev_settings = self.settings_hover;
                self.settings_hover = self.settings_zone.contains(Point::new(x, y));
                if self.settings_hover != prev_settings {
                    return true;
                }

                // Date/time hover for tooltip (no busy, just flag)
                let p = Point::new(x, y);
                let prev_tip = self.show_datetime_tooltip;
                self.show_datetime_tooltip =
                    self.datetime_zone.contains(p) && !self.show_calendar_popover;
                if self.show_datetime_tooltip != prev_tip {
                    return true;
                }

                self.hover != prev
                    || self.running_hover != prev_running
                    || self.show_datetime_tooltip != prev_tip
            }
            Event::MouseUp { x, y, button } if button == 0 => {
                match self.selection_state {
                    DesktopSelectState::Dragging { .. } => {
                        self.update_desktop_marquee(Point::new(x, y));
                        self.suppress_next_click = true;
                        self.end_selection_gesture();
                        return true;
                    }
                    DesktopSelectState::Armed { .. } => {
                        // Simple click on empty desktop — clear selection.
                        self.clear_desktop_selection();
                        self.end_selection_gesture();
                        return true;
                    }
                    DesktopSelectState::Idle => {}
                }
                false
            }
            Event::Tick => {
                let now = monotonic_millis();
                let mut dirty = false;
                if self.sync_app_registry(now, false) {
                    dirty = true;
                }
                if now < self.next_status_poll_ms {
                    return dirty;
                }
                self.next_status_poll_ms = now.saturating_add(STATUS_POLL_MS);
                if self.refresh_status() {
                    dirty = true;
                }
                dirty
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[VORTEX] starting\n");

    // Resolve display_server endpoint (spin until ready).
    let display_ep = loop {
        if let Some(ep) = nameserver_lookup("display_server") {
            break ep;
        }
        process_yield();
    };

    let mut shell = VortexShell::new(display_ep);

    // Query physical framebuffer dimensions before allocating the SHM window.
    // This ensures the shell canvas matches the actual screen, not the image size.
    let metrics = query_display_metrics(display_ep).unwrap_or_else(|| {
        debug_log("[VORTEX] GET_SCREEN_INFO failed, using fallback resolution\n");
        DisplayMetrics::safe_fallback()
    });
    let screen_w = metrics.width_px;
    let screen_h = metrics.height_px;

    debug_log("[VORTEX] screen ");
    debug_log_u32(screen_w);
    debug_log("x");
    debug_log_u32(screen_h);
    debug_log("\n");

    // Create window at the exact physical screen size.
    // The SHM buffer will match canvas.width/height so panel positions are correct.
    let mut window = loop {
        match Window::connect(WindowConfig {
            width: screen_w,
            height: screen_h,
            title: "Vortex Shell",
            decoration: sunlight_ui::WindowDecoration::Normal,
        }) {
            Some(w) => break w,
            None => process_yield(),
        }
    };

    window.configure_flags(DESKTOP_LAYER_FLAGS);
    debug_log("[VORTEX] desktop layer registered, entering event loop\n");

    window.run(&mut shell);
    ProcessExit::exit(0);
}

/// Minimal decimal logger for u32 (avoids pulling in format!/alloc).
fn debug_log_u32(mut n: u32) {
    let mut buf = [0u8; 10];
    let mut len = 0usize;
    if n == 0 {
        debug_log("0");
        return;
    }
    while n > 0 {
        buf[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    // buf is reversed — print digits in correct order
    let mut s = [0u8; 11];
    for i in 0..len {
        s[i] = buf[len - 1 - i];
    }
    if let Ok(text) = core::str::from_utf8(&s[..len]) {
        debug_log(text);
    }
}

fn open_error_notification(err: sun_open::OpenError, path: &str) -> (&'static str, &'static str) {
    match err {
        sun_open::OpenError::NoAssociation => (
            "Cannot open file",
            "No application is registered for this file type",
        ),
        sun_open::OpenError::InvalidDesktopEntry => (
            "Cannot open shortcut",
            "The desktop entry file is invalid or missing Exec=",
        ),
        sun_open::OpenError::MissingPath => ("Cannot open file", "Missing file path"),
        sun_open::OpenError::PathTooLong => ("Cannot open file", "Path is too long"),
        sun_open::OpenError::LaunchFailed(_) => {
            debug_log("[VORTEX] open failed for ");
            debug_log(path);
            debug_log("\n");
            (
                "Cannot open file",
                "Unable to launch the default application",
            )
        }
    }
}

fn launch_error_text(err: sun_exec::LaunchError) -> &'static str {
    match err {
        sun_exec::LaunchError::AppNotFound => "app not found",
        sun_exec::LaunchError::InvalidCommand => "invalid command",
        sun_exec::LaunchError::SpawnFailed(_) => "spawn failed",
        sun_exec::LaunchError::PermissionDenied => "permission denied",
        sun_exec::LaunchError::DisplayUnavailable => "display unavailable",
        sun_exec::LaunchError::TooManyArgs => "too many arguments",
        sun_exec::LaunchError::ArgTooLong => "argument too long",
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    debug_log("[VORTEX] panic");
    if let Some(loc) = info.location() {
        debug_log(" at ");
        debug_log(loc.file());
        debug_log(":");
        debug_log_u32(loc.line());
        debug_log(":");
        debug_log_u32(loc.column());
    }
    debug_log("\n");
    loop {
        process_yield();
    }
}
