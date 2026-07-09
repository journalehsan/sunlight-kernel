#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::GlobalAlloc;

use sun_font::{draw_text, draw_text_vcenter, line_height, measure_text, FontRole, TextStyle};
use sunlight_ipc::{
    debug_log, get_time_utc, ipc_call_timeout, launch_trace::LaunchSource, monotonic_millis,
    nameserver_lookup_timeout, process_yield, shm_alloc, shm_free, shm_map, CapabilityToken,
    IpcMsg, ProcessExit, SHM_PAGE,
};
use sunlight_libc::{self as libc};
use sunlight_locale::{month_name, weekday_name};
use sunlight_tz::{local_now, read_localtime, tz_by_id};
use sunlight_ui::image::TgaImage;
use sunlight_ui::widgets::{
    form_field_style, status_text_color, Button, CalendarCellState, CalendarCellStyle,
    EmptyStateStyle, Panel, StatusTextKind,
};
use sunlight_ui::{
    App, Canvas, Event, GridRow, Point, Rect, Theme, VBox, Window, WindowConfig, WindowDecoration,
};

const WIN_W: u32 = 960;
const WIN_H: u32 = 640;
const HEADER_H: u32 = 34;
const TOOLBAR_H: u32 = 38;
const SIDEBAR_W: u32 = 260;
const STATUS_H: u32 = 22;
const PAD: i32 = 8;
const CALENDAR_INNER_PAD: i32 = 12;
const CALENDAR_SECTION_GAP: i32 = 12;
const CALENDAR_CELL_W_MIN: i32 = 68;
const CALENDAR_CELL_W_MAX: i32 = 90;
const CALENDAR_CELL_H_MIN: i32 = 36;
const CALENDAR_CELL_H_MAX: i32 = 52;
const CALENDAR_HEADER_H: i32 = 24;
const GRID_GAP: i32 = 2;
const DIALOG_W: u32 = 500;
const DIALOG_BTN_W: u32 = 96;
const DIALOG_BTN_H: u32 = 28;
const DIALOG_BTN_GAP: u32 = 10;
const DIALOG_PAD: i32 = 16;
const SIDEBAR_ACTION_H: i32 = 32;
const SIDEBAR_ACTION_GAP: i32 = 8;
const SIDEBAR_BOTTOM_MARGIN: i32 = 12;
const MAX_EVENTS: usize = 256;
const TITLE_LEN: usize = 96;
const DATE_LEN: usize = 10;
const TIME_LEN: usize = 5;
const NOTES_LEN: usize = 256;
const MSG_LEN: usize = 64;
const TOOLBAR_BTN_W: u32 = 36;
const KV_REPLY: u64 = 0x4BFF;
const KV_ERROR: u64 = 0x4BEE;
const KV_VALUE: u64 = 0x4B05;
const KV_PUT_SHM2: u64 = 0x4B08;
const KV_GET_SHM2: u64 = 0x4B09;
const KV_DELETE_SHM2: u64 = 0x4B0A;
const KV_LOOKUP_TIMEOUT_MS: u64 = 250;
const KV_TIMEOUT_MS: u64 = 250;
use sunlight_calendar::{
    build_selected_day_previews, by_date_key, encode_id_list, event_key, parse_id_list, parse_u64,
    setting_key, CAL_INDEX_ALL, CAL_MIGRATION_FILE_V1,
};
use sunlight_reminders::{
    by_date_list_key, decode_list, decode_task, list_key, reminder_date_list_key, task_key,
    TaskList, TaskStatus, DEFAULT_LISTS,
};

const KEY_ESC: u8 = 0x01;
const KEY_ENTER: u8 = 0x1C;
const KEY_TAB: u8 = 0x0F;
const KEY_BACKSPACE: u8 = 0x0E;
const KEY_DELETE: u8 = 0x53;

static mut KV_CAP_CACHE: CapabilityToken = CapabilityToken::INVALID;

static ICON_PREV_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_prev.tga"));
static ICON_NEXT_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_next.tga"));
static ICON_ADD_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_add.tga"));
static ICON_MENU_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_menu.tga"));
static ICON_EVENT_TGA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icon_event.tga"));

struct BumpAllocator;
unsafe impl GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 512 * 1024] = [0; 512 * 1024];
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
    debug_log("[CALENDAR] panic\n");
    loop {
        process_yield();
    }
}

struct SlotString<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> SlotString<N> {
    const fn empty() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    fn set(&mut self, text: &str) {
        let bytes = text.as_bytes();
        self.len = bytes.len().min(N);
        self.buf[..self.len].copy_from_slice(&bytes[..self.len]);
    }

    fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    fn push(&mut self, ch: char) {
        if self.len >= N {
            return;
        }
        let mut enc = [0u8; 4];
        let encoded = ch.encode_utf8(&mut enc);
        let available = N - self.len;
        let to_copy = encoded.len().min(available);
        self.buf[self.len..self.len + to_copy].copy_from_slice(&enc[..to_copy]);
        self.len += to_copy;
    }

    fn pop(&mut self) {
        if self.len == 0 {
            return;
        }
        let s = self.as_str();
        if let Some((i, _)) = s.char_indices().last() {
            self.len = i;
        } else {
            self.len = 0;
        }
    }
}

impl<const N: usize> Clone for SlotString<N> {
    fn clone(&self) -> Self {
        let mut out = Self::empty();
        out.len = self.len;
        out.buf[..self.len].copy_from_slice(&self.buf[..self.len]);
        out
    }
}

impl<const N: usize> Copy for SlotString<N> {}

fn iso_weekday(year: i32, month: i32, day: i32) -> u8 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let w = (y + y / 4 - y / 100 + y / 400 + t[(month - 1) as usize] + day) % 7;
    if w == 0 {
        7
    } else {
        w as u8
    }
}

fn weekday_mon0(year: i32, month: i32, day: i32) -> i32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    ((y + y / 4 - y / 100 + y / 400 + t[(month - 1) as usize] + day) % 7 + 6) % 7
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: i32) -> i32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn month_grid_days(year: i32, month: i32) -> [i32; 42] {
    let mut days = [0i32; 42];
    let total = days_in_month(year, month);
    let start_wday = weekday_mon0(year, month, 1);
    for i in 0..total {
        days[(start_wday + i) as usize % 42] = i + 1;
    }
    days
}

#[derive(Clone, Copy)]
struct MonthGridLayout {
    header_h: i32,
    cell_w: i32,
    cell_h: i32,
    total_cell_w: i32,
    total_cell_h: i32,
    pad_x: i32,
    header_y: i32,
    grid_top: i32,
}

impl MonthGridLayout {
    fn new(rect: Rect) -> Self {
        let width_budget = (rect.w as i32 - 6 * GRID_GAP).max(1);
        let height_budget = (rect.h as i32 - CALENDAR_HEADER_H - 4 - 5 * GRID_GAP).max(1);

        let mut cell_w = (width_budget / 7)
            .max(CALENDAR_CELL_W_MIN)
            .min(CALENDAR_CELL_W_MAX);
        let mut cell_h = (height_budget / 6)
            .max(CALENDAR_CELL_H_MIN)
            .min(CALENDAR_CELL_H_MAX);

        let mut grid_w = 7 * cell_w + 6 * GRID_GAP;
        if grid_w > rect.w as i32 {
            cell_w = ((rect.w as i32 - 6 * GRID_GAP) / 7).max(1);
            grid_w = 7 * cell_w + 6 * GRID_GAP;
        }

        let mut grid_h = CALENDAR_HEADER_H + 4 + 6 * cell_h + 5 * GRID_GAP;
        if grid_h > rect.h as i32 {
            cell_h = ((rect.h as i32 - CALENDAR_HEADER_H - 4 - 5 * GRID_GAP) / 6).max(1);
            grid_h = CALENDAR_HEADER_H + 4 + 6 * cell_h + 5 * GRID_GAP;
        }

        let pad_x = ((rect.w as i32 - grid_w) / 2).max(0);
        let pad_y = ((rect.h as i32 - grid_h) / 2).max(0);
        let header_y = rect.y + pad_y;
        let grid_top = header_y + CALENDAR_HEADER_H + 4;

        Self {
            header_h: CALENDAR_HEADER_H,
            cell_w,
            cell_h,
            total_cell_w: cell_w + GRID_GAP,
            total_cell_h: cell_h + GRID_GAP,
            pad_x,
            header_y,
            grid_top,
        }
    }

    fn cell_rect(&self, base: Rect, col: i32, row: i32) -> Rect {
        let x = base.x + self.pad_x + col * self.total_cell_w;
        let y = self.grid_top + row * self.total_cell_h;
        Rect::new(x, y, self.cell_w as u32, self.cell_h as u32)
    }

    fn contains(&self, base: Rect, x: i32, y: i32) -> Option<i32> {
        let rel_x = x - base.x - self.pad_x;
        let rel_y = y - self.grid_top;

        if rel_x < 0 || rel_y < 0 {
            return None;
        }

        let col = rel_x / self.total_cell_w;
        let row = rel_y / self.total_cell_h;

        if col >= 7 || row >= 6 {
            return None;
        }

        Some(row * 7 + col)
    }
}

#[derive(Clone, Copy)]
struct CalendarEvent {
    id: u64,
    title: SlotString<TITLE_LEN>,
    date: SlotString<DATE_LEN>,
    start_time: SlotString<TIME_LEN>,
    end_time: SlotString<TIME_LEN>,
    all_day: bool,
    notes: SlotString<NOTES_LEN>,
    created_at: u64,
    updated_at: u64,
}

impl CalendarEvent {
    fn new(id: u64) -> Self {
        Self {
            id,
            title: SlotString::empty(),
            date: SlotString::empty(),
            start_time: SlotString::empty(),
            end_time: SlotString::empty(),
            all_day: false,
            notes: SlotString::empty(),
            created_at: monotonic_millis(),
            updated_at: monotonic_millis(),
        }
    }

    fn format_date(year: i32, month: i32, day: i32) -> SlotString<DATE_LEN> {
        let mut s = SlotString::empty();
        push_i32_fixed(&mut s, year, 4);
        s.push('-');
        push_i32_fixed(&mut s, month, 2);
        s.push('-');
        push_i32_fixed(&mut s, day, 2);
        s
    }
}

#[derive(Clone, Copy)]
struct TaskPreview {
    title: SlotString<TITLE_LEN>,
    due_time: SlotString<TIME_LEN>,
    list_name: SlotString<32>,
    status: TaskStatus,
}

