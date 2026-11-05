//! Native threading support for SunlightOS.
//!
//! Each thread gets its own stack (2 MiB mmap'd) and its own TLS block so
//! `errno` is per-thread.  The kernel creates the execution context via
//! `SYS_THREAD_SPAWN` (22) and schedules it immediately.
//!
//! # Current limitations
//! - The alloc-only API does not implement join. The separate native std
//!   runtime waits for kernel quiescence and reclaims these handle resources.
//! - Thread exit re-uses `SYS_PROCESS_EXIT`; MM-0 makes the kernel distinguish
//!   address-space owners from borrower threads, so a borrower exits alone.
//!   Alloc-only handles retain their mappings until explicit cleanup or VM exit.
//! - The spawned thread inherits the parent's open file-descriptor table
//!   (kernel-side shallow copy); mutations in one thread are not visible to
//!   the other without Arc-based sharing.

use crate::mman::{mmap, munmap, MAP_ANONYMOUS, MAP_PRIVATE, PROT_READ, PROT_WRITE};
use crate::sys::{check, Errno, SYS_PROCESS_EXIT};

const SYS_THREAD_SPAWN: u64 = 22;
const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024; // 2 MiB

/// TCB layout that mirrors `tls::Tcb` — duplicated here so thread.rs is
/// self-contained; offset 8 = errno (see errno::__errno_location).
#[repr(C)]
struct Tcb {
    self_ptr: *mut Tcb,
    errno: i32,
}

/// Handle returned by `spawn`.  Carries bookkeeping for future cleanup /
/// join support.  Does NOT automatically free the stack on drop (Phase 1).
pub struct JoinHandle {
    /// Kernel thread ID (unique PID slot in the scheduler).
    pub tid: u32,
    /// Base of the mmap'd stack region (lowest address).
    pub stack_base: *mut u8,
    pub stack_size: usize,
    /// Base of the mmap'd TLS page.
    pub tls_base: *mut u8,
}

/// Trampoline that the kernel's iretq lands on.
///
/// The kernel sets RDI = `func` and RSI = `arg` via `set_initial_args` before
/// scheduling the thread, so this receives both as normal C arguments.
#[unsafe(naked)]
extern "C" fn thread_trampoline(_func: extern "C" fn(*mut u8) -> *mut u8, _arg: *mut u8) -> ! {
    // Native ThreadSpawn enters via IRET, without a call's return-address push.
    // Establish the System V call-site alignment before entering Rust/C code.
    core::arch::naked_asm!(
        "and rsp, -16",
        "call {start}",
        "ud2",
        start = sym thread_start,
    );
}

extern "C" fn thread_start(func: extern "C" fn(*mut u8) -> *mut u8, arg: *mut u8) -> ! {
    let _ret = func(arg);
    // MM-0: ProcessExit terminates this borrower without reclaiming its
    // owner's shared address space.
    unsafe { crate::sys::syscall1(SYS_PROCESS_EXIT, 0) };
    loop {}
}

/// Spawn a new native thread that calls `entry(arg)`.
///
/// # Safety
/// `arg` must remain valid for the lifetime of the new thread.
pub unsafe fn spawn(
    entry: extern "C" fn(*mut u8) -> *mut u8,
    arg: *mut u8,
) -> Result<JoinHandle, Errno> {
    spawn_with_stack(entry, arg, DEFAULT_STACK_SIZE, false)
}

/// Spawn with an explicit stack and opt-in process-wide descriptors for std.
/// # Safety
/// Same lifetime contract as `spawn`; stack size is rounded to whole pages.
pub unsafe fn spawn_with_stack(
    entry: extern "C" fn(*mut u8) -> *mut u8,
    arg: *mut u8,
    stack_size: usize,
    shared_descriptors: bool,
) -> Result<JoinHandle, Errno> {
    let stack_size = stack_size.checked_add(4095).ok_or(Errno::TooBig)? & !4095;
    if stack_size < 4096 {
        return Err(Errno::Inval);
    }
    // ── 1. Allocate the thread stack ─────────────────────────────────────────
    let stack_ptr = mmap(
        core::ptr::null_mut(),
        stack_size,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )?;

    // ── 2. Allocate the TLS block; free the stack on failure ─────────────────
    let tls_ptr = mmap(
        core::ptr::null_mut(),
        4096,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )
    .map_err(|e| {
        let _ = munmap(stack_ptr, stack_size);
        e
    })?;

    // ── 3. Initialize the TCB (self-pointer + zero errno) ────────────────────
    let tcb = tls_ptr as *mut Tcb;
    (*tcb).self_ptr = tcb;
    (*tcb).errno = 0;

    // ── 4. Write entry + arg at the top of the stack ─────────────────────────
    // The kernel reads them from here to set up RDI/RSI in the initial context.
    let stack_top = stack_ptr as u64 + stack_size as u64;
    let aligned_top = (stack_top - 32) & !15u64;
    let stack_args = aligned_top as *mut u64;
    *stack_args = entry as u64;
    *stack_args.add(1) = arg as u64;

    // ── 5. Ask the kernel to create the thread context ───────────────────────
    //   rdi = trampoline entry point (kernel sets RIP here via iretq)
    //   rsi = aligned stack top      (kernel sets RSP)
    //   rdx = TLS pointer            (kernel writes to FS_BASE MSR)
    let ret = crate::sys::syscall4(
        if shared_descriptors {
            158
        } else {
            SYS_THREAD_SPAWN
        },
        thread_trampoline as *const () as u64,
        aligned_top,
        tls_ptr as u64,
        u64::from(shared_descriptors),
    );

    match check(ret) {
        Ok(tid) => Ok(JoinHandle {
            tid: tid as u32,
            stack_base: stack_ptr,
            stack_size,
            tls_base: tls_ptr,
        }),
        Err(e) => {
            let _ = munmap(stack_ptr, stack_size);
            let _ = munmap(tls_ptr, 4096);
            Err(e)
        }
    }
}
