#![no_std]
#![no_main]

use core::alloc::GlobalAlloc;
use core::cmp::Ordering;

use sunlight_ipc::{
    debug_log,
    launch_trace::{self, LaunchSource, LaunchTrace},
    process_yield, show_notification, NotificationKind, ProcessExit,
};
use sunlight_libc::{self as libc, env, DirEntry, FT_DIR, FT_FILE};
use sunlight_ui::image::TgaImage;
use sunlight_ui::widgets::Label;
use sunlight_ui::{App, Canvas, Event, HBox, Rect, Theme, UiSymbol, VBox, Window, WindowConfig};

// ---------------------------------------------------------------------------
// Icon theme: home-folder place icons (256×256 BGRA TGA, embedded at compile time)
// ---------------------------------------------------------------------------

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
static ICON_FOLDER_HOME_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/folder_home.tga");
static ICON_FOLDER_TGA: &[u8] = include_bytes!("../../docs/icons/SunlightOS/places/16/folder.tga");
static ICON_USER_TRASH_TGA: &[u8] =
    include_bytes!("../../docs/icons/SunlightOS/places/16/user-trash.tga");

/// Return the TGA icon for a home folder (Desktop=0 … Trash=7).  Parses the
/// header on each call (O(1), just offset arithmetic — fine for low-fps UI).
fn home_folder_tga(idx: usize, present: bool) -> Option<TgaImage> {
    let bytes: &'static [u8] = if !present {
        ICON_FOLDER_TGA
    } else {
        match idx {
            0 => ICON_FOLDER_DESKTOP_TGA,
            1 => ICON_FOLDER_DOCUMENTS_TGA,
            2 => ICON_FOLDER_DOWNLOADS_TGA,
            3 => ICON_FOLDER_MUSIC_TGA,
            4 => ICON_FOLDER_PICTURES_TGA,
            5 => ICON_FOLDER_VIDEOS_TGA,
            6 => ICON_FOLDER_HOME_TGA,
            7 => ICON_USER_TRASH_TGA,
            _ => ICON_FOLDER_TGA,
        }
    };
    TgaImage::parse(bytes).ok()
}

const WIN_W: u32 = 960;
const WIN_H: u32 = 620;
const TOOLBAR_H: u32 = 64;
const STATUS_H: u32 = 24;
const SIDEBAR_W: u32 = 220;
const GAP: u32 = 12;
const PAD: i32 = 12;
const RADIUS: u32 = 7;
const NAV_BTN_W: u32 = 64;
const NAV_BTN_H: u32 = 28;
const SEARCH_W: u32 = 224;
const SEARCH_H: u32 = 28;
const SIDEBAR_ITEM_H: u32 = 30;
const SIDEBAR_ITEM_GAP: u32 = 6;
const HEADER_H: u32 = 20;
const ROW_H: u32 = 18;
const TYPE_W: u32 = 116;
const SIZE_W: u32 = 92;
const MOD_W: u32 = 152;
const MAX_ENTRIES: usize = 64;
const PATH_LEN: usize = 256;
const ERROR_LEN: usize = 96;
const HOME_FOLDER_COUNT: usize = 8;
const HOME_CARD_H: u32 = 50;
const HOME_VOLUME_H: u32 = 44;
const HOME_CARD_GAP: u32 = 10;
const HOME_COLS: usize = 3;
const HOME_GRID_ROWS: usize = (HOME_FOLDER_COUNT + HOME_COLS - 1) / HOME_COLS;
/// Total sidebar entries: 8 std folders + Home + Root + Volumes + Network
const SIDEBAR_COUNT: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Message {
    ShowHome,
    SelectSidebar(usize),
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

        let mut written = 0usize;
        if bytes[start] != b'/' {
            if written >= PATH_LEN {
                return false;
            }
            self.buf[written] = b'/';
            written += 1;
        }

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
            true
        } else {
            self.len = written;
            true
        }
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

#[derive(Clone, Copy)]
struct HomeFolder {
    name: &'static str,
    path: PathBuf,
    present: bool,
}

