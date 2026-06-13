//! Dynamic DNS resolution chain (Phase 3.0-3.2):
//!
//! 1. `/etc/hosts` (highest priority, reloadable at runtime)
//! 2. In-memory TTL cache ([`cache::DnsCache`])
//! 3. Upstream DNS-over-UDP using a hand-written RFC 1035 codec ([`wire`], [`upstream`])
//!
//! mDNS (`.local`) is roadmap-stubbed for Phase 3.3 — see `ResolverChain::is_mdns_name`.

pub mod cache;
pub mod upstream;
pub mod wire;

use crate::hosts::{parse_hosts, HostsTable};
use cache::DnsCache;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsError {
    NotFound,
    Timeout,
    QueryFailed,
}

/// Default upstream resolver (Google public DNS), used until a
/// `/etc/resolv.conf` equivalent is implemented.
pub const DEFAULT_UPSTREAM: [u8; 4] = [8, 8, 8, 8];

/// Ordered resolver chain: hosts -> cache -> upstream.
///
/// `net_server` owns one instance for the process lifetime. `/etc/hosts`
/// is loaded at startup and can be reloaded on demand via
/// [`ResolverChain::reload_hosts`] (driven by an IPC reload command).
pub struct ResolverChain {
    hosts: HostsTable,
    cache: DnsCache,
    pub upstream: [u8; 4],
}

impl ResolverChain {
    pub fn new(hosts_content: &str) -> Self {
        ResolverChain {
            hosts: parse_hosts(hosts_content),
            cache: DnsCache::new(),
            upstream: DEFAULT_UPSTREAM,
        }
    }

    /// Re-parse `/etc/hosts` content and atomically swap the table in.
    /// Cache entries are left untouched (hosts always take precedence on
    /// lookup, so a newly-added host entry is reflected immediately).
    pub fn reload_hosts(&mut self, hosts_content: &str) {
        self.hosts = parse_hosts(hosts_content);
    }

    /// Step 1+2 of the chain: `/etc/hosts` then the TTL cache.
    /// Returns `None` if an upstream query is required.
    pub fn resolve_local(&mut self, hostname: &str, now: u64) -> Option<[u8; 4]> {
        if let Some(&ip) = self.hosts.get(hostname) {
            return Some(ip);
        }
        self.cache.get(hostname, now)
    }

    /// Step 3 result feeds back into the cache (step 2) for next time.
    pub fn cache_insert(&mut self, hostname: &str, addr: [u8; 4], ttl: u32, now: u64) {
        self.cache.insert(hostname, addr, ttl, now);
    }

    pub fn purge_expired_cache(&mut self, now: u64) {
        self.cache.purge_expired(now);
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Phase 3.3 roadmap stub: `.local` names are routed to mDNS
    /// (224.0.0.251:5353) instead of the upstream resolver. Not yet
    /// implemented — see ROADMAP.md.
    pub fn is_mdns_name(hostname: &str) -> bool {
        hostname.ends_with(".local")
    }
}

// SAFETY: ResolverChain owns only BTreeMaps of Copy/owned data (String,
// [u8;4]/u64). net_server stores it behind a `static mut` written once at
// startup and mutated only from the single-threaded IPC loop thereafter.
unsafe impl Send for ResolverChain {}
unsafe impl Sync for ResolverChain {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_take_priority_over_cache() {
        let mut chain = ResolverChain::new("127.0.0.1 example.com\n");
        chain.cache_insert("example.com", [9, 9, 9, 9], 300, 1000);
        assert_eq!(chain.resolve_local("example.com", 1000), Some([127, 0, 0, 1]));
    }

    #[test]
    fn cache_used_when_no_hosts_entry() {
        let mut chain = ResolverChain::new("");
        assert_eq!(chain.resolve_local("example.com", 1000), None);
        chain.cache_insert("example.com", [93, 184, 216, 34], 300, 1000);
        assert_eq!(chain.resolve_local("example.com", 1200), Some([93, 184, 216, 34]));
        assert_eq!(chain.resolve_local("example.com", 1301), None); // expired
    }

    #[test]
    fn reload_hosts_picks_up_new_entries() {
        let mut chain = ResolverChain::new("");
        assert_eq!(chain.resolve_local("sunlight.local", 0), None);
        chain.reload_hosts("10.0.0.5 sunlight.local\n");
        assert_eq!(chain.resolve_local("sunlight.local", 0), Some([10, 0, 0, 5]));
    }

    #[test]
    fn mdns_name_detection() {
        assert!(ResolverChain::is_mdns_name("printer.local"));
        assert!(!ResolverChain::is_mdns_name("example.com"));
    }
}
