//! Minimal xAPIC local APIC driver — Phase 1 SMP.
//!
//! Each logical CPU has its own Local APIC (LAPIC) that handles:
//! - Per-core timer interrupts (replaces BSP-only 8259 PIC timer)
//! - Inter-processor interrupts (IPIs, not yet used)
//! - EOI acknowledgement for local interrupt delivery
//!
//! This driver uses MMIO mode (xAPIC), where registers are 32-bit MMIO
//! at offsets from the LAPIC base address (~0xFEE00000 on most platforms).
//! The physical address is obtained from `acpi::lapic_base_addr()` (MADT).
//!
//! ## Initialization sequence (per core)
//!
//! 1. Read LAPIC base from MADT (physical addr, identity-mapped in kernel VA).
//! 2. Write SVR to enable LAPIC and set spurious vector.
//! 3. Set LVT timer divisor.
//! 4. (Calibration only on BSP) Measure LAPIC timer frequency against TSC.
//! 5. Write LVT timer: periodic mode, vector 0x20, unmasked.
//! 6. Write initial count to start the timer.
//!
//! ## Timer vector
//!
//! Vector 0x20 is reused from the PIC IRQ0 mapping. After BSP LAPIC init,
//! PIC IRQ0 is masked so only the LAPIC timer fires at vector 0x20.
//! APs also arm their LAPIC timers at vector 0x20; the shared IDT entry
//! (`timer_entry`) handles all cores.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

// ─── LAPIC MMIO register offsets ──────────────────────────────────────────────

const LAPIC_ID: usize = 0x020; // Local APIC ID register (read)
const LAPIC_EOI: usize = 0x0B0; // End-of-interrupt register (write 0)
const LAPIC_ICR_LOW: usize = 0x300; // Interrupt Command Register low
const LAPIC_ICR_HIGH: usize = 0x310; // Interrupt Command Register high
const LAPIC_SVR: usize = 0x0F0; // Spurious Interrupt Vector register
const LAPIC_LVT_TIMER: usize = 0x320; // LVT timer register
const LAPIC_TIMER_INITIAL: usize = 0x380; // Initial count register
const LAPIC_TIMER_CURRENT: usize = 0x390; // Current count register (read)
const LAPIC_TIMER_DCR: usize = 0x3E0; // Divide Configuration register

// ─── Constants ────────────────────────────────────────────────────────────────

/// Timer interrupt vector — same as the PIC IRQ0 mapping in the IDT.
/// The shared `timer_entry` handler fires on all cores via this vector.
const TIMER_VECTOR: u32 = 0x20;

/// LAPIC timer divisor: divide bus clock by 16 before counting.
/// Value 0x3 in DCR means "divide by 16" (per x86 spec Table 10-2).
const TIMER_DIVISOR: u32 = 0x3;

/// Calibrated LAPIC timer initial count for ~100 Hz periodic ticks.
/// Written by the BSP calibration and reused by APs.
static LAPIC_TIMER_INIT_COUNT: AtomicU32 = AtomicU32::new(0);
static LAPIC_TIMER_HZ: AtomicU64 = AtomicU64::new(0);

// ─── MMIO helpers ─────────────────────────────────────────────────────────────

/// Return the virtual address of the LAPIC MMIO region.
///
/// The LAPIC is memory-mapped at a physical address (typically 0xFEE00000)
/// reported by the ACPI MADT.  MMIO regions are NOT identity-mapped in the
/// kernel's lower virtual address range; they live at physical + HHDM_OFFSET.
/// We apply the Limine HHDM offset to get the correct virtual address.
#[inline(always)]
fn lapic_base() -> usize {
    let phys = crate::arch::x86_64::acpi::lapic_base_addr();
    let phys = if phys != 0 {
        phys
    } else {
        // Fallback: read IA32_APIC_BASE MSR (bits [51:12] = physical base).
        let msr_val = unsafe { x86_64::registers::model_specific::Msr::new(0x1B).read() };
        msr_val & 0x000F_FFFF_FFFF_F000
    };
    // The LAPIC MMIO physical address must be translated to a kernel virtual
    // address using the HHDM offset.  This is the same pattern used throughout
    // the kernel (syscall.rs, shared.rs, etc.) for physical → virtual translation.
    let hhdm = crate::HHDM_REQ.response().map_or(0, |r| r.offset);
    (phys + hhdm) as usize
}

