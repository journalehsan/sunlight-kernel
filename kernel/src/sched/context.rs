use super::thread::CpuContext;

/// Switch from `old` context to `new` context.
/// Saves callee-saved registers on old stack, restores from new stack.
/// SAFETY: Both contexts must belong to valid threads with properly initialized stacks.
/// Must not be called while holding any locks that could deadlock.
#[unsafe(naked)]
pub unsafe extern "C" fn switch_to(_old: &mut CpuContext, _new: &CpuContext) {
    core::arch::naked_asm!(
        "push rbp",
        "push rbx",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov [rdi], rsp",       // save old RSP to old->rsp
        "mov rsp, [rsi]",       // load new RSP from new->rsp
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",                  // jumps to new thread's saved RIP
    );
}

/// Switch from a thread that is exiting (no saving) to `new` context.
/// SAFETY: `new` context must be valid. Never returns.
#[unsafe(naked)]
pub unsafe extern "C" fn switch_to_exit(_new: &CpuContext) -> ! {
    core::arch::naked_asm!(
        "mov rsp, [rdi]",       // load new RSP from new->rsp
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbx",
        "pop rbp",
        "ret",
    );
}
