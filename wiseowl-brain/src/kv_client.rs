//! Bounded native sunlight-kv client for Brain MTM (≤16-byte keys via PUT_SHM/GET_SHM).

use crate::mtm::{
    decode_bool, decode_provider, decode_style, decode_u32, decode_u64, encode_bool, encode_style,
    encode_u32, encode_u64, format_user_key, BrainPreferences, GreetingStyle, WelcomeMemoryState,
    KEY_CODE_GEN, KEY_CODE_PROV, KEY_CODE_SIS, KEY_CODE_SMS, KEY_CODE_STYLE, KEY_CODE_VISIT,
};

/// Result of a KV-backed MTM load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MtmLoadResult {
    pub welcome: WelcomeMemoryState,
    pub preferences: BrainPreferences,
    /// True if sunlight-kv was reachable for at least one successful op.
    pub kv_reachable: bool,
    /// True if any key was missing or malformed (defaults applied).
    pub used_defaults: bool,
    /// True if KV lookup/ops failed entirely.
    pub degraded: bool,
}

/// Abstract MTM store used by pipeline (host tests inject memory map; native uses sunlight-kv).
pub trait MtmStore {
    fn get(&self, key: &str) -> Result<Option<heapless::Vec<u8, 64>>, ()>;
    fn put(&self, key: &str, value: &[u8]) -> Result<(), ()>;
}

/// In-memory store for host unit tests.
#[derive(Debug, Default)]
pub struct MemoryMtmStore {
    // fixed slots for tests
    pub entries: heapless::Vec<(heapless::String<16>, heapless::Vec<u8, 64>), 32>,
}

impl MemoryMtmStore {
    pub fn new() -> Self {
        Self {
            entries: heapless::Vec::new(),
        }
    }

    fn find(&self, key: &str) -> Option<usize> {
        self.entries.iter().position(|(k, _)| k.as_str() == key)
    }
}

impl MtmStore for MemoryMtmStore {
    fn get(&self, key: &str) -> Result<Option<heapless::Vec<u8, 64>>, ()> {
        Ok(self.find(key).map(|i| self.entries[i].1.clone()))
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<(), ()> {
        // Interior mutability via raw cast is avoided; host tests use RefCell wrapper.
        let _ = (key, value);
        Err(())
    }
}

/// Mutable memory store for host tests.
pub struct MemoryMtmStoreMut {
    pub inner: core::cell::RefCell<MemoryMtmStore>,
}

impl MemoryMtmStoreMut {
    pub fn new() -> Self {
        Self {
            inner: core::cell::RefCell::new(MemoryMtmStore::new()),
        }
    }
}

impl Default for MemoryMtmStoreMut {
    fn default() -> Self {
        Self::new()
    }
}

impl MtmStore for MemoryMtmStoreMut {
    fn get(&self, key: &str) -> Result<Option<heapless::Vec<u8, 64>>, ()> {
        let store = self.inner.borrow();
        Ok(store.find(key).map(|i| store.entries[i].1.clone()))
    }

