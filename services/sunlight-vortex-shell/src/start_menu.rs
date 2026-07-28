//! SunlightOS Start Menu — dark, structured app launcher overlay.
//!
//! This module owns the Start Menu's data/view model and rendering; it knows
//! nothing about IPC, process launching, or ACPI power calls. `main.rs`
//! drives it (opens/closes it, feeds it live app/recent state) and maps
//! [`StartMenuAction::Launch`] through `open_app_from_ui` — the **same**
//! entry the dock pins, desktop shortcuts, and context-menu items use — so
//! Terminal (and every other app) always launch/focus with identical policy.
//!
//! See `docs/GUI/START_MENU.md` for the architecture writeup, section list,
//! search scope, and documented limitations.

use sun_font::{draw_text_centered, draw_text_vcenter, measure_text, FontRole, TextStyle};
use sunlight_ui::{image::TgaImage, Canvas, Event, Point, Rect, Theme};

use crate::{AppId, AppLaunchState, DockAppState, BOT_H, BOT_Y_OFF, TOP_H, TOP_PAD, TOP_Y};

// ---------------------------------------------------------------------------
// Icons — additive only. Terminal/Settings/Calculator/Files already have
// dock icons in `main.rs`; the Start Menu re-embeds a couple of alternates
// explicitly requested for the larger tile art (Files/Calculator), and adds
// icons for the apps/placeholders that have none today.
// ---------------------------------------------------------------------------

static ICON_TERMINAL_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/utilities-terminal.tga");
static ICON_SETTINGS_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/preferences-system.tga");
static ICON_FILES_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/org.kde.dolphin.tga");
static ICON_CALC_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/galculator.tga");
static ICON_TASKS_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/ksysguard.tga");
static ICON_BENCH_TGA: &[u8] = include_bytes!("../../../docs/icons/SunlightOS/apps/48/cpu-x.tga");
static ICON_TEXT_EDITOR_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/kate.tga");
static ICON_WRITER_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/libreoffice-writer.tga");
static ICON_CALENDAR_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/org.kde.merkuro.calendar.tga");
static ICON_DEVICES_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/hwinfo.tga");
static ICON_RABBIT_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/internet-web-browser.tga");
static ICON_API_LAB_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/48/apifox.tga");
static ICON_MINES_TGA: &[u8] = include_bytes!("../../../docs/icons/SunlightOS/apps/48/bomber.tga");
static ICON_SILICON_ECHOES_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/apps/symbolic/clock-app-symbolic.tga");
static ICON_WELCOME_TGA: &[u8] = include_bytes!("../../../docs/icons/SunlightOS/apps/48/about.tga");

static ICON_SEARCH_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/edit-find-symbolic.tga");
static ICON_LOCK_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/system-lock-screen-symbolic.tga");
static ICON_SLEEP_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/32/system-suspend-symbolic.tga");
static ICON_RESTART_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/system-reboot-symbolic.tga");
static ICON_SHUTDOWN_TGA: &[u8] =
    include_bytes!("../../../docs/icons/SunlightOS/actions/16/system-shutdown-symbolic.tga");

fn icon(bytes: &'static [u8]) -> Option<TgaImage> {
    TgaImage::parse(bytes).ok()
}

const APP_CATALOG_LEN: usize = 16;

#[derive(Clone, Copy)]
struct StartMenuIcons {
    search: Option<TgaImage>,
    power: [Option<TgaImage>; POWER_CAP],
    apps: [Option<TgaImage>; APP_CATALOG_LEN],
}

impl StartMenuIcons {
    fn load() -> Self {
        let mut apps = [None; APP_CATALOG_LEN];
        let mut i = 0usize;
        while i < APP_CATALOG_LEN {
            apps[i] = APP_CATALOG[i].icon_bytes.and_then(icon);
            i += 1;
        }
        Self {
            search: icon(ICON_SEARCH_TGA),
            power: [
                icon(ICON_LOCK_TGA),
                icon(ICON_SLEEP_TGA),
                icon(ICON_RESTART_TGA),
                icon(ICON_SHUTDOWN_TGA),
            ],
            apps,
        }
    }

    fn app_icon(&self, entry: &AppCatalogEntry) -> Option<TgaImage> {
        APP_CATALOG
            .iter()
            .position(|candidate| core::ptr::eq(candidate, entry))
            .and_then(|idx| self.apps[idx])
    }

    fn power_icon(&self, action: PowerAction) -> Option<TgaImage> {
        let idx = match action {
            PowerAction::Lock => 0,
            PowerAction::Sleep => 1,
            PowerAction::Restart => 2,
            PowerAction::Shutdown => 3,
        };
        self.power[idx]
    }
}

// ---------------------------------------------------------------------------
// App catalog
// ---------------------------------------------------------------------------

/// Identifies a Start Menu tile. Real apps reuse the shared [`AppId`] (and
/// its launch/state-sync machinery); placeholder tiles have no backing
/// binary yet and carry only a stable slug for logging/notifications.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CatalogId {
    App(AppId),
    Placeholder(&'static str),
}

pub(crate) struct AppCatalogEntry {
    pub(crate) id: CatalogId,
    pub(crate) name: &'static str,
    pub(crate) category: &'static str,
    icon_bytes: Option<&'static [u8]>,
    pub(crate) available: bool,
}

/// Full "All Apps" catalog — launchable apps. Exactly fills a 5-column × 3-row grid.
static APP_CATALOG: [AppCatalogEntry; APP_CATALOG_LEN] = [
    AppCatalogEntry {
        id: CatalogId::App(AppId::Terminal),
        name: "Terminal",
        category: "Utilities",
        icon_bytes: Some(ICON_TERMINAL_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Chronos),
        name: "Sunlight DOS Terminal",
        category: "Compatibility",
        icon_bytes: Some(ICON_TERMINAL_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Files),
        name: "Files",
        category: "System",
        icon_bytes: Some(ICON_FILES_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Calculator),
        name: "Calculator",
        category: "Utilities",
        icon_bytes: Some(ICON_CALC_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Settings),
        name: "Settings",
        category: "System",
        icon_bytes: Some(ICON_SETTINGS_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Tasks),
        name: "Task Manager",
        category: "System",
        icon_bytes: Some(ICON_TASKS_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Bench),
        name: "Sunlight Bench",
        category: "Utilities",
        icon_bytes: Some(ICON_BENCH_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Calendar),
        name: "Sunlight Calendar",
        category: "Productivity",
        icon_bytes: Some(ICON_CALENDAR_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Devices),
        name: "Sunlight Devices",
        category: "System",
        icon_bytes: Some(ICON_DEVICES_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Writer),
        name: "Sunlight Writer",
        category: "Productivity",
        icon_bytes: Some(ICON_WRITER_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::TextEditor),
        name: "Text Editor",
        category: "Productivity",
        icon_bytes: Some(ICON_TEXT_EDITOR_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::RappidRabbit),
        name: "Rappid Rabbit",
        category: "Network",
        icon_bytes: Some(ICON_RABBIT_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::ApiLab),
        name: "Sunlight API Lab",
        category: "Network",
        icon_bytes: Some(ICON_API_LAB_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Mines),
        name: "Sunlight Mines",
        category: "Games",
        icon_bytes: Some(ICON_MINES_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::SiliconEchoes),
        name: "Silicon Echoes: 1993",
        category: "Games",
        icon_bytes: Some(ICON_SILICON_ECHOES_TGA),
        available: true,
    },
    AppCatalogEntry {
        id: CatalogId::App(AppId::Welcome),
        name: "Welcome to SunlightOS",
        category: "System",
        icon_bytes: Some(ICON_WELCOME_TGA),
        available: true,
    },
];

