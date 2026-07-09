//! sunlight-kbd — User-space PS/2 keyboard driver for SunlightOS.
//!
//! Receives raw scancodes from the kernel's IRQ1 router via syscall polling,
//! processes them into KeyEvents (tracking modifiers, translating to ASCII),
//! and forwards the formatted events to tty_server via IPC.

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, getpid, ipc_call_timeout, ipc_recv, nameserver_lookup,
    nameserver_lookup_timeout, nameserver_register, DevicedMsg, DriverCaps, DriverKind,
    DriverState, IpcMsg,
};

const INPUT_FORWARD_TIMEOUT_MS: u64 = 50;
const TIMEOUT_LOG_INTERVAL: u64 = 32;

/// PS/2 Scancode Set 1 keycodes
mod keycode {
    pub const A: u8 = 0x1E;
    pub const B: u8 = 0x30;
    pub const C: u8 = 0x2E;
    pub const D: u8 = 0x20;
    pub const E: u8 = 0x12;
    pub const F: u8 = 0x21;
    pub const G: u8 = 0x22;
    pub const H: u8 = 0x23;
    pub const I: u8 = 0x17;
    pub const J: u8 = 0x24;
    pub const K: u8 = 0x25;
    pub const L: u8 = 0x26;
    pub const M: u8 = 0x32;
    pub const N: u8 = 0x31;
    pub const O: u8 = 0x18;
    pub const P: u8 = 0x19;
    pub const Q: u8 = 0x10;
    pub const R: u8 = 0x13;
    pub const S: u8 = 0x1F;
    pub const T: u8 = 0x14;
    pub const U: u8 = 0x16;
    pub const V: u8 = 0x2F;
    pub const W: u8 = 0x11;
    pub const X: u8 = 0x2D;
    pub const Y: u8 = 0x15;
    pub const Z: u8 = 0x2C;
    pub const NUM0: u8 = 0x0B;
    pub const NUM1: u8 = 0x02;
    pub const NUM2: u8 = 0x03;
    pub const NUM3: u8 = 0x04;
    pub const NUM4: u8 = 0x05;
    pub const NUM5: u8 = 0x06;
    pub const NUM6: u8 = 0x07;
    pub const NUM7: u8 = 0x08;
    pub const NUM8: u8 = 0x09;
    pub const NUM9: u8 = 0x0A;
    pub const ENTER: u8 = 0x1C;
    pub const ESC: u8 = 0x01;
    pub const BACKSPACE: u8 = 0x0E;
    pub const TAB: u8 = 0x0F;
    pub const SPACE: u8 = 0x39;
    pub const MINUS: u8 = 0x0C;
    pub const EQUALS: u8 = 0x0D;
    pub const LEFT_BRACKET: u8 = 0x1A;
    pub const RIGHT_BRACKET: u8 = 0x1B;
    pub const BACKSLASH: u8 = 0x2B;
    pub const SEMICOLON: u8 = 0x27;
    pub const QUOTE: u8 = 0x28;
    pub const TILDE: u8 = 0x29;
    pub const COMMA: u8 = 0x33;
    pub const PERIOD: u8 = 0x34;
    pub const SLASH: u8 = 0x35;
    pub const LEFT_CTRL: u8 = 0x1D;
    pub const LEFT_SHIFT: u8 = 0x2A;
    pub const LEFT_ALT: u8 = 0x38;
    pub const RIGHT_SHIFT: u8 = 0x36;
    pub const LEFT_SUPER: u8 = 0x5B;
    pub const RIGHT_SUPER: u8 = 0x5C;
    pub const RELEASE_MASK: u8 = 0x80;
    pub const EXTENDED_PREFIX: u8 = 0xE0;
}

/// Modifier state tracker
#[derive(Default, Clone, Copy)]
struct Modifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
}

