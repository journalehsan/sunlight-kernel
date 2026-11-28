//! Authenticated cwd and root transactions. Retention owns provisional pins; this
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
    previous: Option<Scope>,
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
    ) -> Result<(&Call, Scope), IpcError> {
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
                && (p.vfs_root_metadata || c.kind == api::Kind::EnterDownloads)
                && (c.kind != api::Kind::Downloads || (p.vfs_downloads_cwd && !p.vfs_root_rebased))
                && (c.kind != api::Kind::EnterDownloads
                    || (!p.vfs_downloads_cwd && !p.vfs_root_rebased && !p.vfs_stat_routed))
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
        let current = self
            .namespaces
            .lookup(c.identity)
            .and_then(|binding| binding.validate(c.identity))
            .ok()
            .map(|n| Scope {
                caller: c.identity.caller,
                namespace_id: n.id,
                namespace_generation: n.generation,
            });
        if !valid || !waiter_valid || current != c.previous {
            return Err(IpcError::InvalidCapability);
        }
        Ok((c, expected))
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
        || (c.kind != kind
            && !(kind == api::Kind::Directory
                && matches!(c.kind, api::Kind::EnterDownloads | api::Kind::Downloads)))
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

pub(crate) enum DirectoryRoute<'a> {
    Legacy(String),
    Service(api::Kind, alloc::borrow::Cow<'a, str>),
}

/// Common native chdir route, also reached by the Linux syscall translation.
pub(crate) fn route_directory<'a, E>(
    sched: &Scheduler,
    idx: usize,
    path: &'a str,
    legacy: impl FnOnce() -> Result<sunlight_fs::vfs::LegacyCwd, E>,
) -> Result<DirectoryRoute<'a>, E> {
    use alloc::borrow::Cow;
    let owner = &sched.processes[sched.shared_process_index(idx)];
    let current = &sched.processes[idx];
    if owner.vfs_downloads_cwd {
        return Ok(DirectoryRoute::Service(
            api::Kind::Downloads,
            Cow::Borrowed(path),
        ));
    }
    if current.vfs_stat_routed || owner.vfs_stat_routed {
        return Ok(DirectoryRoute::Service(
            api::Kind::Directory,
            Cow::Borrowed(path),
        ));
    }
    match legacy()? {
        sunlight_fs::vfs::LegacyCwd::Directory(resolved) => {
            Ok(DirectoryRoute::Legacy(resolved.path))
        }
        sunlight_fs::vfs::LegacyCwd::Downloads(target) => Ok(DirectoryRoute::Service(
            api::Kind::EnterDownloads,
            Cow::Owned(target),
        )),
    }
}

