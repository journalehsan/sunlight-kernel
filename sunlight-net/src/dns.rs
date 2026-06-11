#[derive(Debug)]
pub enum DnsError {
    NotFound,
    Timeout,
}

/// Resolve a hostname to an IPv4 address
pub fn resolve(_hostname: &str) -> Result<[u8; 4], DnsError> {
    // Phase 5.2: Full DNS resolution with smoltcp DnsSocket
    // For now, return stub values
    // google.com -> 142.250.x.x (example)
    Ok([142, 250, 185, 46])
}
