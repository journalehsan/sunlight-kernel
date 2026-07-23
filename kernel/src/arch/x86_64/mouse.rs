//! PS/2 mouse IRQ12 raw byte router.
//!
//! Minimal kernel-side IRQ12 handler that reads the raw byte from
//! port 0x60 and forwards it to the registered user-space mouse driver
//! (sunlight-mouse) via a lock-free ring buffer.

use crate::serial_println;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, AtomicUsize, Ordering};
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
const DEVICE_ACK: u8 = 0xfa;
const DEVICE_RESEND: u8 = 0xfe;
const IO_TIMEOUT_MS: u64 = 100;
const BAT_TIMEOUT_MS: u64 = 1_000;
const FALLBACK_TIMEOUT_SPINS: usize = 1_000_000;

/// Maximum number of buffered raw mouse bytes before overflow.
const RAW_BUFFER_SIZE: usize = 256;

/// Lock-free ring buffer for raw mouse bytes from IRQ12.
static RAW_MOUSE_BUFFER: [AtomicU8; RAW_BUFFER_SIZE] =
    [const { AtomicU8::new(0) }; RAW_BUFFER_SIZE];

static WRITE_IDX: AtomicUsize = AtomicUsize::new(0);
static READ_IDX: AtomicUsize = AtomicUsize::new(0);
static DROPPED_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Endpoint ID of the registered user-space mouse driver (sunlight-mouse).
static MOUSE_DRIVER_ENDPOINT: AtomicU32 = AtomicU32::new(0);
static IRQ12_LOG_COUNT: AtomicUsize = AtomicUsize::new(0);
static COMMAND_ACTIVE: AtomicBool = AtomicBool::new(false);

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

fn timeout_ticks(timeout_ms: u64) -> u64 {
    crate::arch::x86_64::interrupts::tsc_hz().saturating_mul(timeout_ms) / 1_000
}

