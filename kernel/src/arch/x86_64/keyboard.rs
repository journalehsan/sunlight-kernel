//! PS/2 keyboard IRQ1 raw byte router.
//!
//! Minimal kernel-side IRQ1 handler that reads the raw scancode byte from
//! port 0x60 and forwards it to the registered user-space keyboard driver
//! (sunlight-kbd) via a lock-free ring buffer. The kernel does NOT perform
//! any scancode translation, modifier tracking, or ASCII conversion.

use crate::serial_println;
use core::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering};
use x86_64::instructions::port::Port;

const DATA_PORT: u16 = 0x60;
const STATUS_COMMAND_PORT: u16 = 0x64;
const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_INPUT_FULL: u8 = 1 << 1;
const STATUS_AUX_DATA: u8 = 1 << 5;
const CONTROLLER_CONFIG_IRQ1: u8 = 1 << 0;
const CONTROLLER_CONFIG_IRQ12: u8 = 1 << 1;
const CONTROLLER_CONFIG_FIRST_CLOCK_DISABLED: u8 = 1 << 4;
const CONTROLLER_CONFIG_SECOND_CLOCK_DISABLED: u8 = 1 << 5;
const CONTROLLER_CONFIG_TRANSLATION: u8 = 1 << 6;
const KEYBOARD_ACK: u8 = 0xfa;
const KEYBOARD_RESEND: u8 = 0xfe;
const IO_TIMEOUT_MS: u64 = 100;
const BAT_TIMEOUT_MS: u64 = 1_000;
const FALLBACK_TIMEOUT_SPINS: usize = 1_000_000;

/// Maximum number of buffered raw scancodes before overflow.
const RAW_BUFFER_SIZE: usize = 256;

/// Lock-free ring buffer for raw scancodes from IRQ1.
static RAW_SCANCODE_BUFFER: [AtomicU8; RAW_BUFFER_SIZE] =
    [const { AtomicU8::new(0) }; RAW_BUFFER_SIZE];

static WRITE_IDX: AtomicUsize = AtomicUsize::new(0);
static READ_IDX: AtomicUsize = AtomicUsize::new(0);
static DROPPED_COUNT: AtomicUsize = AtomicUsize::new(0);
static IRQ_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Test automation injects raw set-1 scancodes through the same ring used by
/// IRQ1. The userspace driver then performs the same translation path as real
/// hardware input.
#[cfg(feature = "key_inject")]
pub static mut KEY_INJECT_DATA: [u8; 256] = [0; 256];
#[cfg(feature = "key_inject")]
pub static mut KEY_INJECT_LEN: usize = 0;
#[cfg(feature = "key_inject")]
pub static mut KEY_INJECT_IDX: usize = 0;
#[cfg(feature = "key_inject")]
pub static mut KEY_INJECT_ENABLED: bool = false;

/// Endpoint ID of the registered user-space keyboard driver (sunlight-kbd).
/// Set via syscall during driver initialization. 0 = not registered.
static KBD_DRIVER_ENDPOINT: AtomicU32 = AtomicU32::new(0);

#[inline]
fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags)
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

fn timeout_expired(start_tsc: u64, timeout_ticks: u64, spins: usize) -> bool {
    if timeout_ticks != 0 {
        read_tsc().wrapping_sub(start_tsc) >= timeout_ticks
    } else {
        spins >= FALLBACK_TIMEOUT_SPINS
    }
}

fn timeout_ticks(timeout_ms: u64) -> u64 {
    crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(timeout_ms) / 1_000
}

fn wait_input_buffer_clear() -> bool {
    let start_tsc = read_tsc();
    let timeout_ticks = timeout_ticks(IO_TIMEOUT_MS);
    let mut spins = 0usize;
    unsafe {
        let mut status: Port<u8> = Port::new(STATUS_COMMAND_PORT);
        loop {
            if status.read() & STATUS_INPUT_FULL == 0 {
                return true;
            }
            spins = spins.saturating_add(1);
            if timeout_expired(start_tsc, timeout_ticks, spins) {
                break;
            }
            core::hint::spin_loop();
        }
    }
    false
}