#[derive(Clone, Copy)]
struct ReminderPreview {
    title: SlotString<TITLE_LEN>,
    reminder_time: SlotString<TIME_LEN>,
    linked_task_title: SlotString<TITLE_LEN>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StoreMode {
    Kv,
    MemoryFallback,
}

#[derive(Clone, Copy)]
enum StoreError {
    Unavailable,
    InvalidData,
    TooLarge,
}

trait CalendarStore {
    fn load_events(&mut self) -> Result<Vec<CalendarEvent>, StoreError>;
    fn save_event(&mut self, event: &CalendarEvent) -> Result<(), StoreError>;
    fn delete_event(&mut self, event_id: u64) -> Result<(), StoreError>;
    fn load_setting(&mut self, key: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn save_setting(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError>;
    fn mode(&self) -> StoreMode;
}

struct KvCalendarStore {
    available: bool,
}

impl KvCalendarStore {
    fn new() -> Self {
        Self {
            available: kv_cap().is_ok(),
        }
    }

    fn setting_key(key: &str) -> String {
        setting_key(key)
    }

    fn event_key(event_id: u64) -> String {
        event_key(event_id)
    }

    fn by_date_key(date: &str) -> String {
        by_date_key(date)
    }

    fn get(&mut self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        match kv_get(key) {
            Ok(value) => {
                self.available = true;
                Ok(Some(value))
            }
            Err(KvClientError::NotFound) => Ok(None),
            Err(_) => {
                self.available = false;
                Err(StoreError::Unavailable)
            }
        }
    }

    fn put(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        kv_put(key, value).map_err(|err| {
            self.available = false;
            match err {
                KvClientError::TooLarge => StoreError::TooLarge,
                _ => StoreError::Unavailable,
            }
        })?;
        self.available = true;
        Ok(())
    }

    fn delete_key(&mut self, key: &str) -> Result<(), StoreError> {
        match kv_delete(key) {
            Ok(()) | Err(KvClientError::NotFound) => {
                self.available = true;
                Ok(())
            }
            Err(_) => {
                self.available = false;
                Err(StoreError::Unavailable)
            }
        }
    }

    fn load_id_index(&mut self) -> Result<Vec<u64>, StoreError> {
        let Some(bytes) = self.get(CAL_INDEX_ALL)? else {
            return Ok(Vec::new());
        };
        parse_id_list(&bytes).ok_or(StoreError::InvalidData)
    }

    fn save_id_index(&mut self, ids: &[u64]) -> Result<(), StoreError> {
        let value = encode_id_list(ids);
        self.put(CAL_INDEX_ALL, &value)
    }

    fn load_date_index(&mut self, date: &str) -> Result<Vec<u64>, StoreError> {
        let key = Self::by_date_key(date);
        let Some(bytes) = self.get(&key)? else {
            return Ok(Vec::new());
        };
        Ok(parse_id_list(&bytes).unwrap_or_default())
    }

    fn save_date_index(&mut self, date: &str, ids: &[u64]) -> Result<(), StoreError> {
        let key = Self::by_date_key(date);
        if ids.is_empty() {
            return self.delete_key(&key);
        }
        let value = encode_id_list(ids);
        self.put(&key, &value)
    }

    fn mark_migration_complete(&mut self) -> Result<(), StoreError> {
        self.save_setting(CAL_MIGRATION_FILE_V1, b"1")
    }

    fn migration_complete(&mut self) -> bool {
        matches!(self.load_setting(CAL_MIGRATION_FILE_V1), Ok(Some(_)))
    }
}

impl CalendarStore for KvCalendarStore {
    fn load_events(&mut self) -> Result<Vec<CalendarEvent>, StoreError> {
        let ids = self.load_id_index()?;
        let mut events = Vec::new();
        for id in ids {
            let key = Self::event_key(id);
            match self.get(&key) {
                Ok(Some(bytes)) => match decode_event(&bytes) {
                    Some(event) => events.push(event),
                    None => debug_log("[CALENDAR] skipped malformed KV event\n"),
                },
                Ok(None) => debug_log("[CALENDAR] skipped missing indexed KV event\n"),
                Err(_) => return Err(StoreError::Unavailable),
            }
            if events.len() >= MAX_EVENTS {
                break;
            }
        }
        Ok(events)
    }

    fn save_event(&mut self, event: &CalendarEvent) -> Result<(), StoreError> {
        let mut ids = self.load_id_index().unwrap_or_default();
        if !ids.iter().any(|id| *id == event.id) {
            ids.push(event.id);
        }
        self.save_id_index(&ids)?;

        let events = self.load_events().unwrap_or_default();
        for old in events.iter().filter(|old| old.id == event.id) {
            if old.date.as_str() != event.date.as_str() {
                let mut date_ids = self.load_date_index(old.date.as_str()).unwrap_or_default();
                date_ids.retain(|id| *id != event.id);
                let _ = self.save_date_index(old.date.as_str(), &date_ids);
            }
        }

        let mut date_ids = self
            .load_date_index(event.date.as_str())
            .unwrap_or_default();
        if !date_ids.iter().any(|id| *id == event.id) {
            date_ids.push(event.id);
        }
        self.save_date_index(event.date.as_str(), &date_ids)?;

        let key = Self::event_key(event.id);
        let value = encode_event(event);
        self.put(&key, &value)
    }

    fn delete_event(&mut self, event_id: u64) -> Result<(), StoreError> {
        let mut old_date = String::new();
        let key = Self::event_key(event_id);
        if let Ok(Some(bytes)) = self.get(&key) {
            if let Some(event) = decode_event(&bytes) {
                old_date.push_str(event.date.as_str());
            }
        }
        self.delete_key(&key)?;

        let mut ids = self.load_id_index().unwrap_or_default();
        ids.retain(|id| *id != event_id);
        self.save_id_index(&ids)?;

        if !old_date.is_empty() {
            let mut date_ids = self.load_date_index(&old_date).unwrap_or_default();
            date_ids.retain(|id| *id != event_id);
            self.save_date_index(&old_date, &date_ids)?;
        }
        Ok(())
    }

    fn load_setting(&mut self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.get(&Self::setting_key(key))
    }

    fn save_setting(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        self.put(&Self::setting_key(key), value)
    }

    fn mode(&self) -> StoreMode {
        if self.available {
            StoreMode::Kv
        } else {
            StoreMode::MemoryFallback
        }
    }
}

#[derive(Clone, Copy)]
enum KvClientError {
    Unavailable,
    NotFound,
    TooLarge,
}

fn kv_cap() -> Result<CapabilityToken, KvClientError> {
    let cached = unsafe { KV_CAP_CACHE };
    if cached != CapabilityToken::INVALID {
        return Ok(cached);
    }
    match nameserver_lookup_timeout("sunlight-kv", KV_LOOKUP_TIMEOUT_MS) {
        Some(cap) => {
            unsafe {
                KV_CAP_CACHE = cap;
            }
            Ok(cap)
        }
        None => {
            debug_log("[CALENDAR-KV] lookup sunlight-kv failed/timeout\n");
            Err(KvClientError::Unavailable)
        }
    }
}

fn kv_put(key: &str, value: &[u8]) -> Result<(), KvClientError> {
    if key.len() > SHM_PAGE || value.len() > SHM_PAGE {
        return Err(KvClientError::TooLarge);
    }
    let cap = kv_cap()?;
    let (key_ptr, key_tok) = shm_alloc().map_err(|_| KvClientError::Unavailable)?;
    let (value_ptr, value_tok) = shm_alloc().map_err(|_| {
        let _ = shm_free(key_tok);
        KvClientError::Unavailable
    })?;
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
        core::ptr::copy_nonoverlapping(value.as_ptr(), value_ptr, value.len());
    }
    let msg = IpcMsg::with_label(KV_PUT_SHM2)
        .word(0, key.len() as u64)
        .word(1, value.len() as u64)
        .with_cap(0, key_tok)
        .with_cap(1, value_tok);
    let reply_res = ipc_call_timeout(cap, msg, KV_TIMEOUT_MS);
    let _ = shm_free(key_tok);
    let _ = shm_free(value_tok);
    let reply = reply_res.map_err(|_| {
        debug_log("[CALENDAR-KV] put timeout/error\n");
        KvClientError::Unavailable
    })?;
    if reply.label == KV_REPLY && reply.words[0] == 0 {
        Ok(())
    } else {
        debug_log("[CALENDAR-KV] put failed\n");
        Err(KvClientError::Unavailable)
    }
}

fn kv_get(key: &str) -> Result<Vec<u8>, KvClientError> {
    if key.len() > SHM_PAGE {
        return Err(KvClientError::TooLarge);
    }
    let cap = kv_cap()?;
    let (key_ptr, key_tok) = shm_alloc().map_err(|_| KvClientError::Unavailable)?;
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
    }
    let msg = IpcMsg::with_label(KV_GET_SHM2)
        .word(0, key.len() as u64)
        .with_cap(0, key_tok);
    let reply_res = ipc_call_timeout(cap, msg, KV_TIMEOUT_MS);
    let _ = shm_free(key_tok);
    let reply = reply_res.map_err(|_| {
        debug_log("[CALENDAR-KV] get timeout/error\n");
        KvClientError::Unavailable
    })?;
    if reply.label == KV_ERROR && reply.words[0] == 2 {
        return Err(KvClientError::NotFound);
    }
    if reply.label != KV_VALUE {
        debug_log("[CALENDAR-KV] get failed\n");
        return Err(KvClientError::Unavailable);
    }
    let len = (reply.words[0] as usize).min(SHM_PAGE);
    if len == 0 {
        return Ok(Vec::new());
    }
    let tok = reply.caps[0];
    if tok == CapabilityToken::INVALID {
        return Err(KvClientError::Unavailable);
    }
    let ptr = shm_map(tok).map_err(|_| {
        let _ = shm_free(tok);
        KvClientError::Unavailable
    })?;
    let value = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
    let _ = shm_free(tok);
    Ok(value)
}

fn kv_delete(key: &str) -> Result<(), KvClientError> {
    if key.len() > SHM_PAGE {
        return Err(KvClientError::TooLarge);
    }
    let cap = kv_cap()?;
    let (key_ptr, key_tok) = shm_alloc().map_err(|_| KvClientError::Unavailable)?;
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), key_ptr, key.len());
    }
    let msg = IpcMsg::with_label(KV_DELETE_SHM2)
        .word(0, key.len() as u64)
        .with_cap(0, key_tok);
    let reply_res = ipc_call_timeout(cap, msg, KV_TIMEOUT_MS);
    let _ = shm_free(key_tok);
    let reply = reply_res.map_err(|_| {
        debug_log("[CALENDAR-KV] delete timeout/error\n");
        KvClientError::Unavailable
    })?;
    if reply.label == KV_REPLY && reply.words[0] == 0 {
        Ok(())
    } else if reply.label == KV_ERROR && reply.words[0] == 2 {
        Err(KvClientError::NotFound)
    } else {
        debug_log("[CALENDAR-KV] delete failed\n");
        Err(KvClientError::Unavailable)
    }
}

fn encode_event(event: &CalendarEvent) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"SCAL2\n");
    push_field(&mut out, "id", &format!("{}", event.id));
    push_field(&mut out, "title", event.title.as_str());
    push_field(&mut out, "date", event.date.as_str());
    push_field(&mut out, "start", event.start_time.as_str());
    push_field(&mut out, "end", event.end_time.as_str());
    push_field(&mut out, "all_day", if event.all_day { "1" } else { "0" });
    push_field(&mut out, "notes", event.notes.as_str());
    push_field(&mut out, "created_at", &format!("{}", event.created_at));
    push_field(&mut out, "updated_at", &format!("{}", event.updated_at));
    out
}

fn push_field(out: &mut Vec<u8>, name: &str, value: &str) {
    out.extend_from_slice(name.as_bytes());
    out.push(b'=');
    for &byte in value.as_bytes() {
        match byte {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            _ => out.push(byte),
        }
    }
    out.push(b'\n');
}

fn decode_event(bytes: &[u8]) -> Option<CalendarEvent> {
    let text = core::str::from_utf8(bytes).ok()?;
    if !text.starts_with("SCAL2\n") {
        return None;
    }
    let mut event = CalendarEvent::new(0);
    for line in text.lines().skip(1) {
        let Some(eq) = line.find('=') else { continue };
        let name = &line[..eq];
        let value = unescape_field(&line[eq + 1..]);
        match name {
            "id" => event.id = parse_u64(&value)?,
            "title" => event.title.set(&value),
            "date" => event.date.set(&value),
            "start" => event.start_time.set(&value),
            "end" => event.end_time.set(&value),
            "all_day" => event.all_day = value == "1",
            "notes" => event.notes.set(&value),
            "created_at" => event.created_at = parse_u64(&value).unwrap_or(0),
            "updated_at" => event.updated_at = parse_u64(&value).unwrap_or(0),
            _ => {}
        }
    }
    if event.id == 0 || !valid_date_str(event.date.as_str()) || event.title.len == 0 {
        return None;
    }
    Some(event)
}

fn unescape_field(value: &str) -> String {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            out.push(if ch == 'n' { '\n' } else { ch });
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    out
}

fn push_i32_fixed<const N: usize>(out: &mut SlotString<N>, mut n: i32, digits: usize) {
    if n < 0 {
        out.push('-');
        n = -n;
    }
    let mut buf = [0u8; 12];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    let len = buf.len() - i;
    for _ in len..digits {
        out.push('0');
    }
    if len == 0 {
        if digits == 0 {
            out.push('0');
        }
        return;
    }
    for &b in &buf[i..] {
        out.push(b as char);
    }
}

fn push_i32_into_string(out: &mut String, mut n: i32) {
    if n == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 12];
    let mut i = buf.len();
    let neg = n < 0;
    if neg {
        n = -n;
    }
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    if neg {
        out.push('-');
    }
    for &b in &buf[i..] {
        out.push(b as char);
    }
}

fn parse_i32(s: &str) -> i32 {
    let mut n = 0i32;
    for c in s.chars() {
        if c.is_ascii_digit() {
            n = n * 10 + (c as i32 - '0' as i32);
        }
    }
    n
}

fn valid_date_str(s: &str) -> bool {
    parse_date_parts(s).is_some()
}

