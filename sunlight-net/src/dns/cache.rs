//! Phase 3.2: in-memory TTL cache for resolved A records.
//!
//! Keyed by lowercase hostname. Expiry is tracked as an absolute Unix
//! timestamp (seconds), computed from the upstream response's TTL at
//! insertion time using whatever "now" the caller supplies (net_server
//! gets this from `sunlight_ipc::get_time_utc()`).

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};

#[derive(Debug, Clone, Copy)]
struct CacheEntry {
    addr: [u8; 4],
    expires_at: u64,
}

pub struct DnsCache {
    entries: BTreeMap<String, CacheEntry>,
}

impl DnsCache {
    pub fn new() -> Self {
        DnsCache { entries: BTreeMap::new() }
    }

    /// Look up `hostname`. Returns `None` if absent or expired.
    /// Expired entries are lazily removed on access.
    pub fn get(&mut self, hostname: &str, now: u64) -> Option<[u8; 4]> {
        let key = hostname.to_ascii_lowercase();
        match self.entries.get(&key) {
            Some(entry) if entry.expires_at > now => Some(entry.addr),
            Some(_) => {
                self.entries.remove(&key);
                None
            }
            None => None,
        }
    }

    /// Insert/refresh `hostname` -> `addr`, expiring `ttl_secs` from `now`.
    /// A TTL of 0 is treated as "do not cache".
    pub fn insert(&mut self, hostname: &str, addr: [u8; 4], ttl_secs: u32, now: u64) {
        if ttl_secs == 0 {
            return;
        }
        self.entries.insert(
            hostname.to_ascii_lowercase().to_string(),
            CacheEntry { addr, expires_at: now + ttl_secs as u64 },
        );
    }

    /// Drop all expired entries (call periodically to bound memory use).
    pub fn purge_expired(&mut self, now: u64) {
        self.entries.retain(|_, entry| entry.expires_at > now);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_get() {
        let mut cache = DnsCache::new();
        cache.insert("example.com", [93, 184, 216, 34], 300, 1000);
        assert_eq!(cache.get("example.com", 1000), Some([93, 184, 216, 34]));
        assert_eq!(cache.get("EXAMPLE.COM", 1299), Some([93, 184, 216, 34]));
    }

    #[test]
    fn expires_after_ttl() {
        let mut cache = DnsCache::new();
        cache.insert("example.com", [93, 184, 216, 34], 300, 1000);
        assert_eq!(cache.get("example.com", 1300), None);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn zero_ttl_not_cached() {
        let mut cache = DnsCache::new();
        cache.insert("example.com", [93, 184, 216, 34], 0, 1000);
        assert_eq!(cache.get("example.com", 1000), None);
    }

    #[test]
    fn purge_expired_removes_only_stale() {
        let mut cache = DnsCache::new();
        cache.insert("a.com", [1, 1, 1, 1], 100, 1000);
        cache.insert("b.com", [2, 2, 2, 2], 1000, 1000);
        cache.purge_expired(1101);
        assert_eq!(cache.get("a.com", 1101), None);
        assert_eq!(cache.get("b.com", 1101), Some([2, 2, 2, 2]));
    }
}