fn read_data_timeout(timeout_ms: u64) -> Option<(u8, u8)> {
    let start_tsc = read_tsc();
    let timeout_ticks = timeout_ticks(timeout_ms);
    let mut spins = 0usize;
    unsafe {
        let mut status: Port<u8> = Port::new(STATUS_COMMAND_PORT);
        let mut data: Port<u8> = Port::new(DATA_PORT);
        loop {
            let status_byte = status.read();
            if status_byte & STATUS_OUTPUT_FULL != 0 {
                return Some((status_byte, data.read()));
            }
            spins = spins.saturating_add(1);
            if timeout_expired(start_tsc, timeout_ticks, spins) {
                break;
            }
            core::hint::spin_loop();
        }
    }
    None
}

unsafe fn write_controller_command(command: u8) -> bool {
    if !wait_input_buffer_clear() {
        return false;
    }
    serial_println!("[KBD-INIT] controller tx={:#04x}", command);
    let mut port: Port<u8> = Port::new(STATUS_COMMAND_PORT);
    unsafe { port.write(command) };
    true
}

unsafe fn write_data(data: u8) -> bool {
    if !wait_input_buffer_clear() {
        return false;
    }
    let mut port: Port<u8> = Port::new(DATA_PORT);
    unsafe { port.write(data) };
    true
}

fn drain_output_buffer() {
    unsafe {
        let mut status: Port<u8> = Port::new(STATUS_COMMAND_PORT);
        let mut data: Port<u8> = Port::new(DATA_PORT);
        // A controller has at most a tiny hardware FIFO. Keep this bounded in
        // case firmware reports a stuck output-full status bit.
        for _ in 0..32 {
            if status.read() & STATUS_OUTPUT_FULL == 0 {
                break;
            }
            let _ = data.read();
        }
    }
}

fn send_keyboard_command(command: u8) -> bool {
    for attempt in 1..=3 {
        serial_println!(
            "[KBD-INIT] keyboard tx={:#04x} attempt={}",
            command,
            attempt
        );
        if !unsafe { write_data(command) } {
            serial_println!("[KBD-INIT] keyboard tx timeout");
            return false;
        }
        match read_data_timeout(IO_TIMEOUT_MS) {
            Some((status, KEYBOARD_ACK)) if status & STATUS_AUX_DATA == 0 => {
                serial_println!("[KBD-INIT] keyboard ACK");
                return true;
            }
            Some((status, KEYBOARD_RESEND)) if status & STATUS_AUX_DATA == 0 => {
                serial_println!("[KBD-INIT] keyboard RESEND");
            }
            Some((status, byte)) => {
                serial_println!(
                    "[KBD-INIT] unexpected response={:#04x} source={}",
                    byte,
                    if status & STATUS_AUX_DATA != 0 {
                        "aux"
                    } else {
                        "keyboard"
                    }
                );
                return false;
            }
            None => {
                serial_println!("[KBD-INIT] keyboard response timeout");
                return false;
            }
        }
    }
    serial_println!("[KBD-INIT] keyboard RESEND limit reached");
    false
}

fn read_controller_config() -> Option<u8> {
    if !unsafe { write_controller_command(0x20) } {
        return None;
    }
    match read_data_timeout(IO_TIMEOUT_MS) {
        Some((status, byte)) if status & STATUS_AUX_DATA == 0 => Some(byte),
        Some((_, byte)) => {
            serial_println!(
                "[KBD-INIT] rejected AUX byte {:#04x} while reading controller config",
                byte
            );
            None
        }
        None => None,
    }
}

