//! Vortex Shell Search Palette — keyboard-first application launcher.
//!
//! Opens from the bottom-right Search control. Application ranking is local and
//! bounded; calculator mode reuses the shared Sun Shell expression engine
//! (`sunshell::calc`) via a leading `=`. Launch always goes through the shell's
//! central `open_app_from_ui` path.

use sun_font::Typography;
use sunlight_ui::{
    image::TgaImage,
    widgets::{
        search_page_count, BoundedSearchField, SearchPaletteFonts, SearchPaletteLayout,
        SearchPalettePanel, SearchResultState, SearchResultView, SEARCH_FIELD_CAP,
        SEARCH_PAGE_ROWS,
    },
    Canvas, Event, Point, Theme,
};
use sunshell::calc::CalcSession;

use crate::AppId;

/// Max ranked hits retained (covers full built-in registry with headroom).
const MAX_HITS: usize = 32;

// ---------------------------------------------------------------------------
// Search metadata registry (static, not rebuilt per frame)
// ---------------------------------------------------------------------------

/// Launch destination. Application IDs map to the existing shell launcher;
/// `Preferences` is reserved for a future preferences:// deep-link without
/// changing the result model today.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SearchAction {
    /// Launch / activate a known shell application.
    LaunchApp(AppId),
    /// Future: open System Preferences at a page. Phase-1 always launches Settings.
    Preferences(&'static str),
}

#[derive(Clone, Copy)]
pub(crate) struct SearchEntry {
    pub app_id: AppId,
    pub title: &'static str,
    pub aliases: &'static [&'static str],
    pub keywords: &'static [&'static str],
    pub action: SearchAction,
    pub category: &'static str,
}

