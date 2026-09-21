//! Clipboard access shared by text widgets, Files, and the desktop.

use alloc::{string::String, vec::Vec};
use sunlight_ipc::{
    ipc_call, nameserver_lookup_timeout, shm_alloc, shm_free, shm_map, CapabilityToken, ClipMsg,
    IpcMsg, SHM_PAGE,
};

const WIRE_MAGIC_SET: u32 = 0x4353_4554;
const WIRE_MAGIC_ITEM: u32 = 0x434C_4950;
const WIRE_VERSION: u16 = 1;
const KIND_TEXT: u8 = 1;
const KIND_FILES: u8 = 2;
const MIME_FILES: &[u8] = b"x-sunlight/file-list";
const MIME_CUT: &[u8] = b"x-sunlight/file-list;operation=cut";
const MIME_TEXT: &[u8] = b"text/plain";
const DEFAULT_SOURCE: &[u8] = b"sunlight-ui";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    Unavailable,
    Empty,
    TooLarge,
    Unsupported,
    Invalid,
}

impl ClipboardError {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Unavailable => "Clipboard service unavailable",
            Self::Empty => "Clipboard is empty",
            Self::TooLarge => "Clipboard payload is too large",
            Self::Unsupported => "Clipboard does not contain text",
            Self::Invalid => "Invalid clipboard item",
        }
    }
}

pub fn set_text(text: &str) -> Result<(), ClipboardError> {
    set_text_from(DEFAULT_SOURCE, text)
}

pub fn set_text_from(source_app: &[u8], text: &str) -> Result<(), ClipboardError> {
    set_item(source_app, KIND_TEXT, MIME_TEXT, text.as_bytes(), 0)
}

fn set_item(
    source_app: &[u8],
    kind: u8,
    mime: &[u8],
    payload: &[u8],
    expected_id: u32,
) -> Result<(), ClipboardError> {
    let cap = ensure_service().ok_or(ClipboardError::Unavailable)?;
    let source_app = if source_app.is_empty() {
        DEFAULT_SOURCE
    } else {
        source_app
    };
    let total_len = 16 + mime.len() + source_app.len() + payload.len();
    // Stored item headers are 12 bytes longer than SET headers.
    if total_len > SHM_PAGE - 12
        || mime.len() > u16::MAX as usize
        || source_app.len() > u16::MAX as usize
        || payload.len() > u32::MAX as usize
    {
        return Err(ClipboardError::TooLarge);
    }

    let (ptr, token) = shm_alloc().map_err(|_| ClipboardError::Unavailable)?;
    unsafe {
        let buf = core::slice::from_raw_parts_mut(ptr, SHM_PAGE);
        let mut index = 0usize;
        index += put_u32(&mut buf[index..], WIRE_MAGIC_SET);
        index += put_u16(&mut buf[index..], WIRE_VERSION);
        buf[index] = kind;
        index += 1;
        buf[index] = 1;
        index += 1;
        index += put_u16(&mut buf[index..], mime.len() as u16);
        index += put_u16(&mut buf[index..], source_app.len() as u16);
        index += put_u32(&mut buf[index..], payload.len() as u32);
        index += put_bytes(&mut buf[index..], mime);
        index += put_bytes(&mut buf[index..], source_app);
        let _ = put_bytes(&mut buf[index..], payload);
    }

    let reply = ipc_call(
        cap,
        IpcMsg::with_label(ClipMsg::SET_CLIPBOARD)
            .word(0, total_len as u64)
            .word(1, expected_id as u64)
            .with_cap(0, token),
    );
    let _ = shm_free(token);
    if reply.label == ClipMsg::ERROR {
        return Err(from_service_error(reply.words[0]));
    }
    Ok(())
}

pub fn get_text() -> Result<String, ClipboardError> {
    let page = get_item()?;
    let payload = parse_text_item(&page)?;
    let text = core::str::from_utf8(payload).map_err(|_| ClipboardError::Invalid)?;
    Ok(String::from(text))
}

