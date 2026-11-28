//! Root-service binding is independent of nameserver strings and public SEND
//! capabilities. Call with scheduler and broker lifecycle serialization held.
use super::vfs_resources::{Error as ResourceError, NamespaceBinding, NamespacePolicy, Resources};
use super::IpcError;
use crate::capability::CapabilityBroker;
use crate::process::exec_retirement::PrivateVfsExecRetired;
use crate::sched::image_retirement::PrivateImageRetired;
use crate::sched::Scheduler;
use alloc::sync::Arc;
use sunlight_ipc::vfs_rpc::registration::{EndpointOwner, Registry};
use sunlight_ipc::vfs_rpc::{
    lifecycle::{Caller, RequestIds},
    registration::Registration,
};
use sunlight_ipc::vfs_rpc::{
    pending_table::{PrepareError, Retired, Table},
    retention::{
        ownership::{Error as OwnershipError, Ownership, Retained},
        wire::{Frame, Refusal},
        Receipt, Scope,
    },
    Transfer,
};

// Unresolved old epochs consume these slots. Exhaustion closes registration
// admission rather than losing unknown remote leases during replacement.
const REMOTE_REGISTRATIONS: usize = 4;
const REMOTE_LEASES: usize = 64;

struct RemoteRegistration {
    registration: Registration,
    retired: bool,
    image_retired: bool,
    ownership: Ownership<REMOTE_LEASES>,
    local: [Option<LocalLease>; REMOTE_LEASES],
    unsent: [Option<Frame>; REMOTE_LEASES],
    delivered: Option<u64>,
    callers: [Option<CallerDelivery>; REMOTE_LEASES],
    last_caller_id: u64,
    last_wake_tick: Option<u64>,
}

// Registration construction currently uses a 32-KiB kernel stack. Keep room
// for the register/caller frames; inline handoff receipts exceed that budget.
const _: () = assert!(core::mem::size_of::<RemoteRegistration>() <= 28 * 1024);

/// Capacity is reserved before namespace preparation, including callers whose
/// leases later settle. Metadata acknowledgement preserves this row until a
/// separate file-owner exit/exec acknowledgement. Teardown needs no allocation.
struct CallerDelivery {
    caller: Caller,
    id: u64,
    retired: bool,
    delivered: bool,
    metadata_acked: bool,
    file_retired: bool,
    file_delivered: bool,
    /// Set before old-image retirement and cleared after all surviving
    /// descriptor transfers are published. The file-owner barrier waits.
    file_transfer_pending: bool,
}

impl CallerDelivery {
    fn pending(&self) -> bool {
        (self.retired && !self.metadata_acked) || self.file_retired
    }
}

enum LocalLease {
    Preparing {
        id: u64,
        identity: AdmissionIdentity,
        scope: Scope,
        grants: u64,
        root_policy: bool,
        initial_cwd: Option<alloc::string::String>,
        derivation: Option<Inheritance>,
    },
    Bound {
        receipt: Receipt,
        binding: Arc<NamespaceBinding>,
    },
}