/// Centralized built-in application search metadata.
pub(crate) static SEARCH_REGISTRY: &[SearchEntry] = &[
    SearchEntry {
        app_id: AppId::Terminal,
        title: "Terminal",
        aliases: &["term", "console", "shell", "tty"],
        keywords: &["command", "cli", "bash"],
        action: SearchAction::LaunchApp(AppId::Terminal),
        category: "Utilities",
    },
    SearchEntry {
        app_id: AppId::Chronos,
        title: "Sunlight DOS Terminal",
        aliases: &["dos", "chronos", "msdos"],
        keywords: &["compatibility", "dos"],
        action: SearchAction::LaunchApp(AppId::Chronos),
        category: "Compatibility",
    },
    SearchEntry {
        app_id: AppId::Files,
        title: "Sunlight Files",
        aliases: &["files", "folders", "documents", "home", "dolphin"],
        keywords: &["file manager", "browse", "folder"],
        action: SearchAction::LaunchApp(AppId::Files),
        category: "System",
    },
    SearchEntry {
        app_id: AppId::Settings,
        title: "System Preferences",
        aliases: &["settings", "preferences", "control panel", "system"],
        keywords: &[
            "network",
            "ethernet",
            "ip",
            "display",
            "date",
            "time",
            "timezone",
            "clock",
            "ntp",
            "sound",
            "audio",
            "volume",
            "تنظیمات",
            "شبکه",
        ],
        action: SearchAction::Preferences("root"),
        category: "System",
    },
    SearchEntry {
        app_id: AppId::Settings,
        title: "Sound",
        aliases: &["sound", "audio", "volume", "speaker"],
        keywords: &["mute", "playback", "output", "hda"],
        action: SearchAction::Preferences("sound"),
        category: "System",
    },
    SearchEntry {
        app_id: AppId::Settings,
        title: "Date & Time",
        aliases: &["datetime", "date-time", "timezone", "clock", "tz"],
        keywords: &["ntp", "sync", "solar", "offset", "dst", "time zone"],
        action: SearchAction::Preferences("date-time"),
        category: "System",
    },
    SearchEntry {
        app_id: AppId::Calculator,
        title: "Calculator",
        aliases: &["calc", "math"],
        keywords: &["arithmetic", "numbers"],
        action: SearchAction::LaunchApp(AppId::Calculator),
        category: "Utilities",
    },
    SearchEntry {
        app_id: AppId::Tasks,
        title: "Task Manager",
        aliases: &["tasks", "processes", "monitor"],
        keywords: &["cpu", "memory", "ram", "telemetry", "process"],
        action: SearchAction::LaunchApp(AppId::Tasks),
        category: "System",
    },
    SearchEntry {
        app_id: AppId::Bench,
        title: "Sunlight Bench",
        aliases: &["bench", "benchmark"],
        keywords: &["performance", "cpu-x"],
        action: SearchAction::LaunchApp(AppId::Bench),
        category: "Utilities",
    },
    SearchEntry {
        app_id: AppId::TextEditor,
        title: "Text Editor",
        aliases: &["edit", "editor", "notes", "kate"],
        keywords: &["text", "write"],
        action: SearchAction::LaunchApp(AppId::TextEditor),
        category: "Productivity",
    },
    SearchEntry {
        app_id: AppId::Writer,
        title: "Sunlight Writer",
        aliases: &["writer", "word"],
        keywords: &["document", "office"],
        action: SearchAction::LaunchApp(AppId::Writer),
        category: "Productivity",
    },
    SearchEntry {
        app_id: AppId::Calendar,
        title: "Sunlight Calendar",
        aliases: &["calendar", "date", "events"],
        keywords: &["schedule", "month"],
        action: SearchAction::LaunchApp(AppId::Calendar),
        category: "Productivity",
    },
    SearchEntry {
        app_id: AppId::Devices,
        title: "Sunlight Devices",
        aliases: &["devices", "hardware", "hwinfo"],
        keywords: &["usb", "pci", "device"],
        action: SearchAction::LaunchApp(AppId::Devices),
        category: "System",
    },
    SearchEntry {
        app_id: AppId::RappidRabbit,
        title: "Rappid Rabbit",
        aliases: &["browser", "web", "internet", "rabbit"],
        keywords: &["http", "www", "surf"],
        action: SearchAction::LaunchApp(AppId::RappidRabbit),
        category: "Network",
    },
    SearchEntry {
        app_id: AppId::ApiLab,
        title: "Sunlight API Lab",
        aliases: &["api", "apilab", "http client"],
        keywords: &["rest", "request"],
        action: SearchAction::LaunchApp(AppId::ApiLab),
        category: "Network",
    },
    SearchEntry {
        app_id: AppId::Mines,
        title: "Sunlight Mines",
        aliases: &["mines", "minesweeper"],
        keywords: &["game"],
        action: SearchAction::LaunchApp(AppId::Mines),
        category: "Games",
    },
    SearchEntry {
        app_id: AppId::SiliconEchoes,
        title: "Silicon Echoes: 1993",
        aliases: &["silicon", "echoes", "1993"],
        keywords: &["game", "retro"],
        action: SearchAction::LaunchApp(AppId::SiliconEchoes),
        category: "Games",
    },
    SearchEntry {
        app_id: AppId::Welcome,
        title: "Welcome to SunlightOS",
        aliases: &["welcome", "onboarding", "tour", "wiseowl"],
        keywords: &["help", "wizard", "start"],
        action: SearchAction::LaunchApp(AppId::Welcome),
        category: "System",
    },
];

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MatchKind {
    ExactTitle = 0,
    TitlePrefix = 1,
    AliasOrKeyword = 2,
    Substring = 3,
}

#[derive(Clone, Copy, Debug)]
struct RankedHit {
    entry_index: usize,
    kind: MatchKind,
    /// Stable secondary key: title order in registry.
    registry_order: u16,
}

/// Normalize query: lowercase ASCII + collapse whitespace into a scratch buffer.
pub(crate) fn normalize_query<'a>(raw: &str, out: &'a mut [u8]) -> &'a str {
    let mut len = 0usize;
    let mut prev_space = true;
    for &b in raw.as_bytes() {
        if len >= out.len() {
            break;
        }
        let c = if b.is_ascii_uppercase() { b + 32 } else { b };
        if c == b' ' || c == b'\t' {
            if prev_space {
                continue;
            }
            out[len] = b' ';
            len += 1;
            prev_space = true;
        } else if c.is_ascii_graphic() || c >= 0x80 {
            // Pass through non-ASCII bytes as-is for Persian keywords etc.
            out[len] = c;
            len += 1;
            prev_space = false;
        }
    }
    while len > 0 && out[len - 1] == b' ' {
        len -= 1;
    }
    core::str::from_utf8(&out[..len]).unwrap_or("")
}

fn ascii_lower_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn ascii_lower_starts_with(hay: &str, needle: &str) -> bool {
    if needle.len() > hay.len() {
        return false;
    }
    hay.as_bytes()
        .iter()
        .zip(needle.as_bytes())
        .all(|(h, n)| h.to_ascii_lowercase() == n.to_ascii_lowercase())
}