/// Default pinned apps — only real/working apps. Trivial to extend once
/// persistent user pinning exists (see docs/GUI/START_MENU.md).
static DEFAULT_PINNED: [AppId; 6] = [
    AppId::Terminal,
    AppId::Files,
    AppId::Calculator,
    AppId::Settings,
    AppId::Tasks,
    AppId::Mines,
];

/// A stable, varied discovery row. It deliberately does not change while the
/// menu is open, so drawing and input hit regions always describe the same
/// tiles.
static RANDOM_APPS: [AppId; 6] = [
    AppId::Chronos,
    AppId::Bench,
    AppId::Calendar,
    AppId::Devices,
    AppId::ApiLab,
    AppId::SiliconEchoes,
];

/// Shown in the "Suggested" section as a static fallback until the user has
/// actually launched anything this session (see `VortexShell::recent_apps`).
static SUGGESTED_RECENT: [AppId; 12] = [
    AppId::Files,
    AppId::Terminal,
    AppId::Settings,
    AppId::Calculator,
    AppId::Tasks,
    AppId::Writer,
    AppId::TextEditor,
    AppId::Calendar,
    AppId::RappidRabbit,
    AppId::Bench,
    AppId::Mines,
    AppId::ApiLab,
];

fn find_entry(id: CatalogId) -> Option<&'static AppCatalogEntry> {
    APP_CATALOG.iter().find(|e| e.id == id)
}

fn find_app_entry(id: AppId) -> Option<&'static AppCatalogEntry> {
    find_entry(CatalogId::App(id))
}

fn entry_matches(entry: &AppCatalogEntry, query_lower: &str) -> bool {
    contains_ignore_case(entry.name, query_lower)
        || contains_ignore_case(entry.category, query_lower)
}

fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if needle_lower.len() > haystack.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle_lower.len())
        .any(|window| window.eq_ignore_ascii_case(needle_lower.as_bytes()))
}

// ---------------------------------------------------------------------------
// Power actions
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PowerAction {
    Lock,
    Sleep,
    Restart,
    Shutdown,
}

impl PowerAction {
    fn label(self) -> &'static str {
        match self {
            Self::Lock => "Lock",
            Self::Sleep => "Sleep",
            Self::Restart => "Restart",
            Self::Shutdown => "Shut Down",
        }
    }

    /// Destructive actions (never return once issued) require a confirm
    /// click; Sleep is a no-op placeholder today so it fires immediately.
    fn needs_confirm(self) -> bool {
        !matches!(self, Self::Sleep | Self::Lock)
    }
}

const POWER_ACTIONS: [PowerAction; 4] = [
    PowerAction::Lock,
    PowerAction::Sleep,
    PowerAction::Restart,
    PowerAction::Shutdown,
];
const CONFIRM_WINDOW_MS: u64 = 3_000;

// ---------------------------------------------------------------------------
// Search field — small in-file text input (mirrors sunlight-ui's TextInput
// edit logic) drawn with the Start Menu's own rounded style.
// ---------------------------------------------------------------------------

const SEARCH_BUF: usize = 48;

struct SearchField {
    buf: [u8; SEARCH_BUF],
    len: usize,
    cursor: usize,
    active: bool,
}

impl SearchField {
    const fn new() -> Self {
        Self {
            buf: [0; SEARCH_BUF],
            len: 0,
            cursor: 0,
            active: false,
        }
    }

    fn value(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn clear(&mut self) {
        self.len = 0;
        self.cursor = 0;
    }

    fn insert(&mut self, byte: u8) -> bool {
        if self.len >= SEARCH_BUF {
            return false;
        }
        let mut i = self.len;
        while i > self.cursor {
            self.buf[i] = self.buf[i - 1];
            i -= 1;
        }
        self.buf[self.cursor] = byte;
        self.len += 1;
        self.cursor += 1;
        true
    }

    fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let mut i = self.cursor - 1;
        while i + 1 < self.len {
            self.buf[i] = self.buf[i + 1];
            i += 1;
        }
        self.len -= 1;
        self.cursor -= 1;
        true
    }

    fn delete_forward(&mut self) -> bool {
        if self.cursor >= self.len {
            return false;
        }
        let mut i = self.cursor;
        while i + 1 < self.len {
            self.buf[i] = self.buf[i + 1];
            i += 1;
        }
        self.len -= 1;
        true
    }

    fn move_left(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        true
    }

    fn move_right(&mut self) -> bool {
        if self.cursor >= self.len {
            return false;
        }
        self.cursor += 1;
        true
    }

    fn move_home(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor = 0;
        true
    }

    fn move_end(&mut self) -> bool {
        if self.cursor == self.len {
            return false;
        }
        self.cursor = self.len;
        true
    }

