#![no_std]
#![no_main]

extern crate alloc;

use core::alloc::GlobalAlloc;
use core::cmp::Ordering;

use sun_font::{
    draw_text as sf_draw, draw_text_centered as sf_centered, draw_text_right as sf_right,
    draw_text_vcenter as sf_vcenter, line_height as sf_lh, FontRole, TextStyle, VecFont,
};
use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    monotonic_millis, process_yield, ProcessExit,
};
use sunlight_libc::{self as libc, env, sun_open, DirEntry, FT_DIR, FT_FILE};
use sunlight_ui::image::TgaImage;
use sunlight_ui::widgets::drive_card::{DriveCard, DriveCardLayout};
use sunlight_ui::widgets::sidebar_item::{SidebarItem, SidebarState};
use sunlight_ui::{App, Canvas, Event, HBox, Rect, Theme, UiSymbol, VBox, Window, WindowConfig};

// ── Central GUI font instance (vector / Inter) ───────────────────────────────
// Bitmap font (paint::font) remains available for early-boot and TTY rendering.
// Graphical desktop widgets receive this reference via with_font(); inline text
// uses the sf_* helpers with FontRole directly.
static FONT_UI_REG: VecFont = VecFont(FontRole::UiRegular);

// ---------------------------------------------------------------------------
// Embedded place icons (16 px TGA, nearest-neighbour scaled to 32px in widget)
// ---------------------------------------------------------------------------

static ICON_HOME_TGA: &[u8] = include_bytes!("../../docs/icons/SunlightOS/places/16/user-home.tga");
static ICON_FOLDER_DESKTOP_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/folder-desktop.tga");
static ICON_FOLDER_DOCUMENTS_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/folder-documents.tga");
static ICON_FOLDER_DOWNLOADS_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/folder-downloads.tga");
static ICON_FOLDER_MUSIC_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/folder-music.tga");
static ICON_FOLDER_PICTURES_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/folder-pictures.tga");
static ICON_FOLDER_VIDEOS_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/folder-videos.tga");
static ICON_FOLDER_NETWORK_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/folder-network.tga");
static ICON_USER_TRASH_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/user-trash.tga");
static ICON_FOLDER_TGA: &[u8] = include_bytes!("../../docs/icons/SunlightOS/places/16/folder.tga");
static ICON_IMAGE_FILE_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/mimetypes/32/image-x-generic.tga");

/// Parse a sidebar icon TGA for the given sidebar index.
/// Returns `None` if parsing fails (the widget falls back to the UiSymbol).
fn sidebar_tga(idx: usize) -> Option<TgaImage> {
    let bytes: &'static [u8] = match idx {
        0 => ICON_HOME_TGA,
        1 => ICON_FOLDER_DESKTOP_TGA,
        2 => ICON_FOLDER_DOCUMENTS_TGA,
        3 => ICON_FOLDER_DOWNLOADS_TGA,
        4 => ICON_FOLDER_PICTURES_TGA,
        5 => ICON_FOLDER_MUSIC_TGA,
        6 => ICON_FOLDER_VIDEOS_TGA,
        7 => ICON_FOLDER_TGA, // Root /
        8 => ICON_FOLDER_TGA, // Volumes (no device icon in places set)
        9 => ICON_FOLDER_NETWORK_TGA,
        10 => ICON_USER_TRASH_TGA,
        _ => return None,
    };
    TgaImage::parse(bytes).ok()
}

/// Parse a home-grid folder icon for the given folder index (0=Desktop…5=Videos).
fn home_folder_tga(idx: usize) -> Option<TgaImage> {
    let bytes: &'static [u8] = match idx {
        0 => ICON_FOLDER_DESKTOP_TGA,
        1 => ICON_FOLDER_DOCUMENTS_TGA,
        2 => ICON_FOLDER_DOWNLOADS_TGA,
        3 => ICON_FOLDER_PICTURES_TGA,
        4 => ICON_FOLDER_MUSIC_TGA,
        5 => ICON_FOLDER_VIDEOS_TGA,
        _ => ICON_FOLDER_TGA,
    };
    TgaImage::parse(bytes).ok()
}

fn image_file_tga() -> Option<TgaImage> {
    TgaImage::parse(ICON_IMAGE_FILE_TGA).ok()
}

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------

const WIN_W: u32 = 960;
const WIN_H: u32 = 620;
const TOOLBAR_H: u32 = 64;
const STATUS_H: u32 = 24;
const SIDEBAR_W: u32 = 224;
const GAP: u32 = 10;
const PAD: i32 = 10;
const RADIUS: u32 = 7;
const NAV_BTN_W: u32 = 64;
const NAV_BTN_H: u32 = 28;
const SEARCH_W: u32 = 224;
const SEARCH_H: u32 = 28;
/// Height passed to VBox for each SidebarItem — fits inside the sidebar.
const SIDEBAR_ITEM_H: u32 = 42;
const SIDEBAR_ITEM_GAP: u32 = 2;
/// Sidebar section-header label height + gap below it.
const SIDEBAR_HEADER_H: u32 = 20;
const HEADER_H: u32 = 20;
const ROW_H: u32 = 36;
const ICON_SLOT: u32 = 32; // px — thumbnail / folder icon reserved width
const TYPE_W: u32 = 116;
const SIZE_W: u32 = 92;
const MOD_W: u32 = 152;
const MAX_ENTRIES: usize = 64;
const PATH_LEN: usize = 256;
const ERROR_LEN: usize = 96;
/// Bottom details/preview pane height (Vista/7-style). Compact so it does not
/// eat the file list. Below this window height the pane shrinks further.
const DETAILS_H: u32 = 132;
const DETAILS_MIN_H: u32 = 92;
const DETAILS_PREVIEW_W: u32 = 132;
const DOUBLE_CLICK_MS: u64 = 400;
const TEXT_PREVIEW_LIMIT: usize = 8 * 1024;
const TEXT_PREVIEW_BUF_LEN: usize = 8 * 1024;

// Home grid: 6 core folders (Desktop, Documents, Downloads, Pictures, Music, Videos).
// Templates and Public are available via navigation but not shown on the home page.
const HOME_FOLDER_COUNT: usize = 6;
const HOME_CARD_H: u32 = 50;
const HOME_CARD_GAP: u32 = 10;
const HOME_COLS: usize = 3;
const HOME_GRID_ROWS: usize = (HOME_FOLDER_COUNT + HOME_COLS - 1) / HOME_COLS;

// Sidebar: 11 entries — trimmed to the most-used destinations.
// 0:Home  1:Desktop  2:Documents  3:Downloads  4:Pictures  5:Music  6:Videos
// 7:Root  8:Volumes  9:Network   10:Trash
const SIDEBAR_COUNT: usize = 11;

// ---------------------------------------------------------------------------
// Message / View
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Message {
    ShowHome,
    SelectSidebar(usize),
    SelectRow(usize),
    OpenRow(usize),
    OpenHomeFolder(usize),
    OpenHomeVolume(usize),
    NavigateUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Home,
    Volumes,
    Network,
    Directory,
}

// ---------------------------------------------------------------------------
// PathBuf — fixed-capacity path string (unchanged)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct PathBuf {
    buf: [u8; PATH_LEN],
    len: usize,
}

impl PathBuf {
    const fn root() -> Self {
        let mut buf = [0u8; PATH_LEN];
        buf[0] = b'/';
        Self { buf, len: 1 }
    }

    fn from_str(text: &str) -> Option<Self> {
        let mut out = Self::root();
        if out.set(text) {
            Some(out)
        } else {
            None
        }
    }

    fn set(&mut self, text: &str) -> bool {
        let bytes = text.as_bytes();
        let mut start = 0usize;
        let mut end = bytes.len();
        while start < end && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        while end > start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if start >= end {
            self.buf[0] = b'/';
            self.len = 1;
            return true;
        }
        // Canonical paths always start with '/'. Always write it first regardless
        // of whether the source started with '/'. Without this, set("/root") would
        // produce "root" (no leading slash) because the branch was skipped but the
        // component loop then wrote at offset 0 without a separator.
        if PATH_LEN == 0 {
            return false;
        }
        self.buf[0] = b'/';
        let mut written = 1usize;
        let mut i = start;
        let mut saw_component = false;
        while i < end {
            while i < end && bytes[i] == b'/' {
                i += 1;
            }
            let comp_start = i;
            while i < end && bytes[i] != b'/' {
                i += 1;
            }
            if i > comp_start {
                if saw_component && written < PATH_LEN {
                    self.buf[written] = b'/';
                    written += 1;
                }
                saw_component = true;
                for &b in &bytes[comp_start..i] {
                    if written >= PATH_LEN {
                        return false;
                    }
                    self.buf[written] = b;
                    written += 1;
                }
            }
        }
        if written == 0 {
            self.buf[0] = b'/';
            self.len = 1;
        } else {
            self.len = written;
        }
        true
    }

    fn join(&self, component: &str) -> Option<Self> {
        let mut out = *self;
        if out.len > 1 && out.buf[out.len - 1] != b'/' {
            if out.len >= PATH_LEN {
                return None;
            }
            out.buf[out.len] = b'/';
            out.len += 1;
        }
        if !out.set_suffix(component.as_bytes()) {
            return None;
        }
        Some(out)
    }

    fn set_suffix(&mut self, suffix: &[u8]) -> bool {
        if self.len == 0 {
            self.buf[0] = b'/';
            self.len = 1;
        }
        if self.len > 1 && self.buf[self.len - 1] != b'/' {
            if self.len >= PATH_LEN {
                return false;
            }
            self.buf[self.len] = b'/';
            self.len += 1;
        }
        for &b in suffix {
            if b == 0 || self.len >= PATH_LEN {
                return false;
            }
            self.buf[self.len] = b;
            self.len += 1;
        }
        true
    }

