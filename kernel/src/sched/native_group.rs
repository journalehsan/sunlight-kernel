//! Retained native Process authority. All methods require SCHEDULER.
//! Task Finished and group terminal publication are distinct; VM identity is
//! membership, not permission to impersonate the retained owner.
use super::*;

impl Scheduler {
    /// Observation only: native guest witnesses query syscall 156, which
    /// still rejects an owner as a non-borrower. Record the actual handoff
    /// state before returning to the querying worker; never synthesize a reap.
    #[cfg(feature = "native_c_guest")]
    pub(crate) fn trace_native_owner_handoff(&self, target_pid: usize) {
        let Some(member) = self.current_process_index() else {
            return;
        };
        let Some(owner) = self.native_group_owner_index(member) else {
            return;
        };
        let p = &self.processes[owner];
        if self.processes[member].native_thread
            && p.pid == target_pid
            && p.state == ProcessState::Finished
            && p.native_group_exit.is_none()
        {
            let quiescent = !self.stack_is_pinned(owner)
                && p.owning_core == u8::MAX
                && p.queued_on_core == u8::MAX
                && (0..self.online_cores).all(|cpu| self.core_current_task(cpu) != Some(owner));
            serial_println!("[NATIVE-GROUP-WITNESS] owner={} incarnation={} vm={} worker={} wi={} owner_finished=1 quiescent={} group_live=1 cpu={}",
                p.pid, p.foreground.incarnation, p.address_space.identity().generation,
                self.processes[member].pid, self.processes[member].foreground.incarnation,
                u8::from(quiescent), current_cpu_id());
        }
    }

    pub(crate) fn native_group_owner_index(&self, member: usize) -> Option<usize> {
        let identity = self.processes[member].address_space.identity();
        if self.processes[member].is_linux_compat() {
            return None;
        }
        self.processes.iter().position(|p| {
            p.native_group_owner()
                && p.state != ProcessState::Reaped
                && p.address_space.identity() == identity
        })
    }

    pub(super) fn native_group_has_live_tasks(&self, owner: usize) -> bool {
        let identity = self.processes[owner].address_space.identity();
        self.processes.iter().any(|p| {
            p.address_space.identity() == identity
                && !matches!(p.state, ProcessState::Finished | ProcessState::Reaped)
        })
    }

    /// One terminal receipt. Membership/publication, termination and reaping
    /// all use SCHEDULER. Retain the owner row until every borrower is Reaped.
    pub(super) fn begin_native_group_exit(
        &mut self,
        owner: usize,
        code: i32,
        reason: &str,
        exempt: Option<usize>,
    ) {
        if self.processes[owner].native_group_exit.is_some() {
            return;
        }
        assert!(self.processes[owner].native_group_owner());
        let identity = self.processes[owner].address_space.identity();
        let pid = self.processes[owner].pid;
        self.processes[owner].native_group_exit = Some(code);
        self.processes[owner].exit_code = code;
        self.cancel_foreground(owner, 1);
        // No Vec of targets or counter to overflow. Recursion is bounded to
        // one level: the receipt above fences a second group transition.
        for idx in 0..self.processes.len() {
            if Some(idx) != exempt
                && self.processes[idx].address_space.identity() == identity
                && !matches!(
                    self.processes[idx].state,
                    ProcessState::Finished | ProcessState::Reaped
                )
            {
                let target = self.processes[idx].pid;
                self.terminate_process_by_pid(target, code, reason);
            }
        }
        self.close_owned_ipc_endpoints(pid);
        note_process_finished(pid, self.processes[owner].name_str());
        serial_println!(
            "[NATIVE-GROUP] terminal pid={} incarnation={} vm={} code={} reason={}",
            pid,
            self.processes[owner].foreground.incarnation,
            identity.generation,
            code,
            reason
        );
        let parent_pid = self.processes[owner].ppid;
        if self
            .process_mut_by_pid(parent_pid)
            .is_some_and(|p| p.wait_child == Some(pid))
        {
            self.wake_pid(parent_pid);
        }
    }
}
