#![no_std]
#![no_main]

extern crate alloc;

use alloc::{string::String, vec::Vec};
use core::alloc::GlobalAlloc;

use sun_font::{draw_text, draw_text_vcenter, measure_text, FontRole, TextStyle, VecFont};
use sunlight_ipc::{
    debug_log, get_time_utc, ipc_call_timeout, monotonic_millis, nameserver_lookup_timeout,
    process_yield, shm_alloc, shm_free, shm_map, CapabilityToken, IpcMsg, ProcessExit, SHM_PAGE,
};
use sunlight_reminders::{
    add_id_to_index_list, by_date_list_key, date_index_key, decode_list, decode_task,
    encode_id_list, encode_list, encode_task, list_key, normalize_date_str, parse_id_list,
    reminder_date_index_key, reminder_date_list_key, remove_id_from_index_list, settings_key,
    task_key, valid_date_str, valid_time_str, Task, TaskList, TaskStatus, TinyString, DATE_LEN,
    DEFAULT_LISTS, INDEX_ALL_KEY, INDEX_BY_DATE_PREFIX, INDEX_REMINDER_DATE_PREFIX, NOTES_LEN,
    TIME_LEN, TITLE_LEN,
};
use sunlight_tz::local_now_best_effort;
use sunlight_ui::widgets::{
    BadgeKind, Button, ButtonState, Checkbox, Label, Panel, SidebarGroupHeader, SidebarItem,
    SidebarState, StatusBadge, TextInput,
};
use sunlight_ui::{
    App, Canvas, Event, GridRow, HBox, Point, Rect, Theme, VBox, Window, WindowConfig,
    WindowDecoration,
};

static F_UI: VecFont = VecFont(FontRole::UiRegular);
static F_SMALL: VecFont = VecFont(FontRole::UiSmall);

const WIN_W: u32 = 980;
const WIN_H: u32 = 660;
const HEADER_H: u32 = 34;
const FOOTER_H: u32 = 40;
const BODY_GAP: i32 = 10;
const PAD: i32 = 8;
const KV_LOOKUP_TIMEOUT_MS: u64 = 250;
const KV_TIMEOUT_MS: u64 = 250;
const KV_REPLY: u64 = 0x4BFF;
const KV_ERROR: u64 = 0x4BEE;
const KV_VALUE: u64 = 0x4B05;
const KV_PUT_SHM2: u64 = 0x4B08;
const KV_GET_SHM2: u64 = 0x4B09;
const KV_DELETE_SHM2: u64 = 0x4B0A;
const KEY_ESC: u8 = 0x01;
const KEY_ENTER: u8 = 0x1C;

static mut KV_CAP_CACHE: CapabilityToken = CapabilityToken::INVALID;

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

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static ALLOC: BumpAllocator = BumpAllocator;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[REMINDERS] panic\n");
    loop {
        process_yield();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarView {
    Inbox,
    Work,
    Personal,
    Today,
    Upcoming,
    Completed,
}

impl SidebarView {
    fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Work => "work",
            Self::Personal => "personal",
            Self::Today => "today",
            Self::Upcoming => "upcoming",
            Self::Completed => "completed",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Inbox => "Inbox",
            Self::Work => "Work",
            Self::Personal => "Personal",
            Self::Today => "Today",
            Self::Upcoming => "Upcoming",
            Self::Completed => "Completed",
        }
    }

    fn from_str(text: &str) -> Option<Self> {
        match text {
            "inbox" => Some(Self::Inbox),
            "work" => Some(Self::Work),
            "personal" => Some(Self::Personal),
            "today" => Some(Self::Today),
            "upcoming" => Some(Self::Upcoming),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }

    fn list_id(self) -> Option<&'static str> {
        match self {
            Self::Inbox => Some("inbox"),
            Self::Work => Some("work"),
            Self::Personal => Some("personal"),
            _ => None,
        }
    }
}

struct TaskEditor {
    visible: bool,
    editing_id: Option<u64>,
    selected_list_idx: usize,
    status: TaskStatus,
    error: TinyString<96>,
    title: TextInput<'static, TITLE_LEN>,
    notes: TextInput<'static, NOTES_LEN>,
    due_date: TextInput<'static, DATE_LEN>,
    due_time: TextInput<'static, TIME_LEN>,
    reminder_date: TextInput<'static, DATE_LEN>,
    reminder_time: TextInput<'static, TIME_LEN>,
}

impl TaskEditor {
    fn new() -> Self {
        let mut title = TextInput::new(Rect::default())
            .with_font(&F_UI)
            .with_placeholder("Task title");
        title.active = true;
        Self {
            visible: false,
            editing_id: None,
            selected_list_idx: 0,
            status: TaskStatus::Todo,
            error: TinyString::empty(),
            title,
            notes: TextInput::new(Rect::default())
                .with_font(&F_UI)
                .with_placeholder("Notes"),
            due_date: TextInput::new(Rect::default())
                .with_font(&F_UI)
                .with_placeholder("YYYY-MM-DD"),
            due_time: TextInput::new(Rect::default())
                .with_font(&F_UI)
                .with_placeholder("HH:MM"),
            reminder_date: TextInput::new(Rect::default())
                .with_font(&F_UI)
                .with_placeholder("YYYY-MM-DD"),
            reminder_time: TextInput::new(Rect::default())
                .with_font(&F_UI)
                .with_placeholder("HH:MM"),
        }
    }

    fn clear_focus(&mut self) {
        self.title.active = false;
        self.notes.active = false;
        self.due_date.active = false;
        self.due_time.active = false;
        self.reminder_date.active = false;
        self.reminder_time.active = false;
    }

    fn start_new(&mut self, list_idx: usize) {
        let now = local_now_best_effort(get_time_utc());
        let due_date = sunlight_reminders::format_date(
            now.year.into(),
            now.month as i32,
            now.day as i32,
        );
        let due_time = format_time_hhmm(now.hour, now.minute);
        self.visible = true;
        self.editing_id = None;
        self.selected_list_idx = list_idx.min(2);
        self.status = TaskStatus::Todo;
        self.error.clear();
        self.title.set_text("");
        self.notes.set_text("");
        self.due_date.set_text(&due_date);
        self.due_time.set_text(due_time.as_str());
        self.reminder_date.set_text("");
        self.reminder_time.set_text("");
        self.clear_focus();
        self.title.active = true;
    }

    fn load_task(&mut self, task: &Task, list_idx: usize) {
        self.visible = true;
        self.editing_id = Some(task.id);
        self.selected_list_idx = list_idx.min(2);
        self.status = task.status;
        self.error.clear();
        self.title.set_text(task.title.as_str());
        self.notes.set_text(task.notes.as_str());
        self.due_date.set_text(task.due_date.as_str());
        self.due_time.set_text(task.due_time.as_str());
        self.reminder_date.set_text(task.reminder_date.as_str());
        self.reminder_time.set_text(task.reminder_time.as_str());
        self.clear_focus();
        self.title.active = true;
    }

    fn hide(&mut self) {
        self.visible = false;
        self.editing_id = None;
        self.error.clear();
        self.clear_focus();
    }

