//! Bounded typed attribute map (filtering metadata, not a second payload channel).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::codec::{BufReader, BufWriter};
use crate::error::DbError;
use crate::quotas::DbQuotaConfig;

/// Allowed attribute value types. No recursive / nested documents.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub enum AttributeValue {
    Bool(bool),
    Integer(i64),
    Unsigned(u64),
    Text(String),
    Id(u64),
    Timestamp(u64),
}

impl AttributeValue {
    fn tag(&self) -> u8 {
        match self {
            Self::Bool(_) => 1,
            Self::Integer(_) => 2,
            Self::Unsigned(_) => 3,
            Self::Text(_) => 4,
            Self::Id(_) => 5,
            Self::Timestamp(_) => 6,
        }
    }

    pub fn encode(&self, w: &mut BufWriter) -> Result<(), DbError> {
        w.write_u8(self.tag())?;
        match self {
            Self::Bool(b) => w.write_u8(if *b { 1 } else { 0 }),
            Self::Integer(i) => w.write_i64(*i),
            Self::Unsigned(u) => w.write_u64(*u),
            Self::Text(s) => w.write_bytes_len_u16(s.as_bytes()),
            Self::Id(id) => w.write_u64(*id),
            Self::Timestamp(t) => w.write_u64(*t),
        }
    }

    pub fn decode(r: &mut BufReader<'_>, max_text: u32) -> Result<Self, DbError> {
        match r.read_u8()? {
            1 => Ok(Self::Bool(r.read_u8()? != 0)),
            2 => Ok(Self::Integer(r.read_i64()?)),
            3 => Ok(Self::Unsigned(r.read_u64()?)),
            4 => {
                let b = r.read_bytes_len_u16()?;
                if b.len() as u32 > max_text {
                    return Err(DbError::QuotaExceeded("attribute text"));
                }
                Ok(Self::Text(
                    core::str::from_utf8(b)
                        .map_err(|_| DbError::InvalidValue("attr text utf8"))?
                        .to_string(),
                ))
            }
            5 => Ok(Self::Id(r.read_u64()?)),
            6 => Ok(Self::Timestamp(r.read_u64()?)),
            _ => Err(DbError::InvalidValue("attribute value tag")),
        }
    }
}

/// Single attribute entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct Attribute {
    pub key: String,
    pub value: AttributeValue,
}

/// Deterministically ordered attribute set (sorted by key on encode/normalize).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct AttributeSet {
    pub entries: Vec<Attribute>,
}

impl AttributeSet {
    pub fn normalize(&mut self) {
        self.entries.sort_by(|a, b| a.key.cmp(&b.key));
        self.entries.dedup_by(|a, b| a.key == b.key);
    }

    pub fn validate(&self, quotas: &DbQuotaConfig) -> Result<(), DbError> {
        if self.entries.len() as u32 > quotas.max_attributes_per_record {
            return Err(DbError::QuotaExceeded("attributes"));
        }
        for e in &self.entries {
            if e.key.is_empty() || e.key.len() as u32 > quotas.max_attribute_key_bytes {
                return Err(DbError::InvalidValue("attribute key"));
            }
            if let AttributeValue::Text(t) = &e.value {
                if t.len() as u32 > quotas.max_attribute_text_bytes {
                    return Err(DbError::QuotaExceeded("attribute text"));
                }
            }
        }
        Ok(())
    }

    pub fn encode(&self, w: &mut BufWriter) -> Result<(), DbError> {
        let mut sorted = self.clone();
        sorted.normalize();
        w.write_u16(sorted.entries.len() as u16)?;
        for e in &sorted.entries {
            w.write_bytes_len_u16(e.key.as_bytes())?;
            e.value.encode(w)?;
        }
        Ok(())
    }

    pub fn decode(r: &mut BufReader<'_>, quotas: &DbQuotaConfig) -> Result<Self, DbError> {
        let n = r.read_u16()? as usize;
        if n as u32 > quotas.max_attributes_per_record {
            return Err(DbError::QuotaExceeded("attributes"));
        }
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let key_b = r.read_bytes_len_u16()?;
            if key_b.len() as u32 > quotas.max_attribute_key_bytes {
                return Err(DbError::InvalidValue("attribute key"));
            }
            let key = core::str::from_utf8(key_b)
                .map_err(|_| DbError::InvalidValue("attr key utf8"))?
                .to_string();
            let value = AttributeValue::decode(r, quotas.max_attribute_text_bytes)?;
            entries.push(Attribute { key, value });
        }
        let mut set = Self { entries };
        set.normalize();
        set.validate(quotas)?;
        Ok(set)
    }

    pub fn get(&self, key: &str) -> Option<&AttributeValue> {
        self.entries.iter().find(|e| e.key == key).map(|e| &e.value)
    }
}

/// Filter for queries (equality only; bounded).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct BoundedAttributeFilters {
    pub filters: Vec<(String, AttributeValue)>,
}

impl BoundedAttributeFilters {
    pub fn matches(&self, attrs: &AttributeSet) -> bool {
        for (k, v) in &self.filters {
            match attrs.get(k) {
                Some(av) if av == v => {}
                _ => return false,
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorted_encode() {
        let q = DbQuotaConfig::default();
        let mut s = AttributeSet {
            entries: vec![
                Attribute {
                    key: String::from("z"),
                    value: AttributeValue::Bool(true),
                },
                Attribute {
                    key: String::from("a"),
                    value: AttributeValue::Integer(3),
                },
            ],
        };
        s.normalize();
        assert_eq!(s.entries[0].key, "a");
        let mut w = BufWriter::with_capacity(256);
        s.encode(&mut w).unwrap();
        let mut r = BufReader::new(w.as_slice());
        let d = AttributeSet::decode(&mut r, &q).unwrap();
        assert_eq!(d.entries.len(), 2);
    }
}