fn write_controller_config(config: u8) -> bool {
    if !unsafe { write_controller_command(0x60) } {
        return false;
    }
    serial_println!("[KBD-INIT] controller data={:#04x}", config);
    unsafe { write_data(config) }
}

fn read_keyboard_response(timeout_ms: u64) -> Option<u8> {
    match read_data_timeout(timeout_ms) {
        Some((status, byte)) if status & STATUS_AUX_DATA == 0 => Some(byte),
        Some((_, byte)) => {
            serial_println!(
                "[KBD-INIT] rejected AUX byte {:#04x} during keyboard transaction",
                byte
            );
            None
        }
        None => None,
    }
}

/// Put the first i8042 port and keyboard into a known, translated set-2 state.
///
/// Firmware often leaves this state usable in virtual machines, but physical
/// UEFI systems are allowed to hand off with the port or keyboard scanning
/// disabled. The userspace driver consumes set-1 bytes, so controller
/// translation is explicitly enabled while the device itself uses set 2.
pub fn init_ps2_keyboard() -> bool {
    serial_println!("[KBD] Initializing i8042 keyboard hardware");

    let result = (|| -> Result<(), &'static str> {
        // Stop both ports while controller state and device protocol are
        // changed. Interrupts are disabled throughout early kernel boot.
        if !unsafe { write_controller_command(0xad) } || !unsafe { write_controller_command(0xa7) }
        {
            return Err("port-disable timeout");
        }
        drain_output_buffer();

        let Some(original_config) = read_controller_config() else {
            return Err("no configuration byte");
        };
        serial_println!(
            "[KBD-INIT] controller config initial={:#04x}",
            original_config
        );

        // Keep both device clocks and interrupts disabled while testing the
        // first port. Translation is selected now, but cannot affect traffic
        // until the keyboard clock is enabled below.
        let quiescent_config = (original_config
            | CONTROLLER_CONFIG_FIRST_CLOCK_DISABLED
            | CONTROLLER_CONFIG_SECOND_CLOCK_DISABLED
            | CONTROLLER_CONFIG_TRANSLATION)
            & !(CONTROLLER_CONFIG_IRQ1 | CONTROLLER_CONFIG_IRQ12);
        if !write_controller_config(quiescent_config) {
            return Err("configuration write timeout");
        }

        if !unsafe { write_controller_command(0xab) } {
            return Err("interface-test timeout");
        }
        match read_data_timeout(IO_TIMEOUT_MS) {
            Some((status, 0x00)) if status & STATUS_AUX_DATA == 0 => {}
            Some((_, code)) => {
                serial_println!("[KBD] i8042 first-port test failed: {:#x}", code);
                return Err("first-port interface test failed");
            }
            None => return Err("no interface-test result"),
        }

        if !unsafe { write_controller_command(0xae) } {
            return Err("first-port enable timeout");
        }

        // Do not depend on the side effects of 0xAE. Program and then verify
        // every controller bit the keyboard path relies on.
        let keyboard_config =
            (quiescent_config | CONTROLLER_CONFIG_IRQ1) & !CONTROLLER_CONFIG_FIRST_CLOCK_DISABLED;
        if !write_controller_config(keyboard_config) {
            return Err("failed to enable keyboard clock and IRQ");
        }
        let Some(config) = read_controller_config() else {
            return Err("configuration read-back timed out");
        };
        serial_println!("[KBD-INIT] controller config readback={:#04x}", config);
        let required = CONTROLLER_CONFIG_IRQ1
            | CONTROLLER_CONFIG_SECOND_CLOCK_DISABLED
            | CONTROLLER_CONFIG_TRANSLATION;
        let forbidden = CONTROLLER_CONFIG_IRQ12 | CONTROLLER_CONFIG_FIRST_CLOCK_DISABLED;
        if config & required != required || config & forbidden != 0 {
            return Err("controller configuration read-back mismatch");
        }
        serial_println!("[KBD-INIT] translation=enabled (set 2 -> set 1)");

        // Reset and wait for the keyboard's Basic Assurance Test completion.
        if !send_keyboard_command(0xff) {
            return Err("keyboard reset was not acknowledged");
        }
        match read_keyboard_response(BAT_TIMEOUT_MS) {
            Some(0xaa) => {}
            Some(code) => {
                serial_println!("[KBD] keyboard self-test failed: {:#x}", code);
                return Err("keyboard self-test failed");
            }
            None => return Err("keyboard self-test timed out"),
        }
        serial_println!("[KBD-INIT] BAT=0xaa passed");

        // Establish a command-only transaction window before identify and
        // configuration so scan bytes cannot be mistaken for responses.
        if !send_keyboard_command(0xf5) {
            return Err("failed to disable keyboard scanning");
        }

        if !send_keyboard_command(0xf2) {
            return Err("keyboard identify was not acknowledged");
        }
        let Some(id0) = read_keyboard_response(IO_TIMEOUT_MS) else {
            return Err("keyboard identify first byte timed out");
        };
        serial_println!("[KBD-INIT] identify[0]={:#04x}", id0);
        let Some(id1) = read_keyboard_response(IO_TIMEOUT_MS) else {
            return Err("keyboard identify second byte timed out");
        };
        serial_println!("[KBD-INIT] identify[1]={:#04x}", id1);
        if id0 != 0xab && id0 != 0xac {
            return Err("device on first port is not an AT keyboard");
        }

        if !send_keyboard_command(0xf0) || !send_keyboard_command(0x02) {
            return Err("failed to select scancode set 2");
        }
        if !send_keyboard_command(0xed) || !send_keyboard_command(0x00) {
            return Err("failed to initialize keyboard LEDs");
        }
        if !send_keyboard_command(0xf4) {
            return Err("failed to enable keyboard scanning");
        }
        Ok(())
    })();

    if let Err(error) = result {
        serial_println!("[KBD] i8042 initialization failed: {}", error);
        // Keep the physical port reachable for diagnostics, but do not issue
        // further device commands after a failed transaction.
        let _ = unsafe { write_controller_command(0xae) };
        return false;
    }

    serial_println!("[KBD] i8042 keyboard ready (translated set 2, IRQ1 enabled)");
    true
}

