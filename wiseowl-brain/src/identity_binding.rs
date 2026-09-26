//! Runtime-only binding to MemoryDB's validated persistent identity.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrainIdentityMode {
    Disconnected,
    GenericDegraded,
    Personalized,
    SuspendedMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrainIdentityBinding {
    pub mode: BrainIdentityMode,
    pub fingerprint: Option<[u8; 8]>,
    pub continuity_generation: Option<u64>,
    pub activation_fingerprint: Option<[u8; 8]>,
    #[doc(hidden)]
    pub connected: bool,
    #[doc(hidden)]
    pub endpoint_generation: Option<u64>,
    #[doc(hidden)]
    pub activation_suspended: bool,
}

impl Default for BrainIdentityBinding {
    fn default() -> Self {
        Self {
            mode: BrainIdentityMode::Disconnected,
            fingerprint: None,
            continuity_generation: None,
            activation_fingerprint: None,
            connected: false,
            endpoint_generation: None,
            activation_suspended: false,
        }
    }
}

impl BrainIdentityBinding {
    /// Disconnect invalidates cached authority but retains the observed ID solely for mismatch detection.
    pub fn disconnected(&mut self) {
        if self.mode == BrainIdentityMode::SuspendedMismatch {
            return;
        }
        self.mode = BrainIdentityMode::Disconnected;
        self.continuity_generation = None;
        // A failed query alone does not prove that the endpoint incarnation
        // changed. Only a new endpoint generation may clear activation pins.
    }

    pub fn observe_endpoint_generation(&mut self, generation: Option<u64>) {
        let Some(generation) = generation else { return; };
        if self.endpoint_generation.is_some_and(|old| old != generation) {
            self.connected = false;
            self.activation_suspended = false;
            self.continuity_generation = None;
            if self.mode != BrainIdentityMode::SuspendedMismatch {
                self.mode = BrainIdentityMode::GenericDegraded;
            }
        }
        self.endpoint_generation = Some(generation);
    }

    pub fn observe(&mut self, status: Option<wiseowl_memorydb::identity_status::IdentityStatus>) {
        use wiseowl_memorydb::identity_status::IdentityStatusState;
        if self.mode == BrainIdentityMode::SuspendedMismatch {
            return;
        }
        let Some(status) =
            status.filter(|status| status.validate() && status.state == IdentityStatusState::Ready)
        else {
            self.mode = BrainIdentityMode::GenericDegraded;
            self.continuity_generation = None;
            return;
        };
        if self
            .fingerprint
            .is_some_and(|old| old != status.fingerprint)
        {
            self.mode = BrainIdentityMode::SuspendedMismatch;
            self.continuity_generation = None;
            return;
        }
        if self.activation_suspended {
            self.mode = BrainIdentityMode::GenericDegraded;
            self.continuity_generation = None;
            return;
        }
        if self.connected
            && self.activation_fingerprint.is_some_and(|old| old != status.activation_fingerprint)
        {
            self.mode = BrainIdentityMode::GenericDegraded;
            self.continuity_generation = None;
            self.activation_suspended = true;
            return;
        }
        self.fingerprint = Some(status.fingerprint);
        self.continuity_generation = Some(status.continuity_generation);
        self.activation_fingerprint = Some(status.activation_fingerprint);
        self.connected = true;
        self.mode = BrainIdentityMode::Personalized;
    }

    pub fn fingerprint(&self) -> Option<[u8; 8]> {
        self.fingerprint
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiseowl_memorydb::identity_status::{IdentityStatus, IdentityStatusState};

    fn ready(fp: [u8; 8]) -> IdentityStatus {
        IdentityStatus {
            state: IdentityStatusState::Ready,
            fingerprint: fp,
            lineage_sequence: 1,
            continuity_generation: 1,
            genesis_kind: 1,
            identity_format_version: 1,
            validation_status: 1,
            persistence_available: true,
            activation_state: 3,
            activation_fingerprint: *b"ACTIV001",
        }
    }

    #[test]
    fn rebind_same_identity_and_reject_changed_identity() {
        let mut binding = BrainIdentityBinding::default();
        binding.observe(Some(ready(*b"IDENTITY")));
        assert_eq!(binding.mode, BrainIdentityMode::Personalized);
        binding.disconnected();
        assert_eq!(binding.continuity_generation, None);
        binding.observe(None);
        assert_eq!(binding.mode, BrainIdentityMode::GenericDegraded);
        binding.observe(Some(ready(*b"IDENTITY")));
        assert_eq!(binding.mode, BrainIdentityMode::Personalized);
        binding.disconnected();
        binding.observe(Some(ready(*b"OTHERID!")));
        assert_eq!(binding.mode, BrainIdentityMode::SuspendedMismatch);
        binding.observe(None);
        assert_eq!(binding.mode, BrainIdentityMode::SuspendedMismatch);
    }

    #[test]
    fn restart_has_no_authority_until_memorydb_is_queried_again() {
        let mut running = BrainIdentityBinding::default();
        running.observe(Some(ready(*b"IDENTITY")));
        let restarted = BrainIdentityBinding::default();
        assert_eq!(restarted.mode, BrainIdentityMode::Disconnected);
        assert_eq!(restarted.fingerprint(), None);
        assert_eq!(restarted.continuity_generation, None);
        let mut restarted = restarted;
        restarted.observe(None);
        assert_eq!(restarted.mode, BrainIdentityMode::GenericDegraded);
        restarted.observe(Some(ready(*b"IDENTITY")));
        assert_eq!(restarted.mode, BrainIdentityMode::Personalized);
        assert_eq!(restarted.continuity_generation, Some(1));
    }

    #[test]
    fn invalid_status_cannot_reauthorize_cached_identity() {
        let mut binding = BrainIdentityBinding::default();
        binding.observe(Some(ready(*b"IDENTITY")));
        let mut invalid = ready(*b"IDENTITY");
        invalid.persistence_available = false;
        binding.observe(Some(invalid));
        assert_eq!(binding.mode, BrainIdentityMode::GenericDegraded);
        assert_eq!(binding.continuity_generation, None);
        binding.observe(Some(ready(*b"OTHERID!")));
        assert_eq!(binding.mode, BrainIdentityMode::SuspendedMismatch);
    }

    #[test]
    fn activation_must_remain_active_and_stable_while_connected() {
        let mut binding = BrainIdentityBinding::default();
        binding.observe_endpoint_generation(Some(1));
        binding.observe(Some(ready(*b"IDENTITY")));
        let mut changed = ready(*b"IDENTITY");
        changed.activation_fingerprint = *b"ACTIV002";
        binding.observe(Some(changed));
        assert_eq!(binding.mode, BrainIdentityMode::GenericDegraded);
        assert_eq!(binding.continuity_generation, None);
        binding.observe(Some(ready(*b"IDENTITY")));
        assert_eq!(binding.mode, BrainIdentityMode::GenericDegraded);
        binding.disconnected();
        binding.observe_endpoint_generation(Some(2));
        changed = ready(*b"IDENTITY");
        changed.activation_fingerprint = *b"ACTIV002";
        binding.observe(Some(changed));
        assert_eq!(binding.mode, BrainIdentityMode::Personalized);

        let mut dormant = ready(*b"IDENTITY");
        dormant.activation_state = 1;
        let mut restarted = BrainIdentityBinding::default();
        restarted.observe(Some(dormant));
        assert_eq!(restarted.mode, BrainIdentityMode::GenericDegraded);
    }
}
