#![no_std]

extern crate alloc;

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

pub const HISTORY_LIMIT: usize = 32;
pub const MAX_TEXT_BYTES: usize = 2048;
pub const MAX_FILE_LIST_BYTES: usize = 2048;
pub const MAX_BINARY_BYTES: usize = 1024;
pub const MAX_SUMMARY_BYTES: usize = 64;
pub const WIRE_MAGIC_ITEM: u32 = 0x434C_4950;
pub const WIRE_MAGIC_SET: u32 = 0x4353_4554;
pub const WIRE_MAGIC_LIST: u32 = 0x434C_5354;
pub const WIRE_MAGIC_HISTORY: u32 = 0x4348_4953;
pub const WIRE_MAGIC_CURRENT: u32 = 0x4343_5552;
pub const WIRE_VERSION: u16 = 1;

pub const KV_KEY_CURRENT: &str = "clip/curr";
pub const KV_KEY_HISTORY: &str = "clip/hist";

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardKind {
    Text = 1,
    FileList = 2,
    Binary = 3,
}

impl ClipboardKind {
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Text),
            2 => Some(Self::FileList),
            3 => Some(Self::Binary),
            _ => None,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::FileList => "files",
            Self::Binary => "binary",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardItem {
    pub id: u32,
    pub kind: ClipboardKind,
    pub mime: String,
    pub created_at_ms: u64,
    pub payload: Vec<u8>,
    pub source_app: Option<String>,
}

impl ClipboardItem {
    pub fn size(&self) -> u32 {
        self.payload.len() as u32
    }

