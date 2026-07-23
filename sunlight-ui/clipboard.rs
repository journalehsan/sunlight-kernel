//! Text clipboard access shared by Sunlight UI text widgets.
//!
//! This module deliberately exposes only the text operations widgets need. The
//! clipboard daemon remains responsible for history, persistence, and other
//! payload kinds.

use alloc::string::String;
use sunlight_ipc::{
    ipc_call, nameserver_lookup_timeout, shm_alloc, shm_free, shm_map, CapabilityToken, ClipMsg,
    IpcMsg, SHM_PAGE,
};

const WIRE_MAGIC_SET: u32 = 0x4353_4554;
const WIRE_MAGIC_ITEM: u32 = 0x434C_4950;
const WIRE_VERSION: u16 = 1;
const KIND_TEXT: u8 = 1;
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
    let cap = ensure_service().ok_or(ClipboardError::Unavailable)?;
    let source_app = if source_app.is_empty() {
        DEFAULT_SOURCE
    } else {
        source_app
    };
    let payload = text.as_bytes();
    let total_len = 16 + MIME_TEXT.len() + source_app.len() + payload.len();
    if total_len > SHM_PAGE
        || MIME_TEXT.len() > u16::MAX as usize
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
        buf[index] = KIND_TEXT;
        index += 1;
        buf[index] = 1;
        index += 1;
        index += put_u16(&mut buf[index..], MIME_TEXT.len() as u16);
        index += put_u16(&mut buf[index..], source_app.len() as u16);
        index += put_u32(&mut buf[index..], payload.len() as u32);
        index += put_bytes(&mut buf[index..], MIME_TEXT);
        index += put_bytes(&mut buf[index..], source_app);
        let _ = put_bytes(&mut buf[index..], payload);
    }

    let reply = ipc_call(
        cap,
        IpcMsg::with_label(ClipMsg::SET_CLIPBOARD)
            .word(0, total_len as u64)
            .with_cap(0, token),
    );
    let _ = shm_free(token);
    if reply.label == ClipMsg::ERROR {
        return Err(from_service_error(reply.words[0]));
    }
    Ok(())
}

pub fn get_text() -> Result<String, ClipboardError> {
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
    let payload = parse_text_item(&page[..len])?;
    let text = core::str::from_utf8(payload).map_err(|_| ClipboardError::Invalid)?;
    Ok(String::from(text))
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
