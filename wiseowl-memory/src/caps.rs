//! Capability model for wiseowl-memoryd.
//!
//! Fine-grained rights are enforced by the service on each request.
//! They map onto SunlightOS `ServiceCapability` style bitmasks when the
//! service is wired into the nameserver (Phase 0/1 host tests use local sets).

/// Rights a caller may hold for short-term memory operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MemoryCapability {
    Create = 0,
    ReadOwnSession = 1,
    ReadSharedSession = 2,
    Delete = 3,
    InspectMetadata = 4,
    InspectGlobalStats = 5,
    PromoteToKv = 6,
    RunMaintenance = 7,
    AdminQuota = 8,
    /// Explicit permission to read payload bytes (not just metadata).
    ReadPayload = 9,
}

impl MemoryCapability {
    pub const fn bit(self) -> u64 {
        1u64 << (self as u8)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::ReadOwnSession => "read-own-session",
            Self::ReadSharedSession => "read-shared-session",
            Self::Delete => "delete",
            Self::InspectMetadata => "inspect-metadata",
            Self::InspectGlobalStats => "inspect-global-stats",
            Self::PromoteToKv => "promote-to-kv",
            Self::RunMaintenance => "run-maintenance",
            Self::AdminQuota => "admin-quota",
            Self::ReadPayload => "read-payload",
        }
    }
}

/// Set of granted capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct CapabilitySet {
    mask: u64,
}

impl CapabilitySet {
    pub const fn empty() -> Self {
        Self { mask: 0 }
    }

    pub const fn from_mask(mask: u64) -> Self {
        Self { mask }
    }

    pub const fn mask(self) -> u64 {
        self.mask
    }

    pub fn grant(mut self, cap: MemoryCapability) -> Self {
        self.mask |= cap.bit();
        self
    }

    pub fn has(self, cap: MemoryCapability) -> bool {
        self.mask & cap.bit() != 0
    }

    /// Default unprivileged client: create + read/delete own session + inspect own metadata.
    pub fn default_client() -> Self {
        Self::empty()
            .grant(MemoryCapability::Create)
            .grant(MemoryCapability::ReadOwnSession)
            .grant(MemoryCapability::Delete)
            .grant(MemoryCapability::InspectMetadata)
    }

    /// Diagnostic operator: stats + list metadata, no payload by default.
    pub fn diagnostic() -> Self {
        Self::default_client()
            .grant(MemoryCapability::InspectGlobalStats)
            .grant(MemoryCapability::RunMaintenance)
            .grant(MemoryCapability::ReadSharedSession)
    }

    /// Full administrative set.
    pub fn admin() -> Self {
        Self::from_mask(u64::MAX)
    }

    pub fn require(self, cap: MemoryCapability) -> Result<(), crate::error::MemoryError> {
        if self.has(cap) {
            Ok(())
        } else {
            Err(crate::error::MemoryError::PermissionDenied(cap.as_str()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_client_cannot_global_stats() {
        let c = CapabilitySet::default_client();
        assert!(c.require(MemoryCapability::Create).is_ok());
        assert!(c
            .require(MemoryCapability::InspectGlobalStats)
            .is_err());
        assert!(c.require(MemoryCapability::ReadPayload).is_err());
    }

    #[test]
    fn admin_has_all() {
        let a = CapabilitySet::admin();
        assert!(a.has(MemoryCapability::AdminQuota));
        assert!(a.has(MemoryCapability::PromoteToKv));
    }
}