    fn validate(&self) -> Option<&'static str> {
        if self.title.value().trim().is_empty() {
            return Some("Title is required");
        }
        if !self.due_date.value().is_empty() && !valid_date_str(self.due_date.value()) {
            return Some("Due date must be YYYY-MM-DD");
        }
        if !self.due_time.value().is_empty() && self.due_date.value().is_empty() {
            return Some("Due time requires a due date");
        }
        if !valid_time_str(self.due_time.value()) {
            return Some("Due time must be HH:MM");
        }
        if !self.reminder_date.value().is_empty() && !valid_date_str(self.reminder_date.value()) {
            return Some("Reminder date must be YYYY-MM-DD");
        }
        if !self.reminder_time.value().is_empty()
            && self.reminder_date.value().is_empty()
            && self.due_date.value().is_empty()
        {
            return Some("Reminder time requires a reminder date or due date");
        }
        if !valid_time_str(self.reminder_time.value()) {
            return Some("Reminder time must be HH:MM");
        }
        None
    }

    fn build_task(
        &self,
        id: u64,
        created_at: u64,
        updated_at: u64,
        lists: &[TaskList; 3],
    ) -> Option<Task> {
        self.validate()?;
        let list_idx = self.selected_list_idx.min(2);
        let list_id = lists[list_idx].id.as_str();
        let mut task = Task::blank(id, list_id)?;
        task.status = self.status;
        if !task.title.try_set(self.title.value()) {
            return None;
        }
        if !task.notes.try_set(self.notes.value()) {
            return None;
        }
        let due_date = if self.due_date.value().is_empty() {
            String::new()
        } else {
            normalize_date_str(self.due_date.value())?
        };
        if !task.due_date.try_set(&due_date) {
            return None;
        }
        if !task.due_time.try_set(self.due_time.value()) {
            return None;
        }
        let reminder_date = if self.reminder_date.value().is_empty() {
            String::new()
        } else {
            normalize_date_str(self.reminder_date.value())?
        };
        if !task.reminder_date.try_set(&reminder_date) {
            return None;
        }
        if !task.reminder_time.try_set(self.reminder_time.value()) {
            return None;
        }
        task.created_at = created_at;
        task.updated_at = updated_at;
        Some(task)
    }

    fn update_inputs(&mut self, event: Event) -> bool {
        let mut redraw = false;
        redraw |= self.title.update(event);
        redraw |= self.notes.update(event);
        redraw |= self.due_date.update(event);
        redraw |= self.due_time.update(event);
        redraw |= self.reminder_date.update(event);
        redraw |= self.reminder_time.update(event);
        redraw
    }
}

#[derive(Clone, Copy)]
enum StoreError {
    Unavailable,
    TooLarge,
}

#[derive(Clone, Copy)]
enum KvClientError {
    Unavailable,
    NotFound,
    TooLarge,
}

struct KvReminderStore {
    available: bool,
}

impl KvReminderStore {
    fn new() -> Self {
        Self {
            available: kv_cap().is_ok(),
        }
    }

    fn load_lists(&mut self) -> [TaskList; 3] {
        let now = monotonic_millis();
        let mut lists = [
            TaskList::default_named("inbox", now).unwrap(),
            TaskList::default_named("work", now).unwrap(),
            TaskList::default_named("personal", now).unwrap(),
        ];

        for (idx, (id, name)) in DEFAULT_LISTS.iter().enumerate() {
            let key = list_key(id);
            match self.get(&key) {
                Ok(Some(bytes)) => match decode_list(&bytes) {
                    Some(list) => lists[idx] = list,
                    None => {
                        debug_log("[REMINDERS] skipped malformed list record\n");
                        let default = TaskList::new(id, name, now, now).unwrap();
                        let _ = self.put(&key, &encode_list(&default));
                        lists[idx] = default;
                    }
                },
                Ok(None) => {
                    let default = TaskList::new(id, name, now, now).unwrap();
                    let _ = self.put(&key, &encode_list(&default));
                    lists[idx] = default;
                }
                Err(_) => {}
            }
        }

        lists
    }

    fn load_tasks(&mut self) -> Vec<Task> {
        let mut tasks = Vec::new();
        let ids = match self.get(INDEX_ALL_KEY) {
            Ok(Some(bytes)) => parse_id_list(&bytes).unwrap_or_default(),
            Ok(None) => Vec::new(),
            Err(_) => {
                self.available = false;
                return tasks;
            }
        };

        for id in ids {
            let key = task_key(id);
            match self.get(&key) {
                Ok(Some(bytes)) => match decode_task(&bytes) {
                    Some(task) => tasks.push(task),
                    None => debug_log("[REMINDERS] skipped malformed task record\n"),
                },
                Ok(None) => debug_log("[REMINDERS] skipped missing indexed task\n"),
                Err(_) => {
                    self.available = false;
                    break;
                }
            }
        }

        tasks
    }

    fn load_setting(&mut self, key: &str) -> Result<Option<Vec<u8>>, StoreError> {
        self.get(&settings_key(key))
    }

    fn save_setting(&mut self, key: &str, value: &[u8]) -> Result<(), StoreError> {
        self.put(&settings_key(key), value)
    }

    fn save_task(&mut self, task: &Task, previous: Option<&Task>) -> Result<(), StoreError> {
        if let Some(old) = previous {
            self.delete_task_indexes(old)?;
        }
        self.put_task_record(task)?;
        self.put_task_indexes(task)?;
        self.update_all_index(task.id)?;
        Ok(())
    }

    fn delete_task(&mut self, task: &Task) -> Result<(), StoreError> {
        self.delete_task_indexes(task)?;
        self.delete_key(&task_key(task.id))?;
        let mut ids = self.load_task_ids().unwrap_or_default();
        ids.retain(|id| *id != task.id);
        self.save_task_ids(&ids)
    }

    fn load_task_ids(&mut self) -> Result<Vec<u64>, StoreError> {
        match self.get(INDEX_ALL_KEY)? {
            Some(bytes) => Ok(parse_id_list(&bytes).unwrap_or_default()),
            None => Ok(Vec::new()),
        }
    }

    fn save_task_ids(&mut self, ids: &[u64]) -> Result<(), StoreError> {
        self.put(INDEX_ALL_KEY, &encode_id_list(ids))
    }

    fn update_all_index(&mut self, task_id: u64) -> Result<(), StoreError> {
        let mut ids = self.load_task_ids().unwrap_or_default();
        if !ids.iter().any(|id| *id == task_id) {
            ids.push(task_id);
        }
        self.save_task_ids(&ids)
    }

    fn put_task_record(&mut self, task: &Task) -> Result<(), StoreError> {
        self.put(&task_key(task.id), &encode_task(task))
    }

    fn put_task_indexes(&mut self, task: &Task) -> Result<(), StoreError> {
        self.put_due_date_index(task.due_date.as_str(), task.id)?;
        self.put_reminder_date_index(task.reminder_date.as_str(), task.id)?;
        if task.reminder_date.is_empty()
            && !task.due_date.is_empty()
            && !task.reminder_time.is_empty()
        {
            self.put_reminder_date_index(task.due_date.as_str(), task.id)?;
        }
        Ok(())
    }

    fn delete_task_indexes(&mut self, task: &Task) -> Result<(), StoreError> {
        self.delete_due_date_index(task.due_date.as_str(), task.id)?;
        self.delete_reminder_date_index(task.reminder_date.as_str(), task.id)?;
        if task.reminder_date.is_empty()
            && !task.due_date.is_empty()
            && !task.reminder_time.is_empty()
        {
            self.delete_reminder_date_index(task.due_date.as_str(), task.id)?;
        }
        Ok(())
    }

    fn put_due_date_index(&mut self, date: &str, task_id: u64) -> Result<(), StoreError> {
        if date.is_empty() {
            return Ok(());
        }
        // marker for stability
        let _ = self.put(&date_index_key(date, task_id), b"1");
        // list for queryable by-date (due)
        self.add_id_to_date_list(INDEX_BY_DATE_PREFIX, date, task_id)
    }

    fn delete_due_date_index(&mut self, date: &str, task_id: u64) -> Result<(), StoreError> {
        if date.is_empty() {
            return Ok(());
        }
        let _ = self.delete_key(&date_index_key(date, task_id));
        self.remove_id_from_date_list(INDEX_BY_DATE_PREFIX, date, task_id)
    }

