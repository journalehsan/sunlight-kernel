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
