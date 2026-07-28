use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::grounded::FactKind;

const FOUNDATION_MAGIC: &[u8; 8] = b"WOFM\x01\0\0\0";
pub const FOUNDATION_FORMAT_VERSION: u16 = 1;
pub const FOUNDATION_SCHEMA_VERSION: u16 = 1;
const FOUNDATION_HEADER_LEN: usize = 106;
const INTEGRITY_HASH_OFFSET: usize = 74;

include!(concat!(env!("OUT_DIR"), "/foundation_build.rs"));

static EMBEDDED_FOUNDATION_BLOB: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/wiseowl-foundation.bin"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum FoundationKey {
    AssistantName = 1,
    InternalCodename = 2,
    SunlightOsIdentity = 3,
    GeneralRole = 4,
    HighLevelCapabilities = 5,
    SafetyPrinciples = 6,
    CapabilitySecurityModel = 7,
    RuntimeInfoGuidance = 8,
    MemoryLayerModel = 9,
}

impl FoundationKey {
    pub const fn as_source_key(self) -> &'static str {
        match self {
            Self::AssistantName => "assistant_name",
            Self::InternalCodename => "internal_codename",
            Self::SunlightOsIdentity => "sunlightos_identity",
            Self::GeneralRole => "general_role",
            Self::HighLevelCapabilities => "high_level_capabilities",
            Self::SafetyPrinciples => "safety_principles",
            Self::CapabilitySecurityModel => "capability_security_model",
            Self::RuntimeInfoGuidance => "runtime_info_guidance",
            Self::MemoryLayerModel => "memory_layer_model",
        }
    }

    pub const fn from_tag(tag: u16) -> Option<Self> {
        match tag {
            1 => Some(Self::AssistantName),
            2 => Some(Self::InternalCodename),
            3 => Some(Self::SunlightOsIdentity),
            4 => Some(Self::GeneralRole),
            5 => Some(Self::HighLevelCapabilities),
            6 => Some(Self::SafetyPrinciples),
            7 => Some(Self::CapabilitySecurityModel),
            8 => Some(Self::RuntimeInfoGuidance),
            9 => Some(Self::MemoryLayerModel),
            _ => None,
        }
    }

    pub const fn fact_kind(self) -> FactKind {
        match self {
            Self::AssistantName => FactKind::AssistantName,
            Self::InternalCodename => FactKind::AssistantCodename,
            Self::SunlightOsIdentity => FactKind::SunlightIdentity,
            Self::GeneralRole => FactKind::FoundationRole,
            Self::HighLevelCapabilities => FactKind::FoundationCapabilities,
            Self::SafetyPrinciples => FactKind::FoundationSafety,
            Self::CapabilitySecurityModel => FactKind::FoundationSecurityModel,
            Self::RuntimeInfoGuidance => FactKind::FoundationRuntimeInfo,
            Self::MemoryLayerModel => FactKind::FoundationMemoryModel,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundationRecord {
    pub key: FoundationKey,
    pub value: String,
    pub token_offset: u32,
    pub token_count: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundationTokenEntry {
    pub token_id: u64,
    pub frequency: u16,
    pub positions_truncated: bool,
    pub positions: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundationMemory {
    pub schema_version: u16,
    pub tokenizer_id: u32,
    pub tokenizer_version: u32,
    pub tokenizer_fingerprint: [u8; 32],
    pub records: Vec<FoundationRecord>,
    pub tokens: Vec<FoundationTokenEntry>,
    pub integrity_hash: [u8; 32],
}

impl FoundationMemory {
    pub fn load_embedded() -> Result<Self, FoundationLoadError> {
        Self::decode_and_validate(EMBEDDED_FOUNDATION_BLOB)
    }

    pub fn decode_and_validate(bytes: &[u8]) -> Result<Self, FoundationLoadError> {
        if bytes.len() < FOUNDATION_HEADER_LEN {
            return Err(FoundationLoadError::Truncated);
        }
        if &bytes[0..8] != FOUNDATION_MAGIC {
            return Err(FoundationLoadError::BadMagic);
        }

        let format_version = read_u16(bytes, 8)?;
        if format_version != FOUNDATION_FORMAT_VERSION {
            return Err(FoundationLoadError::UnsupportedFormatVersion(format_version));
        }

        let schema_version = read_u16(bytes, 10)?;
        if schema_version != FOUNDATION_SCHEMA_VERSION {
            return Err(FoundationLoadError::UnsupportedSchemaVersion(schema_version));
        }

        let tokenizer_id = read_u32(bytes, 12)?;
        if tokenizer_id != FOUNDATION_EXPECTED_TOKENIZER_ID {
            return Err(FoundationLoadError::TokenizerIdMismatch(tokenizer_id));
        }

        let tokenizer_version = read_u32(bytes, 16)?;
        if tokenizer_version != FOUNDATION_EXPECTED_TOKENIZER_VERSION {
            return Err(FoundationLoadError::TokenizerVersionMismatch(tokenizer_version));
        }

        let tokenizer_fingerprint = read_array32(bytes, 20)?;
        if tokenizer_fingerprint != FOUNDATION_EXPECTED_TOKENIZER_FINGERPRINT {
            return Err(FoundationLoadError::TokenizerFingerprintMismatch);
        }

        let record_count = read_u16(bytes, 52)? as usize;
        let token_count = read_u32(bytes, 54)? as usize;
        let records_offset = read_u32(bytes, 58)? as usize;
        let records_len = read_u32(bytes, 62)? as usize;
        let tokens_offset = read_u32(bytes, 66)? as usize;
        let tokens_len = read_u32(bytes, 70)? as usize;
        let integrity_hash = read_array32(bytes, INTEGRITY_HASH_OFFSET)?;

        if records_offset != FOUNDATION_HEADER_LEN {
            return Err(FoundationLoadError::BadLayout);
        }
        if records_offset.checked_add(records_len).ok_or(FoundationLoadError::BadLayout)? != tokens_offset
        {
            return Err(FoundationLoadError::BadLayout);
        }
        let tokens_end = tokens_offset
            .checked_add(tokens_len)
            .ok_or(FoundationLoadError::BadLayout)?;
        if tokens_end != bytes.len() {
            return Err(FoundationLoadError::BadLayout);
        }

        let mut hashed = Vec::with_capacity(bytes.len().saturating_sub(32));
        hashed.extend_from_slice(&bytes[..INTEGRITY_HASH_OFFSET]);
        hashed.extend_from_slice(&bytes[INTEGRITY_HASH_OFFSET + 32..]);
        if wiseowl_index::digest::sha256(&hashed) != integrity_hash {
            return Err(FoundationLoadError::IntegrityHashMismatch);
        }

        let records = decode_records(
            &bytes[records_offset..records_offset + records_len],
            record_count,
            token_count,
        )?;
        let tokens = decode_tokens(&bytes[tokens_offset..tokens_end], token_count)?;

        Ok(Self {
            schema_version,
            tokenizer_id,
            tokenizer_version,
            tokenizer_fingerprint,
            records,
            tokens,
            integrity_hash,
        })
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    pub fn token_count(&self) -> usize {
        self.tokens.len()
    }

    pub fn get(&self, key: FoundationKey) -> Option<&str> {
        self.records
            .iter()
            .find(|record| record.key == key)
            .map(|record| record.value.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoundationLoadError {
    BadMagic,
    UnsupportedFormatVersion(u16),
    UnsupportedSchemaVersion(u16),
    TokenizerIdMismatch(u32),
    TokenizerVersionMismatch(u32),
    TokenizerFingerprintMismatch,
    IntegrityHashMismatch,
    BadLayout,
    InvalidKey(u16),
    InvalidUtf8,
    TokenSliceOutOfBounds,
    Truncated,
}

impl FoundationLoadError {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::BadMagic => "bad-magic",
            Self::UnsupportedFormatVersion(_) => "format-version",
            Self::UnsupportedSchemaVersion(_) => "schema-version",
            Self::TokenizerIdMismatch(_) => "tokenizer-id",
            Self::TokenizerVersionMismatch(_) => "tokenizer-version",
            Self::TokenizerFingerprintMismatch => "tokenizer-fingerprint",
            Self::IntegrityHashMismatch => "integrity-hash",
            Self::BadLayout => "layout",
            Self::InvalidKey(_) => "invalid-key",
            Self::InvalidUtf8 => "invalid-utf8",
            Self::TokenSliceOutOfBounds => "token-slice",
            Self::Truncated => "truncated",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FoundationLoadState {
    memory: Option<FoundationMemory>,
    error: Option<FoundationLoadError>,
}

impl FoundationLoadState {
    pub fn load_embedded() -> Self {
        match FoundationMemory::load_embedded() {
            Ok(memory) => Self {
                memory: Some(memory),
                error: None,
            },
            Err(error) => Self {
                memory: None,
                error: Some(error),
            },
        }
    }

    pub fn memory(&self) -> Option<&FoundationMemory> {
        self.memory.as_ref()
    }

    pub fn error(&self) -> Option<&FoundationLoadError> {
        self.error.as_ref()
    }

    pub fn is_ready(&self) -> bool {
        self.memory.is_some()
    }

    pub fn record_count(&self) -> usize {
        self.memory.as_ref().map(|memory| memory.record_count()).unwrap_or(0)
    }

    pub fn token_count(&self) -> usize {
        self.memory.as_ref().map(|memory| memory.token_count()).unwrap_or(0)
    }

    pub fn status_label(&self) -> &'static str {
        match self.error() {
            Some(error) => error.as_str(),
            None => "ready",
        }
    }
}

fn decode_records(
    bytes: &[u8],
    record_count: usize,
    total_tokens: usize,
) -> Result<Vec<FoundationRecord>, FoundationLoadError> {
    let mut cursor = 0usize;
    let mut records = Vec::with_capacity(record_count);
    for _ in 0..record_count {
        let key_tag = read_u16(bytes, cursor)?;
        cursor += 2;
        let value_len = read_u16(bytes, cursor)? as usize;
        cursor += 2;
        let token_offset = read_u32(bytes, cursor)?;
        cursor += 4;
        let token_count = read_u16(bytes, cursor)?;
        cursor += 2;
        let value_end = cursor
            .checked_add(value_len)
            .ok_or(FoundationLoadError::Truncated)?;
        if value_end > bytes.len() {
            return Err(FoundationLoadError::Truncated);
        }
        let value = core::str::from_utf8(&bytes[cursor..value_end])
            .map_err(|_| FoundationLoadError::InvalidUtf8)?
            .to_string();
        cursor = value_end;
        let key = FoundationKey::from_tag(key_tag).ok_or(FoundationLoadError::InvalidKey(key_tag))?;
        let offset = token_offset as usize;
        let count = token_count as usize;
        if offset > total_tokens || offset.saturating_add(count) > total_tokens {
            return Err(FoundationLoadError::TokenSliceOutOfBounds);
        }
        records.push(FoundationRecord {
            key,
            value,
            token_offset,
            token_count,
        });
    }
    if cursor != bytes.len() {
        return Err(FoundationLoadError::BadLayout);
    }
    Ok(records)
}

fn decode_tokens(
    bytes: &[u8],
    token_count: usize,
) -> Result<Vec<FoundationTokenEntry>, FoundationLoadError> {
    let mut cursor = 0usize;
    let mut tokens = Vec::with_capacity(token_count);
    for _ in 0..token_count {
        let token_id = read_u64(bytes, cursor)?;
        cursor += 8;
        let frequency = read_u16(bytes, cursor)?;
        cursor += 2;
        let flags = read_u16(bytes, cursor)?;
        cursor += 2;
        let position_count = read_u16(bytes, cursor)? as usize;
        cursor += 2;
        let mut positions = Vec::with_capacity(position_count);
        for _ in 0..position_count {
            positions.push(read_u32(bytes, cursor)?);
            cursor += 4;
        }
        tokens.push(FoundationTokenEntry {
            token_id,
            frequency,
            positions_truncated: flags & 1 != 0,
            positions,
        });
    }
    if cursor != bytes.len() {
        return Err(FoundationLoadError::BadLayout);
    }
    Ok(tokens)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, FoundationLoadError> {
    let end = offset.checked_add(2).ok_or(FoundationLoadError::Truncated)?;
    let slice = bytes.get(offset..end).ok_or(FoundationLoadError::Truncated)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, FoundationLoadError> {
    let end = offset.checked_add(4).ok_or(FoundationLoadError::Truncated)?;
    let slice = bytes.get(offset..end).ok_or(FoundationLoadError::Truncated)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, FoundationLoadError> {
    let end = offset.checked_add(8).ok_or(FoundationLoadError::Truncated)?;
    let slice = bytes.get(offset..end).ok_or(FoundationLoadError::Truncated)?;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn read_array32(bytes: &[u8], offset: usize) -> Result<[u8; 32], FoundationLoadError> {
    let end = offset.checked_add(32).ok_or(FoundationLoadError::Truncated)?;
    let slice = bytes.get(offset..end).ok_or(FoundationLoadError::Truncated)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(slice);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_foundation_loads() {
        let state = FoundationLoadState::load_embedded();
        assert!(state.is_ready(), "foundation failed: {:?}", state.error());
        assert!(state.record_count() >= 8);
        assert!(state.token_count() > 0);
        let memory = state.memory().unwrap();
        assert_eq!(memory.get(FoundationKey::AssistantName), Some("Wise Owl"));
    }

    #[test]
    fn tampered_blob_fails_integrity_check() {
        let mut tampered = EMBEDDED_FOUNDATION_BLOB.to_vec();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x5A;
        let err = FoundationMemory::decode_and_validate(&tampered).unwrap_err();
        assert_eq!(err, FoundationLoadError::IntegrityHashMismatch);
    }

    #[test]
    fn bad_magic_is_rejected() {
        let mut tampered = EMBEDDED_FOUNDATION_BLOB.to_vec();
        tampered[0] = b'X';
        let err = FoundationMemory::decode_and_validate(&tampered).unwrap_err();
        assert_eq!(err, FoundationLoadError::BadMagic);
    }
}