fn normalize_date_str(s: &str) -> Option<SlotString<DATE_LEN>> {
    let (year, month, day) = parse_date_parts(s)?;
    Some(CalendarEvent::format_date(year, month, day))
}

fn parse_date_parts(s: &str) -> Option<(i32, i32, i32)> {
    let first_sep = s.find(|ch| ch == '-' || ch == '/')?;
    let rest = &s[first_sep + 1..];
    let second_rel = rest.find(|ch| ch == '-' || ch == '/')?;
    let second_sep = first_sep + 1 + second_rel;
    let year_s = &s[..first_sep];
    let month_s = &s[first_sep + 1..second_sep];
    let day_s = &s[second_sep + 1..];
    if year_s.len() != 4
        || month_s.is_empty()
        || month_s.len() > 2
        || day_s.is_empty()
        || day_s.len() > 2
    {
        return None;
    }
    for part in [year_s, month_s, day_s] {
        if !part.as_bytes().iter().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
    }
    let year = parse_i32(year_s);
    let month = parse_i32(month_s);
    let day = parse_i32(day_s);
    if month >= 1 && month <= 12 && day >= 1 && day <= days_in_month(year, month) {
        Some((year, month, day))
    } else {
        None
    }
}

fn valid_time_str(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }
    let bytes = s.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return false;
    }
    for &i in &[0usize, 1, 3, 4] {
        if !bytes[i].is_ascii_digit() {
            return false;
        }
    }
    let hour = parse_i32(&s[0..2]);
    let minute = parse_i32(&s[3..5]);
    hour >= 0 && hour <= 23 && minute >= 0 && minute <= 59
}

fn user_data_dir() -> String {
    let home = if libc::getuid() == 0 {
        "/root"
    } else {
        "/home/user"
    };
    let mut path = String::from(home);
    path.push_str("/.local/share/sunlight-calendar");
    path
}

fn events_file_path() -> String {
    let mut path = user_data_dir();
    path.push_str("/events.dat");
    path
}

fn read_file_bytes(path: &str) -> Option<Vec<u8>> {
    let path_bytes = path.as_bytes();
    let fd = libc::open(path_bytes).ok()?;
    let stat = libc::stat(path_bytes).ok()?;
    let size = stat.size as usize;
    if size == 0 {
        let _ = libc::close(fd);
        return Some(Vec::new());
    }
    let mut data = Vec::with_capacity(size.min(64 * 1024));
    let mut chunk = [0u8; 4096];
    let mut total = 0usize;
    loop {
        let n = libc::read(fd, &mut chunk).ok()?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..n]);
        total += n;
        if total >= size || total >= 64 * 1024 {
            break;
        }
    }
    let _ = libc::close(fd);
    Some(data)
}

const EVENTS_MAGIC: u32 = 0xCA1ECA1E;

fn load_events() -> Vec<CalendarEvent> {
    let path = events_file_path();
    let data = match read_file_bytes(&path) {
        Some(d) => d,
        None => return Vec::new(),
    };
    if data.len() < 10 {
        return Vec::new();
    }

    let magic = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if magic != EVENTS_MAGIC {
        return Vec::new();
    }
    let _version = u16::from_le_bytes([data[4], data[5]]);
    let count = u32::from_le_bytes([data[6], data[7], data[8], data[9]]) as usize;
    let count = count.min(MAX_EVENTS);
    let mut events = Vec::with_capacity(count);
    let mut offset = 10usize;

    for _ in 0..count {
        if offset + 4 > data.len() {
            break;
        }
        let total_len = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        offset += 4;
        if offset + total_len > data.len() {
            break;
        }

        let rec_data = &data[offset..offset + total_len];
        let mut ri = 0usize;

        if ri + 8 > rec_data.len() {
            break;
        }
        let id = u64::from_le_bytes([
            rec_data[ri],
            rec_data[ri + 1],
            rec_data[ri + 2],
            rec_data[ri + 3],
            rec_data[ri + 4],
            rec_data[ri + 5],
            rec_data[ri + 6],
            rec_data[ri + 7],
        ]);
        ri += 8;

        let mut event = CalendarEvent::new(id);
        event.created_at = 0;
        event.updated_at = 0;

        if ri + 2 > rec_data.len() {
            break;
        }
        let title_len = u16::from_le_bytes([rec_data[ri], rec_data[ri + 1]]) as usize;
        ri += 2;
        if ri + title_len > rec_data.len() {
            break;
        }
        if title_len <= TITLE_LEN {
            event.title.buf[..title_len].copy_from_slice(&rec_data[ri..ri + title_len]);
            event.title.len = title_len;
        }
        ri += title_len;

        if ri + DATE_LEN > rec_data.len() {
            break;
        }
        event.date.buf[..DATE_LEN].copy_from_slice(&rec_data[ri..ri + DATE_LEN]);
        event.date.len = DATE_LEN;
        ri += DATE_LEN;

        if ri + TIME_LEN > rec_data.len() {
            break;
        }
        event.start_time.buf[..TIME_LEN].copy_from_slice(&rec_data[ri..ri + TIME_LEN]);
        event.start_time.len = TIME_LEN;
        ri += TIME_LEN;

        if ri + TIME_LEN > rec_data.len() {
            break;
        }
        event.end_time.buf[..TIME_LEN].copy_from_slice(&rec_data[ri..ri + TIME_LEN]);
        event.end_time.len = TIME_LEN;
        ri += TIME_LEN;

        if ri + 1 > rec_data.len() {
            break;
        }
        event.all_day = rec_data[ri] != 0;
        ri += 1;

        if ri + 2 > rec_data.len() {
            break;
        }
        let notes_len = u16::from_le_bytes([rec_data[ri], rec_data[ri + 1]]) as usize;
        ri += 2;
        if ri + notes_len > rec_data.len() {
            break;
        }
        if notes_len <= NOTES_LEN {
            event.notes.buf[..notes_len].copy_from_slice(&rec_data[ri..ri + notes_len]);
            event.notes.len = notes_len;
        }
        ri += notes_len;

        if ri + 16 > rec_data.len() {
            break;
        }
        event.created_at = u64::from_le_bytes([
            rec_data[ri],
            rec_data[ri + 1],
            rec_data[ri + 2],
            rec_data[ri + 3],
            rec_data[ri + 4],
            rec_data[ri + 5],
            rec_data[ri + 6],
            rec_data[ri + 7],
        ]);
        event.updated_at = u64::from_le_bytes([
            rec_data[ri + 8],
            rec_data[ri + 9],
            rec_data[ri + 10],
            rec_data[ri + 11],
            rec_data[ri + 12],
            rec_data[ri + 13],
            rec_data[ri + 14],
            rec_data[ri + 15],
        ]);

        events.push(event);
        offset += total_len;
    }

    events
}

struct CalendarIcons {
    prev: Option<TgaImage>,
    next: Option<TgaImage>,
    add: Option<TgaImage>,
    menu: Option<TgaImage>,
    event: Option<TgaImage>,
}