fn get_item() -> Result<Vec<u8>, ClipboardError> {
    let cap = ensure_service().ok_or(ClipboardError::Unavailable)?;
    let reply = ipc_call(cap, IpcMsg::with_label(ClipMsg::GET_CLIPBOARD));
    if reply.label == ClipMsg::ERROR {
        return Err(from_service_error(reply.words[0]));
    }

    let len = reply.words[1] as usize;
    let token = reply.caps[0];
    if len == 0 || token == CapabilityToken::INVALID {
        return Err(ClipboardError::Empty);
    }
    if len > SHM_PAGE {
        let _ = shm_free(token);
        return Err(ClipboardError::Invalid);
    }

    let ptr = match shm_map(token) {
        Ok(ptr) => ptr,
        Err(_) => {
            let _ = shm_free(token);
            return Err(ClipboardError::Invalid);
        }
    };
    let mut page = [0u8; SHM_PAGE];
    unsafe {
        core::ptr::copy_nonoverlapping(ptr, page.as_mut_ptr(), len);
    }
    let _ = shm_free(token);
    Ok(page[..len].to_vec())
}

/// File-list payloads remain NUL-separated UTF-8 paths, compatible with sunlight-clip.
#[derive(Debug, PartialEq, Eq)]
pub struct FileClipboard {
    pub id: u32,
    pub cut: bool,
    pub paths: Vec<String>,
}

pub fn set_files(source_app: &[u8], paths: &[String], cut: bool) -> Result<(), ClipboardError> {
    set_files_if_current(source_app, paths, cut, 0)
}

fn set_files_if_current(
    source_app: &[u8],
    paths: &[String],
    cut: bool,
    expected_id: u32,
) -> Result<(), ClipboardError> {
    if paths.is_empty() {
        return Err(ClipboardError::Empty);
    }
    let mut payload = Vec::new();
    for path in paths {
        if !path.starts_with('/') || path.as_bytes().contains(&0) {
            return Err(ClipboardError::Invalid);
        }
        if payload.len() + path.len() + 1 > SHM_PAGE {
            return Err(ClipboardError::TooLarge);
        }
        payload.extend_from_slice(path.as_bytes());
        payload.push(0);
    }
    set_item(
        source_app,
        KIND_FILES,
        if cut { MIME_CUT } else { MIME_FILES },
        &payload,
        expected_id,
    )
}

pub fn get_files() -> Result<FileClipboard, ClipboardError> {
    parse_files_item(&get_item()?)
}

/// Consume only completed cut entries. A newer clipboard value is never changed.
pub fn finish_cut(
    item: &FileClipboard,
    completed: usize,
    source_app: &[u8],
) -> Result<(), ClipboardError> {
    if !item.cut || completed == 0 {
        return Ok(());
    }
    let remaining = item.paths.get(completed..).ok_or(ClipboardError::Invalid)?;
    if !remaining.is_empty() {
        return set_files_if_current(source_app, remaining, true, item.id);
    }
    let cap = ensure_service().ok_or(ClipboardError::Unavailable)?;
    let reply = ipc_call(
        cap,
        IpcMsg::with_label(ClipMsg::CLEAR_CLIPBOARD).word(0, item.id as u64),
    );
    if reply.label == ClipMsg::ERROR {
        return Err(from_service_error(reply.words[0]));
    }
    Ok(())
}