    fn put(&self, key: &str, value: &[u8]) -> Result<(), ()> {
        let mut store = self.inner.borrow_mut();
        let mut k: heapless::String<16> = heapless::String::new();
        if k.push_str(key).is_err() {
            return Err(());
        }
        let mut v: heapless::Vec<u8, 64> = heapless::Vec::new();
        for &b in value.iter().take(64) {
            let _ = v.push(b);
        }
        if let Some(i) = store.find(key) {
            store.entries[i].1 = v;
        } else {
            let _ = store.entries.push((k, v));
        }
        Ok(())
    }
}

pub fn load_mtm(store: &dyn MtmStore, uid: u64) -> MtmLoadResult {
    let mut result = MtmLoadResult {
        welcome: WelcomeMemoryState::default(),
        preferences: BrainPreferences::default(),
        kv_reachable: false,
        used_defaults: false,
        degraded: false,
    };

    let mut key = heapless::String::<16>::new();

    // visit_count
    if format_user_key(uid, KEY_CODE_VISIT, &mut key) {
        match store.get(key.as_str()) {
            Ok(Some(bytes)) => {
                result.kv_reachable = true;
                if let Some(v) = decode_u32(&bytes) {
                    result.welcome.visit_count = v;
                } else {
                    result.used_defaults = true;
                }
            }
            Ok(None) => {
                result.kv_reachable = true;
                result.used_defaults = true;
            }
            Err(()) => result.degraded = true,
        }
    }

    // last_completed_generation
    if format_user_key(uid, KEY_CODE_GEN, &mut key) {
        match store.get(key.as_str()) {
            Ok(Some(bytes)) => {
                result.kv_reachable = true;
                if let Some(v) = decode_u64(&bytes) {
                    result.welcome.last_completed_generation = Some(v);
                } else {
                    result.used_defaults = true;
                }
            }
            Ok(None) => {
                result.kv_reachable = true;
            }
            Err(()) => result.degraded = true,
        }
    }

    // last provider
    if format_user_key(uid, KEY_CODE_PROV, &mut key) {
        match store.get(key.as_str()) {
            Ok(Some(bytes)) => {
                result.kv_reachable = true;
                if let Some(p) = decode_provider(&bytes) {
                    result.welcome.last_successful_provider = Some(p);
                } else {
                    result.used_defaults = true;
                }
            }
            Ok(None) => {
                result.kv_reachable = true;
            }
            Err(()) => result.degraded = true,
        }
    }

    // greeting style
    if format_user_key(uid, KEY_CODE_STYLE, &mut key) {
        match store.get(key.as_str()) {
            Ok(Some(bytes)) => {
                result.kv_reachable = true;
                if let Some(s) = decode_style(&bytes) {
                    result.preferences.greeting_style = s;
                } else {
                    result.used_defaults = true;
                }
            }
            Ok(None) => {
                result.kv_reachable = true;
                result.used_defaults = true;
            }
            Err(()) => result.degraded = true,
        }
    }

    // show machine summary
    if format_user_key(uid, KEY_CODE_SMS, &mut key) {
        match store.get(key.as_str()) {
            Ok(Some(bytes)) => {
                result.kv_reachable = true;
                if let Some(b) = decode_bool(&bytes) {
                    result.preferences.show_machine_summary = b;
                } else {
                    result.used_defaults = true;
                }
            }
            Ok(None) => {
                result.kv_reachable = true;
            }
            Err(()) => result.degraded = true,
        }
    }

    // show index status
    if format_user_key(uid, KEY_CODE_SIS, &mut key) {
        match store.get(key.as_str()) {
            Ok(Some(bytes)) => {
                result.kv_reachable = true;
                if let Some(b) = decode_bool(&bytes) {
                    result.preferences.show_index_status = b;
                } else {
                    result.used_defaults = true;
                }
            }
            Ok(None) => {
                result.kv_reachable = true;
            }
            Err(()) => result.degraded = true,
        }
    }

    if result.degraded && !result.kv_reachable {
        result.used_defaults = true;
    }
    result
}

pub fn save_preferences(store: &dyn MtmStore, uid: u64, prefs: &BrainPreferences) -> Result<(), ()> {
    let mut key = heapless::String::<16>::new();
    if !format_user_key(uid, KEY_CODE_STYLE, &mut key) {
        return Err(());
    }
    store.put(key.as_str(), encode_style(prefs.greeting_style).as_bytes())?;
    if !format_user_key(uid, KEY_CODE_SMS, &mut key) {
        return Err(());
    }
    store.put(key.as_str(), encode_bool(prefs.show_machine_summary).as_bytes())?;
    if !format_user_key(uid, KEY_CODE_SIS, &mut key) {
        return Err(());
    }
    store.put(key.as_str(), encode_bool(prefs.show_index_status).as_bytes())?;
    Ok(())
}

pub fn save_welcome_state(store: &dyn MtmStore, uid: u64, state: &WelcomeMemoryState) -> Result<(), ()> {
    let mut key = heapless::String::<16>::new();
    let mut num = heapless::String::<16>::new();
    if !format_user_key(uid, KEY_CODE_VISIT, &mut key) {
        return Err(());
    }
    encode_u32(state.visit_count, &mut num);
    store.put(key.as_str(), num.as_bytes())?;

    if let Some(gen) = state.last_completed_generation {
        if !format_user_key(uid, KEY_CODE_GEN, &mut key) {
            return Err(());
        }
        let mut gbuf = heapless::String::<24>::new();
        encode_u64(gen, &mut gbuf);
        store.put(key.as_str(), gbuf.as_bytes())?;
    }

    if let Some(p) = state.last_successful_provider {
        if !format_user_key(uid, KEY_CODE_PROV, &mut key) {
            return Err(());
        }
        let mut one = heapless::String::<4>::new();
        let _ = core::fmt::Write::write_fmt(&mut one, format_args!("{}", p as u8));
        store.put(key.as_str(), one.as_bytes())?;
    }
    Ok(())
}

pub fn set_preference_field(
    store: &dyn MtmStore,
    uid: u64,
    field: &str,
    value: &str,
) -> Result<BrainPreferences, ()> {
    let mut loaded = load_mtm(store, uid);
    match field {
        "greeting-style" | "greeting_style" | "style" => {
            let s = GreetingStyle::from_str(value).ok_or(())?;
            loaded.preferences.greeting_style = s;
        }
        "show-machine-summary" | "show_machine_summary" | "machine-summary" => {
            let b = match value {
                "true" | "1" | "yes" => true,
                "false" | "0" | "no" => false,
                _ => return Err(()),
            };
            loaded.preferences.show_machine_summary = b;
        }
        "show-index-status" | "show_index_status" | "index-status" => {
            let b = match value {
                "true" | "1" | "yes" => true,
                "false" | "0" | "no" => false,
                _ => return Err(()),
            };
            loaded.preferences.show_index_status = b;
        }
        _ => return Err(()),
    }
    save_preferences(store, uid, &loaded.preferences)?;
    Ok(load_mtm(store, uid).preferences)
}

/// Native sunlight-kv adapter (register key packing, SHM values).
#[cfg(feature = "sunlightos")]
pub mod native {
    use super::*;
    use sunlight_ipc::{
        ipc_call_timeout, nameserver_lookup_timeout, shm_alloc, shm_free, shm_map, CapabilityToken,
        IpcMsg, SHM_PAGE,
    };

