//! Limine Multi-Processor (SMP) bring-up — Phase 0.
//!
//! ## What this phase does
//!
//! 1. Declares the Limine `MpRequest` so the bootloader enumerates all CPUs
//!    and provides their LAPIC IDs and `MpInfo` pointers.
//! 2. Iterates the response in `start_aps()` and calls `MpInfo::bootstrap()`
//!    for each non-BSP CPU, giving each one a dedicated kernel stack and the
//!    `ap_entry` function pointer.
//! 3. Each AP then:
//!    - Switches to its own 64 KiB kernel stack (replaces Limine's 16 KiB temporary stack)
//!    - Enables x87/SSE (`cpu::init_cpu_features`)
//!    - Loads the shared GDT + IDT (`interrupts::ap_load_gdt_and_idt`)
//!    - Initialises its SYSCALL/SYSRET MSRs (`syscall::setup_syscall_msrs`)
//!    - Increments `AP_ONLINE` and parks in `hlt` with interrupts disabled
//!
//! ## Phase 0 limitations (to be lifted in later phases)
//!
//! - **No LAPIC init**: the legacy 8259 PIC is wired to the BSP only.
//!   APs receive no hardware interrupts and will not be scheduled until
//!   per-core LAPIC timers are set up (phase 1 SMP).
//! - **No per-AP TSS**: the BSP's TSS is already "busy" (the processor sets
//!   the Busy bit in the GDT descriptor on `load_tss`).  Loading it a second
//!   time from an AP causes #GP.  Per-AP TSSes, GDTs, and IST stacks are
//!   deferred to phase 1.
//! - **Single global scheduler**: `SCHEDULER` is a spinlock-guarded single
//!   instance.  APs are not dispatching tasks yet.
//!
//! ## Thread-safety invariants
//!
//! - `AP_ONLINE` is `AtomicU32`; safe for concurrent access.
//! - `AP_STACKS` is accessed at index `ap_idx` only by the BSP (which writes
//!   `stack_top`) and by the corresponding AP (which uses it as its RSP).
//!   These accesses are disjoint by construction and sequenced by the
//!   `AP_ONLINE` acquire/release fence.
//! - The shared IDT and GDT are read-only after `interrupts::init()` returns.
//!   Multiple concurrent `lidt`/`lgdt` calls are safe: these instructions
//!   only write to CPU-internal descriptor-table registers, not to memory.

use core::arch::naked_asm;
use core::sync::atomic::{AtomicU32, Ordering};

use x86_64::VirtAddr;

use crate::arch::x86_64::{cpu, interrupts, syscall};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum number of Application Processors (total logical cores − 1 BSP).
/// Increase for large NUMA machines; the only cost is static BSS.
pub const MAX_APS: usize = 63;

/// Per-AP kernel stack size (64 KiB).
///
/// Limine provides a temporary 16 KiB stack before calling `goto_address`.
/// `ap_entry` replaces RSP immediately, so the temporary stack is only used
/// for the naked-function prologue (a single `mov` + `call`).
const AP_STACK_SIZE: usize = 64 * 1024;

// ─── Global state ─────────────────────────────────────────────────────────────

/// Count of Application Processors that have completed phase-0 init.
///
/// Written by each AP with `Release`, read by the BSP with `Acquire`.
/// The BSP uses this as a sequencing barrier between `bootstrap()` calls.
pub static AP_ONLINE: AtomicU32 = AtomicU32::new(0);

/// Per-AP kernel stacks.
///
/// `AP_STACKS[i]` is the 64 KiB stack for the *i*-th non-BSP CPU started
/// by `start_aps`.  Zero-initialized (.bss), so they add no weight to the
/// ELF binary (63 × 64 KiB = 4 MiB of BSS).
///
/// `repr(align(16))` ensures the stack top address is always 16-byte aligned
/// per the System V AMD64 ABI requirement before a `call` instruction.
#[repr(C, align(16))]
struct ApStack([u8; AP_STACK_SIZE]);

static mut AP_STACKS: [ApStack; MAX_APS] =
    [const { ApStack([0u8; AP_STACK_SIZE]) }; MAX_APS];

// ─── Public API ───────────────────────────────────────────────────────────────

/// Bring all non-BSP CPUs online.
///
/// Iterates the Limine MP response, skips the BSP (identified by
/// `bsp_lapic_id`), and calls `MpInfo::bootstrap(ap_entry, stack_top)` for
/// each AP.  After each `bootstrap()`, the BSP spins on `AP_ONLINE` until
/// the AP signals ready, ensuring stacks are not aliased.
///
/// # Safety
///
/// Must be called after:
/// - `interrupts::init()` has finished (shared GDT + IDT are live)
/// - `syscall::setup_syscall_msrs()` has been called on the BSP
///   (establishes `syscall_entry` address for AP MSR init)
///
/// Must be called from the BSP, with interrupts still disabled.
pub fn start_aps(all_cpus: &[&limine::mp::MpInfo], bsp_lapic_id: u32) {
    let mut ap_idx = 0usize;

    for cpu_info in all_cpus {
        if cpu_info.lapic_id == bsp_lapic_id {
            // Skip the Bootstrap Processor; it is already running.
            continue;
        }

        if ap_idx >= MAX_APS {
            crate::serial_println!(
                "[SMP] Warning: more than {} APs detected; ignoring remaining cores",
                MAX_APS
            );
            break;
        }

        // Compute the top of this AP's dedicated kernel stack.
        // Stacks grow downward; `AP_STACKS[ap_idx].0` is the bottom byte,
        // so the initial RSP = base + AP_STACK_SIZE.
        // Safety: `AP_STACKS` is written only once per index (here, on BSP)
        // before the AP that will use it is started.
        let stack_top: u64 = unsafe {
            AP_STACKS[ap_idx].0.as_mut_ptr().add(AP_STACK_SIZE) as u64
        };

        crate::serial_println!(
            "[SMP] Starting AP{}  proc_id={}  lapic_id={}  stack={:#x}..{:#x}",
            ap_idx,
            cpu_info.processor_id,
            cpu_info.lapic_id,
            stack_top - AP_STACK_SIZE as u64,
            stack_top,
        );

        // Write `extra_argument` (Relaxed) then `goto_addr` (Release).
        // The Release fence on `goto_addr` ensures the AP sees `stack_top`
        // before it begins executing `ap_entry`.
        cpu_info.bootstrap(ap_entry, stack_top);

        // Wait for this AP to finish init before we start the next.
        // The Acquire load pairs with the Release fetch_add in `ap_entry_rust`.
        let expected = (ap_idx as u32) + 1;
        loop {
            if AP_ONLINE.load(Ordering::Acquire) >= expected {
                break;
            }
            core::hint::spin_loop();
        }

        ap_idx += 1;
    }

    crate::serial_println!(
        "[SMP] Bring-up complete: {} AP(s) online — {} total logical processor(s)",
        ap_idx,
        ap_idx + 1, // +1 for BSP
    );
}

