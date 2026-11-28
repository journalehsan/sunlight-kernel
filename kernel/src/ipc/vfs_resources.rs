//! Kernel-owned preparation resources. No user mappings or transport authority.
//! Constructors are kernel-private; numeric wire objects are never sufficient
//! to acquire references. Receipt installation requires independently correlated
//! retention success; these helpers do not authenticate transport.
#![allow(dead_code)] // General file dispatch remains disabled; root-stat uses these resources.

use super::vfs_registration::AdmissionIdentity;
use alloc::{sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use sunlight_ipc::vfs_rpc::{
    codec::Request,
    context::{Context, Dispatch, Namespace, LOOKUP_METADATA},
    lifecycle::Caller,
    registration::Registration,
    response::METADATA_BYTES,
    retention::{Receipt, Scope},
    ProtocolError,
};

const MAX_CALLERS: usize = 64;
const CALLER_BYTES: usize = 64 * 1024;
const GLOBAL_BYTES: usize = 1024 * 1024;
const CALLER_REQUESTS: usize = 16;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Error {
    StaleNamespace,
    NamespaceCapacity,
    NamespaceExhausted,
    Quota,
    Allocation,
    Protocol(ProtocolError),
}

/// Local trusted-policy installation component. Keep one instance for the boot;
/// serialize access with caller/service lifecycle validation. This is not a
/// transport authenticator and must not be exposed as a syscall API. The runtime
/// supplies correlated receipts; the numeric installer remains a local scaffold.
pub(super) struct NamespacePolicy<const N: usize> {
    bindings: [Option<Arc<NamespaceBinding>>; N],
    next_generation: Option<u64>,
}

impl<const N: usize> NamespacePolicy<N> {
    pub(super) const fn new() -> Self {
        Self {
            bindings: [const { None }; N],
            next_generation: Some(1),
        }
    }

    /// Allocate a boot-unique scope before trusted service policy provisioning.
    /// Failed/abandoned preparation burns this generation; it is never reused.
    pub(super) fn reserve_scope(&mut self, identity: AdmissionIdentity) -> Result<Scope, Error> {
        let generation = self.next_generation.ok_or(Error::NamespaceExhausted)?;
        self.next_generation = generation.checked_add(1);
        Ok(Scope {
            caller: identity.caller,
            namespace_id: generation,
            namespace_generation: generation,
        })
    }

    /// Only the runtime's correlated retain-success path may call this. Both
    /// object fields name the receipt's pair generation: the root field selects
    /// its root member and the directory field its cwd member, never an inode.
    pub(super) fn install_receipt(
        &mut self,
        identity: AdmissionIdentity,
        receipt: Receipt,
        grants: u64,
    ) -> Result<Arc<NamespaceBinding>, Error> {
        self.publish_receipt(identity, receipt, grants, None)
    }

    pub(super) fn publish_receipt(
        &mut self,
        identity: AdmissionIdentity,
        receipt: Receipt,
        grants: u64,
        previous: Option<Scope>,
    ) -> Result<Arc<NamespaceBinding>, Error> {
        if receipt.registration() != identity.registration
            || receipt.scope().caller != identity.caller
        {
            return Err(Error::StaleNamespace);
        }
        let current = self.bindings.iter().position(|entry| {
            entry.as_ref().is_some_and(|binding| binding.caller == identity.caller)
        });
        let slot = match (previous, current) {
            (None, None) => self.bindings.iter().position(Option::is_none)
                .ok_or(Error::NamespaceCapacity)?,
            (Some(scope), Some(slot)) => {
                let current = self.bindings[slot].as_ref().unwrap().validate(identity)?;
                if scope.caller != identity.caller || scope.namespace_id != current.id
                    || scope.namespace_generation != current.generation
                    || receipt.scope().namespace_id != current.id
                    || receipt.scope().namespace_generation <= current.generation
                {
                    return Err(Error::StaleNamespace);
                }
                slot
            }
            _ => return Err(Error::StaleNamespace),
        };
        let scope = receipt.scope();
        let root = Directory::new(
            identity.registration,
            scope.namespace_id,
            scope.namespace_generation,
            receipt.generation(),
        )?;
        let directory = Directory::new(
            identity.registration,
            scope.namespace_id,
            scope.namespace_generation,
            receipt.generation(),
        )?;
        let binding = NamespaceBinding::new(identity.caller, root, directory, grants)?;
        if let Some(old) = self.bindings[slot].replace(Arc::clone(&binding)) {
            old.retire();
        }
        Ok(binding)
    }

    /// Explicitly reject replacement: withdrawal must invalidate the old binding
    /// before a caller can receive a new one. Failed construction burns its
    /// generation; no fallible operation follows insertion.
    pub(super) fn install(
        &mut self,
        identity: AdmissionIdentity,
        root_object: u64,
        directory_object: u64,
        grants: u64,
    ) -> Result<(), Error> {
        if self
            .bindings
            .iter()
            .flatten()
            .any(|b| b.caller == identity.caller)
        {
            return Err(Error::StaleNamespace);
        }
        let slot = self
            .bindings
            .iter()
            .position(Option::is_none)
            .ok_or(Error::NamespaceCapacity)?;
        let generation = self.next_generation.ok_or(Error::NamespaceExhausted)?;
        self.next_generation = generation.checked_add(1);
        // A boot-unique generation also supplies the local namespace identity.
        let root = Directory::new(identity.registration, generation, generation, root_object)?;
        let directory = Directory::new(
            identity.registration,
            generation,
            generation,
            directory_object,
        )?;
        let binding = NamespaceBinding::new(identity.caller, root, directory, grants)?;
        self.bindings[slot] = Some(binding);
        Ok(())
    }

    pub(super) fn lookup(
        &self,
        identity: AdmissionIdentity,
    ) -> Result<Arc<NamespaceBinding>, Error> {
        let binding = self
            .bindings
            .iter()
            .flatten()
            .find(|b| b.caller == identity.caller)
            .ok_or(Error::StaleNamespace)?;
        binding.validate(identity)?;
        Ok(Arc::clone(binding))
    }

    /// Withdrawal is scoped to the complete captured caller and registration;
    /// stale teardown must never withdraw a replacement service's binding.
    pub(super) fn withdraw(&mut self, identity: AdmissionIdentity) -> Result<(), Error> {
        let slot = self
            .bindings
            .iter()
            .position(|entry| {
                entry.as_ref().is_some_and(|b| {
                    b.caller == identity.caller && b.root.registration == identity.registration
                })
            })
            .ok_or(Error::StaleNamespace)?;
        let binding = self.bindings[slot].take().unwrap();
        binding.retire();
        Ok(())
    }

    /// Lifecycle withdrawal invalidates retained preparation references before
    /// dropping policy ownership. It never frees resources still owned by a
    /// pending operation. Exact identities protect replacements from late events.
    pub(super) fn retire_caller(&mut self, caller: Caller) {
        self.retire_matching(|binding| binding.caller == caller);
    }

    pub(super) fn retire_registration(&mut self, registration: Registration) {
        self.retire_matching(|binding| binding.root.registration == registration);
    }

    fn retire_matching(&mut self, matches: impl Fn(&NamespaceBinding) -> bool) {
        for slot in &mut self.bindings {
            if slot.as_ref().is_some_and(|binding| matches(binding)) {
                let binding = slot.take().unwrap();
                binding.retire();
            }
        }
    }
}

impl<const N: usize> Drop for NamespacePolicy<N> {
    fn drop(&mut self) {
        for binding in self.bindings.iter().flatten() {
            binding.retire();
        }
    }
}

/// One kernel-owned record for an opaque directory object. For receipt-backed
/// bindings the runtime owns its remote lease until all local users drain.
pub(super) struct Directory {
    registration: Registration,
    namespace_id: u64,
    namespace_generation: u64,
    object: u64,
    live: AtomicBool,
}

impl Directory {
    fn new(
        registration: Registration,
        namespace_id: u64,
        namespace_generation: u64,
        object: u64,
    ) -> Result<Arc<Self>, Error> {
        if [
            registration.epoch,
            registration.owner.pid,
            registration.owner.incarnation,
            registration.owner.address_space_generation,
            namespace_id,
            namespace_generation,
            object,
        ]
        .contains(&0)
        {
            return Err(Error::StaleNamespace);
        }
        Ok(Arc::new(Self {
            registration,
            namespace_id,
            namespace_generation,
            object,
            live: AtomicBool::new(true),
        }))
    }

    pub(super) fn retire(&self) {
        self.live.store(false, Ordering::Release);
    }
}

/// Installed only by trusted kernel namespace policy, never from syscall
/// object numbers. Pins both records even if the namespace binding is removed.
pub(super) struct NamespaceBinding {
    caller: Caller,
    root: Arc<Directory>,
    directory: Arc<Directory>,
    grants: u64,
    live: AtomicBool,
}

impl NamespaceBinding {
    /// Under runtime serialization, the runtime's final strong reference proves
    /// local drain after withdrawal. No weak references escape production code.
    pub(super) fn drained(binding: &Arc<Self>) -> bool {
        !binding.live.load(Ordering::Acquire) && Arc::strong_count(binding) == 1
    }

    fn new(
        caller: Caller,
        root: Arc<Directory>,
        directory: Arc<Directory>,
        grants: u64,
    ) -> Result<Arc<Self>, Error> {
        if [
            caller.pid,
            caller.incarnation,
            caller.address_space_generation,
        ]
        .contains(&0)
            || root.registration != directory.registration
            || root.namespace_id != directory.namespace_id
            || root.namespace_generation != directory.namespace_generation
            || grants & !LOOKUP_METADATA != 0
            || !root.live.load(Ordering::Acquire)
            || !directory.live.load(Ordering::Acquire)
        {
            return Err(Error::StaleNamespace);
        }
        Ok(Arc::new(Self {
            caller,
            root,
            directory,
            grants,
            live: AtomicBool::new(true),
        }))
    }

    pub(super) fn registration(&self) -> Registration {
        self.root.registration
    }

    pub(super) fn grants(&self) -> u64 {
        self.grants
    }

    pub(super) fn retire(&self) {
        self.live.store(false, Ordering::Release);
    }

    pub(super) fn validate(&self, identity: AdmissionIdentity) -> Result<Namespace, Error> {
        if self.caller != identity.caller
            || self.root.registration != identity.registration
            || !self.live.load(Ordering::Acquire)
            || !self.root.live.load(Ordering::Acquire)
            || !self.directory.live.load(Ordering::Acquire)
        {
            return Err(Error::StaleNamespace);
        }
        Ok(Namespace {
            id: self.root.namespace_id,
            generation: self.root.namespace_generation,
            root: self.root.object,
            directory: self.directory.object,
        })
    }
}

#[derive(Clone, Copy)]
struct Usage {
    caller: Caller,
    bytes: usize,
    requests: usize,
}
struct Ledger {
    callers: [Option<Usage>; MAX_CALLERS],
    bytes: usize,
}
static LEDGER: spin::Mutex<Ledger> = spin::Mutex::new(Ledger {
    callers: [None; MAX_CALLERS],
    bytes: 0,
});

#[cfg(test)]
pub(super) static TEST_LEDGER_LOCK: spin::Mutex<()> = spin::Mutex::new(());

#[cfg(test)]
pub(super) fn charged_bytes() -> usize {
    LEDGER.lock().bytes
}

#[cfg(all(not(test), debug_assertions, feature = "vfs_l1c_r5_test"))]
pub(super) fn caller_charge(caller: Caller) -> (usize, usize) {
    LEDGER
        .lock()
        .callers
        .iter()
        .flatten()
        .find(|usage| usage.caller == caller)
        .map_or((0, 0), |usage| (usage.bytes, usage.requests))
}

/// Non-cloneable reservation, acquired before buffer allocation or reference
/// cloning. One request reserves two object-reference slots and two buffers.
struct Charge {
    caller: Caller,
    bytes: usize,
}
impl Charge {
    fn acquire(caller: Caller, bytes: usize) -> Result<Self, Error> {
        let mut ledger = LEDGER.lock();
        let total = ledger.bytes.checked_add(bytes).ok_or(Error::Quota)?;
        if total > GLOBAL_BYTES {
            return Err(Error::Quota);
        }
        let index = ledger
            .callers
            .iter()
            .position(|u| u.is_some_and(|u| u.caller == caller))
            .or_else(|| ledger.callers.iter().position(Option::is_none))
            .ok_or(Error::Quota)?;
        let mut usage = ledger.callers[index].unwrap_or(Usage {
            caller,
            bytes: 0,
            requests: 0,
        });
        let caller_bytes = usage.bytes.checked_add(bytes).ok_or(Error::Quota)?;
        if caller_bytes > CALLER_BYTES || usage.requests >= CALLER_REQUESTS {
            return Err(Error::Quota);
        }
        usage.bytes = caller_bytes;
        usage.requests += 1;
        ledger.callers[index] = Some(usage);
        ledger.bytes = total;
        Ok(Self { caller, bytes })
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        let mut ledger = LEDGER.lock();
        let index = ledger
            .callers
            .iter()
            .position(|u| u.is_some_and(|u| u.caller == self.caller))
            .expect("VFS charge owner disappeared");
        let usage = ledger.callers[index]
            .as_mut()
            .expect("VFS charge slot disappeared");
        usage.bytes -= self.bytes;
        usage.requests -= 1;
        if usage.requests == 0 {
            assert_eq!(usage.bytes, 0);
            ledger.callers[index] = None;
        }
        ledger.bytes -= self.bytes;
    }
}

/// Move into the pending table. Fields are private so input cannot be mutated
/// after construction and output cannot alias it. Neither buffer is mapped.
pub(super) struct Resources {
    identity: AdmissionIdentity,
    namespace: Arc<NamespaceBinding>,
    input: Vec<u8>,
    output: Vec<u8>,
    // Last field: backing and references drop before their accounting charge.
    _charge: Charge,
}

impl Resources {
    /// Admission owns ID allocation and resource construction as one transaction.
    /// Runtime callers must hold caller/service and namespace lifecycle
    /// serialization across revalidation and this operation. No IPC is sent.
    pub(super) fn admit_stat<const N: usize>(
        table: &mut sunlight_ipc::vfs_rpc::pending_table::Table<Self, N>,
        identity: AdmissionIdentity,
        namespace: &Arc<NamespaceBinding>,
        path: &str,
    ) -> Result<
        sunlight_ipc::vfs_rpc::Transfer,
        sunlight_ipc::vfs_rpc::pending_table::PrepareError<Error>,
    > {
        table.admit_with(
            identity.caller,
            identity.registration.service(),
            Request::Stat { path },
            METADATA_BYTES as u64,
            |transfer| Self::stat(identity, namespace, transfer.words()[1], path),
        )
    }

    fn stat(
        identity: AdmissionIdentity,
        namespace: &Arc<NamespaceBinding>,
        request_id: u64,
        path: &str,
    ) -> Result<Self, Error> {
        let dispatch = Dispatch {
            request_id,
            context: Context {
                caller: identity.caller,
                service: identity.registration.service(),
                namespace: namespace.validate(identity)?,
                uid: identity.uid,
                gid: identity.gid,
                groups: &identity.groups,
                grants: namespace.grants,
            },
            request: Request::Stat { path },
        };
        // Validate before allocating, using the same production frame codec.
        dispatch
            .transfer(
                sunlight_ipc::vfs_rpc::REQUEST,
                [
                    sunlight_ipc::vfs_rpc::VERSION,
                    request_id,
                    0,
                    METADATA_BYTES as u64,
                ],
                4,
                2,
                METADATA_BYTES as u64,
            )
            .map_err(Error::Protocol)?;
        let input_len = dispatch.encoded_len();
        let bytes = input_len.checked_add(METADATA_BYTES).ok_or(Error::Quota)?;
        let charge = Charge::acquire(identity.caller, bytes)?;
        let mut input = buffer(input_len)?;
        dispatch.encode(&mut input).map_err(Error::Protocol)?;
        let output = buffer(METADATA_BYTES)?;
        Ok(Self {
            identity,
            namespace: Arc::clone(namespace),
            input,
            output,
            _charge: charge,
        })
    }

    /// Invoke under namespace-policy serialization along with registration
    /// revalidation and admission/publication. This check is not a lifetime lock.
    pub(super) fn revalidate_namespace(&self) -> Result<(), Error> {
        self.namespace.validate(self.identity).map(|_| ())
    }

    pub(super) fn matches_stat_path(&self, path: &str) -> bool {
        let mut groups = [0; sunlight_ipc::vfs_rpc::context::MAX_GROUPS];
        sunlight_ipc::vfs_rpc::context::decode(&self.input, &mut groups)
            .is_ok_and(|d| d.request == Request::Stat { path })
    }

    pub(super) fn caller(&self) -> Caller {
        self.identity.caller
    }

    pub(super) fn input(&self) -> &[u8] {
        &self.input
    }
    pub(super) fn output(&mut self) -> &mut [u8] {
        &mut self.output
    }
}

fn buffer(bytes: usize) -> Result<Vec<u8>, Error> {
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(bytes)
        .map_err(|_| Error::Allocation)?;
    buffer.resize(bytes, 0);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sunlight_ipc::vfs_rpc::{context, registration::EndpointOwner};

    fn identity() -> AdmissionIdentity {
        AdmissionIdentity {
            caller: Caller {
                pid: 40,
                incarnation: 41,
                address_space_generation: 42,
            },
            uid: 1000,
            gid: 100,
            groups: [],
            registration: Registration {
                owner: EndpointOwner {
                    endpoint: 0,
                    pid: 3,
                    incarnation: 4,
                    address_space_generation: 5,
                },
                epoch: 6,
            },
        }
    }

    #[test]
    fn trusted_policy_capacity_withdrawal_and_nonreusing_generations() {
        let id = identity();
        let mut policy = NamespacePolicy::<1>::new();
        assert_eq!(
            policy.install(id, 0, 10, LOOKUP_METADATA),
            Err(Error::StaleNamespace)
        );
        assert!(policy.lookup(id).is_err());
        policy.install(id, 9, 10, LOOKUP_METADATA).unwrap();
        let old = policy.lookup(id).unwrap();
        let scope = old.validate(id).unwrap();
        assert_eq!(scope.generation, 2); // Failed construction burned 1.
        assert_eq!(policy.install(id, 9, 10, 0), Err(Error::StaleNamespace));
        let mut other = id;
        other.caller.address_space_generation += 1;
        assert_eq!(
            policy.install(other, 9, 10, 0),
            Err(Error::NamespaceCapacity)
        );
        assert!(policy.lookup(other).is_err());
        let mut stale = id;
        stale.registration.epoch += 1;
        assert!(policy.lookup(stale).is_err());
        assert_eq!(policy.withdraw(stale), Err(Error::StaleNamespace));
        assert_eq!(old.validate(id), Ok(scope));
        policy.withdraw(id).unwrap();
        assert_eq!(old.validate(id), Err(Error::StaleNamespace));
        assert!(policy.lookup(id).is_err());
        // Identical remote object numbers do not recreate the old scope.
        policy.install(stale, 9, 10, 0).unwrap();
        let replacement = policy.lookup(stale).unwrap();
        assert!(replacement.validate(stale).unwrap().generation > scope.generation);
        assert_eq!(policy.withdraw(id), Err(Error::StaleNamespace));
        drop(policy);
        assert_eq!(replacement.validate(stale), Err(Error::StaleNamespace));

        let mut exhausted = NamespacePolicy::<1>::new();
        exhausted.next_generation = Some(u64::MAX);
        exhausted.install(id, 9, 10, 0).unwrap();
        assert_eq!(
            exhausted
                .lookup(id)
                .unwrap()
                .validate(id)
                .unwrap()
                .generation,
            u64::MAX
        );
        exhausted.withdraw(id).unwrap();
        assert_eq!(
            exhausted.install(id, 9, 10, 0),
            Err(Error::NamespaceExhausted)
        );
        assert!(exhausted.lookup(id).is_err());
        assert_eq!(
            NamespacePolicy::<0>::new().install(id, 9, 10, 0),
            Err(Error::NamespaceCapacity)
        );
        let mut reserved = NamespacePolicy::<1>::new();
        reserved.next_generation = Some(u64::MAX);
        assert_eq!(
            reserved.reserve_scope(id).unwrap().namespace_generation,
            u64::MAX
        );
        assert_eq!(reserved.reserve_scope(id), Err(Error::NamespaceExhausted));
        assert!(reserved.lookup(id).is_err());
    }

    #[test]
    fn retained_resources_authentication_accounting_and_cleanup() {
        let _serial = TEST_LEDGER_LOCK.lock();
        let id = identity();
        let root = Directory::new(id.registration, 7, 8, 9).unwrap();
        let dir = Directory::new(id.registration, 7, 8, 10).unwrap();
        let namespace =
            NamespaceBinding::new(id.caller, root.clone(), dir.clone(), LOOKUP_METADATA).unwrap();
        let mut resources = Resources::stat(id, &namespace, 11, "../file").unwrap();
        let mut groups = [0; context::MAX_GROUPS];
        let decoded = context::decode(resources.input(), &mut groups).unwrap();
        assert_eq!(
            decoded.context.namespace,
            Namespace {
                id: 7,
                generation: 8,
                root: 9,
                directory: 10
            }
        );
        assert_eq!(decoded.context.uid, 1000);
        assert_eq!(decoded.request, Request::Stat { path: "../file" });
        assert!(resources.output().iter().all(|byte| *byte == 0));
        let original = resources.input().to_vec();
        resources.output().fill(255);
        assert_eq!(resources.input(), original);
        let charged = LEDGER.lock().bytes;
        assert_eq!(charged, resources.input.len() + METADATA_BYTES);
        // Malformed input never changes accounting.
        assert!(Resources::stat(id, &namespace, 0, "/").is_err());
        assert!(Resources::stat(id, &namespace, 12, "a\0b").is_err());
        assert_eq!(LEDGER.lock().bytes, charged);
        for field in 0..3 {
            let mut stale = id;
            match field {
                0 => stale.caller.incarnation += 1,
                1 => stale.caller.address_space_generation += 1,
                _ => stale.registration.epoch += 1,
            }
            assert!(matches!(
                Resources::stat(stale, &namespace, 12, "/"),
                Err(Error::StaleNamespace)
            ));
        }
        let other = Directory::new(id.registration, 7, 99, 10).unwrap();
        assert!(NamespaceBinding::new(id.caller, root.clone(), other, 0).is_err());
        assert!(NamespaceBinding::new(id.caller, root.clone(), dir.clone(), 2).is_err());
        let mut held = Vec::new();
        for n in 0..CALLER_REQUESTS - 1 {
            held.push(Resources::stat(id, &namespace, 20 + n as u64, "/").unwrap());
        }
        assert!(matches!(
            Resources::stat(id, &namespace, 99, "/"),
            Err(Error::Quota)
        ));
        drop(held);
        assert_eq!(LEDGER.lock().bytes, charged);
        assert!(matches!(
            Charge::acquire(id.caller, CALLER_BYTES + 1),
            Err(Error::Quota)
        ));
        assert!(matches!(
            Charge::acquire(id.caller, usize::MAX),
            Err(Error::Quota)
        ));
        // Withdrawal prevents new preparation but does not destroy retained data.
        namespace.retire();
        assert_eq!(resources.revalidate_namespace(), Err(Error::StaleNamespace));
        assert!(matches!(
            Resources::stat(id, &namespace, 12, "/"),
            Err(Error::StaleNamespace)
        ));
        assert_eq!(resources.input(), original);
        assert_eq!(Arc::strong_count(&namespace), 2);
        drop(namespace);
        drop(resources);
        assert_eq!(Arc::strong_count(&root), 1);
        assert_eq!(Arc::strong_count(&dir), 1);
        assert_eq!(LEDGER.lock().bytes, 0);
        assert!(LEDGER.lock().callers.iter().all(Option::is_none));

        // The actual pending table keeps the charge until terminal reap/drop.
        use sunlight_ipc::vfs_rpc::{lifecycle::Failure, pending_table::Table};
        let root2 = Directory::new(id.registration, 17, 18, 19).unwrap();
        let binding = NamespaceBinding::new(id.caller, root2.clone(), root2, 0).unwrap();
        let resources = Resources::stat(id, &binding, 1, "/").unwrap();
        let mut full = Table::<Resources, 0>::new();
        let (_, returned) = full
            .admit(
                id.caller,
                id.registration.service(),
                Request::Stat { path: "/" },
                METADATA_BYTES as u64,
                resources,
            )
            .err()
            .unwrap();
        assert!(LEDGER.lock().bytes > 0);
        drop(returned);
        assert_eq!(LEDGER.lock().bytes, 0);
        let resources = Resources::stat(id, &binding, 1, "/").unwrap();
        let mut table = Table::<Resources, 1>::new();
        let transfer = table
            .admit(
                id.caller,
                id.registration.service(),
                Request::Stat { path: "/" },
                METADATA_BYTES as u64,
                resources,
            )
            .ok()
            .unwrap();
        assert!(table.reap(transfer.words()[1]).is_none());
        table.fail(transfer.words()[1], Failure::Cancelled).unwrap();
        assert!(LEDGER.lock().bytes > 0);
        let retired = table.reap(transfer.words()[1]).unwrap();
        assert!(table.reap(transfer.words()[1]).is_none());
        assert!(LEDGER.lock().bytes > 0);
        drop(retired);
        assert_eq!(LEDGER.lock().bytes, 0);
        assert_eq!(buffer(usize::MAX), Err(Error::Allocation));

        // Admission frames the exact allocated ID, including after rollback.
        let mut table = Table::<Resources, 1>::new();
        binding.retire();
        assert!(Resources::admit_stat(&mut table, id, &binding, "/").is_err());
        assert!(table.is_empty());
        assert_eq!(LEDGER.lock().bytes, 0);
        let root3 = Directory::new(id.registration, 27, 28, 29).unwrap();
        let binding = NamespaceBinding::new(id.caller, root3.clone(), root3, 0).unwrap();
        // Force a quota failure after table ID allocation.
        let quota = Charge::acquire(id.caller, CALLER_BYTES).unwrap();
        assert!(matches!(
            Resources::admit_stat(&mut table, id, &binding, "/"),
            Err(sunlight_ipc::vfs_rpc::pending_table::PrepareError::Resource(Error::Quota))
        ));
        assert!(table.is_empty());
        drop(quota);
        let transfer = Resources::admit_stat(&mut table, id, &binding, "../exact").unwrap();
        assert_eq!(transfer.words()[1], 3);
        let charged = LEDGER.lock().bytes;
        assert!(Resources::admit_stat(&mut table, id, &binding, "/").is_err());
        assert_eq!(LEDGER.lock().bytes, charged);
        table
            .fail(transfer.words()[1], Failure::SubmissionFailed)
            .unwrap();
        let retired = table.reap(transfer.words()[1]).unwrap();
        let decoded = context::decode(retired.resources.input(), &mut groups).unwrap();
        assert_eq!(decoded.request_id, transfer.words()[1]);
        assert_eq!(decoded.request, Request::Stat { path: "../exact" });
        assert_eq!(LEDGER.lock().bytes, charged);
        drop(retired);
        assert_eq!(LEDGER.lock().bytes, 0);

        // Global capacity and owner slots remain charged across caller reuse.
        let mut charges = Vec::new();
        for n in 0..GLOBAL_BYTES / CALLER_BYTES {
            let mut caller = id.caller;
            caller.incarnation += n as u64;
            charges.push(Charge::acquire(caller, CALLER_BYTES).unwrap());
        }
        assert!(matches!(Charge::acquire(id.caller, 1), Err(Error::Quota)));
        charges.clear();
        for n in 0..MAX_CALLERS {
            let mut caller = id.caller;
            caller.incarnation += n as u64;
            charges.push(Charge::acquire(caller, 1).unwrap());
        }
        let mut extra = id.caller;
        extra.pid += 1;
        assert!(matches!(Charge::acquire(extra, 1), Err(Error::Quota)));
        drop(charges);
        root.retire();
        assert!(NamespaceBinding::new(id.caller, root, dir, 0).is_err());
        assert_eq!(LEDGER.lock().bytes, 0);

        // Installed policy bindings feed transactional admission. Withdrawal
        // prevents new admission while the terminal request retains its bytes.
        let mut policy = NamespacePolicy::<1>::new();
        policy.install(id, 9, 10, LOOKUP_METADATA).unwrap();
        let binding = policy.lookup(id).unwrap();
        let mut table = Table::<Resources, 2>::new();
        let transfer = Resources::admit_stat(&mut table, id, &binding, "/retained").unwrap();
        let charged = LEDGER.lock().bytes;
        policy.withdraw(id).unwrap();
        assert!(Resources::admit_stat(&mut table, id, &binding, "/denied").is_err());
        assert_eq!(LEDGER.lock().bytes, charged);
        table.fail(transfer.words()[1], Failure::Cancelled).unwrap();
        let retired = table.reap(transfer.words()[1]).unwrap();
        assert_eq!(
            retired.resources.revalidate_namespace(),
            Err(Error::StaleNamespace)
        );
        assert_eq!(
            context::decode(retired.resources.input(), &mut groups)
                .unwrap()
                .request,
            Request::Stat { path: "/retained" }
        );
        assert_eq!(LEDGER.lock().bytes, charged);
        drop(retired);
        assert_eq!(LEDGER.lock().bytes, 0);

        // Lifecycle invalidation preserves pending ownership and the boot-lived
        // policy's generation allocator across registration replacements.
        let mut generation = 1; // Initial binding above consumed generation 1.
        for service_retired in [false, true] {
            let mut replacement = id;
            replacement.registration.epoch += generation;
            policy.install(replacement, 9, 10, LOOKUP_METADATA).unwrap();
            let retained_binding = policy.lookup(replacement).unwrap();
            let namespace = retained_binding.validate(replacement).unwrap();
            assert!(namespace.generation > generation);
            generation = namespace.generation;
            let weak = Arc::downgrade(&retained_binding);
            let mut pending = Table::<Resources, 1>::new();
            let transfer =
                Resources::admit_stat(&mut pending, replacement, &retained_binding, "/lifecycle")
                    .unwrap();
            let charged = LEDGER.lock().bytes;
            let reason = if service_retired {
                policy.retire_registration(replacement.registration);
                policy.retire_registration(replacement.registration);
                Failure::ServiceRetired
            } else {
                policy.retire_caller(replacement.caller);
                policy.retire_caller(replacement.caller);
                Failure::CallerRetired
            };
            assert!(policy.lookup(replacement).is_err());
            assert_eq!(
                retained_binding.validate(replacement),
                Err(Error::StaleNamespace)
            );
            assert!(
                Resources::admit_stat(&mut pending, replacement, &retained_binding, "/denied")
                    .is_err()
            );
            drop(retained_binding);
            assert!(weak.upgrade().is_some());
            assert_eq!(LEDGER.lock().bytes, charged);
            assert!(pending.reap(transfer.words()[1]).is_none());
            pending.fail(transfer.words()[1], reason).unwrap();
            let retired = pending.reap(transfer.words()[1]).unwrap();
            assert_eq!(
                retired.resources.revalidate_namespace(),
                Err(Error::StaleNamespace)
            );
            assert_eq!(
                context::decode(retired.resources.input(), &mut groups)
                    .unwrap()
                    .context
                    .namespace,
                namespace
            );
            assert_eq!(LEDGER.lock().bytes, charged);
            drop(retired);
            assert!(weak.upgrade().is_none());
            assert_eq!(LEDGER.lock().bytes, 0);
        }
    }
}