/// Write a 32-bit value to a LAPIC register.
///
/// # Safety
/// `offset` must be a valid LAPIC MMIO register offset.
#[inline(always)]
unsafe fn lapic_write(offset: usize, val: u32) {
    let ptr = (lapic_base() + offset) as *mut u32;
    ptr.write_volatile(val);
}

/// Read a 32-bit value from a LAPIC register.
///
/// # Safety
/// `offset` must be a valid LAPIC MMIO register offset.
#[inline(always)]
unsafe fn lapic_read(offset: usize) -> u32 {
    let ptr = (lapic_base() + offset) as *const u32;
    ptr.read_volatile()
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Enable this core's Local APIC.
///
/// Programs the Spurious Interrupt Vector register (SVR):
/// - Bit 8 enables the APIC unit.
/// - Lower 8 bits are the spurious vector (0xFF, handled by the IDT's
///   default handler or ignored; we do not install a handler for it).
///
/// Must be called once per logical CPU (BSP and each AP) before any
/// LAPIC timer or IPI functionality is used.
///
/// # Safety
/// Caller must ensure interrupts are disabled (IF=0) during init to
/// avoid receiving timer interrupts before the timer is fully armed.
pub unsafe fn init_lapic() {
    // Enable LAPIC; set spurious vector to 0xFF.
    lapic_write(LAPIC_SVR, 0x1FF);
}

/// Return the xAPIC ID of the processor executing this code.
#[inline]
pub fn local_apic_id() -> u32 {
    unsafe { lapic_read(LAPIC_ID) >> 24 }
}

/// Signal end-of-interrupt to the Local APIC.
///
/// Must be called from interrupt handlers for LAPIC-delivered interrupts
/// (i.e., any vector that comes from the LAPIC rather than the 8259 PIC).
/// Writing 0 to the EOI register acknowledges the current interrupt and
/// allows the LAPIC to deliver the next one at this or lower priority.
///
/// # Safety
/// Must be called exactly once per LAPIC interrupt, before returning
/// from the interrupt handler. Calling without an active interrupt is
/// harmless but wasteful.
#[inline(always)]
pub unsafe fn send_eoi() {
    lapic_write(LAPIC_EOI, 0);
}

fn wait_for_icr_idle() {
    while unsafe { lapic_read(LAPIC_ICR_LOW) } & (1 << 12) != 0 {
        core::hint::spin_loop();
    }
}

/// Send a fixed-delivery IPI to one logical CPU/APIC ID.
///
/// SunlightOS currently uses xAPIC mode and its scheduler CPU index is the
/// initial APIC ID, capped to the supported 64-core mask.
pub unsafe fn send_fixed_ipi(cpu_id: usize, vector: u8) {
    assert!(
        cpu_id < crate::sched::MAX_CORES,
        "IPI target is out of range"
    );
    wait_for_icr_idle();
    lapic_write(LAPIC_ICR_HIGH, (cpu_id as u32) << 24);
    lapic_write(LAPIC_ICR_LOW, vector as u32);
    wait_for_icr_idle();
}

/// Send an NMI doorbell to a CPU. MM-2B uses this only to make a published
/// fixed-vector shootdown mailbox consumable while the target has IF cleared.
pub unsafe fn send_nmi(cpu_id: usize) {
    assert!(
        cpu_id < crate::sched::MAX_CORES,
        "NMI target is out of range"
    );
    wait_for_icr_idle();
    lapic_write(LAPIC_ICR_HIGH, (cpu_id as u32) << 24);
    lapic_write(LAPIC_ICR_LOW, 0b100 << 8);
    wait_for_icr_idle();
}

/// Calibrate the LAPIC timer against the TSC and store the result.
///
/// Runs a one-shot measurement: sets the LAPIC timer to a large count,
/// waits for a TSC-measured 10 ms window, reads how many LAPIC ticks
/// elapsed, and computes `initial_count = lapic_hz / target_hz`.
///
/// The result is stored in `LAPIC_TIMER_INIT_COUNT` for APs to reuse
/// without repeating the calibration.
///
/// Falls back to a conservative fixed count (62 500 ≈ 100 Hz with a
/// 100 MHz bus and divisor 16) when TSC is uncalibrated.
///
/// # Safety
/// Must be called with interrupts disabled. Spins for ~10 ms.
pub unsafe fn calibrate_lapic_timer(target_hz: u32) -> u32 {
    let tsc_hz = crate::arch::x86_64::interrupts::tsc_hz();

    if tsc_hz == 0 {
        // TSC not calibrated — use QEMU default: ~100 MHz bus / 16 / 100 Hz
        let fallback = 62_500u32;
        LAPIC_TIMER_INIT_COUNT.store(fallback, Ordering::Release);
        LAPIC_TIMER_HZ.store(fallback as u64 * target_hz as u64, Ordering::Release);
        return fallback;
    }

    // One-shot mode, masked (vector 0xFF so nothing fires) for calibration.
    lapic_write(LAPIC_TIMER_DCR, TIMER_DIVISOR);
    lapic_write(LAPIC_LVT_TIMER, 0x0001_00FF); // one-shot, masked, vector 0xFF

    // Start a full-range countdown.
    lapic_write(LAPIC_TIMER_INITIAL, 0xFFFF_FFFF);

    // Wait exactly 10 ms measured via TSC.
    let wait_tsc = tsc_hz / 100; // 1/100 of a second = 10 ms
    let (lo0, hi0): (u32, u32);
    core::arch::asm!("rdtsc", out("eax") lo0, out("edx") hi0, options(nostack, nomem));
    let start_tsc = (hi0 as u64) << 32 | lo0 as u64;
    loop {
        let (lo1, hi1): (u32, u32);
        core::arch::asm!("rdtsc", out("eax") lo1, out("edx") hi1, options(nostack, nomem));
        let now = (hi1 as u64) << 32 | lo1 as u64;
        if now.wrapping_sub(start_tsc) >= wait_tsc {
            break;
        }
        core::hint::spin_loop();
    }

    let current_count = lapic_read(LAPIC_TIMER_CURRENT);
    let ticks_in_10ms = 0xFFFF_FFFFu32.wrapping_sub(current_count);

    // LAPIC Hz ≈ ticks_in_10ms × 100
    let lapic_hz = (ticks_in_10ms as u64).saturating_mul(100);
    let initial_count = if lapic_hz == 0 {
        62_500u32 // fallback
    } else {
        ((lapic_hz / target_hz as u64) as u32).max(1)
    };

    LAPIC_TIMER_INIT_COUNT.store(initial_count, Ordering::Release);
    LAPIC_TIMER_HZ.store(lapic_hz, Ordering::Release);
    initial_count
}

/// Calibrated LAPIC countdown frequency after the programmed divisor.
pub fn timer_frequency_hz() -> u64 {
    LAPIC_TIMER_HZ.load(Ordering::Acquire)
}

pub fn timer_initial_count() -> u32 {
    LAPIC_TIMER_INIT_COUNT.load(Ordering::Acquire)
}

/// Arm the LAPIC timer in periodic mode at the previously calibrated rate.
///
/// Uses the count stored by `calibrate_lapic_timer()`. If called before
/// calibration (count is 0), falls back to a fixed 62 500 count.
///
/// Configures:
/// - DCR: divide by 16
/// - LVT timer: periodic, vector 0x20 (shared with BSP timer_entry)
/// - Initial count: calibrated value → fires at ~100 Hz
///
/// # Safety
/// Must be called with interrupts disabled.
pub unsafe fn arm_lapic_timer() {
    let mut initial_count = LAPIC_TIMER_INIT_COUNT.load(Ordering::Acquire);
    if initial_count == 0 {
        initial_count = 62_500;
    }
    lapic_write(LAPIC_TIMER_DCR, TIMER_DIVISOR);
    // Periodic mode = bit 17 set; vector = TIMER_VECTOR.
    lapic_write(LAPIC_LVT_TIMER, 0x0002_0000 | TIMER_VECTOR);
    lapic_write(LAPIC_TIMER_INITIAL, initial_count);
}