fn ascii_lower_contains(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > hay.len() {
        return false;
    }
    let nlen = needle.len();
    hay.as_bytes().windows(nlen).any(|window| {
        window
            .iter()
            .zip(needle.as_bytes())
            .all(|(h, n)| h.to_ascii_lowercase() == n.to_ascii_lowercase())
    })
}

fn rank_entry(entry: &SearchEntry, query: &str) -> Option<MatchKind> {
    if query.is_empty() {
        // Empty query: show a stable default set (registry order).
        return Some(MatchKind::Substring);
    }
    if ascii_lower_eq(entry.title, query) {
        return Some(MatchKind::ExactTitle);
    }
    if ascii_lower_starts_with(entry.title, query) {
        return Some(MatchKind::TitlePrefix);
    }
    for alias in entry.aliases {
        if ascii_lower_eq(alias, query) || ascii_lower_starts_with(alias, query) {
            return Some(MatchKind::AliasOrKeyword);
        }
    }
    for kw in entry.keywords {
        if ascii_lower_eq(kw, query) || ascii_lower_starts_with(kw, query) {
            return Some(MatchKind::AliasOrKeyword);
        }
    }
    if ascii_lower_contains(entry.title, query) {
        return Some(MatchKind::Substring);
    }
    for alias in entry.aliases {
        if ascii_lower_contains(alias, query) {
            return Some(MatchKind::Substring);
        }
    }
    for kw in entry.keywords {
        if ascii_lower_contains(kw, query) {
            return Some(MatchKind::Substring);
        }
    }
    // Also match app_id debug-like tokens (terminal, files, …).
    let id_name = app_id_token(entry.app_id);
    if ascii_lower_starts_with(id_name, query) || ascii_lower_contains(id_name, query) {
        return Some(MatchKind::Substring);
    }
    None
}

fn app_id_token(id: AppId) -> &'static str {
    match id {
        AppId::Terminal => "terminal",
        AppId::Chronos => "chronos",
        AppId::Calculator => "calculator",
        AppId::Files => "files",
        AppId::Settings => "settings",
        AppId::Tasks => "tasks",
        AppId::Bench => "bench",
        AppId::TextEditor => "texteditor",
        AppId::Writer => "writer",
        AppId::Calendar => "calendar",
        AppId::Devices => "devices",
        AppId::RappidRabbit => "rappidrabbit",
        AppId::ApiLab => "apilab",
        AppId::Mines => "mines",
        AppId::SiliconEchoes => "siliconechoes",
        AppId::Welcome => "welcome",
        AppId::WiseOwl => "wiseowl",
    }
}

/// Rank applications for `query`. Writes up to `out.len()` unique hits.
/// Returns the number of hits written. Deterministic for equal scores.
pub(crate) fn rank_applications(query: &str, out: &mut [RankedHit]) -> usize {
    let mut norm_buf = [0u8; SEARCH_FIELD_CAP];
    let q = normalize_query(query, &mut norm_buf);

    let mut hits: [Option<RankedHit>; 32] = [None; 32];
    let mut n = 0usize;
    for (i, entry) in SEARCH_REGISTRY.iter().enumerate() {
        if let Some(kind) = rank_entry(entry, q) {
            if n < hits.len() {
                hits[n] = Some(RankedHit {
                    entry_index: i,
                    kind,
                    registry_order: i as u16,
                });
                n += 1;
            }
        }
    }

    // Sort: kind, then registry order (stable deterministic).
    // Simple insertion sort — n is tiny.
    for i in 1..n {
        let mut j = i;
        while j > 0 {
            let a = hits[j - 1].unwrap();
            let b = hits[j].unwrap();
            let swap = (a.kind as u8, a.registry_order) > (b.kind as u8, b.registry_order);
            if !swap {
                break;
            }
            hits.swap(j - 1, j);
            j -= 1;
        }
    }

    // Deduplicate by app_id (keep best rank already first).
    let mut written = 0usize;
    let mut seen = [false; 16]; // APP_COUNT / AppId variant count
    for i in 0..n {
        let hit = hits[i].unwrap();
        let app_id = SEARCH_REGISTRY[hit.entry_index].app_id as usize;
        if app_id < seen.len() && seen[app_id] {
            continue;
        }
        if app_id < seen.len() {
            seen[app_id] = true;
        }
        if written < out.len() {
            out[written] = hit;
            written += 1;
        }
    }
    written
}

// ---------------------------------------------------------------------------
// Calculator mode
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CalcStatus {
    None,
    Result,
    Incomplete,
    Error,
}

/// True when the first non-whitespace character is `=`.
pub(crate) fn is_calculator_query(raw: &str) -> bool {
    raw.bytes()
        .find(|b| !b.is_ascii_whitespace())
        .map(|b| b == b'=')
        .unwrap_or(false)
}

