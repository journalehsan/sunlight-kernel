//! Small per-user preferences use the existing sunlight-kv SHM2 protocol,
//! following Calendar. Never writes inside the application bundle.
use sunlight_empires::sound::Settings;
use sunlight_ipc::{
    ipc_call_timeout, nameserver_lookup_timeout, shm_alloc, shm_free, shm_map, CapabilityToken,
    IpcMsg,
};
fn key() -> alloc::string::String {
    alloc::format!("app.empires.settings/{}/v1", sunlight_libc::getuid())
}
pub fn load() -> Settings {
    read().unwrap_or_default()
}
fn read() -> Option<Settings> {
    let cap = nameserver_lookup_timeout("sunlight-kv", 50)?;
    let key = key();
    let (ptr, token) = shm_alloc().ok()?;
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), ptr, key.len());
    }
    let reply = ipc_call_timeout(
        cap,
        IpcMsg::with_label(0x4B09)
            .word(0, key.len() as u64)
            .with_cap(0, token),
        200,
    );
    let _ = shm_free(token);
    let reply = reply.ok()?;
    if reply.label != 0x4B05 {
        return None;
    }
    let value = reply.caps[0];
    if value == CapabilityToken::INVALID {
        return None;
    }
    let result = (|| {
        if reply.words[0] != 6 {
            return None;
        }
        let ptr = shm_map(value).ok()?;
        Settings::decode(unsafe { core::slice::from_raw_parts(ptr, 6) })
    })();
    let _ = shm_free(value);
    result
}
pub fn save(settings: Settings) -> bool {
    let Some(cap) = nameserver_lookup_timeout("sunlight-kv", 50) else {
        return false;
    };
    let key = key();
    let bytes = settings.encode();
    let Ok((kp, kt)) = shm_alloc() else {
        return false;
    };
    let Ok((vp, vt)) = shm_alloc() else {
        let _ = shm_free(kt);
        return false;
    };
    unsafe {
        core::ptr::copy_nonoverlapping(key.as_ptr(), kp, key.len());
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), vp, bytes.len());
    }
    let reply = ipc_call_timeout(
        cap,
        IpcMsg::with_label(0x4B08)
            .word(0, key.len() as u64)
            .word(1, bytes.len() as u64)
            .with_cap(0, kt)
            .with_cap(1, vt),
        200,
    );
    let _ = shm_free(kt);
    let _ = shm_free(vt);
    reply.is_ok_and(|r| r.label == 0x4BFF && r.words[0] == 0)
}
