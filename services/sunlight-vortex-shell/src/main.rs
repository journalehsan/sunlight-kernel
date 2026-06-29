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
//! Power button: icon only. Click is recognized but performs no action.
//!   TODO(power): wire a small menu (lock, logout, reboot, shutdown) via powerd.
//!   Do not implement actual reboot/shutdown here.
//!
//! Update frequency: driven by Window::POLL_TIMEOUT_MS (~200 ms Event::Tick).
//!   Redraw is requested only on visible change (minute rollover or net state flip)
//!   to keep commits minimal. The shell uses double-buffered SHM; full view() on
//!   dirty still yields no visible flicker for small status updates.
//!
//! Constraints observed:
//!   - No real power actions.
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
//!   TODOs: see TODO(battery) and TODO(power) markers above; power zone click logs only.

#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use sunlight_ipc::{
    debug_log, ipc_call, ipc_call_timeout,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, nameserver_lookup, process_is_alive, process_yield, show_notification,
    unpack_iface_summary, CapabilityToken, InterfaceKind, IpcMsg, LinkState, NetworkdMsg,
    NotificationKind, ProcessExit, SgpMsg, TzMsg,
};
use sun_font::{self, draw_text_vcenter, measure_text, FontRole, TextStyle};
use sunlight_libc::{self as libc, sun_exec, DirEntry, FT_DIR};
use sunlight_ui::{
    image::TgaImage, App, Canvas, Color, Event, Point, Rect, Theme, Window, WindowConfig,
};

// ---------------------------------------------------------------------------
// Wallpaper asset
// ---------------------------------------------------------------------------

static WALLPAPER_TGA: &[u8] = include_bytes!("../../../docs/images/wallpaper.tga");
const FALLBACK_BG: u32 = 0x00121214;

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
static ICON_DRIVE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/drive-harddisk.tga");
static ICON_NETWORK_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/devices/64/network-card.tga");
static ICON_FILE_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/mimetypes/32/text-x-generic.tga");

// Dock icons
static ICON_TERMINAL_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/utilities-terminal.tga");
static ICON_CALC_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/accessories-calculator.tga");
static ICON_FILES_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/places/16/system-file-manager.tga");
static ICON_SETTINGS_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/preferences-system.tga");
static MENU_NEW_FOLDER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/folder-new.tga");
static MENU_NEW_TEXT_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/document-new.tga");
static MENU_REFRESH_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/view-refresh.tga");
static MENU_SORT_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/sort-name.tga");
static MENU_TERMINAL_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/scalable/xsi-utilities-terminal-symbolic.tga");

/// Theme icons for desktop shortcuts. All fields are `Copy` (TgaImage borrows `&'static [u8]`).
#[derive(Clone, Copy)]
struct DesktopTheme {
    computer: Option<TgaImage>,
    home: Option<TgaImage>,
    trash: Option<TgaImage>,
    folder: Option<TgaImage>,
    drive: Option<TgaImage>,
    network: Option<TgaImage>,
    file: Option<TgaImage>,
}

impl DesktopTheme {
    fn load() -> Self {
        Self {
            computer: TgaImage::parse(ICON_COMPUTER_TGA).ok(),
            home: TgaImage::parse(ICON_HOME_TGA).ok(),
            trash: TgaImage::parse(ICON_TRASH_TGA).ok(),
            folder: TgaImage::parse(ICON_FOLDER_TGA).ok(),
            drive: TgaImage::parse(ICON_DRIVE_TGA).ok(),
            network: TgaImage::parse(ICON_NETWORK_TGA).ok(),
            file: TgaImage::parse(ICON_FILE_TGA).ok(),
        }
    }

    fn icon_for(&self, kind: DesktopIconKind) -> Option<TgaImage> {
        match kind {
            DesktopIconKind::Computer => self.computer,
            DesktopIconKind::Home => self.home,
            DesktopIconKind::Trash => self.trash,
            DesktopIconKind::Folder => self.folder,
            DesktopIconKind::File => self.file,
            DesktopIconKind::Drive => self.drive,
            DesktopIconKind::Network => self.network,
        }
    }
}

/// Theme icons for the bottom dock.
#[derive(Clone, Copy)]
struct DockTheme {
    terminal: Option<TgaImage>,
    tasks: Option<TgaImage>,
    calc: Option<TgaImage>,
    files: Option<TgaImage>,
    settings: Option<TgaImage>,
}

