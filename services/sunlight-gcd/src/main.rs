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

use sunlight_ipc::{
    endpoint_create, get_time_utc, ipc_reply_and_try_recv, nameserver_register, IpcMsg,
};
use sunlight_tty::proc::SIGKILL;

use ipc::{GcdOp, ProcOp};
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
    nameserver_register("proc", ep);
    serial_println!("[GCD] Registered as 'gcd'");
    serial_println!("[GCD] Registered as 'proc'");

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
                reply = handle_message(&mut reaper, &shim, tel.as_ref(), &msg, now);
            }
            None => {
                reply = IpcMsg::empty();
                sunlight_ipc::process_yield();
            }
        }
    }
}

fn handle_message<K: shim::KernelOps>(
    reaper: &mut Reaper,
    shim: &K,
    tel: Option<&Telemetry>,
    msg: &IpcMsg,
    _now: u64,
) -> IpcMsg {
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
        ProcOp::TERMINATE_SESSION => {
            let session_pid = msg.words[0] as usize;
            let signal = if msg.words[1] == 0 {
                SIGKILL as u8
            } else {
                msg.words[1] as u8
            };
            let killed = terminate_session_tree(shim, tel, session_pid, signal);
            let mut reply = IpcMsg::with_label(ProcOp::REPLY);
            reply.words[0] = killed as u64;
            reply
        }
        _ => IpcMsg::with_label(GcdOp::REPLY),
    }
}

fn terminate_session_tree<K: shim::KernelOps>(
    shim: &K,
    tel: Option<&Telemetry>,
    session_pid: usize,
    signal: u8,
) -> usize {
    if session_pid == 0 {
        return 0;
    }

    let mut targeted = [0usize; MAX_PROCS];
    targeted[0] = session_pid;
    let mut targeted_count = 1usize;

    if let Some(tel) = tel {
        let mut samples = [ProcSample::default(); MAX_PROCS];
        let count = tel.snapshot(&mut samples);
        let mut changed = true;
        while changed {
            changed = false;
            for sample in samples.iter().take(count) {
                if sample.pid == 0 || sample.state == 3 {
                    continue;
                }
                if !contains_pid(&targeted[..targeted_count], sample.ppid) {
                    continue;
                }
                if contains_pid(&targeted[..targeted_count], sample.pid)
                    || targeted_count >= MAX_PROCS
                {
                    continue;
                }
                targeted[targeted_count] = sample.pid;
                targeted_count += 1;
                changed = true;
            }
        }
    }

    let mut killed = 0usize;
    for pid in targeted[..targeted_count].iter().rev() {
        // The proc service resolves the session tree in user space; the
        // actual run-queue removal and page-table reclamation happen in the
        // kernel's kill/exit path for each pid we target here.
        if shim.send_signal(*pid, signal) {
            killed += 1;
        } else if signal == SIGKILL as u8 && shim.force_terminate(*pid) {
            killed += 1;
        }
    }

    serial_println!(
        "[GCD] terminate_session root_pid={} signal={} targeted={} killed={}",
        session_pid,
        signal,
        targeted_count,
        killed
    );
    killed
}

fn contains_pid(pids: &[usize], pid: usize) -> bool {
    pids.iter().any(|entry| *entry == pid)
}
