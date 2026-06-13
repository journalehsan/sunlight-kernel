//! PS/2 scancode set 1 keyboard driver.
//!
//! Handles IRQ1, translates scancodes to key events, and injects them into the
//! active TTY via IPC. Includes a deterministic key injection path for test
//! automation.

use crate::serial_println;
use x86_64::instructions::port::Port;

// ---------------------------------------------------------------------------
// Key event types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyEvent {
    pub keycode: u8,
    pub pressed: bool,
    pub modifiers: Modifiers,
    pub ascii: Option<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Keycode values (subset of common PS/2 set 1 codes).
#[allow(dead_code)]
pub mod keycode {
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
    pub const CAPS_LOCK: u8 = 0x3A;
    pub const F1: u8 = 0x3B;
    pub const F2: u8 = 0x3C;
    pub const F3: u8 = 0x3D;
    pub const F4: u8 = 0x3E;
    pub const F5: u8 = 0x3F;
    pub const F6: u8 = 0x40;
    pub const F7: u8 = 0x41;
    pub const F8: u8 = 0x42;
    pub const F9: u8 = 0x43;
    pub const F10: u8 = 0x44;
    pub const LEFT_CTRL: u8 = 0x1D;
    pub const LEFT_SHIFT: u8 = 0x2A;
    pub const LEFT_ALT: u8 = 0x38;
    pub const RIGHT_SHIFT: u8 = 0x36;
    pub const RIGHT_CTRL_EXT: u8 = 0x1D; // prefixed with 0xE0
    pub const RIGHT_ALT_EXT: u8 = 0x38; // prefixed with 0xE0
    pub const PAGE_UP_EXT: u8 = 0x49; // prefixed with 0xE0
    pub const PAGE_DOWN_EXT: u8 = 0x51; // prefixed with 0xE0
    pub const EXTENDED_PREFIX: u8 = 0xE0;
    pub const RELEASE_MASK: u8 = 0x80;
}

// ---------------------------------------------------------------------------
// ASCII lookup (US QWERTY, scancode set 1 -> ASCII)
// ---------------------------------------------------------------------------

fn scancode_to_ascii(scancode: u8, mods: &Modifiers) -> Option<u8> {
    // "0" is not a valid ASCII printable, used as sentinel
    let base: u8 = match scancode {
        // Letters
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
        // Numbers
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
        // Symbols (unshifted)
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
        keycode::ENTER => b'\n',
        keycode::TAB => b'\t',
        keycode::BACKSPACE => 0x08,
        _ => return None,
    };

    // Apply shift
    if mods.shift {
        match base {
            b'a'..=b'z' => return Some(base - 32),
            b'1' => return Some(b'!'),
            b'2' => return Some(b'@'),
            b'3' => return Some(b'#'),
            b'4' => return Some(b'$'),
            b'5' => return Some(b'%'),
            b'6' => return Some(b'^'),
            b'7' => return Some(b'&'),
            b'8' => return Some(b'*'),
            b'9' => return Some(b'('),
            b'0' => return Some(b')'),
            b'-' => return Some(b'_'),
            b'=' => return Some(b'+'),
            b'[' => return Some(b'{'),
            b']' => return Some(b'}'),
            b'\\' => return Some(b'|'),
            b';' => return Some(b':'),
            b'\'' => return Some(b'"'),
            b'`' => return Some(b'~'),
            b',' => return Some(b'<'),
            b'.' => return Some(b'>'),
            b'/' => return Some(b'?'),
            b'\n' | b'\t' | b' ' | 0x08 => return Some(base),
            _ => return Some(base),
        }
    }

    // Do NOT apply Ctrl here — let the tty mux handle it via the ctrl modifier flag.
    // Ctrl key combinations will have ctrl=true in the IPC message.

    Some(base)
}

// ---------------------------------------------------------------------------
// Modifier state
// ---------------------------------------------------------------------------

static MODIFIERS: spin::Mutex<Modifiers> = spin::Mutex::new(Modifiers {
    ctrl: false,
    alt: false,
    shift: false,
});

// ---------------------------------------------------------------------------
// Key injection buffer for deterministic testing
// ---------------------------------------------------------------------------

/// When true, the keyboard ISR reads from the injection buffer instead of 0x60.
/// Set this at compile time or before boot to enable test automation.
pub static mut KEY_INJECT_ENABLED: bool = false;

/// Fixed-size key injection buffer. Each u8 is a raw scancode.
/// Set KEY_INJECT_DATA[0..KEY_INJECT_LEN] before enabling KEY_INJECT_ENABLED.
pub static mut KEY_INJECT_DATA: [u8; 256] = [0u8; 256];
pub static mut KEY_INJECT_LEN: usize = 0;
pub static mut KEY_INJECT_IDX: usize = 0;

// ---------------------------------------------------------------------------
// Initialization
// ---------------------------------------------------------------------------

pub fn init() {
    serial_println!("[KBD]  PS/2 keyboard initialized");

    // Flush any pending data in the PS/2 buffer
    let mut data_port: Port<u8> = Port::new(0x60);
    let mut status_port: Port<u8> = Port::new(0x64);
    while unsafe { status_port.read() } & 1 != 0 {
        unsafe {
            data_port.read();
        }
    }

    // Unmask IRQ1 (bit 1) on PIC1
    let mut pic1_data: Port<u8> = Port::new(0x21);
    // SAFETY: we're in kernel mode during boot, port I/O is valid
    unsafe {
        let mask = pic1_data.read();
        pic1_data.write(mask & !0x02); // clear bit 1
    }

    serial_println!("[KBD]  IRQ1 handler installed");
}