    pub fn summary(&self) -> String {
        match self.kind {
            ClipboardKind::Text => sanitize_summary(bytes_to_lossy(&self.payload).as_ref()),
            ClipboardKind::FileList => summarize_file_list(&self.payload),
            ClipboardKind::Binary => format!("{} bytes ({})", self.payload.len(), self.mime),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardSummary {
    pub id: u32,
    pub kind: ClipboardKind,
    pub mime: String,
    pub created_at_ms: u64,
    pub summary: String,
    pub size: u32,
    pub is_current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardSetRequest {
    pub kind: ClipboardKind,
    pub mime: String,
    pub payload: Vec<u8>,
    pub source_app: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardState {
    history: Vec<ClipboardItem>,
    current_id: Option<u32>,
    next_id: u32,
}

impl ClipboardState {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            current_id: None,
            next_id: 1,
        }
    }

    pub fn from_persisted(
        current_id: Option<u32>,
        next_id: u32,
        history: Vec<ClipboardItem>,
    ) -> Self {
        let mut state = Self {
            history,
            current_id,
            next_id: if next_id == 0 { 1 } else { next_id },
        };
        state.compact();
        state
    }

    pub fn current(&self) -> Option<&ClipboardItem> {
        let current_id = self.current_id?;
        self.history.iter().find(|item| item.id == current_id)
    }

    pub fn history(&self) -> &[ClipboardItem] {
        &self.history
    }

    pub fn current_id(&self) -> Option<u32> {
        self.current_id
    }

    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    pub fn summaries(&self) -> Vec<ClipboardSummary> {
        self.history
            .iter()
            .map(|item| ClipboardSummary {
                id: item.id,
                kind: item.kind,
                mime: item.mime.clone(),
                created_at_ms: item.created_at_ms,
                summary: item.summary(),
                size: item.size(),
                is_current: self.current_id == Some(item.id),
            })
            .collect()
    }

    pub fn set_item(
        &mut self,
        request: ClipboardSetRequest,
        created_at_ms: u64,
    ) -> Result<SetOutcome, ClipError> {
        validate_request(&request)?;

        if let Some(current_id) = self.current().and_then(|current| {
            (current.kind == request.kind
                && current.mime == request.mime
                && current.payload == request.payload)
                .then_some(current.id)
        }) {
            self.move_to_front_by_id(current_id);
            self.current_id = Some(current_id);
            return Ok(SetOutcome {
                current_id,
                evicted_ids: Vec::new(),
            });
        }

        let id = self.alloc_id();
        self.history.insert(
            0,
            ClipboardItem {
                id,
                kind: request.kind,
                mime: request.mime,
                created_at_ms,
                payload: request.payload,
                source_app: request.source_app,
            },
        );
        self.current_id = Some(id);
        let evicted_ids = self.trim_history();
        Ok(SetOutcome {
            current_id: id,
            evicted_ids,
        })
    }

    pub fn select_by_index(&mut self, index: usize) -> Result<u32, ClipError> {
        if index >= self.history.len() {
            return Err(ClipError::NotFound);
        }
        let id = self.history[index].id;
        self.move_to_front_by_id(id);
        self.current_id = Some(id);
        Ok(id)
    }

    pub fn select_by_id(&mut self, id: u32) -> Result<u32, ClipError> {
        if self.history.iter().any(|item| item.id == id) {
            self.move_to_front_by_id(id);
            self.current_id = Some(id);
            Ok(id)
        } else {
            Err(ClipError::NotFound)
        }
    }

    pub fn clear_current(&mut self) {
        self.current_id = None;
    }

    pub fn clear_history(&mut self) -> Vec<u32> {
        let ids = self.history.iter().map(|item| item.id).collect();
        self.history.clear();
        self.current_id = None;
        ids
    }

    fn alloc_id(&mut self) -> u32 {
        let id = if self.next_id == 0 { 1 } else { self.next_id };
        self.next_id = id.wrapping_add(1);
        if self.next_id == 0 {
            self.next_id = 1;
        }
        id
    }

    fn move_to_front_by_id(&mut self, id: u32) {
        if let Some(pos) = self.history.iter().position(|item| item.id == id) {
            if pos != 0 {
                let item = self.history.remove(pos);
                self.history.insert(0, item);
            }
        }
    }

    fn trim_history(&mut self) -> Vec<u32> {
        let mut evicted = Vec::new();
        while self.history.len() > HISTORY_LIMIT {
            if let Some(item) = self.history.pop() {
                evicted.push(item.id);
            }
        }
        evicted
    }

    fn compact(&mut self) {
        let mut unique = Vec::new();
        for item in self.history.drain(..) {
            if unique
                .iter()
                .any(|existing: &ClipboardItem| existing.id == item.id)
            {
                continue;
            }
            unique.push(item);
            if unique.len() >= HISTORY_LIMIT {
                break;
            }
        }
        self.history = unique;
        if self.current_id.is_some() && self.current().is_none() {
            self.current_id = None;
        }
    }
}

pub struct SetOutcome {
    pub current_id: u32,
    pub evicted_ids: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipError {
    BadRequest,
    NotFound,
    TooLarge,
    Unsupported,
    Corrupt,
    Internal,
}

impl ClipError {
    pub const fn code(self) -> u64 {
        match self {
            Self::BadRequest => sunlight_ipc::ClipMsg::ERR_BAD_REQUEST,
            Self::NotFound => sunlight_ipc::ClipMsg::ERR_NOT_FOUND,
            Self::TooLarge => sunlight_ipc::ClipMsg::ERR_TOO_LARGE,
            Self::Unsupported => sunlight_ipc::ClipMsg::ERR_UNSUPPORTED,
            Self::Corrupt => sunlight_ipc::ClipMsg::ERR_CORRUPT,
            Self::Internal => sunlight_ipc::ClipMsg::ERR_INTERNAL,
        }
    }
}

pub fn item_key(id: u32) -> String {
    format!("clip/i/{:08x}", id)
}

pub fn encode_current(current_id: Option<u32>) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, WIRE_MAGIC_CURRENT);
    push_u16(&mut out, WIRE_VERSION);
    push_u16(&mut out, 0);
    push_u32(&mut out, current_id.unwrap_or(0));
    out
}

pub fn decode_current(bytes: &[u8]) -> Result<Option<u32>, ClipError> {
    let mut index = 0usize;
    if take_u32(bytes, &mut index) != Some(WIRE_MAGIC_CURRENT) {
        return Err(ClipError::Corrupt);
    }
    if take_u16(bytes, &mut index) != Some(WIRE_VERSION) {
        return Err(ClipError::Corrupt);
    }
    let _ = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)?;
    let current = take_u32(bytes, &mut index).ok_or(ClipError::Corrupt)?;
    if current == 0 {
        Ok(None)
    } else {
        Ok(Some(current))
    }
}

pub fn encode_history_state(next_id: u32, ids: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, WIRE_MAGIC_HISTORY);
    push_u16(&mut out, WIRE_VERSION);
    push_u16(&mut out, ids.len() as u16);
    push_u32(&mut out, next_id);
    for id in ids {
        push_u32(&mut out, *id);
    }
    out
}

