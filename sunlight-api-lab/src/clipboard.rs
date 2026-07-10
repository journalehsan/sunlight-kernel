use sunlight_ipc::{
    ipc_call, nameserver_lookup_timeout, process_yield, shm_alloc, shm_free, CapabilityToken,
    ClipMsg, IpcMsg, SHM_PAGE,
};

const CLIP_WIRE_MAGIC_SET: u32 = 0x4353_4554;
const CLIP_WIRE_VERSION: u16 = 1;
const CLIP_MIME_TEXT: &[u8] = b"text/plain";
const CLIP_SOURCE_APP: &[u8] = b"sunlight-api-lab";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClipPayloadKind {
    Text = 1,
}

impl ClipPayloadKind {
    const fn as_u8(self) -> u8 {
        self as u8
    }
}

pub fn set_clipboard_text(payload: &[u8]) -> Result<(), &'static str> {
    let cap = ensure_clipboard_service().ok_or("Clipboard service unavailable")?;
    let total_len = 16 + CLIP_MIME_TEXT.len() + CLIP_SOURCE_APP.len() + payload.len();
    if total_len > SHM_PAGE {
        return Err("Clipboard payload is too large");
    }

    let (ptr, token) = shm_alloc().map_err(|_| "Clipboard service unavailable")?;
    unsafe {
        let buf = core::slice::from_raw_parts_mut(ptr, SHM_PAGE);
        let mut index = 0usize;
        index += push_u32_le(&mut buf[index..], CLIP_WIRE_MAGIC_SET);
        index += push_u16_le(&mut buf[index..], CLIP_WIRE_VERSION);
        buf[index] = ClipPayloadKind::Text.as_u8();
        index += 1;
        buf[index] = 1;
        index += 1;
        index += push_u16_le(&mut buf[index..], CLIP_MIME_TEXT.len() as u16);
        index += push_u16_le(&mut buf[index..], CLIP_SOURCE_APP.len() as u16);
        index += push_u32_le(&mut buf[index..], payload.len() as u32);
        index += copy_bytes(&mut buf[index..], CLIP_MIME_TEXT);
        index += copy_bytes(&mut buf[index..], CLIP_SOURCE_APP);
        let _ = copy_bytes(&mut buf[index..], payload);
    }

    let reply = ipc_call(
        cap,
        IpcMsg::with_label(ClipMsg::SET_CLIPBOARD)
            .word(0, total_len as u64)
            .with_cap(0, token),
    );
    let _ = shm_free(token);
    if reply.label == ClipMsg::ERROR {
        return Err(clip_error_label(reply.words[0]));
    }
    Ok(())
}

fn ensure_clipboard_service() -> Option<CapabilityToken> {
    if let Some(cap) = nameserver_lookup_timeout("clipd", 50) {
        return Some(cap);
    }
    let _ = sunlight_libc::spawn(b"/sbin/sunlight-clipd", &[b"sunlight-clipd"], None)
        .or_else(|_| sunlight_libc::spawn(b"/bin/sunlight-clipd", &[b"sunlight-clipd"], None));
    for _ in 0..8 {
        if let Some(cap) = nameserver_lookup_timeout("clipd", 75) {
            return Some(cap);
        }
        process_yield();
    }
    None
}

fn clip_error_label(code: u64) -> &'static str {
    match code {
        x if x == ClipMsg::ERR_BAD_REQUEST => "Clipboard request is invalid",
        x if x == ClipMsg::ERR_NOT_FOUND => "Clipboard item not found",
        x if x == ClipMsg::ERR_TOO_LARGE => "Clipboard payload is too large",
        x if x == ClipMsg::ERR_UNSUPPORTED => "Clipboard type is not supported",
        x if x == ClipMsg::ERR_CORRUPT => "Clipboard item is invalid",
        _ => "Clipboard service unavailable",
    }
}

fn push_u16_le(buf: &mut [u8], value: u16) -> usize {
    if buf.len() < 2 {
        return 0;
    }
    buf[..2].copy_from_slice(&value.to_le_bytes());
    2
}

fn push_u32_le(buf: &mut [u8], value: u32) -> usize {
    if buf.len() < 4 {
        return 0;
    }
    buf[..4].copy_from_slice(&value.to_le_bytes());
    4
}

fn copy_bytes(buf: &mut [u8], src: &[u8]) -> usize {
    let len = src.len().min(buf.len());
    buf[..len].copy_from_slice(&src[..len]);
    len
}
