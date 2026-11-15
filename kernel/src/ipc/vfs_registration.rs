//! Root-service binding is independent of nameserver strings and public SEND
//! capabilities. Call with scheduler and broker lifecycle serialization held.
use super::vfs_resources::{Error as ResourceError, NamespacePolicy, Resources};
use super::IpcError;
use crate::capability::CapabilityBroker;
use crate::sched::Scheduler;
use sunlight_ipc::vfs_rpc::registration::{EndpointOwner, Registry};
use sunlight_ipc::vfs_rpc::{lifecycle::Caller, registration::Registration};
use sunlight_ipc::vfs_rpc::{
    pending_table::{PrepareError, Retired, Table},
    Transfer,
};

/// One boot-lived owner and one serialization point for registration and local
/// namespace policy and pending resources. Never recreate this owner when a
/// service is replaced: namespace generations and request IDs are boot-scoped.
struct Runtime {
    registry: Registry,
    namespaces: NamespacePolicy<64>,
    pending: Table<Resources, 64>,
}

impl Runtime {
    const fn new() -> Self {
        Self {
            registry: Registry::new(),
            namespaces: NamespacePolicy::new(),
            pending: Table::new(),
        }
    }

    fn retire_registration(&mut self, expected: Registration) {
        // Check the full registration before projecting to Service, whose
        // identity uses the boot-unique epoch instead of a separate VM field.
        if !self.registry.retire(expected) {
            return;
        }
        self.namespaces.retire_registration(expected);
        self.pending.retire_service(expected.service());
    }

    fn retire_process(&mut self, expected: Caller) {
        self.namespaces.retire_caller(expected);
        self.pending.retire_caller(expected);
        if let Some(active) = self.registry.active() {
            if owner_matches(active.owner, expected) {
                self.retire_registration(active);
            }
        }
    }

    fn retire_endpoint(&mut self, endpoint: u32, owner: Caller) {
        if let Some(active) = self.registry.active() {
            if active.owner.endpoint == endpoint as u64 && owner_matches(active.owner, owner) {
                self.retire_registration(active);
            }
        }
    }

    /// Local preparation only. No publication, mappings or remote lease exists.
    /// Call only after process/credential revalidation under lifecycle locks.
    #[allow(dead_code)] // Awaiting authenticated dispatch integration.
    fn admit_stat(
        &mut self,
        identity: AdmissionIdentity,
        path: &str,
    ) -> Result<Transfer, PrepareError<ResourceError>> {
        if self.registry.active() != Some(identity.registration) {
            return Err(PrepareError::Resource(ResourceError::StaleNamespace));
        }
        let namespace = self
            .namespaces
            .lookup(identity)
            .map_err(PrepareError::Resource)?;
        Resources::admit_stat(&mut self.pending, identity, &namespace, path)
    }
}

fn owner_matches(owner: EndpointOwner, caller: Caller) -> bool {
    owner.pid == caller.pid
        && owner.incarnation == caller.incarnation
        && owner.address_space_generation == caller.address_space_generation
}

static RUNTIME: spin::Mutex<Runtime> = spin::Mutex::new(Runtime::new());

/// Capture from a retained process record under SCHEDULER, never a wire PID.
pub(crate) fn process_identity(process: &crate::process::Process) -> Caller {
    Caller {
        pid: process.pid as u64,
        incarnation: process.foreground.incarnation,
        address_space_generation: process.address_space.identity().generation,
    }
}

/// Call at termination commit or successful live exec, holding SCHEDULER.
/// Revokes local admission and terminalizes matching pending requests, retaining
/// their resources until explicit reap. No remote drain, IPC or VM cleanup.
pub(crate) fn retire_process(expected: Caller) {
    RUNTIME.lock().retire_process(expected);
}

/// Called after authenticated endpoint destruction, still holding SCHEDULER.
pub(crate) fn retire_endpoint(endpoint: u32, owner: Caller) {
    RUNTIME.lock().retire_endpoint(endpoint, owner);
}

/// Owned credential values, not retained process/VM or namespace references.
/// No supplementary groups exist in the current kernel credential model.
/// Adding them requires extending this snapshot before enabling dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // Dispatch admission is the next integration consumer.
pub(super) struct AdmissionIdentity {
    pub caller: Caller,
    pub uid: u32,
    pub gid: u32,
    pub groups: [u32; 0],
    pub registration: Registration,
}

