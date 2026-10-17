//! Bounded native execution ownership. All transitions hold SCHEDULER.
use core::sync::atomic::{AtomicU64, Ordering};

static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);

#[derive(Default, Clone, Copy)]
pub struct Foreground {
    pub incarnation: u64,
    pub parent_generation: u64,
    pub session_owner: bool,
    pub session_broker: u64,
    pub session: u64,
    pub generation: u64,
    pub owned: u64,
    pub member: u64,
    pub cancel: u8,
    pub forced_exit: Option<i32>,
}

impl Foreground {
    pub fn matches_session(&self, broker: u64, session: u64, generation: u64) -> bool {
        broker != 0
            && generation != 0
            && self.session_broker == broker
            && self.session == session
            && self.generation == generation
    }
    pub fn allocate_incarnation() -> Option<u64> {
        NEXT_SCOPE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .ok()
    }
    pub fn execution(&self) -> u64 {
        if self.owned != 0 {
            self.owned
        } else {
            self.member
        }
    }
    pub fn begin(&mut self) -> Option<u64> {
        if self.member != 0 || self.owned != 0 {
            return None;
        }
        let id = Self::allocate_incarnation()?;
        self.owned = id;
        self.cancel = 0;
        Some(id)
    }
}

impl crate::sched::Scheduler {
    /// Protect scoped targets even through the older numeric signal API.
    /// Parent links carry stable publication incarnations, not just PIDs.
    pub fn can_signal_foreground(&self, sender: usize, target: usize) -> bool {
        let scope = self.processes[target].foreground.execution();
        if (scope == 0 && !self.processes[target].foreground.session_owner) || sender == target {
            return true;
        }
        if scope != 0
            && self.processes[target].foreground.owned == 0
            && self.processes[sender].foreground.execution() == scope
        {
            return true;
        }
        let mut child = target;
        for _ in 0..self.processes.len() {
            let generation = self.processes[child].foreground.parent_generation;
            if generation == 0 {
                return false;
            }
            let Some(parent) = self.processes.iter().position(|p| {
                p.pid == self.processes[child].ppid && p.foreground.incarnation == generation
            }) else {
                return false;
            };
            if parent == sender {
                return true;
            }
            if parent == child {
                return false;
            }
            child = parent;
        }
        false
    }
    /// Mark once, wake parked members, and preempt running members. Reclaim
    /// remains in the existing scheduler path, outside this bounded scan.
    pub fn cancel_foreground(&mut self, owner: usize, signal: u8) {
        let scope = self.processes[owner].foreground.owned;
        if scope == 0 || self.processes[owner].foreground.cancel != 0 {
            return;
        }
        self.processes[owner].foreground.cancel = signal;
        let owner_pid = self.processes[owner].pid;
        if let Some(call) = self.processes[owner].pending_call {
            crate::ipc::with_shard(call.endpoint_id, |bus| {
                let _ = crate::ipc::handle_ipc_cancel(owner_pid, self, bus);
            });
        }
        self.wake_for_signal(owner_pid);
        for index in 0..self.processes.len() {
            if self.processes[index].foreground.member != scope
                || self.processes[index].is_exiting()
            {
                continue;
            }
            self.processes[index].foreground.forced_exit = Some(if signal == 126 {
                2
            } else {
                128 + i32::from(signal)
            });
            let pid = self.processes[index].pid;
            self.wake_for_signal(pid);
            if let Some(cpu) = self.live_owner_core(index) {
                crate::sched::request_reschedule_on(cpu);
            }
        }
    }

    /// Terminate every live process in one authenticated PTY session.
    ///
    /// A session controller may be idle while its descendants have no active
    /// foreground execution scope.  Those processes still carry the
    /// generation-qualified session identity, so full session teardown must
    /// use that identity rather than the temporary foreground scope or PPID
    /// links.  Scope cancellation is performed first for active owners so
    /// pending foreground IPC and descendants receive the normal cancellation
    /// transition; the second pass then forces every remaining session member
    /// to exit.  Repeated calls preserve the first accepted termination state.
    pub fn terminate_session(&mut self, broker: u64, session: u64, generation: u64, signal: u8) {
        if broker == 0 || session == 0 || generation == 0 {
            return;
        }

        for index in 0..self.processes.len() {
            if !self.processes[index]
                .foreground
                .matches_session(broker, session, generation)
                || self.processes[index].is_exiting()
                || !self.processes[index].foreground.session_owner
                || self.processes[index].foreground.owned == 0
            {
                continue;
            }
            self.cancel_foreground(index, signal);
        }

        let exit_code = if signal == 126 {
            2
        } else {
            128 + i32::from(signal)
        };
        for index in 0..self.processes.len() {
            if !self.processes[index]
                .foreground
                .matches_session(broker, session, generation)
                || self.processes[index].is_exiting()
            {
                continue;
            }

            let pid = self.processes[index].pid;
            if let Some(call) = self.processes[index].pending_call {
                crate::ipc::with_shard(call.endpoint_id, |bus| {
                    let _ = crate::ipc::handle_ipc_cancel(pid, self, bus);
                });
            }
            if self.processes[index].foreground.forced_exit.is_none() {
                self.processes[index].foreground.forced_exit = Some(exit_code);
            }
            self.wake_for_signal(pid);
            if let Some(cpu) = self.live_owner_core(index) {
                crate::sched::request_reschedule_on(cpu);
            }
        }
    }
}
