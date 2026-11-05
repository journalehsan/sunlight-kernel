use super::channel::Error;

/// A catalog reference, never bearer authority. Resolution also requires the
/// kernel-observed service identity and its trusted embedded-image role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PolicyIdentity {
    pub policy_id: u64,
    pub generation: u64,
}

#[derive(Clone, Copy)]
struct PolicyRecord {
    identity: PolicyIdentity,
    owner: super::delivery::RecipientIdentity,
    service: Service,
}

/// Accessed only under the existing CAP_BROKER lifecycle serialization.
/// Generations are independent of ChannelTable's generation allocator.
pub(crate) struct PolicyCatalog {
    slots: [Option<PolicyRecord>; 8],
    next_generation: u64,
}

impl PolicyCatalog {
    #[cfg(all(debug_assertions, feature = "fastlane_hybrid_lifecycle_test"))]
    pub(super) fn scoped_count(&self, pids: [usize; 2]) -> usize {
        self.slots.iter().flatten().filter(|r| pids.contains(&r.owner.pid)).count()
    }
    pub const fn new() -> Self {
        Self { slots: [None; 8], next_generation: 1 }
    }

    // No syscall exposes issuance. The native adapter supplies a role taken
    // from the actual peer process, never from connection request bytes.
    pub fn for_service(
        &mut self,
        owner: super::delivery::RecipientIdentity,
        service: Service,
    ) -> Result<PolicyIdentity, Error> {
        if service == Service::Unspecified {
            return Err(Error::Denied);
        }
        if let Some(record) = self.slots.iter().flatten().find(|r| r.owner == owner) {
            return if record.service == service { Ok(record.identity) } else { Err(Error::Denied) };
        }
        let slot = self.slots.iter().position(Option::is_none).ok_or(Error::Capacity)?;
        let generation = self.next_generation;
        let next = generation.checked_add(1).ok_or(Error::Exhausted)?;
        let identity = PolicyIdentity { policy_id: slot as u64 + 1, generation };
        self.slots[slot] = Some(PolicyRecord { identity, owner, service });
        self.next_generation = next;
        Ok(identity)
    }

    pub fn resolve(
        &self,
        identity: PolicyIdentity,
        owner: super::delivery::RecipientIdentity,
        service: Service,
    ) -> Result<Policy, Error> {
        let slot = identity.policy_id.checked_sub(1)
            .and_then(|id| usize::try_from(id).ok()).ok_or(Error::Stale)?;
        let record = self.slots.get(slot).and_then(Option::as_ref).ok_or(Error::Stale)?;
        if record.identity != identity { return Err(Error::Stale); }
        if record.owner != owner || record.service != service { return Err(Error::Denied); }
        Ok(record.service.policy())
    }