    /// Handle a decoded character event (mirrors `TextInput::update`'s
    /// `Event::Key` arm: backspace, printable insert, ignore Enter).
    fn handle_char(&mut self, ch: char) -> bool {
        match ch {
            '\u{8}' => self.backspace(),
            '\n' | '\r' => false,
            c if c.is_ascii_graphic() || c == ' ' => self.insert(c as u8),
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// Layout — a pure function of (screen size, query, recent list). Computed
// fresh on every draw *and* every event so hit-testing is never stale.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct TileSlot {
    rect: Rect,
    entry: &'static AppCatalogEntry,
}

#[derive(Clone, Copy)]
struct PowerSlot {
    rect: Rect,
    action: PowerAction,
}

#[derive(Clone, Copy)]
struct ButtonSlot {
    rect: Rect,
}

#[derive(Clone, Copy)]
struct FixedList<T: Copy, const N: usize> {
    items: [Option<T>; N],
    len: usize,
}

impl<T: Copy, const N: usize> FixedList<T, N> {
    const fn new() -> Self {
        Self {
            items: [None; N],
            len: 0,
        }
    }

    fn push(&mut self, item: T) {
        if self.len < N {
            self.items[self.len] = Some(item);
            self.len += 1;
        }
    }

    fn len(&self) -> usize {
        self.len
    }

    fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn get(&self, idx: usize) -> Option<&T> {
        if idx >= self.len {
            None
        } else {
            self.items[idx].as_ref()
        }
    }

    fn iter(&self) -> impl Iterator<Item = &T> {
        self.items[..self.len]
            .iter()
            .filter_map(|item| item.as_ref())
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.items[..self.len]
            .iter_mut()
            .filter_map(|item| item.as_mut())
    }
}

const PINNED_CAP: usize = 6;
const RANDOM_CAP: usize = 6;
const RECENT_CAP: usize = 12;
const SEARCH_RESULTS_CAP: usize = 15;
const POWER_CAP: usize = 4;

struct StartMenuLayout {
    panel: Rect,
    header: Rect,
    close_btn: Rect,
    back_btn: Option<ButtonSlot>,
    search_rect: Rect,
    searching: bool,
    page: StartMenuPage,
    pinned_label: Rect,
    pinned: FixedList<TileSlot, PINNED_CAP>,
    random_label: Rect,
    random: FixedList<TileSlot, RANDOM_CAP>,
    all_apps_button: Option<ButtonSlot>,
    all_apps: FixedList<TileSlot, ALL_APPS_PAGE_CAP>,
    all_apps_page: usize,
    page_dots: FixedList<Rect, ALL_APPS_MAX_PAGES>,
    recent_label: Rect,
    recent: FixedList<TileSlot, RECENT_CAP>,
    recent_is_real: bool,
    results_label: Rect,
    search_results: FixedList<TileSlot, SEARCH_RESULTS_CAP>,
    footer_divider_y: i32,
    user_rect: Rect,
    power: FixedList<PowerSlot, POWER_CAP>,
}

impl StartMenuLayout {
    fn tile_count(&self) -> usize {
        if self.searching {
            self.search_results.len()
        } else if self.page == StartMenuPage::AllApps {
            self.all_apps.len()
        } else {
            self.pinned.len() + self.random.len() + self.recent.len()
        }
    }

    /// All currently visible/clickable tiles in a fixed, stable order:
    /// search results (while searching), All Apps, or Home's pinned → recent.
    /// Keyboard selection indices and mouse-hit-testing both index into
    /// this same ordering.
    fn tile(&self, idx: usize) -> Option<&TileSlot> {
        if self.searching {
            return self.search_results.get(idx);
        }
        if self.page == StartMenuPage::AllApps {
            return self.all_apps.get(idx);
        }
        if idx < self.pinned.len() {
            return self.pinned.get(idx);
        }
        let idx = idx - self.pinned.len();
        if idx < self.random.len() {
            return self.random.get(idx);
        }
        let idx = idx - self.random.len();
        self.recent.get(idx)
    }

    fn tile_index_at(&self, p: Point) -> Option<usize> {
        (0..self.tile_count()).find(|&idx| self.tile(idx).is_some_and(|slot| slot.rect.contains(p)))
    }

    fn tile_at_point(&self, p: Point) -> Option<&TileSlot> {
        self.tile_index_at(p).and_then(|idx| self.tile(idx))
    }
}

const PANEL_W: u32 = 720;
const PANEL_PAD: i32 = 16;
const HEADER_H: u32 = 34;
const CLOSE_BTN: u32 = 22;
const SEARCH_H: u32 = 38;
const SECTION_GAP: i32 = 12;
const LABEL_H: u32 = 16;
const LABEL_GAP: i32 = 6;
const TILE_GAP: i32 = 10;
const SMALL_TILE_H: u32 = 70;
const BIG_TILE_H: u32 = 78;
const SMALL_COLS: usize = 6;
const BIG_COLS: usize = 5;
/// A bounded four-row application grid. Only this page's cells and hit
/// regions are laid out, regardless of how large the catalog becomes.
const ALL_APPS_PAGE_CAP: usize = BIG_COLS * 4;
const ALL_APPS_MAX_PAGES: usize = APP_CATALOG_LEN.div_ceil(ALL_APPS_PAGE_CAP);
const FOOTER_GAP: i32 = 12;
const FOOTER_H: u32 = 48;
const POWER_BTN: u32 = 44;
const POWER_GAP: i32 = 10;
const ALL_APPS_BUTTON_H: u32 = 34;
const PAGE_DOT: u32 = 8;
const PAGE_DOT_GAP: i32 = 10;

#[derive(Clone, Copy, PartialEq, Eq)]
enum StartMenuPage {
    Home,
    AllApps,
}

fn page_count(item_count: usize, page_capacity: usize) -> usize {
    if item_count == 0 || page_capacity == 0 {
        0
    } else {
        item_count.div_ceil(page_capacity)
    }
}

fn all_apps_page_count() -> usize {
    page_count(APP_CATALOG_LEN, ALL_APPS_PAGE_CAP)
}

fn row_tile_w(content_w: u32, cols: usize) -> u32 {
    if cols == 0 {
        return content_w;
    }
    let gaps = (cols.saturating_sub(1)) as u32 * TILE_GAP as u32;
    content_w.saturating_sub(gaps) / cols as u32
}

fn layout_tile_row<const N: usize>(
    entries: impl Iterator<Item = &'static AppCatalogEntry>,
    x0: i32,
    y: i32,
    tile_w: u32,
    tile_h: u32,
) -> FixedList<TileSlot, N> {
    let mut slots = FixedList::new();
    for (i, entry) in entries.enumerate() {
        if i >= N {
            break;
        }
        slots.push(TileSlot {
            rect: Rect::new(
                x0 + i as i32 * (tile_w as i32 + TILE_GAP),
                y,
                tile_w,
                tile_h,
            ),
            entry,
        });
    }
    slots
}

fn layout_tile_grid<const N: usize>(
    entries: impl Iterator<Item = &'static AppCatalogEntry>,
    x0: i32,
    y0: i32,
    tile_w: u32,
    tile_h: u32,
    cols: usize,
) -> (FixedList<TileSlot, N>, u32 /* total height used */) {
    let cols = cols.max(1);
    let mut slots = FixedList::new();
    let mut count = 0usize;
    for entry in entries {
        if count >= N {
            break;
        }
        let col = (count % cols) as i32;
        let row = (count / cols) as i32;
        slots.push(TileSlot {
            rect: Rect::new(
                x0 + col * (tile_w as i32 + TILE_GAP),
                y0 + row * (tile_h as i32 + TILE_GAP),
                tile_w,
                tile_h,
            ),
            entry,
        });
        count += 1;
    }
    let rows = count.div_ceil(cols);
    let height = if rows == 0 {
        0
    } else {
        rows as u32 * tile_h + (rows.saturating_sub(1)) as u32 * TILE_GAP as u32
    };
    (slots, height)
}

fn translate_tiles<const N: usize>(tiles: &mut FixedList<TileSlot, N>, dx: i32, dy: i32) {
    for slot in tiles.iter_mut() {
        slot.rect = slot.rect.translate(dx, dy);
    }
}

fn translate_power<const N: usize>(power: &mut FixedList<PowerSlot, N>, dx: i32, dy: i32) {
    for slot in power.iter_mut() {
        slot.rect = slot.rect.translate(dx, dy);
    }
}

fn compute_layout(
    screen_w: u32,
    screen_h: u32,
    query: &str,
    recent_ids: &[AppId],
    page: StartMenuPage,
    all_apps_page: usize,
) -> StartMenuLayout {
    let panel_w = PANEL_W.min(screen_w.saturating_sub((TOP_PAD * 2) as u32).max(320));
    let content_w = panel_w.saturating_sub((PANEL_PAD * 2) as u32);
    let x0 = PANEL_PAD;

    let query = query.trim();
    let searching = !query.is_empty();

    let mut y = PANEL_PAD;
    let header = Rect::new(x0, y, content_w, HEADER_H);
    let close_btn = Rect::new(
        x0 + content_w as i32 - CLOSE_BTN as i32,
        y + (HEADER_H as i32 - CLOSE_BTN as i32) / 2,
        CLOSE_BTN,
        CLOSE_BTN,
    );
    let back_btn = if page == StartMenuPage::AllApps {
        Some(ButtonSlot {
            rect: Rect::new(
                x0,
                y + (HEADER_H as i32 - CLOSE_BTN as i32) / 2,
                CLOSE_BTN,
                CLOSE_BTN,
            ),
        })
    } else {
        None
    };
    y += HEADER_H as i32 + SECTION_GAP;

    let search_rect = Rect::new(x0, y, content_w, SEARCH_H);
    y += SEARCH_H as i32 + SECTION_GAP;

    let mut pinned_label = Rect::default();
    let mut pinned = FixedList::new();
    let mut random_label = Rect::default();
    let mut random = FixedList::new();
    let mut all_apps_button = None;
    let mut all_apps = FixedList::new();
    let all_apps_page_count = all_apps_page_count();
    let all_apps_page = if all_apps_page_count == 0 {
        0
    } else {
        all_apps_page.min(all_apps_page_count - 1)
    };
    let mut page_dots = FixedList::new();
    let mut recent_label = Rect::default();
    let mut recent = FixedList::new();
    let mut recent_is_real = false;
    let mut results_label = Rect::default();
    let mut search_results = FixedList::new();

    if searching {
        results_label = Rect::new(x0, y, content_w, LABEL_H);
        y += LABEL_H as i32 + LABEL_GAP;
        let tile_w = row_tile_w(content_w, BIG_COLS);
        let matches = APP_CATALOG.iter().filter(|e| entry_matches(e, query));
        let (slots, height) = layout_tile_grid(matches, x0, y, tile_w, BIG_TILE_H, BIG_COLS);
        search_results = slots;
        y += height.max(BIG_TILE_H) as i32 + SECTION_GAP;
    } else if page == StartMenuPage::AllApps {
        let tile_w = row_tile_w(content_w, BIG_COLS);
        let start = all_apps_page * ALL_APPS_PAGE_CAP;
        let entries = APP_CATALOG.iter().skip(start).take(ALL_APPS_PAGE_CAP);
        let (slots, height) = layout_tile_grid(entries, x0, y, tile_w, BIG_TILE_H, BIG_COLS);
        all_apps = slots;
        y += height.max(BIG_TILE_H) as i32 + SECTION_GAP;

        if all_apps_page_count > 1 {
            let dots_w = all_apps_page_count as i32 * PAGE_DOT as i32
                + (all_apps_page_count.saturating_sub(1)) as i32 * PAGE_DOT_GAP;
            let mut dot_x = x0 + (content_w as i32 - dots_w) / 2;
            for _ in 0..all_apps_page_count {
                page_dots.push(Rect::new(dot_x, y, PAGE_DOT, PAGE_DOT));
                dot_x += PAGE_DOT as i32 + PAGE_DOT_GAP;
            }
            y += PAGE_DOT as i32 + SECTION_GAP;
        }
    } else {
        // Pinned
        pinned_label = Rect::new(x0, y, content_w, LABEL_H);
        y += LABEL_H as i32 + LABEL_GAP;
        let tile_w = row_tile_w(content_w, SMALL_COLS);
        let pinned_entries = DEFAULT_PINNED.iter().filter_map(|id| find_app_entry(*id));
        pinned = layout_tile_row(pinned_entries, x0, y, tile_w, SMALL_TILE_H);
        y += SMALL_TILE_H as i32 + SECTION_GAP;

        // Random Apps
        random_label = Rect::new(x0, y, content_w, LABEL_H);
        y += LABEL_H as i32 + LABEL_GAP;
        let random_entries = RANDOM_APPS.iter().filter_map(|id| find_app_entry(*id));
        random = layout_tile_row(random_entries, x0, y, tile_w, SMALL_TILE_H);
        y += SMALL_TILE_H as i32 + SECTION_GAP;

        // Home intentionally exposes navigation, not the full catalog. The
        // complete grid lives in the separate paged All Apps view below.
        all_apps_button = Some(ButtonSlot {
            rect: Rect::new(x0, y, content_w, ALL_APPS_BUTTON_H),
        });
        y += ALL_APPS_BUTTON_H as i32 + SECTION_GAP;

        // Recent / Suggested
        recent_label = Rect::new(x0, y, content_w, LABEL_H);
        y += LABEL_H as i32 + LABEL_GAP;
        recent_is_real = !recent_ids.is_empty();
        let tile_w = row_tile_w(content_w, SMALL_COLS);
        let fallback = &SUGGESTED_RECENT[..];
        let source: &[AppId] = if recent_is_real { recent_ids } else { fallback };
        let recent_entries = source
            .iter()
            .take(RECENT_CAP)
            .filter_map(|id| find_app_entry(*id));
        let (slots, height) =
            layout_tile_grid(recent_entries, x0, y, tile_w, SMALL_TILE_H, SMALL_COLS);
        recent = slots;
        y += height.max(SMALL_TILE_H) as i32 + SECTION_GAP;
    }

    let footer_divider_y = y;
    y += FOOTER_GAP;
    let power_total_w = POWER_ACTIONS.len() as u32 * POWER_BTN
        + (POWER_ACTIONS.len().saturating_sub(1)) as u32 * POWER_GAP as u32;
    let power_x0 = x0 + content_w as i32 - power_total_w as i32;
    let mut power = FixedList::new();
    for (i, action) in POWER_ACTIONS.iter().enumerate() {
        power.push(PowerSlot {
            rect: Rect::new(
                power_x0 + i as i32 * (POWER_BTN as i32 + POWER_GAP),
                y,
                POWER_BTN,
                FOOTER_H,
            ),
            action: *action,
        });
    }
    let user_rect = Rect::new(x0, y, (power_x0 - x0 - 12).max(0) as u32, FOOTER_H);
    y += FOOTER_H as i32 + PANEL_PAD;

    let panel_h = y as u32;

    // Bottom-anchor above the dock (same 10px gap convention as
    // `desktop_area`), clamped so it never crowds the top bar on short
    // screens (documented limitation: very small screens may overlap the
    // dock slightly rather than scroll — see docs/GUI/START_MENU.md).
    let desired_bottom = screen_h as i32 - BOT_Y_OFF - BOT_H as i32 - 10;
    let min_top = TOP_Y + TOP_H as i32 + 8;
    let top = (desired_bottom - panel_h as i32).max(min_top);
    let panel = Rect::new(TOP_PAD, top, panel_w, panel_h);

    let dx = panel.x;
    let dy = panel.y;
    let shift = |r: Rect| r.translate(dx, dy);
    translate_tiles(&mut pinned, dx, dy);
    translate_tiles(&mut random, dx, dy);
    translate_tiles(&mut all_apps, dx, dy);
    translate_tiles(&mut recent, dx, dy);
    translate_tiles(&mut search_results, dx, dy);
    translate_power(&mut power, dx, dy);
    let back_btn = back_btn.map(|slot| ButtonSlot {
        rect: slot.rect.translate(dx, dy),
    });
    let all_apps_button = all_apps_button.map(|slot| ButtonSlot {
        rect: slot.rect.translate(dx, dy),
    });
    for dot in page_dots.iter_mut() {
        *dot = dot.translate(dx, dy);
    }

    StartMenuLayout {
        panel,
        header: shift(header),
        close_btn: shift(close_btn),
        back_btn,
        search_rect: shift(search_rect),
        searching,
        page,
        pinned_label: shift(pinned_label),
        pinned,
        random_label: shift(random_label),
        random,
        all_apps_button,
        all_apps,
        all_apps_page,
        page_dots,
        recent_label: shift(recent_label),
        recent,
        recent_is_real,
        results_label: shift(results_label),
        search_results,
        footer_divider_y: dy + footer_divider_y,
        user_rect: shift(user_rect),
        power,
    }
}

// ---------------------------------------------------------------------------
// Public state + actions
// ---------------------------------------------------------------------------

/// What the Start Menu wants the shell to do in response to an event.
/// `start_menu.rs` never performs IPC/launch/syscalls itself.
pub(crate) enum StartMenuAction {
    None,
    /// Pointer dismissed the menu from outside the panel. The shell keeps
    /// this consumed so the same gesture does not leak through to desktop,
    /// dock, or other shell chrome underneath.
    DismissedOutside {
        x: i32,
        y: i32,
    },
    Launch(AppId),
    Unavailable(&'static str),
    Power(PowerAction),
}

pub(crate) struct StartMenuState {
    is_open: bool,
    icons: StartMenuIcons,
    search: SearchField,
    page: StartMenuPage,
    all_apps_page: usize,
    selected: Option<usize>,
    hover: Option<usize>,
    confirm: Option<(PowerAction, u64)>,
}

impl StartMenuState {
    pub(crate) fn new() -> Self {
        Self {
            is_open: false,
            icons: StartMenuIcons::load(),
            search: SearchField::new(),
            page: StartMenuPage::Home,
            all_apps_page: 0,
            selected: None,
            hover: None,
            confirm: None,
        }
    }

    pub(crate) fn is_open(&self) -> bool {
        self.is_open
    }

    pub(crate) fn open_menu(&mut self) {
        self.is_open = true;
        self.search.clear();
        self.search.active = true;
        self.page = StartMenuPage::Home;
        self.all_apps_page = 0;
        self.selected = None;
        self.hover = None;
        self.confirm = None;
    }

    pub(crate) fn close(&mut self) {
        self.is_open = false;
        self.search.active = false;
        self.page = StartMenuPage::Home;
        self.all_apps_page = 0;
        self.selected = None;
        self.hover = None;
        self.confirm = None;
    }

    fn move_selection(&mut self, delta: i32, len: usize) {
        if len == 0 {
            self.selected = None;
            return;
        }
        let cur = self
            .selected
            .map(|idx| idx.min(len - 1) as i32)
            .unwrap_or_else(|| if delta < 0 { len as i32 - 1 } else { 0 });
        let next = (cur + delta).rem_euclid(len as i32);
        self.selected = Some(next as usize);
    }

    /// Process one input event while the menu is open. Returns
    /// `(redraw_needed, action)`. Callers should check `is_open()` first —
    /// this always reports `redraw_needed = true` while closed is irrelevant
    /// because it early-returns `(false, None)`.
    pub(crate) fn handle_event(
        &mut self,
        event: Event,
        screen_w: u32,
        screen_h: u32,
        recent: &[AppId],
        now: u64,
    ) -> (bool, StartMenuAction) {
        if !self.is_open {
            return (false, StartMenuAction::None);
        }
        if let Some((_, expires)) = self.confirm {
            if now >= expires {
                self.confirm = None;
            }
        }

        let layout = compute_layout(
            screen_w,
            screen_h,
            self.search.value(),
            recent,
            self.page,
            self.all_apps_page,
        );

        match event {
            Event::Click { x, y } => {
                let p = Point::new(x, y);
                if !layout.panel.contains(p) {
                    self.close();
                    return (true, StartMenuAction::DismissedOutside { x, y });
                }
                if layout.close_btn.contains(p) {
                    self.close();
                    return (true, StartMenuAction::None);
                }
                if layout.back_btn.is_some_and(|slot| slot.rect.contains(p)) {
                    self.show_home();
                    return (true, StartMenuAction::None);
                }
                if layout.search_rect.contains(p) {
                    self.search.active = true;
                    return (true, StartMenuAction::None);
                }
                if layout
                    .all_apps_button
                    .is_some_and(|slot| slot.rect.contains(p))
                {
                    self.show_all_apps();
                    return (true, StartMenuAction::None);
                }
                for (index, dot) in layout.page_dots.iter().enumerate() {
                    if dot.contains(p) {
                        self.set_all_apps_page(index);
                        return (true, StartMenuAction::None);
                    }
                }
                for slot in layout.power.iter() {
                    if slot.rect.contains(p) {
                        return (true, self.click_power(slot.action, now));
                    }
                }
                if let Some(slot) = layout.tile_at_point(p) {
                    if !slot.entry.available {
                        return (true, StartMenuAction::Unavailable(slot.entry.name));
                    }
                    let CatalogId::App(id) = slot.entry.id else {
                        return (true, StartMenuAction::Unavailable(slot.entry.name));
                    };
                    self.close();
                    return (true, StartMenuAction::Launch(id));
                }
                // Click landed on panel padding/labels/dividers — swallow it.
                (true, StartMenuAction::None)
            }
            // Close immediately on an outside press so the overlay disappears
            // before the click completes; the shell consumes the follow-up
            // click to avoid click-through into content underneath.
            Event::MouseDown { x, y, button } => {
                let p = Point::new(x, y);
                if !layout.panel.contains(p) {
                    self.close();
                    return (true, StartMenuAction::DismissedOutside { x, y });
                }
                let _ = button;
                (true, StartMenuAction::None)
            }
            Event::MouseUp { .. } => (true, StartMenuAction::None),
            Event::MouseMove { x, y } => {
                let p = Point::new(x, y);
                let prev = self.hover;
                self.hover = if layout.panel.contains(p) {
                    layout.tile_index_at(p)
                } else {
                    None
                };
                (self.hover != prev, StartMenuAction::None)
            }
            Event::Key(ch) => {
                if !self.search.active {
                    return (false, StartMenuAction::None);
                }
                let changed = self.search.handle_char(ch);
                if changed {
                    self.selected = None;
                }
                (changed, StartMenuAction::None)
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } => self.handle_keypress(keycode, &layout),
            _ => (false, StartMenuAction::None),
        }
    }

    fn click_power(&mut self, action: PowerAction, now: u64) -> StartMenuAction {
        if !action.needs_confirm() {
            return StartMenuAction::Power(action);
        }
        match self.confirm {
            Some((pending, expires)) if pending == action && now < expires => {
                self.confirm = None;
                self.close();
                StartMenuAction::Power(action)
            }
            _ => {
                self.confirm = Some((action, now.saturating_add(CONFIRM_WINDOW_MS)));
                StartMenuAction::None
            }
        }
    }

    fn show_home(&mut self) {
        self.page = StartMenuPage::Home;
        self.all_apps_page = 0;
        self.selected = None;
        self.hover = None;
    }

    fn show_all_apps(&mut self) {
        self.page = StartMenuPage::AllApps;
        self.all_apps_page = 0;
        self.selected = None;
        self.hover = None;
    }

    fn set_all_apps_page(&mut self, page: usize) -> bool {
        let count = all_apps_page_count();
        if count == 0 {
            return false;
        }
        let page = page.min(count - 1);
        if self.all_apps_page == page {
            return false;
        }
        self.all_apps_page = page;
        self.selected = None;
        self.hover = None;
        true
    }

    fn move_all_apps_page(&mut self, delta: i32) -> bool {
        let count = all_apps_page_count();
        if self.page != StartMenuPage::AllApps || count <= 1 {
            return false;
        }
        let next = (self.all_apps_page as i32 + delta).rem_euclid(count as i32) as usize;
        self.set_all_apps_page(next)
    }

    fn handle_keypress(
        &mut self,
        keycode: u8,
        layout: &StartMenuLayout,
    ) -> (bool, StartMenuAction) {
        const KEY_ESC: u8 = 0x01;
        const KEY_ENTER: u8 = 0x1C;
        const KEY_UP: u8 = 0x48;
        const KEY_DOWN: u8 = 0x50;
        const KEY_LEFT: u8 = 0x4B;
        const KEY_RIGHT: u8 = 0x4D;
        const KEY_HOME: u8 = 0x47;
        const KEY_END: u8 = 0x4F;
        const KEY_PAGE_UP: u8 = 0x49;
        const KEY_PAGE_DOWN: u8 = 0x51;
        const KEY_DELETE: u8 = 0x53;

        match keycode {
            KEY_ESC => {
                if !self.search.value().is_empty() {
                    self.search.clear();
                    self.selected = None;
                } else if self.page == StartMenuPage::AllApps {
                    self.show_home();
                } else {
                    self.close();
                }
                (true, StartMenuAction::None)
            }
            KEY_ENTER => {
                if let Some(slot) = self
                    .selected
                    .and_then(|idx| layout.tile(idx))
                    .or_else(|| layout.tile(0))
                {
                    if !slot.entry.available {
                        return (true, StartMenuAction::Unavailable(slot.entry.name));
                    }
                    if let CatalogId::App(id) = slot.entry.id {
                        self.close();
                        return (true, StartMenuAction::Launch(id));
                    }
                }
                (false, StartMenuAction::None)
            }
            KEY_UP => {
                self.move_selection(-1, layout.tile_count());
                (true, StartMenuAction::None)
            }
            KEY_DOWN => {
                self.move_selection(1, layout.tile_count());
                (true, StartMenuAction::None)
            }
            KEY_LEFT => {
                if self.search.active && self.search.move_left() {
                    (true, StartMenuAction::None)
                } else if self.move_all_apps_page(-1) {
                    (true, StartMenuAction::None)
                } else if !self.search.active {
                    self.move_selection(-1, layout.tile_count());
                    (true, StartMenuAction::None)
                } else {
                    (false, StartMenuAction::None)
                }
            }
            KEY_RIGHT => {
                if self.search.active && self.search.move_right() {
                    (true, StartMenuAction::None)
                } else if self.move_all_apps_page(1) {
                    (true, StartMenuAction::None)
                } else if !self.search.active {
                    self.move_selection(1, layout.tile_count());
                    (true, StartMenuAction::None)
                } else {
                    (false, StartMenuAction::None)
                }
            }
            KEY_HOME if self.search.active => (self.search.move_home(), StartMenuAction::None),
            KEY_END if self.search.active => (self.search.move_end(), StartMenuAction::None),
            KEY_PAGE_UP => (self.move_all_apps_page(-1), StartMenuAction::None),
            KEY_PAGE_DOWN => (self.move_all_apps_page(1), StartMenuAction::None),
            KEY_DELETE if self.search.active => {
                (self.search.delete_forward(), StartMenuAction::None)
            }
            _ => (false, StartMenuAction::None),
        }
    }

    /// Draw the menu. No-op when closed. Reads `apps` only for a small
    /// running/minimized accent marker on tiles — no IPC happens here.
    pub(crate) fn view(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        screen_w: u32,
        screen_h: u32,
        apps: &[DockAppState],
        recent: &[AppId],
        now: u64,
    ) {
        if !self.is_open {
            return;
        }
        let layout = compute_layout(
            screen_w,
            screen_h,
            self.search.value(),
            recent,
            self.page,
            self.all_apps_page,
        );

        canvas.fill_material(
            layout.panel,
            sunlight_ui::MaterialPalette::new(theme).overlay_glass,
        );

        canvas.fill_rounded_rect(layout.close_btn, 6, theme.panel_alt);
        draw_text_centered(
            canvas,
            layout.close_btn,
            "x",
            &TextStyle::new(FontRole::UiRegular, theme.text_dim),
        );
        let title_x = layout.header.x;
        draw_text_vcenter(
            canvas,
            if layout.page == StartMenuPage::AllApps && !layout.searching {
                "All Apps"
            } else {
                "SunlightOS"
            },
            if layout.back_btn.is_some() {
                title_x + CLOSE_BTN as i32 + 10
            } else {
                title_x
            },
            layout.header.y,
            layout.header.h,
            &TextStyle::new(FontRole::UiTitle, theme.text),
        );
        if let Some(back) = layout.back_btn {
            canvas.fill_rounded_rect(back.rect, 6, theme.panel_alt);
            draw_text_centered(
                canvas,
                back.rect,
                "<",
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
        }

        self.draw_search(canvas, theme, layout.search_rect);

        let mut idx = 0usize;
        if layout.searching {
            draw_section_label(canvas, theme, layout.results_label, "Results");
            for slot in layout.search_results.iter() {
                self.draw_tile(canvas, theme, slot, apps, idx);
                idx += 1;
            }
            if layout.search_results.is_empty() {
                draw_text_centered(
                    canvas,
                    Rect::new(
                        layout.panel.x + PANEL_PAD,
                        layout.results_label.bottom() + 24,
                        layout.panel.w.saturating_sub((PANEL_PAD * 2) as u32),
                        20,
                    ),
                    "No matches",
                    &TextStyle::new(FontRole::UiRegular, theme.text_dim),
                );
            }
        } else if layout.page == StartMenuPage::AllApps {
            for slot in layout.all_apps.iter() {
                self.draw_tile(canvas, theme, slot, apps, idx);
                idx += 1;
            }
            for (page_index, dot) in layout.page_dots.iter().enumerate() {
                if page_index == layout.all_apps_page {
                    canvas.fill_rounded_rect(*dot, PAGE_DOT / 2, theme.accent);
                } else {
                    canvas.stroke_rounded_rect(*dot, PAGE_DOT / 2, 1, theme.text_dim);
                }
            }
        } else {
            draw_section_label(canvas, theme, layout.pinned_label, "Pinned");
            for slot in layout.pinned.iter() {
                self.draw_tile(canvas, theme, slot, apps, idx);
                idx += 1;
            }
            draw_section_label(canvas, theme, layout.random_label, "Random Apps");
            for slot in layout.random.iter() {
                self.draw_tile(canvas, theme, slot, apps, idx);
                idx += 1;
            }
            if let Some(button) = layout.all_apps_button {
                canvas.fill_rounded_rect(button.rect, 8, theme.panel_alt);
                canvas.stroke_rounded_rect(button.rect, 8, 1, theme.border);
                draw_text_centered(
                    canvas,
                    button.rect,
                    "All Apps",
                    &TextStyle::new(FontRole::UiRegular, theme.text),
                );
            }
            let recent_title = if layout.recent_is_real {
                "Recent"
            } else {
                "Suggested"
            };
            draw_section_label(canvas, theme, layout.recent_label, recent_title);
            for slot in layout.recent.iter() {
                self.draw_tile(canvas, theme, slot, apps, idx);
                idx += 1;
            }
        }

        canvas.hline(
            layout.panel.x + PANEL_PAD,
            layout.footer_divider_y,
            layout.panel.w.saturating_sub((PANEL_PAD * 2) as u32),
            theme.border,
        );

        self.draw_user(canvas, theme, layout.user_rect);
        for slot in layout.power.iter() {
            let confirming =
                matches!(self.confirm, Some((a, exp)) if a == slot.action && now < exp);
            draw_power_button(
                canvas,
                theme,
                slot.rect,
                self.icons.power_icon(slot.action),
                slot.action,
                confirming,
            );
        }
    }

    fn draw_search(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        canvas.fill_rounded_rect(rect, 10, theme.panel_alt);
        canvas.stroke_rounded_rect(
            rect,
            10,
            1,
            if self.search.active {
                theme.accent
            } else {
                theme.border
            },
        );
        let mut tx = rect.x + 12;
        if let Some(img) = self.icons.search {
            canvas.draw_tga_icon(
                &img,
                Rect::new(tx, rect.y + (rect.h as i32 - 16) / 2, 16, 16),
            );
            tx += 24;
        }
        let text = self.search.value();
        if text.is_empty() {
            draw_text_vcenter(
                canvas,
                "Search apps, files, settings...",
                tx,
                rect.y,
                rect.h,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
        } else {
            draw_text_vcenter(
                canvas,
                text,
                tx,
                rect.y,
                rect.h,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );
            if self.search.active {
                let prefix = &text[..self.search.cursor.min(text.len())];
                let w = measure_text(prefix, FontRole::UiRegular).w;
                canvas.vline(
                    tx + w as i32 + 1,
                    rect.y + 8,
                    rect.h.saturating_sub(16),
                    theme.accent,
                );
            }
        }
    }

    fn draw_tile(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        slot: &TileSlot,
        apps: &[DockAppState],
        idx: usize,
    ) {
        let r = slot.rect;
        let selected = self.selected == Some(idx);
        let hovered = self.hover == Some(idx);
        if hovered {
            canvas.fill_rounded_rect(r, 8, theme.panel_alt);
        }
        if selected {
            canvas.fill_rounded_rect(r, 8, theme.panel_alt);
            canvas.stroke_rounded_rect(r, 8, 1, theme.accent);
        }

        let icon_size: u32 = 36;
        let icon_rect = Rect::new(
            r.x + (r.w as i32 - icon_size as i32) / 2,
            r.y + 8,
            icon_size,
            icon_size,
        );
        if let Some(img) = self.icons.app_icon(slot.entry) {
            // Soft-rounded app tile icons (radius 8 ≈ modern launcher chips).
            canvas.draw_tga_icon_rounded(&img, icon_rect, 8);
        } else {
            canvas.fill_rounded_rect(icon_rect, 6, theme.panel);
        }

        let label_color = if slot.entry.available {
            theme.text
        } else {
            theme.text_dim
        };
        let mut buf = [0u8; 20];
        let label = truncate_ascii(slot.entry.name, 14, &mut buf);
        let label_rect = Rect::new(r.x + 2, icon_rect.bottom() + 3, r.w.saturating_sub(4), 14);
        draw_text_centered(
            canvas,
            label_rect,
            label,
            &TextStyle::new(FontRole::UiSmall, label_color),
        );

        if !slot.entry.available {
            let tag_rect = Rect::new(r.x, label_rect.bottom(), r.w, 12);
            draw_text_centered(
                canvas,
                tag_rect,
                "Soon",
                &TextStyle::new(FontRole::UiSmall, theme.accent),
            );
        } else if let CatalogId::App(id) = slot.entry.id {
            if let Some(app) = apps.iter().find(|a| a.app_id == id) {
                if matches!(
                    app.state,
                    AppLaunchState::Running | AppLaunchState::Minimized | AppLaunchState::Closing
                ) {
                    canvas.fill_rect(
                        Rect::new(r.x + r.w as i32 / 2 - 8, r.bottom() - 3, 16, 2),
                        theme.accent,
                    );
                }
            }
        }
    }

    fn draw_user(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        if rect.w == 0 {
            return;
        }
        let avatar = Rect::new(rect.x, rect.y + (rect.h as i32 - 32) / 2, 32, 32);
        canvas.fill_rounded_rect(avatar, 16, theme.accent.darken(40));
        draw_text_centered(
            canvas,
            avatar,
            "U",
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        let name_rect = Rect::new(
            avatar.right() + 10,
            rect.y + 4,
            rect.w.saturating_sub(46),
            16,
        );
        draw_text_vcenter(
            canvas,
            "User",
            name_rect.x,
            name_rect.y,
            name_rect.h,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        let sub_rect = Rect::new(
            avatar.right() + 10,
            rect.y + rect.h as i32 - 18,
            rect.w.saturating_sub(46),
            14,
        );
        draw_text_vcenter(
            canvas,
            "SunlightOS",
            sub_rect.x,
            sub_rect.y,
            sub_rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }
}

fn draw_section_label(canvas: &mut Canvas, theme: &Theme, rect: Rect, label: &str) {
    if rect.w == 0 {
        return;
    }
    draw_text_vcenter(
        canvas,
        label,
        rect.x,
        rect.y,
        rect.h,
        &TextStyle::new(FontRole::UiSmall, theme.text_dim),
    );
}

fn draw_power_button(
    canvas: &mut Canvas,
    theme: &Theme,
    rect: Rect,
    icon_img: Option<TgaImage>,
    action: PowerAction,
    confirming: bool,
) {
    let (fill, border, text_color) = if confirming {
        (theme.danger.darken(60), theme.danger, theme.danger)
    } else {
        (theme.panel_alt, theme.border, theme.text_dim)
    };
    canvas.fill_rounded_rect(rect, 8, fill);
    canvas.stroke_rounded_rect(rect, 8, 1, border);
    let icon_size = 18u32;
    let icon_rect = Rect::new(
        rect.x + (rect.w as i32 - icon_size as i32) / 2,
        rect.y + 5,
        icon_size,
        icon_size,
    );
    if let Some(img) = icon_img {
        canvas.draw_tga_icon(&img, icon_rect);
    }
    let label_rect = Rect::new(rect.x - 10, icon_rect.bottom() + 2, rect.w + 20, 12);
    let shown = if confirming {
        "Confirm?"
    } else {
        action.label()
    };
    let mut buf = [0u8; 16];
    let shown = truncate_ascii(shown, 10, &mut buf);
    draw_text_centered(
        canvas,
        label_rect,
        shown,
        &TextStyle::new(FontRole::UiSmall, text_color),
    );
}

fn truncate_ascii<'a>(text: &str, max_chars: usize, buf: &'a mut [u8]) -> &'a str {
    let mut len = 0usize;
    let char_count = text.chars().count();
    let keep = if char_count > max_chars {
        max_chars.saturating_sub(1)
    } else {
        max_chars
    };
    for ch in text.chars().take(keep) {
        if !ch.is_ascii() || len >= buf.len() {
            break;
        }
        buf[len] = ch as u8;
        len += 1;
    }
    if char_count > max_chars && len < buf.len() {
        buf[len] = b'.';
        len += 1;
    }
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::{
        compute_layout, page_count, AppId, StartMenuPage, ALL_APPS_PAGE_CAP, APP_CATALOG_LEN,
        RANDOM_CAP, RECENT_CAP, SMALL_COLS,
    };

    #[test]
    fn all_apps_pagination_handles_empty_and_partial_pages() {
        assert_eq!(page_count(0, ALL_APPS_PAGE_CAP), 0);
        assert_eq!(page_count(ALL_APPS_PAGE_CAP, ALL_APPS_PAGE_CAP), 1);
        assert_eq!(page_count(APP_CATALOG_LEN, ALL_APPS_PAGE_CAP), 1);
        assert_eq!(page_count(ALL_APPS_PAGE_CAP + 1, ALL_APPS_PAGE_CAP), 2);
    }

    #[test]
    fn home_uses_one_random_row_and_two_suggestion_rows() {
        let layout = compute_layout(1280, 900, "", &[], StartMenuPage::Home, 0);

        assert_eq!(layout.random.len(), RANDOM_CAP);
        assert_eq!(layout.recent.len(), RECENT_CAP);
        assert_eq!(
            layout.recent.get(SMALL_COLS).unwrap().rect.y,
            layout.recent.get(0).unwrap().rect.y + 80
        );
        assert_eq!(
            layout.tile_count(),
            layout.pinned.len() + layout.random.len() + layout.recent.len()
        );
    }

    #[test]
    fn recent_history_is_bounded_to_two_rows() {
        let recent = [
            AppId::Terminal,
            AppId::Chronos,
            AppId::Files,
            AppId::Calculator,
            AppId::Settings,
            AppId::Tasks,
            AppId::Bench,
            AppId::Calendar,
            AppId::Devices,
            AppId::Writer,
            AppId::TextEditor,
            AppId::RappidRabbit,
            AppId::ApiLab,
        ];
        let layout = compute_layout(1280, 900, "", &recent, StartMenuPage::Home, 0);

        assert!(layout.recent_is_real);
        assert_eq!(layout.recent.len(), RECENT_CAP);
    }
}
