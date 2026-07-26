//! Capability model for wiseowl-memorydb.

use crate::error::DbError;

/// Rights a caller may hold for long-term memory database operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DbCapability {
    InsertRecord = 0,
    ReadOwnScope = 1,
    ReadSharedScope = 2,
    ReadPayload = 3,
    QueryMetadata = 4,
    CreateRelationship = 5,
    Tombstone = 6,
    DeleteSource = 7,
    InspectStats = 8,
    CreateCheckpoint = 9,
    RunCompaction = 10,
    /// Assign elevated trust (ToolVerified / system trust labels).
    AssignElevatedTrust = 11,
    Admin = 12,
}

impl DbCapability {
    pub const fn bit(self) -> u64 {
        1u64 << (self as u8)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InsertRecord => "insert-record",
            Self::ReadOwnScope => "read-own-scope",
            Self::ReadSharedScope => "read-shared-scope",
            Self::ReadPayload => "read-payload",
            Self::QueryMetadata => "query-metadata",
            Self::CreateRelationship => "create-relationship",
            Self::Tombstone => "tombstone",
            Self::DeleteSource => "delete-source",
            Self::InspectStats => "inspect-stats",
            Self::CreateCheckpoint => "create-checkpoint",
            Self::RunCompaction => "run-compaction",
            Self::AssignElevatedTrust => "assign-elevated-trust",
            Self::Admin => "admin",
        }
    }
}

/// Set of granted capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct DbCapabilitySet {
    mask: u64,
}

impl DbCapabilitySet {
    pub const fn empty() -> Self {
        Self { mask: 0 }
    }

    pub const fn from_mask(mask: u64) -> Self {
        Self { mask }
    }

    pub const fn mask(self) -> u64 {
        self.mask
    }

    pub fn grant(mut self, cap: DbCapability) -> Self {
        self.mask |= cap.bit();
        self
    }

    pub fn has(self, cap: DbCapability) -> bool {
        self.mask & cap.bit() != 0 || self.mask & DbCapability::Admin.bit() != 0
    }

    /// Default unprivileged client.
    pub fn default_client() -> Self {
        Self::empty()
            .grant(DbCapability::InsertRecord)
            .grant(DbCapability::ReadOwnScope)
            .grant(DbCapability::QueryMetadata)
            .grant(DbCapability::CreateRelationship)
    }

    /// Diagnostic operator.
    pub fn diagnostic() -> Self {
        Self::default_client()
            .grant(DbCapability::InspectStats)
            .grant(DbCapability::ReadSharedScope)
            .grant(DbCapability::CreateCheckpoint)
    }

    /// Full administrative set.
    pub fn admin() -> Self {
        Self::from_mask(u64::MAX)
    }

    pub fn require(self, cap: DbCapability) -> Result<(), DbError> {
        if self.has(cap) {
            Ok(())
        } else {
            Err(DbError::PermissionDenied(cap.as_str()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cannot_compact() {
        let c = DbCapabilitySet::default_client();
        assert!(c.require(DbCapability::InsertRecord).is_ok());
        assert!(c.require(DbCapability::RunCompaction).is_err());
        assert!(c.require(DbCapability::ReadPayload).is_err());
        assert!(c.require(DbCapability::AssignElevatedTrust).is_err());
    }

    #[test]
    fn admin_has_all() {
        let a = DbCapabilitySet::admin();
        assert!(a.has(DbCapability::DeleteSource));
        assert!(a.has(DbCapability::RunCompaction));
    }
}