    fn put_reminder_date_index(&mut self, date: &str, task_id: u64) -> Result<(), StoreError> {
        if date.is_empty() {
            return Ok(());
        }
        // marker
        let _ = self.put(&reminder_date_index_key(date, task_id), b"1");
        // list for reminders
        self.add_id_to_date_list(INDEX_REMINDER_DATE_PREFIX, date, task_id)
    }

    fn delete_reminder_date_index(&mut self, date: &str, task_id: u64) -> Result<(), StoreError> {
        if date.is_empty() {
            return Ok(());
        }
        let _ = self.delete_key(&reminder_date_index_key(date, task_id));
        self.remove_id_from_date_list(INDEX_REMINDER_DATE_PREFIX, date, task_id)
    }

    fn add_id_to_date_list(
        &mut self,
        prefix: &str,
        date: &str,
        task_id: u64,
    ) -> Result<(), StoreError> {
        if date.is_empty() {
            return Ok(());
        }
        let list_key = if prefix == INDEX_BY_DATE_PREFIX {
            by_date_list_key(date)
        } else {
            reminder_date_list_key(date)
        };
        let existing = self.get(&list_key)?;
        let next = add_id_to_index_list(existing.as_deref(), task_id);
        self.put(&list_key, &next)?;
        Ok(())
    }

