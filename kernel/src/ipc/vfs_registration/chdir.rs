//! Selected-profile cwd and root transactions. Retention owns provisional pins; this
//! bounded continuation owns the captured path and the single commit decision.
use super::*;
use crate::process::ProcessState;
use alloc::string::String;
use sunlight_ipc::vfs_rpc::{
    chdir_transport as api, context::LOOKUP_METADATA, retention::wire::FRAME_BYTES,
};

pub(super) struct Call {
    pub(super) id: u64,
    pub(super) identity: AdmissionIdentity,
    pub(super) waiter: Caller,
    origin: (u64, u64),
    woken: bool,
    old: Scope,
    kind: api::Kind,
    path: String,
    cwd: String,
    started: u64,
    pub(super) result: Option<u64>,
}

fn string(text: &str) -> Result<String, stat::Error> {
    let mut s = String::new();
    s.try_reserve_exact(text.len())
        .map_err(|_| stat::Error::Capacity)?;
    s.push_str(text);
    Ok(s)
}

impl Runtime {
    /// All members of a shared process serialize against one directory transaction.
    pub(super) fn chdir_busy(&self, idx: usize, sched: &Scheduler) -> bool {
        let owner = sched.shared_process_index(idx);
        self.chdirs.iter().flatten().any(|c| {
            c.result.is_none()
                && sched.processes.iter().enumerate().any(|(i, p)| {
                    process_identity(p) == c.identity.caller
                        && sched.shared_process_index(i) == owner
                })
        })
    }

    pub(super) fn poll_chdirs(&mut self, sched: &mut Scheduler, now: u64) {
        for slot in 0..self.chdirs.len() {
            let Some(c) = self.chdirs[slot].as_ref() else {
                continue;
            };
            let idx = sched
                .processes
                .iter()
                .position(|p| process_identity(p) == c.waiter && !p.is_exiting());
            if c.result.is_none() && now.wrapping_sub(c.started) >= 5 * crate::timekeeping::TICK_HZ
            {
                let identity = c.identity;
                // Unknown remote execution is never replayed. Withdraw the
                // profile and settle the provisional lease through its owner.
                self.retire_registration(identity.registration);
                self.chdirs[slot].as_mut().unwrap().result = Some(110);
                #[cfg(not(test))]
                crate::serial_println!(
                    "[VFS-RPC] chdir timeout retain={} epoch={}",
                    self.chdirs[slot].as_ref().unwrap().id,
                    identity.registration.epoch
                );
                self.reconcile_namespaces();
            }
            let c = self.chdirs[slot].as_ref().unwrap();
            if c.result.is_some() && !c.woken {
                if let Some(idx) = idx {
                    let p = &sched.processes[idx];
                    if p.pending_call.is_some() || p.ipc_reply_target.is_some() {
                        continue;
                    }
                    if p.state == ProcessState::BlockedOnIpc {
                        super::super::wake_terminal(sched, idx);
                    }
                }
                self.chdirs[slot].as_mut().unwrap().woken = true;
            }
        }
    }