impl CalendarIcons {
    fn load() -> Self {
        Self {
            prev: TgaImage::parse(ICON_PREV_TGA).ok(),
            next: TgaImage::parse(ICON_NEXT_TGA).ok(),
            add: TgaImage::parse(ICON_ADD_TGA).ok(),
            menu: TgaImage::parse(ICON_MENU_TGA).ok(),
            event: TgaImage::parse(ICON_EVENT_TGA).ok(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    NewEvent,
    Today,
    Refresh,
    About,
}

#[derive(Clone, Copy)]
struct MenuItem {
    action: MenuAction,
    label: &'static str,
    rect: Rect,
}

#[derive(Clone, Copy)]
struct PopupMenu {
    rect: Rect,
    items: [MenuItem; 4],
    count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogField {
    Title,
    Date,
    StartTime,
    EndTime,
    Notes,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConfirmKind {
    DeleteEvent { event_id: u64 },
    ClearField,
}

#[derive(Clone, Copy)]
struct DialogState {
    visible: bool,
    editing_id: Option<u64>,
    title: SlotString<TITLE_LEN>,
    date: SlotString<DATE_LEN>,
    start_time: SlotString<TIME_LEN>,
    end_time: SlotString<TIME_LEN>,
    all_day: bool,
    notes: SlotString<NOTES_LEN>,
    error_msg: SlotString<MSG_LEN>,
    focus: DialogField,
    cursor: usize,
}

impl DialogState {
    fn new() -> Self {
        Self {
            visible: false,
            editing_id: None,
            title: SlotString::empty(),
            date: SlotString::empty(),
            start_time: SlotString::empty(),
            end_time: SlotString::empty(),
            all_day: false,
            notes: SlotString::empty(),
            error_msg: SlotString::empty(),
            focus: DialogField::Title,
            cursor: 0,
        }
    }

    fn reset(&mut self) {
        self.visible = false;
        self.editing_id = None;
        self.title.clear();
        self.date.clear();
        self.start_time.clear();
        self.end_time.clear();
        self.all_day = false;
        self.notes.clear();
        self.error_msg.clear();
        self.focus = DialogField::Title;
        self.cursor = 0;
    }

    fn start_new(&mut self, date: &SlotString<DATE_LEN>) {
        self.visible = true;
        self.editing_id = None;
        self.title.clear();
        self.date = *date;
        self.start_time.set("09:00");
        self.end_time.set("10:00");
        self.all_day = false;
        self.notes.clear();
        self.error_msg.clear();
        self.focus = DialogField::Title;
        self.cursor = 0;
    }

    fn start_edit(&mut self, event: &CalendarEvent) {
        self.visible = true;
        self.editing_id = Some(event.id);
        self.title = event.title;
        self.date = event.date;
        self.start_time = event.start_time;
        self.end_time = event.end_time;
        self.all_day = event.all_day;
        self.notes = event.notes;
        self.error_msg.clear();
        self.focus = DialogField::Title;
        self.cursor = 0;
    }

    fn validate(&self) -> bool {
        self.validation_error().is_none()
    }

    fn validation_error(&self) -> Option<&'static str> {
        if self.title.len == 0 {
            return Some("Title is required");
        }
        if !valid_date_str(self.date.as_str()) {
            return Some("Date example: 2026/10/6 or 2026-10-06");
        }
        if !valid_time_str(self.start_time.as_str()) || !valid_time_str(self.end_time.as_str()) {
            return Some("Time example: 09:00");
        }
        if !self.all_day {
            if self.start_time.len > 0 && self.end_time.len > 0 {
                let sh = if self.start_time.len >= 2 {
                    parse_i32(&self.start_time.as_str()[..2])
                } else {
                    0
                };
                let sm = if self.start_time.len >= 5 {
                    parse_i32(&self.start_time.as_str()[3..5])
                } else {
                    0
                };
                let eh = if self.end_time.len >= 2 {
                    parse_i32(&self.end_time.as_str()[..2])
                } else {
                    0
                };
                let em = if self.end_time.len >= 5 {
                    parse_i32(&self.end_time.as_str()[3..5])
                } else {
                    0
                };
                if eh < sh || (eh == sh && em < sm) {
                    return Some("End time must be after start");
                }
            }
        }
        None
    }

    fn to_event(&self, id: u64) -> CalendarEvent {
        let mut event = CalendarEvent::new(id);
        event.title = self.title;
        event.date = normalize_date_str(self.date.as_str()).unwrap_or(self.date);
        event.start_time = self.start_time;
        event.end_time = self.end_time;
        event.all_day = self.all_day;
        event.notes = self.notes;
        event.created_at = monotonic_millis();
        event.updated_at = monotonic_millis();
        event
    }
}

struct Confirmation {
    visible: bool,
    kind: ConfirmKind,
    message: SlotString<128>,
}

impl Confirmation {
    fn new() -> Self {
        Self {
            visible: false,
            kind: ConfirmKind::ClearField,
            message: SlotString::empty(),
        }
    }

    fn show_delete(&mut self, event_id: u64) {
        self.visible = true;
        self.kind = ConfirmKind::DeleteEvent { event_id };
        self.message.set("Delete this event?");
    }
}

struct CalendarApp {
    events: Vec<CalendarEvent>,
    store: KvCalendarStore,
    memory_events: Vec<CalendarEvent>,
    icons: CalendarIcons,
    view_year: i32,
    view_month: i32,
    today_year: i32,
    today_month: i32,
    today_day: i32,
    sel_year: i32,
    sel_month: i32,
    sel_day: i32,
    selected_event_idx: Option<usize>,
    locale_str: SlotString<32>,
    timezone_str: SlotString<64>,
    timezone_abbr: SlotString<8>,
    tz_offset_secs: i64,
    menu: Option<PopupMenu>,
    dialog: DialogState,
    confirm: Confirmation,
    toolbar_hover: Option<usize>,
    dialog_hover_btn: Option<usize>,
    data_loaded: bool,
    status_msg: SlotString<MSG_LEN>,
    // Sunlight Reminders & Tasks integration via sunlight-kv
    reminder_lists: [TaskList; 3],
    task_previews: Vec<TaskPreview>,
    reminder_previews: Vec<ReminderPreview>,
    tasks_loaded_for: SlotString<DATE_LEN>,
    reminders_loaded_for: SlotString<DATE_LEN>,
    reminder_lists_loaded: bool,
    last_preview_refresh_ms: u64,
}

impl CalendarApp {
    fn new() -> Self {
        let local = decompose_utc(get_time_utc(), None);
        Self {
            events: Vec::new(),
            store: KvCalendarStore::new(),
            memory_events: Vec::new(),
            icons: CalendarIcons::load(),
            view_year: local.0,
            view_month: local.1,
            today_year: local.0,
            today_month: local.1,
            today_day: local.2,
            sel_year: local.0,
            sel_month: local.1,
            sel_day: local.2,
            selected_event_idx: None,
            locale_str: SlotString::empty(),
            timezone_str: SlotString::empty(),
            timezone_abbr: SlotString::empty(),
            tz_offset_secs: 0,
            menu: None,
            dialog: DialogState::new(),
            confirm: Confirmation::new(),
            toolbar_hover: None,
            dialog_hover_btn: None,
            data_loaded: false,
            status_msg: SlotString::empty(),
            reminder_lists: [
                TaskList::new("inbox", "Inbox", 0, 0).unwrap(),
                TaskList::new("work", "Work", 0, 0).unwrap(),
                TaskList::new("personal", "Personal", 0, 0).unwrap(),
            ],
            task_previews: Vec::new(),
            reminder_previews: Vec::new(),
            tasks_loaded_for: SlotString::empty(),
            reminders_loaded_for: SlotString::empty(),
            reminder_lists_loaded: false,
            last_preview_refresh_ms: 0,
        }
    }

    fn header_rect(&self) -> Rect {
        Rect::new(0, 0, WIN_W, HEADER_H)
    }

    fn toolbar_rect(&self) -> Rect {
        Rect::new(0, HEADER_H as i32, WIN_W, TOOLBAR_H)
    }

    fn main_rect(&self) -> Rect {
        Rect::new(
            0,
            (HEADER_H + TOOLBAR_H) as i32,
            WIN_W,
            WIN_H - HEADER_H - TOOLBAR_H - STATUS_H,
        )
    }

    fn grid_rect(&self) -> Rect {
        Rect::new(
            0,
            (HEADER_H + TOOLBAR_H) as i32,
            WIN_W - SIDEBAR_W,
            WIN_H - HEADER_H - TOOLBAR_H - STATUS_H,
        )
    }

    fn sidebar_rect(&self) -> Rect {
        Rect::new(
            (WIN_W - SIDEBAR_W) as i32,
            (HEADER_H + TOOLBAR_H) as i32,
            SIDEBAR_W,
            WIN_H - HEADER_H - TOOLBAR_H - STATUS_H,
        )
    }

    fn calendar_body_rect(&self) -> Rect {
        self.grid_rect().inset(CALENDAR_INNER_PAD)
    }

    fn calendar_sections(&self) -> (Rect, Rect) {
        let body = self.calendar_body_rect();
        let month_h = self.calendar_month_height(body.h as i32).max(0) as u32;
        let preview_h = body
            .h
            .saturating_sub(month_h)
            .saturating_sub(CALENDAR_SECTION_GAP as u32);
        let section_heights = [month_h, preview_h];
        let mut sections = VBox::new(body)
            .with_spacing(CALENDAR_SECTION_GAP as u32)
            .layout(&section_heights);
        let month = sections.next().unwrap_or(body);
        let preview = sections
            .next()
            .unwrap_or(Rect::new(body.x, body.bottom(), body.w, 0));
        (month, preview)
    }

    fn calendar_month_rect(&self) -> Rect {
        self.calendar_sections().0
    }

    fn calendar_preview_rect(&self) -> Rect {
        self.calendar_sections().1
    }

    fn calendar_month_height(&self, body_h: i32) -> i32 {
        let desired = (body_h * 62) / 100;
        let max_month = body_h - 132;
        if max_month <= 0 {
            return body_h;
        }
        desired.clamp(220.min(max_month), max_month)
    }

    fn sidebar_actions_rects(&self, rect: Rect) -> (Rect, Rect, i32) {
        let action_w = rect.w.saturating_sub((PAD as u32) * 2);
        let add_btn_y = rect.bottom() - SIDEBAR_BOTTOM_MARGIN - SIDEBAR_ACTION_H;
        let tasks_btn_y = add_btn_y - SIDEBAR_ACTION_GAP - SIDEBAR_ACTION_H;
        let tasks_btn = Rect::new(rect.x + PAD, tasks_btn_y, action_w, SIDEBAR_ACTION_H as u32);
        let add_btn = Rect::new(rect.x + PAD, add_btn_y, action_w, SIDEBAR_ACTION_H as u32);
        let events_limit_y = tasks_btn_y - 12;
        (tasks_btn, add_btn, events_limit_y)
    }

    fn status_rect(&self) -> Rect {
        Rect::new(0, (WIN_H - STATUS_H) as i32, WIN_W, STATUS_H)
    }

    fn day_at_point(&self, x: i32, y: i32) -> Option<i32> {
        let month_rect = self.calendar_month_rect();
        let month_inner = month_rect.inset(12);
        let layout = MonthGridLayout::new(month_inner);
        let day_idx = layout.contains(month_inner, x, y)?;

        let start_wday = weekday_mon0(self.view_year, self.view_month, 1);
        if day_idx < start_wday {
            return None;
        }

        let day = day_idx - start_wday + 1;
        let total = days_in_month(self.view_year, self.view_month);
        if day > total {
            return None;
        }

        Some(day)
    }

    fn events_for_date(&self, year: i32, month: i32, day: i32) -> Vec<&CalendarEvent> {
        let target = CalendarEvent::format_date(year, month, day);
        let target_str = target.as_str();
        self.events
            .iter()
            .filter(|e| e.date.as_str() == target_str)
            .collect()
    }

    fn month_name(&self, month: i32, long: bool) -> &'static str {
        month_name(month as u8, long, self.locale_str.as_str())
    }

    fn weekday_short(&self, iso_wd: u8) -> &'static str {
        weekday_name(iso_wd, false, self.locale_str.as_str())
    }

    fn toolbar_buttons(&self) -> [Rect; 5] {
        let rect = self.toolbar_rect();
        let y = rect.y + (rect.h as i32 - TOOLBAR_BTN_W as i32) / 2;
        let gap = 4i32;
        let mut x = rect.x + PAD;

        let mut btns = [Rect::new(0, 0, 0, 0); 5];

        btns[0] = Rect::new(x, y, TOOLBAR_BTN_W, TOOLBAR_BTN_W);
        x += TOOLBAR_BTN_W as i32 + gap;

        let today_w = 56u32;
        btns[1] = Rect::new(x, y, today_w, TOOLBAR_BTN_W);
        x += today_w as i32 + gap;

        btns[2] = Rect::new(x, y, TOOLBAR_BTN_W, TOOLBAR_BTN_W);

        btns[4] = Rect::new(
            rect.right() - PAD - TOOLBAR_BTN_W as i32,
            y,
            TOOLBAR_BTN_W,
            TOOLBAR_BTN_W,
        );

        let add_btn_x = rect.right() - PAD - TOOLBAR_BTN_W as i32 - gap - 36;
        btns[3] = Rect::new(add_btn_x, y, 36, TOOLBAR_BTN_W);

        btns
    }

    fn toolbar_hit(&self, x: i32, y: i32) -> Option<usize> {
        let point = Point::new(x, y);
        self.toolbar_buttons()
            .iter()
            .position(|btn| btn.contains(point))
    }

    fn menu_specs() -> &'static [(MenuAction, &'static str)] {
        const ITEMS: &[(MenuAction, &'static str)] = &[
            (MenuAction::NewEvent, "New Event"),
            (MenuAction::Today, "Today"),
            (MenuAction::Refresh, "Refresh"),
            (MenuAction::About, "About Calendar"),
        ];
        ITEMS
    }

    fn open_menu(&mut self, x: i32, y: i32) {
        let specs = Self::menu_specs();
        let menu_item_h = 28u32;
        let menu_w = 172u32;
        let menu_h = menu_item_h * specs.len() as u32 + 8;
        let max_x = WIN_W as i32 - menu_w as i32 - 6;
        let max_y = WIN_H as i32 - menu_h as i32 - STATUS_H as i32 - 6;
        let rect = Rect::new(
            x.clamp(6, max_x.max(6)),
            y.clamp(6, max_y.max(6)),
            menu_w,
            menu_h,
        );
        let mut items = [MenuItem {
            action: MenuAction::About,
            label: "",
            rect: Rect::new(0, 0, 0, 0),
        }; 4];
        for (i, (action, label)) in specs.iter().enumerate() {
            items[i] = MenuItem {
                action: *action,
                label,
                rect: Rect::new(
                    rect.x + 4,
                    rect.y + 4 + i as i32 * menu_item_h as i32,
                    menu_w - 8,
                    menu_item_h,
                ),
            };
        }
        self.menu = Some(PopupMenu {
            rect,
            items,
            count: specs.len(),
        });
    }

    fn close_menu(&mut self) {
        self.menu = None;
    }

    fn dialog_rect(&self) -> Rect {
        let h = 360u32;
        Rect::new(
            ((WIN_W - DIALOG_W) / 2) as i32,
            ((WIN_H - h) / 2) as i32,
            DIALOG_W,
            h,
        )
    }

    fn dialog_button_rects(&self) -> [Rect; 3] {
        let panel = self.dialog_rect();
        let count = if self.dialog.editing_id.is_some() {
            3usize
        } else {
            2usize
        };
        let total_w = count as u32 * DIALOG_BTN_W + (count as u32 - 1) * DIALOG_BTN_GAP;
        let mut x = panel.x + ((panel.w as i32 - total_w as i32) / 2);
        let y = panel.bottom() - DIALOG_PAD - DIALOG_BTN_H as i32;
        let mut btns = [Rect::new(0, 0, 0, 0); 3];
        for i in 0..count {
            btns[i] = Rect::new(x, y, DIALOG_BTN_W, DIALOG_BTN_H);
            x += DIALOG_BTN_W as i32 + DIALOG_BTN_GAP as i32;
        }
        btns
    }

    fn dialog_button_hit(&self, x: i32, y: i32) -> Option<usize> {
        let point = Point::new(x, y);
        let btns = self.dialog_button_rects();
        btns.iter().position(|btn| btn.contains(point))
    }

    fn dialog_field_rects(&self) -> [Rect; 5] {
        let panel = self.dialog_rect();
        let field_h = 28u32;
        let label_w = 70u32;
        let input_x = panel.x + DIALOG_PAD + label_w as i32;
        let input_w = panel.w - label_w - (DIALOG_PAD as u32) * 2;
        let mut y = panel.y + 36;
        let mut fields = [Rect::new(0, 0, 0, 0); 5];

        fields[0] = Rect::new(input_x, y, input_w, field_h);
        y += field_h as i32 + 8;

        fields[1] = Rect::new(input_x, y, 190, field_h);
        y += field_h as i32 + 8;

        let time_w = 72u32;
        fields[2] = Rect::new(input_x, y, time_w, field_h);
        fields[3] = Rect::new(input_x + time_w as i32 + 44, y, time_w, field_h);
        y += field_h as i32 + 8;

        let notes_h = 60u32;
        fields[4] = Rect::new(input_x, y, input_w, notes_h);

        fields
    }

    fn focus_field_at_point(&self, x: i32, y: i32) -> Option<DialogField> {
        let fields = self.dialog_field_rects();
        let point = Point::new(x, y);
        let map = [
            DialogField::Title,
            DialogField::Date,
            DialogField::StartTime,
            DialogField::EndTime,
            DialogField::Notes,
        ];
        for (i, field) in fields.iter().enumerate() {
            if field.contains(point) {
                return Some(map[i]);
            }
        }
        None
    }

    fn next_event_id(&self) -> u64 {
        let mut max_id = 0u64;
        for event in &self.events {
            if event.id > max_id {
                max_id = event.id;
            }
        }
        max_id + 1
    }

    fn add_event_from_dialog(&mut self) {
        let id = self.next_event_id();
        let mut event = self.dialog.to_event(id);
        event.created_at = monotonic_millis();
        event.updated_at = event.created_at;
        if self.persist_event(event) {
            self.dialog.reset();
        }
    }

    fn update_event_from_dialog(&mut self) {
        let Some(edit_id) = self.dialog.editing_id else {
            return;
        };
        let mut event = self.dialog.to_event(edit_id);
        event.created_at = self
            .events
            .iter()
            .find(|existing| existing.id == edit_id)
            .map(|existing| existing.created_at)
            .unwrap_or_else(monotonic_millis);
        event.updated_at = monotonic_millis();
        if self.persist_event(event) {
            self.dialog.reset();
        }
    }

    fn delete_event(&mut self, event_id: u64) {
        let result = if self.store.mode() == StoreMode::Kv {
            self.store.delete_event(event_id)
        } else {
            self.memory_events.retain(|e| e.id != event_id);
            Ok(())
        };
        if result.is_err() {
            self.status_msg.set("Delete failed; check sunlight-kv");
            return;
        }
        self.events.retain(|e| e.id != event_id);
        self.status_msg.set("Event deleted");
        self.confirm.visible = false;
        self.dialog.reset();
        self.save_and_refresh();
    }

    fn save_and_refresh(&mut self) {
        let (y, m, d) = (self.sel_year, self.sel_month, self.sel_day);
        self.selected_event_idx = self.events_for_date(y, m, d).first().map(|_| 0);
        self.save_selection_settings();
    }

    fn persist_event(&mut self, event: CalendarEvent) -> bool {
        let result = if self.store.mode() == StoreMode::Kv {
            self.store.save_event(&event)
        } else {
            if let Some(existing) = self
                .memory_events
                .iter_mut()
                .find(|existing| existing.id == event.id)
            {
                *existing = event;
            } else {
                self.memory_events.push(event);
            }
            Ok(())
        };
        if result.is_err() {
            self.dialog.error_msg.set("Could not save to sunlight-kv");
            self.status_msg.set("Persistence error");
            return false;
        }
        if let Some(existing) = self
            .events
            .iter_mut()
            .find(|existing| existing.id == event.id)
        {
            *existing = event;
        } else {
            self.events.push(event);
        }
        self.sort_events();
        self.status_msg.set("Event saved");
        self.save_and_refresh();
        true
    }

    fn sort_events(&mut self) {
        self.events.sort_by(|a, b| {
            a.date
                .as_str()
                .cmp(b.date.as_str())
                .then(a.start_time.as_str().cmp(b.start_time.as_str()))
        });
    }

    fn load_calendar_data(&mut self) {
        self.reminder_lists_loaded = false;
        self.tasks_loaded_for.clear();
        self.reminders_loaded_for.clear();
        match self.store.load_events() {
            Ok(events) => {
                self.events = events;
                self.sort_events();
                self.status_msg.set("Loaded from sunlight-kv");
                self.restore_selection();
                self.migrate_old_file_if_needed();
                self.save_and_refresh();
                self.load_reminder_previews_for_selection(true);
            }
            Err(_) => {
                debug_log("[CALENDAR] sunlight-kv unavailable; using memory fallback\n");
                self.events = self.memory_events.clone();
                self.status_msg.set("sunlight-kv unavailable; memory only");
                self.load_reminder_previews_for_selection(true);
            }
        }
    }

    fn restore_selection(&mut self) {
        if let Ok(Some(bytes)) = self.store.load_setting("selected-date") {
            if let Ok(text) = core::str::from_utf8(&bytes) {
                if valid_date_str(text) {
                    self.sel_year = parse_i32(&text[0..4]);
                    self.sel_month = parse_i32(&text[5..7]);
                    self.sel_day = parse_i32(&text[8..10]);
                }
            }
        }
        if let Ok(Some(bytes)) = self.store.load_setting("view-month") {
            if let Ok(text) = core::str::from_utf8(&bytes) {
                if text.len() == 7 && text.as_bytes()[4] == b'-' {
                    let year = parse_i32(&text[0..4]);
                    let month = parse_i32(&text[5..7]);
                    if month >= 1 && month <= 12 {
                        self.view_year = year;
                        self.view_month = month;
                    }
                }
            }
        }
    }

    fn save_selection_settings(&mut self) {
        if self.store.mode() != StoreMode::Kv {
            return;
        }
        let date = CalendarEvent::format_date(self.sel_year, self.sel_month, self.sel_day);
        let _ = self
            .store
            .save_setting("selected-date", date.as_str().as_bytes());
        let mut month = SlotString::<7>::empty();
        push_i32_fixed(&mut month, self.view_year, 4);
        month.push('-');
        push_i32_fixed(&mut month, self.view_month, 2);
        let _ = self
            .store
            .save_setting("view-month", month.as_str().as_bytes());
    }

    fn migrate_old_file_if_needed(&mut self) {
        if self.store.migration_complete() {
            return;
        }
        let old_events = load_events();
        if old_events.is_empty() {
            let _ = self.store.mark_migration_complete();
            return;
        }
        let mut imported = 0usize;
        for event in old_events {
            if self.store.save_event(&event).is_ok() {
                imported += 1;
            }
        }
        if imported > 0 {
            if let Ok(events) = self.store.load_events() {
                self.events = events;
                self.sort_events();
            }
        }
        let _ = self.store.mark_migration_complete();
        self.status_msg.set("Imported old calendar file");
    }

    fn ensure_reminder_lists(&mut self) {
        if self.reminder_lists_loaded {
            return;
        }
        for (idx, (id, _name)) in DEFAULT_LISTS.iter().enumerate() {
            let key = list_key(id);
            match kv_get(&key) {
                Ok(bytes) => {
                    if let Some(list) = decode_list(&bytes) {
                        self.reminder_lists[idx] = list;
                    }
                }
                Err(KvClientError::NotFound) => {}
                Err(_) => {}
            }
        }
        self.reminder_lists_loaded = true;
    }

    fn dynamic_list_name(&self, list_id: &str) -> Option<String> {
        for list in &self.reminder_lists {
            if list.id.as_str() == list_id {
                let name = list.name.as_str();
                if !name.is_empty() {
                    return Some(String::from(name));
                }
            }
        }
        None
    }

    fn selected_date_str(&self) -> SlotString<DATE_LEN> {
        CalendarEvent::format_date(self.sel_year, self.sel_month, self.sel_day)
    }

    fn selected_legacy_slash_date_str(&self) -> SlotString<DATE_LEN> {
        let mut out = SlotString::empty();
        push_i32_fixed(&mut out, self.sel_year, 4);
        out.push('/');
        push_i32_fixed(&mut out, self.sel_month, 2);
        out.push('/');
        push_i32_fixed(&mut out, self.sel_day, 2);
        out
    }

    fn load_reminder_id_list(&self, key: &str) -> Result<Vec<u64>, KvClientError> {
        match kv_get(key) {
            Ok(bytes) => Ok(parse_id_list(&bytes).unwrap_or_default()),
            Err(err) => Err(err),
        }
    }

    fn load_reminder_id_list_with_legacy(
        &self,
        canonical_key: &str,
        legacy_key: &str,
    ) -> Result<Vec<u64>, KvClientError> {
        match self.load_reminder_id_list(canonical_key) {
            Ok(ids) => Ok(ids),
            Err(KvClientError::NotFound) => match self.load_reminder_id_list(legacy_key) {
                Ok(ids) => Ok(ids),
                Err(KvClientError::NotFound) => Ok(Vec::new()),
                Err(err) => Err(err),
            },
            Err(err) => Err(err),
        }
    }

    fn load_reminder_previews_for_selection(&mut self, force: bool) {
        let date_slot = self.selected_date_str();
        let date_str = date_slot.as_str();
        if date_str.is_empty() {
            return;
        }
        if !force
            && self.tasks_loaded_for.as_str() == date_str
            && self.reminders_loaded_for.as_str() == date_str
        {
            return;
        }

        self.ensure_reminder_lists();
        let legacy_date_slot = self.selected_legacy_slash_date_str();
        let due_ids = match self.load_reminder_id_list_with_legacy(
            &by_date_list_key(date_str),
            &by_date_list_key(legacy_date_slot.as_str()),
        ) {
            Ok(ids) => ids,
            Err(_) => {
                self.last_preview_refresh_ms = monotonic_millis();
                return;
            }
        };
        let reminder_ids = match self.load_reminder_id_list_with_legacy(
            &reminder_date_list_key(date_str),
            &reminder_date_list_key(legacy_date_slot.as_str()),
        ) {
            Ok(ids) => ids,
            Err(_) => {
                self.last_preview_refresh_ms = monotonic_millis();
                return;
            }
        };
        let mut load_failed = false;
        let selected = build_selected_day_previews(
            date_str,
            &due_ids,
            &reminder_ids,
            |id| match kv_get(&task_key(id)) {
                Ok(rec) => decode_task(&rec),
                Err(KvClientError::NotFound) => None,
                Err(_) => {
                    load_failed = true;
                    None
                }
            },
            |list_id| self.dynamic_list_name(list_id),
        );
        if load_failed {
            self.last_preview_refresh_ms = monotonic_millis();
            return;
        }

        self.task_previews.clear();
        for preview in selected.tasks {
            let mut task_preview = TaskPreview {
                title: SlotString::empty(),
                due_time: SlotString::empty(),
                list_name: SlotString::empty(),
                status: preview.status,
            };
            task_preview.title.set(&preview.title);
            task_preview.due_time.set(&preview.due_time);
            task_preview.list_name.set(&preview.list_name);
            self.task_previews.push(task_preview);
        }

        self.reminder_previews.clear();
        for preview in selected.reminders {
            let mut reminder_preview = ReminderPreview {
                title: SlotString::empty(),
                reminder_time: SlotString::empty(),
                linked_task_title: SlotString::empty(),
            };
            reminder_preview.title.set(&preview.title);
            reminder_preview.reminder_time.set(&preview.reminder_time);
            reminder_preview
                .linked_task_title
                .set(&preview.linked_task_title);
            self.reminder_previews.push(reminder_preview);
        }

        self.tasks_loaded_for = date_slot;
        self.reminders_loaded_for = date_slot;
        self.last_preview_refresh_ms = monotonic_millis();
    }

    fn refresh_timezone_and_locale(&mut self) {
        let utc_secs = get_time_utc();
        let cfg = read_localtime();
        let zone_str = core::str::from_utf8(&cfg.id[..cfg.id_len]).unwrap_or("UTC");
        self.timezone_str.set(zone_str);

        if let Some(entry) = tz_by_id(zone_str) {
            let local = local_now(utc_secs, entry);
            self.today_year = local.year as i32;
            self.today_month = local.month as i32;
            self.today_day = local.day as i32;
            self.tz_offset_secs = local.utc_offset_secs;
            let abbr = core::str::from_utf8(&local.abbr).unwrap_or("");
            self.timezone_abbr.set(abbr.trim_end_matches('\0'));
        } else {
            let (y, m, d) = decompose_utc(utc_secs, None);
            self.today_year = y;
            self.today_month = m;
            self.today_day = d;
            self.tz_offset_secs = 0;
            self.timezone_abbr.set("UTC");
        }

        let locale_str = read_locale_time();
        self.locale_str.set(&locale_str);
    }
}

fn decompose_utc(utc_secs: u64, _offset_override: Option<i64>) -> (i32, i32, i32) {
    let mut days_rem = utc_secs / 86400;
    let mut y = 1970i32;
    loop {
        let days_in_y = if is_leap_year(y) { 366 } else { 365 };
        if days_rem < days_in_y as u64 {
            break;
        }
        days_rem -= days_in_y as u64;
        y += 1;
    }
    let mut m = 1;
    loop {
        let dim = days_in_month(y, m) as u64;
        if days_rem < dim {
            break;
        }
        days_rem -= dim;
        m += 1;
    }
    let d = days_rem as i32 + 1;
    (y, m, d)
}

fn read_locale_time() -> String {
    let data = match read_file_bytes("/etc/locale.conf") {
        Some(d) => d,
        None => return String::from("en_US.UTF-8"),
    };
    let locale_cfg = sunlight_locale::parse_locale_conf(&data);
    let lc_time = locale_cfg.lc_time();
    if lc_time.is_empty() {
        String::from("en_US.UTF-8")
    } else {
        String::from(lc_time)
    }
}

impl App for CalendarApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let main = self.main_rect();
        canvas.fill_rect(main, theme.bg);