    /// Exit and successful exec revoke references before PID/VM reuse.
    pub fn retire_process(&mut self, pid: usize) {
        for slot in &mut self.slots {
            if slot.is_some_and(|record| record.owner.pid == pid) { *slot = None; }
        }
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum Policy {
    KernelOnly = 0,
    FastLaneAllowed = 1,
    FastLaneRequired = 2,
}

impl TryFrom<u64> for Policy {
    type Error = Error;
    fn try_from(value: u64) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::KernelOnly),
            1 => Ok(Self::FastLaneAllowed),
            2 => Ok(Self::FastLaneRequired),
            _ => Err(Error::Denied),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum Service {
    Unspecified = 0,
    SystemControl = 1,
    AudioStream = 2,
    Protected = 3,
    RequiredStream = 4,
}

impl Service {
    /// Trusted fixture binding: version 1 TRANSFORM_16 requires the existing
    /// service operation permission. Wire bytes never supply that permission.
    pub fn wire_operation(self, version: u8, opcode: u16) -> Result<u64, Error> {
        if version != 1 || opcode != 0x10 {
            return Err(Error::Denied);
        }
        match self {
            Self::SystemControl => Ok(1),
            Self::AudioStream | Self::RequiredStream => Ok(2),
            Self::Protected => Ok(3),
            Self::Unspecified => Err(Error::Denied),
        }
    }

    pub fn embedded_path(path: &str) -> Option<Self> {
        match path {
            "/test/contract-system" => Some(Self::SystemControl),
            "/test/contract-audio" => Some(Self::AudioStream),
            "/test/contract-protected" => Some(Self::Protected),
            "/test/contract-required" => Some(Self::RequiredStream),
            _ => None,
        }
    }

    pub fn policy(self) -> Policy {
        match self {
            Self::Unspecified | Self::SystemControl | Self::Protected => Policy::KernelOnly,
            Self::AudioStream => Policy::FastLaneAllowed,
            Self::RequiredStream => Policy::FastLaneRequired,
        }
    }

    pub fn operation_authorized(self, operation: u64, privileged: bool) -> bool {
        match self {
            Self::Unspecified | Self::SystemControl => operation == 1,
            Self::AudioStream | Self::RequiredStream => operation == 2,
            Self::Protected => operation == 3 && privileged,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum Transport {
    Kernel = 1,
    FastLane = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum Phase {
    Idle = 0,
    Preparing = 1,
    Ready = 2,
    Failed = 3,
    Closed = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Contract {
    /// One operation per single-use hybrid channel. The target is the actual
    /// kernel IPC identity, never a token supplied by the Actor.
    pub hybrid_started: bool,
    pub hybrid_reply: Option<crate::process::IpcReplyTarget>,
    pub hybrid_endpoint: Option<u32>,
    /// Explicit fixture-only TRANSFORM_16 v1: required ring request, IPC reply.
    /// False preserves the existing connection-wide policy contract.
    pub hybrid_transform: bool,
    pub service: Service,
    pub policy_identity: Option<PolicyIdentity>,
    pub policy: Policy,
    pub operation: u64,
    pub transport: Option<Transport>,
    pub phase: Phase,
    pub fallback: bool,
}

impl Contract {
    pub const fn new() -> Self {
        Self {
            hybrid_started: false,
            hybrid_reply: None,
            hybrid_endpoint: None,
            hybrid_transform: false,
            service: Service::Unspecified,
            policy_identity: None,
            policy: Policy::KernelOnly,
            operation: 0,
            transport: None,
            phase: Phase::Idle,
            fallback: false,
        }
    }

    pub fn begin(
        &mut self,
        service: Service,
        request: u64,
        privileged: bool,
    ) -> Result<Transport, Error> {
        if self.phase != Phase::Idle {
            return Err(Error::Duplicate);
        }
        self.phase = Phase::Preparing;
        self.service = service;
        self.policy = service.policy();
        self.operation = (request >> 8) & 0xff;
        let offer = request & 0xff;
        self.hybrid_transform = request == 0x10202 && service == Service::RequiredStream;
        if (request >> 16 != 0 && !self.hybrid_transform)
            || offer > 2
            || !service.operation_authorized(self.operation, privileged)
        {
            return Err(Error::Denied);
        }
        let transport = match (self.policy, offer) {
            (Policy::KernelOnly, 2) => return Err(Error::Denied),
            (Policy::FastLaneRequired, 1) => return Err(Error::Incomplete),
            (Policy::KernelOnly, _) => Transport::Kernel,
            (Policy::FastLaneAllowed, 1) => {
                self.fallback = true;
                Transport::Kernel
            }
            _ => Transport::FastLane,
        };
        self.transport = Some(transport);
        Ok(transport)
    }

    pub fn finish(&mut self) -> Result<Transport, Error> {
        if self.phase != Phase::Preparing {
            return Err(if self.phase == Phase::Ready {
                Error::Duplicate
            } else {
                Error::Terminal
            });
        }
        let selected = self.transport.ok_or(Error::Incomplete)?;
        self.phase = Phase::Ready;
        Ok(selected)
    }

    pub fn retire(&mut self) -> Option<&'static str> {
        match self.phase {
            Phase::Preparing => {
                self.phase = Phase::Failed;
                Some("failure")
            }
            Phase::Ready => {
                self.phase = Phase::Closed;
                Some("closed")
            }
            _ => None,
        }
    }

    pub fn fastlane_setup(&self) -> Result<(), Error> {
        if self.phase == Phase::Preparing && self.transport == Some(Transport::FastLane) {
            Ok(())
        } else {
            Err(Error::Denied)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_directional_contract_fails_closed() {
        let mut c = Contract::new();
        assert_eq!(c.begin(Service::RequiredStream, 0x10202, false), Ok(Transport::FastLane));
        assert!(c.hybrid_transform);
        assert_eq!(c.finish(), Ok(Transport::FastLane));
        assert_eq!(c.begin(Service::RequiredStream, 0x202, false), Err(Error::Duplicate));
        assert!(c.hybrid_transform);
        c.retire();
        assert_eq!(c.phase, Phase::Closed);
        for service in [Service::SystemControl, Service::AudioStream, Service::Protected] {
            assert_eq!(Contract::new().begin(service, 0x10202, false), Err(Error::Denied));
        }
        for request in [0x10201, 0x10302, 0x20202, 0x10102, u64::MAX] {
            assert!(Contract::new().begin(Service::RequiredStream, request, false).is_err());
        }
        let mut old = Contract::new();
        old.begin(Service::RequiredStream, 0x202, false).unwrap();
        assert!(!old.hybrid_transform);
    }
    #[test]
    fn versioned_transform_binding_does_not_grant_permission() {
        use super::Service;
        assert_eq!(Service::AudioStream.wire_operation(1, 0x10), Ok(2));
        assert_eq!(Service::SystemControl.wire_operation(1, 0x10), Ok(1));
        assert!(Service::Unspecified.wire_operation(1, 0x10).is_err());
        assert!(Service::AudioStream.wire_operation(2, 0x10).is_err());
        assert!(Service::AudioStream.wire_operation(1, 0x11).is_err());
        let protected = Service::Protected.wire_operation(1, 0x10).unwrap();
        assert!(!Service::Protected.operation_authorized(protected, false));
        assert!(Service::Protected.operation_authorized(protected, true));
    }
    use super::*;

    fn principal(pid: usize) -> super::super::delivery::RecipientIdentity {
        super::super::delivery::RecipientIdentity { pid, incarnation: 7, vm_generation: 19 }
    }

    #[test]
    fn independent_catalog_revocation_and_foreign_binding() {
        let mut catalog = PolicyCatalog::new();
        let owner = principal(1);
        let first = catalog.for_service(owner, Service::AudioStream).unwrap();
        assert_eq!(catalog.for_service(owner, Service::AudioStream), Ok(first));
        assert_eq!(catalog.resolve(first, principal(2), Service::AudioStream), Err(Error::Denied));
        assert_eq!(catalog.resolve(first, owner, Service::RequiredStream), Err(Error::Denied));
        for changed in [super::super::delivery::RecipientIdentity { incarnation: 8, ..owner },
                        super::super::delivery::RecipientIdentity { vm_generation: 20, ..owner }] {
            assert_eq!(catalog.resolve(first, changed, Service::AudioStream), Err(Error::Denied));
        }
        catalog.retire_process(owner.pid);
        assert_eq!(catalog.resolve(first, owner, Service::AudioStream), Err(Error::Stale));
        let second = catalog.for_service(owner, Service::AudioStream).unwrap();
        assert_eq!(first.policy_id, second.policy_id);
        assert_ne!(first.generation, second.generation);
        assert_eq!(catalog.resolve(first, owner, Service::AudioStream), Err(Error::Stale));
        assert_eq!(catalog.resolve(second, owner, Service::AudioStream), Ok(Policy::FastLaneAllowed));
    }

    #[test]
    fn catalog_capacity_and_generation_exhaustion_fail_closed() {
        let mut catalog = PolicyCatalog::new();
        assert_eq!(catalog.for_service(principal(0), Service::Unspecified), Err(Error::Denied));
        for pid in 1..=8 { catalog.for_service(principal(pid), Service::AudioStream).unwrap(); }
        assert_eq!(catalog.for_service(principal(9), Service::AudioStream), Err(Error::Capacity));
        catalog.retire_process(1);
        catalog.next_generation = u64::MAX;
        assert_eq!(catalog.for_service(principal(9), Service::AudioStream), Err(Error::Exhausted));
        assert!(catalog.slots[0].is_none());
        assert_eq!(catalog.for_service(principal(9), Service::AudioStream), Err(Error::Exhausted));
    }

    #[test]
    fn policy_catalog_and_malformed_values_fail_closed() {
        for value in [3, 255, u64::MAX] {
            assert_eq!(Policy::try_from(value), Err(Error::Denied));
        }
        assert_eq!(Service::Unspecified.policy(), Policy::KernelOnly);
        assert_eq!(Service::embedded_path("/bin/contract-audio"), None);
        assert_eq!(Service::embedded_path("/test/contract-audio"), Some(Service::AudioStream));
    }

    #[test]
    fn independent_operation_authorization_precedes_transport() {
        for offer in 0..=2 {
            for privileged in [false, true] {
                let mut contract = Contract::new();
                assert_eq!(contract.begin(Service::AudioStream, 0x300 | offer, privileged), Err(Error::Denied));
            }
            let mut contract = Contract::new();
            assert_eq!(contract.begin(Service::Protected, 0x300 | offer, false), Err(Error::Denied));
        }
        let mut contract = Contract::new();
        assert_eq!(contract.begin(Service::Protected, 0x300, true), Ok(Transport::Kernel));
    }

    #[test]
    fn selection_matrix_and_frozen_lifetime() {
        for (service, operation) in [(Service::Unspecified, 1), (Service::SystemControl, 1), (Service::AudioStream, 2), (Service::RequiredStream, 2)] {
            for offer in 0..=2 {
                let mut contract = Contract::new();
                let selected = contract.begin(service, operation << 8 | offer, false);
                if service.policy() == Policy::KernelOnly && offer == 2 {
                    assert_eq!(selected, Err(Error::Denied));
                } else if service.policy() == Policy::FastLaneRequired && offer == 1 {
                    assert_eq!(selected, Err(Error::Incomplete));
                } else {
                    let transport = if service.policy() == Policy::KernelOnly || offer == 1 { Transport::Kernel } else { Transport::FastLane };
                    assert_eq!(selected, Ok(transport));
                    assert_eq!(contract.finish(), selected);
                    let frozen = contract;
                    assert_eq!(contract.begin(service, operation << 8 | 1, false), Err(Error::Duplicate));
                    assert_eq!(contract.finish(), Err(Error::Duplicate));
                    assert_eq!(contract, frozen);
                    assert_eq!(contract.fastlane_setup(), Err(Error::Denied));
                    assert_eq!(contract.retire(), Some("closed"));
                }
                if contract.phase == Phase::Preparing {
                    assert_eq!(contract.retire(), Some("failure"));
                }
                assert_eq!(contract.retire(), None);
                assert_eq!(contract.retire(), None);
            }
        }
    }

    #[test]
    fn client_policy_claims_and_partial_setup_never_downgrade() {
        for request in [0x20200, 0xffff0200, 0x203, u64::MAX] {
            let mut contract = Contract::new();
            assert_eq!(contract.begin(Service::AudioStream, request, false), Err(Error::Denied));
            assert_eq!(contract.retire(), Some("failure"));
        }
        let mut contract = Contract::new();
        contract.begin(Service::AudioStream, 0x200, false).unwrap();
        assert_eq!(contract.fastlane_setup(), Ok(()));
        assert_eq!(contract.begin(Service::AudioStream, 0x201, false), Err(Error::Duplicate));
        assert_eq!(contract.transport, Some(Transport::FastLane));
        assert_eq!(contract.retire(), Some("failure"));
        assert_eq!(contract.finish(), Err(Error::Terminal));
        assert_eq!(contract.begin(Service::AudioStream, 0x201, false), Err(Error::Duplicate));
    }
}
