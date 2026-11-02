// Shared kernel-stack park trampoline. See F1.3N stack lifetime proof.
// SAFETY: the dispatcher supplies its live 15-register SyscallRegs frame.
// Its process owns the kernel stack; all guards are released before switching.
// The saved original stack pointer is used only after this same task resumes.
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn on_kernel_stack(
    _frame: &mut crate::arch::x86_64::syscall::SyscallFrame,
) -> u64 {
    core::arch::naked_asm!(
        "push rdi",
        "call {stack_top}",
        "pop rdi",
        "mov r11, rsp",
        "mov rsp, rax",
        "sub rsp, 256",
        "and rsp, -16",
        "push r11",
        "sub rsp, 136",
        "mov rsi, rdi",
        "mov rdi, rsp",
        "mov ecx, 15",
        "cld",
        "rep movsq",
        "mov rdi, rsp",
        "call {fixture}",
        "test rax, rax",
        "jnz 2f",
        "cmp qword ptr [rsp], 0xf1c2",
        "je 3f",
        "cmp qword ptr [rsp + 40], 0",
        "jne 2f",
        "3:",
        "call {flush}",
        "xor eax, eax",
        "int {vector}",
        "2:",
        "add rsp, 136",
        "pop rsp",
        "ret",
        flush = sym flush,
        stack_top = sym stack_top,
        fixture = sym fixture,
        vector = const crate::arch::x86_64::interrupts::RESCHEDULE_VECTOR,
    );
}

extern "C" fn stack_top() -> u64 {
    super::SCHEDULER.lock().current_process().kernel_stack_top
}

extern "C" fn fixture(frame: &crate::arch::x86_64::syscall::SyscallFrame) -> u64 {
    #[cfg(all(debug_assertions, feature = "fastlane_doorbell_test"))]
    if frame.rax == 0xf1c2 {
        use crate::capability::{channel::Handle, CapabilityToken};
        let (mut sched, actor) = crate::capability::bell_race::acquire(12, frame.rsi);
        let result = sched.fastlane_bell(
            12,
            Handle {
                token: CapabilityToken(frame.rdi),
                generation: frame.rsi,
            },
            frame.rdx,
        );
        crate::capability::bell_race::finished(
            actor,
            result.as_ref().err().map_or(0, |e| *e as u64),
        );
        return match result {
            Ok(_) => 0,
            Err(e) => e as u64,
        };
    }
    #[cfg(all(debug_assertions, feature = "native_doorbell_test"))]
    {
        return super::doorbell_test::syscall(frame.rdi);
    }
    #[allow(unreachable_code)]
    1
}

extern "C" fn flush() {
    super::flush_dispatch_notifications();
}

#[cfg(all(debug_assertions, feature = "fastlane_doorbell_test"))]
impl super::Scheduler {
    pub(crate) fn fastlane_bell_physically_parked(&self, index: usize) -> bool {
        let p = &self.processes[index];
        p.state == super::ProcessState::BlockedOnIo
            && p.owning_core == u8::MAX
            && p.queued_on_core == u8::MAX
            && !self.stack_is_pinned(index)
            && self.diagnostic_ready_occurrences(index) == 0
    }
}