        self.draw_header(canvas, theme);
        self.draw_toolbar(canvas, theme);
        self.draw_month_grid(canvas, theme);
        self.draw_sidebar(canvas, theme);
        self.draw_status(canvas, theme);
        self.draw_popup_menu(canvas, theme);
        self.draw_dialog(canvas, theme);
        self.draw_confirmation(canvas, theme);
    }

    fn on_ready(&mut self) -> bool {
        self.refresh_timezone_and_locale();
        if !self.data_loaded {
            self.load_calendar_data();
        }
        self.data_loaded = true;
        true
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Tick => {
                let now = monotonic_millis();
                if now.saturating_sub(self.last_preview_refresh_ms) >= 2_000 {
                    self.load_reminder_previews_for_selection(true);
                    true
                } else {
                    false
                }
            }
            Event::Click { x, y } => self.handle_click(x, y),
            Event::Key(ch) => self.handle_char(ch),
            Event::KeyPress {
                keycode,
                pressed,
                shift: _,
                ctrl,
                ..
            } => self.handle_key(keycode, pressed, ctrl),
            _ => false,
        }
    }
}

impl CalendarApp {
    fn draw_header(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.header_rect();
        canvas.fill_rect(rect, theme.panel);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);
        draw_text_vcenter(
            canvas,
            "Sunlight Calendar",
            PAD,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
        draw_text_vcenter(
            canvas,
            "v0.1",
            rect.right() - 40,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );
    }

