use smoltcp::iface::{Interface, SocketSet};
use smoltcp::phy::Device;
use smoltcp::socket::dhcpv4::{Event as DhcpEvent, Socket as DhcpSocket};
use smoltcp::time::Instant;

const DHCP_ACQUISITION_TIMEOUT_MS: u64 = 30_000;

/// DHCP configuration result
#[derive(Debug, Clone)]
pub struct DhcpConfig {
    pub ip: [u8; 4],       // e.g. 10.0.2.15
    pub mask: u8,          // CIDR prefix length, e.g. 24
    pub gateway: [u8; 4],  // e.g. 10.0.2.2
    pub dns: [[u8; 4]; 2], // e.g. 10.0.2.3, 0.0.0.0
    pub lease: u32,        // seconds
}

#[derive(Debug)]
pub enum DhcpError {
    Timeout,
    InvalidOffer,
    SocketError,
}

/// Run DHCP to acquire an IP address (real smoltcp implementation)
pub fn acquire_lease<D: Device>(
    iface: &mut Interface,
    sockets: &mut SocketSet,
    device: &mut D,
) -> Result<DhcpConfig, DhcpError> {
    // SAFETY: Creating a DHCP socket to manage network configuration
    // The socket is added to the SocketSet and managed by the interface
    let dhcp_socket = DhcpSocket::new();
    let dhcp_handle = sockets.add(dhcp_socket);

    let started_at = sunlight_ipc::monotonic_millis();
    let deadline = started_at.saturating_add(DHCP_ACQUISITION_TIMEOUT_MS);
    loop {
        let now = sunlight_ipc::monotonic_millis();
        if now >= deadline {
            let _ =
                sunlight_ipc::net_mark_backend_event(sunlight_ipc::NetBackendEvent::DhcpTimeout);
            sunlight_ipc::debug_log("[DHCP] timeout waiting for lease");
            sockets.remove(dhcp_handle);
            return Err(DhcpError::Timeout);
        }
        let current_time = Instant::from_millis(now as i64);

        // Poll the network interface
        iface.poll(current_time, device, sockets);

        // Check DHCP socket for events
        let dhcp_socket = sockets.get_mut::<DhcpSocket>(dhcp_handle);
        match dhcp_socket.poll() {
            Some(DhcpEvent::Configured(config)) => {
                // Extract configuration
                let ip_addr = config.address.address();
                let ip_bytes = ip_addr.as_bytes();
                let gateway_ip = config
                    .router
                    .unwrap_or(smoltcp::wire::Ipv4Address::UNSPECIFIED);
                let gateway_bytes = gateway_ip.as_bytes();
                let prefix_len = config.address.prefix_len();

                // Extract DNS servers (up to 2)
                let mut dns_servers = [[0u8; 4]; 2];
                for (i, server) in config.dns_servers.iter().take(2).enumerate() {
                    let server_bytes = server.as_bytes();
                    dns_servers[i][0] = server_bytes[0];
                    dns_servers[i][1] = server_bytes[1];
                    dns_servers[i][2] = server_bytes[2];
                    dns_servers[i][3] = server_bytes[3];
                }

                // Apply configuration to interface
                iface.update_ip_addrs(|addrs| {
                    addrs.clear();
                    let cidr = smoltcp::wire::Ipv4Cidr::new(ip_addr, prefix_len);
                    let _ = addrs.push(smoltcp::wire::IpCidr::Ipv4(cidr));
                });

                // Add default gateway route
                if gateway_ip != smoltcp::wire::Ipv4Address::UNSPECIFIED {
                    let _ = iface.routes_mut().add_default_ipv4_route(gateway_ip);
                }

                // Remove DHCP socket before returning
                sockets.remove(dhcp_handle);
                let _ = sunlight_ipc::net_mark_backend_event(
                    sunlight_ipc::NetBackendEvent::DhcpLeaseInstalled,
                );

                return Ok(DhcpConfig {
                    ip: [ip_bytes[0], ip_bytes[1], ip_bytes[2], ip_bytes[3]],
                    mask: prefix_len,
                    gateway: [
                        gateway_bytes[0],
                        gateway_bytes[1],
                        gateway_bytes[2],
                        gateway_bytes[3],
                    ],
                    dns: dns_servers,
                    lease: 3600, // Default lease time
                });
            }
            Some(DhcpEvent::Deconfigured) => {
                sunlight_ipc::process_yield();
            }
            None => {
                // Yield between bounded polls. VMXNET3 completion processing is
                // driven by this network-service cadence, never a GUI tick.
                sunlight_ipc::process_yield();
            }
        }
    }
}
