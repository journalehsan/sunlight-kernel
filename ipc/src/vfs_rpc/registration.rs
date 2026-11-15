//! Registration lifetime model. Kernel integration authenticates the candidate
//! and checks incumbent liveness before invoking these transitions.
use super::lifecycle::{RequestIds, Service};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndpointOwner {
    pub endpoint: u64,
    pub pid: u64,
    pub incarnation: u64,
    pub address_space_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration {
    pub owner: EndpointOwner,
    pub epoch: u64,
}
impl Registration {
    pub const fn service(self) -> Service {
        Service {
            endpoint: self.owner.endpoint,
            owner_pid: self.owner.pid,
            owner_incarnation: self.owner.incarnation,
            // Historical field name; this is the service registration epoch,
            // distinct from namespace and mount generations.
            mount_epoch: self.epoch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidIdentity,
    Occupied,
    Exhausted,
}

#[derive(Debug)]
pub struct Registry {
    active: Option<Registration>,
    epochs: RequestIds,
}
impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}
impl Registry {
    pub const fn new() -> Self {
        Self {
            active: None,
            epochs: RequestIds::new(),
        }
    }
    pub const fn active(&self) -> Option<Registration> {
        self.active
    }

    pub fn register(&mut self, owner: EndpointOwner) -> Result<Registration, Error> {
        if [owner.pid, owner.incarnation, owner.address_space_generation].contains(&0) {
            return Err(Error::InvalidIdentity);
        }
        if let Some(active) = self.active {
            return if active.owner == owner {
                Ok(active)
            } else {
                Err(Error::Occupied)
            };
        }
        let registration = Registration {
            owner,
            epoch: self.epochs.allocate().ok_or(Error::Exhausted)?,
        };
        self.active = Some(registration);
        Ok(registration)
    }

    /// An old retirement event must not withdraw a replacement registration.
    pub fn retire(&mut self, expected: Registration) -> bool {
        if self.active != Some(expected) {
            return false;
        }
        self.active = None;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const OWNER: EndpointOwner = EndpointOwner {
        endpoint: 8,
        pid: 3,
        incarnation: 5,
        address_space_generation: 2,
    };
    #[test]
    fn idempotence_conflicts_and_replacement_epoch() {
        let mut registry = Registry::new();
        let first = registry.register(OWNER).unwrap();
        assert_eq!(registry.register(OWNER), Ok(first));
        assert_eq!(first.service().mount_epoch, first.epoch);
        for owner in [
            EndpointOwner {
                endpoint: 9,
                ..OWNER
            },
            EndpointOwner { pid: 4, ..OWNER },
            EndpointOwner {
                incarnation: 6,
                ..OWNER
            },
            EndpointOwner {
                address_space_generation: 3,
                ..OWNER
            },
        ] {
            assert_eq!(registry.register(owner), Err(Error::Occupied));
            assert_eq!(registry.active(), Some(first));
        }
        assert!(registry.retire(first));
        let second = registry.register(OWNER).unwrap();
        assert!(second.epoch > first.epoch);
        assert_ne!(second.service(), first.service());
        assert!(!registry.retire(first));
        assert_eq!(registry.active(), Some(second));
    }
    #[test]
    fn malformed_candidate_does_not_replace_live_registration() {
        let mut registry = Registry::new();
        let first = registry.register(OWNER).unwrap();
        for owner in [
            EndpointOwner { pid: 0, ..OWNER },
            EndpointOwner {
                incarnation: 0,
                ..OWNER
            },
            EndpointOwner {
                address_space_generation: 0,
                ..OWNER
            },
        ] {
            assert_eq!(registry.register(owner), Err(Error::InvalidIdentity));
            assert_eq!(registry.active(), Some(first));
        }
    }

    #[test]
    fn first_endpoint_allocation_is_zero_and_can_own_the_service() {
        let mut registry = Registry::new();
        let owner = EndpointOwner {
            endpoint: 0,
            ..OWNER
        };
        assert_eq!(registry.register(owner).unwrap().service().endpoint, 0);
    }
}
