//! PS/2 keyboard IRQ1 raw byte router.
//!
//! Minimal kernel-side IRQ1 handler that reads the raw scancode byte from
//! port 0x60 and forwards it to the registered user-space keyboard driver
//! (sunlight-kbd) via a lock-free ring buffer. The kernel does NOT perform
//! any scancode translation, modifier tracking, or ASCII conversion.

use crate::serial_println;
use core::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering};
use x86_64::instructions::port::Port;

/// Maximum number of buffered raw scancodes before overflow.
const RAW_BUFFER_SIZE: usize = 256;

/// Lock-free ring buffer for raw scancodes from IRQ1.
static RAW_SCANCODE_BUFFER: [AtomicU8; RAW_BUFFER_SIZE] =
    [const { AtomicU8::new(0) }; RAW_BUFFER_SIZE];

static WRITE_IDX: AtomicUsize = AtomicUsize::new(0);
static READ_IDX: AtomicUsize = AtomicUsize::new(0);
static DROPPED_COUNT: AtomicUsize = AtomicUsize::new(0);

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
    // 1. Read raw scancode from hardware
    let scancode = unsafe {
        let mut port: Port<u8> = Port::new(0x60);
        port.read()
    };

    // 2. Push to ring buffer (non-blocking)
    let pushed = push_scancode(scancode);

    // 3. Notify user-space driver if registered and scancode was buffered
    let endpoint = KBD_DRIVER_ENDPOINT.load(Ordering::Acquire);
    if endpoint != 0 && pushed {
        notify_driver(endpoint, scancode);
    }

    // 4. Send EOI to PIC
    unsafe {
        let mut cmd1: Port<u8> = Port::new(0x20);
        cmd1.write(0x20);
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
