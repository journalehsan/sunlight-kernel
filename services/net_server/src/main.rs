#![no_std]
#![no_main]

extern crate alloc;

use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress};
use sunlight_ipc::{
    debug_log, endpoint_create, getpid, ipc_call, ipc_call_timeout, ipc_complete_deferred_reply,
    ipc_defer_reply, ipc_deferred_reply_is_live, ipc_recv, ipc_recv_timeout, ipc_reply,
    monotonic_millis, nameserver_lookup, nameserver_lookup_timeout, nameserver_register, shm_free,
    shm_map, CapabilityToken, DevicedMsg, DriverCaps, DriverKind, DriverState, EndpointId, IpcMsg,
    NetworkdMsg, ResolvedMsg, VfsMsg,
};
use sunlight_net::netop::{NetDiagnostic, NetOp, NetStatus};
use sunlight_net::{ProxyNetDevice, SocketIdentity, TcpError};

use linked_list_allocator::LockedHeap;

// A real freeing heap. The old never-freeing bump allocator leaked one small
// allocation per RECV/SEND and would eventually OOM this long-running daemon
// once it started buffering real traffic.
#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB
static mut HEAP_MEM: [u8; HEAP_SIZE] = [0; HEAP_SIZE];

/// One shared physical page — the bulk-transfer unit for SEND_SHM / RECV_SHM.
const SHM_PAGE: usize = 4096;

/// Phase 3.0 resolver chain: /etc/hosts -> TTL cache -> upstream DNS (Phase 3.1/3.2).
/// Populated at init from /etc/hosts; can be refreshed via NetOp::RELOAD_HOSTS.
static mut RESOLVER_CHAIN: Option<sunlight_net::ResolverChain> = None;

/// Phase 3.4: smoltcp interface + frame-proxy device, used by the RESOLVE
/// handler's upstream DNS fallback. Built once at startup from the same
/// static QEMU user-net config as NetOp::GETIP (10.0.2.15/24 via 10.0.2.2).
static mut NET_DEVICE: Option<ProxyNetDevice> = None;
static mut NET_IFACE: Option<Interface> = None;
static mut LIVE_CONFIG: Option<sunlight_net::DhcpConfig> = None;
/// Backing storage for the TCP SocketSet managed by TcpManager (main loop).
static mut SOCKET_STORAGE: [SocketStorage; 128] = [SocketStorage::EMPTY; 128];
/// Separate backing storage for the DNS SocketSet (RESOLVE handler).
/// Kept isolated so DNS UDP sockets never alias TCP slots in SOCKET_STORAGE.
static mut DNS_SOCKET_STORAGE: [SocketStorage; 4] = [SocketStorage::EMPTY; 4];
static mut TCP_MANAGER: Option<sunlight_net::TcpManager> = None;
static mut WAITERS: Option<alloc::vec::Vec<SocketWaiter>> = None;
const DNS_FALLBACK_SERVERS: [[u8; 4]; 2] = [[208, 67, 222, 222], [208, 67, 220, 220]];
const MAX_WAIT_SET: usize = 32;
const MAX_WAITERS: usize = 64;
const NETWORK_TICK_MS: u64 = 10;

struct SocketWaiter {
    completion_token: u64,
    owner_pid: u64,
    identities: alloc::vec::Vec<SocketIdentity>,
    interest: u32,
    deadline_ms: u64,
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    unsafe {
        ALLOCATOR
            .lock()
            .init(core::ptr::addr_of_mut!(HEAP_MEM) as *mut u8, HEAP_SIZE);
    }

    // Antigravity: Initialize TCP manager dynamically
    unsafe {
        TCP_MANAGER = Some(sunlight_net::TcpManager::new());
        WAITERS = Some(alloc::vec::Vec::new());
    }

    // Note: Cannot do port I/O from user space (ring 3)
    // The kernel will handle PCI scanning and device initialization
    // This service registers with the name server and handles network IPC

    // Create endpoint and register with name server
    let ep = endpoint_create();
    nameserver_register("net", ep);
    debug_log("[NET]  Registered as 'net' with init");
    let nic_info = wait_for_publishable_backend(ep);

    // Construct the same backend-neutral frame proxy for either hardware
    // driver before any deviced/networkd publication is attempted.
    let mac = EthernetAddress(nic_info.mac);
    let mut device = ProxyNetDevice::new(mac.0);
    let config = Config::new(HardwareAddress::Ethernet(mac));
    let mut iface = Interface::new(config, &mut device, Instant::from_millis(0));
    let marked =
        sunlight_ipc::net_mark_backend_event(sunlight_ipc::NetBackendEvent::FrameProxyRegistered);
    debug_log(&alloc::format!(
        "[NET] generic frame backend registered kind={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        nic_info.backend.map(|kind| kind.interface_kind().label()).unwrap_or("unknown"),
        nic_info.mac[0], nic_info.mac[1], nic_info.mac[2], nic_info.mac[3], nic_info.mac[4],
        nic_info.mac[5]
    ));
    if marked.as_ref().and_then(|info| info.backend) != nic_info.backend {
        debug_log("[NET] frame backend authoritative re-query mismatch");
    }
    if !register_with_deviced(nic_info.backend) {
        debug_log("[NET] interface publication failed at deviced registration");
    }
    debug_log("[NET]  NetOp handlers registered");

