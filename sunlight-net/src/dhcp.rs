/// DHCP configuration result
pub struct DhcpConfig {
    pub ip: [u8; 4],      // e.g. 10.0.2.15
    pub mask: [u8; 4],    // e.g. 255.255.255.0
    pub gateway: [u8; 4], // e.g. 10.0.2.2
    pub dns: [[u8; 4]; 2],// e.g. 10.0.2.3
    pub lease: u32,       // seconds
}

#[derive(Debug)]
pub enum DhcpError {
    Timeout,
    InvalidOffer,
}

/// Run DHCP to acquire an IP address
pub fn run_dhcp() -> Result<DhcpConfig, DhcpError> {
    // Phase 5.2: Full DHCP implementation with smoltcp DhcpSocket
    // For now, return stub values matching QEMU user-net defaults
    Ok(DhcpConfig {
        ip: [10, 0, 2, 15],
        mask: [255, 255, 255, 0],
        gateway: [10, 0, 2, 2],
        dns: [[10, 0, 2, 3], [0, 0, 0, 0]],
        lease: 3600,
    })
}