impl LocalLease {
    fn id(&self) -> u64 {
        match self {
            Self::Preparing { id, .. } => *id,
            Self::Bound { receipt, .. } => receipt.request_id(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum NamespaceRetentionError {
    Resource(ResourceError),
    Ownership(OwnershipError),
}

/// One boot-lived owner and serialization point for registration, local namespace
/// policy, pending resources and remote lease obligations. Never recreate this
/// owner on replacement: namespace generations and RPC IDs are boot-scoped;
/// retention IDs are registration-scoped and old owners survive retirement.
struct Runtime {
    publications: [Option<stat::Publication>; 64],
    chdirs: [Option<chdir::Call>; 64],
    spawns: [Option<spawn::Call>; 8],
    file_publications: [Option<file::Publication>; 64],
    file_ids: RequestIds,
    registry: Registry,
    namespaces: NamespacePolicy<64>,
    pending: Table<Resources, 64>,
    remote: [Option<RemoteRegistration>; REMOTE_REGISTRATIONS],
}

impl Runtime {
    const fn new() -> Self {
        Self {
            publications: [const { None }; 64],
            chdirs: [const { None }; 64],
            spawns: [const { None }; 8],
            file_publications: [const { None }; 64],
            file_ids: RequestIds::new(),
            registry: Registry::new(),
            namespaces: NamespacePolicy::new(),
            pending: Table::new(),
            remote: [const { None }; REMOTE_REGISTRATIONS],
        }
    }

    fn register(&mut self, candidate: EndpointOwner) -> Result<Registration, IpcError> {
        if self.registry.active().is_some() {
            // Idempotence must preserve the existing ownership and ID allocator.
            return self
                .registry
                .register(candidate)
                .map_err(|_| IpcError::InvalidCapability);
        }
        self.collect_remote();
        let slot = self
            .remote
            .iter()
            .position(Option::is_none)
            .ok_or(IpcError::InvalidCapability)?;
        // Admit capacity before changing the active registration. Registry
        // validates every field required by Ownership; insertion cannot fail.
        let registration = self
            .registry
            .register(candidate)
            .map_err(|_| IpcError::InvalidCapability)?;
        self.remote[slot] = Some(RemoteRegistration {
            registration,
            retired: false,
            image_retired: false,
            ownership: Ownership::new(registration).expect("validated VFS registration"),
            local: [const { None }; REMOTE_LEASES],
            unsent: [None; REMOTE_LEASES],
            delivered: None,
            callers: [const { None }; REMOTE_LEASES],
            last_caller_id: 0,
            last_wake_tick: None,
        });
        Ok(registration)
    }

    /// Component preparation only: the identity and grants require trusted
    /// provisioning and lifecycle revalidation before publication. Store the
    /// complete snapshot before returning the single-send retain frame.
    #[allow(dead_code)]
    fn prepare_namespace(
        &mut self,
        identity: AdmissionIdentity,
        grants: u64,
    ) -> Result<Frame, NamespaceRetentionError> {
        self.prepare_namespace_inner(identity, grants, None)
    }

    fn prepare_namespace_inner(
        &mut self,
        identity: AdmissionIdentity,
        grants: u64,
        previous: Option<Scope>,
    ) -> Result<Frame, NamespaceRetentionError> {
        use NamespaceRetentionError::{Ownership, Resource};
        if self.registry.active() != Some(identity.registration)
            || grants & !sunlight_ipc::vfs_rpc::context::LOOKUP_METADATA != 0
        {
            return Err(Resource(ResourceError::StaleNamespace));
        }
        let remote = self
            .remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == identity.registration)
            .ok_or(Ownership(OwnershipError::State))?;
        let slot = remote
            .local
            .iter()
            .position(Option::is_none)
            .ok_or(Ownership(OwnershipError::Capacity))?;
        let caller_slot = remote
            .callers
            .iter()
            .position(|c| c.as_ref().is_some_and(|c| c.caller == identity.caller));
        if caller_slot.is_some_and(|i| remote.callers[i].as_ref().unwrap().retired) {
            return Err(Ownership(OwnershipError::State));
        }
        let new_caller = if caller_slot.is_none() {
            let slot = remote
                .callers
                .iter()
                .position(Option::is_none)
                .ok_or(Ownership(OwnershipError::Capacity))?;
            let id = remote
                .last_caller_id
                .checked_add(1)
                .ok_or(Ownership(OwnershipError::Capacity))?;
            Some((slot, id))
        } else {
            None
        };
        let mut scope = self.namespaces.reserve_scope(identity).map_err(Resource)?;
        if let Some(previous) = previous {
            scope.namespace_id = previous.namespace_id;
        }
        let frame = remote.ownership.prepare(scope).map_err(Ownership)?;
        if let Some((slot, id)) = new_caller {
            remote.callers[slot] = Some(CallerDelivery {
                caller: identity.caller,
                id,
                retired: false,
                delivered: false,
                metadata_acked: false,
                file_retired: false,
                file_delivered: false,
                file_transfer_pending: false,
            });
            remote.last_caller_id = id;
        }
        let Frame::Retain { request_id: id, .. } = frame else {
            unreachable!()
        };
        remote.local[slot] = Some(LocalLease::Preparing {
            id,
            identity,
            scope,
            grants,
            root_policy: false,
            initial_cwd: None,
            derivation: None,
        });
        Ok(frame)
    }

    /// Transport must authenticate registration independently and capture
    /// `live` under lifecycle locks (None means no longer authorized). A decoded
    /// reply supplies neither. Malformed replies preserve all preparation state.
    /// Installation failure owns a retryable release before returning the error.
    #[allow(dead_code)]
    fn install_retained(
        &mut self,
        authenticated: Registration,
        id: u64,
        bytes: &[u8],
        live: Option<AdmissionIdentity>,
    ) -> Result<Option<Frame>, NamespaceRetentionError> {
        self.install_transition_receipt(authenticated, id, bytes, live, None)
    }

    fn install_transition_receipt(
        &mut self,
        authenticated: Registration,
        id: u64,
        bytes: &[u8],
        live: Option<AdmissionIdentity>,
        previous: Option<Scope>,
    ) -> Result<Option<Frame>, NamespaceRetentionError> {
        use NamespaceRetentionError::{Ownership, Resource};
        let remote = self
            .remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == authenticated)
            .ok_or(Ownership(OwnershipError::State))?;
        let slot = remote
            .local
            .iter()
            .position(|l| l.as_ref().is_some_and(|l| l.id() == id))
            .ok_or(Ownership(OwnershipError::State))?;
        let Some(LocalLease::Preparing {
            identity,
            scope,
            grants,
            ..
        }) = remote.local[slot]
        else {
            return Err(Ownership(OwnershipError::State));
        };
        match remote
            .ownership
            .retained(id, authenticated, bytes)
            .map_err(Ownership)?
        {
            Retained::Release(frame) => {
                remote.local[slot] = None;
                Ok(Some(frame))
            }
            Retained::Install(receipt) => {
                // Ownership correlation already checked the full receipt scope.
                assert_eq!(receipt.scope(), scope);
                remote.local[slot] = None;
                if live != Some(identity) || self.registry.active() != Some(authenticated) {
                    return remote.ownership.release(id).map(Some).map_err(Ownership);
                }
                match self.namespaces.publish_receipt(identity, receipt, grants, previous) {
                    Ok(binding) => {
                        remote.local[slot] = Some(LocalLease::Bound { receipt, binding });
                        Ok(None)
                    }
                    Err(error) => {
                        remote.ownership.release(id).map_err(Ownership)?;
                        Err(Resource(error))
                    }
                }
            }
        }
    }

    /// Bounded local reconciliation, never IPC. Withdrawal closes admission;
    /// terminal pending resources and explicit reap owners continue pinning the
    /// binding. Release becomes retryable only after their final reference drops.
    #[allow(dead_code)]
    fn reconcile_namespaces(&mut self) {
        for remote in self.remote.iter_mut().flatten() {
            for slot in &mut remote.local {
                if let Some(LocalLease::Bound { receipt, binding }) = slot {
                    if NamespaceBinding::drained(binding) {
                        remote
                            .ownership
                            .release(receipt.request_id())
                            .expect("installed VFS lease remains held until local drain");
                        *slot = None;
                    }
                }
            }
        }
    }

    fn collect_remote(&mut self) {
        for slot in &mut self.remote {
            if slot.as_ref().is_some_and(|r| {
                r.retired
                    && if r.image_retired {
                        // Service-backed objects are gone. Local continuations
                        // retain their bindings until their own terminal cleanup.
                        r.local.iter().flatten().all(|lease| match lease {
                            LocalLease::Bound { binding, .. } => NamespaceBinding::drained(binding),
                            LocalLease::Preparing { .. } => true,
                        })
                    } else {
                        r.ownership.outstanding() == 0 && r.callers.iter().all(Option::is_none)
                    }
            }) {
                *slot = None;
            }
        }
    }

    fn complete_image_exit(&mut self, proof: PrivateImageRetired) {
        self.settle_private_backing(proof.image());
    }

    fn complete_private_exec(&mut self, proof: PrivateVfsExecRetired) {
        let image = proof.image();
        self.settle_private_backing(image);
        #[cfg(feature = "vfs-n1j-test")]
        crate::vfs_n1j_test::image_retired(image);
    }

    fn settle_private_backing(&mut self, image: crate::capability::delivery::RecipientIdentity) {
        let caller = Caller {
            pid: image.pid as u64,
            incarnation: image.incarnation,
            address_space_generation: image.vm_generation,
        };
        for remote in self.remote.iter_mut().flatten() {
            if remote.retired && owner_matches(remote.registration.owner, caller) {
                // Each row already pins the exact endpoint and epoch. The
                // embedded service keeps directory pairs and file-owner rows
                // in private memory, and cannot hand them off by IPC.
                remote.image_retired = true;
                for slot in &mut remote.local {
                    if matches!(slot, Some(LocalLease::Preparing { .. })) {
                        *slot = None;
                    }
                }
            }
        }
        self.collect_remote();
    }

    fn remote_owner(
        &mut self,
        registration: Registration,
    ) -> Result<&mut Ownership<REMOTE_LEASES>, OwnershipError> {
        self.remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == registration)
            .map(|r| &mut r.ownership)
            .ok_or(OwnershipError::State)
    }

    /// Trusted local preparation only. Future callers must revalidate process,
    /// credentials and provisioned scope under lifecycle locks before this call.
    /// The returned retain is single-send, including after uncertain publication.
    #[allow(dead_code)]
    fn prepare_retain(
        &mut self,
        registration: Registration,
        scope: Scope,
    ) -> Result<Frame, OwnershipError> {
        if self.registry.active() != Some(registration) {
            return Err(OwnershipError::State);
        }
        self.remote_owner(registration)?.prepare(scope)
    }

    /// Authentication must come from future transport records, never decoded
    /// bytes. Keep retired epochs routable so late success can own its release.
    /// Install still requires live caller/scope revalidation by the integration.
    #[allow(dead_code)]
    fn retained(
        &mut self,
        authenticated: Registration,
        id: u64,
        bytes: &[u8],
    ) -> Result<Retained, OwnershipError> {
        self.remote_owner(authenticated)?
            .retained(id, authenticated, bytes)
    }

    #[allow(dead_code)]
    fn refused(
        &mut self,
        authenticated: Registration,
        id: u64,
        bytes: &[u8],
    ) -> Result<Refusal, OwnershipError> {
        let reason = self
            .remote_owner(authenticated)?
            .refused(id, authenticated, bytes)?;
        if let Some(remote) = self
            .remote
            .iter_mut()
            .flatten()
            .find(|r| r.registration == authenticated)
        {
            for slot in &mut remote.local {
                if slot.as_ref().is_some_and(|l| l.id() == id) {
                    *slot = None;
                }
            }
        }
        self.collect_remote();
        Ok(reason)
    }

    #[allow(dead_code)]
    fn released(
        &mut self,
        authenticated: Registration,
        id: u64,
        bytes: &[u8],
    ) -> Result<(), OwnershipError> {
        self.remote_owner(authenticated)?
            .released(id, authenticated, bytes)?;
        self.collect_remote();
        Ok(())
    }

    fn retire_registration(&mut self, expected: Registration) {
        // Check the full registration before projecting to Service, whose
        // identity uses the boot-unique epoch instead of a separate VM field.
        if !self.registry.retire(expected) {
            return;
        }
        #[cfg(feature = "vfs-n1j-test")]
        crate::vfs_n1j_test::withdraw(expected);
        for c in self.chdirs.iter_mut().flatten() {
            if c.identity.registration == expected && c.result.is_none() {
                c.result = Some(5);
            }
        }
        self.retire_spawns(expected);
        self.namespaces.retire_registration(expected);
        self.pending.retire_service(expected.service());
        self.retire_file_registration(expected);
        for remote in self.remote.iter_mut().flatten() {
            if remote.registration == expected {
                remote.retired = true;
                remote.ownership.abandon_all();
            }
        }
        self.reconcile_namespaces();
        self.collect_remote();
        #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r2_test"))]
        spawn::fixture::observe_withdrawal(self, expected);
        #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r3_test"))]
        spawn::fixture::r3::withdrawn(self, expected);
    }

    fn retire_process(&mut self, expected: Caller) {
        self.detach_spawns_for(expected);
        self.retire_caller_profile(expected);
        // Reserved before namespace admission and retained after metadata ack:
        // exit/exec needs neither allocation nor still-live metadata authority.
        for remote in self.remote.iter_mut().flatten() {
            for caller in remote.callers.iter_mut().flatten() {
                if caller.caller == expected {
                    caller.file_retired = true;
                }
            }
        }
        if let Some(active) = self.registry.active() {
            if owner_matches(active.owner, expected) {
                self.retire_registration(active);
            }
        }
    }

    fn retire_caller_profile(&mut self, expected: Caller) {
        self.cancel_spawns_for(expected);
        if let Some(registration) = self
            .chdirs
            .iter()
            .flatten()
            .find(|c| c.waiter == expected && c.identity.caller != expected && c.result.is_none())
            .map(|c| c.identity.registration)
        {
            // The worker can die after the service provisionally changed its
            // owner's policy. Retire the epoch rather than regrant a possibly
            // different object from the owner's saved cwd spelling.
            self.retire_registration(registration);
        }
        for slot in &mut self.chdirs {
            if let Some(c) = slot {
                if c.waiter == expected {
                    *slot = None;
                } else if c.identity.caller == expected && c.result.is_none() {
                    c.result = Some(125);
                }
            }
        }
        self.namespaces.retire_caller(expected);
        self.pending.retire_caller(expected);
        self.retire_file_publications(expected);
        CLEANUP_PENDING.store(true, core::sync::atomic::Ordering::Release);
        // Exit/exec or profile withdrawal discards the continuation, leaving
        // terminal resources for cleanup after lifecycle serialization ends.
        for slot in &mut self.publications {
            if slot.as_ref().is_some_and(|p| p.identity.caller == expected) {
                *slot = None;
            }
        }
        // Include quarantined old registrations, not only the active service.
        for remote in self.remote.iter_mut().flatten() {
            remote.ownership.abandon_caller(expected);
            for caller in remote.callers.iter_mut().flatten() {
                if caller.caller == expected {
                    caller.retired = true;
                }
            }
        }
        self.reconcile_namespaces();
    }

    /// A live cwd or credential change discards the pilot continuation, but must wake its
    /// owner before removing publication bookkeeping. The next call observes
    /// withdrawn authority. Never wake a nested IPC operation on its behalf.
    fn withdraw_root_profile(&mut self, idx: usize, sched: &mut Scheduler) {
        if !sched.processes[idx].vfs_root_metadata {
            return;
        }
        let expected = process_identity(&sched.processes[idx]);
        for c in self
            .chdirs
            .iter()
            .flatten()
            .filter(|c| c.identity.caller == expected)
        {
            if let Some(waiter) = sched
                .processes
                .iter()
                .position(|p| process_identity(p) == c.waiter && !p.is_exiting())
            {
                if sched.processes[waiter].state == crate::process::ProcessState::BlockedOnIpc
                    && sched.processes[waiter].pending_call.is_none()
                    && sched.processes[waiter].ipc_reply_target.is_none()
                {
                    super::wake_terminal(sched, waiter);
                }
            }
        }
        sched.processes[idx].vfs_root_metadata = false;
        let caller = process_identity(&sched.processes[idx]);
        if (self
            .publications
            .iter()
            .flatten()
            .any(|p| p.identity.caller == caller)
            || self
                .chdirs
                .iter()
                .flatten()
                .any(|p| p.identity.caller == caller))
            && sched.processes[idx].state == crate::process::ProcessState::BlockedOnIpc
            && sched.processes[idx].pending_call.is_none()
        {
            super::wake_terminal(sched, idx);
        }
        self.retire_caller_profile(caller);
    }

    fn retire_endpoint(&mut self, endpoint: u32, owner: Caller) {
        if let Some(active) = self.registry.active() {
            if active.owner.endpoint == endpoint as u64 && owner_matches(active.owner, owner) {
                self.retire_registration(active);
            }
        }
    }

    /// Local preparation only, without publication or mappings. Receipt-backed
    /// bindings pin their existing remote lease through the pending resources.
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

static CLEANUP_PENDING: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
static RUNTIME: spin::Mutex<Runtime> = spin::Mutex::new(Runtime::new());

/// Scheduler exit delivers synchronously, before the process slot can be reused.
pub(crate) fn complete_image_exit(proof: PrivateImageRetired) {
    RUNTIME.lock().complete_image_exit(proof);
    #[cfg(feature = "vfs-n1j-test")]
    crate::vfs_n1j_test::image_retired(proof.image());
    #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r1_test"))]
    spawn::fixture::observe_service_exit(proof.image());
}

pub(crate) fn complete_private_exec(proof: PrivateVfsExecRetired) {
    RUNTIME.lock().complete_private_exec(proof);
}

#[path = "vfs_registration/chdir.rs"]
mod chdir;
#[path = "vfs_registration/spawn.rs"]
pub(crate) mod spawn;
#[path = "vfs_registration/file.rs"]
mod file;
#[cfg(feature = "vfs_c1e1_test")]
pub(crate) use file::acquire_storage;
#[cfg(feature = "vfs_c1e2_test")]
pub(crate) use file::retire_storage_fixture_caller;
#[path = "vfs_registration/stat.rs"]
mod stat;
pub(crate) use chdir::{
    route_directory as route_vfs_directory, DirectoryRoute, LegacyCwdCommit, busy as chdir_busy, chdir_call, context as chdir_context, finish as chdir_finish, resume as chdir_resume,
};
#[path = "vfs_registration/inheritance.rs"]
mod inheritance;
#[path = "vfs_registration/transport.rs"]
mod transport;
pub(crate) use file::{
    file_call as vfs_file_call, wake_file_callers as vfs_wake_file_callers, FileCall,
    FileCompletion, FileOperation,
};
#[cfg(all(not(test), debug_assertions, feature = "vfs_service_private_mmap"))]
pub(crate) use file::cancel_mapping_read;
#[cfg(all(not(test), debug_assertions, feature = "vfs_d2b_test"))]
pub(crate) fn fixture_mapping_read(caller: Caller, service: Caller, id: u64) -> Option<(u64, u64)> {
    RUNTIME.lock().fixture_mapping_read(caller, service, id)
}
use inheritance::Inheritance;
pub(crate) use inheritance::{capture_inheritance, provision_inheritance};
pub(crate) use stat::{
    complete_stat, copy_failed as stat_copy_failed, stat_call as stat_path_call, take_stat,
    Error as StatError, StatCall,
};
pub(crate) use transport::{control, file_control, initial_cwd, FileControl};

#[path = "vfs_registration/wakeup.rs"]
mod wakeup;
pub(super) use wakeup::{take_control_wakeup, wake_control_receivers};

#[cfg(all(not(test), debug_assertions, feature = "vfs_service_private_mmap"))]
pub(crate) fn mapping_epoch(sched: &Scheduler, caps: &CapabilityBroker) -> Option<u64> {
    let identity = capture(sched.current_pid(), sched, caps).ok()?;
    RUNTIME.lock().caller_receipt(identity)?;
    Some(identity.registration.epoch)
}

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
/// their resources until explicit reap, and abandons matching pending retains.
/// Queue bounded copied-channel barriers for callers admitted by namespace
/// preparation. Remote ownership survives retirement. No synchronous IPC,
/// remote drain or VM cleanup occurs here.
pub(crate) fn retire_process(expected: Caller) {
    #[cfg(all(not(test), debug_assertions, feature = "vfs_service_private_mmap"))]
    crate::process::mmap::service::retire(expected);
    RUNTIME.lock().retire_process(expected);
}

/// Reserve the file-owner barrier before old-image retirement. Every surviving
/// non-CLOEXEC service descriptor must be transferred before it may run.
pub(crate) fn expect_file_transfer(expected: Caller) {
    RUNTIME.lock().expect_file_transfer(expected);
}

/// Publication is complete; pending transfer replies still gate the barrier.
pub(crate) fn finish_file_transfer(expected: Caller) {
    RUNTIME.lock().finish_file_transfer(expected);
}

/// Publish kernel-origin ownership transfers for surviving descriptors. The old
/// caller is intentionally no longer live; the active registration and exact
/// token are the only authority. Returns the number of published requests.
pub(crate) fn transfer_file_owner(
    old: Caller,
    new: Caller,
    tokens: &[(u64, u64)],
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> usize {
    let mut runtime = RUNTIME.lock();
    let Some(registration) = live_registration(&mut runtime, sched, caps) else {
        return 0;
    };
    let mut published = 0;
    for &(token, epoch) in tokens {
        if epoch != registration.epoch {
            continue;
        }
        if runtime
            .publish_file_transfer(registration, old, new, token, sched.global_tick)
            .is_ok()
        {
            published += 1;
        }
    }
    published
}

/// Publish a fire-and-forget close for a descriptor whose final kernel
/// reference was just dropped. The caller has already consumed the fd, so this
/// never parks it and never retransmits. Returns whether a request was queued.
pub(crate) fn close_file_owner(
    caller: Caller,
    token: u64,
    epoch: u64,
    sched: &mut Scheduler,
    caps: &CapabilityBroker,
) -> bool {
    let mut runtime = RUNTIME.lock();
    let Some(registration) = live_registration(&mut runtime, sched, caps) else {
        return false;
    };
    if registration.epoch != epoch {
        return false;
    }
    runtime
        .publish_file_close(registration, caller, token, sched.global_tick)
        .is_ok()
}

/// Hold SCHEDULER across the cwd or credential commit and profile withdrawal.
/// Resource destruction is deferred to syscall-return cleanup after unlock.
pub(crate) fn withdraw_root_profile(idx: usize, sched: &mut Scheduler) {
    RUNTIME.lock().withdraw_root_profile(idx, sched);
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
/// and broker guards as well. Returned resources pin receipt-backed bindings
/// until dropped; invoke reconcile_namespaces afterwards. Future published,
/// mapped operations additionally require their transport/mapping drain.
#[allow(dead_code)]
pub(super) fn reap_next() -> Option<Retired<Resources>> {
    let mut runtime = RUNTIME.lock();
    // Published continuations own terminal delivery; independently drain
    // abandoned operations without stealing a live caller's result.
    let id = runtime
        .pending
        .terminal_ids()
        .find(|id| !runtime.publications.iter().flatten().any(|p| p.id == *id));
    let Some(id) = id else {
        CLEANUP_PENDING.store(false, core::sync::atomic::Ordering::Release);
        return None;
    };
    let retired = runtime.pending.reap(id)?;
    Some(retired)
}

/// Syscall-return cleanup, outside scheduler/broker guards. The fast path is
/// one atomic read; caller retirement requests a bounded drain. Live published
/// continuations retain their results for the caller rather than this worker.
pub(crate) fn cleanup_abandoned() {
    #[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_1_test"))]
    spawn::fixture::dispose_orphan();
    if !CLEANUP_PENDING.load(core::sync::atomic::Ordering::Acquire) {
        return;
    }
    for _ in 0..64 {
        let Some(retired) = reap_next() else {
            break;
        };
        drop(retired);
        reconcile_namespaces();
    }
}

/// Result delivery and abandoned cleanup call after dropping reaped resources
/// and lifecycle guards. Record release obligations without sending or waiting.
#[allow(dead_code)]
pub(crate) fn reconcile_namespaces() {
    let mut runtime = RUNTIME.lock();
    runtime.reconcile_namespaces();
    runtime.collect_remote();
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

fn register_service(
    runtime: &mut Runtime,
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
    live_registration(runtime, sched, caps);
    let registration = runtime.register(candidate)?;
    #[cfg(feature = "vfs-n1j-test")]
    crate::vfs_n1j_test::bind(registration);
    let _ = registration;
    Ok(())
}

pub(super) fn register(
    caller_pid: usize,
    endpoint: u32,
    sched: &Scheduler,
    caps: &CapabilityBroker,
) -> Result<(), IpcError> {
    // Lock order: scheduler -> broker (when needed) -> VFS runtime. Never
    // acquire lifecycle locks or wait for IPC while holding RUNTIME.
    register_service(&mut RUNTIME.lock(), caller_pid, endpoint, sched, caps)
}

#[cfg(test)]
#[path = "vfs_registration/namespace_tests.rs"]
mod namespace_tests;

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
        let registration = runtime.register(owner()).unwrap();
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
        let first = runtime.register(owner()).unwrap();
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
        let second = runtime.register(new_owner).unwrap();
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
            let first = runtime.register(owner()).unwrap();
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
            let second = runtime.register(new_owner).unwrap();
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
        let first = runtime.register(owner()).unwrap();
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
        let second = runtime.register(owner()).unwrap();
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

    fn scope() -> Scope {
        Scope {
            caller: caller(),
            namespace_id: 9,
            namespace_generation: 10,
        }
    }

    #[test]
    fn lifecycle_abandons_exact_caller_across_scopes_without_releasing_local_users() {
        use sunlight_ipc::vfs_rpc::retention::Table as LeaseTable;
        for field in 0..4 {
            let mut runtime = Runtime::new();
            let registration = runtime.register(owner()).unwrap();
            let mut service = LeaseTable::<(), 4>::new(registration).unwrap();
            let mut receipts = alloc::vec::Vec::new();
            for id in 1..=4 {
                let mut s = scope();
                s.namespace_generation += id;
                runtime.prepare_retain(registration, s).unwrap();
                receipts.push(
                    service
                        .retain(registration, id, s, || Ok::<_, ()>(()))
                        .unwrap(),
                );
            }
            // Two successes have already won: one held with potential local
            // users, one releasing with a lost acknowledgement.
            for id in 3..=4 {
                let receipt = receipts[id - 1];
                assert_eq!(
                    runtime.retained(
                        registration,
                        id as u64,
                        &Frame::Retained(receipt).encode().unwrap()
                    ),
                    Ok(Retained::Install(receipt))
                );
            }
            runtime
                .remote_owner(registration)
                .unwrap()
                .release(4)
                .unwrap();
            let mut event = caller();
            match field {
                0 => event.pid += 1,
                1 => event.incarnation += 1,
                2 => event.address_space_generation += 1,
                _ => (),
            }
            runtime.retire_process(event);
            runtime.retire_process(event);
            assert_eq!(runtime.remote_owner(registration).unwrap().outstanding(), 4);
            assert_eq!(
                runtime
                    .remote_owner(registration)
                    .unwrap()
                    .releases()
                    .count(),
                1
            );
            for receipt in &receipts[..2] {
                assert_eq!(
                    runtime.retained(
                        registration,
                        receipt.request_id(),
                        &Frame::Retained(*receipt).encode().unwrap()
                    ),
                    Ok(if field == 3 {
                        Retained::Release(Frame::Release(*receipt))
                    } else {
                        Retained::Install(*receipt)
                    })
                );
            }
            // Neither caller nor service retirement proves local-user drain.
            runtime.retire_registration(registration);
            assert_eq!(runtime.remote_owner(registration).unwrap().outstanding(), 4);
            assert_eq!(
                runtime
                    .remote_owner(registration)
                    .unwrap()
                    .releases()
                    .count(),
                if field == 3 { 3 } else { 1 }
            );
        }
    }

    #[test]
    fn retired_remote_epochs_survive_replacement_and_authenticate_every_reply_field() {
        use sunlight_ipc::vfs_rpc::retention::Table as LeaseTable;
        for event in 0..3 {
            let mut runtime = Runtime::new();
            let first = runtime.register(owner()).unwrap();
            let mut service = LeaseTable::<(), 1>::new(first).unwrap();
            runtime.prepare_retain(first, scope()).unwrap();
            let receipt = service
                .retain(first, 1, scope(), || Ok::<_, ()>(()))
                .unwrap();
            // Lost success ACK, then service endpoint destruction/exit/exec.
            let service_caller = Caller {
                pid: first.owner.pid,
                incarnation: first.owner.incarnation,
                address_space_generation: first.owner.address_space_generation,
            };
            match event {
                0 => runtime.retire_endpoint(0, service_caller),
                1 => runtime.retire_process(service_caller),
                _ => runtime.retire_registration(first), // lazy liveness failure
            }
            assert_eq!(
                runtime.prepare_retain(first, scope()),
                Err(OwnershipError::State)
            );
            let second = runtime.register(owner()).unwrap();
            runtime.prepare_retain(second, scope()).unwrap(); // Same request ID, new epoch.
            let bytes = Frame::Retained(receipt).encode().unwrap();
            for field in 0..5 {
                let mut wrong = first;
                match field {
                    0 => wrong.owner.endpoint += 1,
                    1 => wrong.owner.pid += 1,
                    2 => wrong.owner.incarnation += 1,
                    3 => wrong.owner.address_space_generation += 1,
                    _ => wrong.epoch = second.epoch,
                }
                assert!(runtime.retained(wrong, 1, &bytes).is_err());
            }
            assert!(runtime.retained(second, 1, &bytes).is_err());
            assert!(runtime.retained(first, 1, &bytes[..111]).is_err());
            assert_eq!(runtime.remote_owner(first).unwrap().outstanding(), 1);
            assert_eq!(
                runtime.retained(first, 1, &bytes),
                Ok(Retained::Release(Frame::Release(receipt)))
            );
            assert_eq!(
                runtime.retained(first, 1, &bytes),
                Err(OwnershipError::State)
            );
            assert_eq!(runtime.remote_owner(first).unwrap().releases().count(), 1);
            runtime.retire_registration(first); // Stale hook cannot affect replacement.
            assert_eq!(runtime.registry.active(), Some(second));
            assert_eq!(runtime.remote_owner(second).unwrap().outstanding(), 1);
            service.release(first, receipt).unwrap();
            let released = Frame::Released(receipt).encode().unwrap();
            assert!(runtime.released(second, 1, &released).is_err());
            runtime.released(first, 1, &released).unwrap();
            assert!(runtime.remote_owner(first).is_err());
            assert_eq!(
                runtime.released(first, 1, &released),
                Err(OwnershipError::State)
            );
            assert_eq!(runtime.remote_owner(second).unwrap().outstanding(), 1);
        }
    }

    #[test]
    fn remote_capacity_is_reserved_before_registration_and_only_proven_replies_reclaim_it() {
        use sunlight_ipc::vfs_rpc::retention::wire::PendingRetain;
        let mut runtime = Runtime::new();
        let mut epochs = alloc::vec::Vec::new();
        for _ in 0..REMOTE_REGISTRATIONS {
            let registration = runtime.register(owner()).unwrap();
            assert_eq!(runtime.register(owner()), Ok(registration));
            let Frame::Retain { request_id, .. } =
                runtime.prepare_retain(registration, scope()).unwrap()
            else {
                panic!()
            };
            assert_eq!(request_id, 1);
            runtime.retire_registration(registration);
            assert_eq!(runtime.remote_owner(registration).unwrap().outstanding(), 1);
            epochs.push(registration);
        }
        assert_eq!(runtime.register(owner()), Err(IpcError::InvalidCapability));
        assert_eq!(runtime.registry.active(), None);
        assert_eq!(
            runtime.remote.iter().flatten().count(),
            REMOTE_REGISTRATIONS
        );
        let first = epochs[0];
        let refusal = Frame::Refused {
            request: PendingRetain::new(first, scope(), 1).unwrap(),
            reason: Refusal::Resolve,
        }
        .encode()
        .unwrap();
        assert!(runtime.refused(first, 2, &refusal).is_err());
        assert!(runtime.refused(epochs[1], 1, &refusal).is_err());
        assert_eq!(runtime.register(owner()), Err(IpcError::InvalidCapability));
        assert_eq!(runtime.refused(first, 1, &refusal), Ok(Refusal::Resolve));
        assert!(runtime.remote_owner(first).is_err());
        let replacement = runtime.register(owner()).unwrap();
        assert!(replacement.epoch > epochs.last().unwrap().epoch);
        // Clearing an active empty owner would reset its IDs within the epoch.
        runtime.prepare_retain(replacement, scope()).unwrap();
        let reply = Frame::Refused {
            request: PendingRetain::new(replacement, scope(), 1).unwrap(),
            reason: Refusal::Capacity,
        }
        .encode()
        .unwrap();
        runtime.refused(replacement, 1, &reply).unwrap();
        assert_eq!(runtime.register(owner()), Ok(replacement));
        let Frame::Retain { request_id, .. } =
            runtime.prepare_retain(replacement, scope()).unwrap()
        else {
            panic!()
        };
        assert_eq!(request_id, 2);
        assert!(runtime.refused(first, 1, &refusal).is_err());
        assert_eq!(runtime.remote_owner(replacement).unwrap().outstanding(), 1);
    }

    #[test]
    fn lost_retain_reply_at_service_exit_exhausts_epoch_slots() {
        use sunlight_ipc::vfs_rpc::retention::Table;
        let mut runtime = Runtime::new();
        for generation in 0..REMOTE_REGISTRATIONS {
            let mut identity = owner();
            identity.address_space_generation += generation as u64;
            let registration = runtime.register(identity).unwrap();
            let mut service = Table::<(), 1>::new(registration).unwrap();
            let Frame::Retain { request_id, .. } =
                runtime.prepare_retain(registration, scope()).unwrap()
            else {
                panic!()
            };
            service.retain(registration, request_id, scope(), || Ok::<_, ()>(())).unwrap();
            drop(service);
            runtime.retire_process(Caller {
                pid: identity.pid,
                incarnation: identity.incarnation,
                address_space_generation: identity.address_space_generation,
            });
            assert_eq!(runtime.remote_owner(registration).unwrap().outstanding(), 1);
        }
        assert!(runtime.registry.active().is_none());
        assert!(runtime.register(owner()).is_err());
    }

    #[test]
    fn exact_exit_completion_recovers_one_slot_at_full_capacity() {
        let mut runtime = Runtime::new();
        let mut retired = alloc::vec::Vec::new();
        for offset in 0..REMOTE_REGISTRATIONS as u64 {
            let registration = runtime
                .register(EndpointOwner {
                    pid: owner().pid + offset,
                    ..owner()
                })
                .unwrap();
            runtime.prepare_retain(registration, scope()).unwrap();
            runtime.retire_registration(registration);
            retired.push(registration);
        }
        assert!(runtime.register(owner()).is_err());
        let old = retired.remove(1);
        let image = crate::capability::delivery::RecipientIdentity {
            pid: old.owner.pid as usize,
            incarnation: old.owner.incarnation,
            vm_generation: old.owner.address_space_generation,
        };
        let completion = crate::sched::exit_proof(image, true, true, true).unwrap();
        runtime.complete_image_exit(completion);
        let replacement = runtime.register(owner()).unwrap();
        runtime.complete_image_exit(completion);
        assert_eq!(runtime.registry.active(), Some(replacement));
        assert!(runtime.remote_owner(old).is_err());
        for registration in retired {
            assert_eq!(runtime.remote_owner(registration).unwrap().outstanding(), 1);
        }
    }

    #[test]
    fn full_remote_lease_table_survives_idempotent_registration_and_retirement() {
        let mut runtime = Runtime::new();
        let registration = runtime.register(owner()).unwrap();
        for id in 1..=REMOTE_LEASES {
            let Frame::Retain { request_id, .. } =
                runtime.prepare_retain(registration, scope()).unwrap()
            else {
                panic!()
            };
            assert_eq!(request_id, id as u64);
        }
        assert_eq!(runtime.register(owner()), Ok(registration));
        assert_eq!(
            runtime.prepare_retain(registration, scope()),
            Err(OwnershipError::Capacity)
        );
        runtime.retire_process(caller());
        runtime.retire_registration(registration);
        assert_eq!(
            runtime.remote_owner(registration).unwrap().outstanding(),
            REMOTE_LEASES
        );
        assert_eq!(
            runtime
                .remote_owner(registration)
                .unwrap()
                .releases()
                .count(),
            0
        );
        let replacement = runtime.register(owner()).unwrap();
        assert_ne!(registration, replacement);
        assert_eq!(
            runtime.remote_owner(registration).unwrap().outstanding(),
            REMOTE_LEASES
        );
        let Frame::Retain { request_id, .. } =
            runtime.prepare_retain(replacement, scope()).unwrap()
        else {
            panic!()
        };
        assert_eq!(request_id, 1);
    }
}
