use crate::serial_println;
use core::panic::PanicInfo;

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    serial_println!("[KERNEL PANIC] {}", info);
    loop {
        core::arch::x86_64::_mm_pause();
    }
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    serial_println!(
        "[OOM] Allocation of {} bytes (align={}) failed",
        layout.size(),
        layout.align()
    );

    // Print PMM diagnostic to see if physical frames are leaking
    if let Some(pmm) = crate::PMM.try_lock() {
        pmm.diagnostic_report();
    }

    crate::memory::heap::heap_diagnostic();

    // Try to reclaim memory from finished process slots. The kernel stack
    // (32 KiB Box) and env/fd_table allocations are held until the slot is
    // reused or dropped. Finding and dropping finished slots frees that
    // kernel-heap memory.
    if let Some(mut sched) = crate::sched::SCHEDULER.try_lock() {
        let mut reaped = 0usize;
        for idx in 0..sched.processes.len() {
            if matches!(
                sched.processes[idx].state,
                crate::process::ProcessState::Finished | crate::process::ProcessState::Reaped
            ) {
                // Clear heap-allocated fields to free kernel-heap memory
                // without touching PMM or IPC locks (best-effort).
                sched.processes[idx].ipc_queue.clear();
                sched.processes[idx].capabilities.clear();
                sched.processes[idx].ipc_reply = None;
                sched.processes[idx].ipc_endpoint = None;
                sched.processes[idx].pending_call = None;
                sched.processes[idx].pending_reply_wait = None;
                sched.processes[idx].ipc_reply_target = None;
                sched.processes[idx].env = crate::process::env::EnvMap::new();
                sched.processes[idx].owned_shared.clear();
                sched.processes[idx].mapped_shared.clear();
                sched.processes[idx].cwd = alloc::string::String::new();
                // This is a best-effort emergency purge; the full reaping
                // (PMM frames, IPC endpoints, etc.) remains via the normal
                // reap code path in schedule_tick / terminate_process_by_pid.
                reaped += 1;
            }
        }
        if reaped > 0 {
            serial_println!("[OOM] Emergency-purged {} finished process slots", reaped);
        }
    }

    // Panic instead of hanging, so the failure is immediately visible.
    panic!(
        "[OOM] Allocation of {} bytes (align={}) failed",
        layout.size(),
        layout.align()
    );
}
