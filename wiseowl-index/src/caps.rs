//! Indexer capability bits.

/// Capability rights for indexer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u32)]
pub enum IndexCapability {
    RegisterRoot = 1 << 0,
    ListRoots = 1 << 1,
    ScanOwnRoots = 1 << 2,
    ScanSharedRoots = 1 << 3,
    ReadSourceFile = 1 << 4,
    InspectSourceMetadata = 1 << 5,
    RetryFailedSource = 1 << 6,
    RemoveSource = 1 << 7,
    ReindexSource = 1 << 8,
    InspectIndexerStats = 1 << 9,
    AdminIndexer = 1 << 10,
    TokenizeQuery = 1 << 11,
    SearchLexical = 1 << 12,
}

/// Bitset of capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexCapabilitySet {
    bits: u32,
}

impl IndexCapabilitySet {
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self { bits }
    }

    pub const fn bits(self) -> u32 {
        self.bits
    }

    pub fn grant(self, cap: IndexCapability) -> Self {
        Self {
            bits: self.bits | (cap as u32),
        }
    }

    pub fn has(self, cap: IndexCapability) -> bool {
        if self.bits & (IndexCapability::AdminIndexer as u32) != 0 {
            return true;
        }
        self.bits & (cap as u32) != 0
    }

    pub fn require(self, cap: IndexCapability) -> Result<(), crate::error::IndexError> {
        if self.has(cap) {
            Ok(())
        } else {
            Err(crate::error::IndexError::CapabilityDenied(cap.as_str()))
        }
    }

    /// Default client: list roots, scan own, inspect own sources, tokenize, lexical search.
    pub fn default_client() -> Self {
        Self::empty()
            .grant(IndexCapability::ListRoots)
            .grant(IndexCapability::ScanOwnRoots)
            .grant(IndexCapability::InspectSourceMetadata)
            .grant(IndexCapability::RetryFailedSource)
            .grant(IndexCapability::InspectIndexerStats)
            .grant(IndexCapability::TokenizeQuery)
            .grant(IndexCapability::SearchLexical)
            .grant(IndexCapability::RegisterRoot)
            .grant(IndexCapability::RemoveSource)
            .grant(IndexCapability::ReindexSource)
            .grant(IndexCapability::ReadSourceFile)
    }

    /// Full admin set.
    pub fn admin() -> Self {
        Self { bits: 0xFFFF_FFFF }
    }
}

impl IndexCapability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RegisterRoot => "RegisterRoot",
            Self::ListRoots => "ListRoots",
            Self::ScanOwnRoots => "ScanOwnRoots",
            Self::ScanSharedRoots => "ScanSharedRoots",
            Self::ReadSourceFile => "ReadSourceFile",
            Self::InspectSourceMetadata => "InspectSourceMetadata",
            Self::RetryFailedSource => "RetryFailedSource",
            Self::RemoveSource => "RemoveSource",
            Self::ReindexSource => "ReindexSource",
            Self::InspectIndexerStats => "InspectIndexerStats",
            Self::AdminIndexer => "AdminIndexer",
            Self::TokenizeQuery => "TokenizeQuery",
            Self::SearchLexical => "SearchLexical",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_has_all() {
        let a = IndexCapabilitySet::admin();
        assert!(a.has(IndexCapability::RegisterRoot));
        assert!(a.has(IndexCapability::AdminIndexer));
    }

    #[test]
    fn default_cannot_admin() {
        let c = IndexCapabilitySet::default_client();
        assert!(!c.has(IndexCapability::AdminIndexer));
        assert!(c.has(IndexCapability::ScanOwnRoots));
    }
}
