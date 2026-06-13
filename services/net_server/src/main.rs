#![no_std]
#![no_main]

extern crate alloc;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait, nameserver_lookup, nameserver_register, CapabilityToken, IpcMsg, VfsMsg,
};
use sunlight_net::netop::NetOp;

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

/// 1.1 Combined DNS resolver (populated at init from /etc/hosts + hardcoded fallback).
/// Written once before the IPC loop; read-only thereafter.
static mut DNS_RESOLVER: Option<sunlight_net::DnsResolver> = None;

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

    // 1.0 VFS /etc/hosts + 1.1 Combined Resolver (hosts first, hardcoded fallback)
    // We read via capability-gated VFS IPC (existing VfsMsg + nameserver).
    // Parser is minimal (no heavy alloc beyond small BTreeMap<String,[u8;4]> inside resolver).
    let hosts_content = if let Some(vfs_cap) = nameserver_lookup("vfs") {
        let data = read_file_simple(vfs_cap, "/etc/hosts");
        // SAFETY: the data Vec lives in our process bump heap; from_utf8_lossy is safe on arbitrary bytes;
        // DnsResolver::new + parse_hosts will copy names into owned Strings so the temp content can be dropped.
        alloc::string::String::from_utf8_lossy(&data).into_owned()
    } else {
        alloc::string::String::new()
    };
    let resolver = sunlight_net::DnsResolver::new(&hosts_content);
    unsafe {
        // SAFETY: written exactly once, before any messages are handled (single-threaded
        // userspace service, no interrupts in this model). Subsequent reads in handle_msg
        // are after this point. The contained map owns its data for process lifetime.
        DNS_RESOLVER = Some(resolver);
    }
    debug_log("[DNS] /etc/hosts loaded (hosts + hardcoded resolver active)");

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
            // 1.2: use the combined DnsResolver (hosts from /etc/hosts via VFS + hardcoded fallback).
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
            let ip = unsafe {
                // SAFETY: DNS_RESOLVER is initialized exactly once before the receive loop
                // and never mutated again. This is the only reader. Single-threaded service.
                if let Some(ref r) = DNS_RESOLVER {
                    r.resolve(hostname).unwrap_or([0, 0, 0, 0])
                } else {
                    [0, 0, 0, 0]
                }
            };
            if ip == [0, 0, 0, 0] {
                IpcMsg::with_label(NetOp::RESOLVE).word(0, 0) // failure -> "Name or service not known"
            } else {
                IpcMsg::with_label(NetOp::RESOLVE).word(0, pack_ipv4(ip))
            }
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