/// Read a byte from the keyboard (port 0x60 or injection buffer).
fn read_keyboard_byte() -> u8 {
    // SAFETY: we're in IRQ handler context (ring 0), port I/O is valid.
    // Also reads static injection data, which is safe in single-threaded kernel.
    if unsafe { KEY_INJECT_ENABLED } {
        let idx = unsafe { KEY_INJECT_IDX };
        let len = unsafe { KEY_INJECT_LEN };
        if idx < len {
            let val = unsafe { KEY_INJECT_DATA[idx] };
            unsafe {
                KEY_INJECT_IDX = idx + 1;
            }
            return val;
        }
        // Inject buffer exhausted, fall through to real hardware
        unsafe {
            KEY_INJECT_ENABLED = false;
        }
    }

    let mut port: Port<u8> = Port::new(0x60);
    unsafe { port.read() }
}

/// Timer-driven poll for key injection. Called periodically to simulate
/// keyboard events without requiring actual IRQ1 fires (needed for QEMU
/// test automation when no physical keyboard is connected).
///
/// Waits for a configurable number of ticks after boot before starting
/// injection, giving services time to register.
pub fn poll_inject_buffer() {
    // SAFETY: reading static injection state in IRQ context is safe.
    if !unsafe { KEY_INJECT_ENABLED } {
        return;
    }

    // Wait for enough ticks to pass so all services are registered.
    // init → vfs_server → timer_server → tty_server need time to start.
    static TICK_COUNT: spin::Mutex<u64> = spin::Mutex::new(0);
    const START_DELAY_TICKS: u64 = 120;

    let mut ticks = TICK_COUNT.lock();
    *ticks += 1;
    if *ticks < START_DELAY_TICKS {
        return;
    }
    if *ticks == START_DELAY_TICKS {
        crate::serial_println!("[KBD] injection starting (tick {})", *ticks);
    }
    drop(ticks);

    // The first 5 scancodes are the login password (r,o,o,t,Enter).
    // After that, pause injection until sshl appears in the scheduler so that
    // all shell-command keys arrive AFTER sshl has registered. This keeps the
    // tty_server pre-sshl buffer empty, avoiding dozens of slow IPC round-trips.
    const LOGIN_SCANCODES: usize = 5;
    let inject_idx = unsafe { KEY_INJECT_IDX };
    if inject_idx >= LOGIN_SCANCODES {
        let sshl_up = crate::sched::SCHEDULER
            .lock()
            .processes
            .iter()
            .any(|p| p.name == "sshl");
        if !sshl_up {
            return;
        }
    }

    let mut processed = 0;
    while processed < 4 {
        let idx = unsafe { KEY_INJECT_IDX };
        let len = unsafe { KEY_INJECT_LEN };
        if idx >= len {
            unsafe {
                KEY_INJECT_ENABLED = false;
            }
            break;
        }
        let scancode = unsafe { KEY_INJECT_DATA[idx] };
        unsafe {
            KEY_INJECT_IDX = idx + 1;
        }

        if let Some(event_val) = process_scancode(scancode) {
            send_event_to_tty(event_val);
        }
        processed += 1;
    }
}

/// Send a keyboard event to the tty_server via IPC.
fn send_event_to_tty(event_val: u64) {
    let mut sched = crate::sched::SCHEDULER.lock();
    let (endpoint_id, server_pid) = sched
        .processes
        .iter()
        .find(|p| p.name == "tty_server")
        .and_then(|p| {
            p.ipc_endpoint.map(|ep| {
                let pid = p.pid;
                (ep, pid)
            })
        })
        .unwrap_or((0, 0));

    if endpoint_id != 0 {
        let mut bus = crate::ipc::IPC_BUS.lock();
        bus.send_keyboard_event(endpoint_id, event_val, &mut sched, server_pid);
    }
}

// ---------------------------------------------------------------------------
// IRQ1 handler — called from the keyboard interrupt entry
// ---------------------------------------------------------------------------

/// Process one raw scancode byte. Returns a packed event value suitable for
/// IPC transport, or None if the scancode should be discarded (modifier-only).
fn process_scancode(scancode: u8) -> Option<u64> {
    let mut mods = MODIFIERS.lock();
    let (pressed, code) = if scancode & keycode::RELEASE_MASK != 0 {
        (false, scancode & !keycode::RELEASE_MASK)
    } else {
        (true, scancode)
    };

    match code {
        keycode::LEFT_SHIFT | keycode::RIGHT_SHIFT => mods.shift = pressed,
        keycode::LEFT_CTRL => mods.ctrl = pressed,
        keycode::LEFT_ALT => mods.alt = pressed,
        _ => {}
    };

    let ascii = if pressed {
        scancode_to_ascii(code, &mods)
    } else {
        None
    };

    // Pack event: keycode(u8) | pressed(u8) << 8 | mods_byte(u8) << 16 | ascii(u8) << 24
    let mut val = code as u64;
    val |= (pressed as u64) << 8;
    let mods_byte =
        ((mods.shift as u64) << 0) | ((mods.ctrl as u64) << 1) | ((mods.alt as u64) << 2);
    val |= mods_byte << 16;
    val |= (ascii.unwrap_or(0) as u64) << 24;
    Some(val)
}

/// Main IRQ1 handler: reads scancode, processes it, and sends via IPC.
pub fn handle_irq1() {
    let scancode = read_keyboard_byte();
    if let Some(event_val) = process_scancode(scancode) {
        send_event_to_tty(event_val);
    }

    // Send EOI to PIC
    unsafe {
        let mut cmd1: Port<u8> = Port::new(0x20);
        cmd1.write(0x20);
    }
}