/// Strip leading whitespace and one leading `=`.
pub(crate) fn strip_calc_prefix(raw: &str) -> &str {
    let s = raw.trim_start();
    s.strip_prefix('=').unwrap_or(s).trim()
}

/// Evaluate using the shared Sun Shell calculator. Bounds expression length.
pub(crate) fn eval_calculator(expr: &str, out: &mut [u8]) -> (CalcStatus, usize) {
    const MAX_EXPR: usize = 128;
    if expr.is_empty() {
        return write_status(out, CalcStatus::Incomplete, "Type an expression");
    }
    if expr.len() > MAX_EXPR {
        return write_status(out, CalcStatus::Error, "Expression too long");
    }
    let mut session = CalcSession::new();
    let result = session.run_command(expr);
    let trimmed = result.trim();
    if trimmed.starts_with("calc error:") {
        let msg = trimmed.trim_start_matches("calc error:").trim();
        // Incomplete vs hard error: incomplete often ends mid-token.
        let incomplete = msg.contains("expected") || msg.contains("empty");
        let status = if incomplete {
            CalcStatus::Incomplete
        } else {
            CalcStatus::Error
        };
        return write_status(out, status, msg);
    }
    write_status(out, CalcStatus::Result, trimmed)
}

fn write_status(out: &mut [u8], status: CalcStatus, text: &str) -> (CalcStatus, usize) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(out.len());
    out[..n].copy_from_slice(&bytes[..n]);
    (status, n)
}

// ---------------------------------------------------------------------------
// Overlay state
// ---------------------------------------------------------------------------