    // Phase 3.4: bring up smoltcp over the backend-neutral kernel frame proxy.
    debug_log(&alloc::format!(
        "[NET] backend MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        nic_info.mac[0],
        nic_info.mac[1],
        nic_info.mac[2],
        nic_info.mac[3],
        nic_info.mac[4],
        nic_info.mac[5]
    ));
    debug_log(&alloc::format!(
        "[NET] frame-proxy MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        device.mac_address()[0],
        device.mac_address()[1],
        device.mac_address()[2],
        device.mac_address()[3],
        device.mac_address()[4],
        device.mac_address()[5]
    ));
    debug_log(&alloc::format!(
        "[NET] smoltcp MAC={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        mac.0[0],
        mac.0[1],
        mac.0[2],
        mac.0[3],
        mac.0[4],
        mac.0[5]
    ));

    let sockets_storage: &'static mut [SocketStorage; 128] =
        unsafe { &mut *core::ptr::addr_of_mut!(SOCKET_STORAGE) };
    let mut sockets = SocketSet::new(&mut sockets_storage[..]);
    let lease = sunlight_net::acquire_lease(&mut iface, &mut sockets, &mut device).ok();

    let hosts_content = load_hosts_from_vfs();
    let mut chain = sunlight_net::ResolverChain::new(&hosts_content);
    if let Some(ref config) = lease {
        if config.dns[0] != [0, 0, 0, 0] {
            chain.upstream = config.dns[0];
        }
    }
    refresh_upstream_from_resolved(&mut chain);
    unsafe {
        RESOLVER_CHAIN = Some(chain);
        LIVE_CONFIG = lease;
        NET_DEVICE = Some(device);
        NET_IFACE = Some(iface);
    }
    debug_log("[DNS] /etc/hosts loaded into resolver chain");
    if nic_info.link_up {
        debug_log("[NET]  Ethernet backend link up");
    } else {
        debug_log("[NET]  Ethernet backend link down");
    }
    if unsafe { LIVE_CONFIG.is_some() } {
        let lease = unsafe { LIVE_CONFIG.as_ref().unwrap() };
        debug_log(&alloc::format!(
            "[DHCP] lease acquired address={}.{}.{}.{}/{} gateway={}.{}.{}.{} dns={}.{}.{}.{}",
            lease.ip[0],
            lease.ip[1],
            lease.ip[2],
            lease.ip[3],
            lease.mask,
            lease.gateway[0],
            lease.gateway[1],
            lease.gateway[2],
            lease.gateway[3],
            lease.dns[0][0],
            lease.dns[0][1],
            lease.dns[0][2],
            lease.dns[0][3]
        ));
    } else {
        debug_log("[DHCP] no lease acquired");
    }
    debug_log("[NET]  smoltcp interface up over kernel frame proxy");

    // A bounded timed receive drives the frame-proxy cadence. Socket waiters
    // themselves sleep in the kernel and are completed only by a state change,
    // close, cancellation, or their deadline.
    let mut next_msg = Some(ipc_recv(ep));
    loop {
        progress_network(&mut sockets);
        complete_waiters(&mut sockets);

        let msg = match next_msg.take() {
            Some(msg) => msg,
            None => match ipc_recv_timeout(ep, NETWORK_TICK_MS) {
                Some(msg) => msg,
                None => continue,
            },
        };

        if msg.label == NetOp::WAIT {
            if let Some(reply) = handle_wait(msg, &mut sockets) {
                ipc_reply(reply);
            }
        } else {
            ipc_reply(handle_msg(msg, &mut sockets));
        }
    }
}

fn pack_short_name(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut word = 0u64;
    let mut i = 0usize;
    while i < bytes.len().min(8) {
        word |= (bytes[i] as u64) << (i * 8);
        i += 1;
    }
    word
}

fn register_with_deviced(backend: Option<sunlight_ipc::NetworkBackendKind>) -> bool {
    let Some(backend) = backend else {
        debug_log("[NET] no initialized backend; skipping deviced registration");
        return false;
    };
    let meta = (DriverKind::Network as u64) | ((DriverState::Ready as u64) << 16);
    let caps = DriverCaps::BUS
        | DriverCaps::NETWORK
        | if backend == sunlight_ipc::NetworkBackendKind::VirtioNet {
            DriverCaps::VIRTIO
        } else {
            0
        };
    for _ in 0..8 {
        if let Some(cap) = nameserver_lookup_timeout("deviced", 100) {
            let msg = IpcMsg::with_label(DevicedMsg::REGISTER_DRIVER)
                .word(0, pack_short_name(backend.driver_name()))
                .word(1, getpid())
                .word(2, meta)
                .word(3, caps);
            if let Ok(reply) = ipc_call_timeout(cap, msg, 100) {
                if reply.label == DevicedMsg::REPLY {
                    debug_log(&alloc::format!(
                        "[NET] registered {} with deviced",
                        backend.driver_name()
                    ));
                    return true;
                }
            }
        }
        sunlight_ipc::process_yield();
    }
    debug_log("[NET]  deviced registration failed");
    false
}

fn wait_for_publishable_backend(ep: EndpointId) -> sunlight_ipc::NetDeviceInfo {
    let mut unavailable_logged = false;
    loop {
        if let Some(info) = sunlight_ipc::net_device_info() {
            if info.publishable() {
                debug_log(&alloc::format!(
                    "[NET] active backend query returned {}",
                    info.backend
                        .map(|kind| kind.interface_kind().label())
                        .unwrap_or("unknown")
                ));
                return info;
            }
            if !unavailable_logged {
                debug_log(&alloc::format!(
                    "[NET] backend unavailable state={} error={} stage={} detail={:#x}",
                    info.state,
                    sunlight_ipc::Vmxnet3ErrorCode::from_u64(info.error).label(),
                    info.vmxnet3_stage.label(),
                    info.vmxnet3_error_detail
                ));
                debug_log("[NET] DHCP disabled until a hardware frame backend is registered");
                unavailable_logged = true;
            }
        } else if !unavailable_logged {
            debug_log("[NET] frame backend query failed; waiting for authoritative backend");
            unavailable_logged = true;
        }

        let msg = ipc_recv(ep);
        let reply = match msg.label {
            NetOp::GET_BACKEND => backend_info_reply(),
            NetOp::GETIP => IpcMsg::with_label(NetOp::GETIP)
                .word(0, 0)
                .word(1, 0)
                .word(2, 0)
                .word(3, 0),
            _ => IpcMsg::with_label(msg.label).word(0, 0),
        };
        ipc_reply(reply);
    }
}