impl HomeFolder {
    const fn empty() -> Self {
        Self {
            name: "",
            path: PathBuf::root(),
            present: false,
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
    present: bool,
}

impl VolumeEntry {
    const fn empty() -> Self {
        Self {
            name: [0; 64],
            name_len: 0,
            path: PathBuf::root(),
            present: false,
        }
    }

    fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
}

struct NoAlloc;

unsafe impl GlobalAlloc for NoAlloc {
    unsafe fn alloc(&self, _layout: core::alloc::Layout) -> *mut u8 {
        core::ptr::null_mut()
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: NoAlloc = NoAlloc;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[FILES] panic\n");
    loop {
        process_yield();
    }
}

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
        let home_path = detect_home_path();
        let mut state = Self {
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
        };
        state.refresh_home_volumes();
        if !state.load_directory(state.current_path) {
            let _ = state.load_directory(PathBuf::root());
        }
        state
    }

    fn update(&mut self, message: Message) -> bool {
        match message {
            Message::ShowHome => self.show_home(),
            Message::SelectSidebar(idx) => self.go_to_sidebar(idx),
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
        self.refresh_home_folders();
        self.refresh_home_volumes();
        true
    }

    fn show_volumes(&mut self) -> bool {
        self.view_mode = ViewMode::Volumes;
        self.selected_sidebar = 10;
        self.selected_row = None;
        self.clear_error();
        self.refresh_home_volumes();
        true
    }

    fn show_network(&mut self) -> bool {
        self.view_mode = ViewMode::Network;
        self.selected_sidebar = 11;
        self.selected_row = None;
        self.clear_error();
        true
    }

    fn go_to_sidebar(&mut self, idx: usize) -> bool {
        if idx == 0 {
            return self.show_home();
        }
        match self.sidebar_target(idx) {
            Some(SidebarTarget::Home) => self.show_home(),
            Some(SidebarTarget::Volumes) => self.show_volumes(),
            Some(SidebarTarget::Network) => self.show_network(),
            Some(SidebarTarget::Path(target)) => {
                if (1..=HOME_FOLDER_COUNT).contains(&idx) && !self.home_folders[idx - 1].present {
                    self.selected_sidebar = idx;
                    self.set_error("Folder is missing or could not be created.");
                    let _ = show_notification(
                        NotificationKind::Error,
                        "Folder unavailable",
                        self.home_folders[idx - 1].name,
                        10_000,
                    );
                    true
                } else {
                    self.selected_sidebar = idx;
                    self.navigate_to(target)
                }
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
        if !folder.present {
            self.set_error("Folder is missing or could not be created.");
            let _ = show_notification(
                NotificationKind::Error,
                "Folder unavailable",
                folder.name,
                10_000,
            );
            return true;
        }
        if self.navigate_to(folder.path) {
            self.selected_sidebar = self.sidebar_index_for_path();
            true
        } else {
            false
        }
    }

    fn open_home_volume(&mut self, idx: usize) -> bool {
        let Some(volume) = self.volume_entries.get(idx).copied() else {
            return false;
        };
        if !volume.present {
            self.set_error("Volume is missing. TODO: mount metadata.");
            return false;
        }
        if self.navigate_to(volume.path) {
            self.view_mode = ViewMode::Directory;
            self.selected_sidebar = self.sidebar_index_for_path();
            true
        } else {
            false
        }
    }

    fn open_row(&mut self, idx: usize) -> bool {
        if idx >= self.entry_count {
            return false;
        }

        self.selected_row = Some(idx);
        let entry = self.entries[idx];
        if entry.file_type != FT_DIR {
            return true;
        }

        let name = entry.name_bytes();
        let name = match core::str::from_utf8(name) {
            Ok(s) => s,
            Err(_) => {
                self.set_error("Invalid directory name");
                return false;
            }
        };
        if let Some(next) = self.current_path.join(name) {
            self.navigate_to(next)
        } else {
            self.set_error("Path is too long");
            false
        }
    }

    fn navigate_to(&mut self, target: PathBuf) -> bool {
        if self.load_directory(target) {
            self.view_mode = ViewMode::Directory;
            self.current_path = target;
            self.clear_error();
            self.selected_row = None;
            self.selected_sidebar = self.sidebar_index_for_path();
            true
        } else {
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

    fn refresh_home_folders(&mut self) {
        self.home_folders = build_home_folders(self.home_path);
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
            4 => self.home_path.join("Music").map(SidebarTarget::Path),
            5 => self.home_path.join("Pictures").map(SidebarTarget::Path),
            6 => self.home_path.join("Videos").map(SidebarTarget::Path),
            7 => self.home_path.join("Templates").map(SidebarTarget::Path),
            8 => self.home_path.join("Public").map(SidebarTarget::Path),
            9 => Some(SidebarTarget::Path(PathBuf::root())),
            10 => Some(SidebarTarget::Volumes),
            11 => Some(SidebarTarget::Network),
            _ => None,
        }
    }

    fn sidebar_label(idx: usize) -> &'static str {
        match idx {
            0 => "Home",
            1 => "Desktop",
            2 => "Documents",
            3 => "Downloads",
            4 => "Music",
            5 => "Pictures",
            6 => "Videos",
            7 => "Templates",
            8 => "Public",
            9 => "Root",
            10 => "Volumes",
            _ => "Network",
        }
    }

    fn sidebar_index_for_path(&self) -> usize {
        match self.view_mode {
            ViewMode::Home => return 0,
            ViewMode::Volumes => return 10,
            ViewMode::Network => return 11,
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
        if let Some(i) = check("Music", 4) {
            return i;
        }
        if let Some(i) = check("Pictures", 5) {
            return i;
        }
        if let Some(i) = check("Videos", 6) {
            return i;
        }
        if let Some(i) = check("Templates", 7) {
            return i;
        }
        if let Some(i) = check("Public", 8) {
            return i;
        }

        if current == "/" {
            return 9;
        }
        if current == "/mnt" || current.starts_with("/mnt/") {
            return 10;
        }
        if current == "/boot" || current.starts_with("/boot/") {
            return 10;
        }
        if current == "/network" || current.starts_with("/network/") {
            return 11;
        }

        9
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

struct FilesApp {
    state: State,
}

impl FilesApp {
    fn new() -> Self {
        Self {
            state: State::new(),
        }
    }

    fn root_layout() -> (Rect, Rect, Rect) {
        let root = Rect::new(0, 0, WIN_W, WIN_H);
        let body_h = WIN_H.saturating_sub(TOOLBAR_H + STATUS_H);
        let heights = [TOOLBAR_H, body_h, STATUS_H];
        let mut rows = VBox::new(root).with_spacing(0).layout(&heights);
        let toolbar = rows.next().unwrap_or_default();
        let body = rows.next().unwrap_or_default();
        let status = rows.next().unwrap_or_default();
        (toolbar, body, status)
    }

    fn body_layout(body: Rect) -> (Rect, Rect) {
        let widths = [SIDEBAR_W, body.w.saturating_sub(SIDEBAR_W + GAP)];
        let mut cols = HBox::new(body).with_spacing(GAP).layout(&widths);
        let sidebar = cols.next().unwrap_or_default();
        let main = cols.next().unwrap_or_default();
        (sidebar, main)
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

    fn sidebar_item_rect(sidebar: Rect, idx: usize) -> Rect {
        let inner = sidebar.inset(PAD);
        let heights = [SIDEBAR_ITEM_H; SIDEBAR_COUNT];
        let mut rows = VBox::new(inner)
            .with_spacing(SIDEBAR_ITEM_GAP)
            .layout(&heights);
        rows.nth(idx).unwrap_or_default()
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
        let title_y = inner.y + 40;
        let grid_top = title_y + 18;
        let card_w = ((inner
            .w
            .saturating_sub((HOME_COLS as u32 - 1) * HOME_CARD_GAP))
            / HOME_COLS as u32)
            .max(120);
        let col = idx % HOME_COLS;
        let row = idx / HOME_COLS;
        let x = inner.x + (col as u32 * (card_w + HOME_CARD_GAP)) as i32;
        let y = grid_top + (row as u32 * (HOME_CARD_H + HOME_CARD_GAP)) as i32;
        Rect::new(x, y, card_w, HOME_CARD_H)
    }

    fn home_volume_rect(inner: Rect, idx: usize, volume_count: usize) -> Option<Rect> {
        if idx >= volume_count {
            return None;
        }
        let title_y =
            inner.y + 40 + 18 + HOME_GRID_ROWS as i32 * (HOME_CARD_H + HOME_CARD_GAP) as i32 + 14;
        let mut y = title_y + 18;
        for _ in 0..idx {
            y += HOME_VOLUME_H as i32 + 8;
        }
        Some(Rect::new(inner.x, y, inner.w, HOME_VOLUME_H))
    }

    fn draw_toolbar(&self, canvas: &mut Canvas, theme: &Theme, toolbar: Rect) {
        canvas.fill_rounded_rect_with_border(toolbar, RADIUS, theme.panel, theme.border, 1);

        let (back, forward, up, search, breadcrumb) = Self::toolbar_layout(toolbar);
        let up_disabled = match self.state.view_mode {
            ViewMode::Home => true,
            ViewMode::Volumes => true,
            ViewMode::Network => true,
            ViewMode::Directory => self.state.current_path.parent().is_none(),
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
            "Forward",
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
        Label::new(
            Rect::new(
                search.x + 22,
                search.y,
                search.w.saturating_sub(30),
                search.h,
            ),
            "Search",
        )
        .dim()
        .draw(canvas, theme);

        canvas.fill_rounded_rect(breadcrumb, RADIUS, theme.panel);
        let crumb = if self.state.view_mode == ViewMode::Home {
            "Home"
        } else if self.state.view_mode == ViewMode::Volumes {
            "Volumes"
        } else if self.state.view_mode == ViewMode::Network {
            "Network"
        } else {
            self.state.current_path.as_str()
        };
        canvas.draw_text(breadcrumb.x + 10, breadcrumb.y + 8, crumb, theme.accent);
    }

    fn draw_sidebar(&self, canvas: &mut Canvas, theme: &Theme, sidebar: Rect) {
        canvas.fill_rounded_rect_with_border(sidebar, RADIUS, theme.panel, theme.border, 1);
        let mut clipped = canvas.sub_canvas(sidebar);
        let inner = sidebar.inset(PAD);
        let inner_local = inner.translate(-sidebar.x, -sidebar.y);
        Label::new(Rect::new(inner_local.x, 10, inner_local.w, 14), "Locations")
            .dim()
            .draw(&mut clipped, theme);

        for idx in 0..SIDEBAR_COUNT {
            let rect = Self::sidebar_item_rect(sidebar, idx);
            let selected = idx == self.state.selected_sidebar;
            let fill = if selected {
                theme.accent.darken(185)
            } else {
                theme.panel_alt
            };
            let border = if selected { theme.accent } else { theme.border };
            let local = rect.translate(-sidebar.x, -sidebar.y);
            clipped.fill_rounded_rect_with_border(local, RADIUS, fill, border, 1);
            if selected {
                clipped.fill_rounded_rect(
                    Rect::new(local.x + 1, local.y + 1, 4, local.h.saturating_sub(2)),
                    RADIUS,
                    theme.accent,
                );
            }
            let icon = sidebar_symbol(idx);
            let icon_color = if selected {
                theme.accent
            } else {
                theme.text_dim
            };
            clipped.draw_ui_symbol(local.x + 8, local.y + 10, icon, icon_color);
            Label::new(
                Rect::new(local.x + 24, local.y + 8, local.w.saturating_sub(34), 12),
                State::sidebar_label(idx),
            )
            .draw(&mut clipped, theme);
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

    fn draw_directory_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        Label::new(
            Rect::new(inner.x, inner.y, inner.w, HEADER_H),
            self.state.current_path.as_str(),
        )
        .draw(canvas, theme);

        if self.state.error_len != 0 {
            Label::new(
                Rect::new(inner.x, inner.y + HEADER_H as i32, inner.w, 14),
                self.state.error_str(),
            )
            .dim()
            .draw(canvas, theme);
        } else {
            Label::new(
                Rect::new(inner.x, inner.y + HEADER_H as i32, inner.w, 14),
                "Name | Type | Size | Modified",
            )
            .dim()
            .draw(canvas, theme);
        }

        self.draw_directory_rows(canvas, theme, main);
    }

    fn draw_home_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        canvas.draw_ui_symbol(inner.x, inner.y + 1, UiSymbol::FilesApp, theme.accent);
        Label::new(
            Rect::new(inner.x + 16, inner.y, inner.w.saturating_sub(16), 18),
            "Home",
        )
        .draw(canvas, theme);
        Label::new(
            Rect::new(inner.x, inner.y + 18, inner.w, 14),
            "Drives and folders will appear here",
        )
        .dim()
        .draw(canvas, theme);

        self.draw_home_folders(canvas, theme, inner);
        let after_volumes = self.draw_home_volumes(canvas, theme, inner, "Volumes");
        self.draw_home_network(canvas, theme, inner, after_volumes);
    }

    fn draw_volumes_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        Label::new(Rect::new(inner.x, inner.y, inner.w, 18), "Volumes").draw(canvas, theme);
        Label::new(
            Rect::new(inner.x, inner.y + 18, inner.w, 14),
            "Mounted filesystems and drives",
        )
        .dim()
        .draw(canvas, theme);
        let _ = self.draw_home_volumes(canvas, theme, inner, "Mounted volumes");
    }

    fn draw_network_main(&self, canvas: &mut Canvas, theme: &Theme, main: Rect) {
        let inner = main.inset(PAD);
        Label::new(Rect::new(inner.x, inner.y, inner.w, 18), "Network").draw(canvas, theme);
        let card = Rect::new(inner.x, inner.y + 28, inner.w, 72);
        canvas.fill_rounded_rect_with_border(card, RADIUS, theme.panel_alt, theme.border, 1);
        canvas.draw_text(
            card.x + 12,
            card.y + 10,
            "No network mounts detected",
            theme.text,
        );
        canvas.draw_text(
            card.x + 12,
            card.y + 28,
            "TODO: list network mounts when VFS metadata is available.",
            theme.text_dim,
        );
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
            Label::new(
                Rect::new(inner.x, list_top + 8, inner.w, 14),
                "No entries in this directory",
            )
            .dim()
            .draw(canvas, theme);
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

            let icon = if entry.file_type == FT_DIR {
                UiSymbol::Folder
            } else {
                UiSymbol::File
            };
            let icon_color = if entry.file_type == FT_DIR {
                theme.accent
            } else {
                theme.text_dim
            };
            canvas.draw_ui_symbol(row.x + 8, row.y + 6, icon, icon_color);

            let name = entry_name_str(&entry);
            canvas.draw_text(row.x + 24, row.y + 5, name, theme.text);

            let type_label = if entry.file_type == FT_DIR {
                "Directory"
            } else if entry.file_type == FT_FILE {
                "File"
            } else {
                "Other"
            };
            canvas.draw_text(type_x + 8, row.y + 5, type_label, theme.text_dim);

            if entry.file_type == FT_DIR {
                canvas.draw_text_right(
                    Rect::new(size_x, row.y, SIZE_W, ROW_H),
                    "--",
                    theme.text,
                    8,
                );
            } else {
                let mut scratch = [0u8; 16];
                let len = write_size(entry.size, &mut scratch);
                let size_text = core::str::from_utf8(&scratch[..len]).unwrap_or("--");
                canvas.draw_text_right(
                    Rect::new(size_x, row.y, SIZE_W, ROW_H),
                    size_text,
                    theme.text,
                    8,
                );
            }

            canvas.draw_text(mod_x + 8, row.y + 5, "--", theme.text_dim);
        }
    }

    fn draw_home_folders(&self, canvas: &mut Canvas, theme: &Theme, inner: Rect) {
        let title_y = inner.y + 40;
        Label::new(
            Rect::new(inner.x, title_y, inner.w, 14),
            "Important folders",
        )
        .dim()
        .draw(canvas, theme);

        let grid_top = title_y + 18;
        let card_w = ((inner
            .w
            .saturating_sub((HOME_COLS as u32 - 1) * HOME_CARD_GAP))
            / HOME_COLS as u32)
            .max(120);
        for idx in 0..HOME_FOLDER_COUNT {
            let col = idx % HOME_COLS;
            let row = idx / HOME_COLS;
            let x = inner.x + (col as u32 * (card_w + HOME_CARD_GAP)) as i32;
            let y = grid_top + (row as u32 * (HOME_CARD_H + HOME_CARD_GAP)) as i32;
            let rect = Rect::new(x, y, card_w, HOME_CARD_H);
            let folder = self.state.home_folders[idx];
            let fill = if folder.present {
                theme.panel_alt
            } else {
                theme.panel.darken(16)
            };
            let border = if folder.present {
                theme.border
            } else {
                theme.border.darken(32)
            };
            canvas.fill_rounded_rect_with_border(rect, RADIUS, fill, border, 1);

            // Draw TGA icon if available, fallback to pixel-art UiSymbol.
            let icon_rect = Rect::new(rect.x + 6, rect.y + 6, 36, 36);
            if let Some(tga) = home_folder_tga(idx, folder.present) {
                canvas.draw_tga_icon(&tga, icon_rect);
            } else {
                let sym = home_folder_symbol(idx, folder.present);
                let sym_color = if folder.present {
                    theme.accent
                } else {
                    theme.warn
                };
                canvas.draw_ui_symbol(rect.x + 10, rect.y + 10, sym, sym_color);
            }

            canvas.draw_text(rect.x + 48, rect.y + 8, folder.name, theme.text);
            canvas.draw_text(
                rect.x + 48,
                rect.y + 24,
                if folder.present {
                    "Available"
                } else {
                    "Missing"
                },
                if folder.present {
                    theme.text_dim
                } else {
                    theme.warn
                },
            );
        }
    }

    fn draw_home_volumes(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        inner: Rect,
        section_title: &'static str,
    ) -> i32 {
        let title_y =
            inner.y + 40 + 18 + HOME_GRID_ROWS as i32 * (HOME_CARD_H + HOME_CARD_GAP) as i32 + 14;
        let section_y = title_y + 18;
        Label::new(Rect::new(inner.x, title_y, inner.w, 14), section_title)
            .dim()
            .draw(canvas, theme);

        let mut y = section_y;
        for idx in 0..self.state.volume_count {
            let volume = self.state.volume_entries[idx];
            let rect = Rect::new(inner.x, y, inner.w, HOME_VOLUME_H);
            y += HOME_VOLUME_H as i32 + 8;
            let fill = if volume.present {
                theme.panel_alt
            } else {
                theme.panel.darken(14)
            };
            canvas.fill_rounded_rect_with_border(rect, RADIUS, fill, theme.border, 1);
            let icon = volume_symbol(&volume);
            canvas.draw_ui_symbol(rect.x + 10, rect.y + 10, icon, theme.text_dim);
            canvas.draw_text(rect.x + 28, rect.y + 8, volume.name_str(), theme.text);
            canvas.draw_text(
                rect.x + 28,
                rect.y + 24,
                volume.path.as_str(),
                theme.text_dim,
            );
            canvas.draw_text(rect.right() - 168, rect.y + 8, "Size --", theme.text_dim);
            canvas.draw_text(
                rect.right() - 168,
                rect.y + 22,
                "Free --  Used --",
                theme.text_dim,
            );
        }

        if self.state.volume_count == 0 {
            Label::new(
                Rect::new(inner.x, section_y + 4, inner.w, 14),
                "No mounted volumes detected",
            )
            .dim()
            .draw(canvas, theme);
        }

        y
    }

    fn draw_home_network(&self, canvas: &mut Canvas, theme: &Theme, inner: Rect, y: i32) {
        Label::new(Rect::new(inner.x, y, inner.w, 14), "Network")
            .dim()
            .draw(canvas, theme);
        canvas.draw_ui_symbol(inner.x, y + 18, UiSymbol::Network, theme.text_dim);
        Label::new(
            Rect::new(inner.x + 16, y + 16, inner.w.saturating_sub(16), 14),
            "No network mounts detected",
        )
        .dim()
        .draw(canvas, theme);
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
        canvas.draw_text(status.x + 12, status.y + 5, count_text, theme.text);
        canvas.draw_text(
            status.x + 160,
            status.y + 5,
            summary,
            if self.state.error_len != 0 {
                theme.danger
            } else {
                theme.text_dim
            },
        );
    }
}

impl App for FilesApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);

        let (toolbar, body, status) = Self::root_layout();
        let (sidebar, main) = Self::body_layout(body);
        self.draw_toolbar(canvas, theme, toolbar);
        self.draw_sidebar(canvas, theme, sidebar);
        self.draw_main(canvas, theme, main);
        self.draw_status(canvas, theme, status);
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Click { x, y } => {
                let (toolbar, body, _) = Self::root_layout();
                let (sidebar, main) = Self::body_layout(body);
                let (_, _, up, _, _) = Self::toolbar_layout(toolbar);
                if up.contains(sunlight_ui::Point::new(x, y)) {
                    return self.state.update(Message::NavigateUp);
                }
                if let Some(idx) = Self::hit_test_sidebar(sidebar, x, y) {
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
                                return self.state.update(Message::OpenHomeFolder(idx));
                            }
                        }
                        for idx in 0..self.state.volume_count {
                            if let Some(rect) =
                                Self::home_volume_rect(inner, idx, self.state.volume_count)
                            {
                                if rect.contains(sunlight_ui::Point::new(x, y)) {
                                    return self.state.update(Message::OpenHomeVolume(idx));
                                }
                            }
                        }
                    }
                    ViewMode::Volumes | ViewMode::Network => {}
                    ViewMode::Directory => {
                        if let Some(idx) = Self::hit_test_row(main, x, y, self.state.entry_count) {
                            return self.state.update(Message::OpenRow(idx));
                        }
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
            } => self.state.update(Message::NavigateUp),
            _ => false,
        }
    }
}

fn detect_home_path() -> PathBuf {
    if let Some(home) = env::getenv(b"HOME") {
        if let Some(path) = PathBuf::from_str(home) {
            if dir_readable(path.as_str()) {
                return path;
            }
        }
    }

    let root = PathBuf::from_str("/root").unwrap_or_else(PathBuf::root);
    if dir_readable(root.as_str()) {
        return root;
    }

    PathBuf::root()
}

fn build_home_folders(home_path: PathBuf) -> [HomeFolder; HOME_FOLDER_COUNT] {
    let names: [&'static str; HOME_FOLDER_COUNT] = [
        "Desktop",
        "Documents",
        "Downloads",
        "Music",
        "Pictures",
        "Videos",
        "Templates",
        "Public",
    ];
    let mut folders = [HomeFolder::empty(); HOME_FOLDER_COUNT];
    let mut i = 0usize;
    while i < HOME_FOLDER_COUNT {
        let path = home_path.join(names[i]).unwrap_or_else(PathBuf::root);
        // Try to create each standard folder if it does not exist yet.
        let present = if libc::stat(path.as_str().as_bytes()).is_ok() {
            true
        } else {
            let ok = libc::mkdir_recursive(path.as_str().as_bytes()).is_ok();
            if !ok {
                debug_log("[FILES] could not create standard folder: ");
                debug_log(names[i]);
                debug_log("\n");
            }
            ok
        };
        folders[i] = HomeFolder {
            name: names[i],
            path,
            present,
        };
        i += 1;
    }
    folders
}

fn build_volumes(out: &mut [VolumeEntry; MAX_ENTRIES]) -> usize {
    let mut count = 0usize;

    if count < out.len() {
        out[count] = make_volume_entry("Root Filesystem", PathBuf::root(), true);
        count += 1;
    }

    if let Some(boot) = PathBuf::from_str("/boot") {
        if libc::stat(boot.as_str().as_bytes()).is_ok() && count < out.len() {
            out[count] = make_volume_entry("Boot", boot, true);
            count += 1;
        }
    }

    let mut mounts = [DirEntry::zeroed(); MAX_ENTRIES];
    if let Ok(found) = libc::read_dir(b"/mnt", &mut mounts) {
        let slice_len = found.min(MAX_ENTRIES);
        mounts[..slice_len].sort_by(compare_entries);
        let mut i = 0usize;
        while i < slice_len && count < out.len() {
            let entry = mounts[i];
            i += 1;
            if entry.file_type != FT_DIR {
                continue;
            }
            let name = core::str::from_utf8(entry.name_bytes()).unwrap_or("");
            if name.is_empty() {
                continue;
            }
            if let Some(path) = PathBuf::from_str("/mnt").and_then(|base| base.join(name)) {
                out[count] = make_volume_entry(name, path, true);
                count += 1;
            }
        }
    }

    count
}

fn make_volume_entry(name: &str, path: PathBuf, present: bool) -> VolumeEntry {
    let mut entry = VolumeEntry::empty();
    let bytes = name.as_bytes();
    let len = bytes.len().min(entry.name.len());
    entry.name[..len].copy_from_slice(&bytes[..len]);
    entry.name_len = len;
    entry.path = path;
    entry.present = present;
    entry
}

fn path_matches(current: &str, base: &str) -> bool {
    current == base
        || (current.starts_with(base) && current.as_bytes().get(base.len()) == Some(&b'/'))
}

fn sidebar_symbol(idx: usize) -> UiSymbol {
    match idx {
        0 => UiSymbol::Home,
        1 => UiSymbol::Desktop,
        2 => UiSymbol::Documents,
        3 => UiSymbol::Downloads,
        4 => UiSymbol::Music,
        5 => UiSymbol::Pictures,
        6 => UiSymbol::Videos,
        7 | 8 => UiSymbol::Folder,
        9 => UiSymbol::RootFs,
        10 => UiSymbol::Volume,
        _ => UiSymbol::Network,
    }
}

fn home_folder_symbol(idx: usize, present: bool) -> UiSymbol {
    if !present {
        return UiSymbol::MissingFolder;
    }
    match idx {
        0 => UiSymbol::Desktop,
        1 => UiSymbol::Documents,
        2 => UiSymbol::Downloads,
        3 => UiSymbol::Music,
        4 => UiSymbol::Pictures,
        5 => UiSymbol::Videos,
        _ => UiSymbol::Folder,
    }
}

fn volume_symbol(volume: &VolumeEntry) -> UiSymbol {
    let path = volume.path.as_str();
    if path == "/" {
        UiSymbol::RootFs
    } else if path == "/boot" {
        UiSymbol::Volume
    } else if path.starts_with("/mnt/") {
        UiSymbol::Volume
    } else {
        UiSymbol::Network
    }
}

fn dir_readable(path: &str) -> bool {
    let mut probe = [DirEntry::zeroed(); 1];
    libc::read_dir(path.as_bytes(), &mut probe).is_ok()
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
    pos += write_number(folders as u64, &mut out[pos..], b" folders");
    if pos < out.len() {
        out[pos] = b',';
        pos += 1;
    }
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
    if let Some(icon) = icon {
        canvas.draw_ui_symbol(rect.x + 8, rect.y + 9, icon, color);
        canvas.draw_text(rect.x + 22, rect.y + 9, text, color);
    } else {
        canvas.draw_text_centered(rect, text, color);
    }
}

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

    let mut app = FilesApp::new();
    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Files",
    }) {
        Some(window) => window,
        None => {
            debug_log("[FILES] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}
