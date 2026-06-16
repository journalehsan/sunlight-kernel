//! runas — standalone Ring 3 privilege-delegation tool.
//!
//! `runas` is deliberately **not** part of the kernel or the syscall layer.
//! It is an ordinary user-space binary (`/bin/runas`) that brokers privilege
//! escalation purely through IPC:
//!
//!   1. It looks up the `"uac"` capability broker via the nameserver.
//!   2. It proves it holds the prerequisite *administrative grant* capability
//!      (an IPC token carrying the `can_grant` right). Without it the broker
//!      rejects the request — `runas` cannot escalate on its own authority.
//!   3. It asks the broker (`OP_DELEGATE`) to mint a fresh `CapabilityToken`
//!      scoped to the target execution context (target uid + command), which
//!      the broker returns in the reply's capability slot.
//!
//! Usage (argv is delivered packed in the launch message by the shell):
//!
//!   runas <target-uid> <command> [args...]
//!
//! This file is intentionally a skeleton: the wire protocol and delegation
//! flow are complete, while argv parsing and the post-mint `exec` are marked
//! as integration points for the spawn path.

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 16 * 1024] = [0; 16 * 1024];
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

use sunlight_ipc::{ipc_call, nameserver_lookup, CapabilityToken, IpcMsg};

fn stdout_write(s: &str) {
    let mut data = s.as_bytes();
    while !data.is_empty() {
        match sunlight_libc::write(sunlight_libc::STDOUT, data) {
            Ok(n) if n > 0 => data = &data[n..],
            _ => break,
        }
    }
}

macro_rules! println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        stdout_write(&buf);
        stdout_write("\n");
    }};
}

/// Broker opcode: delegate / mint a capability for a target context.
/// (Mirrors `uac_service`'s `OP_DELEGATE`.)
const OP_DELEGATE: u64 = 3;
/// Reply label for a granted request.
const REPLY_OK: u64 = 1;

/// Pack a NUL-terminated little-endian string into `msg.words[start..]`.
fn pack_str(msg: &mut IpcMsg, start: usize, s: &str) {
    let bytes = s.as_bytes();
    let mut i = 0;
    for word_idx in start..msg.words.len() {
        let mut word: u64 = 0;
        for j in 0..8 {
            if i < bytes.len() {
                word |= (bytes[i] as u64) << (j * 8);
                i += 1;
            }
        }
        msg.words[word_idx] = word;
    }
}

/// Locate the administrative grant capability handed to `runas` at launch.
///
/// In SunlightOS a delegating tool receives its prerequisite *grant* token in
/// its launch environment. Until argv/env-cap plumbing lands, we resolve it
/// from the nameserver under the well-known name `"uac.admin"`; the broker
/// only publishes that name to authorized callers, so an unprivileged `runas`
/// simply gets `None` here and bails out before ever contacting the broker.
fn admin_grant() -> Option<CapabilityToken> {
    nameserver_lookup("uac.admin")
}

/// Ask the broker to mint a capability for the target execution context.
/// Returns the freshly minted token on success.
fn delegate(
    broker: CapabilityToken,
    admin: CapabilityToken,
    target_uid: u32,
    command: &str,
) -> Result<CapabilityToken, ()> {
    let mut m = IpcMsg::empty();
    m.label = OP_DELEGATE;
    m.words[0] = target_uid as u64;
    pack_str(&mut m, 1, command);
    // Present our prerequisite grant capability for the broker to verify.
    m.caps[0] = admin;

    let r = ipc_call(broker, m);
    if r.label == REPLY_OK {
        Ok(r.caps[0])
    } else {
        Err(())
    }
}

#[no_mangle]
fn _start() -> ! {
    // TODO(spawn): parse argv for <target-uid> <command> [args...].
    // Placeholder target context until argv plumbing is wired in.
    let target_uid: u32 = 0;
    let command: &str = "/bin/sh";

    let Some(broker) = nameserver_lookup("uac") else {
        println!("runas: uac broker not found (is it running?)");
        sunlight_libc::exit(1);
    };

    // Prerequisite check: we must already hold an administrative grant.
    let Some(admin) = admin_grant() else {
        println!("runas: permission denied (no administrative grant capability)");
        sunlight_libc::exit(13); // EACCES
    };

    match delegate(broker, admin, target_uid, command) {
        Ok(cap) => {
            println!(
                "runas: granted cap {:#x} for uid={} cmd={}",
                cap.0,
                target_uid,
                command
            );
            // TODO(spawn): exec `command` in the target context carrying `cap`.
            sunlight_libc::exit(0);
        }
        Err(()) => {
            println!("runas: broker rejected delegation for uid={}", target_uid);
            sunlight_libc::exit(1);
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("runas: PANIC: {}", info);
    sunlight_libc::exit(1);
}