fn backend_info_reply() -> IpcMsg {
    let info = sunlight_ipc::net_device_info().unwrap_or_default();
    let mut mac_bytes = [0u8; 8];
    mac_bytes[..6].copy_from_slice(&info.mac);
    IpcMsg::with_label(NetOp::GET_BACKEND)
        .word(0, info.backend.map(|kind| kind as u64).unwrap_or(0))
        .word(1, u64::from_le_bytes(mac_bytes))
        .word(
            2,
            (info.present as u64) | ((info.link_up as u64) << 1) | ((info.mtu as u64) << 16),
        )
        .word(3, info.state | (info.error << 32))
}

/// Best-effort: pull first DNS server from resolved and use as our upstream.
/// If resolved absent or returns no usable server, leave chain unchanged (old behavior).
fn refresh_upstream_from_resolved(chain: &mut sunlight_net::ResolverChain) {
    let Some(cap) = nameserver_lookup_timeout("resolved", 30) else {
        return; // resolved not present — keep working config (QEMU/DHCP)
    };
    // Prefer first listed server via GET_SERVER(0)
    let r = ipc_call(cap, IpcMsg::with_label(ResolvedMsg::GET_SERVER).word(0, 0));
    if r.label == ResolvedMsg::REPLY && r.word_count >= 1 {
        let addr = unpack_ipv4(r.words[0]);
        if addr != [0, 0, 0, 0] {
            chain.upstream = addr;
            debug_log("[DNS] upstream refreshed from resolved");
        }
    } else {
        // Fallback: GET_CONFIG gives first in w1
        let r2 = ipc_call(cap, IpcMsg::with_label(ResolvedMsg::GET_CONFIG));
        if r2.label == ResolvedMsg::REPLY && r2.word_count >= 2 {
            let addr = unpack_ipv4(r2.words[1]);
            if addr != [0, 0, 0, 0] {
                chain.upstream = addr;
                debug_log("[DNS] upstream from resolved GET_CONFIG");
            }
        }
    }
}

/// Best-effort query to networkd for the current default route or eth0 config.
/// Falls back to the classic QEMU slirp numbers if networkd is absent or has no data.
/// This keeps existing behavior when networkd is not present.
fn try_get_config_from_networkd() -> Option<([u8; 4], u8, [u8; 4], [u8; 4])> {
    let cap = nameserver_lookup_timeout("networkd", 60)?;
    // Prefer default route info; follow up with GET_INTERFACE for accurate prefix
    let r = ipc_call_timeout(cap, IpcMsg::with_label(NetworkdMsg::GET_DEFAULT_ROUTE), 80).ok()?;
    if r.label == NetworkdMsg::REPLY && r.words[0] != 0 {
        let addr = unpack_ipv4(r.words[2]);
        let gw = unpack_ipv4(r.words[3]);
        if addr != [0, 0, 0, 0] {
            // Try to get precise prefix for this id (or by name)
            let id = r.words[0];
            if let Ok(gi) = ipc_call_timeout(
                cap,
                IpcMsg::with_label(NetworkdMsg::GET_INTERFACE).word(0, id),
                40,
            ) {
                if let Some(s) = sunlight_ipc::unpack_iface_summary(&gi) {
                    if s.addr == addr {
                        return Some((addr, s.prefix.max(1), gw, [0, 0, 0, 0]));
                    }
                }
            }
            // fallback prefix
            return Some((addr, 24, gw, [0, 0, 0, 0]));
        }
    }
    // Fallback: ask specifically for eth0 (or lo will be ignored by callers)
    for name in ["eth0", "eth1", "virtio"].iter() {
        let key = pack_short_name(name);
        let r = ipc_call_timeout(
            cap,
            IpcMsg::with_label(NetworkdMsg::GET_INTERFACE).word(0, key),
            60,
        )
        .ok()?;
        if r.label == NetworkdMsg::REPLY {
            if let Some(s) = sunlight_ipc::unpack_iface_summary(&r) {
                if s.addr != [0, 0, 0, 0] || s.mode != sunlight_ipc::IpConfigMode::None {
                    return Some((s.addr, s.prefix.max(1), s.gw, s.dns));
                }
            }
        }
    }
    None
}

// Register IPC transports only words[0..4]. For SEND/RECV: words[0]=socket_id,
// words[1]=length, so words[2..4] = 2 × 8 = 16 bytes of payload survive.
const IPC_CHUNK: usize = 16;