/// Caller PID must come from the syscall entry, never a message or user buffer.
/// Hold scheduler -> broker lifecycle locks throughout capture. This function
/// neither publishes a request nor grants namespace authority. After dropping
/// those locks to acquire resources, revalidate before admission/publication.
#[allow(dead_code)]
pub(super) fn capture(
    caller_pid: usize,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<AdmissionIdentity, IpcError> {
    let process = sched
        .processes
        .iter()
        .find(|p| p.pid == caller_pid && !p.is_exiting())
        .ok_or(IpcError::InvalidCapability)?;
    let caller = process_identity(process);
    if [
        caller.pid,
        caller.incarnation,
        caller.address_space_generation,
    ]
    .contains(&0)
    {
        return Err(IpcError::InvalidCapability);
    }
    let mut runtime = RUNTIME.lock();
    let registration =
        live_registration(&mut runtime, sched, caps).ok_or(IpcError::InvalidCapability)?;
    Ok(AdmissionIdentity {
        caller,
        uid: process.uid,
        gid: process.gid,
        groups: [],
        registration,
    })
}

/// Must run under the same scheduler -> broker locks as the eventual table
/// admission and send. Revalidation alone does not pin a lifetime after unlock.
#[allow(dead_code)]
pub(super) fn revalidate(
    expected: AdmissionIdentity,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<(), IpcError> {
    let pid = usize::try_from(expected.caller.pid).map_err(|_| IpcError::InvalidCapability)?;
    if capture(pid, sched, caps)? != expected {
        return Err(IpcError::InvalidCapability);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(super) enum StatAdmissionError {
    Identity,
    Prepare(PrepareError<ResourceError>),
}

/// Kernel-private local preparation. Hold scheduler -> broker throughout this
/// call so retirement cannot interleave between revalidation and admission.
/// Preparation is bounded and may take the quota/allocator locks, in that order
/// below RUNTIME; those paths must never acquire lifecycle locks. No IPC is sent.
#[allow(dead_code)]
pub(super) fn admit_stat(
    identity: AdmissionIdentity,
    path: &str,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<Transfer, StatAdmissionError> {
    revalidate(identity, sched, caps).map_err(|_| StatAdmissionError::Identity)?;
    RUNTIME
        .lock()
        .admit_stat(identity, path)
        .map_err(StatAdmissionError::Prepare)
}

/// Move one terminal entry to cleanup ownership. The runtime guard is released
/// before returning. Drop/deliver the result only after releasing any scheduler
/// and broker guards as well. This is local-only: future published operations
/// additionally require remote release/drain before their resources can be freed.
#[allow(dead_code)]
pub(super) fn reap_next() -> Option<Retired<Resources>> {
    RUNTIME.lock().pending.reap_next()
}

fn live_registration(
    runtime: &mut Runtime,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Option<Registration> {
    let active = runtime.registry.active()?;
    let live = sched.processes.iter().any(|p| {
        p.pid as u64 == active.owner.pid
            && p.foreground.incarnation == active.owner.incarnation
            && p.address_space.identity().generation == active.owner.address_space_generation
            && p.trusted_vfs_service
            && !p.is_exiting()
    }) && u32::try_from(active.owner.endpoint).ok().is_some_and(|id| {
        caps.endpoint_owner(id)
            .is_some_and(|pid| pid as u64 == active.owner.pid)
    });
    if live {
        Some(active)
    } else {
        runtime.retire_registration(active);
        None
    }
}

pub(super) fn register(
    caller_pid: usize,
    endpoint: u32,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<(), IpcError> {
    let process = sched
        .processes
        .iter()
        .find(|p| p.pid == caller_pid)
        .ok_or(IpcError::InvalidCapability)?;
    if !process.trusted_vfs_service
        || process.is_exiting()
        || caps.endpoint_owner(endpoint) != Some(caller_pid)
    {
        return Err(IpcError::InvalidCapability);
    }
    let candidate = EndpointOwner {
        endpoint: endpoint as u64,
        pid: caller_pid as u64,
        incarnation: process.foreground.incarnation,
        address_space_generation: process.address_space.identity().generation,
    };
    // Lock order: scheduler -> broker (when needed) -> VFS runtime. Never
    // acquire lifecycle locks or wait for IPC while holding RUNTIME.
    let mut runtime = RUNTIME.lock();
    live_registration(&mut runtime, sched, caps);
    runtime
        .registry
        .register(candidate)
        .map(|_| ())
        .map_err(|_| IpcError::InvalidCapability)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> EndpointOwner {
        EndpointOwner {
            endpoint: 0,
            pid: 3,
            incarnation: 4,
            address_space_generation: 5,
        }
    }

    fn caller() -> Caller {
        Caller {
            pid: 10,
            incarnation: 11,
            address_space_generation: 12,
        }
    }

    fn identity(caller: Caller, registration: Registration) -> AdmissionIdentity {
        AdmissionIdentity {
            caller,
            registration,
            uid: 1000,
            gid: 100,
            groups: [],
        }
    }

    #[test]
    fn caller_retirement_is_exact_and_preserves_other_bindings_and_generations() {
        let mut runtime = Runtime::new();
        let registration = runtime.registry.register(owner()).unwrap();
        let id = identity(caller(), registration);
        let mut other = id;
        other.caller.pid += 1;
        runtime.namespaces.install(id, 9, 10, 0).unwrap();
        runtime.namespaces.install(other, 9, 10, 0).unwrap();
        let retained = runtime.namespaces.lookup(id).unwrap();
        for field in 0..3 {
            let mut wrong = id.caller;
            match field {
                0 => wrong.pid += 2,
                1 => wrong.incarnation += 1,
                _ => wrong.address_space_generation += 1,
            }
            runtime.retire_process(wrong);
            assert!(runtime.namespaces.lookup(id).is_ok());
        }
        runtime.retire_process(id.caller);
        runtime.retire_process(id.caller);
        assert!(runtime.namespaces.lookup(id).is_err());
        assert!(runtime.namespaces.lookup(other).is_ok());
        assert_eq!(runtime.registry.active(), Some(registration));
        let mut replacement = id;
        replacement.caller.address_space_generation += 1;
        runtime.namespaces.install(replacement, 9, 10, 0).unwrap();
        runtime.retire_process(id.caller);
        assert!(runtime.namespaces.lookup(replacement).is_ok());
        // Policy relinquished its ownership; the independent preparation pin lives.
        assert_eq!(alloc::sync::Arc::strong_count(&retained), 1);
    }

    #[test]
    fn endpoint_and_service_retirement_revoke_all_scopes_without_resetting_epochs() {
        let mut runtime = Runtime::new();
        let first = runtime.registry.register(owner()).unwrap();
        let id = identity(caller(), first);
        runtime.namespaces.install(id, 9, 10, 0).unwrap();
        let service = Caller {
            pid: first.owner.pid,
            incarnation: first.owner.incarnation,
            address_space_generation: first.owner.address_space_generation,
        };
        for field in 0..4 {
            let mut wrong = service;
            let mut endpoint = 0;
            match field {
                0 => endpoint = 1,
                1 => wrong.pid += 1,
                2 => wrong.incarnation += 1,
                _ => wrong.address_space_generation += 1,
            }
            runtime.retire_endpoint(endpoint, wrong);
            assert_eq!(runtime.registry.active(), Some(first));
            assert!(runtime.namespaces.lookup(id).is_ok());
        }
        runtime.retire_endpoint(0, service);
        assert_eq!(runtime.registry.active(), None);
        assert!(runtime.namespaces.lookup(id).is_err());
        let mut new_owner = owner();
        new_owner.address_space_generation += 1;
        let second = runtime.registry.register(new_owner).unwrap();
        assert!(second.epoch > first.epoch);
        let replacement = identity(id.caller, second);
        runtime.namespaces.install(replacement, 9, 10, 0).unwrap();
        runtime.retire_process(service);
        runtime.retire_endpoint(0, service);
        runtime.retire_registration(first);
        assert_eq!(runtime.registry.active(), Some(second));
        assert!(runtime.namespaces.lookup(replacement).is_ok());
        // Successful exec/exit of the exact service revokes every caller scope.
        let mut other = replacement;
        other.caller.pid += 1;
        runtime.namespaces.install(other, 9, 10, 0).unwrap();
        runtime.retire_process(Caller {
            address_space_generation: new_owner.address_space_generation,
            ..service
        });
        assert_eq!(runtime.registry.active(), None);
        assert!(runtime.namespaces.lookup(replacement).is_err());
        assert!(runtime.namespaces.lookup(other).is_err());
    }

    #[test]
    fn runtime_retirement_keeps_pending_resources_until_explicit_reap() {
        use super::super::vfs_resources::{charged_bytes, TEST_LEDGER_LOCK};
        use sunlight_ipc::vfs_rpc::{
            context,
            lifecycle::{Failure, State},
        };
        let _serial = TEST_LEDGER_LOCK.lock();
        assert_eq!(charged_bytes(), 0);
        for event in 0..3 {
            let mut runtime = Runtime::new();
            let first = runtime.registry.register(owner()).unwrap();
            let id = identity(caller(), first);
            let mut other = id;
            other.caller.pid += 1;
            runtime.namespaces.install(id, 9, 10, 0).unwrap();
            runtime.namespaces.install(other, 9, 10, 0).unwrap();
            let binding = runtime.namespaces.lookup(id).unwrap();
            let weak = alloc::sync::Arc::downgrade(&binding);
            drop(binding);
            let request = runtime.admit_stat(id, "/owned").unwrap().words()[1];
            let unrelated = runtime.admit_stat(other, "/other").unwrap().words()[1];
            let charged = charged_bytes();
            assert!(charged > 0);
            assert!(runtime.pending.reap(request).is_none());
            for field in 0..3 {
                let mut stale = id.caller;
                match field {
                    0 => stale.pid += 2,
                    1 => stale.incarnation += 1,
                    _ => stale.address_space_generation += 1,
                }
                runtime.retire_process(stale);
                assert_eq!(runtime.pending.state(request), Some(State::Waiting));
            }
            let service = Caller {
                pid: first.owner.pid,
                incarnation: first.owner.incarnation,
                address_space_generation: first.owner.address_space_generation,
            };
            let reason = if event == 0 {
                runtime.retire_process(id.caller);
                runtime.retire_process(id.caller);
                assert_eq!(runtime.pending.state(unrelated), Some(State::Waiting));
                Failure::CallerRetired
            } else {
                // Every authenticated endpoint/owner field must match.
                for field in 0..4 {
                    let mut wrong = service;
                    let mut endpoint = first.owner.endpoint as u32;
                    match field {
                        0 => endpoint += 1,
                        1 => wrong.pid += 1,
                        2 => wrong.incarnation += 1,
                        _ => wrong.address_space_generation += 1,
                    }
                    runtime.retire_endpoint(endpoint, wrong);
                    assert_eq!(runtime.pending.state(request), Some(State::Waiting));
                }
                if event == 1 {
                    runtime.retire_endpoint(first.owner.endpoint as u32, service);
                } else {
                    runtime.retire_process(service);
                }
                assert_eq!(
                    runtime.pending.state(unrelated),
                    Some(State::Failed(Failure::ServiceRetired))
                );
                Failure::ServiceRetired
            };
            assert_eq!(runtime.pending.state(request), Some(State::Failed(reason)));
            assert!(runtime.admit_stat(id, "/denied").is_err());
            assert!(weak.upgrade().is_some());
            assert_eq!(charged_bytes(), charged);

            // A replacement shares the same table and cannot recycle an ID.
            runtime.retire_registration(first);
            let mut new_owner = owner();
            new_owner.address_space_generation += 1;
            let second = runtime.registry.register(new_owner).unwrap();
            let replacement = identity(id.caller, second);
            runtime.namespaces.install(replacement, 9, 10, 0).unwrap();
            let new_request = runtime.admit_stat(replacement, "/new").unwrap().words()[1];
            assert!(new_request > unrelated);
            runtime.retire_process(service);
            runtime.retire_endpoint(first.owner.endpoint as u32, service);
            runtime.retire_registration(first);
            assert_eq!(runtime.pending.state(new_request), Some(State::Waiting));
            // First terminal result is stable, including after service death.
            assert_eq!(runtime.pending.state(request), Some(State::Failed(reason)));
            let retired = runtime.pending.reap(request).unwrap();
            assert!(runtime.pending.reap(request).is_none());
            assert!(weak.upgrade().is_some());
            assert!(charged_bytes() > charged);
            assert_eq!(
                retired.resources.revalidate_namespace(),
                Err(ResourceError::StaleNamespace)
            );
            let mut groups = [0; context::MAX_GROUPS];
            let decoded = context::decode(retired.resources.input(), &mut groups).unwrap();
            assert_eq!(decoded.request_id, request);
            assert_eq!(decoded.context.caller, id.caller);
            let before_drop = charged_bytes();
            drop(retired); // Explicit cleanup owner releases the final reference.
            assert!(weak.upgrade().is_none());
            assert!(charged_bytes() < before_drop);
            runtime.retire_process(replacement.caller);
            while let Some(retired) = runtime.pending.reap_next() {
                drop(retired);
            }
            assert!(runtime.pending.is_empty());
            assert_eq!(charged_bytes(), 0);
        }
    }

    #[test]
    fn terminal_capacity_completion_and_ids_survive_registration_replacement() {
        use super::super::vfs_resources::{charged_bytes, TEST_LEDGER_LOCK};
        use sunlight_ipc::vfs_rpc::{
            lifecycle::{Failure, State},
            pending_table::AdmissionError,
            response::ReplyMessage,
        };
        let _serial = TEST_LEDGER_LOCK.lock();
        let mut runtime = Runtime::new();
        let first = runtime.registry.register(owner()).unwrap();
        let id = identity(caller(), first);
        for n in 0..4 {
            let mut next = id;
            next.caller.pid += n;
            runtime.namespaces.install(next, 9, 10, 0).unwrap();
            for _ in 0..16 {
                runtime.admit_stat(next, "/bounded").unwrap();
            }
        }
        assert_eq!(runtime.pending.len(), 64);
        let charged = charged_bytes();
        for field in 0..5 {
            let mut wrong = first;
            match field {
                0 => wrong.owner.endpoint += 1,
                1 => wrong.owner.pid += 1,
                2 => wrong.owner.incarnation += 1,
                3 => wrong.owner.address_space_generation += 1,
                _ => wrong.epoch += 1,
            }
            runtime.retire_registration(wrong);
            assert_eq!(runtime.registry.active(), Some(first));
            assert_eq!(runtime.pending.state(1), Some(State::Waiting));
            assert!(runtime.namespaces.lookup(id).is_ok());
        }
        let reply = |request| ReplyMessage {
            label: sunlight_ipc::vfs_rpc::REPLY,
            words: [1, request, 2, 0],
            word_count: 4,
            cap_count: 0,
            output: &[],
        };
        runtime.pending.complete(first.service(), reply(1)).unwrap();
        runtime.retire_registration(first);
        assert!(matches!(
            runtime.pending.state(1),
            Some(State::Completed(_))
        ));
        assert_eq!(
            runtime.pending.state(2),
            Some(State::Failed(Failure::ServiceRetired))
        );
        assert!(runtime.pending.complete(first.service(), reply(2)).is_err());
        assert_eq!(charged_bytes(), charged);
        let second = runtime.registry.register(owner()).unwrap();
        let replacement = identity(id.caller, second);
        runtime.namespaces.install(replacement, 9, 10, 0).unwrap();
        assert_eq!(
            runtime.admit_stat(replacement, "/full"),
            Err(PrepareError::Admission(AdmissionError::Full))
        );
        assert_eq!(charged_bytes(), charged);
        let retired = runtime.pending.reap_next().unwrap();
        assert_eq!(retired.request_id, 1);
        // Reaping frees a table slot; the cleanup owner still holds the charge.
        assert_eq!(charged_bytes(), charged);
        drop(retired);
        let next = runtime
            .admit_stat(replacement, "/replacement")
            .unwrap()
            .words()[1];
        assert!(next > 64);
        runtime.retire_registration(first);
        assert_eq!(runtime.pending.state(next), Some(State::Waiting));
        runtime.retire_registration(second);
        while let Some(retired) = runtime.pending.reap_next() {
            drop(retired);
        }
        assert_eq!(charged_bytes(), 0);
    }
}