    const KV_ENDPOINT: &str = "sunlight-kv";
    const KV_PUT_SHM: u64 = 0x4B06;
    const KV_GET_SHM: u64 = 0x4B07;
    const KV_REPLY: u64 = 0x4BFF;
    const KV_VALUE: u64 = 0x4B05;
    const KV_TIMEOUT_MS: u64 = 50;

    pub struct NativeKvStore;

    fn pack_key(msg: &mut IpcMsg, key: &str) -> bool {
        let bytes = key.as_bytes();
        if bytes.is_empty() || bytes.len() > 16 {
            return false;
        }
        // words[2], words[3]
        msg.words[2] = 0;
        msg.words[3] = 0;
        for (i, &b) in bytes.iter().enumerate() {
            let word = 2 + i / 8;
            let shift = (i % 8) * 8;
            msg.words[word] |= (b as u64) << shift;
        }
        if msg.word_count < 4 {
            msg.word_count = 4;
        }
        true
    }

    impl MtmStore for NativeKvStore {
        fn get(&self, key: &str) -> Result<Option<heapless::Vec<u8, 64>>, ()> {
            let Some(cap) = nameserver_lookup_timeout(KV_ENDPOINT, KV_TIMEOUT_MS) else {
                return Err(());
            };
            let mut msg = IpcMsg::with_label(KV_GET_SHM);
            if !pack_key(&mut msg, key) {
                return Err(());
            }
            let reply = ipc_call_timeout(cap, msg, KV_TIMEOUT_MS).map_err(|_| ())?;
            if reply.label == KV_REPLY {
                // not found / error
                return Ok(None);
            }
            if reply.label != KV_VALUE {
                return Err(());
            }
            let len = (reply.words[0] as usize).min(64);
            if reply.cap_count == 0 || reply.caps[0] == CapabilityToken::INVALID {
                return Ok(Some(heapless::Vec::new()));
            }
            let ptr = shm_map(reply.caps[0]).map_err(|_| ())?;
            let mut out: heapless::Vec<u8, 64> = heapless::Vec::new();
            let slice = unsafe { core::slice::from_raw_parts(ptr, len) };
            for &b in slice {
                let _ = out.push(b);
            }
            let _ = shm_free(reply.caps[0]);
            Ok(Some(out))
        }