fn handle_msg(msg: IpcMsg, sockets: &mut SocketSet<'static>) -> IpcMsg {
    match msg.label {
        NetOp::GET_BACKEND => backend_info_reply(),
        NetOp::GET_DIAGNOSTIC => {
            let value = unsafe {
                TCP_MANAGER
                    .as_ref()
                    .map(|tcp| tcp_diagnostic(tcp.diagnostics(), msg.words[0]))
                    .unwrap_or(0)
            };
            IpcMsg::with_label(NetOp::GET_DIAGNOSTIC).word(0, value)
        }
        NetOp::GETIP => {
            // Ask networkd first for explicit policy; otherwise report the live DHCP lease.
            if let Some((raw_ip, raw_pfx, raw_gw, dns_from_net)) = try_get_config_from_networkd() {
                debug_log("[NET] GETIP served from networkd policy");
                IpcMsg::with_label(NetOp::GETIP)
                    .word(0, pack_ipv4(raw_ip))
                    .word(1, raw_pfx as u64)
                    .word(2, pack_ipv4(raw_gw))
                    .word(3, pack_ipv4(dns_from_net))
            } else if let Some(config) = unsafe { LIVE_CONFIG.as_ref() } {
                IpcMsg::with_label(NetOp::GETIP)
                    .word(0, pack_ipv4(config.ip))
                    .word(1, config.mask as u64)
                    .word(2, pack_ipv4(config.gateway))
                    .word(3, pack_ipv4(config.dns[0]))
            } else {
                IpcMsg::with_label(NetOp::GETIP)
                    .word(0, 0)
                    .word(1, 0)
                    .word(2, 0)
                    .word(3, 0)
            }
        }
        NetOp::SOCKET => {
            let owner_pid = msg.badge;
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.alloc_socket(owner_pid, sockets))
            };
            socket_reply(
                NetOp::SOCKET,
                result.map_or(0, |identity| identity.0),
                result.err(),
            )
        }
        NetOp::CONNECT => {
            let identity = SocketIdentity(msg.words[0]);
            let ip = unpack_ipv4(msg.words[1]);
            let port = msg.words[2] as u16;
            let result = unsafe {
                match (NET_IFACE.as_mut(), NET_DEVICE.as_mut()) {
                    (Some(iface), Some(device)) => TCP_MANAGER
                        .as_mut()
                        .ok_or(TcpError::SocketError)
                        .and_then(|tcp| {
                            tcp.connect(msg.badge, identity, ip, port, iface, sockets, device)
                        }),
                    _ => Err(TcpError::SocketError),
                }
            };
            if result.is_err() {
                unsafe {
                    if let Some(tcp) = TCP_MANAGER.as_mut() {
                        let _ = tcp.close(msg.badge, identity, sockets);
                    }
                }
            }
            socket_reply(NetOp::CONNECT, u64::from(result.is_ok()), result.err())
        }
        NetOp::SEND => {
            let identity = SocketIdentity(msg.words[0]);
            let len = msg.words[1] as usize;
            let data = unpack_chunk(&msg.words, len.min(IPC_CHUNK));
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.send(msg.badge, identity, &data, sockets))
            };
            socket_reply(NetOp::SEND, result.unwrap_or(0) as u64, result.err())
        }
        NetOp::RECV => {
            let identity = SocketIdentity(msg.words[0]);
            let max_len = (msg.words[1] as usize).min(IPC_CHUNK).max(1);
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.recv(msg.badge, identity, max_len, sockets))
            };
            match result {
                Ok(data) => pack_recv_reply(data, None),
                Err(error) => pack_recv_reply(alloc::vec::Vec::new(), Some(error)),
            }
        }
        NetOp::SEND_SHM => {
            let identity = SocketIdentity(msg.words[0]);
            let len = (msg.words[1] as usize).min(SHM_PAGE);
            let tok = msg.caps[0];
            let data = if tok != CapabilityToken::INVALID && len > 0 {
                match shm_map(tok) {
                    // SAFETY: kernel guarantees the mapped page is at least one
                    // page (>= SHM_PAGE >= len) and valid for the call duration.
                    Ok(ptr) => {
                        let data = unsafe { core::slice::from_raw_parts(ptr, len) }.to_vec();
                        let _ = shm_free(tok);
                        data
                    }
                    Err(_) => alloc::vec::Vec::new(),
                }
            } else {
                alloc::vec::Vec::new()
            };
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.send(msg.badge, identity, &data, sockets))
            };
            socket_reply(NetOp::SEND_SHM, result.unwrap_or(0) as u64, result.err())
        }
        NetOp::RECV_SHM => {
            let identity = SocketIdentity(msg.words[0]);
            let max_len = (msg.words[1] as usize).min(SHM_PAGE).max(1);
            let tok = msg.caps[0];
            if tok == CapabilityToken::INVALID {
                return socket_reply(NetOp::RECV_SHM, 0, Some(TcpError::InvalidState));
            }
            let ptr = match shm_map(tok) {
                Ok(ptr) => ptr,
                Err(_) => return socket_reply(NetOp::RECV_SHM, 0, Some(TcpError::SocketError)),
            };
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.recv(msg.badge, identity, max_len, sockets))
            };
            let (data, error) = match result {
                Ok(data) => (data, None),
                Err(error) => (alloc::vec::Vec::new(), Some(error)),
            };
            if data.is_empty() {
                let _ = shm_free(tok);
                socket_reply(NetOp::RECV_SHM, 0, error)
            } else {
                let n = data.len().min(SHM_PAGE);
                // SAFETY: caller provided a mapped page of at least SHM_PAGE bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(data.as_ptr(), ptr, n);
                }
                let _ = shm_free(tok);
                IpcMsg::with_label(NetOp::RECV_SHM)
                    .word(0, n as u64)
                    .word(1, NetStatus::OK)
            }
        }
        NetOp::CLOSE => {
            let identity = SocketIdentity(msg.words[0]);
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.close(msg.badge, identity, sockets))
            };
            socket_reply(NetOp::CLOSE, u64::from(result.is_ok()), result.err())
        }
        NetOp::BIND => {
            let identity = SocketIdentity(msg.words[0]);
            let (addr, port) = if msg.word_count >= 3 {
                (unpack_ipv4(msg.words[1]), msg.words[2] as u16)
            } else {
                ([0, 0, 0, 0], msg.words[1] as u16)
            };
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.bind(msg.badge, identity, addr, port, sockets))
            };
            socket_reply(NetOp::BIND, u64::from(result.is_err()), result.err())
        }
        NetOp::LISTEN => {
            let identity = SocketIdentity(msg.words[0]);
            let backlog = msg.words[1] as usize;
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.listen(msg.badge, identity, backlog, sockets))
            };
            socket_reply(NetOp::LISTEN, u64::from(result.is_err()), result.err())
        }
        NetOp::ACCEPT => {
            let identity = SocketIdentity(msg.words[0]);
            let result = unsafe {
                TCP_MANAGER
                    .as_mut()
                    .ok_or(TcpError::SocketError)
                    .and_then(|tcp| tcp.take_accepted(msg.badge, identity))
            };
            socket_reply(
                NetOp::ACCEPT,
                result.map_or(0, |client| client.0),
                result.err(),
            )
        }
        NetOp::POLL => {
            let count = msg.words[0].min(3) as usize;
            let mut ready = [0u64; 3];
            let mut ready_count = 0usize;
            for i in 0..count {
                let identity = SocketIdentity(msg.words[i + 1]);
                let is_ready = unsafe {
                    TCP_MANAGER
                        .as_ref()
                        .and_then(|tcp| tcp.ready(msg.badge, identity, sockets).ok())
                        .is_some_and(|ready| ready.bits() != 0)
                };
                if is_ready {
                    ready[ready_count] = identity.0;
                    ready_count += 1;
                }
            }
            let mut reply = IpcMsg::with_label(NetOp::POLL).word(0, ready_count as u64);
            for (index, identity) in ready.into_iter().take(ready_count).enumerate() {
                reply = reply.word(index + 1, identity);
            }
            reply
        }
        NetOp::RESOLVE => {
            // Phase 3.0: resolver chain - /etc/hosts -> TTL cache -> upstream DNS.
            // Unpack hostname from client (same packing as before for RESOLVE).
            let name_len = msg.words[0] as usize;
            let mut name_buf = [0u8; 64];
            let mut collected = 0usize;
            for wi in 1..8 {
                if collected >= name_len {
                    break;
                }
                let w = msg.words[wi];
                for j in 0..8 {
                    if collected >= name_len {
                        break;
                    }
                    name_buf[collected] = ((w >> (j * 8)) & 0xff) as u8;
                    collected += 1;
                }
            }
            let hostname =
                core::str::from_utf8(&name_buf[..core::cmp::min(name_len, 63)]).unwrap_or("");
            let now = sunlight_ipc::get_time_utc();
            debug_log(&alloc::format!(
                "[DNSDBG] resolve request host='{}' len={} now={}",
                hostname,
                name_len,
                now
            ));

            let ip = unsafe {
                // SAFETY: RESOLVER_CHAIN, NET_DEVICE and NET_IFACE are each
                // initialized exactly once before the receive loop. This handler
                // is the only place that reads or mutates them (single-threaded
                // IPC service), so &mut access here never aliases.
                if let Some(ref mut chain) = RESOLVER_CHAIN {
                    match chain.resolve_local(hostname, now) {
                        Some(ip) => {
                            debug_log(&alloc::format!(
                                "[DNSDBG] local hit host='{}' ip={}.{}.{}.{}",
                                hostname,
                                ip[0],
                                ip[1],
                                ip[2],
                                ip[3]
                            ));
                            Some(ip)
                        }
                        None => {
                            debug_log(&alloc::format!(
                                "[DNSDBG] local miss host='{}' upstream={}.{}.{}.{}",
                                hostname,
                                chain.upstream[0],
                                chain.upstream[1],
                                chain.upstream[2],
                                chain.upstream[3]
                            ));
                            // v0: consult resolved for current config (best effort)
                            refresh_upstream_from_resolved(chain);
                            // Phase 3.1/3.4: fall through to upstream DNS-over-UDP via
                            // the kernel frame proxy.
                            match (NET_IFACE.as_mut(), NET_DEVICE.as_mut()) {
                                (Some(iface), Some(device)) => {
                                    let mut sockets = SocketSet::new(&mut DNS_SOCKET_STORAGE[..]);
                                    match sunlight_net::dns::upstream::query_a(
                                        hostname,
                                        chain.upstream,
                                        iface,
                                        &mut sockets,
                                        device,
                                    ) {
                                        Ok((ip, ttl)) => {
                                            debug_log(&alloc::format!(
                                                "[DNSDBG] upstream ok host='{}' ip={}.{}.{}.{} ttl={}",
                                                hostname, ip[0], ip[1], ip[2], ip[3], ttl
                                            ));
                                            chain.cache_insert(hostname, ip, ttl, now);
                                            Some(ip)
                                        }
                                        Err(err) => {
                                            debug_log(&alloc::format!(
                                                "[DNSDBG] upstream err host='{}' err={:?}",
                                                hostname,
                                                err
                                            ));
                                            if err == sunlight_net::DnsError::Timeout {
                                                let mut resolved = None;
                                                for server in DNS_FALLBACK_SERVERS {
                                                    debug_log(&alloc::format!(
                                                        "[DNSDBG] fallback try host='{}' upstream={}.{}.{}.{}",
                                                        hostname, server[0], server[1], server[2], server[3]
                                                    ));
                                                    match sunlight_net::dns::upstream::query_a(
                                                        hostname,
                                                        server,
                                                        iface,
                                                        &mut sockets,
                                                        device,
                                                    ) {
                                                        Ok((ip, ttl)) => {
                                                            debug_log(&alloc::format!(
                                                                "[DNSDBG] fallback ok host='{}' ip={}.{}.{}.{} ttl={}",
                                                                hostname, ip[0], ip[1], ip[2], ip[3], ttl
                                                            ));
                                                            chain.cache_insert(
                                                                hostname, ip, ttl, now,
                                                            );
                                                            resolved = Some(ip);
                                                            break;
                                                        }
                                                        Err(fallback_err) => {
                                                            debug_log(&alloc::format!(
                                                                "[DNSDBG] fallback err host='{}' upstream={}.{}.{}.{} err={:?}",
                                                                hostname,
                                                                server[0], server[1], server[2], server[3],
                                                                fallback_err
                                                            ));
                                                        }
                                                    }
                                                }
                                                resolved
                                            } else {
                                                None // no route / NXDOMAIN / parse error
                                            }
                                        }
                                    }
                                }
                                _ => {
                                    debug_log(
                                        "[DNSDBG] upstream unavailable: iface/device missing",
                                    );
                                    None // interface not yet brought up
                                }
                            }
                        }
                    }
                } else {
                    debug_log("[DNSDBG] resolver chain missing");
                    None
                }
            };

            match ip {
                Some(ip) => IpcMsg::with_label(NetOp::RESOLVE).word(0, pack_ipv4(ip)),
                None => IpcMsg::with_label(NetOp::RESOLVE).word(0, 0), // "Name or service not known"
            }
        }
        NetOp::RELOAD_HOSTS => {
            // Phase 3.0: re-read /etc/hosts from VFS and atomically swap the table.
            let hosts_content = load_hosts_from_vfs();
            unsafe {
                // SAFETY: see RESOLVE above - single-threaded, exclusive access.
                if let Some(ref mut chain) = RESOLVER_CHAIN {
                    chain.reload_hosts(&hosts_content);
                }
            }
            IpcMsg::with_label(NetOp::RELOAD_HOSTS).word(0, 1)
        }
        NetOp::PING => {
            // ICMP echo (ping) over the real device.
            // words[0] = packed IPv4, words[1] = count (1..16).
            let target = unpack_ipv4(msg.words[0]);
            let count = msg.words[1].max(1).min(16) as u32;
            unsafe {
                // SAFETY: see RESOLVE above — single-threaded, exclusive access.
                match (NET_IFACE.as_mut(), NET_DEVICE.as_mut()) {
                    (Some(iface), Some(device)) => {
                        match sunlight_net::icmp::ping(target, count, iface, sockets, device) {
                            Ok(stats) => IpcMsg::with_label(NetOp::PING)
                                .word(0, 1)
                                .word(1, stats.packets_received as u64)
                                .word(
                                    2,
                                    if stats.packets_received > 0 {
                                        stats.total_rtt_ms / (stats.packets_received as u64)
                                    } else {
                                        0
                                    },
                                ),
                            Err(_) => IpcMsg::with_label(NetOp::PING).word(0, 0),
                        }
                    }
                    _ => IpcMsg::with_label(NetOp::PING).word(0, 0),
                }
            }
        }
        _ => IpcMsg::with_label(0).word(0, 0),
    }
}

