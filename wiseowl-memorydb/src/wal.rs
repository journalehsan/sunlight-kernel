//! Versioned write-ahead log for atomic transactions.
//!
//! A corrupt WAL tail must not invalidate earlier committed transactions.
//! Recovery stops at the first corrupt boundary without inventing data.
//!
//! ## Record layout (little-endian)
//!
//! ```text
//! magic            u32   = WAL_MAGIC
//! format_version   u16
//! record_type      u8
//! flags            u8
//! transaction_id   u64
//! sequence         u64
//! payload_len      u32
//! checksum         u32   CRC32 over header fields + payload (checksum field zeroed)
//! payload          [payload_len]
//! ```

use alloc::vec::Vec;

use wiseowl_memory::compression::crc32_ieee;

use crate::codec::{BufReader, BufWriter};
use crate::error::DbError;

/// WAL file magic: "OWLW" (Owl WAL).
pub const WAL_MAGIC: u32 = 0x574C_574F;
/// WAL format version.
pub const WAL_FORMAT_VERSION: u16 = 1;
/// Fixed header size before payload.
pub const WAL_HEADER_LEN: usize = 32;

/// WAL record types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WalRecordType {
    Begin = 1,
    InsertRecord = 2,
    InsertRelationship = 3,
    TombstoneRecord = 4,
    SourceDelete = 5,
    Commit = 6,
    Abort = 7,
    Checkpoint = 8,
}

impl WalRecordType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Begin),
            2 => Some(Self::InsertRecord),
            3 => Some(Self::InsertRelationship),
            4 => Some(Self::TombstoneRecord),
            5 => Some(Self::SourceDelete),
            6 => Some(Self::Commit),
            7 => Some(Self::Abort),
            8 => Some(Self::Checkpoint),
            _ => None,
        }
    }
}

/// One WAL record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalRecord {
    pub record_type: WalRecordType,
    pub transaction_id: u64,
    pub sequence: u64,
    pub payload: Vec<u8>,
}

impl WalRecord {
    pub fn encode(&self, max_payload: u32) -> Result<Vec<u8>, DbError> {
        if self.payload.len() as u32 > max_payload {
            return Err(DbError::PayloadTooLarge {
                size: self.payload.len() as u32,
                max: max_payload,
            });
        }
        let mut w = BufWriter::with_capacity(WAL_HEADER_LEN + self.payload.len());
        w.write_u32(WAL_MAGIC)?;
        w.write_u16(WAL_FORMAT_VERSION)?;
        w.write_u8(self.record_type as u8)?;
        w.write_u8(0)?; // flags
        w.write_u64(self.transaction_id)?;
        w.write_u64(self.sequence)?;
        w.write_u32(self.payload.len() as u32)?;
        w.write_u32(0)?; // checksum placeholder
        w.write_bytes(&self.payload)?;
        let mut bytes = w.into_vec();
        // Zero checksum field already; CRC over entire frame with checksum=0.
        let crc = crc32_ieee(&bytes);
        bytes[28..32].copy_from_slice(&crc.to_le_bytes());
        Ok(bytes)
    }

    /// Decode one record from the front of `data`. Returns (record, bytes_consumed).
    pub fn decode_one(data: &[u8], max_payload: u32) -> Result<(Self, usize), DbError> {
        if data.len() < WAL_HEADER_LEN {
            return Err(DbError::WalIncomplete);
        }
        let mut r = BufReader::new(data);
        let magic = r.read_u32()?;
        if magic != WAL_MAGIC {
            return Err(DbError::Corrupt {
                reason: "wal magic",
            });
        }
        let version = r.read_u16()?;
        if version != WAL_FORMAT_VERSION {
            return Err(DbError::Corrupt {
                reason: "wal version",
            });
        }
        let rt = r.read_u8()?;
        let _flags = r.read_u8()?;
        let transaction_id = r.read_u64()?;
        let sequence = r.read_u64()?;
        let payload_len = r.read_u32()?;
        let checksum = r.read_u32()?;
        if payload_len > max_payload {
            return Err(DbError::PayloadTooLarge {
                size: payload_len,
                max: max_payload,
            });
        }
        let total = WAL_HEADER_LEN
            .checked_add(payload_len as usize)
            .ok_or(DbError::Corrupt {
                reason: "wal size overflow",
            })?;
        if data.len() < total {
            return Err(DbError::WalIncomplete);
        }
        let payload = data[WAL_HEADER_LEN..total].to_vec();

        // Verify CRC: reconstruct with checksum zeroed.
        let mut check_buf = data[..total].to_vec();
        check_buf[28..32].copy_from_slice(&0u32.to_le_bytes());
        let computed = crc32_ieee(&check_buf);
        if computed != checksum {
            return Err(DbError::Corrupt {
                reason: "wal checksum",
            });
        }

        let record_type = WalRecordType::from_u8(rt).ok_or(DbError::Corrupt {
            reason: "unknown wal record type",
        })?;
        Ok((
            Self {
                record_type,
                transaction_id,
                sequence,
                payload,
            },
            total,
        ))
    }
}