    fn parent(&self) -> Option<Self> {
        if self.len <= 1 {
            return None;
        }
        let mut end = self.len;
        while end > 1 && self.buf[end - 1] == b'/' {
            end -= 1;
        }
        while end > 1 && self.buf[end - 1] != b'/' {
            end -= 1;
        }
        if end <= 1 {
            Some(Self::root())
        } else {
            let mut out = *self;
            out.len = end - 1;
            if out.len == 0 {
                out.len = 1;
                out.buf[0] = b'/';
            }
            Some(out)
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("/")
    }
}

// ---------------------------------------------------------------------------
// Model types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct HomeFolder {
    name: &'static str,
    path: PathBuf,
}

impl HomeFolder {
    const fn placeholder() -> Self {
        Self {
            name: "",
            path: PathBuf::root(),
        }
    }
}

enum SidebarTarget {
    Home,
    Volumes,
    Network,
    Path(PathBuf),
}

#[derive(Clone, Copy)]
struct VolumeEntry {
    name: [u8; 64],
    name_len: usize,
    path: PathBuf,
}

impl VolumeEntry {
    const fn empty() -> Self {
        Self {
            name: [0; 64],
            name_len: 0,
            path: PathBuf::root(),
        }
    }

    fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Debug logging helpers (no_std — no format!, manual integer rendering)
// ---------------------------------------------------------------------------

fn log_i32(value: i32) {
    let mut buf = [0u8; 12];
    let mut i = buf.len();
    let neg = value < 0;
    let mut n = if neg {
        (-(value as i64)) as u64
    } else {
        value as u64
    };
    if n == 0 {
        debug_log("0");
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    if neg {
        if i == 0 {
            debug_log("-");
        } else {
            i -= 1;
            buf[i] = b'-';
        }
    }
    if let Ok(s) = core::str::from_utf8(&buf[i..]) {
        debug_log(s);
    }
}

fn log_usize(value: usize) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    let mut n = value;
    if n == 0 {
        debug_log("0");
        return;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    if let Ok(s) = core::str::from_utf8(&buf[i..]) {
        debug_log(s);
    }
}

// ---------------------------------------------------------------------------
// Allocator / panic
// ---------------------------------------------------------------------------

struct NoAlloc;

unsafe impl GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

// ---------------------------------------------------------------------------
// Thumbnail helpers — no heap; uses a static read buffer.
// ---------------------------------------------------------------------------

// Static buffer large enough for a 128×128 BGRA24 TGA (18-byte header + pixels).
/// Returns true if the file name ends in .simg or .tga.
fn is_image_name(name: &[u8]) -> bool {
    ends_with_ignore_ascii_case(name, b".simg") || ends_with_ignore_ascii_case(name, b".tga")
}

fn ascii_lower(byte: u8) -> u8 {
    if byte.is_ascii_uppercase() {
        byte + 32
    } else {
        byte
    }
}

fn ends_with_ignore_ascii_case(name: &[u8], suffix: &[u8]) -> bool {
    if name.len() < suffix.len() {
        return false;
    }
    let start = name.len() - suffix.len();
    for i in 0..suffix.len() {
        if ascii_lower(name[start + i]) != ascii_lower(suffix[i]) {
            return false;
        }
    }
    true
}

fn is_known_text_name(name: &[u8]) -> bool {
    const TEXT_EXTS: [&[u8]; 9] = [
        b".txt", b".md", b".rs", b".toml", b".json", b".log", b".conf", b".ini", b".sh",
    ];
    for ext in TEXT_EXTS {
        if ends_with_ignore_ascii_case(name, ext) {
            return true;
        }
    }
    false
}

fn is_likely_text_bytes(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    let mut control_count = 0usize;
    for &byte in bytes {
        if byte == 0 {
            return false;
        }
        if byte < 0x20 && byte != b'\n' && byte != b'\r' && byte != b'\t' {
            control_count += 1;
        }
    }
    control_count * 16 <= bytes.len()
}

fn text_type_label(name: &[u8]) -> &'static str {
    if ends_with_ignore_ascii_case(name, b".md") {
        "Markdown Text"
    } else if ends_with_ignore_ascii_case(name, b".rs") {
        "Rust Source"
    } else if ends_with_ignore_ascii_case(name, b".toml") {
        "TOML Config"
    } else if ends_with_ignore_ascii_case(name, b".json") {
        "JSON File"
    } else if ends_with_ignore_ascii_case(name, b".sh") {
        "Shell Script"
    } else {
        "Text File"
    }
}

/// Human-readable type label for a supported image file name.
fn image_type_label(name: &[u8]) -> &'static str {
    if ends_with_ignore_ascii_case(name, b".simg") {
        "Sunlight Image"
    } else {
        "TGA Image"
    }
}

