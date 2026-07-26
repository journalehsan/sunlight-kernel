//! Long-term provenance (extends Phase 0 fields; does not flatten to a string).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::{MemoryId, SourceId, SourceKind, TrustLevel};

use crate::codec::{BufReader, BufWriter};
use crate::error::DbError;
use crate::quotas::DbQuotaConfig;

/// How a long-term record was derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum DerivationKind {
    DirectImport = 1,
    ShortTermPromotion = 2,
    UserConfirmed = 3,
    ToolVerified = 4,
    Merged = 5,
    Supersedes = 6,
}

impl DerivationKind {
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::DirectImport),
            2 => Some(Self::ShortTermPromotion),
            3 => Some(Self::UserConfirmed),
            4 => Some(Self::ToolVerified),
            5 => Some(Self::Merged),
            6 => Some(Self::Supersedes),
            _ => None,
        }
    }

    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Durable provenance attached to every long-term record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct LongTermProvenance {
    pub source_kind: SourceKind,
    pub source_id: Option<SourceId>,
    pub producer_service: String,
    /// Original short-term / prior memory IDs (bounded).
    pub original_memory_ids: Vec<MemoryId>,
    /// Parent long-term record IDs (bounded).
    pub parent_lt_ids: Vec<MemoryId>,
    pub insertion_time_ns: u64,
    pub trust: TrustLevel,
    pub source_content_hash: Option<u64>,
    pub external_ref: Option<String>,
    pub derivation: DerivationKind,
}

impl LongTermProvenance {
    pub fn validate(&self, quotas: &DbQuotaConfig) -> Result<(), DbError> {
        if self.producer_service.len() > 64 {
            return Err(DbError::InvalidValue("producer too long"));
        }
        if self.original_memory_ids.len() as u32 > quotas.max_provenance_parents {
            return Err(DbError::QuotaExceeded("original memory ids"));
        }
        if self.parent_lt_ids.len() as u32 > quotas.max_provenance_parents {
            return Err(DbError::QuotaExceeded("parent lt ids"));
        }
        if let Some(ref er) = self.external_ref {
            if er.len() > 128 {
                return Err(DbError::InvalidValue("external_ref too long"));
            }
        }
        Ok(())
    }

    pub fn encode(&self, w: &mut BufWriter) -> Result<(), DbError> {
        w.write_u8(self.source_kind.as_u8())?;
        match self.source_id {
            None => w.write_u8(0)?,
            Some(id) => {
                w.write_u8(1)?;
                w.write_u64(id.get())?;
            }
        }
        w.write_bytes_len_u16(self.producer_service.as_bytes())?;
        w.write_u16(self.original_memory_ids.len() as u16)?;
        for id in &self.original_memory_ids {
            w.write_u64(id.get())?;
        }
        w.write_u16(self.parent_lt_ids.len() as u16)?;
        for id in &self.parent_lt_ids {
            w.write_u64(id.get())?;
        }
        w.write_u64(self.insertion_time_ns)?;
        w.write_u8(self.trust.as_u8())?;
        BufReader::write_opt_u64(w, self.source_content_hash)?;
        match &self.external_ref {
            None => w.write_u8(0)?,
            Some(s) => {
                w.write_u8(1)?;
                w.write_bytes_len_u16(s.as_bytes())?;
            }
        }
        w.write_u8(self.derivation.as_u8())?;
        Ok(())
    }

