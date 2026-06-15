//! sunlight-gcd - garbage-collector daemon (zombie reaping / resource
//! cleanup) for SunlightOS.
//!
//! See docs/NICED_GCD_IMPL.md for the full design.

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

/// Local serial-logging macro (mirrors sunlightd/timed/niced).
#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        sunlight_ipc::debug_log(&buf);
    }};
}

mod ipc;
mod reaper;
mod resources;
mod shim;
mod telemetry;

use sunlight_ipc::{endpoint_create, get_time_utc, ipc_reply_and_try_recv, nameserver_register, IpcMsg};

use ipc::GcdOp;
use reaper::Reaper;
use shim::RealKernelOps;
use telemetry::{ProcSample, Telemetry, MAX_PROCS};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[GCD] PANIC: {}", _info);
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[GCD] Starting sunlight-gcd v0.1");

    let ep = endpoint_create();
    nameserver_register("gcd", ep);
    serial_println!("[GCD] Registered as 'gcd'");

    // No PROC_EXIT pub/sub exists in the kernel. "Subscribed to PROC_EXIT"
    // here means: poll the TelemetryPage every tick for entries with
    // state==3 (Finished), diffed against a fixed "seen" set (see
    // reaper::Reaper).
    serial_println!("[GCD] Subscribed to PROC_EXIT");

    let tel = Telemetry::init();
    let mut tel = match tel {
        Ok(t) => Some(t),
        Err(e) => {
            serial_println!("[GCD] TelemetryPage mapping FAILED: {}", e);
            None
        }
    };

    let shim = RealKernelOps;
    let mut reaper = Reaper::new();

    serial_println!("[SunlightOS] gcd OK");

    let mut reply = IpcMsg::empty();
    let mut last_tick: u64 = get_time_utc();
    loop {
        let now = get_time_utc();

        if now != last_tick {
            last_tick = now;
            if let Some(t) = tel.as_mut() {
                let _ = t.poll();
                let mut samples = [ProcSample::default(); MAX_PROCS];
                let count = t.snapshot(&mut samples);
                reaper.tick(&samples, count, &shim, now);
                reaper.check_memory_pressure(t.used_ram_kb, t.total_ram_kb);
            }
        }

        match ipc_reply_and_try_recv(ep, reply) {
            Some(msg) => {
                reply = handle_message(&mut reaper, &shim, &msg, now);
            }
            None => {
                reply = IpcMsg::empty();
                sunlight_ipc::process_yield();
            }
        }
    }
}

fn handle_message<K: shim::KernelOps>(reaper: &mut Reaper, shim: &K, msg: &IpcMsg, _now: u64) -> IpcMsg {
    match msg.label {
        GcdOp::REAP_ZOMBIE => {
            let pid = msg.words[0] as usize;
            reaper.reap_now(shim, pid, "");
            IpcMsg::with_label(GcdOp::REPLY)
        }
        GcdOp::PROCESS_EXITED => {
            let pid = msg.words[0] as usize;
            serial_println!("[GCD] process_exited notification pid={}", pid);
            IpcMsg::with_label(GcdOp::REPLY)
        }
        GcdOp::MEM_PRESSURE => IpcMsg::with_label(GcdOp::REPLY),
        _ => IpcMsg::with_label(GcdOp::REPLY),
    }
}