/// Register the user-space keyboard driver endpoint.
/// Called by sunlight-kbd during initialization via syscall.
pub fn register_kbd_driver(endpoint_id: u32) {
    KBD_DRIVER_ENDPOINT.store(endpoint_id, Ordering::Release);
    serial_println!(
        "[KBD] Registered user-space driver at endpoint {}",
        endpoint_id
    );
}

/// Unregister the keyboard driver (for cleanup/testing).
pub fn unregister_kbd_driver() {
    KBD_DRIVER_ENDPOINT.store(0, Ordering::Release);
}

pub fn unregister_kbd_endpoint(endpoint_id: u32) {
    let _ =
        KBD_DRIVER_ENDPOINT.compare_exchange(endpoint_id, 0, Ordering::AcqRel, Ordering::Acquire);
}

/// Push a raw scancode into the ring buffer (non-blocking).
/// Returns false if buffer is full (scancode dropped).
fn push_scancode(scancode: u8) -> bool {
    let write = WRITE_IDX.load(Ordering::Relaxed);
    let read = READ_IDX.load(Ordering::Acquire);
    let next = (write + 1) % RAW_BUFFER_SIZE;

    if next == read {
        // Buffer full
        DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
        false
    } else {
        RAW_SCANCODE_BUFFER[write].store(scancode, Ordering::Release);
        WRITE_IDX.store(next, Ordering::Release);
        true
    }
}