pub fn decode_history_state(bytes: &[u8]) -> Result<(u32, Vec<u32>), ClipError> {
    let mut index = 0usize;
    if take_u32(bytes, &mut index) != Some(WIRE_MAGIC_HISTORY) {
        return Err(ClipError::Corrupt);
    }
    if take_u16(bytes, &mut index) != Some(WIRE_VERSION) {
        return Err(ClipError::Corrupt);
    }
    let count = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
    if count > HISTORY_LIMIT {
        return Err(ClipError::Corrupt);
    }
    let next_id = take_u32(bytes, &mut index).ok_or(ClipError::Corrupt)?;
    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(take_u32(bytes, &mut index).ok_or(ClipError::Corrupt)?);
    }
    Ok((if next_id == 0 { 1 } else { next_id }, ids))
}

pub fn encode_item(item: &ClipboardItem) -> Vec<u8> {
    let mut out = Vec::new();
    let flags = if item.source_app.is_some() { 1u8 } else { 0u8 };
    let source = item.source_app.as_deref().unwrap_or("");
    push_u32(&mut out, WIRE_MAGIC_ITEM);
    push_u16(&mut out, WIRE_VERSION);
    out.push(item.kind as u8);
    out.push(flags);
    push_u32(&mut out, item.id);
    push_u64(&mut out, item.created_at_ms);
    push_u32(&mut out, item.payload.len() as u32);
    push_u16(&mut out, item.mime.len() as u16);
    push_u16(&mut out, source.len() as u16);
    out.extend_from_slice(item.mime.as_bytes());
    out.extend_from_slice(source.as_bytes());
    out.extend_from_slice(&item.payload);
    out
}

