//! sunlight-mouse — User-space PS/2 mouse driver for SunlightOS.
//!
//! Initializes the 8042 controller's auxiliary port, receives raw IRQ12 bytes
//! from the kernel via IPC, parses 3-byte PS/2 packets, and forwards raw relative
//! motion (dx/dy/buttons) to display_server when available, falling back to absolute
//! coordinates sent to tty_server for legacy terminal UI.

#![no_std]
#![no_main]

use sunlight_ipc::{
    endpoint_create, getpid, ipc_call, ipc_call_timeout, ipc_recv, nameserver_lookup,
    nameserver_lookup_timeout, nameserver_register, process_yield, DevicedMsg, DriverCaps,
    DriverKind, DriverState, IpcMsg, MouseMsg, ProcessExit,
};

/// Set to true to print raw packet bytes and decoded dx/dy on every complete packet.
const MOUSE_DEBUG: bool = false;
const NICED_SET_NICE: u64 = 0x9010;
const NICED_REPLY: u64 = 0x90FF;
const MOUSE_INPUT_NICE: i8 = -10;

/// PS/2 mouse packet state machine
#[derive(Clone, Copy, PartialEq)]
enum PacketState {
    WaitingByte0,
    WaitingByte1,
    WaitingByte2,
}

/// Mouse state tracker
struct MouseState {
    packet_state: PacketState,
    byte0: u8,
    byte1: u8,
    byte2: u8,
    abs_x: i32,
    abs_y: i32,
    screen_width: i32,
    screen_height: i32,
}

impl MouseState {
    fn new(width: i32, height: i32) -> Self {
        Self {
            packet_state: PacketState::WaitingByte0,
            byte0: 0,
            byte1: 0,
            byte2: 0,
            abs_x: width / 2,
            abs_y: height / 2,
            screen_width: width,
            screen_height: height,
        }
    }

    /// Process one raw byte from IRQ12. Returns Some(event) when a complete packet is ready.
    fn process_byte(&mut self, byte: u8) -> Option<MouseEvent> {
        match self.packet_state {
            PacketState::WaitingByte0 => {
                if byte & 0x08 == 0 {
                    return None;
                }
                self.byte0 = byte;
                self.packet_state = PacketState::WaitingByte1;
                None
            }
            PacketState::WaitingByte1 => {
                self.byte1 = byte;
                self.packet_state = PacketState::WaitingByte2;
                None
            }
            PacketState::WaitingByte2 => {
                self.byte2 = byte;
                self.packet_state = PacketState::WaitingByte0;

                let flags = self.byte0;

                // Discard overflow packets — data is unreliable when either overflow bit is set.
                if (flags & 0x40) != 0 || (flags & 0x80) != 0 {
                    if MOUSE_DEBUG {
                        syscall::debug_log("[MOUSE] overflow packet discarded\n");
                    }
                    return None;
                }

                let left_btn = (flags & 0x01) != 0;
                let right_btn = (flags & 0x02) != 0;
                let middle_btn = (flags & 0x04) != 0;

                // Sign-extend dx/dy from 9-bit two's complement (sign bit in byte0).
                let mut dx = self.byte1 as i32;
                let mut dy = self.byte2 as i32;

                if (flags & 0x10) != 0 {
                    dx |= !0xFF;
                }
                if (flags & 0x20) != 0 {
                    dy |= !0xFF;
                }

                // Invert Y: PS/2 defines up as positive; screen Y grows downward.
                dy = -dy;

                let raw_dx = dx as i16;
                let raw_dy = dy as i16;

                if MOUSE_DEBUG {
                    syscall::debug_log("[MOUSE] pkt b0=");
                    syscall::debug_log_byte(flags);
                    syscall::debug_log(" dx=");
                    syscall::debug_log_i16(raw_dx);
                    syscall::debug_log(" dy=");
                    syscall::debug_log_i16(raw_dy);
                    syscall::debug_log("\n");
                }

                self.abs_x = (self.abs_x + dx).max(0).min(self.screen_width - 1);
                self.abs_y = (self.abs_y + dy).max(0).min(self.screen_height - 1);

                Some(MouseEvent {
                    dx: raw_dx,
                    dy: raw_dy,
                    abs_x: self.abs_x as u16,
                    abs_y: self.abs_y as u16,
                    left_button: left_btn,
                    right_button: right_btn,
                    middle_button: middle_btn,
                })
            }
        }
    }
}

#[derive(Clone, Copy)]
struct MouseEvent {
    dx: i16,
    dy: i16,
    abs_x: u16,
    abs_y: u16,
    left_button: bool,
    right_button: bool,
    middle_button: bool,
}

/// Syscall wrappers for mouse-specific operations
mod syscall {
    const SYS_MOUSE_INIT: u64 = 116;
    const SYS_MOUSE_REGISTER: u64 = 114;
    const SYS_MOUSE_POP_BYTE: u64 = 115;
    const SYS_DEBUG_LOG: u64 = 99;

