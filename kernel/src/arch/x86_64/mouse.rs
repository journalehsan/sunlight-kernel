//! PS/2 mouse IRQ12 raw byte router.
//!
//! Minimal kernel-side IRQ12 handler that reads the raw byte from
//! port 0x60 and forwards it to the registered user-space mouse driver
//! (sunlight-mouse) via a lock-free ring buffer.

use crate::serial_println;
use core::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering};
use x86_64::instructions::port::Port;

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

fn wait_input_buffer_clear() -> bool {
    unsafe {
        let mut status: Port<u8> = Port::new(0x64);
        for _ in 0..100_000 {
            if status.read() & 0x02 == 0 {
                return true;
            }
            core::hint::spin_loop();
        }
    }
    false
}

fn wait_output_buffer_full() -> bool {
    unsafe {
        let mut status: Port<u8> = Port::new(0x64);
        for _ in 0..100_000 {
            if status.read() & 0x01 != 0 {
                return true;
            }
            core::hint::spin_loop();
        }
    }
    false
}

unsafe fn write_cmd(cmd: u8) -> bool {
    if !wait_input_buffer_clear() {
        return false;
    }
    let mut port: Port<u8> = Port::new(0x64);
    port.write(cmd);
    true
}

unsafe fn write_data(data: u8) -> bool {
    if !wait_input_buffer_clear() {
        return false;
    }
    let mut port: Port<u8> = Port::new(0x60);
    port.write(data);
    true
}

unsafe fn read_data() -> Option<u8> {
    if !wait_output_buffer_full() {
        return None;
    }
    let mut port: Port<u8> = Port::new(0x60);
    Some(port.read())
}

/// Initialize the PS/2 auxiliary mouse port in ring 0 so keyboard IRQ handling
/// cannot consume the controller command-byte response while user space waits.
pub fn init_ps2_mouse() -> bool {
    serial_println!("[MOUSE] Initializing PS/2 mouse hardware");
    unsafe {
        if !write_cmd(0xA8) {
            serial_println!("[MOUSE] init failed: aux enable timeout");
            return false;
        }

        if !write_cmd(0x20) {
            serial_println!("[MOUSE] init failed: command-byte read timeout");
            return false;
        }
        let Some(mut status) = read_data() else {
            serial_println!("[MOUSE] init failed: no command-byte response");
            return false;
        };
        status |= 0x01 | 0x02;
        status &= !(0x10 | 0x20);

        if !write_cmd(0x60) || !write_data(status) {
            serial_println!("[MOUSE] init failed: command-byte write timeout");
            return false;
        }

        if !write_cmd(0xD4) || !write_data(0xF4) {
            serial_println!("[MOUSE] init failed: enable-reporting timeout");
            return false;
        }

        match read_data() {
            Some(0xFA) => {
                serial_println!("[MOUSE] PS/2 mouse initialized successfully");
                true
            }
            Some(byte) => {
                serial_println!("[MOUSE] init failed: expected ACK, got {:#x}", byte);
                false
            }
            None => {
                serial_println!("[MOUSE] init failed: no ACK");
                false
            }
        }
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

/// Main IRQ12 handler: read raw byte, push to buffer, notify driver, send EOI.
pub fn handle_irq12() {
    // 1. Read raw mouse byte from hardware
    let byte = unsafe {
        let mut port: Port<u8> = Port::new(0x60);
        port.read()
    };

    // 2. Push to ring buffer (non-blocking)
    let pushed = push_mouse_byte(byte);

    // 3. Notify user-space driver if registered
    let endpoint = MOUSE_DRIVER_ENDPOINT.load(Ordering::Acquire);
    if endpoint != 0 && pushed {
        use crate::sched::SCHEDULER;

        let mut sched = SCHEDULER.lock();

        let server_pid = sched
            .processes
            .iter()
            .find(|p| p.name_str() == "sunlight-mouse")
            .map(|p| p.pid)
            .unwrap_or(0);

        if server_pid != 0 {
            crate::ipc::with_shard(endpoint, |bus| {
                bus.send_keyboard_event(endpoint, byte as u64, &mut sched, server_pid);
            });
        }
    }

    // 4. Send EOI to slave PIC first, then master PIC (IRQ12 is on PIC2).
    unsafe {
        let mut cmd2: Port<u8> = Port::new(0xA0);
        let mut cmd1: Port<u8> = Port::new(0x20);
        cmd2.write(0x20);
        cmd1.write(0x20);
    }
}
