//! Narrow append-only persistence for Wise Owl action receipt blobs.
//!
//! The memory database owns the durable store. Callers provide bounded opaque
//! fragments and a sealed receipt image; this module never interprets action
//! semantics and cannot authorize or execute actions.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use wiseowl_memory::compression::crc32_ieee;

use crate::database::DurableStore;
use crate::error::DbError;

pub const ACTION_RECEIPT_NAMESPACE: &str = "ACTION_RECEIPTS";
pub const ACTION_RECEIPT_STORAGE_VERSION: u16 = 1;
pub const MAX_RECEIPT_FRAGMENT_BYTES: usize = 1024;
pub const MAX_ACTIVE_RECEIPT_BYTES: usize = 8192;
pub const MAX_SEALED_RECEIPT_BYTES: usize = 8192;

const FRAGMENT_MAGIC: u32 = 0x4652_4357; // "WCRF"
const SEALED_MAGIC: u32 = 0x5352_4357; // "WCRS"
const FRAME_HEADER_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ReceiptBlobKey {
    pub owner_domain: u64,
    pub session_id: u64,
    pub receipt_id: [u8; 16],
}

impl ReceiptBlobKey {
    pub const fn new(owner_domain: u64, session_id: u64, receipt_id: [u8; 16]) -> Self {
        Self {
            owner_domain,
            session_id,
            receipt_id,
        }
    }

    fn stem(self) -> String {
        format!(
            "{:016x}-{:016x}-{}",
            self.owner_domain,
            self.session_id,
            hex_receipt_id(self.receipt_id)
        )
    }
}

/// Capability-separated receipt namespace hosted by `wiseowl-memorydb`.
pub struct ActionReceiptBlobStore<S: DurableStore> {
    store: S,
}

impl<S: DurableStore> ActionReceiptBlobStore<S> {
    pub fn open(mut store: S) -> Result<Self, DbError> {
        store.ensure_layout()?;
        store.write_file_atomic(
            &format!("{ACTION_RECEIPT_NAMESPACE}/.keep"),
            b"action-receipts-v1",
        )?;
        Ok(Self { store })
    }

    pub fn append_fragment(&mut self, key: ReceiptBlobKey, fragment: &[u8]) -> Result<(), DbError> {
        if fragment.is_empty() {
            return Err(DbError::InvalidRequest("empty receipt fragment"));
        }
        if fragment.len() > MAX_RECEIPT_FRAGMENT_BYTES {
            return Err(DbError::PayloadTooLarge {
                size: fragment.len() as u32,
                max: MAX_RECEIPT_FRAGMENT_BYTES as u32,
            });
        }
        if self.read_sealed(key)?.is_some() {
            return Err(DbError::Conflict("receipt already sealed"));
        }
        let path = active_path(key);
        let current = self
            .store
            .read_file(path.as_str())?
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        let frame = encode_frame(FRAGMENT_MAGIC, fragment, MAX_RECEIPT_FRAGMENT_BYTES)?;
        let next = current
            .checked_add(frame.len())
            .ok_or(DbError::QuotaExceeded("active receipt bytes"))?;
        if next > MAX_ACTIVE_RECEIPT_BYTES {
            return Err(DbError::QuotaExceeded("active receipt bytes"));
        }
        self.store.append_file(path.as_str(), frame.as_slice())
    }

    /// Atomically publishes the sealed image, then removes the active fragment
    /// stream. A crash can leave fragments beside a valid seal, but cannot
    /// expose a partial sealed receipt.
    pub fn seal(&mut self, key: ReceiptBlobKey, receipt: &[u8]) -> Result<(), DbError> {
        if receipt.is_empty() {
            return Err(DbError::InvalidRequest("empty sealed receipt"));
        }
        if receipt.len() > MAX_SEALED_RECEIPT_BYTES {
            return Err(DbError::PayloadTooLarge {
                size: receipt.len() as u32,
                max: MAX_SEALED_RECEIPT_BYTES as u32,
            });
        }
        if self.read_sealed(key)?.is_some() {
            return Err(DbError::Conflict("receipt already sealed"));
        }
        let frame = encode_frame(SEALED_MAGIC, receipt, MAX_SEALED_RECEIPT_BYTES)?;
        self.store
            .write_file_atomic(sealed_path(key).as_str(), frame.as_slice())?;
        self.store.remove_file(active_path(key).as_str())
    }

    pub fn read_sealed(&self, key: ReceiptBlobKey) -> Result<Option<Vec<u8>>, DbError> {
        let Some(frame) = self.store.read_file(sealed_path(key).as_str())? else {
            return Ok(None);
        };
        decode_frame(SEALED_MAGIC, frame.as_slice(), MAX_SEALED_RECEIPT_BYTES).map(Some)
    }

    pub fn read_active_fragments(&self, key: ReceiptBlobKey) -> Result<Vec<Vec<u8>>, DbError> {
        let Some(bytes) = self.store.read_file(active_path(key).as_str())? else {
            return Ok(Vec::new());
        };
        decode_fragment_stream(bytes.as_slice())
    }

    pub fn list_sealed(&self, owner_domain: u64, session_id: u64) -> Result<Vec<String>, DbError> {
        let prefix = format!("{:016x}-{:016x}-", owner_domain, session_id);
        self.store.list_prefix(
            &format!("{ACTION_RECEIPT_NAMESPACE}/sealed"),
            prefix.as_str(),
        )
    }

    pub fn evict_sealed(&mut self, key: ReceiptBlobKey) -> Result<(), DbError> {
        self.store.remove_file(sealed_path(key).as_str())
    }

    pub fn into_store(self) -> S {
        self.store
    }
}

