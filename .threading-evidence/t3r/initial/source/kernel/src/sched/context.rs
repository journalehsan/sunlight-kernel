use crate::process::Process;

/// Switch to a process context by loading its saved_rsp and doing iretq.
/// `context_rsp` points to the start of the saved register frame (15 GPRs).
/// We pop all 15 GPRs, then point to the IRETQ frame (RIP, CS, RFLAGS, RSP, SS).
/// SAFETY: `context_rsp` must point to a valid saved frame on a kernel stack.
pub unsafe fn iretq_to_context(context_rsp: u64) -> ! {
    core::arch::asm!(
        "mov rsp, rax",
        "pop r15", "pop r14", "pop r13", "pop r12", "pop rbp", "pop rbx",
        "pop r11", "pop r10", "pop r9", "pop r8",
        "pop rdi", "pop rsi", "pop rdx", "pop rcx", "pop rax",
        "iretq",
        in("rax") context_rsp,
        options(noreturn)
    );
}

/// Save the current interrupt context into a process.
/// `current_rsp` points to the saved registers (15 GPRs) pushed by the timer handler.
/// SAFETY: Must be called from the timer handler with a valid stack.
pub unsafe fn save_current_context(current_rsp: u64, process: &mut Process) {
    process.context_rsp = current_rsp;
}