    fn draw_toolbar(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.toolbar_rect();
        canvas.fill_rect(rect, theme.panel);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);

        let btns = self.toolbar_buttons();
        let icons = &self.icons;

        let btn_states = [
            self.toolbar_hover == Some(0),
            self.toolbar_hover == Some(1),
            self.toolbar_hover == Some(2),
            self.toolbar_hover == Some(3),
            self.toolbar_hover == Some(4),
        ];

        for i in 0..5 {
            let bg = if btn_states[i] {
                theme.panel_alt
            } else {
                theme.panel
            };
            canvas.fill_rounded_rect(btns[i], 6, bg);
            canvas.stroke_rounded_rect(btns[i], 6, 1, theme.border);
        }

        if let Some(icon) = &icons.prev {
            canvas.draw_tga_icon_tinted(
                icon,
                Rect::new(btns[0].x + 10, btns[0].y + 10, 16, 16),
                theme.icon_foreground,
            );
        }

        draw_text_vcenter(
            canvas,
            "Today",
            btns[1].x + 4,
            btns[1].y,
            btns[1].h,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );

        if let Some(icon) = &icons.next {
            canvas.draw_tga_icon_tinted(
                icon,
                Rect::new(btns[2].x + 10, btns[2].y + 10, 16, 16),
                theme.icon_foreground,
            );
        }

        if let Some(icon) = &icons.add {
            canvas.draw_tga_icon_tinted(
                icon,
                Rect::new(btns[3].x + 10, btns[3].y + 10, 16, 16),
                theme.icon_foreground,
            );
        }

        if let Some(icon) = &icons.menu {
            canvas.draw_tga_icon_tinted(
                icon,
                Rect::new(btns[4].x + 10, btns[4].y + 10, 16, 16),
                theme.icon_foreground,
            );
        }

