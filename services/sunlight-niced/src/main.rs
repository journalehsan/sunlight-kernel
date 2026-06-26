//! sunlight-niced - nice/priority daemon for SunlightOS.
//!
//! Monitors process CPU usage via the TelemetryPage and applies dynamic
//! nice-value penalties/recovery, plus a watchdog (ping) protocol for
//! supervised services. See docs/NICED_GCD_IMPL.md for the full design.

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 256 * 1024] = [0; 256 * 1024];
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

/// Local serial-logging macro (mirrors the pattern used by sunlightd/timed):
/// formats into a fixed-size heapless::String and forwards to
/// `sunlight_ipc::debug_log`.
#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        sunlight_ipc::debug_log(&buf);
    }};
}

mod action;
mod config;
mod ipc;
mod monitor;
mod shim;
mod telemetry;
mod watchdog;

use sunlight_ipc::{
    endpoint_create, get_time_utc, ipc_reply_and_try_recv, nameserver_register, IpcMsg,
};

use config::load_defaults;
use monitor::Table;
use shim::RealKernelOps;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[NICED] PANIC: {}", _info);
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[NICED] Starting sunlight-niced v0.1");

    let (configs, dyn_rules, recovery_rules) = load_defaults();
    serial_println!(
        "[NICED] Loaded {} configs, {} rules",
        configs.len(),
        dyn_rules.len() + recovery_rules.len()
    );

    let tel = telemetry::Telemetry::init();
    match &tel {
        Ok(_) => serial_println!("[NICED] TelemetryPage mapped (read-only)"),
        Err(e) => serial_println!("[NICED] TelemetryPage mapping FAILED: {}", e),
    }
    let mut tel = tel.ok();

    let ep = endpoint_create();
    nameserver_register("niced", ep);
    serial_println!("[NICED] Registered as 'niced'");

    let mut table = Table::new();
    let shim = RealKernelOps;
    let now0 = get_time_utc();

    serial_println!("[NICED] Monitor loop started (1Hz)");
    serial_println!("[SunlightOS] niced OK");

    let mut reply = IpcMsg::empty();
    let mut last_tick: u64 = now0;
    loop {
        let now = get_time_utc();

        if now != last_tick {
            last_tick = now;
            if let Some(t) = tel.as_mut() {
                let _ = t.poll();
                monitor::tick(
                    &mut table,
                    t,
                    &shim,
                    &configs,
                    &dyn_rules,
                    &recovery_rules,
                    now,
                );
                watchdog::tick(&mut table, t, &shim, now);
            }
        }

        // Non-blocking IPC poll so the 1Hz monitor loop keeps making progress
        // even when no watchdog client is currently calling in.
        match ipc_reply_and_try_recv(ep, reply) {
            Some(msg) => {
                reply = watchdog::handle_message(&mut table, &msg, now);
            }
            None => {
                reply = IpcMsg::empty();
                sunlight_ipc::process_yield();
            }
        }
    }
}
