//! Userspace thread spawn using SunlightOS ThreadSpawn syscall (22).
//!
//! ABI (from kernel/src/arch/x86_64/syscall.rs):
//!   rdi = trampoline fn pointer
//!   rsi = user_stack_top — kernel reads [rsi+0]=func, [rsi+8]=arg
//!   rdx = FS_BASE (TLS; 0 = none)
//!
//! Kernel then starts the new thread with:
//!   RIP = trampoline, RSP = user_stack_top, RDI = func, RSI = arg

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use sunlight_ipc::ProcessExit;

pub const THREAD_STACK_BYTES: usize = 32 * 1024; // 32 KiB per thread stack
pub const MAX_THREAD_STACKS: usize = 320;

static mut THREAD_STACK_POOL: [[u8; THREAD_STACK_BYTES]; MAX_THREAD_STACKS] =
    [[0u8; THREAD_STACK_BYTES]; MAX_THREAD_STACKS];
static NEXT_STACK_SLOT: AtomicU64 = AtomicU64::new(0);

/// The trampoline receives func in RDI and arg in RSI (set by the kernel via
/// `set_initial_args`), then calls the actual benchmark worker.
unsafe extern "C" fn thread_trampoline(func: u64, arg: u64) -> ! {
    let f: unsafe extern "C" fn(u64) = core::mem::transmute(func as *const ());
    f(arg);
    // MM-0 makes ProcessExit ownership-aware: this borrower becomes Finished
    // and Reaped without reclaiming the owner's shared user address space.
    ProcessExit::exit(0);
}

/// Spawn a thread running `func(arg)`. Returns the new thread ID, or 0 on error.
///
/// Stacks are drawn from a fixed process-lifetime pool because real `munmap`
/// is deferred. Slots are deliberately not reused: logical completion is not
/// a kernel join, so a unique stack prevents reuse before the old borrower is
/// definitively non-runnable and Reaped. The configured benchmark consumes at
/// most 144 of the 320 slots (4.5 MiB of the 10 MiB static pool).
pub fn spawn(func: unsafe extern "C" fn(u64), arg: u64) -> u64 {
    let slot = NEXT_STACK_SLOT.fetch_add(1, Ordering::SeqCst) as usize;
    if slot >= MAX_THREAD_STACKS {
        return 0;
    }

    let stack = unsafe { &mut THREAD_STACK_POOL[slot] };

    // Write [func, arg] at the TOP of the stack (highest addresses).
    // The kernel reads them from [rsi+0] and [rsi+8], then sets
    // RDI=func and RSI=arg on the new thread, so the trampoline receives
    // them as its first two arguments.
    //
    // A Vec<u8> is only 1-byte aligned, so round the top down to a 16-byte
    // boundary (System V ABI requires a 16-aligned RSP at function entry, and
    // the kernel reads these slots as aligned u64s). An unaligned stack top
    // makes the kernel's ThreadSpawn deref a misaligned *const u64.
    let top = (stack.as_mut_ptr() as usize + THREAD_STACK_BYTES) & !0xF;
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
    if tid == u64::MAX {
        0
    } else {
        tid
    }
}

// ---------------------------------------------------------------------------
// Lightweight spin barrier
// ---------------------------------------------------------------------------

/// All N participants call `arrive_and_wait(N)` before proceeding.
/// Uses a single global atomic counter; safe for a one-shot barrier.
pub static BARRIER: AtomicU64 = AtomicU64::new(0);

/// Number of `core::hint::spin_loop()` (PAUSE) iterations to spin on the
/// shared barrier counter before issuing a `process_yield()`.
///
/// Tuned so that:
///   * The fast path (all workers arrive nearly together) finishes entirely
///     in pure PAUSE spin with no syscall overhead.
///   * The slow path — where some workers are stuck queued behind a busy
///     spawner core — yields between spin bursts. Because `process_yield`
///     flips the task to `Ready` and requests a reschedule on its core, the
///     next LAPIC timer tick re-enqueues the yielding worker and rotates the
///     next queued worker forward, accelerating barrier arrival on the
///     contested core. Without this rotation the spawner core waits one full
///     scheduler quantum (~90 ms with `nice=-10`) per queued worker before
///     pulling the next one, which dominates the 12-core multi-stage
///     bootstrapping time and inflates the run-to-run spread.
const SPIN_BEFORE_YIELD: u32 = 1024;

/// Reset barrier for the next multi-core run (call from main thread before spawning).
pub fn barrier_reset() {
    BARRIER.store(0, Ordering::SeqCst);
}

/// Increment the arrival count and spin until all `n` threads have arrived.
///
/// Spins `SPIN_BEFORE_YIELD` PAUSE iterations, then yields once, then spins
/// again, repeating until the counter reaches `n`. The yield is a perf hint,
/// not a correctness change — `BARRIER` is monotonic per stage so the loop is
/// guaranteed to terminate once all participants have arrived.
pub fn arrive_and_wait(n: u64, aborted: &AtomicBool) -> bool {
    BARRIER.fetch_add(1, Ordering::Release);
    let mut spins = 0u32;
    while BARRIER.load(Ordering::Acquire) < n {
        if aborted.load(Ordering::Acquire) {
            return false;
        }
        if spins < SPIN_BEFORE_YIELD {
            core::hint::spin_loop();
            spins = spins.saturating_add(1);
        } else {
            sunlight_ipc::process_yield();
            spins = 0;
        }
    }
    true
}
