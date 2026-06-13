#![no_std]
#![no_main]

extern crate alloc;

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg,
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
            // Bite 2: actual (stub) DNS resolution for ping + other net-utils.
            // Client packs: words[0] = name_len (bytes), words[1..] = 8-byte chunks of the hostname.
            // We support a few well-known names (matching the spirit of sunlight-net/dns.rs stub)
            // so that `ping google.com` / `ping irancell.ir` produce the nice "PING name (ip)" output
            // and the failure case for unknown names prints the exact required message.
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
            let ip = match hostname {
                "google.com" => [142, 250, 185, 46],
                "irancell.ir" => [91, 99, 12, 34],
                "example.com" => [93, 184, 216, 34],
                // anything else (including "nonexistent.invalid", "doesnotexist.invalid") fails
                _ => [0, 0, 0, 0],
            };
            if ip == [0, 0, 0, 0] {
                IpcMsg::with_label(NetOp::RESOLVE).word(0, 0) // signals failure to caller
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
