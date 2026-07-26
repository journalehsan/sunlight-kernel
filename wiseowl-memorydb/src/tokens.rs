//! Caller-supplied token sets (no tokenizer implementation in Phase 2).
//!
//! Token IDs must be stable integers supplied by the caller. Do not use Rust's
//! default randomized `HashMap` hasher for persistent token identities.

use alloc::vec::Vec;

use crate::codec::{BufReader, BufWriter};
use crate::error::DbError;
use crate::quotas::DbQuotaConfig;

/// Reference describing which tokenizer produced the token set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct TokenSetRef {
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub token_count: u32,
}

impl TokenSetRef {
    pub fn encode(&self, w: &mut BufWriter) -> Result<(), DbError> {
        w.write_u32(self.tokenizer_id)?;
        w.write_u32(self.tokenizer_version)?;
        w.write_u32(self.token_count)?;
        Ok(())
    }

    pub fn decode(r: &mut BufReader<'_>) -> Result<Self, DbError> {
        Ok(Self {
            tokenizer_id: r.read_u32()?,
            tokenizer_version: r.read_u32()?,
            token_count: r.read_u32()?,
        })
    }
}

/// Bounded optional position list for a token within a record.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct BoundedPositions {
    pub positions: Vec<u32>,
}

impl BoundedPositions {
    pub fn validate(&self, max: u32) -> Result<(), DbError> {
        if self.positions.len() as u32 > max {
            return Err(DbError::QuotaExceeded("token positions"));
        }
        Ok(())
    }
}

/// One indexed token entry supplied by the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct IndexedToken {
    pub token_id: u64,
    pub frequency: u16,
    pub positions: Option<BoundedPositions>,
}

impl IndexedToken {
    pub fn validate(&self, quotas: &DbQuotaConfig) -> Result<(), DbError> {
        if self.frequency == 0 {
            return Err(DbError::InvalidValue("token frequency zero"));
        }
        if let Some(p) = &self.positions {
            p.validate(quotas.max_positions_per_token)?;
        }
        Ok(())
    }

    pub fn encode(&self, w: &mut BufWriter) -> Result<(), DbError> {
        w.write_u64(self.token_id)?;
        w.write_u16(self.frequency)?;
        match &self.positions {
            None => w.write_u8(0)?,
            Some(p) => {
                w.write_u8(1)?;
                w.write_u16(p.positions.len() as u16)?;
                for pos in &p.positions {
                    w.write_u32(*pos)?;
                }
            }
        }
        Ok(())
    }

    pub fn decode(r: &mut BufReader<'_>, quotas: &DbQuotaConfig) -> Result<Self, DbError> {
        let token_id = r.read_u64()?;
        let frequency = r.read_u16()?;
        let positions = match r.read_u8()? {
            0 => None,
            1 => {
                let n = r.read_u16()? as usize;
                if n as u32 > quotas.max_positions_per_token {
                    return Err(DbError::QuotaExceeded("token positions"));
                }
                let mut positions = Vec::with_capacity(n);
                for _ in 0..n {
                    positions.push(r.read_u32()?);
                }
                Some(BoundedPositions { positions })
            }
            _ => return Err(DbError::InvalidValue("token positions tag")),
        };
        let t = Self {
            token_id,
            frequency,
            positions,
        };
        t.validate(quotas)?;
        Ok(t)
    }
}

/// Normalize token list: sort by token_id, merge duplicates (sum frequency), reject on overflow.
pub fn normalize_tokens(
    mut tokens: Vec<IndexedToken>,
    quotas: &DbQuotaConfig,
) -> Result<Vec<IndexedToken>, DbError> {
    if tokens.len() as u32 > quotas.max_tokens_per_record {
        return Err(DbError::QuotaExceeded("tokens per record"));
    }
    tokens.sort_by_key(|t| t.token_id);
    let mut out: Vec<IndexedToken> = Vec::new();
    for t in tokens {
        t.validate(quotas)?;
        if let Some(last) = out.last_mut() {
            if last.token_id == t.token_id {
                last.frequency = last
                    .frequency
                    .checked_add(t.frequency)
                    .ok_or(DbError::InvalidValue("token frequency overflow"))?;
                if let Some(pos) = t.positions {
                    let dest = last.positions.get_or_insert_with(BoundedPositions::default);
                    dest.positions.extend(pos.positions);
                    dest.positions.sort_unstable();
                    dest.positions.dedup();
                    dest.validate(quotas.max_positions_per_token)?;
                }
                continue;
            }
        }
        out.push(t);
    }
    Ok(out)
}

/// Token match mode for queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum TokenMatchMode {
    Any,
    All,
    MinimumCount(u16),
}

/// Token query parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct TokenQuery {
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub token_ids: Vec<u64>,
    pub mode: TokenMatchMode,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_merges_duplicates() {
        let q = DbQuotaConfig::default();
        let t = normalize_tokens(
            vec![
                IndexedToken {
                    token_id: 5,
                    frequency: 1,
                    positions: None,
                },
                IndexedToken {
                    token_id: 5,
                    frequency: 2,
                    positions: None,
                },
                IndexedToken {
                    token_id: 1,
                    frequency: 1,
                    positions: None,
                },
            ],
            &q,
        )
        .unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].token_id, 1);
        assert_eq!(t[1].token_id, 5);
        assert_eq!(t[1].frequency, 3);
    }
}