    fn transition(
        &self,
        registration: Registration,
        bytes: &[u8],
        sched: &Scheduler,
    ) -> Result<&Call, IpcError> {
        let Frame::Retain {
            registration: claimed,
            request_id,
            ..
        } = Frame::decode(bytes).map_err(|_| IpcError::InvalidCapability)?
        else {
            return Err(IpcError::InvalidCapability);
        };
        if claimed != registration || self.registry.active() != Some(registration) {
            return Err(IpcError::InvalidCapability);
        }
        let remote = self
            .remote
            .iter()
            .flatten()
            .find(|r| {
                r.registration == registration && !r.retired && r.delivered == Some(request_id)
            })
            .ok_or(IpcError::InvalidCapability)?;
        // Correlate the exact request bytes, not just an application chosen ID.
        let expected = remote
            .local
            .iter()
            .flatten()
            .find_map(|l| match l {
                LocalLease::Preparing { id, scope, .. } if *id == request_id => Some(*scope),
                _ => None,
            })
            .ok_or(IpcError::InvalidCapability)?;
        if (Frame::Retain {
            registration,
            scope: expected,
            request_id,
        })
        .encode()
        .map_err(|_| IpcError::InvalidCapability)?
        .as_slice()
            != bytes
        {
            return Err(IpcError::InvalidCapability);
        }
        let c = self
            .chdirs
            .iter()
            .flatten()
            .find(|c| {
                c.id == request_id && c.identity.registration == registration && c.result.is_none()
            })
            .ok_or(IpcError::InvalidCapability)?;
        let valid = sched.processes.iter().enumerate().any(|(idx, p)| {
            process_identity(p) == c.identity.caller
                && !p.is_exiting()
                && p.vfs_root_metadata
                && (c.kind != api::Kind::Root || p.uid == 0)
                && p.uid == c.identity.uid
                && p.gid == c.identity.gid
                && sched.processes[sched.shared_process_index(idx)].cwd == c.cwd
        });
        let waiter_valid = sched.processes.iter().enumerate().any(|(idx, p)| {
            process_identity(p) == c.waiter
                && !p.is_exiting()
                && p.uid == c.identity.uid
                && p.gid == c.identity.gid
                && process_identity(&sched.processes[sched.shared_process_index(idx)])
                    == c.identity.caller
        });
        if !valid || !waiter_valid || self.namespaces.lookup(c.identity).is_err() {
            return Err(IpcError::InvalidCapability);
        }
        Ok(c)
    }
}

/// None owns a parked operation; Some(errno) is its terminal delivery.
pub(crate) fn resume(
    pid: usize,
    origin: (u64, u64),
    kind: api::Kind,
    sched: &mut Scheduler,
) -> Result<Option<Option<u64>>, stat::Error> {
    resume_runtime(&mut RUNTIME.lock(), pid, origin, kind, sched)
}

pub(super) fn resume_runtime(
    runtime: &mut Runtime,
    pid: usize,
    origin: (u64, u64),
    kind: api::Kind,
    sched: &mut Scheduler,
) -> Result<Option<Option<u64>>, stat::Error> {
    runtime.poll_chdirs(sched, sched.global_tick);
    let idx = sched
        .processes
        .iter()
        .position(|p| p.pid == pid && !p.is_exiting())
        .ok_or(stat::Error::Identity)?;
    let waiter = process_identity(&sched.processes[idx]);
    let Some(slot) = runtime
        .chdirs
        .iter()
        .position(|c| c.as_ref().is_some_and(|c| c.waiter == waiter))
    else {
        return Ok(None);
    };
    let c = runtime.chdirs[slot].as_ref().unwrap();
    if c.origin != origin
        || c.kind != kind
        || sched.processes[idx].pending_call.is_some()
        || sched.processes[idx].ipc_reply_target.is_some()
    {
        #[cfg(not(test))]
        crate::serial_println!(
            "[VFS-RPC] chdir resume refused caller={} origin={:?} expected={:?}",
            pid,
            origin,
            c.origin
        );
        return Err(stat::Error::Busy);
    }
    if let Some(errno) = c.result {
        runtime.chdirs[slot] = None;
        return Ok(Some(Some(errno)));
    }
    sched.processes[idx].state = ProcessState::BlockedOnIpc;
    Ok(Some(None))
}

