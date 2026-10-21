use super::doorbell_state::{Completion, ConditionalWait, Handle, Outcome, Registration, Token};
use super::{current_cpu_id, request_park_after_sysret, ProcessState, Scheduler};
use sunlight_fastlane_doorbell::{Error, Identity};

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
        "cmp qword ptr [rsp + 40], 0",
        "jne 2f",
        "int {vector}",
        "2:",
        "add rsp, 136",
        "pop rsp",
        "ret",
        stack_top = sym stack_top,
        fixture = sym fixture,
        vector = const crate::arch::x86_64::interrupts::RESCHEDULE_VECTOR,
    );
}

extern "C" fn stack_top() -> u64 {
    super::SCHEDULER.lock().current_process().kernel_stack_top
}

extern "C" fn fixture(frame: &crate::arch::x86_64::syscall::SyscallFrame) -> u64 {
    super::doorbell_test::syscall(frame.rdi)
}

impl Scheduler {
    pub(super) fn doorbell_identity(&self, index: usize) -> Result<Identity, Error> {
        let process = self.processes.get(index).ok_or(Error::Stale)?;
        if process.is_exiting() || process.foreground.incarnation == 0 {
            return Err(Error::Stale);
        }
        Ok(Identity {
            pid: process.pid as u64,
            incarnation: process.foreground.incarnation,
        })
    }

    pub(super) fn doorbell_create(&mut self, consumer: usize) -> Result<Handle, Error> {
        let identity = self.doorbell_identity(consumer)?;
        if identity.pid == 0 {
            return Err(Error::Unauthorized);
        }
        self.doorbells.create(identity)
    }

    pub(super) fn doorbell_snapshot(&self, handle: Handle, consumer: usize) -> Result<u64, Error> {
        self.doorbells
            .snapshot(handle, self.doorbell_identity(consumer)?)
    }

    pub(super) fn doorbell_wait_if_unchanged(
        &mut self,
        handle: Handle,
        observed: u64,
    ) -> Result<ConditionalWait, Error> {
        let index = self.current_process_index().ok_or(Error::Stale)?;
        let identity = self.doorbell_identity(index)?;
        if self.live_owner_core(index) != Some(current_cpu_id())
            || self.processes[index].state != ProcessState::Running
        {
            return Err(Error::Invalid);
        }
        if self.processes[index].doorbell_wait.is_some() {
            return Err(Error::AlreadyWaiting);
        }
        super::doorbell_test::registration_checkpoint(1);
        let result = self.doorbells.wait(handle, identity, observed)?;
        if let ConditionalWait::Registered(token) = result {
            self.processes[index].doorbell_wait = Some(Registration {
                token,
                outcome: None,
            });
            super::doorbell_test::registration_checkpoint(2);
            self.processes[index].block_start_tick = self.global_tick;
            self.set_state(index, ProcessState::BlockedOnIo);
            request_park_after_sysret();
        }
        Ok(result)
    }

    fn doorbell_complete(&mut self, completion: Option<Completion>) -> bool {
        let Some(completion) = completion else {
            return false;
        };
        let Some(index) = self.processes.iter().position(|process| {
            process.pid as u64 == completion.identity.pid
                && process.foreground.incarnation == completion.identity.incarnation
                && !process.is_exiting()
        }) else {
            return false;
        };
        let Some(registration) = self.processes[index].doorbell_wait.as_mut() else {
            return false;
        };
        if registration.token != completion.token || registration.outcome.is_some() {
            return false;
        }
        registration.outcome = Some(completion.outcome);
        let pid = self.processes[index].pid;
        self.wake_io_pid(pid);
        if self.live_owner_core(index) == Some(current_cpu_id())
            || self.processes[index].queued_on_core as usize == current_cpu_id()
        {
            request_park_after_sysret();
        }
        true
    }

    pub(super) fn doorbell_notify(&mut self, handle: Handle) -> Result<bool, Error> {
        let completion = self.doorbells.notify(handle)?;
        if completion.is_some() {
            super::doorbell_test::notify_commit();
        }
        Ok(self.doorbell_complete(completion))
    }

    pub(super) fn doorbell_close(&mut self, handle: Handle) -> Result<bool, Error> {
        let completion = self.doorbells.close(handle)?;
        Ok(self.doorbell_complete(completion))
    }

    pub(super) fn doorbell_producer_exit(&mut self, handle: Handle) -> Result<bool, Error> {
        let completion = self.doorbells.producer_exit(handle)?;
        Ok(self.doorbell_complete(completion))
    }

    pub(super) fn doorbell_cancel(
        &mut self,
        token: Token,
        identity: Identity,
    ) -> Result<bool, Error> {
        let completion = self.doorbells.cancel(token, identity, Outcome::Cancelled)?;
        Ok(self.doorbell_complete(completion))
    }

    pub(super) fn doorbell_take_outcome(&mut self, index: usize) -> Result<Outcome, Error> {
        self.doorbell_identity(index)?;
        let registration = self.processes[index].doorbell_wait.ok_or(Error::Invalid)?;
        let outcome = registration.outcome.ok_or(Error::AlreadyWaiting)?;
        self.processes[index].doorbell_wait = None;
        Ok(outcome)
    }

    pub(super) fn doorbell_retire(&mut self, handle: Handle) -> Result<(), Error> {
        if self.processes.iter().any(|process| {
            process
                .doorbell_wait
                .is_some_and(|registration| registration.token.handle == handle)
        }) {
            return Err(Error::AlreadyWaiting);
        }
        self.doorbells.retire(handle)
    }

    pub(super) fn doorbell_spurious_wake(&mut self, index: usize) {
        let Some(registration) = self.processes[index].doorbell_wait else {
            return;
        };
        if registration.outcome.is_some() {
            return;
        }
        let identity = self.doorbell_identity(index).expect("live Doorbell waiter");
        let completion = self
            .doorbells
            .cancel(registration.token, identity, Outcome::Spurious)
            .expect("exact Doorbell registration");
        assert!(completion.is_some());
        self.processes[index]
            .doorbell_wait
            .as_mut()
            .unwrap()
            .outcome = Some(Outcome::Spurious);
    }

    pub(super) fn doorbell_peer_exit(&mut self, index: usize) {
        let process = &self.processes[index];
        let identity = Identity {
            pid: process.pid as u64,
            incarnation: process.foreground.incarnation,
        };
        self.processes[index].doorbell_wait = None;
        for completion in self.doorbells.peer_exit(identity) {
            self.doorbell_complete(completion);
        }
    }
}