// ─── AP entry point ───────────────────────────────────────────────────────────

/// Naked entry point handed to Limine via `MpInfo::bootstrap`.
///
/// Limine calls this function with:
/// - `rdi` = pointer to `MpInfo`
/// - `rsp` = temporary 16 KiB stack allocated by the bootloader
///
/// `MpInfo` field layout (x86_64, limine 0.6):
/// ```text
/// offset  0: processor_id:   u32
/// offset  4: lapic_id:        u32
/// offset  8: _resvd0:         u64
/// offset 16: goto_addr:       AtomicPtr<()>  [8 bytes]
/// offset 24: extra_argument:  AtomicU64      [8 bytes]  ← stack top
/// ```
///
/// The function immediately loads `extra_argument` (the kernel stack top
/// written by `start_aps`) into RSP, then tail-calls `ap_entry_rust` with
/// `rdi` (= `&MpInfo`) unchanged as the first argument.
#[unsafe(naked)]
unsafe extern "C" fn ap_entry(info: &limine::mp::MpInfo) -> ! {
    naked_asm!(
        // Load the pre-allocated kernel stack top from MpInfo.extra_argument
        // (offset 24). This replaces the bootloader's temporary 16 KiB stack.
        "mov rsp, qword ptr [rdi + 24]",
        // Enforce 16-byte RSP alignment required by the System V AMD64 ABI
        // before any `call` instruction (the compiler relies on this).
        "and rsp, -16",
        // Null out RBP so stack-unwinding tools see a clean sentinel frame.
        "xor ebp, ebp",
        // Tail-call into the real Rust init body.
        // rdi is unchanged so &MpInfo is still the first argument.
        "call {entry}",
        // ap_entry_rust diverges (-> !); this ud2 is unreachable but
        // satisfies the assembler and produces an obvious trap if somehow reached.
        "ud2",
        entry = sym ap_entry_rust,
    );
}

/// Non-naked AP initialization body, called from `ap_entry` on the AP's own
/// kernel stack with interrupts disabled (IF=0 from reset; Limine leaves IF=0).
///
/// Execution order mirrors the BSP's early boot sequence, omitting the
/// PIC/PIT setup which is done once on the BSP.
unsafe extern "C" fn ap_entry_rust(info: &limine::mp::MpInfo) -> ! {
    // ── Step 1: CPU feature initialisation ───────────────────────────────────
    // Enable x87/SSE/FXSAVE so compiler-emitted XMM instructions in kernel
    // code do not raise #UD or #NM on this core.  CR0/CR4 and MSRs written
    // here are per-core registers — each AP must initialise them independently.
    cpu::init_cpu_features();

    // ── Step 2: Load shared GDT + IDT ────────────────────────────────────────
    // The BSP's GDT and IDT are in read-only globals after `interrupts::init`.
    // All APs share them; `lgdt`/`lidt` are non-destructive (they only update
    // the CPU's own GDTR/IDTR register from the pointer in memory).
    //
    // TSS loading is intentionally skipped in phase 0 (see module doc).
    interrupts::ap_load_gdt_and_idt();

    // ── Step 3: SYSCALL/SYSRET MSRs ──────────────────────────────────────────
    // STAR (CS/SS selectors), LSTAR (entry point), and FMASK (RFLAGS mask)
    // are per-core MSRs.  Each logical CPU must program them before it can
    // accept syscalls from ring-3 code.
    unsafe {
        syscall::setup_syscall_msrs(VirtAddr::new(
            syscall::syscall_entry as *const () as u64,
        ));
    }

    // ── Step 4: Signal the BSP ───────────────────────────────────────────────
    // Release ordering: ensures all prior writes (CR0/CR4, MSRs, segment regs)
    // are globally visible before the BSP observes this counter change.
    let seq = AP_ONLINE.fetch_add(1, Ordering::Release) + 1;
    crate::serial_println!(
        "[SMP] AP{} (LAPIC {}) online — parked (awaiting LAPIC timer, phase 1)",
        seq,
        info.lapic_id,
    );

    // ── Step 5: Park in a halting idle loop ───────────────────────────────────
    // Interrupts remain disabled (IF=0).  The 8259 PIC is wired to the BSP
    // only, so no external interrupt will arrive here until a per-core LAPIC
    // is configured.  The BSP will send a Reschedule IPI to wake APs once
    // per-AP scheduler queues and LAPIC timers are set up (phase 1).
    loop {
        x86_64::instructions::hlt();
    }
}