fn timeout_expired(start_tsc: u64, timeout_ticks: u64, spins: usize) -> bool {
    if timeout_ticks != 0 {
        read_tsc().wrapping_sub(start_tsc) >= timeout_ticks
    } else {
        spins >= FALLBACK_TIMEOUT_SPINS
    }
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

fn read_data_timeout(timeout_ms: u64) -> Option<u8> {
    let start_tsc = read_tsc();
    let timeout_ticks = timeout_ticks(timeout_ms);
    let mut spins = 0usize;
    unsafe {
        let mut status: Port<u8> = Port::new(STATUS_COMMAND_PORT);
        let mut data: Port<u8> = Port::new(DATA_PORT);
        loop {
            if status.read() & STATUS_OUTPUT_FULL != 0 {
                return Some(data.read());
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

unsafe fn write_cmd(cmd: u8) -> bool {
    if !wait_input_buffer_clear() {
        return false;
    }
    let mut port: Port<u8> = Port::new(STATUS_COMMAND_PORT);
    port.write(cmd);
    true
}

unsafe fn write_data(data: u8) -> bool {
    if !wait_input_buffer_clear() {
        return false;
    }
    let mut port: Port<u8> = Port::new(DATA_PORT);
    port.write(data);
    true
}

fn drain_output_buffer() {
    unsafe {
        let mut status: Port<u8> = Port::new(STATUS_COMMAND_PORT);
        let mut data: Port<u8> = Port::new(DATA_PORT);
        for _ in 0..32 {
            if status.read() & STATUS_OUTPUT_FULL == 0 {
                break;
            }
            let _ = data.read();
        }
    }
}

fn send_aux_command(command: u8) -> bool {
    for _ in 0..2 {
        if !unsafe { write_cmd(0xd4) } || !unsafe { write_data(command) } {
            return false;
        }
        match read_data_timeout(IO_TIMEOUT_MS) {
            Some(DEVICE_ACK) => return true,
            Some(DEVICE_RESEND) => continue,
            _ => return false,
        }
    }
    false
}

fn configure_aux_irq(enabled: bool) -> bool {
    if !unsafe { write_cmd(0x20) } {
        return false;
    }
    let Some(mut config) = read_data_timeout(IO_TIMEOUT_MS) else {
        return false;
    };
    config |= CONTROLLER_CONFIG_IRQ1;
    config &= !CONTROLLER_CONFIG_FIRST_CLOCK_DISABLED;
    if enabled {
        config |= CONTROLLER_CONFIG_IRQ12;
        config &= !CONTROLLER_CONFIG_SECOND_CLOCK_DISABLED;
    } else {
        config &= !CONTROLLER_CONFIG_IRQ12;
        config |= CONTROLLER_CONFIG_SECOND_CLOCK_DISABLED;
    }
    unsafe { write_cmd(0x60) && write_data(config) }
}

fn reset_raw_buffer() {
    READ_IDX.store(WRITE_IDX.load(Ordering::Acquire), Ordering::Release);
}

/// Initialize the PS/2 auxiliary mouse port in ring 0 so keyboard IRQ handling
/// cannot consume the controller command-byte response while user space waits.
pub fn init_ps2_mouse() -> bool {
    serial_println!("[MOUSE] Initializing PS/2 mouse hardware");
    COMMAND_ACTIVE.store(true, Ordering::Release);

    let success = x86_64::instructions::interrupts::without_interrupts(|| {
        let result = (|| -> Result<u8, &'static str> {
            if !unsafe { write_cmd(0xa7) } {
                return Err("aux disable timeout");
            }
            drain_output_buffer();
            if !configure_aux_irq(false) {
                return Err("controller configuration timeout");
            }
            if !unsafe { write_cmd(0xa8) } {
                return Err("aux enable timeout");
            }
            if !unsafe { write_cmd(0xa9) } {
                return Err("aux interface-test timeout");
            }
            match read_data_timeout(IO_TIMEOUT_MS) {
                Some(0x00) => {}
                Some(code) => {
                    serial_println!("[MOUSE] auxiliary interface test failed: {:#x}", code);
                    return Err("auxiliary interface test failed");
                }
                None => return Err("no auxiliary interface-test result"),
            }

            // UEFI may leave a ThinkPad Synaptics clickpad in its native six-byte
            // protocol. Resetting restores standard three-byte relative packets,
            // where pressing the single mechanical clickpad button is bit 0.
            if !send_aux_command(0xff) {
                return Err("device reset was not acknowledged");
            }
            match read_data_timeout(BAT_TIMEOUT_MS) {
                Some(0xaa) => {}
                Some(code) => {
                    serial_println!("[MOUSE] device self-test failed: {:#x}", code);
                    return Err("device self-test failed");
                }
                None => return Err("device self-test timed out"),
            }
            let device_id =
                read_data_timeout(IO_TIMEOUT_MS).ok_or("device ID timed out after reset")?;

            if !send_aux_command(0xf6) {
                return Err("failed to restore device defaults");
            }
            if !configure_aux_irq(true) {
                return Err("failed to enable auxiliary IRQ");
            }
            if !send_aux_command(0xf4) {
                return Err("failed to enable data reporting");
            }
            Ok(device_id)
        })();

        match result {
            Ok(device_id) => {
                reset_raw_buffer();
                serial_println!(
                    "[MOUSE] PS/2 pointing device ready in relative mode (id={:#x})",
                    device_id
                );
                true
            }
            Err(error) => {
                serial_println!("[MOUSE] full initialization failed: {}", error);
                // Preserve compatibility with controllers that reject reset or
                // interface-test commands but still support basic reporting.
                drain_output_buffer();
                let _ = unsafe { write_cmd(0xa8) };
                let irq = configure_aux_irq(true);
                let reporting = send_aux_command(0xf4);
                if reporting && irq {
                    reset_raw_buffer();
                    serial_println!("[MOUSE] basic reporting fallback enabled");
                    true
                } else {
                    serial_println!("[MOUSE] basic reporting fallback failed");
                    false
                }
            }
        }
    });

    COMMAND_ACTIVE.store(false, Ordering::Release);
    success
}

pub(super) fn command_active() -> bool {
    COMMAND_ACTIVE.load(Ordering::Acquire)
}

fn controller_has_aux_data() -> bool {
    let status = unsafe {
        let mut status: Port<u8> = Port::new(STATUS_COMMAND_PORT);
        status.read()
    };
    status & STATUS_OUTPUT_FULL != 0 && status & STATUS_AUX_DATA != 0
}

unsafe fn read_aux_byte() -> u8 {
    let mut port: Port<u8> = Port::new(DATA_PORT);
    port.read()
}

fn irq_byte() -> Option<u8> {
    if command_active() || !controller_has_aux_data() {
        None
    } else {
        Some(unsafe { read_aux_byte() })
    }
}

/// Register the user-space mouse driver endpoint.
pub fn register_mouse_driver(endpoint_id: u32) {
    MOUSE_DRIVER_ENDPOINT.store(endpoint_id, Ordering::Release);
    serial_println!(
        "[MOUSE] Registered user-space driver at endpoint {}",
        endpoint_id
    );
}

/// Unregister the mouse driver.
pub fn unregister_mouse_driver() {
    MOUSE_DRIVER_ENDPOINT.store(0, Ordering::Release);
}

pub fn unregister_mouse_endpoint(endpoint_id: u32) {
    let _ =
        MOUSE_DRIVER_ENDPOINT.compare_exchange(endpoint_id, 0, Ordering::AcqRel, Ordering::Acquire);
}

/// Push a raw mouse byte into the ring buffer (non-blocking).
fn push_mouse_byte(byte: u8) -> bool {
    let write = WRITE_IDX.load(Ordering::Relaxed);
    let read = READ_IDX.load(Ordering::Acquire);
    let next = (write + 1) % RAW_BUFFER_SIZE;

    if next == read {
        DROPPED_COUNT.fetch_add(1, Ordering::Relaxed);
        false
    } else {
        RAW_MOUSE_BUFFER[write].store(byte, Ordering::Release);
        WRITE_IDX.store(next, Ordering::Release);
        true
    }
}

/// Pop a raw mouse byte from the ring buffer.
pub fn pop_mouse_byte() -> Option<u8> {
    let read = READ_IDX.load(Ordering::Relaxed);
    let write = WRITE_IDX.load(Ordering::Acquire);

    if read == write {
        None
    } else {
        let byte = RAW_MOUSE_BUFFER[read].load(Ordering::Acquire);
        READ_IDX.store((read + 1) % RAW_BUFFER_SIZE, Ordering::Release);
        Some(byte)
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

/// Main IRQ12 handler: read a raw byte and notify the userspace driver.
/// Interrupt-controller acknowledgement is owned by `interrupts.rs`, which
/// selects LAPIC or legacy PIC EOI according to the active route.
pub fn handle_irq12() {
    let Some(byte) = irq_byte() else {
        return;
    };

    // 1. Push to ring buffer (non-blocking)
    let pushed = push_mouse_byte(byte);

    // 2. Notify user-space driver if registered
    let endpoint = MOUSE_DRIVER_ENDPOINT.load(Ordering::Acquire);
    if endpoint != 0 && pushed {
        use crate::sched::SCHEDULER;

        let mut sched = SCHEDULER.lock();

        let server_pid = sched
            .processes
            .iter()
            .find(|p| {
                p.name_str() == "sunlight-mouse"
                    && !matches!(
                        p.state,
                        crate::process::ProcessState::Finished
                            | crate::process::ProcessState::Reaped
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
}