pub(crate) fn chdir_call(
    pid: usize,
    path: &str,
    origin: (u64, u64),
    kind: api::Kind,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<Option<u64>, stat::Error> {
    let mut runtime = RUNTIME.lock();
    let result = transition_call_runtime(&mut runtime, pid, path, kind, sched, caps);
    #[cfg(not(test))]
    if let Err(error) = &result {
        crate::serial_println!(
            "[VFS-RPC] chdir admission refused caller={} error={:?} state={:?}",
            pid,
            error,
            sched
                .processes
                .iter()
                .find(|p| p.pid == pid)
                .map(|p| p.state)
        );
    }
    if matches!(result, Ok(None)) {
        if let Some(c) = runtime
            .chdirs
            .iter_mut()
            .flatten()
            .find(|c| c.waiter.pid == pid as u64)
        {
            c.origin = origin;
        }
    }
    result
}

#[cfg(test)]
pub(super) fn chdir_call_runtime(
    runtime: &mut Runtime,
    pid: usize,
    path: &str,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<Option<u64>, stat::Error> {
    transition_call_runtime(runtime, pid, path, api::Kind::Directory, sched, caps)
}

pub(super) fn transition_call_runtime(
    runtime: &mut Runtime,
    pid: usize,
    path: &str,
    kind: api::Kind,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<Option<u64>, stat::Error> {
    let idx = sched
        .processes
        .iter()
        .position(|p| p.pid == pid && !p.is_exiting())
        .ok_or(stat::Error::Identity)?;
    let waiter = process_identity(&sched.processes[idx]);
    let owner = sched.shared_process_index(idx);
    if !sched.processes[owner].vfs_root_metadata {
        return Err(stat::Error::Identity);
    }
    if kind == api::Kind::Root && (sched.processes[owner].uid != 0 || sched.processes[idx].uid != 0)
    {
        return Ok(Some(13));
    }
    let caller = process_identity(&sched.processes[owner]);
    runtime.poll_chdirs(sched, sched.global_tick);
    if sched.processes[idx].pending_call.is_some()
        || sched.processes[idx].ipc_reply_target.is_some()
    {
        return Err(stat::Error::Busy);
    }
    if let Some(slot) = runtime
        .chdirs
        .iter()
        .position(|c| c.as_ref().is_some_and(|c| c.waiter == waiter))
    {
        let c = runtime.chdirs[slot].as_ref().unwrap();
        if c.path != path || c.kind != kind {
            return Err(stat::Error::Busy);
        }
        if let Some(errno) = c.result {
            runtime.chdirs[slot] = None;
            return Ok(Some(errno));
        }
        sched.processes[idx].state = ProcessState::BlockedOnIpc;
        return Ok(None);
    }
    // A wake can mark the active caller Ready before its syscall return
    // finishes. Syscall origin and lifecycle identity still select that caller.
    if runtime.chdir_busy(idx, sched)
        || !matches!(
            sched.processes[idx].state,
            ProcessState::Running | ProcessState::Ready
        )
    {
        return Err(stat::Error::Busy);
    }
    // A scope transition cannot race a sibling's copied metadata continuation.
    let owner = sched.shared_process_index(idx);
    if runtime.publications.iter().flatten().any(|c| {
        sched.processes.iter().enumerate().any(|(i, p)| {
            process_identity(p) == c.identity.caller && sched.shared_process_index(i) == owner
        })
    }) {
        return Err(stat::Error::Busy);
    }
    let slot = runtime
        .chdirs
        .iter()
        .position(Option::is_none)
        .ok_or(stat::Error::Capacity)?;
    let identity = capture_locked(runtime, sched.processes[owner].pid, sched, caps)?;
    if sched.processes[idx].uid != identity.uid || sched.processes[idx].gid != identity.gid {
        return Err(stat::Error::Identity);
    }
    let n = runtime
        .namespaces
        .lookup(identity)
        .and_then(|b| b.validate(identity))
        .map_err(|_| stat::Error::Prepare)?;
    let old = Scope {
        caller,
        namespace_id: n.id,
        namespace_generation: n.generation,
    };
    let path = string(path)?;
    api::encode_transition(old, identity.uid, identity.gid, &path, kind)
        .map_err(|_| stat::Error::Protocol)?;
    let cwd = string(&sched.processes[owner].cwd)?;
    runtime
        .queue_namespace_inner(identity, LOOKUP_METADATA, Some(old))
        .map_err(|_| stat::Error::Capacity)?;
    let id = runtime
        .remote
        .iter()
        .flatten()
        .find(|r| r.registration == identity.registration)
        .unwrap()
        .local
        .iter()
        .flatten()
        .map(LocalLease::id)
        .max()
        .unwrap();
    runtime.chdirs[slot] = Some(Call {
        id,
        identity,
        waiter,
        origin: (0, 0),
        woken: false,
        old,
        kind,
        path,
        cwd,
        started: sched.global_tick,
        result: None,
    });
    // Preserve service routing if the shared owner exits before this worker
    // resumes. A withdrawn namespace must never resume in the kernel resolver.
    #[cfg(not(test))]
    {
        sched.processes[idx].vfs_stat_routed = true;
    }
    sched.processes[idx].state = ProcessState::BlockedOnIpc;
    #[cfg(not(test))]
    crate::serial_println!(
        "[VFS-RPC] chdir parked caller={} waiter={} retain={} epoch={}",
        identity.caller.pid,
        waiter.pid,
        id,
        identity.registration.epoch
    );
    Ok(None)
}

// Capture while already holding RUNTIME; public capture takes that lock itself.
fn capture_locked(
    runtime: &mut Runtime,
    pid: usize,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<AdmissionIdentity, stat::Error> {
    let registration = live_registration(runtime, sched, caps).ok_or(stat::Error::Identity)?;
    let p = sched
        .processes
        .iter()
        .find(|p| p.pid == pid && !p.is_exiting())
        .ok_or(stat::Error::Identity)?;
    Ok(AdmissionIdentity {
        caller: process_identity(p),
        uid: p.uid,
        gid: p.gid,
        groups: [],
        registration,
    })
}

pub(crate) fn context(
    pid: usize,
    epoch: u64,
    input: &[u8],
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<[u8; api::CONTEXT_BYTES], IpcError> {
    let result = context_runtime(&mut RUNTIME.lock(), pid, epoch, input, sched, caps);
    #[cfg(not(test))]
    if result.is_ok() {
        crate::serial_println!(
            "[VFS-RPC] chdir context copied service={} epoch={}",
            pid,
            epoch
        );
    }
    result
}

pub(super) fn context_runtime(
    runtime: &mut Runtime,
    pid: usize,
    epoch: u64,
    input: &[u8],
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<[u8; api::CONTEXT_BYTES], IpcError> {
    let registration = super::transport::authenticate(runtime, pid, epoch, sched, caps)?;
    let c = runtime.transition(registration, input, sched)?;
    api::encode_transition(c.old, c.identity.uid, c.identity.gid, &c.path, c.kind)
        .map_err(|_| IpcError::InvalidCapability)
}

pub(crate) fn finish(
    pid: usize,
    epoch: u64,
    input: &[u8; api::FINISH_BYTES],
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<(), IpcError> {
    let result = finish_runtime(&mut RUNTIME.lock(), pid, epoch, input, sched, caps);
    #[cfg(not(test))]
    crate::serial_println!(
        "[VFS-RPC] chdir commit validated={} errno={} epoch={}",
        result.is_ok(),
        u64::from_le_bytes(input[FRAME_BYTES..FRAME_BYTES + 8].try_into().unwrap()),
        epoch
    );
    result
}

pub(super) fn finish_runtime(
    runtime: &mut Runtime,
    pid: usize,
    epoch: u64,
    input: &[u8; api::FINISH_BYTES],
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<(), IpcError> {
    use sunlight_ipc::vfs_rpc::retention::transport::{decode_cwd, CWD_BYTES};
    let frame = Frame::decode(&input[..FRAME_BYTES]).map_err(|_| IpcError::InvalidCapability)?;
    let errno = u64::from_le_bytes(input[FRAME_BYTES..FRAME_BYTES + 8].try_into().unwrap());
    let cwd: &[u8; CWD_BYTES] = input[FRAME_BYTES + 8..].try_into().unwrap();
    let cwd = decode_cwd(cwd).map_err(|_| IpcError::InvalidCapability)?;
    let (registration, id, scope) = match frame {
        Frame::Retained(r) if errno == 0 => (r.registration(), r.request_id(), r.scope()),
        Frame::Refused { request, .. } if (1..=4095).contains(&errno) => {
            let Frame::Retain {
                registration,
                request_id,
                scope,
            } = request.request()
            else {
                unreachable!()
            };
            (registration, request_id, scope)
        }
        _ => return Err(IpcError::InvalidCapability),
    };
    runtime.poll_chdirs(sched, sched.global_tick);
    let authenticated = super::transport::authenticate(runtime, pid, epoch, sched, caps)?;
    if authenticated != registration {
        return Err(IpcError::InvalidCapability);
    }
    let retain = Frame::Retain {
        registration,
        request_id: id,
        scope,
    }
    .encode()
    .map_err(|_| IpcError::InvalidCapability)?;
    let c = match runtime.transition(authenticated, &retain, sched) {
        Ok(c) => c,
        Err(_) => {
            // A late authenticated success still owns compensation, even if
            // timeout/exit already discarded the syscall continuation.
            let settled = match frame {
                Frame::Retained(_) => runtime
                    .install_retained(authenticated, id, &input[..FRAME_BYTES], None)
                    .is_ok(),
                Frame::Refused { .. } => runtime
                    .refused(authenticated, id, &input[..FRAME_BYTES])
                    .is_ok(),
                _ => false,
            };
            if settled {
                if let Some(remote) = runtime
                    .remote
                    .iter_mut()
                    .flatten()
                    .find(|r| r.registration == authenticated && r.delivered == Some(id))
                {
                    remote.delivered = None;
                }
            }
            return Err(IpcError::InvalidCapability);
        }
    };
    if errno == 0 && c.kind == api::Kind::Root && cwd != "/" {
        return Err(IpcError::InvalidCapability);
    }
    let identity = c.identity;
    let slot = runtime
        .chdirs
        .iter()
        .position(|c| {
            c.as_ref()
                .is_some_and(|c| c.id == id && c.identity == identity)
        })
        .unwrap();
    if errno != 0 {
        runtime
            .refused(authenticated, id, &input[..FRAME_BYTES])
            .map_err(|_| IpcError::InvalidCapability)?;
    } else {
        // All allocations and structural checks precede cwd mutation.
        let cwd = string(cwd).map_err(|_| IpcError::InvalidCapability)?;
        runtime
            .namespaces
            .withdraw(identity)
            .map_err(|_| IpcError::InvalidCapability)?;
        let installed =
            runtime.install_retained(authenticated, id, &input[..FRAME_BYTES], Some(identity));
        if !matches!(installed, Ok(None)) {
            runtime.chdirs[slot].as_mut().unwrap().result = Some(5);
            runtime.poll_chdirs(sched, sched.global_tick);
            return Err(IpcError::InvalidCapability);
        }
        let idx = sched
            .processes
            .iter()
            .position(|p| process_identity(p) == identity.caller)
            .unwrap();
        let owner = sched.shared_process_index(idx);
        sched.processes[owner].cwd = cwd;
        sched.processes[owner].vfs_boot_root = false;
        // Sibling profiles refer to the previous cwd. Revoke them atomically
        // with the shared-process commit; sticky routing prevents fallback.
        for i in 0..sched.processes.len() {
            if i != idx && sched.shared_process_index(i) == owner {
                runtime.withdraw_root_profile(i, sched);
            }
        }
        runtime.reconcile_namespaces();
    }
    if let Some(remote) = runtime
        .remote
        .iter_mut()
        .flatten()
        .find(|r| r.registration == authenticated)
    {
        remote.delivered = None;
    }
    runtime.chdirs[slot].as_mut().unwrap().result = Some(errno);
    runtime.poll_chdirs(sched, sched.global_tick);
    Ok(())
}
