//! Level-triggered receive hints. No queue allocation and no control delivery.
//! Scheduler serialization covers both the final receive check and blocking;
//! the BSP tick retries outstanding work after at most one clock tick.
use super::*;
use crate::ipc::IpcMsg;
use crate::process::ProcessState;
use sunlight_ipc::vfs_rpc::retention::transport::CONTROL_WAKEUP;

impl RemoteRegistration {
    fn control_ready(&self, now_tick: u64) -> bool {
        self.last_wake_tick != Some(now_tick)
            && (self.callers.iter().flatten().any(CallerDelivery::pending)
                || (self.delivered.is_none() && self.unsent.iter().any(Option::is_some))
                || self.ownership.releases().next().is_some())
    }
}

impl Runtime {
    /// Any unreplied file publication (descriptor op or kernel-origin transfer)
    /// is real work the service must pull. Without this the copied file channel
    /// would only run when some unrelated wakeup happened to arrive.
    fn file_ready(&self, registration: Registration) -> bool {
        self.file_publications.iter().flatten().any(|publication| {
            publication.identity.registration == registration && !publication.replied
        })
    }

    fn remote_ready(&self, remote: &RemoteRegistration, now_tick: u64) -> bool {
        remote.control_ready(now_tick)
            || (remote.last_wake_tick != Some(now_tick) && self.stat_ready(remote.registration))
            || (remote.last_wake_tick != Some(now_tick) && self.file_ready(remote.registration))
    }

    fn wake_owner(&self, process: &crate::process::Process, endpoint: u32, now_tick: u64) -> bool {
        if process.is_exiting() || !process.trusted_vfs_service {
            return false;
        }
        let caller = process_identity(process);
        let registered_endpoint = self.remote.iter().flatten().any(|r| {
            r.registration.owner.endpoint == u64::from(endpoint)
                && owner_matches(r.registration.owner, caller)
        });
        // The service polls every retained epoch on each hint. Old work must
        // also wake a replacement endpoint owned by the same exact process.
        registered_endpoint
            && self.remote.iter().flatten().any(|r| {
                owner_matches(r.registration.owner, caller) && self.remote_ready(r, now_tick)
            })
    }

    fn take_control_wakeup(
        &mut self,
        process: &crate::process::Process,
        endpoint: u32,
        now_tick: u64,
    ) -> Option<IpcMsg> {
        if !self.wake_owner(process, endpoint, now_tick) {
            return None;
        }
        // Coalesce across all epochs of this owner. Failed replies/copies remain
        // eligible next tick, but cannot make receive spin within the same tick.
        let caller = process_identity(process);
        for remote in self.remote.iter_mut().flatten() {
            if owner_matches(remote.registration.owner, caller) {
                remote.last_wake_tick = Some(now_tick);
            }
        }
        Some(IpcMsg::with_label(CONTROL_WAKEUP))
    }
}

/// Call after ordinary queued messages, immediately before blocking, holding
/// SCHEDULER. The syscall has already authenticated endpoint receive ownership.
/// A hint neither consumes NEXT nor acknowledges any control obligation.
pub(in crate::ipc) fn take_control_wakeup(
    process: &crate::process::Process,
    endpoint: u32,
    now_tick: u64,
) -> Option<IpcMsg> {
    RUNTIME
        .lock()
        .take_control_wakeup(process, endpoint, now_tick)
}

fn wake_control_receivers_runtime(runtime: &Runtime, sched: &mut Scheduler, now_tick: u64) {
    if !runtime
        .remote
        .iter()
        .flatten()
        .any(|r| runtime.remote_ready(r, now_tick))
    {
        return;
    }
    for idx in 0..sched.processes.len() {
        let process = &sched.processes[idx];
        if process.state != ProcessState::BlockedOnIpc
            || process.pending_call.is_some()
            || process.ipc_reply_target.is_some()
        {
            continue;
        }
        if process
            .ipc_endpoint
            .is_some_and(|ep| runtime.wake_owner(process, ep, now_tick))
        {
            super::super::wake_terminal(sched, idx);
        }
    }
}

/// BSP tick only, holding SCHEDULER. Readiness survives queue saturation,
/// dropped hints and the running-to-blocked boundary. No idle polling wakeups.
pub(in crate::ipc) fn wake_control_receivers(sched: &mut Scheduler, now_tick: u64) {
    super::stat::wake_stat_callers(sched, now_tick);
    super::file::wake_file_callers(sched, now_tick);
    wake_control_receivers_runtime(&RUNTIME.lock(), sched, now_tick);
}

#[cfg(test)]
#[path = "wakeup_tests.rs"]
mod tests;
