//! Limine Multi-Processor (SMP) bring-up — Phase 1.
//!
//! ## Phase 1 additions vs Phase 0
//!
//! Phase 0 brought APs online with x87/SSE, shared GDT+IDT, and SYSCALL MSRs,
//! then parked them in `hlt` with interrupts disabled.  Phase 1 completes the
//! AP initialisation so each core can receive timer interrupts and participate
//! in work-stealing scheduling:
//!
//! 1. **Per-AP TSS** — each AP gets its own `TaskStateSegment` with RSP0
//!    pointing to the top of its kernel stack.  RSP0 is updated by the
//!    scheduler on every context switch so interrupt delivery from ring-3
//!    lands on the correct per-core kernel stack.
//!
//! 2. **Per-AP GDT** — because the BSP's TSS descriptor has its Busy bit set
//!    by the processor (`ltr` makes a TSS busy), loading the same selector on
//!    an AP causes #GP.  Each AP gets an identical GDT to the BSP's except with
//!    a freshly constructed (not-busy) TSS descriptor pointing to its own TSS.
//!
//! 3. **LAPIC timer** — each AP initialises its local APIC (SVR enable) and
//!    arms a periodic timer at vector 0x20 (~100 Hz).  The shared `timer_entry`
//!    ISR handles the interrupt; `timer_rust` forwards it to `schedule_tick`
//!    for the correct core.
//!
//! 4. **Interrupt enable + idle loop** — after full per-core init, APs enable
//!    interrupts (STI) and enter a `hlt` idle loop.  Timer ticks preempt them
//!    and the scheduler can migrate tasks onto them.
//!
//! ## Thread-safety invariants
//!
//! - The BSP starts APs one at a time and waits for `AP_ONLINE` to advance
//!   before starting the next AP.  Therefore only one AP runs `ap_entry_rust`
//!   at a time, and indexing `AP_STACKS`, `AP_TSS_STORE`, and `AP_GDTS` by
//!   `AP_ONLINE.load()` (before incrementing) is race-free.
//! - `AP_STACKS`, `AP_TSS_STORE`, and `AP_GDTS` are written once by the
//!   respective AP and thereafter read-only (except that `AP_TSS_STORE[i]`
//!   RSP0 is updated atomically by the scheduler on every context switch on
//!   that core).
//! - `AP_ONLINE` is `AtomicU32`; the Release fetch_add pairs with the BSP's
//!   Acquire load in the spin-wait loop.

use core::arch::naked_asm;
use core::sync::atomic::{AtomicU32, Ordering};

use x86_64::VirtAddr;
use x86_64::structures::gdt::SegmentSelector;
use x86_64::structures::tss::TaskStateSegment;

use crate::arch::x86_64::{cpu, interrupts, lapic, syscall};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Maximum number of Application Processors (total logical cores − 1 BSP).
pub const MAX_APS: usize = 63;

/// Per-AP kernel stack size (64 KiB).
const AP_STACK_SIZE: usize = 64 * 1024;

// ─── Known GDT entry values (x86_64 crate 0.15 + kernel custom segments) ─────
//
// These match the selectors built by `interrupts::init()`:
//   index 0 → null
//   index 1 → kernel code  (selector 0x08)
//   index 2 → kernel data  (selector 0x10)
//   index 3 → user code compat (selector 0x18)
//   index 4 → user data    (selector 0x23)
//   index 5 → user code 64 (selector 0x2B)
//   index 6 → TSS low      (selector 0x30)
//   index 7 → TSS high
//
// Verified against x86_64 0.15 crate assert: KERNEL_CODE64 = 0x00af9b000000ffff.

const GDT_NULL:            u64 = 0x0000_0000_0000_0000;
const GDT_KERNEL_CODE:     u64 = 0x00af_9b00_0000_ffff; // kernel_code_segment()
const GDT_KERNEL_DATA:     u64 = 0x00cf_9300_0000_ffff; // kernel_data_segment()
const GDT_USER_CODE_COMPAT:u64 = 0x00AF_FA00_0000_FFFF; // user_code_segment() (compat)
const GDT_USER_DATA:       u64 = 0x00AF_F200_0000_FFFF; // user_data_segment()
const GDT_USER_CODE64:     u64 = 0x00AF_FA00_0000_FFFF; // user_code_segment() (64-bit)

