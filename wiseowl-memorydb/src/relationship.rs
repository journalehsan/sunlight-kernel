//! Directed typed relationships between long-term records.

use wiseowl_memory::MemoryId;

use crate::codec::{BufReader, BufWriter};
use crate::error::DbError;
use crate::provenance::RelationshipProvenance;

/// Relationship edge kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum RelationshipKind {
    DerivedFrom = 1,
    Supports = 2,
    Contradicts = 3,
    Supersedes = 4,
    RelatedTo = 5,
    AppliesTo = 6,
    ProducedBy = 7,
}

impl RelationshipKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::DerivedFrom),
            2 => Some(Self::Supports),
            3 => Some(Self::Contradicts),
            4 => Some(Self::Supersedes),
            5 => Some(Self::RelatedTo),
            6 => Some(Self::AppliesTo),
            7 => Some(Self::ProducedBy),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DerivedFrom => "derived_from",
            Self::Supports => "supports",
            Self::Contradicts => "contradicts",
            Self::Supersedes => "supersedes",
            Self::RelatedTo => "related_to",
            Self::AppliesTo => "applies_to",
            Self::ProducedBy => "produced_by",
        }
    }
}

/// Directed relationship between two memory records.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct MemoryRelationship {
    pub source: MemoryId,
    pub target: MemoryId,
    pub kind: RelationshipKind,
    pub confidence: u16,
    pub created_at_ns: u64,
    pub provenance: RelationshipProvenance,
    /// Soft-deleted edge (historical reference may remain).
    pub tombstoned: bool,
}

impl MemoryRelationship {
    pub fn validate(&self) -> Result<(), DbError> {
        if self.source == self.target && self.kind == RelationshipKind::Supersedes {
            return Err(DbError::SupersessionLoop);
        }
        if self.confidence > 10_000 {
            return Err(DbError::InvalidValue("relationship confidence"));
        }
        Ok(())
    }

    pub fn encode(&self, w: &mut BufWriter) -> Result<(), DbError> {
        w.write_u64(self.source.get())?;
        w.write_u64(self.target.get())?;
        w.write_u8(self.kind.as_u8())?;
        w.write_u16(self.confidence)?;
        w.write_u64(self.created_at_ns)?;
        self.provenance.encode(w)?;
        w.write_u8(if self.tombstoned { 1 } else { 0 })?;
        Ok(())
    }

    pub fn decode(r: &mut BufReader<'_>) -> Result<Self, DbError> {
        let source = MemoryId::from_raw(r.read_u64()?)
            .map_err(|_| DbError::InvalidValue("rel source"))?;
        let target = MemoryId::from_raw(r.read_u64()?)
            .map_err(|_| DbError::InvalidValue("rel target"))?;
        let kind = RelationshipKind::from_u8(r.read_u8()?)
            .ok_or(DbError::InvalidValue("rel kind"))?;
        let confidence = r.read_u16()?;
        let created_at_ns = r.read_u64()?;
        let provenance = RelationshipProvenance::decode(r)?;
        let tombstoned = r.read_u8()? != 0;
        let rel = Self {
            source,
            target,
            kind,
            confidence,
            created_at_ns,
            provenance,
            tombstoned,
        };
        rel.validate()?;
        Ok(rel)
    }
}

/// Relationship query filter.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct RelationshipQuery {
    pub of: MemoryId,
    pub direction: RelDirection,
    pub kind: Option<RelationshipKind>,
    pub max_depth: u32,
    pub max_edges: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum RelDirection {
    Outgoing,
    Incoming,
    Both,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiseowl_memory::TrustLevel;

    #[test]
    fn supersedes_self_rejected() {
        let id = MemoryId::from_raw_unchecked(1);
        let r = MemoryRelationship {
            source: id,
            target: id,
            kind: RelationshipKind::Supersedes,
            confidence: 100,
            created_at_ns: 1,
            provenance: RelationshipProvenance {
                producer_service: alloc::string::String::from("t"),
                created_at_ns: 1,
                trust: TrustLevel::Untrusted,
            },
            tombstoned: false,
        };
        assert_eq!(r.validate(), Err(DbError::SupersessionLoop));
    }
}
