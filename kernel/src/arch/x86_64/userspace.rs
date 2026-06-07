use x86_64::structures::gdt::{Descriptor, SegmentSelector};
use x86_64::structures::tss::TaskStateSegment;

/// User segment selectors. Must match GDT layout.
pub const USER_CS: u64 = 0x2B; // ring 3, 64-bit code
pub const USER_SS: u64 = 0x23; // ring 3, data

/// Build a GDT that includes user segments.
/// This replaces the simple GDT in interrupts.rs.
/// SAFETY: Must be called during early boot before loading segments.
pub unsafe fn setup_gdt() {
    // For now, this function is a placeholder.
    // The actual GDT setup will be done in interrupts.rs.
}

/// Jump to user-space using IRETQ.
/// This is simpler than SYSRETQ and doesn't require user GDT segments.
/// SAFETY: `entry` must be a valid user-space code address.
/// `user_stack_top` must be a valid writable user address.
pub unsafe fn jump_to_userspace(entry: u64, user_stack_top: u64) -> ! {
    let rip = entry;
    let rsp = user_stack_top;
    let rflags: u64 = 0x202; // IF set

    // Push the IRETQ frame onto the current stack.
    // IRETQ pops in this order: RIP, CS, RFLAGS, RSP, SS
    core::arch::asm!(
        "push {ss}",
        "push {rsp}",
        "push {rflags}",
        "push {cs}",
        "push {rip}",
        "iretq",
        ss = in(reg) 0x23u64,   // ring 3 data segment selector
        rsp = in(reg) rsp,
        rflags = in(reg) rflags,
        cs = in(reg) 0x2Bu64,   // ring 3 code segment selector (64-bit)
        rip = in(reg) rip,
        options(noreturn)
    );
}