fn parse_files_item(bytes: &[u8]) -> Result<FileClipboard, ClipboardError> {
    let mut i = 0;
    if take_u32(bytes, &mut i) != Some(WIRE_MAGIC_ITEM)
        || take_u16(bytes, &mut i) != Some(WIRE_VERSION)
    {
        return Err(ClipboardError::Invalid);
    }
    if take_u8(bytes, &mut i) != Some(KIND_FILES) {
        return Err(ClipboardError::Unsupported);
    }
    let _flags = take_u8(bytes, &mut i).ok_or(ClipboardError::Invalid)?;
    let id = take_u32(bytes, &mut i).ok_or(ClipboardError::Invalid)?;
    let _created = take_u64(bytes, &mut i).ok_or(ClipboardError::Invalid)?;
    let len = take_u32(bytes, &mut i).ok_or(ClipboardError::Invalid)? as usize;
    let mime_len = take_u16(bytes, &mut i).ok_or(ClipboardError::Invalid)? as usize;
    let source_len = take_u16(bytes, &mut i).ok_or(ClipboardError::Invalid)? as usize;
    let mime = take_slice(bytes, &mut i, mime_len).ok_or(ClipboardError::Invalid)?;
    if mime != MIME_FILES && mime != MIME_CUT {
        return Err(ClipboardError::Unsupported);
    }
    take_slice(bytes, &mut i, source_len).ok_or(ClipboardError::Invalid)?;
    let payload = take_slice(bytes, &mut i, len).ok_or(ClipboardError::Invalid)?;
    if i != bytes.len() {
        return Err(ClipboardError::Invalid);
    }
    let mut paths = Vec::new();
    for part in payload.split(|b| *b == 0).filter(|part| !part.is_empty()) {
        let path = core::str::from_utf8(part).map_err(|_| ClipboardError::Invalid)?;
        if !path.starts_with('/') {
            return Err(ClipboardError::Invalid);
        }
        paths.push(String::from(path));
    }
    if paths.is_empty() {
        return Err(ClipboardError::Empty);
    }
    Ok(FileClipboard {
        id,
        cut: mime == MIME_CUT,
        paths,
    })
}

/// Return whether the current clipboard item can be pasted as text.
pub fn text_available() -> bool {
    get_text().is_ok()
}

fn parse_text_item(bytes: &[u8]) -> Result<&[u8], ClipboardError> {
    let mut index = 0usize;
    if take_u32(bytes, &mut index) != Some(WIRE_MAGIC_ITEM)
        || take_u16(bytes, &mut index) != Some(WIRE_VERSION)
    {
        return Err(ClipboardError::Invalid);
    }
    if take_u8(bytes, &mut index) != Some(KIND_TEXT) {
        return Err(ClipboardError::Unsupported);
    }
    let _flags = take_u8(bytes, &mut index).ok_or(ClipboardError::Invalid)?;
    let _id = take_u32(bytes, &mut index).ok_or(ClipboardError::Invalid)?;
    let _created = take_u64(bytes, &mut index).ok_or(ClipboardError::Invalid)?;
    let payload_len = take_u32(bytes, &mut index).ok_or(ClipboardError::Invalid)? as usize;
    let mime_len = take_u16(bytes, &mut index).ok_or(ClipboardError::Invalid)? as usize;
    let source_len = take_u16(bytes, &mut index).ok_or(ClipboardError::Invalid)? as usize;
    let _mime = take_slice(bytes, &mut index, mime_len).ok_or(ClipboardError::Invalid)?;
    let _source = take_slice(bytes, &mut index, source_len).ok_or(ClipboardError::Invalid)?;
    take_slice(bytes, &mut index, payload_len).ok_or(ClipboardError::Invalid)
}

fn ensure_service() -> Option<CapabilityToken> {
    // clipd is a system service. Widget code does not own service lifecycle;
    // callers receive a normal Unavailable result if the session is not ready.
    nameserver_lookup_timeout("clipd", 100)
}

fn from_service_error(code: u64) -> ClipboardError {
    match code {
        x if x == ClipMsg::ERR_TOO_LARGE => ClipboardError::TooLarge,
        x if x == ClipMsg::ERR_UNSUPPORTED => ClipboardError::Unsupported,
        x if x == ClipMsg::ERR_CORRUPT || x == ClipMsg::ERR_BAD_REQUEST => ClipboardError::Invalid,
        x if x == ClipMsg::ERR_NOT_FOUND => ClipboardError::Empty,
        _ => ClipboardError::Unavailable,
    }
}

fn put_u16(buf: &mut [u8], value: u16) -> usize {
    buf[..2].copy_from_slice(&value.to_le_bytes());
    2
}

fn put_u32(buf: &mut [u8], value: u32) -> usize {
    buf[..4].copy_from_slice(&value.to_le_bytes());
    4
}

fn put_bytes(buf: &mut [u8], source: &[u8]) -> usize {
    buf[..source.len()].copy_from_slice(source);
    source.len()
}

fn take_u8(bytes: &[u8], index: &mut usize) -> Option<u8> {
    let value = *bytes.get(*index)?;
    *index += 1;
    Some(value)
}