/// TSS selector: index 6, RPL 0 → 6 × 8 = 0x30.
const AP_TSS_SELECTOR: u16 = 0x30;

// ─── Static storage ───────────────────────────────────────────────────────────

/// Count of APs that have completed phase-1 init (LAPIC timer armed).
pub static AP_ONLINE: AtomicU32 = AtomicU32::new(0);

/// Per-AP kernel stacks (zero-init BSS; 63 × 64 KiB = 4 MiB).
#[repr(C, align(16))]
struct ApStack([u8; AP_STACK_SIZE]);

static mut AP_STACKS: [ApStack; MAX_APS] = [const { ApStack([0u8; AP_STACK_SIZE]) }; MAX_APS];

/// Per-AP Task State Segments.
///
/// `AP_TSS_STORE[i]` is the TSS for the (i+1)-th logical CPU (i.e. AP #i).
/// Core 0 = BSP uses the global `TSS` in interrupts.rs.
/// Core k (k ≥ 1) uses `AP_TSS_STORE[k-1]`.
///
/// RSP0 is updated by `set_current_cpu_tss_rsp0` on every context switch so
/// that the next interrupt from ring-3 on that core delivers to the correct
/// kernel stack.
static mut AP_TSS_STORE: [TaskStateSegment; MAX_APS] =
    [const { TaskStateSegment::new() }; MAX_APS];

/// Per-AP raw GDT storage: 8 × u64 = 64 bytes per AP.
///
/// Each AP builds its own GDT here (identical to the BSP's except for the
/// TSS descriptor which points to that AP's `AP_TSS_STORE` entry).
/// The GDT must remain valid as long as the AP is running — storing it in
/// `.bss` satisfies this.
#[repr(C, align(8))]
struct ApGdt([u64; 8]);

static mut AP_GDTS: [ApGdt; MAX_APS] = [const { ApGdt([0u64; 8]) }; MAX_APS];

// ─── Public API ───────────────────────────────────────────────────────────────

/// Bring all non-BSP CPUs through Phase 1 initialisation.
///
/// BSP call order (must match main.rs):
/// 1. `interrupts::init()` — shared GDT, IDT, BSP LAPIC timer armed.
/// 2. `syscall::setup_syscall_msrs()` — BSP SYSCALL/SYSRET MSRs.
/// 3. `smp::start_aps()` (this function) — each AP gets its own TSS, GDT,
///    LAPIC, and is left running in an interrupt-enabled idle loop.
pub fn start_aps(all_cpus: &[&limine::mp::MpInfo], bsp_lapic_id: u32) {
    let mut ap_idx = 0usize;

    for cpu_info in all_cpus {
        if cpu_info.lapic_id == bsp_lapic_id {
            continue;
        }

        if ap_idx >= MAX_APS {
            crate::serial_println!(
                "[SMP] Warning: more than {} APs; ignoring remaining cores",
                MAX_APS
            );
            break;
        }

        let stack_top: u64 =
            unsafe { AP_STACKS[ap_idx].0.as_mut_ptr().add(AP_STACK_SIZE) as u64 };

        crate::serial_println!(
            "[SMP] Starting AP{}  proc_id={}  lapic_id={}  stack={:#x}..{:#x}",
            ap_idx,
            cpu_info.processor_id,
            cpu_info.lapic_id,
            stack_top - AP_STACK_SIZE as u64,
            stack_top,
        );

        cpu_info.bootstrap(ap_entry, stack_top);

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
        "[SMP] Phase-1 bring-up complete: {} AP(s) online ({} total logical CPUs)",
        ap_idx,
        ap_idx + 1,
    );
}