fn unpack_ipv4(v: u64) -> [u8; 4] {
    [
        (v & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 24) & 0xff) as u8,
    ]
}

fn unpack_chunk(words: &[u64; 8], len: usize) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(len);
    for i in 0..len {
        let word = words[2 + i / 8];
        let byte_idx = i % 8;
        out.push(((word >> (byte_idx * 8)) & 0xff) as u8);
    }
    out
}

fn socket_status(error: Option<TcpError>) -> u64 {
    match error {
        None => NetStatus::OK,
        Some(TcpError::WouldBlock) => NetStatus::WOULD_BLOCK,
        Some(TcpError::Timeout) => NetStatus::TIMEOUT,
        Some(TcpError::Reset) => NetStatus::RESET,
        Some(TcpError::Closed) => NetStatus::CLOSED,
        Some(TcpError::InvalidSocket) => NetStatus::INVALID_SOCKET,
        Some(TcpError::AccessDenied) => NetStatus::ACCESS_DENIED,
        Some(TcpError::AddressInUse) => NetStatus::ADDRESS_IN_USE,
        Some(TcpError::InvalidState) => NetStatus::INVALID_STATE,
        Some(TcpError::NotConnected) | Some(TcpError::Refused) => NetStatus::NOT_CONNECTED,
        Some(TcpError::BacklogFull) => NetStatus::BACKLOG_FULL,
        Some(TcpError::NoSlots) => NetStatus::NO_SLOTS,
        Some(TcpError::SocketError) => NetStatus::INTERNAL,
    }
}

