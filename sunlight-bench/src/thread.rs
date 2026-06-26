//! Userspace thread spawn using SunlightOS ThreadSpawn syscall (22).
//!
//! ABI (from kernel/src/arch/x86_64/syscall.rs):
//!   rdi = trampoline fn pointer
//!   rsi = user_stack_top — kernel reads [rsi+0]=func, [rsi+8]=arg
//!   rdx = FS_BASE (TLS; 0 = none)
//!
//! Kernel then starts the new thread with:
//!   RIP = trampoline, RSP = user_stack_top, RDI = func, RSI = arg

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

pub const THREAD_STACK_BYTES: usize = 64 * 1024; // 64 KiB per thread stack

/// The trampoline receives func in RDI and arg in RSI (set by the kernel via
/// `set_initial_args`), then calls the actual benchmark worker.
unsafe extern "C" fn thread_trampoline(func: u64, arg: u64) -> ! {
    let f: unsafe extern "C" fn(u64) = core::mem::transmute(func as *const ());
    f(arg);
    // The kernel currently treats ProcessExit as process-wide teardown even
    // for ThreadSpawn children, so benchmark workers must not return through
    // ProcessExit or they will unmap the shared address space out from under
    // the rest of the benchmark. Park until the main benchmark process exits.
    loop {
        sunlight_ipc::process_yield();
    }
}

/// Spawn a thread running `func(arg)`.  Returns the new thread ID, or 0 on error.
///
/// `stack` must live at least as long as the thread.  Ownership is transferred
/// to the caller; nothing reclaims it automatically (bump allocator, no free).
pub fn spawn(func: unsafe extern "C" fn(u64), arg: u64) -> (u64, Vec<u8>) {
    let mut stack = alloc::vec![0u8; THREAD_STACK_BYTES];

    // Write [func, arg] at the TOP of the stack (highest addresses).
    // The kernel reads them from [rsi+0] and [rsi+8], then sets
    // RDI=func and RSI=arg on the new thread, so the trampoline receives
    // them as its first two arguments.
    let top = stack.as_mut_ptr() as usize + THREAD_STACK_BYTES;
    let write_ptr = (top - 16) as *mut u64;
    unsafe {
        *write_ptr = func as *const () as u64;
        *write_ptr.add(1) = arg;
    }

    let user_stack_top = (top - 16) as u64;
    let trampoline = thread_trampoline as *const () as u64;

    let tid: u64;
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") 22u64,
            in("rdi") trampoline,
            in("rsi") user_stack_top,
            in("rdx") 0u64,
            lateout("rax") tid,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    (tid, stack)
}

// ---------------------------------------------------------------------------
// Lightweight spin barrier
// ---------------------------------------------------------------------------

/// All N participants call `arrive_and_wait(N)` before proceeding.
/// Uses a single global atomic counter; safe for a one-shot barrier.
pub static BARRIER: AtomicU64 = AtomicU64::new(0);

/// Reset barrier for the next multi-core run (call from main thread before spawning).
pub fn barrier_reset() {
    BARRIER.store(0, Ordering::SeqCst);
}

/// Increment the arrival count and spin until all `n` threads have arrived.
pub fn arrive_and_wait(n: u64) {
    BARRIER.fetch_add(1, Ordering::SeqCst);
    while BARRIER.load(Ordering::SeqCst) < n {
        core::hint::spin_loop();
    }
}