pub fn decode_item(bytes: &[u8]) -> Result<ClipboardItem, ClipError> {
    let mut index = 0usize;
    if take_u32(bytes, &mut index) != Some(WIRE_MAGIC_ITEM) {
        return Err(ClipError::Corrupt);
    }
    if take_u16(bytes, &mut index) != Some(WIRE_VERSION) {
        return Err(ClipError::Corrupt);
    }
    let kind = ClipboardKind::from_u8(take_u8(bytes, &mut index).ok_or(ClipError::Corrupt)?)
        .ok_or(ClipError::Corrupt)?;
    let flags = take_u8(bytes, &mut index).ok_or(ClipError::Corrupt)?;
    let id = take_u32(bytes, &mut index).ok_or(ClipError::Corrupt)?;
    let created_at_ms = take_u64(bytes, &mut index).ok_or(ClipError::Corrupt)?;
    let payload_len = take_u32(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
    let mime_len = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
    let source_len = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
    let mime = take_string(bytes, &mut index, mime_len)?;
    let source_app = if (flags & 1) != 0 && source_len > 0 {
        Some(take_string(bytes, &mut index, source_len)?)
    } else {
        let _ = take_vec(bytes, &mut index, source_len)?;
        None
    };
    let payload = take_vec(bytes, &mut index, payload_len)?;
    Ok(ClipboardItem {
        id,
        kind,
        mime,
        created_at_ms,
        payload,
        source_app,
    })
}

pub fn encode_set_request(request: &ClipboardSetRequest) -> Vec<u8> {
    let mut out = Vec::new();
    let flags = if request.source_app.is_some() {
        1u8
    } else {
        0u8
    };
    let source = request.source_app.as_deref().unwrap_or("");
    push_u32(&mut out, WIRE_MAGIC_SET);
    push_u16(&mut out, WIRE_VERSION);
    out.push(request.kind as u8);
    out.push(flags);
    push_u16(&mut out, request.mime.len() as u16);
    push_u16(&mut out, source.len() as u16);
    push_u32(&mut out, request.payload.len() as u32);
    out.extend_from_slice(request.mime.as_bytes());
    out.extend_from_slice(source.as_bytes());
    out.extend_from_slice(&request.payload);
    out
}

pub fn decode_set_request(bytes: &[u8]) -> Result<ClipboardSetRequest, ClipError> {
    let mut index = 0usize;
    if take_u32(bytes, &mut index) != Some(WIRE_MAGIC_SET) {
        return Err(ClipError::Corrupt);
    }
    if take_u16(bytes, &mut index) != Some(WIRE_VERSION) {
        return Err(ClipError::Corrupt);
    }
    let kind = ClipboardKind::from_u8(take_u8(bytes, &mut index).ok_or(ClipError::Corrupt)?)
        .ok_or(ClipError::Corrupt)?;
    let flags = take_u8(bytes, &mut index).ok_or(ClipError::Corrupt)?;
    let mime_len = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
    let source_len = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
    let payload_len = take_u32(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
    let mime = take_string(bytes, &mut index, mime_len)?;
    let source_app = if (flags & 1) != 0 && source_len > 0 {
        Some(take_string(bytes, &mut index, source_len)?)
    } else {
        let _ = take_vec(bytes, &mut index, source_len)?;
        None
    };
    let payload = take_vec(bytes, &mut index, payload_len)?;
    Ok(ClipboardSetRequest {
        kind,
        mime,
        payload,
        source_app,
    })
}

pub fn encode_summary_list(list: &[ClipboardSummary]) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, WIRE_MAGIC_LIST);
    push_u16(&mut out, WIRE_VERSION);
    push_u16(&mut out, list.len() as u16);
    for item in list {
        push_u32(&mut out, item.id);
        out.push(item.kind as u8);
        out.push(if item.is_current { 1 } else { 0 });
        push_u16(&mut out, item.mime.len() as u16);
        push_u16(&mut out, item.summary.len() as u16);
        push_u32(&mut out, item.size);
        push_u64(&mut out, item.created_at_ms);
        out.extend_from_slice(item.mime.as_bytes());
        out.extend_from_slice(item.summary.as_bytes());
    }
    out
}

pub fn decode_summary_list(bytes: &[u8]) -> Result<Vec<ClipboardSummary>, ClipError> {
    let mut index = 0usize;
    if take_u32(bytes, &mut index) != Some(WIRE_MAGIC_LIST) {
        return Err(ClipError::Corrupt);
    }
    if take_u16(bytes, &mut index) != Some(WIRE_VERSION) {
        return Err(ClipError::Corrupt);
    }
    let count = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
    if count > HISTORY_LIMIT {
        return Err(ClipError::Corrupt);
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let id = take_u32(bytes, &mut index).ok_or(ClipError::Corrupt)?;
        let kind = ClipboardKind::from_u8(take_u8(bytes, &mut index).ok_or(ClipError::Corrupt)?)
            .ok_or(ClipError::Corrupt)?;
        let current = take_u8(bytes, &mut index).ok_or(ClipError::Corrupt)? == 1;
        let mime_len = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
        let summary_len = take_u16(bytes, &mut index).ok_or(ClipError::Corrupt)? as usize;
        let size = take_u32(bytes, &mut index).ok_or(ClipError::Corrupt)?;
        let created_at_ms = take_u64(bytes, &mut index).ok_or(ClipError::Corrupt)?;
        let mime = take_string(bytes, &mut index, mime_len)?;
        let summary = take_string(bytes, &mut index, summary_len)?;
        out.push(ClipboardSummary {
            id,
            kind,
            mime,
            created_at_ms,
            summary,
            size,
            is_current: current,
        });
    }
    Ok(out)
}

pub fn encode_file_list(paths: &[&str]) -> Vec<u8> {
    let mut out = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        if index > 0 {
            out.push(0);
        }
        out.extend_from_slice(path.as_bytes());
    }
    out
}