fn tcp_diagnostic(diagnostics: sunlight_net::TcpDiagnostics, selector: u64) -> u64 {
    match selector {
        NetDiagnostic::SOCKET_ALLOC_TOTAL => diagnostics.socket_alloc_total,
        NetDiagnostic::SOCKET_RELEASE_TOTAL => diagnostics.socket_release_total,
        NetDiagnostic::SOCKET_LIVE => diagnostics.socket_live,
        NetDiagnostic::SOCKET_PEAK_LIVE => diagnostics.socket_peak_live,
        NetDiagnostic::LISTENER_ALLOC_TOTAL => diagnostics.listener_alloc_total,
        NetDiagnostic::LISTENER_RELEASE_TOTAL => diagnostics.listener_release_total,
        NetDiagnostic::LISTENER_LIVE => diagnostics.listener_live,
        NetDiagnostic::STREAM_ALLOC_TOTAL => diagnostics.stream_alloc_total,
        NetDiagnostic::STREAM_RELEASE_TOTAL => diagnostics.stream_release_total,
        NetDiagnostic::STREAM_LIVE => diagnostics.stream_live,
        NetDiagnostic::RX_BUFFERS_LIVE => diagnostics.rx_buffers_live,
        NetDiagnostic::TX_BUFFERS_LIVE => diagnostics.tx_buffers_live,
        NetDiagnostic::RX_BYTES_RESERVED => diagnostics.rx_bytes_reserved,
        NetDiagnostic::TX_BYTES_RESERVED => diagnostics.tx_bytes_reserved,
        NetDiagnostic::ALLOCATION_FAILURES_TOTAL => diagnostics.allocation_failures_total,
        NetDiagnostic::ALLOCATION_ROLLBACKS_TOTAL => diagnostics.allocation_rollbacks_total,
        NetDiagnostic::FAILED_CONNECT_CLEANUP_TOTAL => diagnostics.failed_connect_cleanup_total,
        NetDiagnostic::PEER_RESET_REAPS_TOTAL => diagnostics.peer_reset_reaps_total,
        NetDiagnostic::HALF_CLOSE_REAPS_TOTAL => diagnostics.half_close_reaps_total,
        NetDiagnostic::CLOSE_DEADLINE_REAPS_TOTAL => diagnostics.close_deadline_reaps_total,
        NetDiagnostic::OWNER_EXIT_REAPS_TOTAL => diagnostics.owner_exit_reaps_total,
        _ => 0,
    }
}