    pub fn decode(r: &mut BufReader<'_>, quotas: &DbQuotaConfig) -> Result<Self, DbError> {
        let source_kind =
            SourceKind::from_u8(r.read_u8()?).ok_or(DbError::InvalidValue("source kind"))?;
        let source_id = match r.read_u8()? {
            0 => None,
            1 => Some(
                SourceId::from_raw(r.read_u64()?)
                    .map_err(|_| DbError::InvalidValue("source id"))?,
            ),
            _ => return Err(DbError::InvalidValue("source_id tag")),
        };
        let producer_service = {
            let b = r.read_bytes_len_u16()?;
            core::str::from_utf8(b)
                .map_err(|_| DbError::InvalidValue("producer utf8"))?
                .to_string()
        };
        let n_orig = r.read_u16()? as usize;
        if n_orig as u32 > quotas.max_provenance_parents {
            return Err(DbError::QuotaExceeded("original memory ids"));
        }
        let mut original_memory_ids = Vec::with_capacity(n_orig);
        for _ in 0..n_orig {
            original_memory_ids.push(
                MemoryId::from_raw(r.read_u64()?)
                    .map_err(|_| DbError::InvalidValue("original memory id"))?,
            );
        }
        let n_par = r.read_u16()? as usize;
        if n_par as u32 > quotas.max_provenance_parents {
            return Err(DbError::QuotaExceeded("parent lt ids"));
        }
        let mut parent_lt_ids = Vec::with_capacity(n_par);
        for _ in 0..n_par {
            parent_lt_ids.push(
                MemoryId::from_raw(r.read_u64()?)
                    .map_err(|_| DbError::InvalidValue("parent lt id"))?,
            );
        }
        let insertion_time_ns = r.read_u64()?;
        let trust =
            TrustLevel::from_u8(r.read_u8()?).ok_or(DbError::InvalidValue("prov trust"))?;
        let source_content_hash = r.read_opt_u64()?;
        let external_ref = match r.read_u8()? {
            0 => None,
            1 => {
                let b = r.read_bytes_len_u16()?;
                Some(
                    core::str::from_utf8(b)
                        .map_err(|_| DbError::InvalidValue("external_ref utf8"))?
                        .to_string(),
                )
            }
            _ => return Err(DbError::InvalidValue("external_ref tag")),
        };
        let derivation = DerivationKind::from_u8(r.read_u8()?)
            .ok_or(DbError::InvalidValue("derivation"))?;
        let p = Self {
            source_kind,
            source_id,
            producer_service,
            original_memory_ids,
            parent_lt_ids,
            insertion_time_ns,
            trust,
            source_content_hash,
            external_ref,
            derivation,
        };
        p.validate(quotas)?;
        Ok(p)
    }
}

/// Provenance for a relationship edge (lightweight).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct RelationshipProvenance {
    pub producer_service: String,
    pub created_at_ns: u64,
    pub trust: TrustLevel,
}

impl RelationshipProvenance {
    pub fn encode(&self, w: &mut BufWriter) -> Result<(), DbError> {
        w.write_bytes_len_u16(self.producer_service.as_bytes())?;
        w.write_u64(self.created_at_ns)?;
        w.write_u8(self.trust.as_u8())?;
        Ok(())
    }

    pub fn decode(r: &mut BufReader<'_>) -> Result<Self, DbError> {
        let producer_service = {
            let b = r.read_bytes_len_u16()?;
            if b.len() > 64 {
                return Err(DbError::InvalidValue("rel producer too long"));
            }
            core::str::from_utf8(b)
                .map_err(|_| DbError::InvalidValue("rel producer utf8"))?
                .to_string()
        };
        let created_at_ns = r.read_u64()?;
        let trust =
            TrustLevel::from_u8(r.read_u8()?).ok_or(DbError::InvalidValue("rel trust"))?;
        Ok(Self {
            producer_service,
            created_at_ns,
            trust,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_roundtrip() {
        let q = DbQuotaConfig::default();
        let p = LongTermProvenance {
            source_kind: SourceKind::LocalTool,
            source_id: Some(SourceId::from_raw_unchecked(9)),
            producer_service: String::from("importer"),
            original_memory_ids: vec![MemoryId::from_raw_unchecked(1)],
            parent_lt_ids: vec![],
            insertion_time_ns: 55,
            trust: TrustLevel::Untrusted,
            source_content_hash: Some(0xABC),
            external_ref: Some(String::from("doc:1")),
            derivation: DerivationKind::ShortTermPromotion,
        };
        let mut w = BufWriter::with_capacity(1024);
        p.encode(&mut w).unwrap();
        let mut r = BufReader::new(w.as_slice());
        let d = LongTermProvenance::decode(&mut r, &q).unwrap();
        assert_eq!(d, p);
    }

    #[test]
    fn parent_bound() {
        let q = DbQuotaConfig {
            max_provenance_parents: 2,
            ..DbQuotaConfig::default()
        };
        let p = LongTermProvenance {
            source_kind: SourceKind::UserInput,
            source_id: None,
            producer_service: String::from("t"),
            original_memory_ids: vec![
                MemoryId::from_raw_unchecked(1),
                MemoryId::from_raw_unchecked(2),
                MemoryId::from_raw_unchecked(3),
            ],
            parent_lt_ids: vec![],
            insertion_time_ns: 1,
            trust: TrustLevel::Untrusted,
            source_content_hash: None,
            external_ref: None,
            derivation: DerivationKind::DirectImport,
        };
        assert!(p.validate(&q).is_err());
    }
}
