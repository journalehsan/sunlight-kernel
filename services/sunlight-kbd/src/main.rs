//! sunlight-kbd — User-space PS/2 keyboard driver for SunlightOS.
//!
//! Receives raw scancodes from the kernel's IRQ1 router via syscall polling,
//! processes them into KeyEvents (tracking modifiers, translating to ASCII),
//! and forwards the formatted events to tty_server via IPC.

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, getpid, ipc_call, ipc_call_timeout, ipc_recv,
    nameserver_lookup_timeout, DevicedMsg, DriverCaps, DriverKind, DriverState, IpcMsg,
    ProcessExit,
};

use sunlight_kbd::Keyboard;

/// Syscall wrappers for keyboard-specific operations.
///
/// Pass the FULL endpoint capability token (u64), not a truncated u32: the
/// kernel resolves it to the real internal endpoint id so keyboard events are
/// queued on the same endpoint this driver receives on.
fn kbd_register(endpoint_token: u64) -> bool {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 110u64 => ret,
            in("rdi") endpoint_token,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    ret == 0
}

fn kbd_pop_scancode() -> Option<u8> {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 112u64 => ret,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    if ret == u64::MAX {
        None
    } else {
        Some(ret as u8)
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

fn register_with_deviced(state: DriverState) {
    let Some(cap) = nameserver_lookup_timeout("deviced", 5) else {
        debug_log("[KBD] deviced unavailable; continuing without registration");
        return;
    };
    let meta = (DriverKind::Keyboard as u64) | ((state as u64) << 16);
    let caps = DriverCaps::INPUT | DriverCaps::KEYBOARD;
    let msg = IpcMsg::with_label(DevicedMsg::REGISTER_DRIVER)
        .word(0, pack_short_name("keyboard"))
        .word(1, getpid())
        .word(2, meta)
        .word(3, caps);
    match ipc_call_timeout(cap, msg, 20) {
        Ok(reply) if reply.label == DevicedMsg::REPLY => {
            debug_log("[KBD] registered with deviced");
        }
        _ => debug_log("[KBD] deviced registration failed; continuing"),
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    debug_log("[KBD] PANIC\n");
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[KBD] sunlight-kbd starting");

    let my_endpoint = endpoint_create();

    // Resolve tty BEFORE registering with the kernel IRQ1 router so that no
    // keyboard events (and their wake_pid) are delivered to this process while
    // it is still blocked resolving "tty" — keeping the lookup IPC clean.
    let tty_token = loop {
        if let Some(t) = nameserver_lookup_timeout("tty", 50) {
            break t;
        }
        sunlight_ipc::process_yield();
    };
    debug_log("[KBD] found tty, ready to process keys");

    if !kbd_register(my_endpoint.0) {
        register_with_deviced(DriverState::Failed);
        debug_log("[KBD] FATAL: kernel IRQ1 router registration failed\n");
        ProcessExit::exit(1);
    }
    debug_log("[KBD] registered with kernel IRQ1 router");
    register_with_deviced(DriverState::Ready);

    let mut keyboard = Keyboard::default();
    let mut extended_prefix = false;
    loop {
        while let Some(scancode) = kbd_pop_scancode() {
            if scancode == 0xe0 {
                extended_prefix = true;
                continue;
            }
            let (raw, commit) = keyboard.process(scancode, extended_prefix);
            let _ = ipc_call(tty_token, IpcMsg::with_label(0x1).word(0, raw));
            if let Some(commit) = commit {
                let _ = ipc_call(tty_token, IpcMsg::with_label(0x1).word(0, commit));
            }
            extended_prefix = false;
        }
        let _ = ipc_recv(my_endpoint);
    }
}
