//! Short, IRQ-masked entry bridge, never a shared execution stack.
//!
//! IA32_KERNEL_GS_BASE points at the local slot even when userspace loads GS.
//! Entry borrows it with SWAPGS, saves RSP without a stack access, selects the
//! dispatched thread's stack, pushes the saved RSP there, and swaps back before
//! any call or scheduling. NMI uses IST[1], never this slot, and cannot schedule.
//! No continuation depends on a slot after that second SWAPGS; migration is OK.
use core::sync::atomic::{AtomicU64, Ordering};

#[repr(C, align(64))]
struct Entry {
    top: AtomicU64,
    user_rsp: AtomicU64,
}

static ENTRIES: [Entry; 64] = [const {
    Entry {
        top: AtomicU64::new(0),
        user_rsp: AtomicU64::new(0),
    }
}; 64];

const _: () = assert!(core::mem::offset_of!(Entry, top) == 0);
const _: () = assert!(core::mem::offset_of!(Entry, user_rsp) == 8);

/// Called locally before this CPU can run userspace. FSGSBASE must stay off;
/// userspace has no API that writes IA32_KERNEL_GS_BASE.
///
/// # Safety
/// Invoke on this CPU before CPL3 entry, with IRQs disabled and its final CPU
/// identity installed. No caller may issue a CPL0 SYSCALL or enable FSGSBASE.
pub unsafe fn init() {
    let slot = &ENTRIES[crate::sched::current_cpu_id()];
    x86_64::registers::model_specific::Msr::new(0xc0000102).write(slot as *const Entry as u64);
}

/// Validate allocation identity under the scheduler's existing ownership lock,
/// before installing its top for the next CPL3 entry. No new hot-path lock.
pub fn validate(process: &crate::process::Process) {
    let stack = process
        .kernel_stack
        .as_ref()
        .expect("dispatch without owned stack");
    assert_eq!(
        stack.as_ptr() as u64 + crate::process::KERNEL_STACK_SIZE as u64,
        process.kernel_stack_top
    );
}

/// Publish only while IRQs are disabled, together with local TSS RSP0.
/// The scheduler owns the selected task; its existing physical handoff pins
/// protect the old stack until the final pivot. Zero disarms idle entry.
pub fn select(stack_top: u64) {
    assert!(!x86_64::instructions::interrupts::are_enabled());
    assert!(stack_top == 0 || stack_top >= 0xffff_8000_0000_0000);
    ENTRIES[crate::sched::current_cpu_id()]
        .top
        .store(stack_top, Ordering::Relaxed);
}

/// Distinct supervisor NMI stacks cover the pre-pivot and post-restore windows.
/// NMI delivery blocks further NMIs until IRET; the handler never schedules.
#[repr(align(16))]
struct NmiStack([u8; 16 * 1024]);
static mut NMI_STACKS: [NmiStack; 64] = [const { NmiStack([0; 16 * 1024]) }; 64];

pub fn nmi_stack_top(cpu: usize) -> u64 {
    // No Rust reference to storage modified by hardware/interrupt entry.
    unsafe {
        core::ptr::addr_of_mut!(NMI_STACKS[cpu].0)
            .cast::<u8>()
            .add(16 * 1024) as u64
    }
}
