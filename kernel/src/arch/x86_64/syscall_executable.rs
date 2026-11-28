//! Restartable source acquisition shared by native spawn and native/Linux exec.
use super::{sched, SyscallFrame};
use crate::capability::CAP_BROKER;
use crate::ipc::vfs_registration::executable::{self, Source};
use alloc::{string::String, vec::Vec};
use sunlight_ipc::vfs_rpc::lifecycle::Caller;

static PENDING: spin::Mutex<Option<(Caller, bool, Source)>> = spin::Mutex::new(None);

pub(crate) fn retire(caller: Caller) -> bool {
    let mut pending = PENDING.lock();
    if pending
        .as_ref()
        .is_some_and(|(owner, _, _)| *owner == caller)
    {
        return pending.take().is_some();
    }
    false
}

pub(super) fn abandon_if_different_syscall(number: u64) {
    if PENDING
        .lock()
        .as_ref()
        .is_none_or(|(_, spawn, _)| number == if *spawn { 39 } else { 31 })
    {
        return;
    }
    let sched = sched::SCHEDULER.lock();
    let caller = crate::ipc::vfs_process_identity(sched.current_process());
    if retire(caller) {
        executable::settle(caller);
    }
}

pub(super) struct Acquisition {
    caller: Caller,
    spawn: bool,
    parked: bool,
    authority: Option<executable::Authority>,
}

impl Acquisition {
    pub(super) fn new(spawn: bool) -> Self {
        Self {
            caller: sched::with_scheduler(|s| {
                crate::ipc::vfs_process_identity(s.current_process())
            }),
            spawn,
            parked: false,
            authority: None,
        }
    }

    pub(super) fn step(&mut self, path: &str) -> Option<Result<Option<Vec<u8>>, ()>> {
        let mut sched = sched::SCHEDULER.lock();
        let process = sched.current_process();
        let shared = sched.current_shared_process();
        let route = executable::route(path, &shared.cwd, shared.vfs_downloads_cwd);
        let mut pending = PENDING.lock();
        let replay = pending
            .as_ref()
            .is_some_and(|(caller, _, _)| *caller == self.caller);
        if !replay && route.is_none() {
            return None;
        }
        let Some(path) = route else {
            return Some(Err(()));
        };
        if (!path.starts_with('/') && process.pid != shared.pid)
            || process.vfs_root_rebased
            || shared.vfs_root_rebased
            || process.is_exiting()
            || process.foreground.cancel != 0
        {
            return Some(Err(()));
        }
        if replay && process.state == crate::process::ProcessState::Ready {
            return Some(Ok(None));
        }
        if !replay && pending.is_some() {
            return Some(Err(()));
        }
        let caps = CAP_BROKER.lock();
        let pid = sched.current_pid();
        let Some(authority) = executable::admission(pid, &sched, &caps) else {
            return Some(Err(()));
        };
        let mut source = if replay {
            let (_, spawn, source) = pending.as_ref().unwrap();
            if *spawn != self.spawn || source.path != path || source.authority != authority {
                return Some(Err(()));
            }
            pending.take().unwrap().2
        } else {
            Source::new(String::from(path), authority)
        };
        let result = source.step(|operation, descriptor| {
            crate::ipc::vfs_file_call(
                pid,
                operation,
                descriptor,
                &mut sched,
                &caps,
                |_, token, _| Ok(token),
            )
        });
        if matches!(result, Ok(None)) {
            *pending = Some((self.caller, self.spawn, source));
            sched.account_and_apply_churn_penalty();
            let tick = sched.global_tick;
            if let Some(idx) = sched.current_process_index() {
                sched.processes[idx].block_start_tick = tick;
                sched.remove_from_ready_queues(idx);
            }
        } else {
            executable::settle(self.caller);
            self.authority = Some(authority);
        }
        Some(result.map_err(|_| ()))
    }

    pub(super) fn park(&mut self, frame: &mut SyscallFrame) -> u64 {
        self.parked = true;
        super::park_ordinary_spawn(frame)
    }

    pub(super) fn live(&self, sched: &sched::Scheduler) -> bool {
        self.authority.is_none_or(|authority| {
            let process = sched.current_process();
            !process.is_exiting()
                && process.foreground.cancel == 0
                && !process.vfs_root_rebased
                && !sched.current_shared_process().vfs_root_rebased
                && executable::admission(sched.current_pid(), sched, &CAP_BROKER.lock())
                    == Some(authority)
        })
    }
}

impl Drop for Acquisition {
    fn drop(&mut self) {
        if !self.parked
            && PENDING
                .lock()
                .as_ref()
                .is_some_and(|(owner, _, _)| *owner == self.caller)
        {
            let _sched = sched::SCHEDULER.lock();
            if retire(self.caller) {
                executable::settle(self.caller);
            }
        }
    }
}
