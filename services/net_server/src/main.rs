#![no_std]
#![no_main]

extern crate alloc;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait, nameserver_lookup, nameserver_register, CapabilityToken, IpcMsg, VfsMsg,
};
use sunlight_net::netop::NetOp;
use sunlight_net::ProxyNetDevice;
use smoltcp::iface::{Config, Interface, SocketSet, SocketStorage};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr, Ipv4Address, Ipv4Cidr};

// Simple bump allocator for the network server
struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 65536] = [0; 65536];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

/// Phase 3.0 resolver chain: /etc/hosts -> TTL cache -> upstream DNS (Phase 3.1/3.2).
/// Populated at init from /etc/hosts; can be refreshed via NetOp::RELOAD_HOSTS.
static mut RESOLVER_CHAIN: Option<sunlight_net::ResolverChain> = None;

/// Phase 3.4: smoltcp interface + frame-proxy device, used by the RESOLVE
/// handler's upstream DNS fallback. Built once at startup from the same
/// static QEMU user-net config as NetOp::GETIP (10.0.2.15/24 via 10.0.2.2).
static mut NET_DEVICE: Option<ProxyNetDevice> = None;
static mut NET_IFACE: Option<Interface> = None;
/// Backing storage for the UDP socket `upstream::query_a` allocates per query.
static mut SOCKET_STORAGE: [SocketStorage; 4] = [SocketStorage::EMPTY; 4];
const DNS_FALLBACK_SERVERS: [[u8; 4]; 2] = [
    [8, 8, 8, 8],
    [1, 1, 1, 1],
];

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Note: Cannot do port I/O from user space (ring 3)
    // The kernel will handle PCI scanning and device initialization
    // This service registers with the name server and handles network IPC

    // Create endpoint and register with name server
    let ep = endpoint_create();
    nameserver_register("net", ep);
    debug_log("[NET]  Registered as 'net' with init");
    debug_log("[NET]  Interface: eth0 MAC=52:54:00:12:34:56");
    debug_log("[NET]  NetOp handlers registered");

    // Phase 3.0: VFS /etc/hosts -> ResolverChain (hosts -> TTL cache -> upstream DNS).
    // We read via capability-gated VFS IPC (existing VfsMsg + nameserver).
    let hosts_content = load_hosts_from_vfs();
    let mut chain = sunlight_net::ResolverChain::new(&hosts_content);
    // QEMU user-net (slirp) built-in DNS forwarder — reliably reachable inside
    // the guest's NAT'd subnet and forwards to the host's real resolver.
    chain.upstream = [10, 0, 2, 3];
    unsafe {
        // SAFETY: written exactly once, before any messages are handled (single-threaded
        // userspace service, no interrupts in this model). Subsequent reads/writes in
        // handle_msg happen after this point and never alias.
        RESOLVER_CHAIN = Some(chain);
    }
    debug_log("[DNS] /etc/hosts loaded into resolver chain");

    // Phase 3.4: bring up the smoltcp interface over the kernel frame proxy
    // using the same static QEMU user-net config as NetOp::GETIP.
    let mac = EthernetAddress([0x52, 0x54, 0x00, 0x12, 0x34, 0x56]);
    let mut device = ProxyNetDevice::new(mac.0);
    let config = Config::new(HardwareAddress::Ethernet(mac));
    let mut iface = Interface::new(config, &mut device, Instant::from_millis(0));
    iface.update_ip_addrs(|addrs| {
        let _ = addrs.push(IpCidr::Ipv4(Ipv4Cidr::new(Ipv4Address::new(10, 0, 2, 15), 24)));
    });
    let _ = iface.routes_mut().add_default_ipv4_route(Ipv4Address::new(10, 0, 2, 2));
    unsafe {
        // SAFETY: written exactly once before the receive loop; see RESOLVER_CHAIN.
        NET_DEVICE = Some(device);
        NET_IFACE = Some(iface);
    }
    debug_log("[NET]  smoltcp interface up over kernel frame proxy (10.0.2.15/24)");

    // Main service loop
    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle_msg(msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle_msg(msg: IpcMsg) -> IpcMsg {
    match msg.label {
        NetOp::GETIP => {
            // QEMU user-net defaults (also returned by real DHCP in a full impl).
            IpcMsg::with_label(NetOp::GETIP)
                .word(0, pack_ipv4([10, 0, 2, 15]))
                .word(1, 24)
                .word(2, pack_ipv4([10, 0, 2, 2]))
                .word(3, pack_ipv4([10, 0, 2, 3]))
        }
        NetOp::SOCKET => {
            // Minimal: allocate a synthetic socket id (we are a userspace service; real
            // smoltcp sockets would live in our SocketSet). Return id=1 as "ok".
            IpcMsg::with_label(NetOp::SOCKET).word(0, 1)
        }
        NetOp::CONNECT | NetOp::SEND | NetOp::RECV | NetOp::CLOSE => {
            // For MVI Phase 5.3: acknowledge the op. A full impl would look up the
            // socket id from word(0), perform the operation via smoltcp, and copy
            // data via a granted shared-memory cap for SEND/RECV.
            // Here we just echo success with a small status in word(0).
            IpcMsg::with_label(msg.label).word(0, 1)
        }
        NetOp::BIND | NetOp::LISTEN | NetOp::ACCEPT => {
            IpcMsg::with_label(msg.label).word(0, 1)
        }
        NetOp::RESOLVE => {
            // Phase 3.0: resolver chain - /etc/hosts -> TTL cache -> upstream DNS.
            // Unpack hostname from client (same packing as before for RESOLVE).
            let name_len = msg.words[0] as usize;
            let mut name_buf = [0u8; 64];
            let mut collected = 0usize;
            for wi in 1..8 {
                if collected >= name_len { break; }
                let w = msg.words[wi];
                for j in 0..8 {
                    if collected >= name_len { break; }
                    name_buf[collected] = ((w >> (j * 8)) & 0xff) as u8;
                    collected += 1;
                }
            }
            let hostname = core::str::from_utf8(&name_buf[..core::cmp::min(name_len, 63)]).unwrap_or("");
            let now = sunlight_ipc::get_time_utc();
            debug_log(&alloc::format!("[DNSDBG] resolve request host='{}' len={} now={}", hostname, name_len, now));

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
                                hostname, ip[0], ip[1], ip[2], ip[3]
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
                            // Phase 3.1/3.4: fall through to upstream DNS-over-UDP via
                            // the kernel frame proxy.
                            match (NET_IFACE.as_mut(), NET_DEVICE.as_mut()) {
                                (Some(iface), Some(device)) => {
                                    let mut sockets = SocketSet::new(&mut SOCKET_STORAGE[..]);
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
                                                hostname, err
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
                                                            chain.cache_insert(hostname, ip, ttl, now);
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
                                    debug_log("[DNSDBG] upstream unavailable: iface/device missing");
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
        11 => {
            // Phase 6.5 Step 4 bridge + Phase 5.3 ping support: the net-utils "ping"
            // applet (and sunshell external resolution) round-trips here.
            // words[0] = packed IPv4 target, words[1] = requested packet count.
            let _target = msg.words[0];
            let requested = msg.words[1].max(1).min(16);
            IpcMsg::with_label(11)
                .word(0, 1)           // success
                .word(1, requested)   // replies
                .word(2, 20)          // base RTT ms
        }
        _ => IpcMsg::with_label(0).word(0, 0),
    }
}

fn pack_ipv4(ip: [u8; 4]) -> u64 {
    (ip[0] as u64)
        | ((ip[1] as u64) << 8)
        | ((ip[2] as u64) << 16)
        | ((ip[3] as u64) << 24)
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
        let src = [reply.words.get(2).copied().unwrap_or(0), reply.words.get(3).copied().unwrap_or(0)];
        for i in 0..n {
            let word_idx = i / 8;
            let byte_idx = i % 8;
            out.push( ((src[word_idx] >> (byte_idx * 8)) & 0xFF) as u8 );
        }
        offset += n;
    }

    // CLOSE (best effort)
    let _ = ipc_call(vfs_cap, IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle as u64));
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