/// Pop a raw scancode from the ring buffer (called by syscall from user-space).
/// Returns None if buffer is empty.
pub fn pop_scancode() -> Option<u8> {
    let read = READ_IDX.load(Ordering::Relaxed);
    let write = WRITE_IDX.load(Ordering::Acquire);

    if read == write {
        None
    } else {
        let scancode = RAW_SCANCODE_BUFFER[read].load(Ordering::Acquire);
        READ_IDX.store((read + 1) % RAW_BUFFER_SIZE, Ordering::Release);
        Some(scancode)
    }
}

/// Get statistics for monitoring/debugging.
pub fn get_stats() -> (usize, usize, usize) {
    let dropped = DROPPED_COUNT.load(Ordering::Relaxed);
    let read = READ_IDX.load(Ordering::Relaxed);
    let write = WRITE_IDX.load(Ordering::Relaxed);
    let pending = if write >= read {
        write - read
    } else {
        RAW_BUFFER_SIZE - read + write
    };
    (pending, dropped, RAW_BUFFER_SIZE)
}

/// Main IRQ1 handler: read raw byte, push to buffer, notify driver, send EOI.
pub fn handle_irq1() {
    if super::mouse::command_active() {
        return;
    }

    let status = unsafe {
        let mut status: Port<u8> = Port::new(STATUS_COMMAND_PORT);
        status.read()
    };
    // The keyboard and auxiliary device share port 0x60. A command-response
    // edge can remain pending while a later mouse byte reaches the output
    // buffer, so validate both OBF and the source bit before consuming it.
    if status & STATUS_OUTPUT_FULL == 0 || status & STATUS_AUX_DATA != 0 {
        return;
    }

    if IRQ_COUNT.fetch_add(1, Ordering::Relaxed) == 0 {
        serial_println!("[KBD] first hardware IRQ1 received");
    }

    // 1. Read raw scancode from hardware
    let scancode = unsafe {
        let mut port: Port<u8> = Port::new(DATA_PORT);
        port.read()
    };
    if IRQ_COUNT.load(Ordering::Relaxed) == 1 {
        serial_println!("[KBD] first scan code={:#04x}", scancode);
    }

    // 2. Push to ring buffer (non-blocking)
    let pushed = push_scancode(scancode);

    // 3. Notify user-space driver if registered and scancode was buffered
    let endpoint = KBD_DRIVER_ENDPOINT.load(Ordering::Acquire);
    if endpoint != 0 && pushed {
        notify_driver(endpoint, scancode);
    }
}

/// Compatibility stub: timer polling for key injection (deprecated).
/// The user-space driver now handles all key event processing.
pub fn poll_inject_buffer() {
    #[cfg(feature = "key_inject")]
    {
        if KBD_DRIVER_ENDPOINT.load(Ordering::Acquire) == 0 {
            return;
        }

        let scancode = unsafe {
            if !KEY_INJECT_ENABLED || KEY_INJECT_IDX >= KEY_INJECT_LEN {
                return;
            }
            let scancode = KEY_INJECT_DATA[KEY_INJECT_IDX];
            KEY_INJECT_IDX += 1;
            if KEY_INJECT_IDX >= KEY_INJECT_LEN {
                KEY_INJECT_ENABLED = false;
                serial_println!("[KBD]  Key injection complete");
            }
            scancode
        };

        if push_scancode(scancode) {
            notify_driver(KBD_DRIVER_ENDPOINT.load(Ordering::Acquire), scancode);
        }
    }
}

fn notify_driver(endpoint: u32, _scancode: u8) {
    use crate::sched::SCHEDULER;

    let mut sched = SCHEDULER.lock();

    let server_pid = sched
        .processes
        .iter()
        .find(|p| {
            p.name_str() == "sunlight-kbd"
                && !matches!(
                    p.state,
                    crate::process::ProcessState::Finished | crate::process::ProcessState::Reaped
                )
        })
        .map(|p| p.pid)
        .unwrap_or(0);

    if server_pid != 0 {
        crate::ipc::with_shard(endpoint, |bus| {
            bus.send_input_notification(endpoint);
        });
        sched.wake_pid(server_pid);
    }
}
