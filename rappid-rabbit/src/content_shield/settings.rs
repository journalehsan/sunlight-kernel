//! Browser-chrome-only KV access. No file paths, settings values or methods are
//! registered with SunJS. KV authenticates the caller and owns the record ACL.
use super::FilterSettings;

#[cfg(target_os = "sunlightos")]
fn message(label: u64) -> Option<sunlight_ipc::IpcMsg> {
    let key = format!("rr.cs:{}", sunlight_ipc::getuid());
    if key.len() > 16 {
        return None;
    }
    let mut msg = sunlight_ipc::IpcMsg::with_label(label);
    for (i, byte) in key.bytes().enumerate() {
        msg.words[2 + i / 8] |= (byte as u64) << ((i % 8) * 8);
    }
    Some(msg)
}
pub fn load_settings() -> Option<FilterSettings> {
    #[cfg(target_os = "sunlightos")]
    {
        use sunlight_ipc::*;
        let cap = nameserver_lookup_timeout("sunlight-kv", 200)?;
        let reply = ipc_call_timeout(cap, message(0x4B07)?, 300).ok()?;
        if reply.label != 0x4B05 {
            return None;
        }
        let token = reply.caps[0];
        if token == CapabilityToken::INVALID {
            return None;
        }
        let settings = (|| {
            let n = usize::try_from(reply.words[0]).ok()?;
            if n == 0 || n > 4096 {
                return None;
            }
            let ptr = shm_map(token).ok()?;
            let bytes = unsafe { core::slice::from_raw_parts(ptr, n) };
            FilterSettings::decode(core::str::from_utf8(bytes).ok()?)
        })();
        let _ = shm_free(token);
        settings
    }
    #[cfg(not(target_os = "sunlightos"))]
    {
        None
    }
}
pub fn save_settings(settings: &FilterSettings) -> bool {
    #[cfg(target_os = "sunlightos")]
    {
        use sunlight_ipc::*;
        let Some(encoded) = settings.encode() else {
            return false;
        };
        let Some(cap) = nameserver_lookup_timeout("sunlight-kv", 200) else {
            return false;
        };
        let Some(mut msg) = message(0x4B06) else {
            return false;
        };
        let Ok((ptr, token)) = shm_alloc() else {
            return false;
        };
        unsafe {
            core::ptr::copy_nonoverlapping(encoded.as_ptr(), ptr, encoded.len());
        }
        msg.words[0] = encoded.len() as u64;
        msg = msg.with_cap(0, token);
        let reply = ipc_call_timeout(cap, msg, 300);
        let _ = shm_free(token);
        reply.is_ok_and(|r| r.label == 0x4BFF && r.words[0] == 0)
    }
    #[cfg(not(target_os = "sunlightos"))]
    {
        let _ = settings;
        false
    }
}