pub fn decode_file_list(bytes: &[u8]) -> Result<Vec<String>, ClipError> {
    let text = core::str::from_utf8(bytes).map_err(|_| ClipError::Corrupt)?;
    Ok(text
        .split('\0')
        .filter(|part| !part.is_empty())
        .map(ToString::to_string)
        .collect())
}

pub fn summarize_file_list(bytes: &[u8]) -> String {
    match decode_file_list(bytes) {
        Ok(paths) if paths.is_empty() => "(no paths)".to_string(),
        Ok(paths) => {
            let first = sanitize_summary(&paths[0]);
            if paths.len() == 1 {
                first
            } else {
                format!("{} (+{} more)", first, paths.len() - 1)
            }
        }
        Err(_) => "(invalid file list)".to_string(),
    }
}

pub fn bytes_to_lossy(bytes: &[u8]) -> Cow<'_, str> {
    match core::str::from_utf8(bytes) {
        Ok(text) => Cow::Borrowed(text),
        Err(_) => String::from_utf8_lossy(bytes),
    }
}

pub fn sanitize_summary(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() >= MAX_SUMMARY_BYTES {
            break;
        }
        out.push(if ch.is_control() { ' ' } else { ch });
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        "(empty)".to_string()
    } else if trimmed.len() == text.len() {
        trimmed.to_string()
    } else {
        let mut summary = trimmed.to_string();
        if summary.len() < MAX_SUMMARY_BYTES {
            summary.push('…');
        }
        summary
    }
}

fn validate_request(request: &ClipboardSetRequest) -> Result<(), ClipError> {
    if request.mime.is_empty() {
        return Err(ClipError::BadRequest);
    }
    match request.kind {
        ClipboardKind::Text => {
            if request.mime != "text/plain" {
                return Err(ClipError::BadRequest);
            }
            if request.payload.len() > MAX_TEXT_BYTES {
                return Err(ClipError::TooLarge);
            }
            if core::str::from_utf8(&request.payload).is_err() {
                return Err(ClipError::BadRequest);
            }
        }
        ClipboardKind::FileList => {
            if request.mime != "x-sunlight/file-list" {
                return Err(ClipError::BadRequest);
            }
            if request.payload.len() > MAX_FILE_LIST_BYTES {
                return Err(ClipError::TooLarge);
            }
            if decode_file_list(&request.payload)?.is_empty() {
                return Err(ClipError::BadRequest);
            }
        }
        ClipboardKind::Binary => {
            if request.payload.len() > MAX_BINARY_BYTES {
                return Err(ClipError::TooLarge);
            }
            return Err(ClipError::Unsupported);
        }
    }
    Ok(())
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn take_u8(bytes: &[u8], index: &mut usize) -> Option<u8> {
    let value = *bytes.get(*index)?;
    *index += 1;
    Some(value)
}

fn take_u16(bytes: &[u8], index: &mut usize) -> Option<u16> {
    if *index + 2 > bytes.len() {
        return None;
    }
    let mut raw = [0u8; 2];
    raw.copy_from_slice(&bytes[*index..*index + 2]);
    *index += 2;
    Some(u16::from_le_bytes(raw))
}

fn take_u32(bytes: &[u8], index: &mut usize) -> Option<u32> {
    if *index + 4 > bytes.len() {
        return None;
    }
    let mut raw = [0u8; 4];
    raw.copy_from_slice(&bytes[*index..*index + 4]);
    *index += 4;
    Some(u32::from_le_bytes(raw))
}

fn take_u64(bytes: &[u8], index: &mut usize) -> Option<u64> {
    if *index + 8 > bytes.len() {
        return None;
    }
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&bytes[*index..*index + 8]);
    *index += 8;
    Some(u64::from_le_bytes(raw))
}

fn take_string(bytes: &[u8], index: &mut usize, len: usize) -> Result<String, ClipError> {
    String::from_utf8(take_vec(bytes, index, len)?).map_err(|_| ClipError::Corrupt)
}

fn take_vec(bytes: &[u8], index: &mut usize, len: usize) -> Result<Vec<u8>, ClipError> {
    if *index + len > bytes.len() {
        return Err(ClipError::Corrupt);
    }
    let out = bytes[*index..*index + len].to_vec();
    *index += len;
    Ok(out)
}