/// Translate PS/2 scancode to ASCII (US QWERTY layout)
fn scancode_to_ascii(scancode: u8, mods: &Modifiers) -> Option<u8> {
    let base: u8 = match scancode {
        keycode::A => b'a',
        keycode::B => b'b',
        keycode::C => b'c',
        keycode::D => b'd',
        keycode::E => b'e',
        keycode::F => b'f',
        keycode::G => b'g',
        keycode::H => b'h',
        keycode::I => b'i',
        keycode::J => b'j',
        keycode::K => b'k',
        keycode::L => b'l',
        keycode::M => b'm',
        keycode::N => b'n',
        keycode::O => b'o',
        keycode::P => b'p',
        keycode::Q => b'q',
        keycode::R => b'r',
        keycode::S => b's',
        keycode::T => b't',
        keycode::U => b'u',
        keycode::V => b'v',
        keycode::W => b'w',
        keycode::X => b'x',
        keycode::Y => b'y',
        keycode::Z => b'z',
        keycode::NUM1 => b'1',
        keycode::NUM2 => b'2',
        keycode::NUM3 => b'3',
        keycode::NUM4 => b'4',
        keycode::NUM5 => b'5',
        keycode::NUM6 => b'6',
        keycode::NUM7 => b'7',
        keycode::NUM8 => b'8',
        keycode::NUM9 => b'9',
        keycode::NUM0 => b'0',
        keycode::MINUS => b'-',
        keycode::EQUALS => b'=',
        keycode::LEFT_BRACKET => b'[',
        keycode::RIGHT_BRACKET => b']',
        keycode::BACKSLASH => b'\\',
        keycode::SEMICOLON => b';',
        keycode::QUOTE => b'\'',
        keycode::TILDE => b'`',
        keycode::COMMA => b',',
        keycode::PERIOD => b'.',
        keycode::SLASH => b'/',
        keycode::SPACE => b' ',
        keycode::TAB => b'\t',
        keycode::ENTER => b'\n',
        keycode::ESC => 0x1b,
        keycode::BACKSPACE => 0x08,
        _ => return None,
    };

    if base >= b'a' && base <= b'z' {
        if mods.shift {
            Some(base - 32)
        } else {
            Some(base)
        }
    } else if base >= b'1' && base <= b'9' && mods.shift {
        let shifted = match base {
            b'1' => b'!',
            b'2' => b'@',
            b'3' => b'#',
            b'4' => b'$',
            b'5' => b'%',
            b'6' => b'^',
            b'7' => b'&',
            b'8' => b'*',
            b'9' => b'(',
            _ => base,
        };
        Some(shifted)
    } else if base == b'0' && mods.shift {
        Some(b')')
    } else if mods.shift {
        let shifted = match base {
            b'-' => b'_',
            b'=' => b'+',
            b'[' => b'{',
            b']' => b'}',
            b'\\' => b'|',
            b';' => b':',
            b'\'' => b'"',
            b'`' => b'~',
            b',' => b'<',
            b'.' => b'>',
            b'/' => b'?',
            _ => base,
        };
        Some(shifted)
    } else {
        Some(base)
    }
}

/// Process one raw scancode into a packed event value.
fn process_scancode(scancode: u8, extended: bool, mods: &mut Modifiers) -> Option<u64> {
    let (pressed, code) = if scancode & keycode::RELEASE_MASK != 0 {
        (false, scancode & !keycode::RELEASE_MASK)
    } else {
        (true, scancode)
    };

    if extended {
        match code {
            keycode::LEFT_CTRL => mods.ctrl = pressed,
            keycode::LEFT_ALT => mods.alt = pressed,
            keycode::LEFT_SUPER | keycode::RIGHT_SUPER => mods.super_key = pressed,
            _ => {}
        }
    } else {
        match code {
            keycode::LEFT_SHIFT | keycode::RIGHT_SHIFT => mods.shift = pressed,
            keycode::LEFT_CTRL => mods.ctrl = pressed,
            keycode::LEFT_ALT => mods.alt = pressed,
            _ => {}
        }
    }

    let ascii = if pressed && !extended {
        scancode_to_ascii(code, mods)
    } else {
        None
    };

    // Pack: keycode | pressed<<8 | mods<<16 | ascii<<24
    let mut val = code as u64;
    val |= (pressed as u64) << 8;
    let mods_byte = ((mods.shift as u64) << 0)
        | ((mods.ctrl as u64) << 1)
        | ((mods.alt as u64) << 2)
        | ((mods.super_key as u64) << 3);
    val |= mods_byte << 16;
    val |= (ascii.unwrap_or(0) as u64) << 24;
    Some(val)
}

/// Syscall wrappers for keyboard-specific operations.
///
/// Pass the FULL endpoint capability token (u64), not a truncated u32: the
/// kernel resolves it to the real internal endpoint id so keyboard events are
/// queued on the same endpoint this driver receives on.
fn kbd_register(endpoint_token: u64) {
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
    let _ = ret;
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

fn register_with_deviced() {
    let Some(cap) = nameserver_lookup_timeout("deviced", 5) else {
        debug_log("[KBD] deviced unavailable; continuing without registration");
        return;
    };
    let meta = (DriverKind::Keyboard as u64) | ((DriverState::Ready as u64) << 16);
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
    nameserver_register("kbd_driver", my_endpoint);
    register_with_deviced();

    // Resolve tty BEFORE registering with the kernel IRQ1 router so that no
    // keyboard events (and their wake_pid) are delivered to this process while
    // it is still blocked resolving "tty" — keeping the lookup IPC clean.
    let tty_token = loop {
        if let Some(t) = nameserver_lookup("tty") {
            break t;
        }
        sunlight_ipc::process_yield();
    };
    debug_log("[KBD] found tty, ready to process keys");

    kbd_register(my_endpoint.0);
    debug_log("[KBD] registered with kernel IRQ1 router");

    let mut modifiers = Modifiers::default();
    let mut extended_prefix = false;
    let mut tty_timeout_count = 0u64;
    loop {
        while let Some(scancode) = kbd_pop_scancode() {
            if scancode == keycode::EXTENDED_PREFIX {
                extended_prefix = true;
                continue;
            }
            if let Some(event_val) = process_scancode(scancode, extended_prefix, &mut modifiers) {
                let msg = IpcMsg::with_label(0x1).word(0, event_val);
                if ipc_call_timeout(tty_token, msg, INPUT_FORWARD_TIMEOUT_MS).is_err() {
                    tty_timeout_count = tty_timeout_count.saturating_add(1);
                    if tty_timeout_count == 1 || tty_timeout_count % TIMEOUT_LOG_INTERVAL == 0 {
                        debug_log("[KBD] tty forward timeout\n");
                    }
                }
            }
            extended_prefix = false;
        }
        let _ = ipc_recv(my_endpoint);
    }
}
