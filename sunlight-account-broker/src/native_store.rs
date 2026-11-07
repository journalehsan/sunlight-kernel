//! Broker-only KV IPC. UID comes only from the kernel-authenticated panel role.
use alloc::{format, string::String};
use sunlight_account_broker::{storage, Error, KvClient, Result};
use sunlight_ipc::{ipc_call_timeout, nameserver_lookup, shm_alloc, shm_free, shm_map, IpcMsg};
use zeroize::Zeroizing;

pub struct LiveKv(pub u32);
impl LiveKv {
    fn call(&self, op: u64, key: &str, value: Option<&str>) -> Result<IpcMsg> {
        let key = key
            .strip_prefix("account-broker:")
            .ok_or(Error::Unauthorized)?;
        let key = format!("account-broker:uid:{}:{key}", self.0);
        if key.len() > 4096 {
            return Err(Error::Capacity);
        }
        let ep = nameserver_lookup("sunlight-kv").ok_or(Error::Kv)?;
        let (ptr, tok) = shm_alloc().map_err(|_| Error::Kv)?;
        unsafe {
            core::ptr::copy_nonoverlapping(key.as_ptr(), ptr, key.len());
        }
        let mut msg = IpcMsg::with_label(op)
            .word(0, key.len() as u64)
            .with_cap(0, tok);
        let mut payload = None;
        if let Some(value) = value {
            if value.len() > 4096 {
                let _ = shm_free(tok);
                return Err(Error::Capacity);
            }
            let (p, t) = match shm_alloc() {
                Ok(v) => v,
                Err(_) => {
                    let _ = shm_free(tok);
                    return Err(Error::Kv);
                }
            };
            unsafe {
                core::ptr::copy_nonoverlapping(value.as_ptr(), p, value.len());
            }
            msg = msg.word(1, value.len() as u64).with_cap(1, t);
            payload = Some((p, t));
        }
        let reply = ipc_call_timeout(ep, msg, 3000).map_err(|_| Error::Kv);
        let _ = shm_free(tok);
        if let Some((p, t)) = payload {
            unsafe {
                sunlight_ipc::accounts::clear_secret(core::slice::from_raw_parts_mut(p, 4096));
            }
            let _ = shm_free(t);
        }
        let reply = reply?;
        if reply.label == 0x4BEE {
            return Err(match reply.words[0] {
                10 => Error::NotFound,
                11 => Error::Unauthorized,
                _ => Error::Kv,
            });
        }
        Ok(reply)
    }
}
impl KvClient for LiveKv {
    fn put(&mut self, key: &str, value: &str) -> Result<()> {
        let r = self.call(0x4B08, key, Some(value))?;
        if r.label == 0x4BFF && r.words[0] == 0 {
            Ok(())
        } else {
            Err(Error::Kv)
        }
    }
    fn get(&mut self, key: &str) -> Result<Zeroizing<String>> {
        let r = self.call(0x4B09, key, None)?;
        if r.label != 0x4B05 || r.words[0] > 4096 {
            return Err(Error::Kv);
        }
        let tok = r.caps[0];
        let p = shm_map(tok).map_err(|_| Error::Kv)?;
        let raw = unsafe { core::slice::from_raw_parts(p, r.words[0] as usize) };
        let value = core::str::from_utf8(raw)
            .map(|s| Zeroizing::new(String::from(s)))
            .map_err(|_| Error::Kv);
        unsafe {
            sunlight_ipc::accounts::clear_secret(core::slice::from_raw_parts_mut(p, 4096));
        }
        let _ = shm_free(tok);
        value
    }
    fn delete(&mut self, key: &str) -> Result<()> {
        let r = self.call(0x4B0A, key, None)?;
        if r.label == 0x4BFF && r.words[0] == 0 {
            Ok(())
        } else {
            Err(Error::Kv)
        }
    }
}

pub fn disconnect(uid: u32, handle: &str) -> Result<()> {
    let mut kv = LiveKv(uid);
    // Verify ownership via this UID's index before any mutation.
    if !storage::read_index(&mut kv)?.iter().any(|h| h == handle) {
        return Err(Error::NotFound);
    }
    kv.delete(&storage::record_key(handle))?;
    storage::verify_absent(&mut kv, handle)?;
    storage::index_remove(&mut kv, handle)
}