impl DockTheme {
    fn load() -> Self {
        Self {
            terminal: TgaImage::parse(ICON_TERMINAL_TGA).ok(),
            tasks: None, // pixel-art fallback
            calc: TgaImage::parse(ICON_CALC_TGA).ok(),
            files: TgaImage::parse(ICON_FILES_TGA).ok(),
            settings: TgaImage::parse(ICON_SETTINGS_TGA).ok(),
        }
    }

    /// Return TGA icon for dock slot index (0=grid/launcher, 1=term, 2=tasks, 3=calc, 4=files).
    fn dock_icon(&self, idx: usize) -> Option<TgaImage> {
        match idx {
            0 => None, // launcher grid — keep pixel-art
            1 => self.terminal,
            2 => self.tasks,
            3 => self.calc,
            4 => self.files,
            _ => None,
        }
    }

    fn icon_for_app(&self, app_id: AppId) -> Option<TgaImage> {
        match app_id {
            AppId::Terminal => self.terminal,
            AppId::Calculator => self.calc,
            AppId::Files => self.files,
            AppId::Settings => self.settings,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppId {
    Terminal,
    Calculator,
    Files,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AppLaunchState {
    NotRunning,
    Launching,
    Running,
    Minimized,
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DockZone {
    Placeholder,
    App(AppId),
}

#[derive(Clone, Copy)]
struct DockAppState {
    app_id: AppId,
    display_name: &'static str,
    icon: AppId,
    pid: Option<u64>,
    main_window_id: Option<u64>,
    state: AppLaunchState,
    last_launch_id: u64,
    last_launch_source: LaunchSource,
    last_launch_started_at: u64,
    last_click_at: u64,
    launch_error: [u8; 64],
    launch_error_len: usize,
    launch_attempts: u32,
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
}

// ---------------------------------------------------------------------------
// Window geometry
// ---------------------------------------------------------------------------

// Fallback if GET_SCREEN_INFO fails (display server not yet ready on first poll).
const FALLBACK_W: u32 = 1280;
const FALLBACK_H: u32 = 800;

// Desktop-layer config flags (see app.rs WindowConfig docs).
// bits[1:0]=2 Desktop, bits[3:2]=3 Fullscreen, bit[4]=1 NoChrome → 0x1E
const DESKTOP_LAYER_FLAGS: u64 = 0x1E;

// ---------------------------------------------------------------------------
// Panel geometry constants
// ---------------------------------------------------------------------------

const RADIUS: u32 = 7;
const TOP_H: u32 = 36; // top bar height
const TOP_Y: i32 = 6; // top bar Y offset from screen top
const TOP_PAD: i32 = 8; // horizontal margin from screen edge

const BOT_H: u32 = 44; // bottom cluster height
const BOT_Y_OFF: i32 = 8; // distance from screen bottom to bottom of cluster
const ICON_BTN: u32 = 36; // square size for icon buttons in clusters
const CLUSTER_PAD: i32 = 6; // inner horizontal padding inside clusters
const ICON_GAP: i32 = 4; // gap between icon buttons

const SEARCH_W: u32 = 200; // search box width
const SEARCH_H: u32 = 32; // search box height
const STATUS_POLL_MS: u64 = 1000;
const TIME_IPC_TIMEOUT_MS: u64 = 250;
const NET_IPC_TIMEOUT_MS: u64 = 50;
const DISPLAY_IPC_TIMEOUT_MS: u64 = 50;
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

// ---------------------------------------------------------------------------
// Heap (bump allocator — no dynamic allocations used)
// ---------------------------------------------------------------------------

const HEAP_SIZE: usize = 64 * 1024;
#[repr(align(16))]
struct BumpHeap(core::cell::UnsafeCell<[u8; HEAP_SIZE]>);
unsafe impl Sync for BumpHeap {}
static BUMP_HEAP: BumpHeap = BumpHeap(core::cell::UnsafeCell::new([0u8; HEAP_SIZE]));

struct BumpAlloc;
unsafe impl core::alloc::GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        use core::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let heap_ptr = BUMP_HEAP.0.get() as *mut u8;
        let cur = NEXT.load(Ordering::Relaxed);
        let aligned = (cur + layout.align() - 1) & !(layout.align() - 1);
        let end = aligned + layout.size();
        if end > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        NEXT.store(end, Ordering::Relaxed);
        unsafe { heap_ptr.add(aligned) }
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: BumpAlloc = BumpAlloc;

// ---------------------------------------------------------------------------
// Pixel-art icon bitmaps (1 bit per pixel, u16 rows, MSB = leftmost pixel)
// Width is the number of significant bits; stored in the MSBs of each u16.
// All icons are 16×16 pixel fields scaled to fit an ICON_BTN×ICON_BTN cell.
// ---------------------------------------------------------------------------

/// Sun icon — filled circle + 4 cardinal rays (N/S/E/W).
const SUN_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0000000000000000,
    0b0000000100000000,  // N ray
    0b0000000000000000,
    0b0000011111000000,  // circle top (cols 5-9)
    0b0000111111100000,  // circle (cols 4-10)
    0b0000111111100000,
    0b0100111111100100,  // W ray + circle + E ray
    0b0000111111100000,
    0b0000111111100000,
    0b0000011111000000,  // circle bottom (cols 5-9)
    0b0000000000000000,
    0b0000000100000000,  // S ray
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

/// Tasks monitor icon — 3 horizontal bars (activity / list).
const TASKS_ROWS: [u16; 16] = [
    0b0000000000000000,
    0b0000000000000000,
    0b1111111111110000,
    0b1111111111110000,
    0b0000000000000000,
    0b1111111000000000,
    0b1111111000000000,
    0b0000000000000000,
    0b1111111111000000,
    0b1111111111000000,
    0b0000000000000000,
    0b0000000000000000,
    0b0000000000000000,
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
    File,
}

struct DesktopIcon {
    name: String,
    label: String,
    _tooltip: String,
    _action: String,
    kind: DesktopIconKind,
    rect: Rect,
}

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
}

#[derive(Clone, Copy)]
struct MenuItem {
    action: ContextMenuAction,
    rect: Rect,
    icon: Option<TgaImage>,
}

struct ContextMenuState {
    rect: Rect,
    items: [MenuItem; 5],
}

const MENU_LABELS: [(&str, ContextMenuAction); 5] = [
    ("New Folder", ContextMenuAction::NewFolder),
    ("New Text File", ContextMenuAction::NewTextFile),
    ("Refresh", ContextMenuAction::Refresh),
    ("Sort By Name", ContextMenuAction::SortByName),
    ("Open Terminal", ContextMenuAction::OpenTerminalHere),
];

// ---------------------------------------------------------------------------
// Shell application state
// ---------------------------------------------------------------------------

struct VortexShell {
    wallpaper: Option<TgaImage>,
    display_ep: CapabilityToken,
    desktop_paths: DesktopPaths,
    desktop_icons: Vec<DesktopIcon>,
    screen_w: u32,
    screen_h: u32,
    /// Bounds of each clickable dock button (local coords), plus the action.
    dock_zones: [(Rect, DockZone); 4],
    selected_icons: Vec<usize>,
    context_menu: Option<ContextMenuState>,
    drag_start: Option<Point>,
    drag_current: Option<Point>,
    drag_selecting: bool,
    drag_moved: bool,
    mouse_left_down: bool,
    suppress_next_click: bool,
    /// Tracks whether mouse is hovering over a dock icon (index 0..4).
    hover: Option<usize>,
    /// Tracks hover on the settings button in the left cluster.
    settings_hover: bool,
    /// Cached local hour/min for the status clock.
    status_hour: u8,
    status_min: u8,
    /// Cached "any non-loopback interface up/carrier".
    status_net_up: bool,
    /// Next monotonic deadline for best-effort status polling.
    next_status_poll_ms: u64,
    /// Bounds of the power button for future click handling.
    power_zone: Rect,
    /// Bounds of the settings button in the left cluster.
    settings_zone: Rect,
    /// TGA icon theme for desktop shortcuts.
    desktop_theme: DesktopTheme,
    /// TGA icon theme for the bottom dock.
    dock_theme: DockTheme,
    /// Dock app registry used for launch/focus/restore behavior.
    apps: [DockAppState; 4],
    /// Next monotonic deadline for app/window registry polling.
    next_app_poll_ms: u64,
    /// Next launch trace id assigned by this shell process.
    next_launch_id: u64,
}

impl VortexShell {
    fn new(display_ep: CapabilityToken) -> Self {
        let wallpaper = TgaImage::parse(WALLPAPER_TGA).ok();
        let desktop_paths = resolve_desktop_paths();
        ensure_directory(&desktop_paths.desktop_dir);
        if wallpaper.is_some() {
            debug_log("[VORTEX] wallpaper loaded\n");
        } else {
            debug_log("[VORTEX] wallpaper unavailable — using fallback\n");
        }
        let desktop_theme = DesktopTheme::load();
        let dock_theme = DockTheme::load();
        let mut shell = Self {
            wallpaper,
            display_ep,
            desktop_paths,
            desktop_icons: Vec::new(),
            screen_w: FALLBACK_W,
            screen_h: FALLBACK_H,
            dock_zones: [(Rect::new(0, 0, 0, 0), DockZone::Placeholder); 4],
            selected_icons: Vec::new(),
            context_menu: None,
            drag_start: None,
            drag_current: None,
            drag_selecting: false,
            drag_moved: false,
            mouse_left_down: false,
            suppress_next_click: false,
            hover: None,
            settings_hover: false,
            status_hour: 0xff,
            status_min: 0xff,
            status_net_up: false,
            next_status_poll_ms: 0,
            power_zone: Rect::new(0, 0, 0, 0),
            settings_zone: Rect::new(0, 0, 0, 0),
            desktop_theme,
            dock_theme,
            apps: [
                DockAppState::new(AppId::Terminal, "Sunlight Terminal", AppId::Terminal),
                DockAppState::new(AppId::Calculator, "Sunlight Calculator", AppId::Calculator),
                DockAppState::new(AppId::Files, "Sunlight Files", AppId::Files),
                DockAppState::new(AppId::Settings, "System Preferences", AppId::Settings),
            ],
            next_app_poll_ms: 0,
            next_launch_id: 1,
        };
        shell.reload_desktop_icons();
        shell
    }

    fn refresh_status(&mut self) -> bool {
        let mut dirty = false;
        if let Some((h, m)) = query_local_hm() {
            if h != self.status_hour || m != self.status_min {
                self.status_hour = h;
                self.status_min = m;
                dirty = true;
            }
        }
        if let Some(net_up) = query_net_up() {
            if net_up != self.status_net_up {
                self.status_net_up = net_up;
                dirty = true;
            }
        }
        dirty
    }

    fn reload_desktop_icons(&mut self) {
        self.desktop_icons = load_desktop_icons(&self.desktop_paths);
        self.selected_icons
            .retain(|idx| *idx < self.desktop_icons.len());
        if self.selected_icons.is_empty() {
            self.drag_start = None;
            self.drag_current = None;
            self.drag_selecting = false;
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
            AppLaunchState::Failed => "Failed",
        }
    }

    fn app_launch_path(app_id: AppId) -> &'static str {
        match app_id {
            AppId::Terminal => "/bin/sunlight-terminal",
            AppId::Calculator => "/bin/calculator",
            AppId::Files => "/bin/sunlight-files",
            AppId::Settings => "/bin/control-panel",
        }
    }

    fn app_launch_command(app_id: AppId) -> &'static [u8] {
        match app_id {
            AppId::Terminal => b"terminal",
            AppId::Calculator => b"calculator",
            AppId::Files => b"files",
            AppId::Settings => b"settings",
        }
    }

    fn app_trace_subject(app_id: AppId) -> &'static str {
        match app_id {
            AppId::Terminal => "app=terminal",
            AppId::Calculator => "app=calculator",
            AppId::Files => "app=files",
            AppId::Settings => "app=control-panel",
        }
    }

    fn desktop_icon_app(kind: DesktopIconKind) -> AppId {
        match kind {
            DesktopIconKind::Network => AppId::Settings,
            DesktopIconKind::Computer
            | DesktopIconKind::Home
            | DesktopIconKind::Trash
            | DesktopIconKind::Drive
            | DesktopIconKind::Folder
            | DesktopIconKind::File => AppId::Files,
        }
    }

    fn select_only_desktop_icon(&mut self, idx: usize) {
        self.selected_icons.clear();
        self.selected_icons.push(idx);
    }

    fn clear_desktop_selection(&mut self) {
        self.selected_icons.clear();
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
        if !self.drag_selecting {
            return None;
        }
        let start = self.drag_start?;
        let cur = self.drag_current.unwrap_or(start);
        let x = start.x.min(cur.x);
        let y = start.y.min(cur.y);
        let w = (start.x - cur.x).unsigned_abs().max(1);
        let h = (start.y - cur.y).unsigned_abs().max(1);
        Some(Rect::new(x, y, w, h))
    }

    fn launch_desktop_icon(&mut self, idx: usize, now: u64) -> bool {
        let Some(icon) = self.desktop_icons.get(idx) else {
            return false;
        };
        let app_id = Self::desktop_icon_app(icon.kind);
        self.select_only_desktop_icon(idx);
        self.handle_app_click(app_id, now, LaunchSource::Shortcut)
    }

    fn begin_desktop_marquee(&mut self, point: Point) {
        self.drag_start = Some(point);
        self.drag_current = Some(point);
        self.drag_selecting = true;
        self.drag_moved = false;
        self.suppress_next_click = false;
        self.clear_desktop_selection();
    }

    fn update_desktop_marquee(&mut self, point: Point) {
        if !self.drag_selecting {
            return;
        }
        self.drag_current = Some(point);
        if self.drag_start != self.drag_current {
            self.drag_moved = true;
        }
        if let Some(rect) = self.desktop_selection_rect() {
            self.select_desktop_icons_in_rect(rect);
        }
    }

    fn finish_desktop_marquee(&mut self) {
        self.drag_start = None;
        self.drag_current = None;
        self.drag_selecting = false;
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
            1 => DockZone::Placeholder,
            2 => DockZone::App(AppId::Calculator),
            3 => DockZone::App(AppId::Files),
            _ => DockZone::Placeholder,
        }
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

    fn fetch_window_snapshots(&self) -> Option<Vec<WindowSnapshot>> {
        let mut windows = Vec::new();
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
            windows.push(WindowSnapshot {
                id: reply.words[0],
                owner_pid: reply.words[1],
                state: reply.words[2],
            });
            idx = idx.saturating_add(1);
        }
        Some(windows)
    }