fn take_u16(bytes: &[u8], index: &mut usize) -> Option<u16> {
    let value = bytes.get(*index..*index + 2)?;
    *index += 2;
    Some(u16::from_le_bytes([value[0], value[1]]))
}

fn take_u32(bytes: &[u8], index: &mut usize) -> Option<u32> {
    let value = bytes.get(*index..*index + 4)?;
    *index += 4;
    Some(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn take_u64(bytes: &[u8], index: &mut usize) -> Option<u64> {
    let value = bytes.get(*index..*index + 8)?;
    *index += 8;
    Some(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

fn take_slice<'a>(bytes: &'a [u8], index: &mut usize, len: usize) -> Option<&'a [u8]> {
    let value = bytes.get(*index..*index + len)?;
    *index += len;
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::{parse_text_item, ClipboardError, KIND_TEXT, WIRE_MAGIC_ITEM, WIRE_VERSION};

    fn file_item(mime: &[u8], payload: &[u8]) -> alloc::vec::Vec<u8> {
        let mut bytes = alloc::vec::Vec::new();
        bytes.extend_from_slice(&WIRE_MAGIC_ITEM.to_le_bytes());
        bytes.extend_from_slice(&WIRE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&[super::KIND_FILES, 1]);
        bytes.extend_from_slice(&42u32.to_le_bytes());
        bytes.extend_from_slice(&123u64.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(mime.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(mime);
        bytes.extend_from_slice(b"app");
        bytes.extend_from_slice(payload);
        bytes
    }

    #[test]
    fn parses_cli_copy_and_desktop_cut_lists() {
        for (mime, cut) in [(super::MIME_FILES, false), (super::MIME_CUT, true)] {
            let bytes = file_item(mime, b"/home/a.txt\0/home/Folder with spaces\0");
            let item = super::parse_files_item(&bytes).unwrap();
            assert_eq!(item.id, 42);
            assert_eq!(item.cut, cut);
            assert_eq!(item.paths, ["/home/a.txt", "/home/Folder with spaces"]);
            for length in 0..bytes.len() {
                assert!(super::parse_files_item(&bytes[..length]).is_err());
            }
        }
    }

    #[test]
    fn file_items_reject_unknown_operations_relative_paths_and_invalid_utf8() {
        assert_eq!(
            super::parse_files_item(&file_item(b"other", b"/a")),
            Err(ClipboardError::Unsupported)
        );
        assert_eq!(
            super::parse_files_item(&file_item(super::MIME_FILES, b"relative")),
            Err(ClipboardError::Invalid)
        );
        assert_eq!(
            super::parse_files_item(&file_item(super::MIME_FILES, &[b'/', 255])),
            Err(ClipboardError::Invalid)
        );
        assert_eq!(
            super::parse_files_item(&file_item(super::MIME_FILES, b"")),
            Err(ClipboardError::Empty)
        );
    }

    #[test]
    fn parses_text_item() {
        let mut bytes = alloc::vec::Vec::new();
        bytes.extend_from_slice(&WIRE_MAGIC_ITEM.to_le_bytes());
        bytes.extend_from_slice(&WIRE_VERSION.to_le_bytes());
        bytes.push(KIND_TEXT);
        bytes.push(1);
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(&123u64.to_le_bytes());
        bytes.extend_from_slice(&5u32.to_le_bytes());
        bytes.extend_from_slice(&10u16.to_le_bytes());
        bytes.extend_from_slice(&3u16.to_le_bytes());
        bytes.extend_from_slice(b"text/plain");
        bytes.extend_from_slice(b"app");
        bytes.extend_from_slice(b"hello");
        assert_eq!(parse_text_item(&bytes), Ok(&b"hello"[..]));
    }

    #[test]
    fn rejects_non_text_item() {
        let mut bytes = [0u8; 24];
        bytes[..4].copy_from_slice(&WIRE_MAGIC_ITEM.to_le_bytes());
        bytes[4..6].copy_from_slice(&WIRE_VERSION.to_le_bytes());
        bytes[6] = 2;
        assert_eq!(parse_text_item(&bytes), Err(ClipboardError::Unsupported));
    }
}
