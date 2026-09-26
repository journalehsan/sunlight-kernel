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
}

impl Default for BrainIdentityBinding {
    fn default() -> Self {
        Self {
            mode: BrainIdentityMode::Disconnected,
            fingerprint: None,
            continuity_generation: None,
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
        self.fingerprint = Some(status.fingerprint);
        self.continuity_generation = Some(status.continuity_generation);
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
        }
    }

    #[test]
    fn rebind_same_identity_and_reject_changed_identity() {
        let mut binding = BrainIdentityBinding::default();
        binding.observe(Some(ready(*b"IDENTITY")));
        assert_eq!(binding.mode, BrainIdentityMode::Personalized);
        binding.disconnected();
        binding.observe(Some(ready(*b"IDENTITY")));
        assert_eq!(binding.mode, BrainIdentityMode::Personalized);
        binding.disconnected();
        binding.observe(Some(ready(*b"OTHERID!")));
        assert_eq!(binding.mode, BrainIdentityMode::SuspendedMismatch);
        binding.observe(None);
        assert_eq!(binding.mode, BrainIdentityMode::SuspendedMismatch);
    }
}
