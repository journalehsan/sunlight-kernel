#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

pub const LIST_PREFIX: &str = "app.reminders.lists/";
pub const TASK_PREFIX: &str = "app.reminders.tasks/";
pub const INDEX_BY_DATE_PREFIX: &str = "app.reminders.index.by-date/";
pub const INDEX_REMINDER_DATE_PREFIX: &str = "app.reminders.index.reminder-date/";
pub const INDEX_ALL_KEY: &str = "app.reminders.index/all";
pub const SETTINGS_PREFIX: &str = "app.reminders.settings/";

pub const DEFAULT_LISTS: [(&str, &str); 3] = [
    ("inbox", "Inbox"),
    ("work", "Work"),
    ("personal", "Personal"),
];

pub const TITLE_LEN: usize = 96;
pub const NOTES_LEN: usize = 256;
pub const LIST_ID_LEN: usize = 16;
pub const LIST_NAME_LEN: usize = 32;
pub const DATE_LEN: usize = 10;
pub const TIME_LEN: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TinyString<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> TinyString<N> {
    pub const fn empty() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    pub fn try_set(&mut self, text: &str) -> bool {
        if text.len() > N {
            return false;
        }
        self.len = text.len();
        self.buf[..self.len].copy_from_slice(text.as_bytes());
        true
    }

    pub fn set(&mut self, text: &str) {
        let _ = self.try_set(text);
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn as_str(&self) -> &str {
        if self.len == 0 {
            return "";
        }
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

impl<const N: usize> Default for TinyString<N> {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Todo,
    Done,
}

impl TaskStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(Self::Todo),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskList {
    pub id: TinyString<LIST_ID_LEN>,
    pub name: TinyString<LIST_NAME_LEN>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl TaskList {
    pub fn new(id: &str, name: &str, created_at: u64, updated_at: u64) -> Option<Self> {
        let mut list = Self {
            id: TinyString::empty(),
            name: TinyString::empty(),
            created_at,
            updated_at,
        };
        if !list.id.try_set(id) || !list.name.try_set(name) {
            return None;
        }
        Some(list)
    }

    pub fn default_named(id: &str, now_ms: u64) -> Option<Self> {
        let name = default_list_name(id)?;
        Self::new(id, name, now_ms, now_ms)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: u64,
    pub title: TinyString<TITLE_LEN>,
    pub notes: TinyString<NOTES_LEN>,
    pub list_id: TinyString<LIST_ID_LEN>,
    pub status: TaskStatus,
    pub due_date: TinyString<DATE_LEN>,
    pub due_time: TinyString<TIME_LEN>,
    pub reminder_date: TinyString<DATE_LEN>,
    pub reminder_time: TinyString<TIME_LEN>,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Task {
    pub fn blank(id: u64, list_id: &str) -> Option<Self> {
        let mut task = Self {
            id,
            title: TinyString::empty(),
            notes: TinyString::empty(),
            list_id: TinyString::empty(),
            status: TaskStatus::Todo,
            due_date: TinyString::empty(),
            due_time: TinyString::empty(),
            reminder_date: TinyString::empty(),
            reminder_time: TinyString::empty(),
            created_at: 0,
            updated_at: 0,
        };
        if !task.list_id.try_set(list_id) {
            return None;
        }
        Some(task)
    }

    pub fn primary_date(&self) -> Option<&str> {
        if !self.due_date.is_empty() {
            Some(self.due_date.as_str())
        } else if !self.reminder_date.is_empty() {
            Some(self.reminder_date.as_str())
        } else {
            None
        }
    }

    pub fn primary_time(&self) -> Option<&str> {
        if !self.due_time.is_empty() {
            Some(self.due_time.as_str())
        } else if !self.reminder_time.is_empty() {
            Some(self.reminder_time.as_str())
        } else {
            None
        }
    }
}

pub fn list_key(list_id: &str) -> String {
    let mut out = String::from(LIST_PREFIX);
    out.push_str(list_id);
    out
}

pub fn task_key(task_id: u64) -> String {
    let mut out = String::from(TASK_PREFIX);
    push_u64_decimal(&mut out, task_id);
    out
}

pub fn date_index_key(date: &str, task_id: u64) -> String {
    let mut out = String::from(INDEX_BY_DATE_PREFIX);
    out.push_str(date);
    out.push('/');
    push_u64_decimal(&mut out, task_id);
    out
}

pub fn reminder_date_index_key(date: &str, task_id: u64) -> String {
    let mut out = String::from(INDEX_REMINDER_DATE_PREFIX);
    out.push_str(date);
    out.push('/');
    push_u64_decimal(&mut out, task_id);
    out
}

pub fn by_date_list_key(date: &str) -> String {
    let mut out = String::from(INDEX_BY_DATE_PREFIX);
    out.push_str(date);
    out
}

pub fn reminder_date_list_key(date: &str) -> String {
    let mut out = String::from(INDEX_REMINDER_DATE_PREFIX);
    out.push_str(date);
    out
}

pub fn settings_key(key: &str) -> String {
    let mut out = String::from(SETTINGS_PREFIX);
    out.push_str(key);
    out
}

pub fn encode_id_list(ids: &[u64]) -> Vec<u8> {
    let mut out = String::new();
    for id in ids {
        if !out.is_empty() {
            out.push('\n');
        }
        push_u64_decimal(&mut out, *id);
    }
    out.into_bytes()
}

pub fn parse_id_list(bytes: &[u8]) -> Option<Vec<u64>> {
    let text = core::str::from_utf8(bytes).ok()?;
    let mut ids = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(id) = parse_u64(line) {
            if !ids.iter().any(|existing| *existing == id) {
                ids.push(id);
            }
        }
    }
    Some(ids)
}

pub fn add_id_to_index_list(existing: Option<&[u8]>, task_id: u64) -> Vec<u8> {
    let mut ids = existing.and_then(parse_id_list).unwrap_or_default();
    if !ids.iter().any(|existing| *existing == task_id) {
        ids.push(task_id);
        ids.sort_unstable();
    }
    encode_id_list(&ids)
}

pub fn remove_id_from_index_list(existing: Option<&[u8]>, task_id: u64) -> Option<Vec<u8>> {
    let mut ids = existing.and_then(parse_id_list).unwrap_or_default();
    if ids.is_empty() {
        return None;
    }
    ids.retain(|id| *id != task_id);
    if ids.is_empty() {
        None
    } else {
        Some(encode_id_list(&ids))
    }
}

pub fn valid_date_str(text: &str) -> bool {
    parse_date_parts(text).is_some()
}

pub fn normalize_date_str(text: &str) -> Option<String> {
    let (year, month, day) = parse_date_parts(text)?;
    Some(format_date(year, month, day))
}

pub fn valid_time_str(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    let bytes = text.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return false;
    }
    for &idx in &[0usize, 1, 3, 4] {
        if !bytes[idx].is_ascii_digit() {
            return false;
        }
    }
    let Some(hour) = parse_u32(&text[0..2]) else {
        return false;
    };
    let Some(minute) = parse_u32(&text[3..5]) else {
        return false;
    };
    hour <= 23 && minute <= 59
}

pub fn format_date(year: i32, month: i32, day: i32) -> String {
    let mut out = String::new();
    push_i32_fixed(&mut out, year, 4);
    out.push('-');
    push_i32_fixed(&mut out, month, 2);
    out.push('-');
    push_i32_fixed(&mut out, day, 2);
    out
}

pub fn default_list_name(list_id: &str) -> Option<&'static str> {
    DEFAULT_LISTS
        .iter()
        .find_map(|(id, name)| if *id == list_id { Some(*name) } else { None })
}

pub fn is_supported_list_id(list_id: &str) -> bool {
    default_list_name(list_id).is_some()
}

pub fn encode_task(task: &Task) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"SRM1\n");
    push_u64_field(&mut out, "id", task.id);
    push_field(&mut out, "title", task.title.as_str());
    push_field(&mut out, "notes", task.notes.as_str());
    push_field(&mut out, "list_id", task.list_id.as_str());
    push_field(&mut out, "status", task.status.as_str());
    push_field(&mut out, "due_date", task.due_date.as_str());
    push_field(&mut out, "due_time", task.due_time.as_str());
    push_field(&mut out, "reminder_date", task.reminder_date.as_str());
    push_field(&mut out, "reminder_time", task.reminder_time.as_str());
    push_u64_field(&mut out, "created_at", task.created_at);
    push_u64_field(&mut out, "updated_at", task.updated_at);
    out
}

pub fn decode_task(bytes: &[u8]) -> Option<Task> {
    let text = core::str::from_utf8(bytes).ok()?;
    if !text.starts_with("SRM1\n") {
        return None;
    }

    let mut task = Task::blank(0, "inbox")?;
    task.status = TaskStatus::Todo;
    let mut seen_title = false;
    let mut seen_list = false;
    let mut seen_status = false;
    let mut seen_created = false;
    let mut seen_updated = false;

    for line in text.lines().skip(1) {
        let Some(eq) = line.find('=') else {
            continue;
        };
        let name = &line[..eq];
        let value = unescape_field(&line[eq + 1..]);
        match name {
            "id" => task.id = parse_u64(&value)?,
            "title" => {
                if !task.title.try_set(&value) || value.is_empty() {
                    return None;
                }
                seen_title = true;
            }
            "notes" => {
                if !value.is_empty() && !task.notes.try_set(&value) {
                    return None;
                }
            }
            "list_id" => {
                if !is_supported_list_id(&value) || !task.list_id.try_set(&value) {
                    return None;
                }
                seen_list = true;
            }
            "status" => {
                task.status = TaskStatus::parse(&value)?;
                seen_status = true;
            }
            "due_date" => {
                if !value.is_empty() && !valid_date_str(&value) {
                    return None;
                }
                if !task.due_date.try_set(&value) {
                    return None;
                }
            }
            "due_time" => {
                if !valid_time_str(&value) {
                    return None;
                }
                if !task.due_time.try_set(&value) {
                    return None;
                }
            }
            "reminder_date" => {
                if !value.is_empty() && !valid_date_str(&value) {
                    return None;
                }
                if !task.reminder_date.try_set(&value) {
                    return None;
                }
            }
            "reminder_time" => {
                if !valid_time_str(&value) {
                    return None;
                }
                if !task.reminder_time.try_set(&value) {
                    return None;
                }
            }
            "created_at" => {
                task.created_at = parse_u64(&value)?;
                seen_created = true;
            }
            "updated_at" => {
                task.updated_at = parse_u64(&value)?;
                seen_updated = true;
            }
            _ => {}
        }
    }

    if task.id == 0 || !seen_title || !seen_list || !seen_status || !seen_created || !seen_updated {
        return None;
    }

    if task.title.is_empty() || !is_supported_list_id(task.list_id.as_str()) {
        return None;
    }

    Some(task)
}

pub fn encode_list(list: &TaskList) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"SRL1\n");
    push_field(&mut out, "id", list.id.as_str());
    push_field(&mut out, "name", list.name.as_str());
    push_u64_field(&mut out, "created_at", list.created_at);
    push_u64_field(&mut out, "updated_at", list.updated_at);
    out
}

