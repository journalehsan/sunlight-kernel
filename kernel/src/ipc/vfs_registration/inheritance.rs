//! Trusted spawn/exec snapshot. A strong binding reference fences release of the
//! source receipt after its caller exits, execs or changes root/cwd. No pathname
//! resolution or parent credentials are used to derive the child's objects.
use super::*;

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
    fn source_receipt(&self, source: &Inheritance) -> Option<Receipt> {
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
        self.queue_namespace_inner(identity, source.binding.grants(), Some(receipt.scope()))?;
        let latest = self
            .remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == identity.registration)
            .unwrap()
            .local
            .iter_mut()
            .flatten()
            .max_by_key(|l| l.id())
            .unwrap();
        let LocalLease::Preparing {
            derivation,
            initial_cwd,
            ..
        } = latest
        else {
            unreachable!()
        };
        *derivation = Some(source);
        *initial_cwd = Some(directory);
        Ok(())
    }

    pub(super) fn authorize_derivation(
        &self,
        registration: Registration,
        bytes: &[u8],
        sched: &Scheduler,
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
            || remote.callers.iter().flatten().any(CallerDelivery::pending)
        {
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
                    && sched.processes.iter().enumerate().any(|(idx, p)| {
                        process_identity(p) == identity.caller
                            && !p.is_exiting()
                            && p.vfs_root_metadata
                            && p.uid == identity.uid
                            && p.gid == identity.gid
                            && sched.processes[sched.shared_process_index(idx)].cwd == *cwd
                    }) =>
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