const KEY_ESC: u8 = 1;
const KEY_ENTER: u8 = 0x1C;
const KEY_UP: u8 = 0x48;
const KEY_DOWN: u8 = 0x50;
const KEY_HOME: u8 = 0x47;
const KEY_END: u8 = 0x4F;
const KEY_BACKSPACE: u8 = 0x0E;
const KEY_DELETE: u8 = 0x53;
const KEY_LEFT: u8 = 0x4B;
const KEY_RIGHT: u8 = 0x4D;
const KEY_PGUP: u8 = 0x49;
const KEY_PGDN: u8 = 0x51;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SearchPaletteAction {
    None,
    Close,
    /// Launch via central shell path.
    Launch(AppId),
    /// Open System Preferences, optionally at a page (`control-panel --page`).
    LaunchPreferences(&'static str),
}

#[derive(Clone, Copy)]
enum ResultKind {
    App { entry_index: usize },
    Calculator,
}

pub(crate) struct SearchPaletteState {
    open: bool,
    field: BoundedSearchField<SEARCH_FIELD_CAP>,
    /// Full ranked hit list (all apps when query is empty).
    hits: [RankedHit; MAX_HITS],
    hit_count: usize,
    /// Parallel kinds for each hit (apps or single calculator row).
    result_kinds: [ResultKind; MAX_HITS],
    result_count: usize,
    /// Global selection index into `result_kinds[0..result_count]`.
    selected: usize,
    /// Page index for pagination dots (Start-menu style).
    page: usize,
    /// Hover is page-local row index.
    hover: Option<usize>,
    calc_status: CalcStatus,
    calc_text: [u8; 48],
    calc_text_len: usize,
}

impl SearchPaletteState {
    pub(crate) const fn new() -> Self {
        Self {
            open: false,
            field: BoundedSearchField::new(),
            hits: [RankedHit {
                entry_index: 0,
                kind: MatchKind::Substring,
                registry_order: 0,
            }; MAX_HITS],
            hit_count: 0,
            result_kinds: [ResultKind::App { entry_index: 0 }; MAX_HITS],
            result_count: 0,
            selected: 0,
            page: 0,
            hover: None,
            calc_status: CalcStatus::None,
            calc_text: [0; 48],
            calc_text_len: 0,
        }
    }

    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn open(&mut self) {
        self.open = true;
        self.field.active = true;
        self.hover = None;
        self.recompute_results();
    }

    pub(crate) fn close(&mut self) -> bool {
        let was = self.open;
        self.open = false;
        self.field.active = false;
        self.hover = None;
        was
    }

    pub(crate) fn query(&self) -> &str {
        self.field.value()
    }

    fn page_count(&self) -> usize {
        search_page_count(self.result_count, SEARCH_PAGE_ROWS)
    }

    fn page_start(&self) -> usize {
        self.page.saturating_mul(SEARCH_PAGE_ROWS)
    }

    fn visible_row_count(&self) -> usize {
        let start = self.page_start();
        self.result_count
            .saturating_sub(start)
            .min(SEARCH_PAGE_ROWS)
    }

    fn sync_page_to_selection(&mut self) {
        if self.result_count == 0 {
            self.page = 0;
            self.selected = 0;
            return;
        }
        if self.selected >= self.result_count {
            self.selected = self.result_count - 1;
        }
        self.page = self.selected / SEARCH_PAGE_ROWS;
        let pages = self.page_count();
        if pages > 0 && self.page >= pages {
            self.page = pages - 1;
        }
    }

    fn recompute_results(&mut self) {
        let q = self.field.value();
        self.calc_status = CalcStatus::None;
        self.calc_text_len = 0;
        self.result_count = 0;
        self.hit_count = 0;
        self.page = 0;
        self.hover = None;

        if is_calculator_query(q) {
            let expr = strip_calc_prefix(q);
            let (status, n) = eval_calculator(expr, &mut self.calc_text);
            self.calc_status = status;
            self.calc_text_len = n;
            if status == CalcStatus::Result
                || status == CalcStatus::Incomplete
                || status == CalcStatus::Error
            {
                self.result_kinds[0] = ResultKind::Calculator;
                self.result_count = 1;
            }
        } else {
            // Empty query → full app list (registry order via rank_entry).
            self.hit_count = rank_applications(q, &mut self.hits);
            for i in 0..self.hit_count {
                self.result_kinds[i] = ResultKind::App {
                    entry_index: self.hits[i].entry_index,
                };
            }
            self.result_count = self.hit_count;
        }

        self.selected = 0;
        self.sync_page_to_selection();
    }

    fn calc_text_str(&self) -> &str {
        core::str::from_utf8(&self.calc_text[..self.calc_text_len]).unwrap_or("")
    }

    pub(crate) fn layout(&self, screen_w: u32, screen_h: u32) -> SearchPaletteLayout {
        let rows = if self.result_count == 0 {
            1
        } else {
            self.visible_row_count().max(1)
        };
        SearchPaletteLayout::compute(screen_w, screen_h, rows, self.page_count())
    }

    pub(crate) fn contains(&self, point: Point, screen_w: u32, screen_h: u32) -> bool {
        self.open && self.layout(screen_w, screen_h).contains(point)
    }

    pub(crate) fn handle_event(
        &mut self,
        event: Event,
        screen_w: u32,
        screen_h: u32,
    ) -> (bool, SearchPaletteAction) {
        if !self.open {
            return (false, SearchPaletteAction::None);
        }
        let layout = self.layout(screen_w, screen_h);
        let page_start = self.page_start();
        let visible = self.visible_row_count();
        match event {
            Event::Click { x, y } => {
                let p = Point::new(x, y);
                if !layout.contains(p) {
                    return (true, SearchPaletteAction::Close);
                }
                if let Some(page) = layout.page_dot_at(p) {
                    if page < self.page_count() {
                        self.page = page;
                        self.selected = page * SEARCH_PAGE_ROWS;
                        if self.selected >= self.result_count && self.result_count > 0 {
                            self.selected = self.result_count - 1;
                        }
                        self.hover = None;
                        return (true, SearchPaletteAction::None);
                    }
                }
                if let Some(row) = layout.row_index_at(p) {
                    if row < visible {
                        self.selected = page_start + row;
                        return (true, self.activate_selected());
                    }
                }
                if layout.input.contains(p) {
                    self.field.active = true;
                    return (true, SearchPaletteAction::None);
                }
                (true, SearchPaletteAction::None)
            }
            Event::MouseMove { x, y, .. } => {
                let p = Point::new(x, y);
                let next = layout.row_index_at(p).filter(|&i| i < visible);
                if self.hover != next {
                    self.hover = next;
                    if let Some(row) = next {
                        self.selected = page_start + row;
                    }
                    return (true, SearchPaletteAction::None);
                }
                (false, SearchPaletteAction::None)
            }
            Event::MouseDown { x, y, .. } => {
                let p = Point::new(x, y);
                if !layout.contains(p) {
                    return (true, SearchPaletteAction::Close);
                }
                if let Some(page) = layout.page_dot_at(p) {
                    if page < self.page_count() {
                        self.page = page;
                        self.selected = page * SEARCH_PAGE_ROWS;
                        if self.selected >= self.result_count && self.result_count > 0 {
                            self.selected = self.result_count - 1;
                        }
                    }
                }
                if let Some(row) = layout.row_index_at(p) {
                    if row < visible {
                        self.selected = page_start + row;
                    }
                }
                (true, SearchPaletteAction::None)
            }
            Event::Key('\x1b') => (true, SearchPaletteAction::Close),
            Event::Key('\n') | Event::Key('\r') => (true, self.activate_selected()),
            Event::Key(ch) => {
                if self.field.handle_char(ch) {
                    self.recompute_results();
                    return (true, SearchPaletteAction::None);
                }
                (false, SearchPaletteAction::None)
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ctrl: false,
                super_key: false,
                ..
            } => self.handle_keypress(keycode),
            _ => (false, SearchPaletteAction::None),
        }
    }

    fn handle_keypress(&mut self, keycode: u8) -> (bool, SearchPaletteAction) {
        match keycode {
            KEY_ESC => (true, SearchPaletteAction::Close),
            KEY_ENTER => (true, self.activate_selected()),
            KEY_UP => {
                if self.result_count > 0 && self.selected > 0 {
                    self.selected -= 1;
                    self.sync_page_to_selection();
                }
                (true, SearchPaletteAction::None)
            }
            KEY_DOWN => {
                if self.result_count > 0 && self.selected + 1 < self.result_count {
                    self.selected += 1;
                    self.sync_page_to_selection();
                }
                (true, SearchPaletteAction::None)
            }
            KEY_PGUP => {
                if self.page > 0 {
                    self.page -= 1;
                    self.selected = self.page * SEARCH_PAGE_ROWS;
                }
                (true, SearchPaletteAction::None)
            }
            KEY_PGDN => {
                let pages = self.page_count();
                if pages > 0 && self.page + 1 < pages {
                    self.page += 1;
                    self.selected = self.page * SEARCH_PAGE_ROWS;
                    if self.selected >= self.result_count {
                        self.selected = self.result_count.saturating_sub(1);
                    }
                }
                (true, SearchPaletteAction::None)
            }
            KEY_HOME => {
                self.selected = 0;
                self.sync_page_to_selection();
                (true, SearchPaletteAction::None)
            }
            KEY_END => {
                if self.result_count > 0 {
                    self.selected = self.result_count - 1;
                    self.sync_page_to_selection();
                }
                (true, SearchPaletteAction::None)
            }
            KEY_BACKSPACE => {
                if self.field.backspace() {
                    self.recompute_results();
                    return (true, SearchPaletteAction::None);
                }
                (false, SearchPaletteAction::None)
            }
            KEY_DELETE => {
                if self.field.delete_forward() {
                    self.recompute_results();
                    return (true, SearchPaletteAction::None);
                }
                (false, SearchPaletteAction::None)
            }
            // Left/Right move the caret in the query when on a single page;
            // with multiple pages they also flip pages (Walker-style paging).
            KEY_LEFT => {
                if self.page_count() > 1 {
                    if self.page > 0 {
                        self.page -= 1;
                        self.selected = self.page * SEARCH_PAGE_ROWS;
                    }
                    (true, SearchPaletteAction::None)
                } else {
                    (self.field.move_left(), SearchPaletteAction::None)
                }
            }
            KEY_RIGHT => {
                if self.page_count() > 1 {
                    let pages = self.page_count();
                    if self.page + 1 < pages {
                        self.page += 1;
                        self.selected = self.page * SEARCH_PAGE_ROWS;
                        if self.selected >= self.result_count {
                            self.selected = self.result_count.saturating_sub(1);
                        }
                    }
                    (true, SearchPaletteAction::None)
                } else {
                    (self.field.move_right(), SearchPaletteAction::None)
                }
            }
            _ => (false, SearchPaletteAction::None),
        }
    }

    fn activate_selected(&self) -> SearchPaletteAction {
        if self.result_count == 0 {
            return SearchPaletteAction::None;
        }
        match self.result_kinds[self.selected] {
            ResultKind::Calculator => SearchPaletteAction::None,
            ResultKind::App { entry_index } => {
                let entry = &SEARCH_REGISTRY[entry_index];
                match entry.action {
                    SearchAction::LaunchApp(id) => SearchPaletteAction::Launch(id),
                    SearchAction::Preferences(page) => SearchPaletteAction::LaunchPreferences(page),
                }
            }
        }
    }

    /// AppIds for the currently visible page rows (for shell icon resolution).
    pub(crate) fn visible_app_ids(&self) -> [Option<AppId>; SEARCH_PAGE_ROWS] {
        let mut out = [None; SEARCH_PAGE_ROWS];
        let page_start = self.page_start();
        let visible = self.visible_row_count();
        for row in 0..visible {
            let gi = page_start + row;
            if let ResultKind::App { entry_index } = self.result_kinds[gi] {
                out[row] = Some(SEARCH_REGISTRY[entry_index].app_id);
            }
        }
        out
    }

    /// Draw the palette. `row_icons` are pre-resolved by the shell for the page.
    pub(crate) fn view(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        screen_w: u32,
        screen_h: u32,
        row_icons: &[Option<TgaImage>; SEARCH_PAGE_ROWS],
    ) {
        if !self.open {
            return;
        }
        let layout = self.layout(screen_w, screen_h);
        let page_start = self.page_start();
        let visible = self.visible_row_count();

        let mut titles: [&str; SEARCH_PAGE_ROWS] = [""; SEARCH_PAGE_ROWS];
        let mut subtitles: [Option<&str>; SEARCH_PAGE_ROWS] = [None; SEARCH_PAGE_ROWS];
        let mut views: [SearchResultView; SEARCH_PAGE_ROWS] = [SearchResultView {
            title: "",
            subtitle: None,
            icon: None,
            state: SearchResultState::Normal,
        }; SEARCH_PAGE_ROWS];

        for row in 0..visible {
            let gi = page_start + row;
            match self.result_kinds[gi] {
                ResultKind::Calculator => {
                    titles[row] = self.calc_text_str();
                    subtitles[row] = Some(match self.calc_status {
                        CalcStatus::Result => "Calculator",
                        CalcStatus::Incomplete => "Incomplete expression",
                        CalcStatus::Error => "Invalid expression",
                        CalcStatus::None => "Calculator",
                    });
                }
                ResultKind::App { entry_index } => {
                    let e = &SEARCH_REGISTRY[entry_index];
                    titles[row] = e.title;
                    subtitles[row] = Some(e.category);
                }
            }
            let state = if gi == self.selected {
                SearchResultState::Selected
            } else if self.hover == Some(row) {
                SearchResultState::Hovered
            } else {
                SearchResultState::Normal
            };
            views[row] = SearchResultView {
                title: titles[row],
                subtitle: subtitles[row],
                icon: row_icons[row].as_ref(),
                state,
            };
        }

        let empty = if self.result_count == 0 {
            Some("No matching applications")
        } else {
            None
        };
        let result_slice = &views[..visible];
        let footer = if self.page_count() > 1 {
            "↑↓ select  ·  ←→ page  ·  Enter open  ·  Esc close"
        } else {
            "↑↓ navigate  ·  Enter open  ·  Esc close"
        };

        SearchPalettePanel {
            layout,
            field: &self.field,
            results: result_slice,
            empty_label: empty,
            footer_hint: footer,
            status: None,
            active_page: self.page,
            fonts: SearchPaletteFonts {
                regular: Some(&Typography::UI_REGULAR),
                medium: Some(&Typography::UI_MEDIUM),
                small: Some(&Typography::UI_SMALL),
            },
        }
        .draw(canvas, theme);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles_for(query: &str) -> alloc::vec::Vec<&'static str> {
        let mut hits = [RankedHit {
            entry_index: 0,
            kind: MatchKind::Substring,
            registry_order: 0,
        }; MAX_HITS];
        let n = rank_applications(query, &mut hits);
        (0..n)
            .map(|i| SEARCH_REGISTRY[hits[i].entry_index].title)
            .collect()
    }

    #[test]
    fn empty_query_lists_apps_without_duplicates() {
        let mut hits = [RankedHit {
            entry_index: 0,
            kind: MatchKind::Substring,
            registry_order: 0,
        }; 32];
        let n = rank_applications("", &mut hits);
        assert!(n >= 8);
        let mut seen = [false; 16];
        for i in 0..n {
            let id = SEARCH_REGISTRY[hits[i].entry_index].app_id as usize;
            assert!(!seen[id], "duplicate app id");
            seen[id] = true;
        }
    }

    #[test]
    fn exact_title_ranks_first() {
        let t = titles_for("Terminal");
        assert_eq!(t[0], "Terminal");
    }

    #[test]
    fn prefix_ranking() {
        let t = titles_for("term");
        assert_eq!(t[0], "Terminal");
    }

    #[test]
    fn alias_and_keyword_matching() {
        let t = titles_for("network");
        assert!(t.iter().any(|x| *x == "System Preferences"));
        let t = titles_for("sound");
        assert!(t.iter().any(|x| *x == "Sound"));
        let t = titles_for("web");
        assert!(t.iter().any(|x| *x == "Rappid Rabbit"));
        let t = titles_for("cpu");
        assert!(t.iter().any(|x| *x == "Task Manager"));
    }

    #[test]
    fn case_insensitive_and_whitespace() {
        let a = titles_for("  TeRm  ");
        let b = titles_for("term");
        assert_eq!(a[0], b[0]);
        assert_eq!(a[0], "Terminal");
    }

    #[test]
    fn deterministic_ordering() {
        let a = titles_for("s");
        let b = titles_for("s");
        assert_eq!(a, b);
    }

    #[test]
    fn no_results_state() {
        let t = titles_for("zzzz-not-an-app-xyz");
        assert!(t.is_empty());
    }

    #[test]
    fn query_length_bounded() {
        let mut f = BoundedSearchField::<SEARCH_FIELD_CAP>::new();
        for _ in 0..200 {
            let _ = f.insert(b'a');
        }
        assert!(f.value().len() <= SEARCH_FIELD_CAP);
    }

    #[test]
    fn selection_clamps_when_results_shrink() {
        let mut sp = SearchPaletteState::new();
        sp.open();
        // Type something with many hits then shrink.
        sp.field.set_text("s");
        sp.recompute_results();
        if sp.result_count > 0 {
            sp.selected = sp.result_count - 1;
        }
        sp.field.set_text("zzzz-not-an-app-xyz");
        sp.recompute_results();
        assert_eq!(sp.selected, 0);
        assert_eq!(sp.result_count, 0);
    }

    #[test]
    fn calculator_two_plus_three() {
        let mut buf = [0u8; 48];
        let (st, n) = eval_calculator("2 + 3", &mut buf);
        assert_eq!(st, CalcStatus::Result);
        assert_eq!(core::str::from_utf8(&buf[..n]).unwrap(), "5");
    }

    #[test]
    fn calculator_invalid_does_not_panic() {
        let mut buf = [0u8; 48];
        let (st, _) = eval_calculator("2 +", &mut buf);
        assert!(matches!(st, CalcStatus::Incomplete | CalcStatus::Error));
        let (st, _) = eval_calculator("1 / 0", &mut buf);
        assert_eq!(st, CalcStatus::Error);
    }

    #[test]
    fn calculator_query_does_not_rank_apps() {
        let mut sp = SearchPaletteState::new();
        sp.open();
        sp.field.set_text("= 2 + 3");
        sp.recompute_results();
        assert_eq!(sp.result_count, 1);
        assert!(matches!(sp.result_kinds[0], ResultKind::Calculator));
    }

    #[test]
    fn is_calculator_requires_leading_equals() {
        assert!(is_calculator_query("= 1+1"));
        assert!(is_calculator_query("  =2"));
        assert!(!is_calculator_query("2 + 2"));
        assert!(!is_calculator_query("calc 2"));
    }

    #[test]
    fn escape_closes() {
        let mut sp = SearchPaletteState::new();
        sp.open();
        assert!(sp.is_open());
        let (dirty, action) = sp.handle_event(Event::key('\x1b'), 1366, 768);
        assert!(dirty);
        assert_eq!(action, SearchPaletteAction::Close);
    }

    #[test]
    fn enter_launches_selected_app() {
        let mut sp = SearchPaletteState::new();
        sp.open();
        sp.field.set_text("term");
        sp.recompute_results();
        let action = sp.activate_selected();
        assert_eq!(action, SearchPaletteAction::Launch(AppId::Terminal));
    }

    #[test]
    fn repeated_open_close_preserves_query_buffer_capacity() {
        let mut sp = SearchPaletteState::new();
        for i in 0..32 {
            sp.open();
            sp.field.set_text(if i % 2 == 0 { "files" } else { "web" });
            sp.recompute_results();
            assert!(sp.close());
        }
        assert!(!sp.is_open());
        assert!(sp.query().len() <= SEARCH_FIELD_CAP);
    }

    #[test]
    fn reopening_focuses_the_existing_palette_and_preserves_its_query() {
        let mut sp = SearchPaletteState::new();
        sp.open();
        sp.field.set_text("files");
        sp.recompute_results();

        // `open` is intentionally idempotent for shelf/shortcut activation:
        // there is one state object, its field stays active, and its query is
        // retained rather than creating a second launcher surface.
        sp.open();
        assert!(sp.is_open());
        assert!(sp.field.active);
        assert_eq!(sp.query(), "files");
    }
}