    fn remove_id_from_date_list(
        &mut self,
        prefix: &str,
        date: &str,
        task_id: u64,
    ) -> Result<(), StoreError> {
        if date.is_empty() {
            return Ok(());
        }
        let list_key = if prefix == INDEX_BY_DATE_PREFIX {
            by_date_list_key(date)
        } else {
            reminder_date_list_key(date)
        };
        let existing = self.get(&list_key)?;
        match remove_id_from_index_list(existing.as_deref(), task_id) {
            Some(next) => self.put(&list_key, &next)?,
            None => {
                if existing.is_some() {
                    let _ = self.delete_key(&list_key);
                }
            }
        }
        Ok(())
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
            debug_log("[REMINDERS-KV] lookup sunlight-kv failed/timeout\n");
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
    let reply = reply_res.map_err(|_| KvClientError::Unavailable)?;
    if reply.label == KV_REPLY && reply.words[0] == 0 {
        Ok(())
    } else {
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
    let reply = reply_res.map_err(|_| KvClientError::Unavailable)?;
    if reply.label == KV_ERROR && reply.words[0] == 2 {
        return Err(KvClientError::NotFound);
    }
    if reply.label != KV_VALUE {
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
    let reply = reply_res.map_err(|_| KvClientError::Unavailable)?;
    if reply.label == KV_REPLY && reply.words[0] == 0 {
        Ok(())
    } else if reply.label == KV_ERROR && reply.words[0] == 2 {
        Err(KvClientError::NotFound)
    } else {
        Err(KvClientError::Unavailable)
    }
}

struct ReminderApp {
    store: KvReminderStore,
    lists: [TaskList; 3],
    tasks: Vec<Task>,
    editor: TaskEditor,
    view: SidebarView,
    today: TinyString<DATE_LEN>,
    status: TinyString<96>,
    selected_task_id: Option<u64>,
    next_task_id: u64,
    delete_confirm: bool,
    loaded: bool,
}

impl ReminderApp {
    fn new() -> Self {
        let today = current_local_date();
        let mut editor = TaskEditor::new();
        editor.hide();
        Self {
            store: KvReminderStore::new(),
            lists: [
                TaskList::default_named("inbox", monotonic_millis()).unwrap(),
                TaskList::default_named("work", monotonic_millis()).unwrap(),
                TaskList::default_named("personal", monotonic_millis()).unwrap(),
            ],
            tasks: Vec::new(),
            editor,
            view: SidebarView::Inbox,
            today,
            status: TinyString::empty(),
            selected_task_id: None,
            next_task_id: 1,
            delete_confirm: false,
            loaded: false,
        }
    }

    fn current_list_index(&self) -> usize {
        match self.view.list_id() {
            Some("inbox") => 0,
            Some("work") => 1,
            Some("personal") => 2,
            _ => 0,
        }
    }

    fn select_task(&mut self, task_id: u64) {
        if let Some(task) = self.tasks.iter().find(|task| task.id == task_id).copied() {
            let list_idx = self.list_index_for_id(task.list_id.as_str()).unwrap_or(0);
            self.editor.load_task(&task, list_idx);
            self.selected_task_id = Some(task.id);
        }
    }

    fn list_index_for_id(&self, list_id: &str) -> Option<usize> {
        self.lists
            .iter()
            .position(|list| list.id.as_str() == list_id)
    }

    fn task_matches_view(&self, task: &Task) -> bool {
        match self.view {
            SidebarView::Inbox | SidebarView::Work | SidebarView::Personal => {
                task.list_id.as_str() == self.view.list_id().unwrap_or("")
            }
            SidebarView::Today => {
                task.status == TaskStatus::Todo && self.task_date_is_today_or_past(task)
            }
            SidebarView::Upcoming => {
                task.status == TaskStatus::Todo && self.task_date_is_future(task)
            }
            SidebarView::Completed => task.status == TaskStatus::Done,
        }
    }

    fn task_date_is_today_or_past(&self, task: &Task) -> bool {
        match self.task_primary_date(task) {
            Some(date) => date <= self.today.as_str(),
            None => false,
        }
    }

    fn task_date_is_future(&self, task: &Task) -> bool {
        match self.task_primary_date(task) {
            Some(date) => date > self.today.as_str(),
            None => false,
        }
    }

    fn task_primary_date<'a>(&self, task: &'a Task) -> Option<&'a str> {
        match (task.due_date.as_str(), task.reminder_date.as_str()) {
            ("", "") => None,
            (due, "") => Some(due),
            ("", rem) => Some(rem),
            (due, rem) => {
                if due <= rem {
                    Some(due)
                } else {
                    Some(rem)
                }
            }
        }
    }

    fn task_primary_time<'a>(&self, task: &'a Task) -> Option<&'a str> {
        let due_date = task.due_date.as_str();
        let reminder_date = task.reminder_date.as_str();
        match (due_date.is_empty(), reminder_date.is_empty()) {
            (true, true) => None,
            (false, true) => {
                if task.due_time.is_empty() {
                    None
                } else {
                    Some(task.due_time.as_str())
                }
            }
            (true, false) => {
                if task.reminder_time.is_empty() {
                    None
                } else {
                    Some(task.reminder_time.as_str())
                }
            }
            (false, false) => {
                if due_date < reminder_date {
                    if task.due_time.is_empty() {
                        None
                    } else {
                        Some(task.due_time.as_str())
                    }
                } else if reminder_date < due_date {
                    if task.reminder_time.is_empty() {
                        None
                    } else {
                        Some(task.reminder_time.as_str())
                    }
                } else if !task.due_time.is_empty() && !task.reminder_time.is_empty() {
                    if task.due_time.as_str() <= task.reminder_time.as_str() {
                        Some(task.due_time.as_str())
                    } else {
                        Some(task.reminder_time.as_str())
                    }
                } else if !task.due_time.is_empty() {
                    Some(task.due_time.as_str())
                } else if !task.reminder_time.is_empty() {
                    Some(task.reminder_time.as_str())
                } else {
                    None
                }
            }
        }
    }

    fn sort_tasks(&mut self) {
        self.tasks.sort_by(task_cmp);
    }

    fn reload_from_store(&mut self) {
        if let Ok(Some(bytes)) = self.store.load_setting("selected-view") {
            if let Ok(text) = core::str::from_utf8(&bytes) {
                if let Some(view) = SidebarView::from_str(text) {
                    self.view = view;
                }
            }
        }
        self.lists = self.store.load_lists();
        self.tasks = self.store.load_tasks();
        self.sort_tasks();
        self.next_task_id = self.tasks.iter().map(|task| task.id).max().unwrap_or(0) + 1;
        self.status.clear();
        if self.store.available {
            self.status.set("Loaded from sunlight-kv");
        } else {
            self.status.set("sunlight-kv unavailable; memory only");
        }
        self.loaded = true;
        if self.selected_task_id.is_none() {
            if let Some(task_id) = self.first_visible_task_id() {
                self.select_task(task_id);
            }
        } else if let Some(task_id) = self.selected_task_id {
            if !self.tasks.iter().any(|task| task.id == task_id) {
                self.selected_task_id = None;
                self.editor.hide();
            }
        }
    }

    fn first_visible_task_id(&self) -> Option<u64> {
        self.tasks
            .iter()
            .find(|task| self.task_matches_view(task))
            .map(|task| task.id)
    }

    fn count_tasks_for_view(&self, view: SidebarView) -> usize {
        self.tasks
            .iter()
            .filter(|task| match view {
                SidebarView::Inbox => task.list_id.as_str() == "inbox",
                SidebarView::Work => task.list_id.as_str() == "work",
                SidebarView::Personal => task.list_id.as_str() == "personal",
                SidebarView::Today => {
                    task.status == TaskStatus::Todo && self.task_date_is_today_or_past(task)
                }
                SidebarView::Upcoming => {
                    task.status == TaskStatus::Todo && self.task_date_is_future(task)
                }
                SidebarView::Completed => task.status == TaskStatus::Done,
            })
            .count()
    }

    fn count_tasks_for_list(&self, list_id: &str) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.list_id.as_str() == list_id)
            .count()
    }

    fn add_task(&mut self) {
        self.editor.start_new(self.current_list_index());
        self.selected_task_id = None;
        self.delete_confirm = false;
        self.status.set("New task");
    }

    fn save_task(&mut self) {
        let now = monotonic_millis();
        let existing = self
            .editor
            .editing_id
            .and_then(|id| self.tasks.iter().find(|task| task.id == id).copied());
        let id = existing.map(|task| task.id).unwrap_or(self.next_task_id);
        let created_at = existing.map(|task| task.created_at).unwrap_or(now);
        let Some(task) = self.editor.build_task(id, created_at, now, &self.lists) else {
            self.editor
                .error
                .set(self.editor.validate().unwrap_or("Invalid task"));
            self.status.set("Validation error");
            return;
        };

        let mut updated = false;
        if let Some(slot) = self.tasks.iter_mut().find(|task| task.id == id) {
            *slot = task;
            updated = true;
        } else {
            self.tasks.push(task);
            self.next_task_id = self.next_task_id.max(id.saturating_add(1));
        }
        self.sort_tasks();

        let store_result = if let Some(old) = existing {
            self.store.save_task(&task, Some(&old))
        } else {
            self.store.save_task(&task, None)
        };

        if store_result.is_err() {
            self.status.set("Saved in memory only");
        } else {
            self.status.set("Task saved");
        }

        self.selected_task_id = Some(id);
        self.editor.load_task(&task, self.editor.selected_list_idx);
        if !updated {
            self.next_task_id = self.tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        }
        let _ = self
            .store
            .save_setting("selected-view", self.view.as_str().as_bytes());
    }

    fn toggle_selected_task(&mut self) {
        let Some(task_id) = self.selected_task_id else {
            return;
        };
        let Some(index) = self.tasks.iter().position(|task| task.id == task_id) else {
            return;
        };
        let mut task = self.tasks[index];
        task.status = match task.status {
            TaskStatus::Todo => TaskStatus::Done,
            TaskStatus::Done => TaskStatus::Todo,
        };
        task.updated_at = monotonic_millis();
        let previous = self.tasks[index];
        self.tasks[index] = task;
        self.sort_tasks();
        if self.store.save_task(&task, Some(&previous)).is_err() {
            self.status.set("Updated in memory only");
        } else {
            self.status.set("Task updated");
        }
        if self.editor.visible {
            self.editor.status = task.status;
            self.editor.load_task(&task, self.editor.selected_list_idx);
        }
    }

    fn delete_selected_task(&mut self) {
        let Some(task_id) = self.selected_task_id else {
            return;
        };
        let Some(index) = self.tasks.iter().position(|task| task.id == task_id) else {
            return;
        };
        let task = self.tasks[index];
        if self.store.delete_task(&task).is_err() {
            self.status.set("Delete failed; memory only");
        } else {
            self.status.set("Task deleted");
        }
        self.tasks.remove(index);
        self.selected_task_id = self.first_visible_task_id();
        if let Some(id) = self.selected_task_id {
            self.select_task(id);
        } else {
            self.editor.hide();
        }
        self.delete_confirm = false;
        self.next_task_id = self.tasks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
    }

    fn update_today(&mut self) -> bool {
        let mut today = TinyString::<DATE_LEN>::empty();
        let now = local_now_best_effort(get_time_utc());
        today.set(&sunlight_reminders::format_date(
            now.year.into(),
            now.month as i32,
            now.day as i32,
        ));
        if today.as_str() != self.today.as_str() {
            self.today = today;
            self.sort_tasks();
            return true;
        }
        false
    }

    fn task_status_label(&self) -> &'static str {
        match self.editor.status {
            TaskStatus::Todo => "Mark Done",
            TaskStatus::Done => "Mark Todo",
        }
    }

    fn footer_buttons(&self) -> (Rect, Rect) {
        let footer = self.footer_rect();
        let y = footer.y + (footer.h as i32 - 28) / 2;
        (
            Rect::new(footer.x + PAD, y, 96, 28),
            Rect::new(footer.x + PAD + 104, y, 84, 28),
        )
    }

    fn header_rect(&self) -> Rect {
        Rect::new(0, 0, WIN_W, HEADER_H)
    }

    fn body_rect(&self) -> Rect {
        Rect::new(0, HEADER_H as i32, WIN_W, WIN_H - HEADER_H - FOOTER_H)
    }

    fn footer_rect(&self) -> Rect {
        Rect::new(0, (WIN_H - FOOTER_H) as i32, WIN_W, FOOTER_H)
    }

    fn panel_rects(&self) -> (Rect, Rect, Rect) {
        let body = self.body_rect().inset(8);
        let widths = [2, 4, 4];
        let mut cols = GridRow::new(body).with_gap(BODY_GAP as u32).layout(&widths);
        let left = cols.next().unwrap_or_default();
        let center = cols.next().unwrap_or_default();
        let right = cols.next().unwrap_or_default();
        (left, center, right)
    }

    fn sidebar_rects(&self) -> [Rect; 8] {
        let (left, _, _) = self.panel_rects();
        let content = Panel::with_title(left, "Lists & Views")
            .content_rect()
            .inset(8);
        let rows = [
            SidebarGroupHeader::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarGroupHeader::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarItem::HEIGHT,
        ];
        let mut iter = VBox::new(content).with_spacing(4).layout(&rows);
        [
            iter.next().unwrap_or_default(),
            iter.next().unwrap_or_default(),
            iter.next().unwrap_or_default(),
            iter.next().unwrap_or_default(),
            iter.next().unwrap_or_default(),
            iter.next().unwrap_or_default(),
            iter.next().unwrap_or_default(),
            iter.next().unwrap_or_default(),
        ]
    }

    fn current_filter(&self) -> SidebarView {
        self.view
    }

    fn set_filter(&mut self, view: SidebarView) {
        self.view = view;
        let _ = self
            .store
            .save_setting("selected-view", view.as_str().as_bytes());
    }

    fn draw_header(&self, canvas: &mut Canvas, theme: &Theme) {
        let rect = self.header_rect();
        canvas.fill_rect(rect, theme.bg);
        canvas.hbar(rect.x, rect.bottom() - 1, rect.w, 1, theme.border);

        draw_text(
            canvas,
            "Sunlight Reminders",
            rect.x + PAD,
            rect.y + 5,
            &TextStyle::new(FontRole::UiMedium, theme.accent),
        );
        draw_text(
            canvas,
            "Personal tasks, reminders, and daily planning",
            rect.x + PAD + 188,
            rect.y + 9,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );

        let mut status = TinyString::<64>::empty();
        if self.store.available {
            status.set("sunlight-kv");
        } else {
            status.set("memory");
        }
        let right_label = status.as_str();
        let tw = measure_text(right_label, FontRole::UiSmall).w as i32;
        draw_text_vcenter(
            canvas,
            right_label,
            rect.right() - PAD - tw,
            rect.y,
            rect.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }

    fn visible_task_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| self.task_matches_view(task))
            .count()
    }

    fn draw_sidebar(&self, canvas: &mut Canvas, theme: &Theme) {
        let (left, _, _) = self.panel_rects();
        Panel::with_title(left, "Lists & Views").draw(canvas, theme);
        let content = Panel::with_title(left, "Lists & Views")
            .content_rect()
            .inset(8);
        let rows = [
            SidebarGroupHeader::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarGroupHeader::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarItem::HEIGHT,
            SidebarItem::HEIGHT,
        ];
        let mut rows = VBox::new(content).with_spacing(4).layout(&rows);
        let _lists_header = rows.next().unwrap_or_default();
        let inbox = rows.next().unwrap_or_default();
        let work = rows.next().unwrap_or_default();
        let personal = rows.next().unwrap_or_default();
        let _views_header = rows.next().unwrap_or_default();
        let today = rows.next().unwrap_or_default();
        let upcoming = rows.next().unwrap_or_default();
        let completed = rows.next().unwrap_or_default();

        SidebarGroupHeader::new(_lists_header, "Lists").draw(canvas, theme);
        let items = [
            (
                SidebarView::Inbox,
                inbox,
                self.lists[0].name.as_str(),
                self.count_tasks_for_list("inbox"),
            ),
            (
                SidebarView::Work,
                work,
                self.lists[1].name.as_str(),
                self.count_tasks_for_list("work"),
            ),
            (
                SidebarView::Personal,
                personal,
                self.lists[2].name.as_str(),
                self.count_tasks_for_list("personal"),
            ),
            (
                SidebarView::Today,
                today,
                "Today",
                self.count_tasks_for_view(SidebarView::Today),
            ),
            (
                SidebarView::Upcoming,
                upcoming,
                "Upcoming",
                self.count_tasks_for_view(SidebarView::Upcoming),
            ),
            (
                SidebarView::Completed,
                completed,
                "Completed",
                self.count_tasks_for_view(SidebarView::Completed),
            ),
        ];

        for (view, rect, label, count) in items {
            let mut badge = TinyString::<8>::empty();
            badge.set("");
            if count > 0 {
                let mut value = TinyString::<8>::empty();
                push_count(&mut value, count as u64);
                badge = value;
            }
            let state = if self.current_filter() == view {
                SidebarState::Selected
            } else {
                SidebarState::Normal
            };
            let mut item = SidebarItem::new(rect, label)
                .with_state(state)
                .with_font(&F_UI);
            if !badge.is_empty() {
                item = item.with_badge(badge.as_str());
            }
            item.draw(canvas, theme);
        }

        SidebarGroupHeader::new(_views_header, "Views").draw(canvas, theme);
    }

    fn draw_task_list(&self, canvas: &mut Canvas, theme: &Theme) {
        let (_, center, _) = self.panel_rects();
        Panel::with_title(center, "Tasks").draw(canvas, theme);
        let content = Panel::with_title(center, "Tasks").content_rect().inset(8);
        let count = self.visible_task_count();
        let mut summary = TinyString::<64>::empty();
        summary.set(self.current_filter().label());
        draw_text(
            canvas,
            summary.as_str(),
            content.x,
            content.y - 4,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );
        let mut y = content.y + 14;
        let row_h = 42i32;
        let mut drew_any = false;

        for (visible_index, task) in self
            .tasks
            .iter()
            .filter(|task| self.task_matches_view(task))
            .enumerate()
        {
            let row = Rect::new(content.x, y, content.w, row_h as u32);
            self.draw_task_row(canvas, theme, task, row, visible_index);
            drew_any = true;
            y += row_h + 4;
            if y > content.bottom() - row_h {
                break;
            }
        }

        if !drew_any {
            draw_text_vcenter(
                canvas,
                "No tasks here yet",
                content.x,
                content.y + 40,
                30,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
            draw_text_vcenter(
                canvas,
                "Use Add Task to create one",
                content.x,
                content.y + 68,
                26,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
        }

        let mut count_str = TinyString::<16>::empty();
        push_count(&mut count_str, count as u64);
        draw_text_vcenter(
            canvas,
            count_str.as_str(),
            content.right() - 36,
            content.y - 2,
            18,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
    }

    fn draw_task_row(
        &self,
        canvas: &mut Canvas,
        theme: &Theme,
        task: &Task,
        row: Rect,
        row_index: usize,
    ) {
        let selected = self.selected_task_id == Some(task.id);
        let bg = if selected {
            theme.panel_alt
        } else if row_index % 2 == 0 {
            theme.panel
        } else {
            theme.panel_alt.darken(6)
        };
        canvas.fill_rect(row, bg);
        canvas.draw_rect(row, if selected { theme.accent } else { theme.border });
        if selected {
            canvas.fill_rect(Rect::new(row.x, row.y, 3, row.h), theme.accent);
        }

        let box_rect = Rect::new(row.x + 8, row.y + 13, 14, 14);
        let mut checkbox = Checkbox::new(box_rect, "");
        checkbox.checked = task.status == TaskStatus::Done;
        checkbox.draw(canvas, theme);

        let text_x = row.x + 28;
        let title_color = if task.status == TaskStatus::Done {
            theme.text_dim
        } else {
            theme.text
        };
        draw_text(
            canvas,
            task.title.as_str(),
            text_x,
            row.y + 7,
            &TextStyle::new(FontRole::UiRegular, title_color),
        );

        let mut meta = TinyString::<96>::empty();
        if let Some(date) = self.task_primary_date(task) {
            meta.set(date);
            if let Some(time) = self.task_primary_time(task) {
                let mut combined = TinyString::<96>::empty();
                combined.set(date);
                let _ = combined.try_set("");
                let mut text = TinyString::<96>::empty();
                text.set(date);
                draw_text(
                    canvas,
                    text.as_str(),
                    text_x,
                    row.y + 22,
                    &TextStyle::new(FontRole::UiSmall, theme.text_muted),
                );
                draw_text(
                    canvas,
                    time,
                    text_x + 94,
                    row.y + 22,
                    &TextStyle::new(FontRole::UiSmall, theme.text_muted),
                );
            } else {
                draw_text(
                    canvas,
                    date,
                    text_x,
                    row.y + 22,
                    &TextStyle::new(FontRole::UiSmall, theme.text_muted),
                );
            }
        }

        if !task.notes.is_empty() {
            let icon_rect = Rect::new(row.right() - 20, row.y + 14, 6, 6);
            canvas.fill_rect(icon_rect, theme.accent);
        }

        if !task.reminder_date.is_empty() {
            StatusBadge::new(row.right() - 12, row.y + 12, BadgeKind::Accent).draw(canvas, theme);
        }
    }

    fn draw_editor(&mut self, canvas: &mut Canvas, theme: &Theme) {
        let (_, _, right) = self.panel_rects();
        Panel::with_title(
            right,
            if self.editor.visible {
                if self.editor.editing_id.is_some() {
                    "Task Details"
                } else {
                    "New Task"
                }
            } else {
                "Details"
            },
        )
        .draw(canvas, theme);
        let content = Panel::with_title(right, "Details").content_rect().inset(8);

        if !self.editor.visible {
            draw_text_vcenter(
                canvas,
                "Select a task or press Add Task",
                content.x,
                content.y + 20,
                28,
                &TextStyle::new(FontRole::UiRegular, theme.text_dim),
            );
            draw_text_vcenter(
                canvas,
                "Notes stay in sunlight-kv, separate from the system task monitor.",
                content.x,
                content.y + 48,
                40,
                &TextStyle::new(FontRole::UiSmall, theme.text_dim),
            );
            return;
        }

        let rows = [14, 28, 14, 56, 14, 28, 14, 28, 14, 28, 14, 28, 14, 28];
        let mut layout = VBox::new(content).with_spacing(4).layout(&rows);
        let title_label = layout.next().unwrap_or_default();
        let title_input = layout.next().unwrap_or_default();
        let notes_label = layout.next().unwrap_or_default();
        let notes_input = layout.next().unwrap_or_default();
        let due_label = layout.next().unwrap_or_default();
        let due_row = layout.next().unwrap_or_default();
        let reminder_label = layout.next().unwrap_or_default();
        let reminder_row = layout.next().unwrap_or_default();
        let list_label = layout.next().unwrap_or_default();
        let list_row = layout.next().unwrap_or_default();
        let action_label = layout.next().unwrap_or_default();
        let action_row = layout.next().unwrap_or_default();
        let error_line = layout.next().unwrap_or_default();

        Label::new(title_label, "Title")
            .with_font(&F_SMALL)
            .dim()
            .draw(canvas, theme);
        self.editor.title.rect = title_input;
        self.editor.title.draw(canvas, theme);

        Label::new(notes_label, "Notes")
            .with_font(&F_SMALL)
            .dim()
            .draw(canvas, theme);
        self.editor.notes.rect = notes_input;
        self.editor.notes.draw(canvas, theme);

        Label::new(due_label, "Due")
            .with_font(&F_SMALL)
            .dim()
            .draw(canvas, theme);
        let due_widths = [due_row.w.saturating_sub(78), 72];
        let mut due_inputs = HBox::new(due_row).with_spacing(6).layout(&due_widths);
        let due_date = due_inputs.next().unwrap_or_default();
        let due_time = due_inputs.next().unwrap_or_default();
        self.editor.due_date.rect = due_date;
        self.editor.due_time.rect = due_time;
        self.editor.due_date.draw(canvas, theme);
        self.editor.due_time.draw(canvas, theme);

        Label::new(reminder_label, "Reminder")
            .with_font(&F_SMALL)
            .dim()
            .draw(canvas, theme);
        let reminder_widths = [reminder_row.w.saturating_sub(78), 72];
        let mut reminder_inputs = HBox::new(reminder_row)
            .with_spacing(6)
            .layout(&reminder_widths);
        let reminder_date = reminder_inputs.next().unwrap_or_default();
        let reminder_time = reminder_inputs.next().unwrap_or_default();
        self.editor.reminder_date.rect = reminder_date;
        self.editor.reminder_time.rect = reminder_time;
        self.editor.reminder_date.draw(canvas, theme);
        self.editor.reminder_time.draw(canvas, theme);

        Label::new(list_label, "List")
            .with_font(&F_SMALL)
            .dim()
            .draw(canvas, theme);
        let list_widths = [
            list_row.w.saturating_sub(12) / 3,
            list_row.w.saturating_sub(12) / 3,
            list_row.w.saturating_sub(12) / 3,
        ];
        let mut list_buttons = HBox::new(list_row).with_spacing(6).layout(&list_widths);
        let inbox_btn = list_buttons.next().unwrap_or_default();
        let work_btn = list_buttons.next().unwrap_or_default();
        let personal_btn = list_buttons.next().unwrap_or_default();
        let mut btn = if self.editor.selected_list_idx == 0 {
            Button::new(inbox_btn, self.lists[0].name.as_str())
        } else {
            Button::secondary(inbox_btn, self.lists[0].name.as_str())
        };
        btn.state = ButtonState::Normal;
        btn.draw(canvas, theme);
        let mut btn = if self.editor.selected_list_idx == 1 {
            Button::new(work_btn, self.lists[1].name.as_str())
        } else {
            Button::secondary(work_btn, self.lists[1].name.as_str())
        };
        btn.state = ButtonState::Normal;
        btn.draw(canvas, theme);
        let mut btn = if self.editor.selected_list_idx == 2 {
            Button::new(personal_btn, self.lists[2].name.as_str())
        } else {
            Button::secondary(personal_btn, self.lists[2].name.as_str())
        };
        btn.state = ButtonState::Normal;
        btn.draw(canvas, theme);

        Label::new(action_label, "Actions")
            .with_font(&F_SMALL)
            .dim()
            .draw(canvas, theme);
        let action_widths_3 = [
            (action_row.w.saturating_sub(12)) / 3,
            (action_row.w.saturating_sub(12)) / 3,
            (action_row.w.saturating_sub(12)) / 3,
        ];
        let action_widths_2 = [
            (action_row.w.saturating_sub(6)) / 2,
            (action_row.w.saturating_sub(6)) / 2,
        ];
        let mut action_buttons = if self.editor.editing_id.is_some() {
            HBox::new(action_row)
                .with_spacing(6)
                .layout(&action_widths_3)
        } else {
            HBox::new(action_row)
                .with_spacing(6)
                .layout(&action_widths_2)
        };
        let save_btn = action_buttons.next().unwrap_or_default();
        let delete_btn = if self.editor.editing_id.is_some() {
            action_buttons.next().unwrap_or_default()
        } else {
            Rect::new(0, 0, 0, 0)
        };
        let status_btn = action_buttons.next().unwrap_or_default();
        let mut save = Button::new(save_btn, "Save").with_font(&F_UI);
        save.state = ButtonState::Normal;
        save.draw(canvas, theme);
        if self.editor.editing_id.is_some() {
            let mut delete = Button::secondary(delete_btn, "Delete").with_font(&F_UI);
            delete.state = ButtonState::Normal;
            delete.draw(canvas, theme);
        }
        let mut status = Button::secondary(status_btn, self.task_status_label()).with_font(&F_UI);
        status.state = ButtonState::Normal;
        status.draw(canvas, theme);

        if self.editor.error.len() > 0 {
            draw_text(
                canvas,
                self.editor.error.as_str(),
                error_line.x,
                error_line.y + 1,
                &TextStyle::new(FontRole::UiSmall, theme.danger_text),
            );
        }

        if self.delete_confirm {
            self.draw_delete_confirm(canvas, theme, content);
        }
    }

    fn draw_delete_confirm(&self, canvas: &mut Canvas, theme: &Theme, content: Rect) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg.darken(70));
        let panel = Rect::new(
            content.x + 20,
            content.y + 64,
            content.w.saturating_sub(40),
            120,
        );
        canvas.fill_rect(panel, theme.panel);
        canvas.draw_rect(panel, theme.border);
        draw_text(
            canvas,
            "Delete this task?",
            panel.x + 12,
            panel.y + 14,
            &TextStyle::new(FontRole::UiRegular, theme.text),
        );
        draw_text(
            canvas,
            "This removes the task and its date indexes from sunlight-kv.",
            panel.x + 12,
            panel.y + 38,
            &TextStyle::new(FontRole::UiSmall, theme.text_dim),
        );
        let btn_y = panel.bottom() - 36;
        let btn_w = (panel.w.saturating_sub(30)) / 2;
        let delete_btn = Rect::new(panel.x + 10, btn_y, btn_w, 28);
        let cancel_btn = Rect::new(panel.x + 20 + btn_w as i32, btn_y, btn_w, 28);
        let mut delete = Button::new(delete_btn, "Delete");
        delete.state = ButtonState::Normal;
        delete.draw(canvas, theme);
        let mut cancel = Button::secondary(cancel_btn, "Cancel");
        cancel.state = ButtonState::Normal;
        cancel.draw(canvas, theme);
    }

    fn handle_click(&mut self, x: i32, y: i32) -> bool {
        let (_, _center, right) = self.panel_rects();
        let content = Panel::with_title(right, "Details").content_rect().inset(8);
        if self.delete_confirm {
            let panel = Rect::new(
                content.x + 20,
                content.y + 64,
                content.w.saturating_sub(40),
                120,
            );
            let btn_y = panel.bottom() - 36;
            let btn_w = (panel.w.saturating_sub(30)) / 2;
            let delete_btn = Rect::new(panel.x + 10, btn_y, btn_w, 28);
            let cancel_btn = Rect::new(panel.x + 20 + btn_w as i32, btn_y, btn_w, 28);
            if delete_btn.contains(Point::new(x, y)) {
                self.delete_selected_task();
                return true;
            }
            if cancel_btn.contains(Point::new(x, y)) {
                self.delete_confirm = false;
                return true;
            }
            return true;
        }

        let footer = self.footer_buttons();
        if footer.0.contains(Point::new(x, y)) {
            self.add_task();
            return true;
        }
        if footer.1.contains(Point::new(x, y)) {
            self.reload_from_store();
            self.status.set("Refreshed");
            return true;
        }

        let sidebar = self.sidebar_rects();
        let sidebar_items = [
            (SidebarView::Inbox, sidebar[1]),
            (SidebarView::Work, sidebar[2]),
            (SidebarView::Personal, sidebar[3]),
            (SidebarView::Today, sidebar[5]),
            (SidebarView::Upcoming, sidebar[6]),
            (SidebarView::Completed, sidebar[7]),
        ];
        for (view, rect) in sidebar_items {
            if rect.contains(Point::new(x, y)) {
                self.set_filter(view);
                return true;
            }
        }

        if self.editor.visible {
            if self.editor.update_inputs(Event::Click { x, y }) {
                return true;
            }

            let rows = [14, 28, 14, 56, 14, 28, 14, 28, 14, 28, 14, 28, 14, 28];
            let mut layout = VBox::new(content).with_spacing(4).layout(&rows);
            let _title_label = layout.next().unwrap_or_default();
            let title_input = layout.next().unwrap_or_default();
            let _notes_label = layout.next().unwrap_or_default();
            let notes_input = layout.next().unwrap_or_default();
            let _due_label = layout.next().unwrap_or_default();
            let due_row = layout.next().unwrap_or_default();
            let _reminder_label = layout.next().unwrap_or_default();
            let reminder_row = layout.next().unwrap_or_default();
            let _list_label = layout.next().unwrap_or_default();
            let list_row = layout.next().unwrap_or_default();
            let _action_label = layout.next().unwrap_or_default();
            let action_row = layout.next().unwrap_or_default();

            if title_input.contains(Point::new(x, y))
                || notes_input.contains(Point::new(x, y))
                || due_row.contains(Point::new(x, y))
                || reminder_row.contains(Point::new(x, y))
                || list_row.contains(Point::new(x, y))
                || action_row.contains(Point::new(x, y))
            {
                if due_row.contains(Point::new(x, y)) {
                    let due_widths = [due_row.w.saturating_sub(78), 72];
                    let mut due_inputs = HBox::new(due_row).with_spacing(6).layout(&due_widths);
                    let due_date = due_inputs.next().unwrap_or_default();
                    let due_time = due_inputs.next().unwrap_or_default();
                    if due_date.contains(Point::new(x, y)) {
                        self.editor.due_date.active = true;
                        self.editor.due_time.active = false;
                    }
                    if due_time.contains(Point::new(x, y)) {
                        self.editor.due_date.active = false;
                        self.editor.due_time.active = true;
                    }
                }
                if reminder_row.contains(Point::new(x, y)) {
                    let reminder_widths = [reminder_row.w.saturating_sub(78), 72];
                    let mut reminder_inputs = HBox::new(reminder_row)
                        .with_spacing(6)
                        .layout(&reminder_widths);
                    let reminder_date = reminder_inputs.next().unwrap_or_default();
                    let reminder_time = reminder_inputs.next().unwrap_or_default();
                    if reminder_date.contains(Point::new(x, y)) {
                        self.editor.reminder_date.active = true;
                        self.editor.reminder_time.active = false;
                    }
                    if reminder_time.contains(Point::new(x, y)) {
                        self.editor.reminder_date.active = false;
                        self.editor.reminder_time.active = true;
                    }
                }
                if list_row.contains(Point::new(x, y)) {
                    let list_widths = [
                        list_row.w.saturating_sub(12) / 3,
                        list_row.w.saturating_sub(12) / 3,
                        list_row.w.saturating_sub(12) / 3,
                    ];
                    let mut list_buttons = HBox::new(list_row).with_spacing(6).layout(&list_widths);
                    let inbox_btn = list_buttons.next().unwrap_or_default();
                    let work_btn = list_buttons.next().unwrap_or_default();
                    let personal_btn = list_buttons.next().unwrap_or_default();
                    if inbox_btn.contains(Point::new(x, y)) {
                        self.editor.selected_list_idx = 0;
                        return true;
                    }
                    if work_btn.contains(Point::new(x, y)) {
                        self.editor.selected_list_idx = 1;
                        return true;
                    }
                    if personal_btn.contains(Point::new(x, y)) {
                        self.editor.selected_list_idx = 2;
                        return true;
                    }
                }
                if action_row.contains(Point::new(x, y)) {
                    let count = if self.editor.editing_id.is_some() {
                        3
                    } else {
                        2
                    };
                    let action_widths_3 = [
                        (action_row.w.saturating_sub(12)) / 3,
                        (action_row.w.saturating_sub(12)) / 3,
                        (action_row.w.saturating_sub(12)) / 3,
                    ];
                    let action_widths_2 = [
                        (action_row.w.saturating_sub(6)) / 2,
                        (action_row.w.saturating_sub(6)) / 2,
                    ];
                    let mut action_buttons = if count == 3 {
                        HBox::new(action_row)
                            .with_spacing(6)
                            .layout(&action_widths_3)
                    } else {
                        HBox::new(action_row)
                            .with_spacing(6)
                            .layout(&action_widths_2)
                    };
                    let save_btn = action_buttons.next().unwrap_or_default();
                    let delete_btn = if count == 3 {
                        action_buttons.next().unwrap_or_default()
                    } else {
                        Rect::new(0, 0, 0, 0)
                    };
                    let status_btn = action_buttons.next().unwrap_or_default();
                    if save_btn.contains(Point::new(x, y)) {
                        self.editor.error.clear();
                        self.save_task();
                        return true;
                    }
                    if count == 3 && delete_btn.contains(Point::new(x, y)) {
                        self.delete_confirm = true;
                        return true;
                    }
                    if status_btn.contains(Point::new(x, y)) {
                        self.editor.status = match self.editor.status {
                            TaskStatus::Todo => TaskStatus::Done,
                            TaskStatus::Done => TaskStatus::Todo,
                        };
                        if self.editor.editing_id.is_some() {
                            self.save_task();
                        } else {
                            self.status.set("Draft updated");
                        }
                        return true;
                    }
                }
            }
        }

        let (left, center, _) = self.panel_rects();
        let center_content = Panel::with_title(center, "Tasks").content_rect().inset(8);
        if center.contains(Point::new(x, y)) {
            let row_h = 42i32;
            let mut y_off = center_content.y + 14;
            for task in &self.tasks {
                if !self.task_matches_view(task) {
                    continue;
                }
                let row = Rect::new(center_content.x, y_off, center_content.w, row_h as u32);
                if row.contains(Point::new(x, y)) {
                    let check_rect = Rect::new(row.x + 8, row.y + 13, 14, 14);
                    if check_rect.contains(Point::new(x, y)) {
                        self.selected_task_id = Some(task.id);
                        self.select_task(task.id);
                        self.toggle_selected_task();
                        return true;
                    }
                    self.selected_task_id = Some(task.id);
                    self.select_task(task.id);
                    return true;
                }
                y_off += row_h + 4;
                if y_off > center_content.bottom() - row_h {
                    break;
                }
            }
        }

        let _ = left;
        false
    }

    fn draw(&mut self, canvas: &mut Canvas, theme: &Theme) {
        canvas.fill_rect(Rect::new(0, 0, WIN_W, WIN_H), theme.bg);
        self.draw_header(canvas, theme);

        let (left, center, right) = self.panel_rects();
        Panel::with_title(left, "Lists & Views").draw(canvas, theme);
        Panel::with_title(center, "Tasks").draw(canvas, theme);
        Panel::with_title(right, "Details").draw(canvas, theme);

        self.draw_sidebar(canvas, theme);
        self.draw_task_list(canvas, theme);
        self.draw_editor(canvas, theme);

        let footer = self.footer_rect();
        canvas.fill_rect(footer, theme.panel_alt);
        canvas.hbar(footer.x, footer.y, footer.w, 1, theme.border);
        let (add_btn_r, refresh_btn_r) = self.footer_buttons();
        let mut add = Button::new(add_btn_r, "Add Task").with_font(&F_UI);
        add.state = ButtonState::Normal;
        add.draw(canvas, theme);
        let mut refresh = Button::secondary(refresh_btn_r, "Refresh").with_font(&F_UI);
        refresh.state = ButtonState::Normal;
        refresh.draw(canvas, theme);

        let mut status = TinyString::<96>::empty();
        if self.status.len() > 0 {
            status = self.status;
        } else if self.store.available {
            status.set("sunlight-kv ready");
        } else {
            status.set("memory only");
        }
        draw_text_vcenter(
            canvas,
            status.as_str(),
            footer.x + 210,
            footer.y,
            footer.h,
            &TextStyle::new(FontRole::UiSmall, theme.text_muted),
        );
    }
}