    fn sync_app_registry(&mut self, now: u64, force: bool) -> bool {
        if !force && now < self.next_app_poll_ms {
            return false;
        }
        self.next_app_poll_ms = now.saturating_add(APP_STATE_POLL_MS);

        let Some(windows) = self.fetch_window_snapshots() else {
            return false;
        };

        let mut dirty = false;
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
                AppLaunchState::Running | AppLaunchState::Minimized => {
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
                            app.state = AppLaunchState::Failed;
                            app.set_error("window disappeared");
                            app.main_window_id = None;
                            debug_log("[VORTEX] app_window_lost(");
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
            if let Some(pid) = app.pid {
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
                AppLaunchState::Running | AppLaunchState::Minimized | AppLaunchState::Launching
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
        let request = sun_exec::LaunchRequest::new(trace, source, Self::app_launch_command(app_id));
        match sun_exec::launch(request) {
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

    fn handle_app_click(&mut self, app_id: AppId, now: u64, source: LaunchSource) -> bool {
        self.sync_app_registry(now, true);
        self.log_app_click(app_id);
        let state = self.app(app_id).state;
        match state {
            AppLaunchState::NotRunning | AppLaunchState::Failed => {
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
    let mut top_marker = false;
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
            fill = theme.panel_alt;
            border = theme.accent;
            icon_color = theme.accent;
            bottom_marker = true;
            if hovered {
                fill = theme.accent.darken(74);
            }
        }
        AppLaunchState::Minimized => {
            fill = theme.panel_alt;
            border = theme.accent;
            icon_color = theme.accent;
            top_marker = true;
            if hovered {
                fill = theme.accent.darken(74);
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
    if top_marker {
        canvas.fill_rect(
            Rect::new(cell.x + 4, cell.y + 1, cell.w.saturating_sub(8), 2),
            theme.accent,
        );
    }
    if bottom_marker {
        canvas.fill_rect(
            Rect::new(cell.x + 4, cell.bottom() - 3, cell.w.saturating_sub(8), 2),
            theme.accent,
        );
    }

    if let Some(tga) = dock.icon_for_app(app.icon) {
        canvas.draw_tga_icon(&tga, cell.inset(2));
    } else {
        draw_icon16(canvas, cell, rows, icon_color);
    }
}

// ---------------------------------------------------------------------------
// Status cluster: clock (tz), network (networkd), battery (placeholder), power
// ---------------------------------------------------------------------------

/// Query "tz" for local time. Returns Some(hour,min) on success.
fn query_local_hm() -> Option<(u8, u8)> {
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
    // word(0): y<<48 | m<<40 | d<<32 | h<<24 | min<<16 | s<<8
    let w = reply.words[0];
    let h = ((w >> 24) & 0xff) as u8;
    let m = ((w >> 16) & 0xff) as u8;
    Some((h, m))
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
    draw_text_vcenter(canvas, ts, clock_x, bar.y, bar.h, &TextStyle::new(FontRole::UiSmall, theme.text));
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

fn draw_top_bar(
    canvas: &mut Canvas,
    theme: &Theme,
    screen_w: u32,
    net_up: bool,
    h: u8,
    m: u8,
) -> i32 {
    let bar = Rect::new(TOP_PAD, TOP_Y, screen_w - (TOP_PAD * 2) as u32, TOP_H);
    draw_panel(canvas, bar, theme.panel, theme.border);

    // ── Left zone: sun + workspace dots ──────────────────────────────────────
    let left_x = bar.x + 10;

    // Sun icon (orange)
    let sun_cell = Rect::new(left_x, bar.y, 28, TOP_H);
    draw_icon16(canvas, sun_cell, &SUN_ROWS, theme.accent);

    // Three workspace indicator dots
    let dot_start_x = left_x + 34;
    let dot_cy = bar.y + bar.h as i32 / 2;
    for i in 0..3i32 {
        let dot_x = dot_start_x + i * 14;
        let dot_color = if i == 0 { theme.accent } else { theme.text_dim };
        // 6×6 filled rounded rect as a dot
        canvas.fill_rounded_rect(Rect::new(dot_x, dot_cy - 3, 6, 6), 3, dot_color);
    }

    // ── Center zone: current app title ───────────────────────────────────────
    let title = "SunlightOS";
    let title_w = measure_text(title, FontRole::UiMedium).w;
    let title_x = bar.x + (bar.w as i32 - title_w as i32) / 2;
    draw_text_vcenter(canvas, title, title_x, bar.y, bar.h, &TextStyle::new(FontRole::UiMedium, theme.text));

    // ── Right zone: status cluster (power | net | bat | clock) ───────────────
    // Clock source: "tz" service (TzMsg::GET_LOCAL_TIME) — same path used by
    // sunlight-top via telemetry. Format is compact local time.
    // Network source: "networkd" via LIST_INTERFACES (no shelling to networkctl).
    let power_left = draw_status_cluster(canvas, theme, bar, net_up, h, m);
    power_left
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

fn join_path(base: &str, leaf: &str) -> String {
    let mut out = String::with_capacity(base.len() + leaf.len() + 1);
    out.push_str(base);
    if !out.ends_with('/') {
        out.push('/');
    }
    out.push_str(leaf.trim_start_matches('/'));
    out
}

fn read_file_bytes(path: &[u8], limit: usize) -> Option<Vec<u8>> {
    let fd = libc::open(path).ok()?;
    let mut out = Vec::new();
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
            icons.push(make_desktop_icon(
                name,
                "Desktop entry",
                path,
                if entry.file_type == FT_DIR {
                    DesktopIconKind::Folder
                } else {
                    DesktopIconKind::File
                },
            ));
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
        DesktopIconKind::File => (&FILE_ROWS, theme.text),
    }
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
        if is_selected {
            let highlight = slot.inset(4);
            canvas.fill_rounded_rect(highlight, 8, theme.panel);
            canvas.stroke_rounded_rect(highlight, 8, 1, theme.accent);
        }

        let icon_rect = Rect::new(slot.x + 18, slot.y + 6, 48, 40);

        // Prefer TGA theme icon; fall back to pixel-art if not loaded.
        if let Some(tga) = dt.icon_for(icon.kind) {
            canvas.draw_tga_icon(&tga, icon_rect);
        } else {
            let (rows, color) = desktop_icon_visual(icon.kind, theme);
            draw_icon16_scaled(canvas, icon_rect, rows, color, DESKTOP_ICON_SCALE);
        }

        let icon_color = if is_selected { theme.text } else { theme.text_dim.lighten(90) };
        let label_w = measure_text(&icon.label, FontRole::UiSmall).w;
        let label_x = slot.x + (slot.w as i32 - label_w as i32) / 2;
        let label_h = sun_font::line_height(FontRole::UiSmall) + 4;
        let label_rect_y = slot.y + 58;
        draw_text_vcenter(canvas, &icon.label, label_x, label_rect_y, label_h,
            &TextStyle::new(FontRole::UiSmall, icon_color));
    }
}

fn draw_desktop_marquee(canvas: &mut Canvas, theme: &Theme, rect: Rect) {
    if rect.w == 0 || rect.h == 0 {
        return;
    }
    let fill = Color::rgba(
        theme.accent.r(),
        theme.accent.g(),
        theme.accent.b(),
        48,
    );
    let border = Color::rgba(
        theme.accent.r(),
        theme.accent.g(),
        theme.accent.b(),
        180,
    );
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
    }; 5];
    for (i, (_, action)) in MENU_LABELS.iter().enumerate() {
        let icon = match action {
            ContextMenuAction::NewFolder => TgaImage::parse(MENU_NEW_FOLDER_TGA).ok(),
            ContextMenuAction::NewTextFile => TgaImage::parse(MENU_NEW_TEXT_TGA).ok(),
            ContextMenuAction::Refresh => TgaImage::parse(MENU_REFRESH_TGA).ok(),
            ContextMenuAction::SortByName => TgaImage::parse(MENU_SORT_TGA).ok(),
            ContextMenuAction::OpenTerminalHere => TgaImage::parse(MENU_TERMINAL_TGA).ok(),
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

// ---------------------------------------------------------------------------
// Bottom bar layout
// ---------------------------------------------------------------------------

/// Compute y coordinate of the top of the bottom clusters.
fn bot_y(screen_h: u32) -> i32 {
    screen_h as i32 - BOT_Y_OFF - BOT_H as i32
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
    let n = icons.len() as u32;
    let cluster_w = CLUSTER_PAD as u32 * 2 + n * ICON_BTN + (n - 1) * ICON_GAP as u32;
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

/// Draw the bottom-center dock and return the clickable zone rects
/// (terminal, tasks, calc, files).
fn draw_bot_center(
    canvas: &mut Canvas,
    theme: &Theme,
    by: i32,
    screen_w: u32,
    hover: Option<usize>,
    dock: DockTheme,
    terminal_app: &DockAppState,
    calc_app: &DockAppState,
    files_app: &DockAppState,
    now: u64,
) -> [Rect; 4] {
    let icons: &[&[u16; 16]; 5] = &[
        &GRID_ROWS,
        &TERMINAL_ROWS,
        &TASKS_ROWS,
        &CALC_ROWS,
        &FOLDER_ROWS,
    ];
    let n = icons.len() as u32;
    let cluster_w = CLUSTER_PAD as u32 * 2 + n * ICON_BTN + (n - 1) * ICON_GAP as u32;
    let cx_start = (screen_w as i32 - cluster_w as i32) / 2;
    let cluster = Rect::new(cx_start, by, cluster_w, BOT_H);
    draw_panel(canvas, cluster, theme.panel, theme.border);

    let mut x = cluster.x + CLUSTER_PAD;
    let mut clickable = [Rect::new(0, 0, 0, 0); 4];
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

        // Icons 1,2,3 are clickable (terminal, tasks, calc); icon 0 (grid) is placeholder.
        // Icon 4 is Sunlight Files.
        if i >= 1 {
            clickable[i - 1] = cell;
        }
        x += ICON_BTN as i32 + ICON_GAP;
    }
    clickable
}

/// Draw the bottom-right search box.
fn draw_bot_right(canvas: &mut Canvas, theme: &Theme, by: i32, screen_w: u32) {
    let sx = screen_w as i32 - TOP_PAD - SEARCH_W as i32;
    let sy = by + (BOT_H as i32 - SEARCH_H as i32) / 2;
    let search_rect = Rect::new(sx, sy, SEARCH_W, SEARCH_H);
    draw_panel(canvas, search_rect, theme.panel_alt, theme.border);

    // Placeholder text
    let ph = "Search...";
    let ph_w = measure_text(ph, FontRole::UiSmall).w;
    let ph_x = search_rect.x + (search_rect.w as i32 - ph_w as i32) / 2;
    draw_text_vcenter(canvas, ph, ph_x, search_rect.y, search_rect.h,
        &TextStyle::new(FontRole::UiSmall, theme.text_dim));
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
        self.screen_w = cw;
        self.screen_h = ch;

        // ── Wallpaper ────────────────────────────────────────────────────────
        if let Some(ref wp) = self.wallpaper {
            canvas.draw_image_cover(wp);
        } else {
            canvas.fill_rect(Rect::new(0, 0, cw, ch), Color(FALLBACK_BG));
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
        let pwr_left = draw_top_bar(
            canvas,
            theme,
            cw,
            self.status_net_up,
            self.status_hour,
            self.status_min,
        );
        // Record power zone for clicks (x,y,w,h). Height matches icon cell.
        self.power_zone = Rect::new(
            pwr_left,
            TOP_Y + (TOP_H as i32 - ICON_BTN as i32) / 2,
            ICON_BTN,
            ICON_BTN,
        );

        // ── Bottom panels ────────────────────────────────────────────────────
        let by = bot_y(ch);
        let dock_theme = self.dock_theme;
        let settings_app = *self.app(AppId::Settings);
        let terminal_app = *self.app(AppId::Terminal);
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
        let dock_cells = draw_bot_center(
            canvas,
            theme,
            by,
            cw,
            self.hover,
            dock_theme,
            &terminal_app,
            &calc_app,
            &files_app,
            now,
        );
        draw_bot_right(canvas, theme, by, cw);

        // Record clickable zones (terminal, tasks, calc, files).
        self.dock_zones = [
            (dock_cells[0], Self::dock_zone_app(0)),
            (dock_cells[1], Self::dock_zone_app(1)), // tasks — placeholder
            (dock_cells[2], Self::dock_zone_app(2)),
            (dock_cells[3], Self::dock_zone_app(3)),
        ];

        if let Some(menu) = &self.context_menu {
            draw_context_menu(canvas, theme, menu);
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                let point = Point::new(x, y);
                if self.suppress_next_click {
                    self.suppress_next_click = false;
                    return true;
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
                        }
                        return true;
                    }
                    return true;
                }
                // Power button click: no behavior yet.
                // TODO(power): show menu (lock, logout, reboot, shutdown).
                // Real actions must go through sunlight-powerd; do not implement here.
                if self.power_zone.contains(point) {
                    debug_log("[VORTEX] power clicked (no-op; TODO menu)\n");
                    return false;
                }
                if self.settings_zone.contains(point) {
                    return self.handle_app_click(
                        AppId::Settings,
                        monotonic_millis(),
                        LaunchSource::Shortcut,
                    );
                }
                if let Some(idx) = icon_at(&self.desktop_icons, point) {
                    return self.launch_desktop_icon(idx, monotonic_millis());
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
                let changed = !self.selected_icons.is_empty();
                self.clear_desktop_selection();
                changed
            }
            Event::MouseDown { x, y, button } if button == 0 => {
                let point = Point::new(x, y);
                self.mouse_left_down = true;
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
                if let Some(idx) = icon_at(&self.desktop_icons, point) {
                    self.select_only_desktop_icon(idx);
                } else {
                    self.begin_desktop_marquee(point);
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
                if let Some(idx) = icon_at(&self.desktop_icons, point) {
                    self.select_only_desktop_icon(idx);
                } else {
                    self.clear_desktop_selection();
                }
                self.context_menu = Some(make_context_menu(x, y, self.screen_w, self.screen_h));
                true
            }
            Event::MouseMove { x, y } => {
                if self.drag_selecting && self.mouse_left_down {
                    self.update_desktop_marquee(Point::new(x, y));
                    return true;
                } else if self.drag_selecting && !self.mouse_left_down {
                    // MouseUp was missed (e.g. released outside window); cancel drag.
                    self.finish_desktop_marquee();
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
                let prev_settings = self.settings_hover;
                self.settings_hover = self.settings_zone.contains(Point::new(x, y));
                if self.settings_hover != prev_settings {
                    return true;
                }
                self.hover != prev
            }
            Event::MouseUp { x, y, button } if button == 0 => {
                let _ = (x, y);
                self.mouse_left_down = false;
                if self.drag_selecting {
                    self.update_desktop_marquee(Point::new(x, y));
                    if self.drag_moved {
                        self.suppress_next_click = true;
                    }
                    self.finish_desktop_marquee();
                    return true;
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
    let packed = ipc_call(display_ep, IpcMsg::with_label(SgpMsg::GET_SCREEN_INFO));
    let (screen_w, screen_h) = if packed.label == SgpMsg::REPLY && packed.words[0] != 0 {
        let w = (packed.words[0] & 0xFFFF_FFFF) as u32;
        let h = (packed.words[0] >> 32) as u32;
        (w.max(320), h.max(240)) // clamp to sanity minimums
    } else {
        debug_log("[VORTEX] GET_SCREEN_INFO failed, using fallback resolution\n");
        (FALLBACK_W, FALLBACK_H)
    };

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
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[VORTEX] panic\n");
    loop {
        process_yield();
    }
}