        let month_name_str = self.month_name(self.view_month, true);
        let mut title = String::from(month_name_str);
        title.push(' ');
        push_i32_into_string(&mut title, self.view_year);
        let title_x =
            rect.x + (rect.w as i32 / 2) - (measure_text(&title, FontRole::UiMedium).w as i32 / 2);
        draw_text_vcenter(
            canvas,
            &title,
            title_x,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );
    }

    fn draw_month_grid(&self, canvas: &mut Canvas, theme: &Theme) {
        let body = self.calendar_body_rect();
        canvas.fill_rect(body, theme.bg);

        let month_rect = self.calendar_month_rect();
        canvas.fill_rounded_rect_with_border(month_rect, 10, theme.panel, theme.border, 1);
        let month_inner = month_rect.inset(12);
        let layout = MonthGridLayout::new(month_inner);
        let days = month_grid_days(self.view_year, self.view_month);

        for col in 0..7 {
            let iso_wd = if col == 6 { 7u8 } else { (col + 1) as u8 };
            let wd_name = self.weekday_short(iso_wd);
            let cell_rect = Rect::new(
                month_inner.x + layout.pad_x + col * layout.total_cell_w,
                layout.header_y,
                layout.cell_w as u32,
                layout.header_h as u32,
            );
            draw_text_vcenter(
                canvas,
                wd_name,
                cell_rect.x + 4,
                cell_rect.y,
                cell_rect.h,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
        }

        for i in 0i32..42 {
            let row = i / 7;
            let col = i % 7;
            let day = days[i as usize];
            if day == 0 {
                continue;
            }

            let cell_rect = layout.cell_rect(month_inner, col, row);

            let is_today = day == self.today_day
                && self.view_month == self.today_month
                && self.view_year == self.today_year;
            let is_selected = day == self.sel_day
                && self.view_month == self.sel_month
                && self.view_year == self.sel_year;
            let has_events = self
                .events_for_date(self.view_year, self.view_month, day)
                .len()
                > 0;

            let state = if is_selected && is_today {
                CalendarCellState::SelectedToday
            } else if is_selected {
                CalendarCellState::Selected
            } else if is_today {
                CalendarCellState::Today
            } else {
                CalendarCellState::Normal
            };
            let cell_style = CalendarCellStyle::from_theme(theme, state, has_events);
            if let Some(fill) = cell_style.fill {
                canvas.fill_rounded_rect(cell_rect, 6, fill);
            }
            if state != CalendarCellState::Normal {
                canvas.stroke_rounded_rect(cell_rect, 6, 1, cell_style.border);
            }

            let mut day_str = String::new();
            push_i32_into_string(&mut day_str, day);
            draw_text(
                canvas,
                &day_str,
                cell_rect.x + 6,
                cell_rect.y + 6,
                &TextStyle::new(FontRole::UiSmall, cell_style.text),
            );

            if has_events {
                let dot_x = cell_rect.x + cell_rect.w as i32 - 10;
                let dot_y = cell_rect.y + cell_rect.h as i32 - 8;
                canvas.fill_rounded_rect(Rect::new(dot_x, dot_y, 6, 6), 3, cell_style.marker);
            }
        }

        let preview_rect = self.calendar_preview_rect();
        if preview_rect.w > 0 && preview_rect.h > 0 {
            let preview_col_blocks = [5u32, 5u32];
            let mut preview_cols = GridRow::new(preview_rect)
                .with_gap(CALENDAR_SECTION_GAP as u32)
                .layout(&preview_col_blocks);
            let tasks_rect = preview_cols.next().unwrap_or(preview_rect);
            let reminders_rect = preview_cols.next().unwrap_or(preview_rect);
            self.draw_tasks_preview(canvas, theme, tasks_rect);
            self.draw_reminders_preview(canvas, theme, reminders_rect);
        }
    }

    fn draw_tasks_preview(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "Tasks");
        panel.draw(canvas, theme);
        let content = panel.content_rect().inset(6);
        if content.w == 0 || content.h == 0 {
            return;
        }
        if self.task_previews.is_empty() {
            draw_text_vcenter(
                canvas,
                "No tasks for this day",
                content.x,
                content.y,
                content.h,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
            return;
        }

        let item_h: i32 = 18;
        let mut y = content.y + 2;
        for tp in &self.task_previews {
            if y + item_h > content.bottom() {
                break;
            }
            // status marker
            let marker = if tp.status == TaskStatus::Done {
                "[x]"
            } else {
                "[ ]"
            };
            let mut line = String::new();
            line.push_str(marker);
            line.push(' ');
            let t = if tp.title.len > 22 {
                &tp.title.as_str()[..22]
            } else {
                tp.title.as_str()
            };
            line.push_str(t);
            if tp.due_time.len > 0 {
                line.push(' ');
                line.push_str(tp.due_time.as_str());
            }
            if tp.list_name.len > 0 {
                line.push_str(" (");
                let l = if tp.list_name.as_str().len() > 8 {
                    &tp.list_name.as_str()[..8]
                } else {
                    tp.list_name.as_str()
                };
                line.push_str(l);
                line.push(')');
            }
            let style = if tp.status == TaskStatus::Done {
                TextStyle::new(FontRole::UiSmall, theme.text_muted)
            } else {
                TextStyle::new(FontRole::UiSmall, theme.text)
            };
            draw_text(canvas, &line, content.x + 2, y, &style);
            y += item_h;
        }
    }

    fn draw_reminders_preview(&self, canvas: &mut Canvas, theme: &Theme, rect: Rect) {
        let panel = Panel::with_title(rect, "Reminders");
        panel.draw(canvas, theme);
        let content = panel.content_rect().inset(6);
        if content.w == 0 || content.h == 0 {
            return;
        }
        if self.reminder_previews.is_empty() {
            draw_text_vcenter(
                canvas,
                "No reminders",
                content.x,
                content.y,
                content.h,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );
            return;
        }

        let item_h: i32 = 18;
        let mut y = content.y + 2;
        for rp in &self.reminder_previews {
            if y + item_h > content.bottom() {
                break;
            }
            let mut line = String::new();
            let t = if rp.title.len > 20 {
                &rp.title.as_str()[..20]
            } else {
                rp.title.as_str()
            };
            line.push_str(t);
            if rp.reminder_time.len > 0 {
                line.push(' ');
                line.push_str(rp.reminder_time.as_str());
            }
            if rp.linked_task_title.len > 0 && rp.linked_task_title.as_str() != rp.title.as_str() {
                // rarely different but per spec
                line.push_str(" <- ");
                let lt = if rp.linked_task_title.len > 12 {
                    &rp.linked_task_title.as_str()[..12]
                } else {
                    rp.linked_task_title.as_str()
                };
                line.push_str(lt);
            }
            draw_text(
                canvas,
                &line,
                content.x + 2,
                y,
                &TextStyle::new(FontRole::UiSmall, theme.text),
            );
            y += item_h;
        }
    }

    fn draw_sidebar(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.sidebar_rect();
        canvas.fill_rect(rect, theme.panel);
        canvas.vline(rect.x, rect.y, rect.h, theme.border);

        let lh = line_height(FontRole::UiRegular);

        let mut date_str = String::from(self.month_name(self.sel_month, true));
        date_str.push(' ');
        push_i32_into_string(&mut date_str, self.sel_day);
        date_str.push_str(", ");
        push_i32_into_string(&mut date_str, self.sel_year);

        let wd = iso_weekday(self.sel_year, self.sel_month, self.sel_day);
        let wd_name = weekday_name(wd, true, self.locale_str.as_str());
        let mut header = String::from(wd_name);
        header.push_str("  ");
        header.push_str(&date_str);

        draw_text_vcenter(
            canvas,
            &header,
            rect.x + PAD,
            rect.y + PAD,
            lh + 4,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        canvas.hbar(
            rect.x + PAD,
            rect.y + PAD + (lh + 4) as i32 + 4,
            rect.w - (PAD as u32) * 2,
            1,
            theme.border,
        );

        let events_header_y = rect.y + PAD + (lh + 4) as i32 + 10;
        let mut events_label = String::from("Events (");
        let sel_events = self.events_for_date(self.sel_year, self.sel_month, self.sel_day);
        push_i32_into_string(&mut events_label, sel_events.len() as i32);
        events_label.push(')');

        draw_text(
            canvas,
            &events_label,
            rect.x + PAD,
            events_header_y,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );

        let events_y = events_header_y + lh as i32 + 6;
        let event_item_h = 36i32;
        let (tasks_btn, add_btn, events_limit_y) = self.sidebar_actions_rects(rect);
        let max_visible = ((events_limit_y - events_y - 8) / event_item_h).max(1) as usize;

        if sel_events.is_empty() {
            let empty_style =
                EmptyStateStyle::new(Rect::new(rect.x + PAD, events_y, rect.w - 16, 24), theme);
            draw_text(
                canvas,
                "No events for this day.",
                empty_style.rect.x,
                empty_style.rect.y + 8,
                &TextStyle::new(FontRole::UiSmall, empty_style.text),
            );
        } else {
            for (idx, event) in sel_events.iter().enumerate().take(max_visible) {
                let item_rect = Rect::new(
                    rect.x + 4,
                    events_y + idx as i32 * event_item_h,
                    rect.w - 8,
                    event_item_h as u32,
                );

                if self.selected_event_idx == Some(idx) {
                    canvas.fill_rounded_rect(item_rect, 5, theme.panel_alt);
                }

                if let Some(icon) = &self.icons.event {
                    canvas.draw_tga_icon_tinted(
                        icon,
                        Rect::new(item_rect.x + 4, item_rect.y + 10, 16, 16),
                        theme.accent,
                    );
                }

                let title_text = if event.title.len > 20 {
                    event.title.as_str().split_at(20).0
                } else {
                    event.title.as_str()
                };
                let time_str = if event.all_day {
                    "All day"
                } else if event.start_time.len > 0 {
                    event.start_time.as_str()
                } else {
                    ""
                };

                draw_text(
                    canvas,
                    title_text,
                    item_rect.x + 24,
                    item_rect.y + 4,
                    &TextStyle::new(FontRole::UiRegular, theme.text),
                );

                if !time_str.is_empty() {
                    draw_text(
                        canvas,
                        time_str,
                        item_rect.x + 24,
                        item_rect.y + lh as i32 + 6,
                        &TextStyle::new(FontRole::UiSmall, theme.text_muted),
                    );
                }
            }
        }

        let launch_btn = Button::secondary(tasks_btn, "Tasks & Reminders");
        launch_btn.draw(canvas, theme);
        canvas.fill_rounded_rect(add_btn, 6, theme.accent);
        draw_text_vcenter(
            canvas,
            "+ Add Event",
            add_btn.x + 8,
            add_btn.y,
            add_btn.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_on_accent),
        );
    }

    fn draw_status(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.status_rect();
        canvas.fill_rect(rect, theme.panel_alt);
        canvas.hbar(rect.x, rect.y, rect.w, 1, theme.border);

        let wd = iso_weekday(self.sel_year, self.sel_month, self.sel_day);
        let wd_name = weekday_name(wd, false, self.locale_str.as_str());
        let mut sel_str = String::from("Selected: ");
        sel_str.push_str(wd_name);
        sel_str.push(' ');
        push_i32_into_string(&mut sel_str, self.sel_day);
        sel_str.push(' ');
        sel_str.push_str(self.month_name(self.sel_month, false));
        sel_str.push(' ');
        push_i32_into_string(&mut sel_str, self.sel_year);

        let mut center_str = String::new();
        if self.status_msg.len > 0 {
            center_str.push_str(self.status_msg.as_str());
        } else if self.store.mode() == StoreMode::MemoryFallback {
            center_str.push_str("Memory-only mode");
        }

        let mut right_str = String::new();
        let tz = self.timezone_abbr.as_str();
        if !tz.is_empty() {
            right_str.push_str(tz);
            right_str.push_str(" | ");
        }
        right_str.push_str(self.locale_str.as_str());
        let sel_count = self
            .events_for_date(self.sel_year, self.sel_month, self.sel_day)
            .len();
        right_str.push_str(" | ");
        push_i32_into_string(&mut right_str, sel_count as i32);
        right_str.push_str(" events");

        draw_text_vcenter(
            canvas,
            &sel_str,
            rect.x + PAD,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );

        if !center_str.is_empty() {
            let cw = measure_text(&center_str, FontRole::UiSmall).w;
            let kind = if center_str.contains("error")
                || center_str.contains("failed")
                || center_str.contains("unavailable")
            {
                StatusTextKind::Error
            } else {
                StatusTextKind::Muted
            };
            draw_text_vcenter(
                canvas,
                &center_str,
                rect.x + (rect.w as i32 - cw as i32) / 2,
                rect.y,
                rect.h,
                &TextStyle::new(FontRole::UiSmall, status_text_color(theme, kind)),
            );
        }

        let tw = measure_text(&right_str, FontRole::UiSmall).w;
        draw_text_vcenter(
            canvas,
            &right_str,
            rect.right() - tw as i32 - PAD,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );
    }

    fn draw_popup_menu(&self, canvas: &mut Canvas, theme: &Theme) {
        let Some(menu) = self.menu else {
            return;
        };
        canvas.fill_rounded_rect(menu.rect, 8, theme.panel);
        canvas.stroke_rounded_rect(menu.rect, 8, 1, theme.border);
        for item in &menu.items[..menu.count] {
            canvas.fill_rect(item.rect, theme.panel);
            draw_text_vcenter(
                canvas,
                item.label,
                item.rect.x + 8,
                item.rect.y,
                item.rect.h,
                &TextStyle::new(FontRole::UiRegular, theme.text),
            );
        }
    }

    fn draw_dialog(&self, canvas: &mut Canvas, theme: &Theme) {
        if !self.dialog.visible {
            return;
        }
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg.darken(70));
        let panel = self.dialog_rect();
        canvas.fill_rounded_rect_with_border(panel, 8, theme.panel, theme.border, 1);

        let title = if self.dialog.editing_id.is_some() {
            "Edit Event"
        } else {
            "New Event"
        };
        draw_text(
            canvas,
            title,
            panel.x + DIALOG_PAD,
            panel.y + 10,
            &TextStyle::new(FontRole::UiMedium, theme.text),
        );

        let fields = self.dialog_field_rects();
        let labels = ["Title:", "Date:", "Start:", "End:", "Notes:"];
        let focus_map = [
            DialogField::Title,
            DialogField::Date,
            DialogField::StartTime,
            DialogField::EndTime,
            DialogField::Notes,
        ];

        for i in 0..5 {
            let label_rect = Rect::new(
                if i == 3 {
                    fields[2].right() + 12
                } else {
                    panel.x + DIALOG_PAD
                },
                fields[i].y,
                if i == 3 { 28 } else { 70 },
                fields[i].h,
            );
            draw_text_vcenter(
                canvas,
                labels[i],
                label_rect.x,
                label_rect.y,
                label_rect.h,
                &TextStyle::new(FontRole::UiSmall, theme.text_muted),
            );

            let is_focused = self.dialog.focus == focus_map[i];
            let field_style = form_field_style(theme, is_focused);

            if i < 4 {
                canvas.fill_rounded_rect_with_border(
                    fields[i],
                    4,
                    field_style.fill,
                    field_style.border,
                    1,
                );
            } else {
                canvas.fill_rounded_rect_with_border(
                    fields[i],
                    4,
                    field_style.fill,
                    field_style.border,
                    1,
                );
            }

            let text = match i {
                0 => self.dialog.title.as_str(),
                1 => self.dialog.date.as_str(),
                2 => self.dialog.start_time.as_str(),
                3 => self.dialog.end_time.as_str(),
                4 => self.dialog.notes.as_str(),
                _ => "",
            };

            if i < 4 {
                draw_text_vcenter(
                    canvas,
                    text,
                    fields[i].x + 4,
                    fields[i].y,
                    fields[i].h,
                    &TextStyle::new(FontRole::UiSmall, field_style.text),
                );
            } else {
                draw_text(
                    canvas,
                    text,
                    fields[i].x + 4,
                    fields[i].y + 4,
                    &TextStyle::new(FontRole::UiSmall, field_style.text),
                );
            }

            if is_focused && i < 4 {
                let caret_x = fields[i].x + 4 + measure_text(text, FontRole::UiSmall).w as i32;
                if caret_x < fields[i].right() {
                    canvas.vline(caret_x, fields[i].y + 4, fields[i].h - 8, theme.accent);
                }
            }
        }

        let all_day_rect = Rect::new(
            fields[1].x + fields[1].w as i32 + 12,
            fields[1].y,
            80,
            fields[1].h,
        );
        let check_size = 14;
        let check_rect = Rect::new(
            all_day_rect.x,
            all_day_rect.y + (all_day_rect.h as i32 - check_size as i32) / 2,
            check_size,
            check_size,
        );
        canvas.fill_rounded_rect_with_border(check_rect, 3, theme.bg, theme.border, 1);
        if self.dialog.all_day {
            canvas.fill_rounded_rect(check_rect.inset(2), 2, theme.accent);
        }
        draw_text_vcenter(
            canvas,
            "All day",
            check_rect.right() + 6,
            all_day_rect.y,
            all_day_rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );

        let btns = self.dialog_button_rects();
        let btn_count = if self.dialog.editing_id.is_some() {
            3usize
        } else {
            2usize
        };
        for i in 0..btn_count {
            let is_delete = self.dialog.editing_id.is_some() && i == 1;
            let btn_label = if i == 0 {
                "Save"
            } else if is_delete {
                "Delete"
            } else {
                "Cancel"
            };
            let bg = if self.dialog_hover_btn == Some(i) {
                if is_delete {
                    theme.danger
                } else if i == 0 {
                    theme.accent
                } else {
                    theme.panel_alt
                }
            } else {
                if is_delete {
                    theme.danger.darken(30)
                } else if i == 0 {
                    theme.accent.darken(20)
                } else {
                    theme.panel
                }
            };
            canvas.fill_rounded_rect_with_border(btns[i], 5, bg, theme.border, 1);
            draw_text_vcenter(
                canvas,
                btn_label,
                btns[i].x
                    + ((btns[i].w as i32 - measure_text(btn_label, FontRole::UiSmall).w as i32)
                        / 2),
                btns[i].y,
                btns[i].h,
                &TextStyle::new(
                    FontRole::UiSmall,
                    if i == 0 {
                        theme.text_on_accent
                    } else {
                        theme.text
                    },
                ),
            );
        }

        if self.dialog.error_msg.len > 0 {
            draw_text(
                canvas,
                self.dialog.error_msg.as_str(),
                panel.x + DIALOG_PAD,
                panel.bottom() - DIALOG_PAD - 30,
                &TextStyle::new(FontRole::UiSmall, theme.danger_text),
            );
        }
    }

    fn draw_confirmation(&self, canvas: &mut Canvas, theme: &Theme) {
        if !self.confirm.visible {
            return;
        }
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg.darken(70));
        let w = 320u32;
        let h = 100u32;
        let panel = Rect::new(((WIN_W - w) / 2) as i32, ((WIN_H - h) / 2) as i32, w, h);
        canvas.fill_rounded_rect_with_border(panel, 8, theme.panel, theme.border, 1);
        draw_text_vcenter(
            canvas,
            self.confirm.message.as_str(),
            panel.x + DIALOG_PAD,
            panel.y + DIALOG_PAD,
            28,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        let btn_y = panel.bottom() - DIALOG_PAD - DIALOG_BTN_H as i32;
        let ok_btn = Rect::new(
            panel.x + (panel.w as i32 / 2) - DIALOG_BTN_W as i32 - 5,
            btn_y,
            DIALOG_BTN_W,
            DIALOG_BTN_H,
        );
        let cancel_btn = Rect::new(
            panel.x + (panel.w as i32 / 2) + 5,
            btn_y,
            DIALOG_BTN_W,
            DIALOG_BTN_H,
        );
        canvas.fill_rounded_rect_with_border(ok_btn, 5, theme.danger, theme.border, 1);
        draw_text_vcenter(
            canvas,
            "Delete",
            ok_btn.x + 8,
            ok_btn.y,
            ok_btn.h,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );
        canvas.fill_rounded_rect_with_border(cancel_btn, 5, theme.panel, theme.border, 1);
        draw_text_vcenter(
            canvas,
            "Cancel",
            cancel_btn.x + 8,
            cancel_btn.y,
            cancel_btn.h,
            &TextStyle::new(FontRole::UiSmall, theme.text),
        );
    }

    fn handle_click(&mut self, x: i32, y: i32) -> bool {
        if self.confirm.visible {
            let w = 320u32;
            let h = 100u32;
            let panel = Rect::new(((WIN_W - w) / 2) as i32, ((WIN_H - h) / 2) as i32, w, h);
            let btn_y = panel.bottom() - DIALOG_PAD - DIALOG_BTN_H as i32;
            let ok_btn = Rect::new(
                panel.x + (panel.w as i32 / 2) - DIALOG_BTN_W as i32 - 5,
                btn_y,
                DIALOG_BTN_W,
                DIALOG_BTN_H,
            );
            let cancel_btn = Rect::new(
                panel.x + (panel.w as i32 / 2) + 5,
                btn_y,
                DIALOG_BTN_W,
                DIALOG_BTN_H,
            );

            if ok_btn.contains(Point::new(x, y)) {
                match self.confirm.kind {
                    ConfirmKind::DeleteEvent { event_id } => {
                        self.delete_event(event_id);
                    }
                    _ => {}
                }
                return true;
            }
            if cancel_btn.contains(Point::new(x, y)) {
                self.confirm.visible = false;
                return true;
            }
            return true;
        }

        if self.dialog.visible {
            if let Some(btn_idx) = self.dialog_button_hit(x, y) {
                let is_edit = self.dialog.editing_id.is_some();
                match (is_edit, btn_idx) {
                    (false, 0) => {
                        if self.dialog.validate() {
                            self.add_event_from_dialog();
                        } else {
                            self.dialog
                                .error_msg
                                .set(self.dialog.validation_error().unwrap_or("Invalid input"));
                        }
                    }
                    (true, 0) => {
                        if self.dialog.validate() {
                            self.update_event_from_dialog();
                        } else {
                            self.dialog
                                .error_msg
                                .set(self.dialog.validation_error().unwrap_or("Invalid input"));
                        }
                    }
                    (true, 1) => {
                        if let Some(edit_id) = self.dialog.editing_id {
                            self.confirm.show_delete(edit_id);
                            self.dialog.visible = false;
                        }
                    }
                    (_, 2) | (false, 1) => {
                        self.dialog.reset();
                    }
                    _ => {}
                }
                return true;
            }

            if let Some(field) = self.focus_field_at_point(x, y) {
                self.dialog.focus = field;
                self.dialog.cursor = match field {
                    DialogField::Title => self.dialog.title.len,
                    DialogField::Date => self.dialog.date.len,
                    DialogField::StartTime => self.dialog.start_time.len,
                    DialogField::EndTime => self.dialog.end_time.len,
                    DialogField::Notes => self.dialog.notes.len,
                };
                return true;
            }

            let all_day_click = {
                let fields = self.dialog_field_rects();
                let all_day_rect = Rect::new(
                    fields[1].x + fields[1].w as i32 + 12,
                    fields[1].y,
                    80,
                    fields[1].h,
                );
                all_day_rect.contains(Point::new(x, y))
            };
            if all_day_click {
                self.dialog.all_day = !self.dialog.all_day;
                return true;
            }

            return true;
        }

        if let Some(menu) = self.menu {
            let p = Point::new(x, y);
            if menu.rect.contains(p) {
                if let Some(item) = menu.items[..menu.count]
                    .iter()
                    .find(|item| item.rect.contains(p))
                {
                    return self.handle_menu_action(item.action);
                }
                return true;
            }
            self.close_menu();
            return true;
        }

        if let Some(idx) = self.toolbar_hit(x, y) {
            match idx {
                0 => {
                    self.view_month -= 1;
                    if self.view_month < 1 {
                        self.view_month = 12;
                        self.view_year -= 1;
                    }
                    self.save_selection_settings();
                }
                1 => {
                    self.view_year = self.today_year;
                    self.view_month = self.today_month;
                    self.sel_year = self.today_year;
                    self.sel_month = self.today_month;
                    self.sel_day = self.today_day;
                    self.selected_event_idx = None;
                    self.save_selection_settings();
                    self.load_reminder_previews_for_selection(false);
                }
                2 => {
                    self.view_month += 1;
                    if self.view_month > 12 {
                        self.view_month = 1;
                        self.view_year += 1;
                    }
                    self.save_selection_settings();
                }
                3 => {
                    self.open_new_event_dialog();
                }
                4 => {
                    let btn = self.toolbar_buttons()[4];
                    self.open_menu(btn.x - 100, btn.bottom() + 4);
                }
                _ => {}
            }
            return true;
        }

        let sidebar = self.sidebar_rect();
        if sidebar.contains(Point::new(x, y)) {
            let (tasks_btn, add_btn, events_limit_y) = self.sidebar_actions_rects(sidebar);
            if tasks_btn.contains(Point::new(x, y)) {
                return self.launch_tasks_and_reminders();
            }
            if add_btn.contains(Point::new(x, y)) {
                self.open_new_event_dialog();
                return true;
            }

            let events_header_y =
                sidebar.y + PAD + (line_height(FontRole::UiRegular) + 4) as i32 + 10;
            let lh = line_height(FontRole::UiRegular);
            let events_y = events_header_y + lh as i32 + 6;
            let event_item_h = 36i32;
            let max_visible = ((events_limit_y - events_y - 8) / event_item_h).max(1) as usize;
            let sel_events = self.events_for_date(self.sel_year, self.sel_month, self.sel_day);
            let event_data: Vec<CalendarEvent> = sel_events.iter().map(|e| **e).collect();
            drop(sel_events);

            for (idx, event) in event_data.iter().enumerate().take(max_visible) {
                let item_rect = Rect::new(
                    sidebar.x + 4,
                    events_y + idx as i32 * event_item_h,
                    sidebar.w - 8,
                    event_item_h as u32,
                );
                if item_rect.contains(Point::new(x, y)) {
                    self.selected_event_idx = Some(idx);
                    self.dialog.start_edit(event);
                    return true;
                }
            }
            return true;
        }

        if let Some(day) = self.day_at_point(x, y) {
            let old_sel = (self.sel_year, self.sel_month, self.sel_day);
            self.sel_year = self.view_year;
            self.sel_month = self.view_month;
            self.sel_day = day;
            self.selected_event_idx = None;
            if (self.sel_year, self.sel_month, self.sel_day) != old_sel {
                self.save_selection_settings();
                self.load_reminder_previews_for_selection(false);
                return true;
            }
        }

        // Optional simple behavior: click in Tasks/Reminders preview area launches reminders app (no deep link)
        let preview_rect = self.calendar_preview_rect();
        if preview_rect.contains(Point::new(x, y)) && preview_rect.h > 8 {
            return self.launch_tasks_and_reminders();
        }

        false
    }

    fn handle_menu_action(&mut self, action: MenuAction) -> bool {
        self.close_menu();
        match action {
            MenuAction::NewEvent => {
                self.open_new_event_dialog();
                true
            }
            MenuAction::Today => {
                self.view_year = self.today_year;
                self.view_month = self.today_month;
                self.sel_year = self.today_year;
                self.sel_month = self.today_month;
                self.sel_day = self.today_day;
                self.selected_event_idx = None;
                self.save_selection_settings();
                self.load_reminder_previews_for_selection(false);
                true
            }
            MenuAction::Refresh => {
                self.refresh_timezone_and_locale();
                self.load_calendar_data();
                true
            }
            MenuAction::About => true,
        }
    }

    fn open_new_event_dialog(&mut self) {
        let date_str = CalendarEvent::format_date(self.sel_year, self.sel_month, self.sel_day);
        self.dialog.start_new(&date_str);
    }

    fn launch_tasks_and_reminders(&mut self) -> bool {
        match libc::sun_exec::launch(libc::sun_exec::LaunchRequest {
            trace: libc::sun_exec::next_cli_trace(LaunchSource::Shortcut),
            source: LaunchSource::Shortcut,
            command: b"sunlight-reminders",
            args: &[],
            require_display: true,
        }) {
            Ok(_) => {
                self.status_msg.set("Launching Sunlight Reminders");
            }
            Err(_) => {
                debug_log("[CALENDAR] sun-exec launch of sunlight-reminders failed\n");
                self.status_msg
                    .set("Sunlight Reminders is not available yet");
            }
        }
        true
    }

    fn dialog_push_char(&mut self, ch: char) {
        let max = match self.dialog.focus {
            DialogField::Title => TITLE_LEN,
            DialogField::Date => DATE_LEN,
            DialogField::StartTime => TIME_LEN,
            DialogField::EndTime => TIME_LEN,
            DialogField::Notes => NOTES_LEN,
        };
        match self.dialog.focus {
            DialogField::Title => {
                if self.dialog.title.len < max {
                    self.dialog.title.push(ch);
                }
            }
            DialogField::Date => {
                if self.dialog.date.len < max {
                    self.dialog.date.push(ch);
                }
            }
            DialogField::StartTime => {
                if self.dialog.start_time.len < max {
                    self.dialog.start_time.push(ch);
                }
            }
            DialogField::EndTime => {
                if self.dialog.end_time.len < max {
                    self.dialog.end_time.push(ch);
                }
            }
            DialogField::Notes => {
                if self.dialog.notes.len < max {
                    self.dialog.notes.push(ch);
                }
            }
        }
    }

    fn dialog_pop_char(&mut self) {
        match self.dialog.focus {
            DialogField::Title => self.dialog.title.pop(),
            DialogField::Date => self.dialog.date.pop(),
            DialogField::StartTime => self.dialog.start_time.pop(),
            DialogField::EndTime => self.dialog.end_time.pop(),
            DialogField::Notes => self.dialog.notes.pop(),
        }
    }

    fn dialog_clear_field(&mut self) {
        match self.dialog.focus {
            DialogField::Title => self.dialog.title.clear(),
            DialogField::Date => self.dialog.date.clear(),
            DialogField::StartTime => self.dialog.start_time.clear(),
            DialogField::EndTime => self.dialog.end_time.clear(),
            DialogField::Notes => self.dialog.notes.clear(),
        }
    }

    fn handle_char(&mut self, ch: char) -> bool {
        if !self.dialog.visible {
            return false;
        }
        if ch.is_control() {
            return false;
        }
        self.dialog_push_char(ch);
        true
    }

    fn handle_key(&mut self, keycode: u8, pressed: bool, ctrl: bool) -> bool {
        if !pressed {
            return false;
        }

        if self.confirm.visible {
            if keycode == KEY_ESC || keycode == KEY_ENTER {
                if keycode == KEY_ENTER {
                    match self.confirm.kind {
                        ConfirmKind::DeleteEvent { event_id } => {
                            self.delete_event(event_id);
                        }
                        _ => {}
                    }
                } else {
                    self.confirm.visible = false;
                }
                return true;
            }
            return false;
        }

        if self.dialog.visible {
            match keycode {
                KEY_ESC => {
                    if ctrl {
                        self.dialog.reset();
                    } else {
                        self.dialog.reset();
                    }
                    return true;
                }
                KEY_ENTER => {
                    if ctrl {
                        if self.dialog.validate() {
                            if self.dialog.editing_id.is_some() {
                                self.update_event_from_dialog();
                            } else {
                                self.add_event_from_dialog();
                            }
                        } else {
                            self.dialog
                                .error_msg
                                .set(self.dialog.validation_error().unwrap_or("Invalid input"));
                        }
                        return true;
                    }
                    return true;
                }
                KEY_TAB => {
                    let fields = [
                        DialogField::Title,
                        DialogField::Date,
                        DialogField::StartTime,
                        DialogField::EndTime,
                        DialogField::Notes,
                    ];
                    let cur = fields
                        .iter()
                        .position(|f| *f == self.dialog.focus)
                        .unwrap_or(0);
                    self.dialog.focus = fields[(cur + 1) % fields.len()];
                    self.dialog.cursor = 0;
                    return true;
                }
                KEY_BACKSPACE => {
                    self.dialog_pop_char();
                    return true;
                }
                KEY_DELETE => {
                    self.dialog_clear_field();
                    return true;
                }
                _ => {}
            }
            return true;
        }

        match keycode {
            KEY_ESC => {
                if self.menu.is_some() {
                    self.close_menu();
                    return true;
                }
            }
            _ => {}
        }

        false
    }
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);

    let mut app = CalendarApp::new();

    let mut window = match Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Calendar",
        decoration: WindowDecoration::Normal,
    }) {
        Some(w) => w,
        None => {
            debug_log("[CALENDAR] failed to connect window\n");
            loop {
                process_yield();
            }
        }
    };

    window.run(&mut app);
    ProcessExit::exit(0);
}