pub fn decode_list(bytes: &[u8]) -> Option<TaskList> {
    let text = core::str::from_utf8(bytes).ok()?;
    if !text.starts_with("SRL1\n") {
        return None;
    }

    let mut list = TaskList {
        id: TinyString::empty(),
        name: TinyString::empty(),
        created_at: 0,
        updated_at: 0,
    };
    let mut seen_id = false;
    let mut seen_name = false;
    let mut seen_created = false;
    let mut seen_updated = false;

    for line in text.lines().skip(1) {
        let Some(eq) = line.find('=') else {
            continue;
        };
        let name = &line[..eq];
        let value = unescape_field(&line[eq + 1..]);
        match name {
            "id" => {
                if !is_supported_list_id(&value) || !list.id.try_set(&value) {
                    return None;
                }
                seen_id = true;
            }
            "name" => {
                if !list.name.try_set(&value) || value.is_empty() {
                    return None;
                }
                seen_name = true;
            }
            "created_at" => {
                list.created_at = parse_u64(&value)?;
                seen_created = true;
            }
            "updated_at" => {
                list.updated_at = parse_u64(&value)?;
                seen_updated = true;
            }
            _ => {}
        }
    }

    if !seen_id || !seen_name || !seen_created || !seen_updated {
        return None;
    }
    Some(list)
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

fn push_u64_field(out: &mut Vec<u8>, name: &str, value: u64) {
    out.extend_from_slice(name.as_bytes());
    out.push(b'=');
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    let mut n = value;
    if n == 0 {
        out.push(b'0');
    } else {
        while n > 0 {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        out.extend_from_slice(&buf[i..]);
    }
    out.push(b'\n');
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

fn parse_date_parts(text: &str) -> Option<(i32, i32, i32)> {
    if text.len() != 10 {
        return None;
    }
    let bytes = text.as_bytes();
    let sep = bytes[4];
    if (sep != b'-' && sep != b'/') || bytes[7] != sep {
        return None;
    }
    let year = parse_i32(&text[0..4]);
    let month = parse_i32(&text[5..7]);
    let day = parse_i32(&text[8..10]);
    if month >= 1 && month <= 12 && day >= 1 && day <= days_in_month(year, month) {
        Some((year, month, day))
    } else {
        None
    }
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

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn parse_i32(text: &str) -> i32 {
    let mut value = 0i32;
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            value = value * 10 + (ch as i32 - '0' as i32);
        }
    }
    value
}

fn parse_u32(text: &str) -> Option<u32> {
    if text.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for ch in text.chars() {
        if !ch.is_ascii_digit() {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add((ch as u32).saturating_sub('0' as u32))?;
    }
    Some(value)
}

pub fn parse_u64(text: &str) -> Option<u64> {
    if text.is_empty() {
        return None;
    }
    let mut value = 0u64;
    for ch in text.chars() {
        if !ch.is_ascii_digit() {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add((ch as u64).saturating_sub('0' as u64))?;
    }
    Some(value)
}

pub fn push_u64_decimal(out: &mut String, mut value: u64) {
    if value == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    while value > 0 {
        i -= 1;
        buf[i] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    for &byte in &buf[i..] {
        out.push(byte as char);
    }
}

fn push_i32_fixed(out: &mut String, mut value: i32, digits: usize) {
    if value < 0 {
        out.push('-');
        value = -value;
    }
    let mut buf = [0u8; 12];
    let mut i = buf.len();
    while value > 0 {
        i -= 1;
        buf[i] = b'0' + (value % 10) as u8;
        value /= 10;
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
    for &byte in &buf[i..] {
        out.push(byte as char);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn stable_namespace_keys_are_readable() {
        assert_eq!(list_key("inbox"), "app.reminders.lists/inbox");
        assert_eq!(task_key(42), "app.reminders.tasks/42");
        assert_eq!(
            date_index_key("2026-07-08", 42),
            "app.reminders.index.by-date/2026-07-08/42"
        );
        assert_eq!(
            reminder_date_index_key("2026-07-08", 42),
            "app.reminders.index.reminder-date/2026-07-08/42"
        );
        assert_eq!(
            by_date_list_key("2026-07-08"),
            "app.reminders.index.by-date/2026-07-08"
        );
        assert_eq!(
            reminder_date_list_key("2026-07-08"),
            "app.reminders.index.reminder-date/2026-07-08"
        );
        assert_eq!(
            settings_key("selected-view"),
            "app.reminders.settings/selected-view"
        );
    }

    #[test]
    fn id_index_round_trips_and_ignores_malformed_lines() {
        let encoded = encode_id_list(&[7, 9, 7]);
        assert_eq!(core::str::from_utf8(&encoded).unwrap(), "7\n9\n7");
        assert_eq!(parse_id_list(b"7\nbad\n9\n7\n").unwrap(), vec![7, 9]);
    }

    #[test]
    fn date_and_time_validation_is_strict() {
        assert!(valid_date_str("2026-07-08"));
        assert!(!valid_date_str("2026/07/08"));
        assert!(!valid_date_str("2026-13-01"));
        assert!(valid_time_str(""));
        assert!(valid_time_str("09:05"));
        assert!(!valid_time_str("9:05"));
        assert!(!valid_time_str("24:00"));
    }

    #[test]
    fn task_record_round_trips() {
        let mut task = Task::blank(42, "work").unwrap();
        task.title.set("Write notes");
        task.notes.set("pack the update");
        task.status = TaskStatus::Todo;
        task.due_date.set("2026-07-08");
        task.due_time.set("09:30");
        task.reminder_date.set("2026-07-07");
        task.reminder_time.set("18:15");
        task.created_at = 123;
        task.updated_at = 456;

        let bytes = encode_task(&task);
        let decoded = decode_task(&bytes).unwrap();
        assert_eq!(decoded.id, 42);
        assert_eq!(decoded.title.as_str(), "Write notes");
        assert_eq!(decoded.notes.as_str(), "pack the update");
        assert_eq!(decoded.list_id.as_str(), "work");
        assert_eq!(decoded.status, TaskStatus::Todo);
        assert_eq!(decoded.due_date.as_str(), "2026-07-08");
        assert_eq!(decoded.due_time.as_str(), "09:30");
        assert_eq!(decoded.reminder_date.as_str(), "2026-07-07");
        assert_eq!(decoded.reminder_time.as_str(), "18:15");
        assert_eq!(decoded.created_at, 123);
        assert_eq!(decoded.updated_at, 456);
    }

    #[test]
    fn malformed_records_are_skipped() {
        assert!(decode_task(b"bad").is_none());
        assert!(decode_list(b"bad").is_none());
        let malformed_task = b"SRM1\nid=1\nstatus=todo\ncreated_at=1\nupdated_at=1\n";
        assert!(decode_task(malformed_task).is_none());
    }

    #[test]
    fn list_round_trips() {
        let list = TaskList::new("personal", "Personal", 1, 2).unwrap();
        let bytes = encode_list(&list);
        let decoded = decode_list(&bytes).unwrap();
        assert_eq!(decoded.id.as_str(), "personal");
        assert_eq!(decoded.name.as_str(), "Personal");
        assert_eq!(decoded.created_at, 1);
        assert_eq!(decoded.updated_at, 2);
    }

    #[test]
    fn by_date_and_reminder_date_indexes_are_distinct_and_stable() {
        assert_eq!(
            by_date_list_key("2026-07-09"),
            "app.reminders.index.by-date/2026-07-09"
        );
        assert_eq!(
            reminder_date_list_key("2026-07-09"),
            "app.reminders.index.reminder-date/2026-07-09"
        );
        // markers use /id suffix
        assert!(date_index_key("2026-07-09", 7).ends_with("/7"));
        assert!(reminder_date_index_key("2026-07-09", 7)
            .starts_with("app.reminders.index.reminder-date/"));
    }

    #[test]
    fn id_lists_roundtrip_for_date_indexes() {
        let ids = vec![3u64, 1, 99];
        let enc = encode_id_list(&ids);
        let parsed = parse_id_list(&enc).unwrap();
        assert_eq!(parsed, vec![3, 1, 99]);
    }

    #[test]
    fn by_date_index_list_is_written_on_create() {
        let encoded = add_id_to_index_list(None, 42);
        assert_eq!(parse_id_list(&encoded).unwrap(), vec![42]);
    }

    #[test]
    fn old_index_entry_is_removed_when_due_date_changes() {
        let old_date = add_id_to_index_list(None, 42);
        assert_eq!(remove_id_from_index_list(Some(&old_date), 42), None);

        let new_date = add_id_to_index_list(None, 42);
        assert_eq!(parse_id_list(&new_date).unwrap(), vec![42]);
    }

    #[test]
    fn reminder_date_index_is_written_when_present() {
        let encoded = add_id_to_index_list(None, 7);
        assert_eq!(parse_id_list(&encoded).unwrap(), vec![7]);
    }

    #[test]
    fn malformed_records_are_skipped_and_dont_panic() {
        assert!(decode_task(b"").is_none());
        assert!(decode_task(b"SRM1\nid=abc\n").is_none());
        assert!(decode_list(b"SRL1\nid=foo").is_none());
    }
}
