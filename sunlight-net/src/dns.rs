use smoltcp::iface::{Interface, SocketSet};

#[derive(Debug)]
pub enum DnsError {
    NotFound,
    Timeout,
    QueryFailed,
}

/// Stub DNS resolver - returns common test IPs
/// Full DNS implementation requires smoltcp DnsSocket with network polling
pub fn resolve(
    hostname: &str,
    _dns_server: [u8; 4],
    _iface: &mut Interface,
    _sockets: &mut SocketSet,
    _device: &mut crate::device::SunlightNetDevice,
) -> Result<[u8; 4], DnsError> {
    // Phase 5.x.1: DNS resolution with simulated responses
    match hostname {
        "google.com" => Ok([142, 250, 185, 46]),
        "example.com" => Ok([93, 184, 216, 34]),
        "8.8.8.8" => Ok([8, 8, 8, 8]),
        _ => Err(DnsError::NotFound),
    }
}

/// 1.1 Combined resolver for net_server (userspace IPC path).
/// Loads /etc/hosts (via VFS read in net_server) at construction, then falls back to
/// the same hardcoded names used by the old stub.
/// Hosts entries take precedence. Memory: small BTreeMap<String, [u8;4]> + static fallback.
pub struct DnsResolver {
    hosts: crate::hosts::HostsTable,
}

impl DnsResolver {
    /// Create from the raw content of /etc/hosts (caller responsible for reading it via VFS IPC + capability).
    pub fn new(hosts_content: &str) -> Self {
        DnsResolver {
            hosts: crate::hosts::parse_hosts(hosts_content),
        }
    }

    /// resolve(hostname) -> Some(ip) using hosts first, then hardcoded.
    /// Returns None for unknown (caller turns into RESOLVE failure reply).
    pub fn resolve(&self, hostname: &str) -> Option<[u8; 4]> {
        if let Some(&ip) = self.hosts.get(hostname) {
            return Some(ip);
        }
        // Hardcoded fallback (kept in sync with previous net_server and kernel boot prints)
        match hostname {
            "google.com" => Some([142, 250, 185, 46]),
            "irancell.ir" => Some([91, 99, 12, 34]),
            "example.com" => Some([93, 184, 216, 34]),
            "localhost" => Some([127, 0, 0, 1]),
            _ => None,
        }
    }
}

// SAFETY: DnsResolver is Send/Sync because it only contains owned BTreeMap of Copy data (String + [u8;4]).
// No shared mutable state here; the net_server owns the instance behind a static mut with explicit unsafe.
unsafe impl Send for DnsResolver {}
unsafe impl Sync for DnsResolver {}
