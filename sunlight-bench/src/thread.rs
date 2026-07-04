//! Userspace thread spawn using SunlightOS ThreadSpawn syscall (22).
//!
//! ABI (from kernel/src/arch/x86_64/syscall.rs):
//!   rdi = trampoline fn pointer
//!   rsi = user_stack_top — kernel reads [rsi+0]=func, [rsi+8]=arg
//!   rdx = FS_BASE (TLS; 0 = none)
//!
//! Kernel then starts the new thread with:
//!   RIP = trampoline, RSP = user_stack_top, RDI = func, RSI = arg

use core::arch::asm;
use core::sync::atomic::{AtomicU64, Ordering};

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
    // The kernel still reclaims shared address-space mappings on ProcessExit,
    // so benchmark workers must not exit through that path. After recording
    // completion through shared flags (THREAD_END[slot] etc., set by the
    // worker before returning) the trampoline parks via [`park_forever`].
    park_forever();
}

/// Permanently park a finished benchmark worker thread by transitioning to
/// `BlockedOnIpc`.
///
/// # Why this exists (12-core VMware regression fix)
///
/// The previous park implementation was a tight `loop { process_yield(); }`
/// busy-yield. `process_yield` only marks the task `Ready` and requests a
/// reschedule; it does not block. Every timer tick re-dispatched the parked
/// worker on its core, so:
///
///   1. The worker's vCPU stayed 100% busy forever after its workload
///      finished, forcing VMware ESXi's relaxed vCPU co-scheduling back into
///      strict co-scheduling — which scales badly past ~8 vCPUs and is the
///      primary cause of the 12-core regression (overall and multi raw
///      regressing vs 8-core, plus the high spread / "noisy" stability class).
///   2. The parked worker never left its core's run queue, so the queue
///      stayed non-empty. Other idle cores' `pick_next` always had local work
///      and never fell through to `steal_work`, which meant fresh workers
///      spawned by the next multi-core stage (enqueued on the spawner's core)
///      were slow to be redistributed across the system.
///
/// IpcNotifyWait (SunlightOS syscall 6) sets the calling task's state to
/// `BlockedOnIpc` and returns `WouldBlock` without rescheduling. Reissuing it
/// keeps the task blocked, so the next LAPIC timer tick on its core sees a
/// non-Running current task, [`Scheduler::schedule_tick`] removes it from the
/// ready queue, [`Scheduler::pick_next`] finds the local queue empty, calls
/// [`Scheduler::steal_work`], and — if nothing remains to steal — switches to
/// the per-core HLT idle context (`core_idle_entry`: `sti; hlt`). That HLT
/// releases the vCPU from VMware co-scheduling pressure, and the now-empty
/// local run queue means subsequent multi-stage worker spawns can be stolen
/// away from the spawner's core by idle APs.
///
/// The park never returns unless an external sender wakes the (unused)
/// endpoint; benchmark workers do not, so this is effectively permanent
/// until the benchmark process exits and the kernel reaps its address space
/// and child threads. The fixed stack pool prevents stack corruption across
/// runs even though these blocked threads persist.
fn park_forever() -> ! {
    loop {
        // SAFETY: a bare `syscall` with rax=6 (IpcNotifyWait) and rdi=0 (the
        // kernel handler ignores the endpoint token). The kernel sets state =
        // BlockedOnIpc, requests a reschedule on this core, and returns
        // WouldBlock in rax. We do not need to read the return value because
        // we only want to re-enter this syscall until the timer path
        // deschedules us. Syscall/`sysretq` clobbers rcx and r11 (the latter
        // holds the restored RFLAGS), so we annotate those as lateout.
        unsafe {
            asm!(
                "syscall",
                in("rax") 6u64, // SunlightSyscall::IpcNotifyWait
                in("rdi") 0u64, // endpoint token (kernel ignores it)
                lateout("rax") _,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack),
            );
        }
    }
}

/// Spawn a thread running `func(arg)`. Returns the new thread ID, or 0 on error.
///
/// Benchmark threads never exit cleanly, so stacks are drawn from a fixed
/// process-lifetime pool instead of the bump heap. That keeps repeated runs
/// from corrupting live stack memory when the benchmark heap is reset.
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
    tid
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
pub fn arrive_and_wait(n: u64) {
    BARRIER.fetch_add(1, Ordering::SeqCst);
    let mut spins = 0u32;
    while BARRIER.load(Ordering::SeqCst) < n {
        if spins < SPIN_BEFORE_YIELD {
            core::hint::spin_loop();
            spins = spins.saturating_add(1);
        } else {
            sunlight_ipc::process_yield();
            spins = 0;
        }
    }
}