fn socket_reply(label: u64, value: u64, error: Option<TcpError>) -> IpcMsg {
    IpcMsg::with_label(label)
        .word(0, value)
        .word(1, socket_status(error))
}

fn pack_recv_reply(data: alloc::vec::Vec<u8>, error: Option<TcpError>) -> IpcMsg {
    let len = data.len().min(IPC_CHUNK);
    let status = if data.is_empty() && error.is_none() {
        NetStatus::EOF
    } else {
        socket_status(error)
    };
    let mut reply = IpcMsg::with_label(NetOp::RECV)
        .word(0, len as u64)
        .word(1, status);
    for i in 0..len {
        let word_idx = 2 + i / 8;
        let byte_idx = i % 8;
        let word = reply.words[word_idx];
        let byte = data[i] as u64;
        reply.words[word_idx] = word | (byte << (byte_idx * 8));
    }
    reply
}

fn progress_network(sockets: &mut SocketSet<'static>) {
    unsafe {
        if let (Some(tcp), Some(iface), Some(device)) = (
            TCP_MANAGER.as_mut(),
            NET_IFACE.as_mut(),
            NET_DEVICE.as_mut(),
        ) {
            tcp.progress(iface, sockets, device, monotonic_millis());
            tcp.reap_dead_owners(sockets, sunlight_ipc::process_is_alive);
        }
    }
}

fn first_ready(
    owner_pid: u64,
    identities: &[SocketIdentity],
    interest: u32,
    sockets: &SocketSet<'static>,
) -> Option<(SocketIdentity, u32)> {
    unsafe {
        let tcp = TCP_MANAGER.as_ref()?;
        for identity in identities {
            match tcp.ready(owner_pid, *identity, sockets) {
                Ok(ready)
                    if ready.bits() != 0
                        && (ready.bits() & interest != 0
                            || ready.bits()
                                & (sunlight_net::SocketReady::EOF
                                    | sunlight_net::SocketReady::RESET
                                    | sunlight_net::SocketReady::CLOSED
                                    | sunlight_net::SocketReady::ERROR)
                                != 0) =>
                {
                    return Some((*identity, ready.bits()))
                }
                Ok(_) | Err(TcpError::WouldBlock) => {}
                Err(error) => return Some((*identity, socket_ready_error(error))),
            }
        }
    }
    None
}

fn socket_ready_error(error: TcpError) -> u32 {
    match error {
        TcpError::Reset => sunlight_net::SocketReady::RESET,
        TcpError::Closed => sunlight_net::SocketReady::CLOSED,
        _ => sunlight_net::SocketReady::ERROR,
    }
}

fn wait_reply(status: u64, identity: SocketIdentity, ready_bits: u32) -> IpcMsg {
    IpcMsg::with_label(NetOp::WAIT)
        .word(0, status)
        .word(1, identity.0)
        .word(2, ready_bits as u64)
}