fn active_path(key: ReceiptBlobKey) -> String {
    format!("{ACTION_RECEIPT_NAMESPACE}/active/{}.fragments", key.stem())
}

fn sealed_path(key: ReceiptBlobKey) -> String {
    format!("{ACTION_RECEIPT_NAMESPACE}/sealed/{}.receipt", key.stem())
}

fn hex_receipt_id(receipt_id: [u8; 16]) -> String {
    let mut value = String::with_capacity(32);
    for byte in receipt_id {
        value.push(hex_digit(byte >> 4));
        value.push(hex_digit(byte & 0x0f));
    }
    value
}

const fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'a' + value - 10) as char,
    }
}

fn encode_frame(magic: u32, payload: &[u8], max: usize) -> Result<Vec<u8>, DbError> {
    if payload.len() > max {
        return Err(DbError::PayloadTooLarge {
            size: payload.len() as u32,
            max: max as u32,
        });
    }
    let checksum = crc32_ieee(payload);
    let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    frame.extend_from_slice(&magic.to_le_bytes());
    frame.extend_from_slice(&ACTION_RECEIPT_STORAGE_VERSION.to_le_bytes());
    frame.extend_from_slice(&0u16.to_le_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&checksum.to_le_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

fn decode_frame(magic: u32, frame: &[u8], max: usize) -> Result<Vec<u8>, DbError> {
    if frame.len() < FRAME_HEADER_LEN {
        return Err(DbError::Corrupt {
            reason: "receipt frame truncated",
        });
    }
    if u32::from_le_bytes(frame[0..4].try_into().unwrap()) != magic {
        return Err(DbError::Corrupt {
            reason: "receipt frame magic",
        });
    }
    if u16::from_le_bytes(frame[4..6].try_into().unwrap()) != ACTION_RECEIPT_STORAGE_VERSION {
        return Err(DbError::Corrupt {
            reason: "receipt frame version",
        });
    }
    if frame[6] != 0 || frame[7] != 0 {
        return Err(DbError::Corrupt {
            reason: "receipt frame flags",
        });
    }
    let length = u32::from_le_bytes(frame[8..12].try_into().unwrap()) as usize;
    if length > max || frame.len() != FRAME_HEADER_LEN + length {
        return Err(DbError::Corrupt {
            reason: "receipt frame length",
        });
    }
    let expected = u32::from_le_bytes(frame[12..16].try_into().unwrap());
    let payload = &frame[FRAME_HEADER_LEN..];
    if crc32_ieee(payload) != expected {
        return Err(DbError::Corrupt {
            reason: "receipt frame checksum",
        });
    }
    Ok(payload.to_vec())
}

fn decode_fragment_stream(bytes: &[u8]) -> Result<Vec<Vec<u8>>, DbError> {
    let mut fragments = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        if bytes.len() - offset < FRAME_HEADER_LEN {
            return Err(DbError::WalIncomplete);
        }
        let length =
            u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap()) as usize;
        let end = offset
            .checked_add(FRAME_HEADER_LEN)
            .and_then(|value| value.checked_add(length))
            .ok_or(DbError::Corrupt {
                reason: "receipt fragment overflow",
            })?;
        if end > bytes.len() {
            return Err(DbError::WalIncomplete);
        }
        fragments.push(decode_frame(
            FRAGMENT_MAGIC,
            &bytes[offset..end],
            MAX_RECEIPT_FRAGMENT_BYTES,
        )?);
        offset = end;
    }
    Ok(fragments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::MemoryStore;

    fn key(value: u8) -> ReceiptBlobKey {
        ReceiptBlobKey::new(7, 11, [value; 16])
    }

    #[test]
    fn fragments_append_and_seal_atomically() {
        let mut receipts = ActionReceiptBlobStore::open(MemoryStore::default()).unwrap();
        receipts.append_fragment(key(1), b"opened").unwrap();
        receipts.append_fragment(key(1), b"policy").unwrap();
        assert_eq!(
            receipts.read_active_fragments(key(1)).unwrap(),
            vec![b"opened".to_vec(), b"policy".to_vec()]
        );
        receipts.seal(key(1), b"sealed receipt").unwrap();
        assert_eq!(
            receipts.read_sealed(key(1)).unwrap(),
            Some(b"sealed receipt".to_vec())
        );
        assert!(receipts.read_active_fragments(key(1)).unwrap().is_empty());
        assert!(receipts.append_fragment(key(1), b"late").is_err());
    }

    #[test]
    fn sealed_listing_is_domain_bounded() {
        let mut receipts = ActionReceiptBlobStore::open(MemoryStore::default()).unwrap();
        receipts.seal(key(1), b"one").unwrap();
        receipts.seal(key(2), b"two").unwrap();
        receipts
            .seal(ReceiptBlobKey::new(8, 11, [3; 16]), b"other")
            .unwrap();
        let listed = receipts.list_sealed(7, 11).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed[0] < listed[1]);
    }

    #[test]
    fn corrupted_seal_fails_closed() {
        let mut receipts = ActionReceiptBlobStore::open(MemoryStore::default()).unwrap();
        receipts.seal(key(1), b"sealed").unwrap();
        let mut store = receipts.into_store();
        let path = sealed_path(key(1));
        let mut frame = store.read_file(path.as_str()).unwrap().unwrap();
        *frame.last_mut().unwrap() ^= 0xff;
        store
            .write_file_atomic(path.as_str(), frame.as_slice())
            .unwrap();
        let receipts = ActionReceiptBlobStore::open(store).unwrap();
        assert!(matches!(
            receipts.read_sealed(key(1)),
            Err(DbError::Corrupt { .. })
        ));
    }
}