/// Update RSP0 in the TSS belonging to the currently-executing CPU.
///
/// Called by the scheduler on every context switch so that the NEXT interrupt
/// from ring-3 on this core delivers to the new task's kernel stack.
///
/// - Core 0 (BSP): updates the global `TSS` in interrupts.rs.
/// - Core k (AP):  updates `AP_TSS_STORE[k-1]`.
pub fn set_current_cpu_tss_rsp0(stack_top: u64) {
    let cpu_id = crate::sched::current_cpu_id();
    if cpu_id == 0 {
        crate::arch::x86_64::interrupts::set_tss_rsp0(stack_top);
    } else {
        let ap_idx = cpu_id - 1;
        if ap_idx < MAX_APS {
            unsafe {
                AP_TSS_STORE[ap_idx].privilege_stack_table[0] = VirtAddr::new(stack_top);
            }
        }
    }
}

// ─── AP entry point ───────────────────────────────────────────────────────────

/// Naked entry point given to Limine via `MpInfo::bootstrap`.
///
/// Limine calls this with rdi = &MpInfo, rsp = temporary 16 KiB stack.
/// `extra_argument` (offset 24) holds the kernel stack top written by
/// `start_aps`.  We immediately switch RSP and tail-call `ap_entry_rust`.
#[unsafe(naked)]
unsafe extern "C" fn ap_entry(info: &limine::mp::MpInfo) -> ! {
    naked_asm!(
        "mov rsp, qword ptr [rdi + 24]",
        "and rsp, -16",
        "xor ebp, ebp",
        "call {entry}",
        "ud2",
        entry = sym ap_entry_rust,
    );
}

/// Compute the 16-byte (two u64) 64-bit TSS descriptor for a given TSS.
///
/// Layout (per Intel SDM Vol. 3A §3.5.2):
/// ```text
/// lo[15:0]  = limit[15:0]
/// lo[39:16] = base[23:0]
/// lo[47:40] = type/access = 0x89 (Present, Available 64-bit TSS)
/// lo[51:48] = limit[19:16] = 0   (limit < 64 KiB so high nibble is 0)
/// lo[55:52] = flags = 0
/// lo[63:56] = base[31:24]
/// hi[31:0]  = base[63:32]
/// hi[63:32] = 0 (reserved)
/// ```
fn build_tss_descriptor(tss: *const TaskStateSegment) -> (u64, u64) {
    let base = tss as u64;
    let limit = (core::mem::size_of::<TaskStateSegment>() - 1) as u64;

    let lo = (limit & 0xFFFF)                      // bits 15:0 — limit low
        | ((base & 0x00FF_FFFF) << 16)              // bits 39:16 — base [23:0]
        | (0x89u64 << 40)                            // bits 47:40 — Present + type 0x9
        | (((limit >> 16) & 0xF) << 48)             // bits 51:48 — limit high nibble
        | (((base >> 24) & 0xFF) << 56);             // bits 63:56 — base [31:24]

    let hi = base >> 32; // bits 31:0 — base [63:32]

    (lo, hi)
}