        fn put(&self, key: &str, value: &[u8]) -> Result<(), ()> {
            let Some(cap) = nameserver_lookup_timeout(KV_ENDPOINT, KV_TIMEOUT_MS) else {
                return Err(());
            };
            if value.len() > SHM_PAGE {
                return Err(());
            }
            let (ptr, tok) = shm_alloc().map_err(|_| ())?;
            unsafe {
                core::ptr::copy_nonoverlapping(value.as_ptr(), ptr, value.len());
            }
            let mut msg = IpcMsg::with_label(KV_PUT_SHM)
                .word(0, value.len() as u64)
                .with_cap(0, tok);
            if !pack_key(&mut msg, key) {
                let _ = shm_free(tok);
                return Err(());
            }
            let reply = match ipc_call_timeout(cap, msg, KV_TIMEOUT_MS) {
                Ok(r) => r,
                Err(_) => {
                    let _ = shm_free(tok);
                    return Err(());
                }
            };
            let _ = shm_free(tok);
            if reply.label == KV_REPLY && reply.words[0] == 0 {
                Ok(())
            } else {
                Err(())
            }
        }
    }

    pub fn native_store() -> NativeKvStore {
        NativeKvStore
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_defaults_when_empty() {
        let store = MemoryMtmStoreMut::new();
        let r = load_mtm(&store, 0);
        assert_eq!(r.preferences, BrainPreferences::default());
        assert_eq!(r.welcome.visit_count, 0);
        assert!(r.used_defaults || r.kv_reachable);
    }

    #[test]
    fn preferences_persist_roundtrip() {
        let store = MemoryMtmStoreMut::new();
        let prefs = BrainPreferences {
            show_machine_summary: false,
            show_index_status: true,
            greeting_style: GreetingStyle::Technical,
        };
        save_preferences(&store, 1000, &prefs).unwrap();
        let r = load_mtm(&store, 1000);
        assert_eq!(r.preferences, prefs);
    }

    #[test]
    fn cross_user_isolation() {
        let store = MemoryMtmStoreMut::new();
        let mut prefs = BrainPreferences::default();
        prefs.greeting_style = GreetingStyle::Friendly;
        save_preferences(&store, 1, &prefs).unwrap();
        let other = load_mtm(&store, 2);
        assert_eq!(other.preferences.greeting_style, GreetingStyle::Concise);
    }

    #[test]
    fn completion_persists_visit() {
        let store = MemoryMtmStoreMut::new();
        let mut state = WelcomeMemoryState::default();
        state.record_completion(42);
        save_welcome_state(&store, 0, &state).unwrap();
        let r = load_mtm(&store, 0);
        assert_eq!(r.welcome.visit_count, 1);
        assert_eq!(r.welcome.last_completed_generation, Some(42));
        assert!(r.welcome.is_returning_visit());
    }

    #[test]
    fn set_preference_field_rejects_unknown() {
        let store = MemoryMtmStoreMut::new();
        assert!(set_preference_field(&store, 0, "mood", "happy").is_err());
        assert!(set_preference_field(&store, 0, "greeting-style", "chatty").is_err());
        let p = set_preference_field(&store, 0, "greeting-style", "friendly").unwrap();
        assert_eq!(p.greeting_style, GreetingStyle::Friendly);
    }

    #[test]
    fn malformed_value_falls_back() {
        let store = MemoryMtmStoreMut::new();
        let mut key = heapless::String::<16>::new();
        assert!(format_user_key(0, KEY_CODE_STYLE, &mut key));
        store.put(key.as_str(), b"!!!").unwrap();
        let r = load_mtm(&store, 0);
        assert_eq!(r.preferences.greeting_style, GreetingStyle::Concise);
        assert!(r.used_defaults);
    }
}