/// Scan a WAL buffer, returning committed transaction payloads and the last good offset.
/// Incomplete / corrupt tail is isolated; earlier commits remain.
#[derive(Debug, Default)]
pub struct WalScanResult {
    /// All successfully decoded records up to the first error (inclusive of complete ones).
    pub records: Vec<WalRecord>,
    /// Byte offset of first unreadable/corrupt region (or end of data).
    pub good_end: usize,
    /// True if a corrupt or truncated record was found after good_end.
    pub tail_corrupt: bool,
}

pub fn scan_wal(data: &[u8], max_payload: u32) -> WalScanResult {
    let mut records = Vec::new();
    let mut offset = 0usize;
    while offset < data.len() {
        match WalRecord::decode_one(&data[offset..], max_payload) {
            Ok((rec, n)) => {
                records.push(rec);
                offset += n;
            }
            Err(DbError::WalIncomplete) | Err(DbError::Corrupt { .. })
            | Err(DbError::PayloadTooLarge { .. }) => {
                return WalScanResult {
                    records,
                    good_end: offset,
                    tail_corrupt: offset < data.len(),
                };
            }
            Err(_) => {
                return WalScanResult {
                    records,
                    good_end: offset,
                    tail_corrupt: true,
                };
            }
        }
    }
    WalScanResult {
        records,
        good_end: offset,
        tail_corrupt: false,
    }
}

/// Identify fully committed transaction IDs from a WAL record list.
/// Incomplete (begin without commit) transactions are ignored on recovery.
pub fn committed_tx_ids(records: &[WalRecord]) -> alloc::collections::BTreeSet<u64> {
    use alloc::collections::{BTreeMap, BTreeSet};
    let mut state: BTreeMap<u64, bool> = BTreeMap::new(); // true = committed
    for r in records {
        match r.record_type {
            WalRecordType::Begin => {
                state.entry(r.transaction_id).or_insert(false);
            }
            WalRecordType::Commit => {
                state.insert(r.transaction_id, true);
            }
            WalRecordType::Abort => {
                state.insert(r.transaction_id, false);
            }
            _ => {}
        }
    }
    state
        .into_iter()
        .filter_map(|(id, ok)| if ok { Some(id) } else { None })
        .collect::<BTreeSet<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode() {
        let r = WalRecord {
            record_type: WalRecordType::Begin,
            transaction_id: 7,
            sequence: 1,
            payload: b"meta".to_vec(),
        };
        let bytes = r.encode(1024).unwrap();
        let (d, n) = WalRecord::decode_one(&bytes, 1024).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(d, r);
    }

    #[test]
    fn corrupt_checksum_isolated() {
        let r1 = WalRecord {
            record_type: WalRecordType::Begin,
            transaction_id: 1,
            sequence: 1,
            payload: vec![],
        };
        let r2 = WalRecord {
            record_type: WalRecordType::Commit,
            transaction_id: 1,
            sequence: 2,
            payload: vec![],
        };
        let mut buf = r1.encode(1024).unwrap();
        buf.extend(r2.encode(1024).unwrap());
        // Corrupt last byte of second record.
        let last = buf.len() - 1;
        buf[last] ^= 0xFF;
        let scan = scan_wal(&buf, 1024);
        assert_eq!(scan.records.len(), 1);
        assert!(scan.tail_corrupt);
        let committed = committed_tx_ids(&scan.records);
        assert!(committed.is_empty()); // begin without commit
    }

    #[test]
    fn commit_visible() {
        let records = vec![
            WalRecord {
                record_type: WalRecordType::Begin,
                transaction_id: 3,
                sequence: 1,
                payload: vec![],
            },
            WalRecord {
                record_type: WalRecordType::InsertRecord,
                transaction_id: 3,
                sequence: 2,
                payload: b"rec".to_vec(),
            },
            WalRecord {
                record_type: WalRecordType::Commit,
                transaction_id: 3,
                sequence: 3,
                payload: vec![],
            },
        ];
        assert!(committed_tx_ids(&records).contains(&3));
    }

    #[test]
    fn truncated_header() {
        let r = WalRecord {
            record_type: WalRecordType::Begin,
            transaction_id: 1,
            sequence: 1,
            payload: vec![1, 2, 3],
        };
        let mut bytes = r.encode(1024).unwrap();
        bytes.truncate(10);
        let scan = scan_wal(&bytes, 1024);
        assert!(scan.records.is_empty());
        assert!(scan.tail_corrupt);
    }
}