/// Fill `AP_GDTS[ap_idx]` with the kernel GDT entries and a fresh TSS
/// descriptor for `AP_TSS_STORE[ap_idx]`, then load it and the IDT on
/// the calling AP, and execute `ltr` to load the AP's TSS.
///
/// # Safety
/// Must be called once per AP during its startup, with interrupts disabled.
unsafe fn ap_setup_gdt_and_tss(ap_idx: usize) {
    // Initialise TSS: RSP0 and IST[0] both point to the top of the AP's
    // kernel stack.  IST[0] is used only by the double-fault handler.
    let stack_top = AP_STACKS[ap_idx].0.as_ptr().add(AP_STACK_SIZE) as u64;
    let tss = &mut AP_TSS_STORE[ap_idx];
    tss.privilege_stack_table[0] = VirtAddr::new(stack_top);
    tss.interrupt_stack_table[0] = VirtAddr::new(stack_top);

    // Build the raw GDT.
    let (tss_lo, tss_hi) = build_tss_descriptor(tss as *const _);
    let gdt = &mut AP_GDTS[ap_idx];
    gdt.0[0] = GDT_NULL;
    gdt.0[1] = GDT_KERNEL_CODE;
    gdt.0[2] = GDT_KERNEL_DATA;
    gdt.0[3] = GDT_USER_CODE_COMPAT;
    gdt.0[4] = GDT_USER_DATA;
    gdt.0[5] = GDT_USER_CODE64;
    gdt.0[6] = tss_lo;
    gdt.0[7] = tss_hi;

    // Load the GDT.  The limit is (8 entries × 8 bytes) − 1 = 63.
    let gdtr = x86_64::structures::DescriptorTablePointer {
        base: VirtAddr::new(gdt.0.as_ptr() as u64),
        limit: (8 * 8 - 1) as u16,
    };
    x86_64::instructions::tables::lgdt(&gdtr);

    // Reload segment registers to flush the old GDT selectors.
    use x86_64::instructions::segmentation::{CS, DS, ES, SS, Segment};
    CS::set_reg(SegmentSelector(0x08)); // kernel code: index 1, RPL 0
    SS::set_reg(SegmentSelector(0));
    DS::set_reg(SegmentSelector(0));
    ES::set_reg(SegmentSelector(0));

    // Load the shared IDT (same as BSP; read-only after init()).
    interrupts::idt_ref().load_unsafe();

    // Load our TSS.  The descriptor we just wrote is not Busy, so ltr succeeds.
    x86_64::instructions::tables::load_tss(SegmentSelector(AP_TSS_SELECTOR));
}

/// Non-naked AP initialisation body.
///
/// Execution order (mirrors BSP early boot, adding per-AP TSS + LAPIC):
///
/// 1. CPU feature init (x87/SSE)
/// 2. Read AP index from AP_ONLINE (before incrementing — BSP serialises startup)
/// 3. Build and load per-AP GDT + TSS
/// 4. Setup SYSCALL MSRs
/// 5. Init LAPIC (enable via SVR)
/// 6. Arm LAPIC timer in periodic mode at ~100 Hz (vector 0x20)
/// 7. Increment AP_ONLINE (Release) — signals BSP to start next AP
/// 8. Enable interrupts (STI)
/// 9. Idle loop (HLT; timer preempts and calls schedule_tick)
unsafe extern "C" fn ap_entry_rust(info: &limine::mp::MpInfo) -> ! {
    // ── Step 1: CPU features ─────────────────────────────────────────────────
    cpu::init_cpu_features();

    // ── Step 2: Determine AP index ───────────────────────────────────────────
    // The BSP starts APs one at a time and waits between each, so at most one
    // AP runs this code concurrently.  AP_ONLINE.load() gives our 0-based index.
    let ap_idx = AP_ONLINE.load(Ordering::Relaxed) as usize;

    // ── Step 3: Per-AP GDT + TSS ─────────────────────────────────────────────
    ap_setup_gdt_and_tss(ap_idx);

    // ── Step 4: SYSCALL/SYSRET MSRs ──────────────────────────────────────────
    unsafe {
        syscall::setup_syscall_msrs(VirtAddr::new(
            syscall::syscall_entry as *const () as u64,
        ));
    }

    // ── Step 5+6: LAPIC init + periodic timer ────────────────────────────────
    // BSP already calibrated the timer; APs just read the stored count.
    lapic::init_lapic();
    lapic::arm_lapic_timer();

    // ── Step 7: Signal BSP ───────────────────────────────────────────────────
    let seq = AP_ONLINE.fetch_add(1, Ordering::Release) + 1;
    crate::serial_println!(
        "[SMP] AP{} (LAPIC {}) online — TSS loaded, LAPIC timer armed",
        seq,
        info.lapic_id,
    );

    // ── Step 8+9: Enable interrupts + idle loop ───────────────────────────────
    // The scheduler will preempt us via LAPIC timer ticks and migrate tasks
    // from other cores or from its own run queue once init_cores() is called.
    x86_64::instructions::interrupts::enable();
    loop {
        x86_64::instructions::hlt();
    }
}