impl App for ReminderApp {
    fn view(&mut self, canvas: &mut Canvas, theme: &Theme) {
        self.draw(canvas, theme);
    }

    fn update(&mut self, event: Event) -> bool {
        if matches!(event, Event::Tick) && self.update_today() {
            return true;
        }

        if self.editor.visible && self.editor.update_inputs(event) {
            return true;
        }

        match event {
            Event::Click { x, y } => self.handle_click(x, y),
            Event::Key(ch) if self.editor.visible => {
                if ch == '\u{1b}' {
                    self.editor.hide();
                    return true;
                }
                false
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } if keycode == KEY_ESC => {
                self.editor.hide();
                true
            }
            Event::KeyPress {
                keycode,
                pressed: true,
                ..
            } if keycode == KEY_ENTER => {
                if self.editor.visible {
                    self.save_task();
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn on_ready(&mut self) -> bool {
        self.reload_from_store();
        true
    }
}

fn current_local_date() -> TinyString<DATE_LEN> {
    let now = local_now_best_effort(get_time_utc());
    let mut out = TinyString::<DATE_LEN>::empty();
    out.set(&sunlight_reminders::format_date(
        now.year.into(),
        now.month as i32,
        now.day as i32,
    ));
    out
}

fn format_time_hhmm(hour: u8, minute: u8) -> TinyString<TIME_LEN> {
    let mut out = TinyString::<TIME_LEN>::empty();
    let buf = [
        b'0' + hour / 10,
        b'0' + hour % 10,
        b':',
        b'0' + minute / 10,
        b'0' + minute % 10,
    ];
    if let Ok(text) = core::str::from_utf8(&buf) {
        out.set(text);
    }
    out
}

fn task_cmp(a: &Task, b: &Task) -> core::cmp::Ordering {
    let a_status = match a.status {
        TaskStatus::Todo => 0,
        TaskStatus::Done => 1,
    };
    let b_status = match b.status {
        TaskStatus::Todo => 0,
        TaskStatus::Done => 1,
    };
    a_status
        .cmp(&b_status)
        .then_with(|| task_sort_date(a).cmp(&task_sort_date(b)))
        .then_with(|| task_sort_time(a).cmp(&task_sort_time(b)))
        .then_with(|| a.title.as_str().cmp(b.title.as_str()))
}

fn task_sort_date(task: &Task) -> &str {
    task_primary_date_time(task)
        .map(|pair| pair.0)
        .unwrap_or("9999-12-31")
}

fn task_sort_time(task: &Task) -> &str {
    task_primary_date_time(task)
        .and_then(|pair| pair.1)
        .unwrap_or("23:59")
}

fn task_primary_date_time(task: &Task) -> Option<(&str, Option<&str>)> {
    let due_date = task.due_date.as_str();
    let due_time = task.due_time.as_str();
    let rem_date = task.reminder_date.as_str();
    let rem_time = task.reminder_time.as_str();
    match (due_date.is_empty(), rem_date.is_empty()) {
        (true, true) => None,
        (false, true) => Some((
            due_date,
            if due_time.is_empty() {
                None
            } else {
                Some(due_time)
            },
        )),
        (true, false) => Some((
            rem_date,
            if rem_time.is_empty() {
                None
            } else {
                Some(rem_time)
            },
        )),
        (false, false) => {
            if due_date < rem_date {
                Some((
                    due_date,
                    if due_time.is_empty() {
                        None
                    } else {
                        Some(due_time)
                    },
                ))
            } else if rem_date < due_date {
                Some((
                    rem_date,
                    if rem_time.is_empty() {
                        None
                    } else {
                        Some(rem_time)
                    },
                ))
            } else if !due_time.is_empty() && !rem_time.is_empty() {
                if due_time <= rem_time {
                    Some((due_date, Some(due_time)))
                } else {
                    Some((rem_date, Some(rem_time)))
                }
            } else if !due_time.is_empty() {
                Some((due_date, Some(due_time)))
            } else if !rem_time.is_empty() {
                Some((rem_date, Some(rem_time)))
            } else {
                Some((due_date, None))
            }
        }
    }
}

fn push_count<const N: usize>(out: &mut TinyString<N>, mut value: u64) {
    if value == 0 {
        out.set("0");
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while value > 0 {
        i -= 1;
        buf[i] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    let len = buf.len() - i;
    let str = core::str::from_utf8(&buf[i..i + len]).unwrap_or("0");
    out.set(str);
}

#[no_mangle]
pub extern "C" fn _start(argc: u64, argv: *const *const u8, _envp: *const *const u8) -> ! {
    sunlight_libc::launch_trace::init_from_argv(argc, argv);
    let mut app = ReminderApp::new();
    let Some(mut window) = Window::connect(WindowConfig {
        width: WIN_W,
        height: WIN_H,
        title: "Sunlight Reminders & Tasks",
        decoration: WindowDecoration::Normal,
    }) else {
        debug_log("[REMINDERS] failed to open window\n");
        ProcessExit::exit(1)
    };
    window.run(&mut app);
    ProcessExit::exit(0);
}