/// Draw a raw TGA type-2 (24bpp) byte slice directly onto `canvas` at `dst`,
/// scaling with nearest-neighbour. No allocation; reads pixel data inline.
fn draw_tga_bytes(canvas: &mut Canvas, data: &[u8], dst: Rect) {
    if data.len() < 18 {
        return;
    }
    if data[2] != 2 {
        return;
    }
    let bpp = data[16];
    if bpp != 24 && bpp != 32 {
        return;
    }
    let w = u16::from_le_bytes([data[12], data[13]]) as u32;
    let h = u16::from_le_bytes([data[14], data[15]]) as u32;
    if w == 0 || h == 0 {
        return;
    }
    let top_down = (data[17] & 0x20) != 0;
    let bpp_b = (bpp / 8) as u32;
    let cm_len = u16::from_le_bytes([data[5], data[6]]) as u32;
    let cm_entry_bits = data[7] as u32;
    let cm_bytes = if data[1] != 0 {
        cm_len * ((cm_entry_bits + 7) / 8)
    } else {
        0
    };
    let data_off = (18 + data[0] as u32 + cm_bytes) as usize;
    let needed = data_off + (w * h * bpp_b) as usize;
    if data.len() < needed {
        return;
    }

    let cx0 = dst.x.max(0) as u32;
    let cy0 = dst.y.max(0) as u32;
    let cx1 = (dst.right() as u32).min(canvas.width);
    let cy1 = (dst.bottom() as u32).min(canvas.height);
    if cx0 >= cx1 || cy0 >= cy1 {
        return;
    }
    let dw = cx1 - cx0;
    let dh = cy1 - cy0;

    for dy in 0..dh {
        let src_y_scale = dy * h / dh;
        let file_row = if top_down {
            src_y_scale
        } else {
            h - 1 - src_y_scale
        };
        for dx in 0..dw {
            let src_x = dx * w / dw;
            let idx = data_off + (file_row * w + src_x) as usize * bpp_b as usize;
            if idx + 2 >= data.len() {
                continue;
            }
            let b = data[idx] as u32;
            let g = data[idx + 1] as u32;
            let r = data[idx + 2] as u32;
            use sunlight_ui::theme::Color;
            canvas.put_pixel(
                (cx0 + dx) as i32,
                (cy0 + dy) as i32,
                Color((r << 16) | (g << 8) | b),
            );
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[FILES] panic\n");
    loop {
        process_yield();
    }
}

// ---------------------------------------------------------------------------
// Async image preview (details/preview pane)
// ---------------------------------------------------------------------------
//
// Preview-on-selection is generated OFF the UI thread by a background worker.
// The UI thread never decodes image data: on selection change it bumps a
// generation counter and hands the path to the worker; on each Event::Tick it
// checks a single-slot mailbox. A result is applied only if its generation
// matches the current selection — stale results are dropped silently. Decode
// failures flip the slot to Failed so the pane shows a broken-image glyph and
// never retries the same selection. If the worker cannot be spawned the File
// Manager keeps working with placeholders only.

// ---------------------------------------------------------------------------
// Synchronous image preview — no threads, no heap.
//
// When a .simg/.tga file is selected in the directory view, we read it
// directly into a static source buffer and draw it in the details pane on
// every frame using the existing draw_tga_bytes helper. No thread is spawned,
// no atomic mailbox is needed, and the decode is zero-alloc.
// ---------------------------------------------------------------------------

/// Maximum source file size we will preview. Files larger than this show
/// "Preview unavailable" without reading. 4 MiB covers the sample pictures.
const PREVIEW_SRC_BUF_LEN: usize = 4 * 1024 * 1024;

/// 0 = no preview, 1 = ready (SRC_BUF valid), 2 = failed / unsupported.
static mut PREVIEW_READY: u8 = 0;
/// Bytes of valid TGA data at the start of PREVIEW_SRC_BUF.
static mut PREVIEW_SRC_FILLED: usize = 0;
/// Native dimensions extracted from the TGA header.
static mut PREVIEW_SRC_W: u32 = 0;
static mut PREVIEW_SRC_H: u32 = 0;
/// Raw file bytes — in BSS, costs nothing in the binary on disk.
static mut PREVIEW_SRC_BUF: [u8; PREVIEW_SRC_BUF_LEN] = [0u8; PREVIEW_SRC_BUF_LEN];
static mut TEXT_PREVIEW_READY: u8 = 0;
static mut TEXT_PREVIEW_LEN: usize = 0;
static mut TEXT_PREVIEW_TRUNCATED: u8 = 0;
static mut TEXT_PREVIEW_BUF: [u8; TEXT_PREVIEW_BUF_LEN] = [0u8; TEXT_PREVIEW_BUF_LEN];

/// Load `path` synchronously into PREVIEW_SRC_BUF and validate the TGA header.
/// Sets PREVIEW_READY to 1 on success, 2 on any failure.
fn load_preview_sync(path: &[u8]) {
    unsafe {
        PREVIEW_READY = 0;
        PREVIEW_SRC_FILLED = 0;
        PREVIEW_SRC_W = 0;
        PREVIEW_SRC_H = 0;
    }

    let stat = match libc::stat(path) {
        Ok(s) => s,
        Err(_) => {
            unsafe {
                PREVIEW_READY = 2;
            }
            return;
        }
    };
    let file_size = stat.size as usize;
    if file_size > PREVIEW_SRC_BUF_LEN || file_size < 18 {
        unsafe {
            PREVIEW_READY = 2;
        }
        return;
    }

    let fd = match libc::open(path) {
        Ok(f) => f,
        Err(_) => {
            unsafe {
                PREVIEW_READY = 2;
            }
            return;
        }
    };

    let mut total = 0usize;
    loop {
        let remaining = file_size - total;
        if remaining == 0 {
            break;
        }
        let chunk = remaining.min(8192);
        let n = unsafe { libc::read(fd, &mut PREVIEW_SRC_BUF[total..total + chunk]).unwrap_or(0) };
        if n == 0 {
            break;
        }
        total += n;
    }
    let _ = libc::close(fd);

    unsafe {
        PREVIEW_SRC_FILLED = total;
        // Validate TGA type-2 header
        if total >= 18 && PREVIEW_SRC_BUF[2] == 2 {
            let bpp = PREVIEW_SRC_BUF[16];
            let w = u16::from_le_bytes([PREVIEW_SRC_BUF[12], PREVIEW_SRC_BUF[13]]) as u32;
            let h = u16::from_le_bytes([PREVIEW_SRC_BUF[14], PREVIEW_SRC_BUF[15]]) as u32;
            if (bpp == 24 || bpp == 32) && w > 0 && h > 0 {
                PREVIEW_SRC_W = w;
                PREVIEW_SRC_H = h;
                PREVIEW_READY = 1;
                return;
            }
        }
        PREVIEW_READY = 2;
    }
}

fn clear_text_preview() {
    unsafe {
        TEXT_PREVIEW_READY = 0;
        TEXT_PREVIEW_LEN = 0;
        TEXT_PREVIEW_TRUNCATED = 0;
    }
}

fn load_text_preview_sync(path: &[u8], treat_as_text: bool) {
    clear_text_preview();
    let stat = match libc::stat(path) {
        Ok(s) => s,
        Err(_) => {
            unsafe {
                TEXT_PREVIEW_READY = 2;
            }
            return;
        }
    };
    let file_size = stat.size as usize;
    let capped = file_size.min(TEXT_PREVIEW_LIMIT);
    let fd = match libc::open(path) {
        Ok(f) => f,
        Err(_) => {
            unsafe {
                TEXT_PREVIEW_READY = 2;
            }
            return;
        }
    };
    let mut raw = [0u8; TEXT_PREVIEW_LIMIT];
    let mut total = 0usize;
    while total < capped {
        let chunk = (capped - total).min(1024);
        let n = libc::read(fd, &mut raw[total..total + chunk]).unwrap_or(0);
        if n == 0 {
            break;
        }
        total += n;
    }
    let _ = libc::close(fd);
    if total == 0 && file_size > 0 {
        unsafe {
            TEXT_PREVIEW_READY = 2;
        }
        return;
    }
    if !treat_as_text && !is_likely_text_bytes(&raw[..total]) {
        unsafe {
            TEXT_PREVIEW_READY = 3;
        }
        return;
    }
    let mut out = 0usize;
    let mut i = 0usize;
    while i < total && out < TEXT_PREVIEW_BUF_LEN {
        let byte = raw[i];
        if byte == b'\r' {
            i += 1;
            continue;
        }
        unsafe {
            TEXT_PREVIEW_BUF[out] =
                if byte == b'\n' || byte == b'\t' || (0x20..=0x7e).contains(&byte) {
                    byte
                } else if byte >= 0x80 {
                    b'?'
                } else {
                    b'.'
                };
        }
        out += 1;
        i += 1;
    }
    unsafe {
        TEXT_PREVIEW_LEN = out;
        TEXT_PREVIEW_TRUNCATED = if file_size > total { 1 } else { 0 };
        TEXT_PREVIEW_READY = 1;
    }
}

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct State {
    home_path: PathBuf,
    current_path: PathBuf,
    view_mode: ViewMode,
    selected_sidebar: usize,
    selected_row: Option<usize>,
    entries: [DirEntry; MAX_ENTRIES],
    entry_count: usize,
    folder_count: usize,
    file_count: usize,
    home_folders: [HomeFolder; HOME_FOLDER_COUNT],
    volume_entries: [VolumeEntry; MAX_ENTRIES],
    volume_count: usize,
    error: [u8; ERROR_LEN],
    error_len: usize,
}

impl State {
    fn new() -> Self {
        // Lightweight init: trust env vars, no filesystem probing.
        // Heavy work (directory listing, volume scan) is deferred to first
        // navigation or explicit user action.
        let home_path = detect_home_path();
        Self {
            current_path: home_path,
            home_path,
            view_mode: ViewMode::Home,
            selected_sidebar: 0,
            selected_row: None,
            entries: [DirEntry::zeroed(); MAX_ENTRIES],
            entry_count: 0,
            folder_count: 0,
            file_count: 0,
            home_folders: build_home_folders(home_path),
            volume_entries: [VolumeEntry::empty(); MAX_ENTRIES],
            volume_count: 0,
            error: [0; ERROR_LEN],
            error_len: 0,
        }
    }

    fn update(&mut self, message: Message) -> bool {
        match message {
            Message::ShowHome => self.show_home(),
            Message::SelectSidebar(idx) => self.go_to_sidebar(idx),
            Message::SelectRow(idx) => self.select_row(idx),
            Message::OpenRow(idx) => self.open_row(idx),
            Message::OpenHomeFolder(idx) => self.open_home_folder(idx),
            Message::OpenHomeVolume(idx) => self.open_home_volume(idx),
            Message::NavigateUp => self.go_up(),
        }
    }

    fn show_home(&mut self) -> bool {
        self.view_mode = ViewMode::Home;
        self.current_path = self.home_path;
        self.selected_sidebar = 0;
        self.selected_row = None;
        self.clear_error();
        // Refresh volumes on demand (user navigated home explicitly).
        self.refresh_home_volumes();
        true
    }

    fn show_volumes(&mut self) -> bool {
        self.view_mode = ViewMode::Volumes;
        self.selected_sidebar = 8;
        self.selected_row = None;
        self.clear_error();
        self.refresh_home_volumes();
        true
    }

    fn show_network(&mut self) -> bool {
        self.view_mode = ViewMode::Network;
        self.selected_sidebar = 9;
        self.selected_row = None;
        self.clear_error();
        true
    }

    fn go_to_sidebar(&mut self, idx: usize) -> bool {
        match self.sidebar_target(idx) {
            Some(SidebarTarget::Home) => self.show_home(),
            Some(SidebarTarget::Volumes) => self.show_volumes(),
            Some(SidebarTarget::Network) => self.show_network(),
            Some(SidebarTarget::Path(target)) => {
                self.selected_sidebar = idx;
                // Navigate; if the directory doesn't exist, load_directory
                // sets the error message — no pre-probing needed.
                // Always return true: sidebar selection changed, or error needs display.
                let _ = self.navigate_to(target);
                true
            }
            None => false,
        }
    }

    fn go_up(&mut self) -> bool {
        if self.view_mode != ViewMode::Directory {
            return false;
        }
        let Some(parent) = self.current_path.parent() else {
            return false;
        };
        self.navigate_to(parent)
    }

    fn open_home_folder(&mut self, idx: usize) -> bool {
        let Some(folder) = self.home_folders.get(idx).copied() else {
            return false;
        };
        debug_log("[FILES] place_card_click idx=");
        log_usize(idx);
        debug_log(" name=\"");
        debug_log(folder.name);
        debug_log("\" path=\"");
        debug_log(folder.path.as_str());
        debug_log("\"\n");
        // Navigate immediately; error surfaces only if the path is unreadable.
        // Always return true: either we navigated, or error needs to be displayed.
        if self.navigate_to(folder.path) {
            self.selected_sidebar = self.sidebar_index_for_path();
        }
        true
    }

    fn open_home_volume(&mut self, idx: usize) -> bool {
        let Some(volume) = self.volume_entries.get(idx).copied() else {
            return false;
        };
        // Always return true: either we navigated, or error needs to be displayed.
        if self.navigate_to(volume.path) {
            self.selected_sidebar = self.sidebar_index_for_path();
        }
        true
    }

    fn select_row(&mut self, idx: usize) -> bool {
        if idx >= self.entry_count {
            return false;
        }
        self.selected_row = Some(idx);
        self.clear_error();
        true
    }

    fn clear_row_selection(&mut self) -> bool {
        let changed = self.selected_row.is_some();
        self.selected_row = None;
        changed
    }

    fn open_row(&mut self, idx: usize) -> bool {
        if idx >= self.entry_count {
            return false;
        }
        self.selected_row = Some(idx);
        let entry = self.entries[idx];
        let name = entry.name_bytes();
        let name = match core::str::from_utf8(name) {
            Ok(s) => s,
            Err(_) => {
                self.set_error(if entry.file_type == FT_DIR {
                    "Invalid directory name"
                } else {
                    "Invalid file name"
                });
                return false;
            }
        };
        if entry.file_type == FT_DIR {
            if let Some(next) = self.current_path.join(name) {
                return self.navigate_to(next);
            }
            self.set_error("Path is too long");
            return false;
        }
        if let Some(file_path) = self.current_path.join(name) {
            self.open_file_with_resolver(&file_path)
        } else {
            self.set_error("Path is too long");
            false
        }
    }

    fn open_file_with_resolver(&mut self, path: &PathBuf) -> bool {
        let trace =
            launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
        match sun_open::open_path(trace, LaunchSource::Unknown, path.as_str().as_bytes()) {
            Ok(_) => {
                self.clear_error();
                true
            }
            Err(sun_open::OpenError::NoAssociation) => {
                self.set_error("No application is registered for this file type");
                false
            }
            Err(sun_open::OpenError::InvalidDesktopEntry) => {
                self.set_error("Invalid desktop entry file");
                false
            }
            Err(sun_open::OpenError::MissingPath) => {
                self.set_error("Missing file path");
                false
            }
            Err(sun_open::OpenError::PathTooLong) => {
                self.set_error("Path is too long");
                false
            }
            Err(sun_open::OpenError::LaunchFailed(_)) => {
                self.set_error("Unable to open file");
                false
            }
        }
    }

    fn navigate_to(&mut self, target: PathBuf) -> bool {
        debug_log("[FILES] navigate_start path=\"");
        debug_log(target.as_str());
        debug_log("\"\n");
        if self.load_directory(target) {
            self.view_mode = ViewMode::Directory;
            self.current_path = target;
            self.clear_error();
            self.selected_row = None;
            self.selected_sidebar = self.sidebar_index_for_path();
            debug_log("[FILES] navigate_done path=\"");
            debug_log(self.current_path.as_str());
            debug_log("\" items=");
            log_usize(self.entry_count);
            debug_log("\n");
            true
        } else {
            debug_log("[FILES] navigate_failed path=\"");
            debug_log(target.as_str());
            debug_log("\" error=\"");
            debug_log(self.error_str());
            debug_log("\"\n");
            false
        }
    }

    fn load_directory(&mut self, path: PathBuf) -> bool {
        self.entry_count = 0;
        self.folder_count = 0;
        self.file_count = 0;
        self.selected_row = None;
        match libc::read_dir(path.as_str().as_bytes(), &mut self.entries) {
            Ok(count) => {
                self.entry_count = count.min(MAX_ENTRIES);
                self.entries[..self.entry_count].sort_by(compare_entries);
                for entry in self.entries[..self.entry_count].iter() {
                    if entry.file_type == FT_DIR {
                        self.folder_count += 1;
                    } else {
                        self.file_count += 1;
                    }
                }
                self.current_path = path;
                self.clear_error();
                true
            }
            Err(_) => {
                self.set_error("Unable to read directory");
                false
            }
        }
    }

    fn refresh_home_volumes(&mut self) {
        self.volume_count = build_volumes(&mut self.volume_entries);
    }

    fn sidebar_target(&self, idx: usize) -> Option<SidebarTarget> {
        match idx {
            0 => Some(SidebarTarget::Home),
            1 => self.home_path.join("Desktop").map(SidebarTarget::Path),
            2 => self.home_path.join("Documents").map(SidebarTarget::Path),
            3 => self.home_path.join("Downloads").map(SidebarTarget::Path),
            4 => self.home_path.join("Pictures").map(SidebarTarget::Path),
            5 => self.home_path.join("Music").map(SidebarTarget::Path),
            6 => self.home_path.join("Videos").map(SidebarTarget::Path),
            7 => Some(SidebarTarget::Path(PathBuf::root())),
            8 => Some(SidebarTarget::Volumes),
            9 => Some(SidebarTarget::Network),
            10 => self.home_path.join(".Trash").map(SidebarTarget::Path),
            _ => None,
        }
    }

    fn sidebar_label(idx: usize) -> &'static str {
        match idx {
            0 => "Home",
            1 => "Desktop",
            2 => "Documents",
            3 => "Downloads",
            4 => "Pictures",
            5 => "Music",
            6 => "Videos",
            7 => "Root",
            8 => "Volumes",
            9 => "Network",
            _ => "Trash",
        }
    }

    fn sidebar_index_for_path(&self) -> usize {
        match self.view_mode {
            ViewMode::Home => return 0,
            ViewMode::Volumes => return 8,
            ViewMode::Network => return 9,
            ViewMode::Directory => {}
        }
        let current = self.current_path.as_str();
        if path_matches(current, self.home_path.as_str()) {
            return 0;
        }
        let check = |name: &str, idx: usize| -> Option<usize> {
            self.home_path
                .join(name)
                .filter(|p| path_matches(current, p.as_str()))
                .map(|_| idx)
        };
        if let Some(i) = check("Desktop", 1) {
            return i;
        }
        if let Some(i) = check("Documents", 2) {
            return i;
        }
        if let Some(i) = check("Downloads", 3) {
            return i;
        }
        if let Some(i) = check("Pictures", 4) {
            return i;
        }
        if let Some(i) = check("Music", 5) {
            return i;
        }
        if let Some(i) = check("Videos", 6) {
            return i;
        }
        if current == "/" {
            return 7;
        }
        if current == "/mnt" || current.starts_with("/mnt/") {
            return 8;
        }
        if current == "/boot" || current.starts_with("/boot/") {
            return 8;
        }
        if current == "/network" || current.starts_with("/network/") {
            return 9;
        }
        7
    }

    fn clear_error(&mut self) {
        self.error_len = 0;
    }

    fn set_error(&mut self, text: &str) {
        let bytes = text.as_bytes();
        self.error_len = bytes.len().min(self.error.len());
        self.error[..self.error_len].copy_from_slice(&bytes[..self.error_len]);
    }

    fn error_str(&self) -> &str {
        core::str::from_utf8(&self.error[..self.error_len]).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

struct FilesApp {
    state: State,
    /// Cached source dimensions of the last loaded preview (for metadata display).
    preview_src_w: u32,
    preview_src_h: u32,
    last_clicked_row: Option<usize>,
    last_click_at: u64,
}

impl FilesApp {
    fn new() -> Self {
        Self {
            state: State::new(),
            preview_src_w: 0,
            preview_src_h: 0,
            last_clicked_row: None,
            last_click_at: 0,
        }
    }

    // ── Layout helpers ────────────────────────────────────────────────────

    /// Compute the details pane height, shrinking it on short windows.
    fn details_height() -> u32 {
        // Keep at least ~260px for the file list; otherwise clamp the pane down.
        let reserved = TOOLBAR_H + STATUS_H + DETAILS_H;
        if WIN_H > reserved + 260 {
            DETAILS_H
        } else {
            DETAILS_MIN_H
        }
    }

    fn root_layout() -> (Rect, Rect, Rect, Rect) {
        let root = Rect::new(0, 0, WIN_W, WIN_H);
        let details_h = Self::details_height();
        let body_h = WIN_H
            .saturating_sub(TOOLBAR_H)
            .saturating_sub(STATUS_H)
            .saturating_sub(details_h);
        let heights = [TOOLBAR_H, body_h, details_h, STATUS_H];
        let mut rows = VBox::new(root).with_spacing(0).layout(&heights);
        let toolbar = rows.next().unwrap_or_default();
        let body = rows.next().unwrap_or_default();
        let details = rows.next().unwrap_or_default();
        let status = rows.next().unwrap_or_default();
        (toolbar, body, details, status)
    }

    fn body_layout(body: Rect) -> (Rect, Rect) {
        let main_w = body.w.saturating_sub(SIDEBAR_W + GAP);
        let widths = [SIDEBAR_W, main_w];
        let mut cols = HBox::new(body).with_spacing(GAP).layout(&widths);
        let sidebar = cols.next().unwrap_or_default();
        let main = cols.next().unwrap_or_default();
        (sidebar, main)
    }

    // ── Preview methods ───────────────────────────────────────────────────

    fn update_preview_for_selection(&mut self) {
        self.preview_src_w = 0;
        self.preview_src_h = 0;
        unsafe {
            PREVIEW_READY = 0;
        }
        clear_text_preview();

        if self.state.view_mode != ViewMode::Directory {
            return;
        }
        let Some(idx) = self.state.selected_row else {
            return;
        };
        if idx >= self.state.entry_count {
            return;
        }
        let entry = self.state.entries[idx];
        if entry.file_type == FT_DIR {
            return;
        }
        let name_bytes = entry.name_bytes();
        let name = match core::str::from_utf8(name_bytes) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(full_path) = self.state.current_path.join(name) {
            if is_image_name(name_bytes) {
                load_preview_sync(full_path.as_str().as_bytes());
                self.preview_src_w = unsafe { PREVIEW_SRC_W };
                self.preview_src_h = unsafe { PREVIEW_SRC_H };
            } else {
                load_text_preview_sync(
                    full_path.as_str().as_bytes(),
                    is_known_text_name(name_bytes),
                );
            }
        }
    }

    fn reset_row_click_state(&mut self) {
        self.last_clicked_row = None;
        self.last_click_at = 0;
    }

    fn select_item(&mut self, idx: usize) -> bool {
        let changed = self.state.update(Message::SelectRow(idx));
        self.update_preview_for_selection();
        changed
    }

    fn open_selected_or_item(&mut self, idx: usize) -> bool {
        let changed = self.state.update(Message::OpenRow(idx));
        self.update_preview_for_selection();
        if changed {
            self.reset_row_click_state();
        }
        changed
    }

    fn handle_directory_click(&mut self, idx: usize) -> bool {
        let now = monotonic_millis();
        let is_double_click = self.last_clicked_row == Some(idx)
            && now.saturating_sub(self.last_click_at) <= DOUBLE_CLICK_MS;
        self.last_clicked_row = Some(idx);
        self.last_click_at = now;
        if is_double_click {
            self.open_selected_or_item(idx)
        } else {
            self.select_item(idx)
        }
    }

    fn toolbar_layout(toolbar: Rect) -> (Rect, Rect, Rect, Rect, Rect) {
        let nav_y = toolbar.y + (toolbar.h as i32 - NAV_BTN_H as i32) / 2;
        let nav_x = toolbar.x + PAD;
        let back = Rect::new(nav_x, nav_y, NAV_BTN_W, NAV_BTN_H);
        let forward = Rect::new(back.right() + 8, nav_y, NAV_BTN_W, NAV_BTN_H);
        let up = Rect::new(forward.right() + 8, nav_y, NAV_BTN_W, NAV_BTN_H);
        let search_x = toolbar.right() - SEARCH_W as i32 - PAD;
        let search_y = toolbar.y + (toolbar.h as i32 - SEARCH_H as i32) / 2;
        let search = Rect::new(search_x, search_y, SEARCH_W, SEARCH_H);
        let breadcrumb_x = up.right() + 12;
        let breadcrumb_w = (search.x - breadcrumb_x - 12).max(0) as u32;
        let breadcrumb = Rect::new(breadcrumb_x, search_y, breadcrumb_w, SEARCH_H);
        (back, forward, up, search, breadcrumb)
    }

    /// Compute the rect for sidebar item `idx` using the same VBox layout as `draw_sidebar`.
    fn sidebar_item_rect(sidebar: Rect, idx: usize) -> Rect {
        let inner = sidebar.inset(PAD);
        let items_area = Rect::new(
            inner.x,
            inner.y + SIDEBAR_HEADER_H as i32,
            inner.w,
            inner.h.saturating_sub(SIDEBAR_HEADER_H),
        );
        let heights = [SIDEBAR_ITEM_H; SIDEBAR_COUNT];
        VBox::new(items_area)
            .with_spacing(SIDEBAR_ITEM_GAP)
            .layout(&heights)
            .nth(idx)
            .unwrap_or_default()
    }

    fn hit_test_sidebar(sidebar: Rect, x: i32, y: i32) -> Option<usize> {
        let point = sunlight_ui::Point::new(x, y);
        for idx in 0..SIDEBAR_COUNT {
            if Self::sidebar_item_rect(sidebar, idx).contains(point) {
                return Some(idx);
            }
        }
        None
    }

    fn row_rect(main: Rect, idx: usize) -> Rect {
        let inner = main.inset(PAD);
        let header_bottom = inner.y + HEADER_H as i32 + 24;
        Rect::new(
            inner.x,
            header_bottom + (idx as u32 * ROW_H) as i32,
            inner.w,
            ROW_H,
        )
    }

    fn hit_test_row(main: Rect, x: i32, y: i32, rows: usize) -> Option<usize> {
        let point = sunlight_ui::Point::new(x, y);
        for idx in 0..rows {
            if Self::row_rect(main, idx).contains(point) {
                return Some(idx);
            }
        }
        None
    }

    fn home_folder_rect(inner: Rect, idx: usize) -> Rect {
        let title_y = inner.y + 36;
        let grid_top = title_y + 18;
        let card_w = (inner
            .w
            .saturating_sub((HOME_COLS as u32 - 1) * HOME_CARD_GAP)
            / HOME_COLS as u32)
            .max(120);
        let col = idx % HOME_COLS;
        let row = idx / HOME_COLS;
        let x = inner.x + (col as u32 * (card_w + HOME_CARD_GAP)) as i32;
        let y = grid_top + (row as u32 * (HOME_CARD_H + HOME_CARD_GAP)) as i32;
        Rect::new(x, y, card_w, HOME_CARD_H)
    }

    fn home_volume_rect(inner: Rect, idx: usize) -> Rect {
        let section_y = Self::volumes_section_y(inner);
        let y = section_y + idx as i32 * (DriveCard::ROW_H as i32 + 6);
        Rect::new(inner.x, y, inner.w, DriveCard::ROW_H)
    }

    fn volumes_section_y(inner: Rect) -> i32 {
        // title_y = folder section top + grid height + gap
        let title_y =
            inner.y + 36 + 18 + HOME_GRID_ROWS as i32 * (HOME_CARD_H + HOME_CARD_GAP) as i32 + 12;
        title_y + 18 // after section label
    }

    // ── Draw ──────────────────────────────────────────────────────────────

    fn draw_toolbar(&self, canvas: &mut Canvas, theme: &Theme, toolbar: Rect) {
        canvas.fill_rounded_rect_with_border(toolbar, RADIUS, theme.panel, theme.border, 1);
        let (back, forward, up, search, breadcrumb) = Self::toolbar_layout(toolbar);
        let up_disabled = match self.state.view_mode {
            ViewMode::Directory => self.state.current_path.parent().is_none(),
            _ => true,
        };
        draw_pill(
            canvas,
            theme,
            back,
            "Back",
            Some(UiSymbol::Back),
            false,
            true,
        );
        draw_pill(
            canvas,
            theme,
            forward,
            "Fwd",
            Some(UiSymbol::Forward),
            false,
            true,
        );
        draw_pill(
            canvas,
            theme,
            up,
            "Up",
            Some(UiSymbol::Up),
            false,
            up_disabled,
        );

        canvas.fill_rounded_rect_with_border(search, RADIUS, theme.panel_alt, theme.border, 1);
        canvas.draw_ui_symbol(search.x + 8, search.y + 9, UiSymbol::Search, theme.text_dim);
        sf_vcenter(
            canvas,
            "Search",
            search.x + 22,
            search.y,
            search.h,
            &TextStyle::new(FontRole::UiRegular, theme.text_dim),
        );

        canvas.fill_rounded_rect(breadcrumb, RADIUS, theme.panel);
        let crumb = match self.state.view_mode {
            ViewMode::Home => "Home",
            ViewMode::Volumes => "Volumes",
            ViewMode::Network => "Network",
            ViewMode::Directory => self.state.current_path.as_str(),
        };
        sf_vcenter(
            canvas,
            crumb,
            breadcrumb.x + 10,
            breadcrumb.y,
            breadcrumb.h,
            &TextStyle::new(FontRole::MonoRegular, theme.accent),
        );
    }

    fn draw_sidebar(&self, canvas: &mut Canvas, theme: &Theme, sidebar: Rect) {
        canvas.fill_rounded_rect_with_border(sidebar, RADIUS, theme.panel, theme.border, 1);

        let inner = sidebar.inset(PAD);

        // Section label
        sf_vcenter(
            canvas,
            "Places",
            inner.x,
            inner.y + 3,
            14,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        // Items via SidebarItem widget
        let items_area = Rect::new(
            inner.x,
            inner.y + SIDEBAR_HEADER_H as i32,
            inner.w,
            inner.h.saturating_sub(SIDEBAR_HEADER_H),
        );
        let heights = [SIDEBAR_ITEM_H; SIDEBAR_COUNT];
        for (idx, item_rect) in VBox::new(items_area)
            .with_spacing(SIDEBAR_ITEM_GAP)
            .layout(&heights)
            .enumerate()
        {
            if idx >= SIDEBAR_COUNT {
                break;
            }
            let state = if idx == self.state.selected_sidebar {
                SidebarState::Selected
            } else {
                SidebarState::Normal
            };
            let tga = sidebar_tga(idx);
            let label = State::sidebar_label(idx);

            // Build the item — borrow tga only if it parsed
            if let Some(ref icon) = tga {
                SidebarItem::new(item_rect, label)
                    .with_icon(icon)
                    .with_state(state)
                    .with_font(&FONT_UI_REG)
                    .draw(canvas, theme);
            } else {
                SidebarItem::new(item_rect, label)
                    .with_state(state)
                    .with_font(&FONT_UI_REG)
                    .draw(canvas, theme);
            }
        }
    }

    fn draw_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        canvas.fill_rounded_rect_with_border(main, RADIUS, theme.panel, theme.border, 1);
        match self.state.view_mode {
            ViewMode::Home => self.draw_home_main(canvas, theme, main),
            ViewMode::Volumes => self.draw_volumes_main(canvas, theme, main),
            ViewMode::Network => self.draw_network_main(canvas, theme, main),
            ViewMode::Directory => self.draw_directory_main(canvas, theme, main),
        }
    }

    fn draw_home_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        canvas.draw_ui_symbol(inner.x, inner.y + 1, UiSymbol::FilesApp, theme.accent);
        sf_vcenter(
            canvas,
            "Home",
            inner.x + 16,
            inner.y,
            18,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        sf_vcenter(
            canvas,
            "Folders and volumes",
            inner.x,
            inner.y + 18,
            14,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        self.draw_home_folders(canvas, theme, inner);
        let after_volumes = self.draw_home_volumes(canvas, theme, inner);
        self.draw_home_network(canvas, theme, inner, after_volumes);
    }

    fn draw_home_folders(&self, canvas: &mut Canvas, theme: &Theme, inner: Rect) {
        let title_y = inner.y + 36;
        sf_vcenter(
            canvas,
            "Places",
            inner.x,
            title_y,
            14,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        let card_w = (inner
            .w
            .saturating_sub((HOME_COLS as u32 - 1) * HOME_CARD_GAP)
            / HOME_COLS as u32)
            .max(120);

        for idx in 0..HOME_FOLDER_COUNT {
            let rect = Self::home_folder_rect(inner, idx);
            let folder = self.state.home_folders[idx];

            canvas.fill_rounded_rect_with_border(rect, RADIUS, theme.panel_alt, theme.border, 1);

            // Icon — TGA preferred, UiSymbol fallback
            let icon_rect = Rect::new(rect.x + 6, rect.y + 7, 34, 34);
            if let Some(tga) = home_folder_tga(idx) {
                canvas.draw_tga_icon(&tga, icon_rect);
            } else {
                let sym = home_folder_symbol(idx);
                canvas.draw_ui_symbol(rect.x + 10, rect.y + 10, sym, theme.accent);
            }

            // Name and path hint — no "Missing" labels on startup
            let text_x = rect.x + 46;
            sf_draw(
                canvas,
                folder.name,
                text_x,
                rect.y + 8,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );
            // Show a shortened path if there's room
            let _ = card_w; // used for layout calculation above
            sf_draw(
                canvas,
                folder.path.as_str(),
                text_x,
                rect.y + 24,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }
    }

    /// Draw the Volumes section; returns the y coordinate directly below it.
    fn draw_home_volumes(&self, canvas: &mut Canvas, theme: &Theme, inner: Rect) -> i32 {
        let title_y =
            inner.y + 36 + 18 + HOME_GRID_ROWS as i32 * (HOME_CARD_H + HOME_CARD_GAP) as i32 + 12;
        sf_vcenter(
            canvas,
            "Volumes",
            inner.x,
            title_y,
            14,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        let mut y = title_y + 18;

        if self.state.volume_count == 0 {
            sf_vcenter(
                canvas,
                "No mounted volumes detected",
                inner.x,
                y + 4,
                14,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
            return y + 24;
        }

        for idx in 0..self.state.volume_count {
            let volume = self.state.volume_entries[idx];
            let rect = Rect::new(inner.x, y, inner.w, DriveCard::ROW_H);
            y += DriveCard::ROW_H as i32 + 6;

            DriveCard::new(rect, volume.name_str())
                .with_layout(DriveCardLayout::Row)
                .with_mount_path(volume.path.as_str())
                .with_font(&FONT_UI_REG)
                .draw(canvas, theme);
        }

        y
    }

    fn draw_home_network(&self, canvas: &mut Canvas, theme: &Theme, inner: Rect, y: i32) {
        if y >= inner.bottom() - 18 {
            return; // no space
        }
        sf_vcenter(
            canvas,
            "Network",
            inner.x,
            y + 4,
            14,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        canvas.draw_ui_symbol(inner.x, y + 20, UiSymbol::Network, theme.text_dim);
        sf_vcenter(
            canvas,
            "No network mounts",
            inner.x + 16,
            y + 18,
            14,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }

    fn draw_volumes_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        sf_vcenter(
            canvas,
            "Volumes",
            inner.x,
            inner.y,
            18,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        sf_vcenter(
            canvas,
            "Mounted filesystems and drives",
            inner.x,
            inner.y + 18,
            14,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );

        let mut y = inner.y + 42;
        for idx in 0..self.state.volume_count {
            let volume = self.state.volume_entries[idx];
            let rect = Rect::new(inner.x, y, inner.w, DriveCard::ROW_H);
            y += DriveCard::ROW_H as i32 + 6;
            DriveCard::new(rect, volume.name_str())
                .with_layout(DriveCardLayout::Row)
                .with_mount_path(volume.path.as_str())
                .with_font(&FONT_UI_REG)
                .draw(canvas, theme);
        }

        if self.state.volume_count == 0 {
            sf_vcenter(
                canvas,
                "No mounted volumes",
                inner.x,
                y + 8,
                14,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }
    }

    fn draw_network_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        sf_vcenter(
            canvas,
            "Network",
            inner.x,
            inner.y,
            18,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        let card = Rect::new(inner.x, inner.y + 28, inner.w, 60);
        canvas.fill_rounded_rect_with_border(card, RADIUS, theme.panel_alt, theme.border, 1);
        sf_draw(
            canvas,
            "No network mounts",
            card.x + 12,
            card.y + 10,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        sf_draw(
            canvas,
            "Network mounts appear here when VFS metadata is available.",
            card.x + 12,
            card.y + 28,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }

    fn draw_directory_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        sf_vcenter(
            canvas,
            self.state.current_path.as_str(),
            inner.x,
            inner.y,
            HEADER_H,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        let subtitle = if self.state.error_len != 0 {
            self.state.error_str()
        } else {
            "Name | Type | Size"
        };
        let subtitle_color = if self.state.error_len != 0 {
            theme.danger
        } else {
            theme.text_dim
        };
        sf_draw(
            canvas,
            subtitle,
            inner.x,
            inner.y + HEADER_H as i32,
            &TextStyle::new(FontRole::UiSmall, subtitle_color),
        );

        self.draw_directory_rows(canvas, theme, main);
    }

    fn draw_directory_rows(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        let list_top = inner.y + HEADER_H as i32 + 24;
        let list_h = inner.bottom() - list_top;
        if list_h <= 0 {
            return;
        }
        let visible_rows = (list_h as u32 / ROW_H) as usize;
        let row_count = self.state.entry_count.min(visible_rows);

        if row_count == 0 {
            sf_vcenter(
                canvas,
                "No entries in this directory",
                inner.x,
                list_top + 8,
                14,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
            return;
        }

        let name_w = inner
            .w
            .saturating_sub(TYPE_W + SIZE_W + MOD_W + 24)
            .max(180);
        let type_x = inner.x + name_w as i32;
        let size_x = type_x + TYPE_W as i32;
        let mod_x = size_x + SIZE_W as i32;

        for idx in 0..row_count {
            let entry = self.state.entries[idx];
            let row = Self::row_rect(main, idx);
            let selected = self.state.selected_row == Some(idx);
            let fill = if selected {
                theme.accent.darken(190)
            } else if idx % 2 == 0 {
                theme.panel
            } else {
                theme.panel_alt
            };
            canvas.fill_rect(row, fill);

            let font_lh = sf_lh(FontRole::UiRegular) as i32;
            let text_y = row.y + (ROW_H as i32 - font_lh) / 2;
            let icon_rect = Rect::new(row.x + 4, row.y + 2, 24, 24);
            if entry.file_type == FT_DIR {
                let sym_x = row.x + (ICON_SLOT as i32 - 12) / 2;
                let sym_y = row.y + (ROW_H as i32 - 14) / 2;
                canvas.draw_ui_symbol(sym_x, sym_y, UiSymbol::Folder, theme.accent);
            } else if is_image_name(entry.name_bytes()) {
                if let Some(icon) = image_file_tga() {
                    canvas.draw_tga_icon(&icon, icon_rect);
                } else {
                    let sym_x = row.x + (ICON_SLOT as i32 - 12) / 2;
                    let sym_y = row.y + (ROW_H as i32 - 14) / 2;
                    canvas.draw_ui_symbol(sym_x, sym_y, UiSymbol::Pictures, theme.accent);
                }
            } else {
                let sym_x = row.x + (ICON_SLOT as i32 - 12) / 2;
                let sym_y = row.y + (ROW_H as i32 - 14) / 2;
                canvas.draw_ui_symbol(sym_x, sym_y, UiSymbol::File, theme.text_dim);
            }
            // Text starts after the icon slot + 6px gap.
            let text_x = row.x + ICON_SLOT as i32 + 6;
            sf_draw(
                canvas,
                entry_name_str(&entry),
                text_x,
                text_y,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );

            let type_label = match entry.file_type {
                FT_DIR => "Directory",
                FT_FILE => {
                    if is_image_name(entry.name_bytes()) {
                        image_type_label(entry.name_bytes())
                    } else {
                        "File"
                    }
                }
                _ => "Other",
            };
            sf_draw(
                canvas,
                type_label,
                type_x + 8,
                text_y,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );

            if entry.file_type == FT_DIR {
                sf_right(
                    canvas,
                    Rect::new(size_x, row.y, SIZE_W, ROW_H),
                    "--",
                    &TextStyle::new(FontRole::UiRegular, theme.text),
                    8,
                );
            } else {
                let mut scratch = [0u8; 16];
                let len = write_size(entry.size, &mut scratch);
                let size_text = core::str::from_utf8(&scratch[..len]).unwrap_or("--");
                sf_right(
                    canvas,
                    Rect::new(size_x, row.y, SIZE_W, ROW_H),
                    size_text,
                    &TextStyle::new(FontRole::UiRegular, theme.text),
                    8,
                );
            }
            sf_draw(
                canvas,
                "--",
                mod_x + 8,
                text_y,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
        }
    }

    fn draw_details_pane(&self, canvas: &mut Canvas, theme: &Theme, details: Rect) {
        canvas.fill_rounded_rect_with_border(details, RADIUS, theme.panel_alt, theme.border, 1);
        canvas.hline(details.x, details.right(), details.y as u32, theme.border);

        let inner = details.inset(PAD);
        let preview_x = inner.right() - DETAILS_PREVIEW_W as i32;
        let left_area = Rect::new(
            inner.x,
            inner.y,
            inner.w.saturating_sub(DETAILS_PREVIEW_W + PAD as u32),
            inner.h,
        );
        let preview_area = Rect::new(preview_x, inner.y, DETAILS_PREVIEW_W, inner.h);

        match self.state.view_mode {
            ViewMode::Directory => {
                if let Some(idx) = self.state.selected_row {
                    if idx < self.state.entry_count {
                        let entry = self.state.entries[idx];
                        self.draw_file_details(canvas, theme, left_area, preview_area, entry);
                        return;
                    }
                }
                self.draw_folder_summary(canvas, theme, left_area, preview_area);
            }
            ViewMode::Home | ViewMode::Volumes | ViewMode::Network => {
                self.draw_folder_summary(canvas, theme, left_area, preview_area);
            }
        }
    }

    fn draw_folder_summary(&self, canvas: &mut Canvas, theme: &Theme, left: Rect, preview: Rect) {
        let lh_sm = sf_lh(FontRole::UiSmall) as i32;
        let lh_rg = sf_lh(FontRole::UiRegular) as i32;
        let mut y = left.y + 6;
        sf_draw(
            canvas,
            "Folder:",
            left.x,
            y,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        y += lh_sm + 2;
        let folder_name = self
            .state
            .current_path
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or("");
        sf_draw(
            canvas,
            folder_name,
            left.x,
            y,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        y += lh_rg + 6;

        sf_draw(
            canvas,
            "Path:",
            left.x,
            y,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        y += lh_sm + 2;
        sf_draw(
            canvas,
            self.state.current_path.as_str(),
            left.x,
            y,
            &TextStyle::new(FontRole::MonoRegular, theme.text),
        );
        y += lh_rg + 6;

        let mut buf = [0u8; 64];
        let mut len = 0;

        let items = self.state.entry_count;
        len += write_to_buf(&mut buf[len..], b"Items: ");
        len += write_usize(&mut buf[len..], items);
        if let Ok(s) = core::str::from_utf8(&buf[..len]) {
            sf_draw(
                canvas,
                s,
                left.x,
                y,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
        }
        y += lh_sm + 2;

        len = 0;
        len += write_to_buf(&mut buf[len..], b"Files: ");
        len += write_usize(&mut buf[len..], self.state.file_count);
        len += write_to_buf(&mut buf[len..], b", Dirs: ");
        len += write_usize(&mut buf[len..], self.state.folder_count);
        if let Ok(s) = core::str::from_utf8(&buf[..len]) {
            sf_draw(
                canvas,
                s,
                left.x,
                y,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
        }
        y += lh_sm + 2;

        let mut image_count = 0;
        for i in 0..self.state.entry_count {
            if is_image_name(self.state.entries[i].name_bytes()) {
                image_count += 1;
            }
        }
        len = 0;
        len += write_to_buf(&mut buf[len..], b"Images: ");
        len += write_usize(&mut buf[len..], image_count);
        len += write_to_buf(&mut buf[len..], b" (SIMG/TGA)");
        if let Ok(s) = core::str::from_utf8(&buf[..len]) {
            sf_draw(
                canvas,
                s,
                left.x,
                y,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
        }

        canvas.draw_ui_symbol_centered(preview, UiSymbol::Folder, theme.accent);
    }

    fn draw_file_details(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        left: Rect,
        preview: Rect,
        entry: DirEntry,
    ) {
        let lh_sm = sf_lh(FontRole::UiSmall) as i32;
        let lh_rg = sf_lh(FontRole::UiRegular) as i32;
        let mut y = left.y + 6;
        let name_bytes = entry.name_bytes();
        if let Ok(name) = core::str::from_utf8(name_bytes) {
            sf_draw(
                canvas,
                name,
                left.x,
                y,
                &TextStyle::new(FontRole::UiMedium, theme.accent),
            );
        }
        y += lh_rg + 4;

        let type_label = if is_image_name(name_bytes) {
            image_type_label(name_bytes)
        } else if is_known_text_name(name_bytes) {
            text_type_label(name_bytes)
        } else {
            "File"
        };
        sf_draw(
            canvas,
            type_label,
            left.x,
            y,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );
        y += lh_sm + 4;

        let mut buf = [0u8; 64];
        let mut len = write_to_buf(&mut buf, b"Size: ");
        let mut size_buf = [0u8; 16];
        let size_len = write_size(entry.size, &mut size_buf);
        if len + size_len <= buf.len() {
            buf[len..len + size_len].copy_from_slice(&size_buf[..size_len]);
            len += size_len;
        }
        if let Ok(s) = core::str::from_utf8(&buf[..len]) {
            sf_draw(
                canvas,
                s,
                left.x,
                y,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
        }
        y += lh_sm + 4;

        if is_image_name(name_bytes) && self.preview_src_w > 0 && self.preview_src_h > 0 {
            len = 0;
            len += write_to_buf(&mut buf[len..], b"Dimensions: ");
            len += write_usize(&mut buf[len..], self.preview_src_w as usize);
            len += write_to_buf(&mut buf[len..], b"x");
            len += write_usize(&mut buf[len..], self.preview_src_h as usize);
            if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                sf_draw(
                    canvas,
                    s,
                    left.x,
                    y,
                    &TextStyle::new(FontRole::UiSmall, theme.text),
                );
            }
            y += lh_sm + 4;

            sf_draw(
                canvas,
                "Format: RGBA8888",
                left.x,
                y,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
        }

        if is_image_name(name_bytes) {
            let ready = unsafe { PREVIEW_READY };
            if ready == 1 {
                let filled = unsafe { PREVIEW_SRC_FILLED };
                unsafe {
                    draw_tga_bytes(canvas, &PREVIEW_SRC_BUF[..filled], preview);
                }
            } else if ready == 2 {
                canvas.draw_ui_symbol_centered(preview, UiSymbol::MissingFolder, theme.danger);
                sf_vcenter(
                    canvas,
                    "Preview unavailable",
                    preview.x + 8,
                    preview.bottom() - sf_lh(FontRole::UiSmall) as i32 - 4,
                    sf_lh(FontRole::UiSmall),
                    &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                );
            } else {
                if let Some(icon) = image_file_tga() {
                    let icon_rect = Rect::new(
                        preview.x + (preview.w as i32 - 72) / 2,
                        preview.y + (preview.h as i32 - 72) / 2 - 10,
                        72,
                        72,
                    );
                    canvas.draw_tga_icon(&icon, icon_rect);
                } else {
                    canvas.draw_ui_symbol_centered(preview, UiSymbol::Pictures, theme.accent);
                }
            }
        } else if unsafe { TEXT_PREVIEW_READY } == 1 {
            canvas.fill_rect(preview, theme.panel);
            let text = unsafe { &TEXT_PREVIEW_BUF[..TEXT_PREVIEW_LEN] };
            let line_h = sf_lh(FontRole::UiSmall) as i32;
            let mut line_y = preview.y + 6;
            let mut start = 0usize;
            while start < text.len() && line_y + line_h <= preview.bottom() - line_h - 4 {
                let mut end = start;
                let mut cols = 0usize;
                while end < text.len() && text[end] != b'\n' && cols < 26 {
                    end += 1;
                    cols += 1;
                }
                let line = core::str::from_utf8(&text[start..end]).unwrap_or("");
                sf_draw(
                    canvas,
                    line,
                    preview.x + 6,
                    line_y,
                    &TextStyle::new(FontRole::UiSmall, theme.text),
                );
                line_y += line_h;
                start = end;
                if start < text.len() && text[start] == b'\n' {
                    start += 1;
                }
            }
            if unsafe { TEXT_PREVIEW_TRUNCATED } != 0 {
                sf_draw(
                    canvas,
                    "Truncated",
                    preview.x + 6,
                    preview.bottom() - line_h - 4,
                    &TextStyle::new(FontRole::UiSmall, theme.text_dim),
                );
            }
        } else if unsafe { TEXT_PREVIEW_READY } == 2 {
            canvas.draw_ui_symbol_centered(preview, UiSymbol::File, theme.danger);
            sf_vcenter(
                canvas,
                "Preview read error",
                preview.x + 8,
                preview.bottom() - sf_lh(FontRole::UiSmall) as i32 - 4,
                sf_lh(FontRole::UiSmall),
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        } else if unsafe { TEXT_PREVIEW_READY } == 3 {
            canvas.draw_ui_symbol_centered(preview, UiSymbol::File, theme.text_dim);
            sf_vcenter(
                canvas,
                "Preview unavailable",
                preview.x + 8,
                preview.bottom() - sf_lh(FontRole::UiSmall) as i32 - 4,
                sf_lh(FontRole::UiSmall),
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        } else {
            canvas.draw_ui_symbol_centered(preview, UiSymbol::File, theme.text);
        }
    }

    fn draw_status(&self, canvas: &mut Canvas, theme: &Theme, status: Rect) {
        canvas.fill_rounded_rect_with_border(status, RADIUS, theme.panel_alt, theme.border, 1);
        let summary = if self.state.error_len != 0 {
            self.state.error_str()
        } else {
            "Ready"
        };
        let mut count_buf = [0u8; 24];
        let count_len = write_count(
            self.state.entry_count,
            self.state.folder_count,
            self.state.file_count,
            &mut count_buf,
        );
        let count_text = core::str::from_utf8(&count_buf[..count_len]).unwrap_or("0 items");
        sf_vcenter(
            canvas,
            count_text,
            status.x + 12,
            status.y,
            STATUS_H,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );
        let summary_color = if self.state.error_len != 0 {
            theme.danger
        } else {
            theme.text_dim
        };
        sf_vcenter(
            canvas,
            summary,
            status.x + 160,
            status.y,
            STATUS_H,
            &TextStyle::new(FontRole::UiSmall, summary_color),
        );
    }
}

impl App for FilesApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        let (toolbar, body, details, status) = Self::root_layout();
        let (sidebar, main) = Self::body_layout(body);
        self.draw_toolbar(canvas, theme, toolbar);
        self.draw_sidebar(canvas, theme, sidebar);
        self.draw_main(canvas, theme, main);
        self.draw_details_pane(canvas, theme, details);
        self.draw_status(canvas, theme, status);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                debug_log("[FILES] mouse_down x=");
                log_i32(x);
                debug_log(" y=");
                log_i32(y);
                debug_log("\n");

                let (toolbar, body, _details, _) = Self::root_layout();
                let (sidebar, main) = Self::body_layout(body);
                let (_, _, up, _, _) = Self::toolbar_layout(toolbar);
                if up.contains(sunlight_ui::Point::new(x, y)) {
                    debug_log("[FILES] hit_test up_button\n");
                    return self.state.update(Message::NavigateUp);
                }
                if let Some(idx) = Self::hit_test_sidebar(sidebar, x, y) {
                    debug_log("[FILES] hit_test sidebar idx=");
                    log_usize(idx);
                    debug_log(" label=\"");
                    debug_log(State::sidebar_label(idx));
                    debug_log("\"\n");
                    debug_log("[FILES] sidebar_item_click idx=");
                    log_usize(idx);
                    debug_log(" label=\"");
                    debug_log(State::sidebar_label(idx));
                    debug_log("\"\n");
                    return if idx == 0 {
                        self.state.update(Message::ShowHome)
                    } else {
                        self.state.update(Message::SelectSidebar(idx))
                    };
                }
                match self.state.view_mode {
                    ViewMode::Home => {
                        let inner = main.inset(PAD);
                        for idx in 0..HOME_FOLDER_COUNT {
                            if Self::home_folder_rect(inner, idx)
                                .contains(sunlight_ui::Point::new(x, y))
                            {
                                debug_log("[FILES] hit_test home_folder idx=");
                                log_usize(idx);
                                debug_log("\n");
                                return self.state.update(Message::OpenHomeFolder(idx));
                            }
                        }
                        for idx in 0..self.state.volume_count {
                            let rect = Self::home_volume_rect(inner, idx);
                            if rect.contains(sunlight_ui::Point::new(x, y)) {
                                debug_log("[FILES] hit_test home_volume idx=");
                                log_usize(idx);
                                debug_log("\n");
                                return self.state.update(Message::OpenHomeVolume(idx));
                            }
                        }
                        debug_log("[FILES] hit_test none (home view)\n");
                    }
                    ViewMode::Volumes => {
                        // Click on DriveCard in full volumes view.
                        // Use row_y for layout cursor; outer y remains the click coordinate.
                        let inner = main.inset(PAD);
                        let mut row_y = inner.y + 42;
                        for idx in 0..self.state.volume_count {
                            let rect = Rect::new(inner.x, row_y, inner.w, DriveCard::ROW_H);
                            row_y += DriveCard::ROW_H as i32 + 6;
                            if rect.contains(sunlight_ui::Point::new(x, y)) {
                                debug_log("[FILES] hit_test volume_card idx=");
                                log_usize(idx);
                                debug_log("\n");
                                return self.state.update(Message::OpenHomeVolume(idx));
                            }
                        }
                        debug_log("[FILES] hit_test none (volumes view)\n");
                    }
                    ViewMode::Network => {}
                    ViewMode::Directory => {
                        if let Some(idx) = Self::hit_test_row(main, x, y, self.state.entry_count) {
                            debug_log("[FILES] hit_test directory_row idx=");
                            log_usize(idx);
                            debug_log("\n");
                            return self.handle_directory_click(idx);
                        }
                        debug_log("[FILES] hit_test none (directory view)\n");
                        let changed = self.state.clear_row_selection();
                        self.update_preview_for_selection();
                        self.reset_row_click_state();
                        return changed;
                    }
                }
                false
            }
            Event::KeyPress {
                keycode: 0x01,
                pressed: true,
                ..
            } => {
                sunlight_ui::request_close();
                true
            }
            Event::KeyPress {
                keycode: 0x4B,
                pressed: true,
                ..
            } => {
                let changed = self.state.update(Message::NavigateUp);
                self.update_preview_for_selection();
                self.reset_row_click_state();
                changed
            }
            Event::Tick => false,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

/// Determine the home path from the environment — no filesystem probing.
/// Trusts the HOME env var; falls back to /root then /.
fn detect_home_path() -> PathBuf {
    if let Some(home) = env::getenv(b"HOME") {
        if let Some(path) = PathBuf::from_str(home) {
            return path;
        }
    }
    PathBuf::from_str("/root").unwrap_or_else(PathBuf::root)
}

/// Build the home folder model — paths only, no presence probing.
/// The OS is responsible for creating standard user directories; the file
/// manager assumes they exist and surfaces an error only if navigation fails.
fn build_home_folders(home_path: PathBuf) -> [HomeFolder; HOME_FOLDER_COUNT] {
    let names: [&'static str; HOME_FOLDER_COUNT] = [
        "Desktop",
        "Documents",
        "Downloads",
        "Pictures",
        "Music",
        "Videos",
    ];
    let mut folders = [HomeFolder::placeholder(); HOME_FOLDER_COUNT];
    let mut i = 0usize;
    while i < HOME_FOLDER_COUNT {
        folders[i] = HomeFolder {
            name: names[i],
            path: home_path.join(names[i]).unwrap_or_else(PathBuf::root),
        };
        i += 1;
    }
    folders
}

fn build_volumes(out: &mut [VolumeEntry; MAX_ENTRIES]) -> usize {
    let mut count = 0usize;

    if count < out.len() {
        out[count] = make_volume("Root Filesystem", PathBuf::root());
        count += 1;
    }

    if let Some(boot) = PathBuf::from_str("/boot") {
        if libc::stat(boot.as_str().as_bytes()).is_ok() && count < out.len() {
            out[count] = make_volume("Boot", boot);
            count += 1;
        }
    }

    let mut mounts = [DirEntry::zeroed(); MAX_ENTRIES];
    if let Ok(found) = libc::read_dir(b"/mnt", &mut mounts) {
        let n = found.min(MAX_ENTRIES);
        mounts[..n].sort_by(compare_entries);
        let mut i = 0usize;
        while i < n && count < out.len() {
            let entry = mounts[i];
            i += 1;
            if entry.file_type != FT_DIR {
                continue;
            }
            let name = core::str::from_utf8(entry.name_bytes()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            if let Some(path) = PathBuf::from_str("/mnt").and_then(|b| b.join(name)) {
                out[count] = make_volume(name, path);
                count += 1;
            }
        }
    }

    count
}

fn make_volume(name: &str, path: PathBuf) -> VolumeEntry {
    let mut entry = VolumeEntry::empty();
    let bytes = name.as_bytes();
    let len = bytes.len().min(entry.name.len());
    entry.name[..len].copy_from_slice(&bytes[..len]);
    entry.name_len = len;
    entry.path = path;
    entry
}

fn path_matches(current: &str, base: &str) -> bool {
    current == base
        || (current.starts_with(base) && current.as_bytes().get(base.len()) == Some(&b'/'))
}

fn home_folder_symbol(idx: usize) -> UiSymbol {
    match idx {
        0 => UiSymbol::Desktop,
        1 => UiSymbol::Documents,
        2 => UiSymbol::Downloads,
        3 => UiSymbol::Pictures,
        4 => UiSymbol::Music,
        5 => UiSymbol::Videos,
        _ => UiSymbol::Folder,
    }
}

fn compare_entries(a: &DirEntry, b: &DirEntry) -> Ordering {
    let a_dir = a.file_type == FT_DIR;
    let b_dir = b.file_type == FT_DIR;
    match (a_dir, b_dir) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        _ => cmp_ascii_ci(a.name_bytes(), b.name_bytes()),
    }
}

fn cmp_ascii_ci(a: &[u8], b: &[u8]) -> Ordering {
    let len = a.len().min(b.len());
    for i in 0..len {
        let la = a[i].to_ascii_lowercase();
        let lb = b[i].to_ascii_lowercase();
        match la.cmp(&lb) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }
    a.len().cmp(&b.len())
}

fn entry_name_str(entry: &DirEntry) -> &str {
    core::str::from_utf8(entry.name_bytes()).unwrap_or("?")
}

fn write_to_buf(out: &mut [u8], data: &[u8]) -> usize {
    let len = data.len().min(out.len());
    out[..len].copy_from_slice(&data[..len]);
    len
}

fn write_usize(out: &mut [u8], val: usize) -> usize {
    let mut n = val;
    let mut buf = [0u8; 20];
    let mut i = 0;
    if n == 0 {
        buf[0] = b'0';
        i = 1;
    } else {
        while n > 0 {
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
            i += 1;
        }
        buf[..i].reverse();
    }
    let len = i.min(out.len());
    out[..len].copy_from_slice(&buf[..len]);
    len
}

fn write_size(size: u64, out: &mut [u8; 16]) -> usize {
    if size < 1024 {
        return write_number(size, out, b" B");
    }
    if size < 1024 * 1024 {
        return write_number(size / 1024, out, b" KiB");
    }
    write_number(size / (1024 * 1024), out, b" MiB")
}

fn write_count(items: usize, folders: usize, files: usize, out: &mut [u8; 24]) -> usize {
    let mut pos = 0usize;
    pos += write_number(items as u64, &mut out[pos..], b" items");
    if pos < out.len() {
        out[pos] = b' ';
        pos += 1;
    }
    pos += write_number(folders as u64, &mut out[pos..], b" folders,");
    if pos < out.len() {
        out[pos] = b' ';
        pos += 1;
    }
    pos += write_number(files as u64, &mut out[pos..], b" files");
    pos.min(out.len())
}

fn write_number(value: u64, out: &mut [u8], suffix: &[u8]) -> usize {
    let mut n = value;
    let mut tmp = [0u8; 20];
    let mut len = 0usize;
    if n == 0 {
        tmp[len] = b'0';
        len += 1;
    } else {
        while n > 0 {
            tmp[len] = b'0' + (n % 10) as u8;
            len += 1;
            n /= 10;
        }
    }
    let mut pos = 0usize;
    for i in (0..len).rev() {
        if pos >= out.len() {
            return pos;
        }
        out[pos] = tmp[i];
        pos += 1;
    }
    for &byte in suffix {
        if pos >= out.len() {
            return pos;
        }
        out[pos] = byte;
        pos += 1;
    }
    pos
}

fn draw_pill(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    text: &str,
    icon: Option<UiSymbol>,
    active: bool,
    disabled: bool,
) {
    let fill = if disabled {
        theme.panel_alt
    } else if active {
        theme.accent
    } else {
        theme.panel
    };
    let border = if active {
        theme.accent_hover
    } else {
        theme.border
    };
    let color = if disabled {
        theme.text_dim
    } else if active {
        theme.bg
    } else {
        theme.text
    };
    canvas.fill_rounded_rect_with_border(rect, RADIUS, fill, border, 1);
    if let Some(sym) = icon {
        canvas.draw_ui_symbol(rect.x + 8, rect.y + 9, sym, color);
        sf_vcenter(
            canvas,
            text,
            rect.x + 22,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiSmall, color),
        );
    } else {
        sf_centered(
            canvas,
            rect,
            text,
            &TextStyle::new(FontRole::UiSmall, color),
        );
    }
}

// ---------------------------------------------------------------------------
// Entry point — window is opened BEFORE state init to show UI immediately
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start(_argc: u64, _argv: *const *const u8, envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(_argc, _argv);
    let trace = launch_trace::current().unwrap_or(LaunchTrace::new(0, LaunchSource::Unknown, 0));
    launch_trace::log_phase_now(
        trace,
        "app=files",
        "app_main_started",
        Some(sunlight_ipc::getpid()),
    );
    env::init(envp);

    // Open the window first so the compositor can begin displaying the
    // skeleton UI while the app model initialises.
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Files",
        decoration: sunlight_ui::WindowDecoration::Normal,
    }) {
        Some(w) => w,
        None => {
            debug_log("[FILES] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };

    // Lightweight model init — no filesystem probing, no IPC beyond env.
    let mut app = FilesApp::new();
    window.run(&mut app);
    ProcessExit::exit(0);
}