fn handle_wait(msg: IpcMsg, sockets: &mut SocketSet<'static>) -> Option<IpcMsg> {
    let count = (msg.words[0] as usize).min(MAX_WAIT_SET);
    let timeout_ms = msg.words[1];
    let interest = if msg.words[2] == 0 {
        u32::MAX
    } else {
        msg.words[2] as u32
    };
    if count == 0 || msg.caps[0] == CapabilityToken::INVALID {
        return Some(wait_reply(
            NetStatus::INVALID_STATE,
            SocketIdentity::INVALID,
            0,
        ));
    }
    let page = match shm_map(msg.caps[0]) {
        Ok(ptr) => ptr,
        Err(_) => return Some(wait_reply(NetStatus::INTERNAL, SocketIdentity::INVALID, 0)),
    };
    let identities = unsafe {
        core::slice::from_raw_parts(page as *const u64, count)
            .iter()
            .copied()
            .map(SocketIdentity)
            .collect::<alloc::vec::Vec<_>>()
    };
    let _ = shm_free(msg.caps[0]);
    if identities
        .iter()
        .any(|identity| *identity == SocketIdentity::INVALID)
    {
        return Some(wait_reply(
            NetStatus::INVALID_SOCKET,
            SocketIdentity::INVALID,
            0,
        ));
    }
    if let Some((identity, ready)) = first_ready(msg.badge, &identities, interest, sockets) {
        return Some(wait_reply(NetStatus::OK, identity, ready));
    }
    if timeout_ms == 0 {
        return Some(wait_reply(
            NetStatus::WOULD_BLOCK,
            SocketIdentity::INVALID,
            0,
        ));
    }
    let completion_token = match ipc_defer_reply() {
        Ok(token) => token,
        Err(error) => {
            return Some(wait_reply(
                socket_status(Some(match error {
                    sunlight_ipc::IpcError::QueueFull => TcpError::NoSlots,
                    _ => TcpError::SocketError,
                })),
                SocketIdentity::INVALID,
                0,
            ));
        }
    };
    unsafe {
        let waiters = WAITERS.as_mut().expect("waiter table initialized");
        if waiters.len() >= MAX_WAITERS {
            let _ = ipc_complete_deferred_reply(
                completion_token,
                wait_reply(NetStatus::NO_SLOTS, SocketIdentity::INVALID, 0),
            );
            return None;
        }
        waiters.push(SocketWaiter {
            completion_token,
            owner_pid: msg.badge,
            identities,
            interest,
            deadline_ms: monotonic_millis().saturating_add(timeout_ms),
        });
    }
    None
}

fn complete_waiters(sockets: &mut SocketSet<'static>) {
    let now = monotonic_millis();
    let mut pending =
        unsafe { core::mem::take(WAITERS.as_mut().expect("waiter table initialized")) };
    let mut retained = alloc::vec::Vec::with_capacity(pending.len());
    for waiter in pending.drain(..) {
        if !ipc_deferred_reply_is_live(waiter.completion_token) {
            continue;
        }
        let reply = if let Some((identity, ready)) = first_ready(
            waiter.owner_pid,
            &waiter.identities,
            waiter.interest,
            sockets,
        ) {
            Some(wait_reply(NetStatus::OK, identity, ready))
        } else if now >= waiter.deadline_ms {
            Some(wait_reply(NetStatus::TIMEOUT, SocketIdentity::INVALID, 0))
        } else {
            None
        };
        if let Some(reply) = reply {
            let _ = ipc_complete_deferred_reply(waiter.completion_token, reply);
        } else {
            retained.push(waiter);
        }
    }
    unsafe {
        *WAITERS.as_mut().expect("waiter table initialized") = retained;
    }
}

fn pack_ipv4(ip: [u8; 4]) -> u64 {
    (ip[0] as u64) | ((ip[1] as u64) << 8) | ((ip[2] as u64) << 16) | ((ip[3] as u64) << 24)
}

/// Read `/etc/hosts` via the VFS capability and return its UTF-8 content.
/// Used at startup and on every NetOp::RELOAD_HOSTS.
fn load_hosts_from_vfs() -> alloc::string::String {
    if let Some(vfs_cap) = nameserver_lookup("vfs") {
        let data = read_file_simple(vfs_cap, "/etc/hosts");
        // SAFETY: from_utf8_lossy is safe on arbitrary bytes; parse_hosts copies
        // names into owned Strings so this temporary buffer can be dropped.
        alloc::string::String::from_utf8_lossy(&data).into_owned()
    } else {
        alloc::string::String::new()
    }
}

/// Minimal VFS file reader for net_server (used only for /etc/hosts at init).
/// 16-byte chunks via VfsMsg READ; data returned packed in reply.words[2..].
/// SAFETY comments only on the bump alloc (existing); this path has no raw pointers.
fn read_file_simple(vfs_cap: CapabilityToken, path: &str) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::new();

    // OPEN
    let open_msg = path_msg(VfsMsg::OPEN, path);
    let reply = ipc_call(vfs_cap, open_msg);
    if reply.label != VfsMsg::REPLY || reply.words[0] != 0 {
        return out;
    }
    let handle = reply.words[1] as u32;

    let mut offset = 0usize;
    loop {
        let read_msg = IpcMsg::with_label(VfsMsg::READ)
            .word(0, handle as u64)
            .word(1, offset as u64)
            .word(2, 16);
        let reply = ipc_call(vfs_cap, read_msg);
        if reply.label != VfsMsg::REPLY {
            break;
        }
        let n = reply.words[1] as usize;
        if n == 0 {
            break;
        }
        // data packed in words[2] and [3] (up to 16 bytes)
        let src = [
            reply.words.get(2).copied().unwrap_or(0),
            reply.words.get(3).copied().unwrap_or(0),
        ];
        for i in 0..n {
            let word_idx = i / 8;
            let byte_idx = i % 8;
            out.push(((src[word_idx] >> (byte_idx * 8)) & 0xFF) as u8);
        }
        offset += n;
    }

    // CLOSE (best effort)
    let _ = ipc_call(
        vfs_cap,
        IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64),
    );
    out
}

/// Pack a path into the first 4 words (same as sunshell VFS client).
fn path_msg(label: u64, path: &str) -> IpcMsg {
    let bytes = path.as_bytes();
    let mut msg = IpcMsg::with_label(label);
    for word_idx in 0..4 {
        let start = word_idx * 8;
        let end = (start + 8).min(bytes.len());
        if start < bytes.len() {
            let mut word = 0u64;
            for (i, &b) in bytes[start..end].iter().enumerate() {
                word |= (b as u64) << (i * 8);
            }
            msg = msg.word(word_idx, word);
        }
    }
    msg
}
