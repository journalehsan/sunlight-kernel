//! Trusted spawn/exec snapshot. A strong binding reference fences release of the
//! source receipt after its caller exits, execs or changes root/cwd. No pathname
//! resolution or parent credentials are used to derive the child's objects.
use super::*;

#[derive(Clone)]
pub(crate) struct Inheritance {
    binding: Arc<NamespaceBinding>,
}

impl Runtime {
    pub(super) fn capture_inheritance(&self, identity: AdmissionIdentity) -> Option<Inheritance> {
        let binding = self.namespaces.lookup(identity).ok()?;
        let source = Inheritance { binding };
        self.source_receipt(&source)?;
        Some(source)
    }

    // The strong binding reference fences reclamation of this exact Bound row.
    // Keep only the reference in Preparing rather than duplicating the receipt
    // in every inline lease slot (registration construction uses a kernel stack).
    pub(super) fn source_receipt(&self, source: &Inheritance) -> Option<Receipt> {
        self.remote
            .iter()
            .flatten()
            .flat_map(|r| r.local.iter().flatten())
            .find_map(|l| match l {
                LocalLease::Bound { receipt, binding } if Arc::ptr_eq(binding, &source.binding) => {
                    Some(*receipt)
                }
                _ => None,
            })
    }

    pub(super) fn queue_inheritance(
        &mut self,
        source: Inheritance,
        identity: AdmissionIdentity,
        cwd: &str,
    ) -> Result<(), NamespaceRetentionError> {
        self.stage_inheritance(source, identity, cwd, None)
            .map(|_| ())
    }

    pub(super) fn stage_inheritance(
        &mut self,
        source: Inheritance,
        identity: AdmissionIdentity,
        cwd: &str,
        worker_started: Option<u64>,
    ) -> Result<u64, NamespaceRetentionError> {
        let receipt = self
            .source_receipt(&source)
            .ok_or(NamespaceRetentionError::Ownership(OwnershipError::State))?;
        if receipt.registration() != identity.registration
            || receipt.scope().caller == identity.caller
        {
            return Err(NamespaceRetentionError::Ownership(OwnershipError::State));
        }
        sunlight_ipc::vfs_rpc::retention::transport::encode_cwd(cwd)
            .map_err(|_| NamespaceRetentionError::Ownership(OwnershipError::State))?;
        let mut directory = alloc::string::String::new();
        directory
            .try_reserve_exact(cwd.len())
            .map_err(|_| NamespaceRetentionError::Ownership(OwnershipError::Capacity))?;
        directory.push_str(cwd);
        let id =
            self.queue_namespace_inner(identity, source.binding.grants(), Some(receipt.scope()))?;
        let staged = self
            .remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == identity.registration)
            .unwrap()
            .local
            .iter_mut()
            .flatten()
            .find(|lease| lease.id() == id)
            .unwrap();
        let LocalLease::Preparing {
            derivation,
            initial_cwd,
            worker_started: started,
            ..
        } = staged
        else {
            unreachable!()
        };
        *derivation = Some(source);
        *initial_cwd = Some(directory);
        *started = worker_started;
        Ok(id)
    }

    pub(super) fn authorize_derivation(
        &self,
        registration: Registration,
        bytes: &[u8],
        sched: &Scheduler,
    ) -> Result<Receipt, IpcError> {
        self.validate_derivation(registration, bytes, sched, false)
    }

    pub(super) fn validate_derivation(
        &self,
        registration: Registration,
        bytes: &[u8],
        sched: &Scheduler,
        detached_reply: bool,
    ) -> Result<Receipt, IpcError> {
        let Frame::Retain {
            registration: claimed,
            scope,
            request_id,
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
            .find(|r| r.registration == registration && !r.retired)
            .ok_or(IpcError::InvalidCapability)?;
        if remote.delivered != Some(request_id)
            || (!detached_reply && remote.callers.iter().flatten().any(CallerDelivery::pending))
        {
            return Err(IpcError::InvalidCapability);
        }
        let spawn = self.spawn_request(registration, request_id);
        if spawn.is_some_and(|call| !self.spawn_live(call, sched)) {
            return Err(IpcError::InvalidCapability);
        }
        remote
            .local
            .iter()
            .flatten()
            .find_map(|l| match l {
                LocalLease::Preparing {
                    id,
                    identity,
                    scope: expected,
                    derivation: Some(source),
                    initial_cwd: Some(cwd),
                    ..
                } if *id == request_id
                    && *expected == scope
                    && (spawn.is_some()
                        || sched.processes.iter().enumerate().any(|(idx, p)| {
                            process_identity(p) == identity.caller
                                && !p.is_exiting()
                                && {
                                    #[cfg(feature = "vfs_c2e_candidate")]
                                    let joining = is_shared_cwd_join(p, *identity, request_id);
                                    #[cfg(not(feature = "vfs_c2e_candidate"))]
                                    let joining = false;
                                    p.vfs_root_metadata || joining
                                }
                                && p.uid == identity.uid
                                && p.gid == identity.gid
                                && {
                                    let owner = sched.shared_process_index(idx);
                                    let parent = &sched.processes[owner];
                                    parent.cwd == *cwd
                                        && (idx == owner
                                            || (parent.vfs_downloads_cwd
                                                && !parent.is_exiting()
                                                && parent.address_space.identity().generation
                                                    == p.address_space.identity().generation
                                                && self
                                                    .capture_inheritance(AdmissionIdentity {
                                                        caller: process_identity(parent),
                                                        uid: parent.uid,
                                                        gid: parent.gid,
                                                        groups: [],
                                                        registration,
                                                    })
                                                    .is_some_and(|current| {
                                                        Arc::ptr_eq(
                                                            &current.binding,
                                                            &source.binding,
                                                        )
                                                    })))
                                }
                        })) =>
                {
                    self.source_receipt(source)
                }
                _ => None,
            })
            .ok_or(IpcError::InvalidCapability)
    }
}