    pub fn mouse_init() -> bool {
        let ret: u64;
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") SYS_MOUSE_INIT => ret,
                lateout("rcx") _, lateout("r11") _,
                options(nostack)
            );
        }
        ret == 0
    }

    pub fn mouse_register(endpoint_token: u64) {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") SYS_MOUSE_REGISTER => _,
                in("rdi") endpoint_token,
                lateout("rcx") _, lateout("r11") _,
                options(nostack)
            );
        }
    }

    pub fn mouse_pop_byte() -> Option<u8> {
        let ret: u64;
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") SYS_MOUSE_POP_BYTE => ret,
                lateout("rcx") _, lateout("r11") _,
                options(nostack)
            );
        }
        if ret == u64::MAX {
            None
        } else {
            Some(ret as u8)
        }
    }

    pub fn debug_log(msg: &str) {
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") SYS_DEBUG_LOG => _,
                in("rdi") msg.as_ptr() as u64,
                in("rsi") msg.len() as u64,
                lateout("rcx") _, lateout("r11") _,
                options(nostack)
            );
        }
    }

    pub fn debug_log_byte(v: u8) {
        let hex = b"0123456789ABCDEF";
        let buf = [b'0', b'x', hex[(v >> 4) as usize], hex[(v & 0xF) as usize]];
        let s = unsafe { core::str::from_utf8_unchecked(&buf) };
        debug_log(s);
    }

    pub fn debug_log_i16(v: i16) {
        let mut buf = [0u8; 7];
        let (neg, mut n) = if v < 0 {
            (true, (-(v as i32)) as u32)
        } else {
            (false, v as u32)
        };
        let mut i = 7usize;
        loop {
            i -= 1;
            buf[i] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        if neg {
            i -= 1;
            buf[i] = b'-';
        }
        let s = unsafe { core::str::from_utf8_unchecked(&buf[i..]) };
        debug_log(s);
    }
}

/// Phase 1: Initialize the PS/2 mouse (8042 controller auxiliary port)
fn init_ps2_mouse() -> Result<(), ()> {
    if syscall::mouse_init() {
        Ok(())
    } else {
        syscall::debug_log("[MOUSE] ERROR: Kernel PS/2 mouse init failed\n");
        Err(())
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
    let Some(cap) = nameserver_lookup_timeout("deviced", 100) else {
        syscall::debug_log("[MOUSE] deviced unavailable; continuing without registration\n");
        return;
    };
    let meta = (DriverKind::Mouse as u64) | ((DriverState::Ready as u64) << 16);
    let caps = DriverCaps::INPUT | DriverCaps::POINTER | DriverCaps::RELATIVE_MOTION;
    let msg = IpcMsg::with_label(DevicedMsg::REGISTER_DRIVER)
        .word(0, pack_short_name("mouse"))
        .word(1, getpid())
        .word(2, meta)
        .word(3, caps);
    match ipc_call_timeout(cap, msg, 100) {
        Ok(reply) if reply.label == DevicedMsg::REPLY => {
            syscall::debug_log("[MOUSE] registered with deviced\n");
        }
        _ => syscall::debug_log("[MOUSE] deviced registration failed; continuing\n"),
    }
}

fn request_priority_boost(boosted: &mut bool) {
    if *boosted {
        return;
    }

    let Some(niced) = nameserver_lookup("niced") else {
        return;
    };

    let msg = IpcMsg::with_label(NICED_SET_NICE)
        .word(0, getpid())
        .word(1, MOUSE_INPUT_NICE as i64 as u64);
    match ipc_call_timeout(niced, msg, 50) {
        Ok(reply) if reply.label == NICED_REPLY => {
            *boosted = true;
            syscall::debug_log("[MOUSE] boosted to critical scheduler priority via niced\n");
        }
        _ => {}
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::debug_log("[MOUSE] PANIC\n");
    ProcessExit::exit(101);
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    syscall::debug_log("[MOUSE] sunlight-mouse starting\n");

    // Phase 1: Initialize PS/2 mouse hardware
    if init_ps2_mouse().is_err() {
        syscall::debug_log("[MOUSE] FATAL: Hardware initialization failed; exiting driver\n");
        ProcessExit::exit(1);
    }

    // Create IPC endpoint and register with nameserver
    let my_endpoint = endpoint_create();
    nameserver_register("mouse_driver", my_endpoint);
    register_with_deviced();
    syscall::mouse_register(my_endpoint.0);
    syscall::debug_log("[MOUSE] registered with kernel IRQ12 router\n");

    // Lookup tty_server capability (for legacy TUI)
    let tty_token = loop {
        if let Some(t) = nameserver_lookup("tty") {
            break t;
        }
        process_yield();
    };

    // Also lookup display_server for the new graphical stack (Phase 3+).
    // Display may start later (e.g. on Desktop login), so we do a lazy lookup
    // on first use and cache the result.
    let mut display_token: Option<sunlight_ipc::CapabilityToken> = None;
    let mut priority_boosted = false;

    syscall::debug_log("[MOUSE] found tty, ready to process mouse events\n");

    let mut mouse_state = MouseState::new(1024, 768);

    loop {
        request_priority_boost(&mut priority_boosted);

        while let Some(byte) = syscall::mouse_pop_byte() {
            if let Some(event) = mouse_state.process_byte(byte) {
                let buttons = (event.left_button as u64)
                    | ((event.right_button as u64) << 1)
                    | ((event.middle_button as u64) << 2);

                // Lazy lookup display so we work even if it starts after us
                if display_token.is_none() {
                    display_token = nameserver_lookup("display_server");
                    if display_token.is_some() {
                        syscall::debug_log(
                            "[MOUSE] display_server available, routing raw events there\n",
                        );
                    }
                }

                if let Some(disp) = display_token {
                    // --- Graphical mode: send raw relative motion (dx/dy/buttons) ---
                    let raw = (event.dx as u64 & 0xFFFF)
                        | ((event.dy as u64 & 0xFFFF) << 16)
                        | (buttons << 32);
                    let msg = IpcMsg::with_label(MouseMsg::RAW_MOTION).word(0, raw);
                    let _ = ipc_call(disp, msg);
                } else {
                    // --- Legacy TTY mode: send packed absolute coordinates ---
                    let mut abs = event.abs_x as u64;
                    abs |= (event.abs_y as u64) << 16;
                    abs |= buttons << 32;
                    let msg = IpcMsg::with_label(0x2).word(0, abs);
                    let _ = ipc_call(tty_token, msg);
                }
            }
        }

        let _ = ipc_recv(my_endpoint);
    }
}
