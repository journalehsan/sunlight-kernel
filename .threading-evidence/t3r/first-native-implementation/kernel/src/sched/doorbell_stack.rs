// Shared kernel-stack park trampoline. See F1.3N stack lifetime proof.
// SAFETY: the dispatcher supplies its live 15-register SyscallRegs frame.
// Its process owns the kernel stack; all guards are released before switching.
// RBX retains the trusted input frame only while this same task owns execution.
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn on_kernel_stack(
    _frame: &mut crate::arch::x86_64::syscall::SyscallFrame,
) -> u64 {
    core::arch::naked_asm!(
        // Ordinary entry already owns this task's supervisor stack. Never
        // repivot over the live dispatcher; retain its frame across parking.
        "push rbx",
        "mov rbx, rdi",
        "call {fixture}",
        "test rax, rax",
        "jnz 2f",
        "cmp qword ptr [rbx], 0xf1c2",
        "je 3f",
        "cmp qword ptr [rbx + 40], 0",
        "jne 2f",
        "3:",
        "call {flush}",
        "xor eax, eax",
        "int {vector}",
        "2:",
        ".if {trace}",
        "mov rdi, rbx",
        "pushfq",
        "pop rsi",
        "cli",
        "push rax",
        "sub rsp, 8",
        "call {resume}",
        "add rsp, 8",
        "pop rax",
        ".endif",
        "pop rbx",
        "ret",
        flush = sym flush,
        fixture = sym fixture,
        resume = sym resume,
        trace = const cfg!(all(debug_assertions, feature = "fastlane_doorbell_trace")) as u8,
        vector = const crate::arch::x86_64::interrupts::RESCHEDULE_VECTOR,
    );
}

extern "C" fn resume(_frame: &crate::arch::x86_64::syscall::SyscallFrame, _flags: u64) {
    #[cfg(all(debug_assertions, feature = "fastlane_doorbell_trace"))]
    {
        let s = super::SCHEDULER.lock();
        let p = s.current_process();
        super::bell_trace::event(
            "resume",
            _frame.rsi,
            p.pid,
            p.foreground.incarnation,
            p.address_space.identity().generation,
            _flags,
            _frame.rcx,
        );
        assert_eq!(
            _flags & 0x200,
            0,
            "Doorbell kernel continuation resumed with IF set"
        );
    }
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