/// Hold scheduler -> broker through capture. This is a snapshot of live admission,
/// never a way to restore a profile withdrawn before spawn/exec began.
pub(crate) fn capture_inheritance(
    idx: usize,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Option<Inheritance> {
    let process = &sched.processes[idx];
    if !process.vfs_root_metadata || process.is_exiting() {
        return None;
    }
    let mut runtime = RUNTIME.lock();
    let registration = live_registration(&mut runtime, sched, caps)?;
    let identity = AdmissionIdentity {
        caller: process_identity(process),
        uid: process.uid,
        gid: process.gid,
        groups: [],
        registration,
    };
    runtime.capture_inheritance(identity)
}

/// Before enqueueing the child or entering the replacement image. Failure keeps
/// sticky routing selected and admission closed; no later initial-root staging.
pub(crate) fn provision_inheritance(
    idx: usize,
    sched: &mut Scheduler,
    source: Option<Inheritance>,
) {
    let Some(source) = source else {
        return;
    };
    let process = &sched.processes[idx];
    let identity = AdmissionIdentity {
        caller: process_identity(process),
        uid: process.uid,
        gid: process.gid,
        groups: [],
        registration: source.binding.registration(),
    };
    let cwd = &sched.processes[sched.shared_process_index(idx)].cwd;
    let result = RUNTIME.lock().queue_inheritance(source, identity, cwd);
    sched.processes[idx].vfs_root_metadata = result.is_ok();
    sched.processes[idx].vfs_boot_root = false;
}

/// A worker created while cwd was legacy can join on its first service pathname
/// operation. The existing retention queue owns the receipt and compensation;
/// this record holds only the exact waiting task and its bounded continuation.
#[cfg(feature = "vfs_c2e_candidate")]
#[derive(Clone, Copy)]
pub(crate) struct SharedCwdJoin {
    caller: Caller,
    registration: Registration,
    request: u64,
    uid: u32,
    gid: u32,
    woken: bool,
}

#[cfg(feature = "vfs_c2e_candidate")]
impl Runtime {
    fn shared_join_preparing(&self, join: SharedCwdJoin) -> bool {
        self.registry.active() == Some(join.registration)
            && self.remote.iter().flatten().any(|remote| {
                remote.registration == join.registration
                    && remote.local.iter().flatten().any(|slot| {
                        matches!(slot, LocalLease::Preparing { id, identity, .. }
                        if *id == join.request && identity.caller == join.caller)
                    })
            })
    }

    pub(super) fn shared_join_busy(&self, owner: usize, sched: &Scheduler) -> bool {
        sched.processes.iter().enumerate().any(|(idx, process)| {
            process.vfs_shared_cwd_join.is_some_and(|join| {
                sched.shared_process_index(idx) == owner
                    && process_identity(process) == join.caller
                    && self.shared_join_preparing(join)
            })
        })
    }

    pub(super) fn join_shared_cwd(
        &mut self,
        idx: usize,
        sched: &mut Scheduler,
        caps: &CapabilityBroker,
    ) -> Result<bool, ()> {
        use crate::process::ProcessState;
        let owner_idx = sched.shared_process_index(idx);
        if idx == owner_idx {
            return Ok(true);
        }
        let caller = process_identity(&sched.processes[idx]);
        if let Some(join) = sched.processes[idx]
            .vfs_shared_cwd_join
            .take()
            .filter(|join| join.caller == caller)
        {
            let process = &sched.processes[idx];
            if (process.uid, process.gid) != (join.uid, join.gid) {
                self.retire_caller_profile(caller);
                sched.processes[idx].vfs_shared_cwd_join_allowed = false;
                return Err(());
            }
            if self.shared_join_preparing(join) {
                sched.processes[idx].vfs_shared_cwd_join = Some(join);
                sched.processes[idx].state = ProcessState::BlockedOnIpc;
                return Ok(false);
            }
            let process = &mut sched.processes[idx];
            let identity = AdmissionIdentity {
                caller,
                uid: process.uid,
                gid: process.gid,
                groups: [],
                registration: join.registration,
            };
            if live_registration(self, sched, caps) != Some(join.registration)
                || self.namespaces.lookup(identity).is_err()
            {
                return Err(());
            }
            sched.processes[idx].vfs_root_metadata = true;
            sched.processes[idx].vfs_shared_cwd_join_allowed = false;
            return Ok(true);
        }
        let process = &sched.processes[idx];
        let owner = &sched.processes[owner_idx];
        if process.vfs_root_metadata || !owner.vfs_downloads_cwd {
            return Ok(true);
        }
        if !process.vfs_shared_cwd_join_allowed
            || self.chdir_busy(idx, sched)
            || process.state != ProcessState::Running
            || process.pending_call.is_some()
            || process.ipc_reply_target.is_some()
            || owner.is_exiting()
            || !owner.vfs_root_metadata
            || owner.address_space.identity().generation
                != process.address_space.identity().generation
        {
            return Err(());
        }
        let registration = live_registration(self, sched, caps).ok_or(())?;
        let source = self
            .capture_inheritance(AdmissionIdentity {
                caller: process_identity(owner),
                uid: owner.uid,
                gid: owner.gid,
                groups: [],
                registration,
            })
            .ok_or(())?;
        let identity = AdmissionIdentity {
            caller,
            uid: process.uid,
            gid: process.gid,
            groups: [],
            registration,
        };
        let request = self
            .stage_inheritance(source, identity, &owner.cwd, Some(sched.global_tick))
            .map_err(|_| ())?;
        let process = &mut sched.processes[idx];
        process.vfs_shared_cwd_join = Some(SharedCwdJoin {
            caller,
            registration,
            request,
            uid: identity.uid,
            gid: identity.gid,
            woken: false,
        });
        process.state = ProcessState::BlockedOnIpc;
        Ok(false)
    }

    pub(super) fn poll_shared_cwd_joins(&mut self, sched: &mut Scheduler, now: u64) {
        // The budget stays with the retention record after its waiter exits
        // or withdraws. Unknown remote execution still needs epoch settlement.
        if let Some(registration) = self.registry.active().filter(|registration| {
            self.remote.iter().flatten().any(|remote| {
                remote.registration == *registration
                    && remote.local.iter().flatten().any(|lease| {
                        matches!(lease, LocalLease::Preparing { worker_started: Some(started), .. }
                        if now.wrapping_sub(*started) >= super::spawn::WAIT_TICKS)
                    })
            })
        }) {
            self.retire_registration(registration);
        }
        for idx in 0..sched.processes.len() {
            let process = &sched.processes[idx];
            let Some(join) = process.vfs_shared_cwd_join else {
                continue;
            };
            if process_identity(process) != join.caller || process.is_exiting() {
                sched.processes[idx].vfs_shared_cwd_join = None;
                continue;
            }
            if join.woken || self.shared_join_preparing(join) {
                continue;
            }
            if sched.processes[idx].pending_call.is_some()
                || sched.processes[idx].ipc_reply_target.is_some()
            {
                continue;
            }
            if sched.processes[idx].state == crate::process::ProcessState::BlockedOnIpc {
                super::super::wake_terminal(sched, idx);
            }
            sched.processes[idx]
                .vfs_shared_cwd_join
                .as_mut()
                .unwrap()
                .woken = true;
        }
    }
}

#[cfg(feature = "vfs_c2e_candidate")]
pub(super) fn is_shared_cwd_join(
    process: &crate::process::Process,
    identity: AdmissionIdentity,
    request: u64,
) -> bool {
    process.vfs_shared_cwd_join.is_some_and(|join| {
        join.caller == identity.caller
            && join.registration == identity.registration
            && join.request == request
            && (join.uid, join.gid) == (identity.uid, identity.gid)
    })
}