pub(crate) fn busy(idx: usize, sched: &Scheduler) -> bool {
    RUNTIME.lock().chdir_busy(idx, sched)
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
    if !sched.processes[owner].vfs_root_metadata && kind != api::Kind::EnterDownloads {
        return Err(stat::Error::Identity);
    }
    if kind == api::Kind::Root && (sched.processes[owner].uid != 0 || sched.processes[idx].uid != 0)
    {
        return Ok(Some(13));
    }
    #[cfg(feature = "vfs_c2e_candidate")]
    if kind == api::Kind::Downloads
        && !runtime.join_shared_cwd(idx, sched, caps).map_err(|_| stat::Error::Identity)?
    {
        return Ok(None);
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
    #[cfg(feature = "vfs_c2e_candidate")]
    if runtime.shared_join_busy(owner, sched) {
        return Err(stat::Error::Busy);
    }
    if runtime.shared_worker_publication(caller) || runtime.group_retained_file_busy(owner, sched) || runtime.publications.iter().flatten().any(|c| {
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
    let previous = runtime
        .namespaces
        .lookup(identity)
        .and_then(|b| b.validate(identity))
        .ok()
        .map(|n| Scope {
            caller,
            namespace_id: n.id,
            namespace_generation: n.generation,
        });
    if previous.is_none() && kind != api::Kind::EnterDownloads {
        return Err(stat::Error::Prepare);
    }
    if cfg!(feature = "vfs_c2e_candidate") && idx != owner
        && matches!(kind, api::Kind::Directory | api::Kind::Downloads)
    {
        let worker = &sched.processes[idx];
        let worker_identity = AdmissionIdentity {
            caller: waiter, uid: worker.uid, gid: worker.gid,
            groups: [], registration: identity.registration,
        };
        let shared = runtime.namespaces.lookup(worker_identity)
            .map_err(|_| stat::Error::Identity)?;
        if !previous.is_some_and(|scope| {
            shared.shared_directory_generation() == scope.namespace_generation
        }) {
            return Err(stat::Error::Identity);
        }
    }
    if matches!(kind, api::Kind::EnterDownloads | api::Kind::Downloads) {
        let process = &sched.processes[owner];
        if !cfg!(feature = "vfs_c2e_candidate")
            || process.vfs_root_rebased
            || process.vfs_downloads_cwd != (kind == api::Kind::Downloads)
            || (kind == api::Kind::EnterDownloads
                && (!sunlight_fs::path::service_downloads(path) || process.vfs_stat_routed))
        {
            return Err(stat::Error::Identity);
        }
    }
    let path = string(path)?;
    // Validate the path before queuing. A new entry uses its own provisional
    // scope in the context when there is no previous service binding.
    let placeholder = Scope {
        caller,
        namespace_id: 1,
        namespace_generation: 1,
    };
    api::encode_transition(
        previous.unwrap_or(placeholder),
        identity.uid,
        identity.gid,
        &path,
        kind,
    )
    .map_err(|_| stat::Error::Protocol)?;
    let cwd = string(&sched.processes[owner].cwd)?;
    let id = runtime
        .queue_namespace_inner(identity, LOOKUP_METADATA, previous)
        .map_err(|_| stat::Error::Capacity)?;
    runtime.chdirs[slot] = Some(Call {
        id,
        identity,
        waiter,
        origin: (0, 0),
        woken: false,
        previous,
        kind,
        path,
        cwd,
        started: sched.global_tick,
        result: None,
    });
    // Preserve service routing if the shared owner exits before this worker
    // resumes. A withdrawn namespace must never resume in the kernel resolver.
    #[cfg(not(test))]
    if matches!(kind, api::Kind::Directory | api::Kind::Root) {
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
    let (c, next) = runtime.transition(registration, input, sched)?;
    api::encode_transition(
        c.previous.unwrap_or(next),
        c.identity.uid,
        c.identity.gid,
        &c.path,
        c.kind,
    )
    .map_err(|_| IpcError::InvalidCapability)
}

pub(crate) type LegacyCwdCommit<'a> = dyn FnMut(&mut Scheduler, String) -> Result<(), u64> + 'a;

pub(crate) fn finish(
    pid: usize,
    epoch: u64,
    input: &[u8; api::FINISH_BYTES],
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
    legacy: impl FnOnce(&mut Scheduler, usize, &str, &mut LegacyCwdCommit<'_>) -> Result<(), u64>,
) -> Result<(), IpcError> {
    let result = finish_with_legacy(&mut RUNTIME.lock(), pid, epoch, input, sched, caps, legacy);
    #[cfg(not(test))]
    crate::serial_println!(
        "[VFS-RPC] chdir commit validated={} errno={} epoch={}",
        result.is_ok(),
        u64::from_le_bytes(input[FRAME_BYTES..FRAME_BYTES + 8].try_into().unwrap()),
        epoch
    );
    result
}

#[cfg(test)]
pub(super) fn finish_runtime(
    runtime: &mut Runtime,
    pid: usize,
    epoch: u64,
    input: &[u8; api::FINISH_BYTES],
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> Result<(), IpcError> {
    finish_with_legacy(runtime, pid, epoch, input, sched, caps, |_, _, _, _| Err(5))
}

pub(super) fn finish_with_legacy(
    runtime: &mut Runtime,
    pid: usize,
    epoch: u64,
    input: &[u8; api::FINISH_BYTES],
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
    legacy: impl FnOnce(&mut Scheduler, usize, &str, &mut LegacyCwdCommit<'_>) -> Result<(), u64>,
) -> Result<(), IpcError> {
    use sunlight_ipc::vfs_rpc::retention::transport::{decode_cwd, CWD_BYTES};
    let frame = Frame::decode(&input[..FRAME_BYTES]).map_err(|_| IpcError::InvalidCapability)?;
    let errno = u64::from_le_bytes(input[FRAME_BYTES..FRAME_BYTES + 8].try_into().unwrap());
    let cwd: &[u8; CWD_BYTES] = input[FRAME_BYTES + 8..].try_into().unwrap();
    let cwd = decode_cwd(cwd).map_err(|_| IpcError::InvalidCapability)?;
    let (registration, id, scope) = match frame {
        Frame::Retained(r) if errno == 0 || errno == api::LEGACY_BOUNDARY => {
            (r.registration(), r.request_id(), r.scope())
        }
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
        Ok((c, _)) => c,
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
                if let Some(call) = runtime.chdirs.iter_mut().flatten().find(|call| {
                    call.id == id
                        && call.identity.registration == authenticated
                        && call.result.is_none()
                }) {
                    call.result = Some(5);
                    runtime.poll_chdirs(sched, sched.global_tick);
                }
            }
            return Err(IpcError::InvalidCapability);
        }
    };
    let legacy_boundary_valid =
        if c.path.starts_with('/') && !sunlight_fs::path::service_downloads(&c.path) {
            cwd == c.path
        } else {
            sunlight_fs::path::strip_mount(cwd, "/root").is_some()
        };
    let previous = c.previous;
    let kind = c.kind;
    if kind == api::Kind::Root && errno == 0 && cwd != "/" {
        return Err(IpcError::InvalidCapability);
    }
    let waiter = c.waiter;
    let changes_root = kind == api::Kind::Root;
    let downloads = matches!(kind, api::Kind::EnterDownloads | api::Kind::Downloads);
    let mut terminal = errno;
    let mut accepted = true;
    let identity = c.identity;
    let slot = runtime
        .chdirs
        .iter()
        .position(|c| {
            c.as_ref()
                .is_some_and(|c| c.id == id && c.identity == identity)
        })
        .unwrap();
    let owner = sched
        .processes
        .iter()
        .position(|p| process_identity(p) == identity.caller)
        .unwrap();
    if let Frame::Refused { .. } = frame {
        runtime
            .refused(authenticated, id, &input[..FRAME_BYTES])
            .map_err(|_| IpcError::InvalidCapability)?;
    } else {
        let leaving = errno == api::LEGACY_BOUNDARY;
        let entering_or_staying = downloads && !leaving;
        let mut receipt_settled = false;
        let mut publish = |sched: &mut Scheduler, path| {
            receipt_settled = true;
            match runtime.install_transition_receipt(
                authenticated,
                id,
                &input[..FRAME_BYTES],
                Some(identity),
                previous,
            ) {
                Ok(None) => {}
                _ => return Err(5),
            }
            let process = &mut sched.processes[owner];
            process.cwd = path;
            process.vfs_boot_root = false;
            if downloads {
                process.vfs_stat_routed = entering_or_staying;
                process.vfs_root_metadata = true;
            }
            if changes_root {
                process.vfs_root_rebased = true;
            }
            for i in 0..sched.processes.len() {
                if sched.shared_process_index(i) == owner {
                    if downloads || changes_root {
                        sched.processes[i].vfs_downloads_cwd = entering_or_staying;
                    }
                    if cfg!(feature = "vfs_c2e_candidate") && i != owner && !changes_root {
                        let worker = &sched.processes[i];
                        let identity = AdmissionIdentity {
                            caller: process_identity(worker),
                            uid: worker.uid,
                            gid: worker.gid,
                            groups: [],
                            registration: authenticated,
                        };
                        if let Ok(binding) = runtime.namespaces.lookup(identity) {
                            if binding.shared_directory_generation() != 0 {
                                binding.set_shared_directory_generation(scope.namespace_generation);
                            }
                        }
                    }
                    if downloads && i != owner {
                        // The shared owner selects service routing while inside
                        // Downloads. Clear a worker's sticky route only on a
                        // successfully authenticated transition back to legacy.
                        sched.processes[i].vfs_stat_routed = entering_or_staying;
                    }
                }
            }
            runtime.reconcile_namespaces();
            Ok(())
        };

        let result = if leaving {
            let waiter = sched
                .processes
                .iter()
                .position(|p| process_identity(p) == waiter)
                .unwrap();
            if kind == api::Kind::Downloads && legacy_boundary_valid {
                legacy(sched, waiter, cwd, &mut publish)
            } else {
                Err(5)
            }
        } else if downloads && !sunlight_fs::path::service_downloads(cwd) {
            Err(5)
        } else {
            string(cwd)
                .map_err(|_| 12u64)
                .and_then(|path| publish(sched, path))
        };
        terminal = match result {
            Ok(()) => 0,
            Err(error) => {
                if !receipt_settled {
                    // The legacy resolver may fail before publication. Settle
                    // the authenticated provisional retain without changing admission.
                    let _ = runtime.install_transition_receipt(
                        authenticated,
                        id,
                        &input[..FRAME_BYTES],
                        None,
                        previous,
                    );
                }
                accepted = false;
                error
            }
        };
    }
    if let Some(remote) = runtime
        .remote
        .iter_mut()
        .flatten()
        .find(|r| r.registration == authenticated)
    {
        remote.delivered = None;
    }
    let mut completed = runtime.chdirs[slot].take().unwrap();
    completed.result = Some(terminal);
    if terminal == 0 && matches!(frame, Frame::Retained(_)) {
        for idx in 0..sched.processes.len() {
            if idx != owner && sched.shared_process_index(idx) == owner
                && (!cfg!(feature = "vfs_c2e_candidate") || changes_root)
            {
                runtime.withdraw_root_profile(idx, sched);
            }
        }
    }
    runtime.chdirs[slot] = Some(completed);
    runtime.poll_chdirs(sched, sched.global_tick);
    if accepted {
        Ok(())
    } else {
        Err(IpcError::InvalidCapability)
    }
}
