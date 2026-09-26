use core::fmt;

use crate::codec::{read_u16, read_u64, require_exact, sha256, short_hex};
use crate::{IdentityError, IdentityId, IDENTITY_FORMAT_VERSION};

const LINEAGE_MAGIC: &[u8; 8] = b"WOIDLIN1";
const LINEAGE_DOMAIN: &[u8] = b"wiseowl.identity.lineage.v1\0";
pub const LINEAGE_RECORD_LEN: usize = 164;
const LINEAGE_HASH_OFFSET: usize = LINEAGE_RECORD_LEN - 32;

const HEAD_MAGIC: &[u8; 8] = b"WOIDHEAD";
const HEAD_DOMAIN: &[u8] = b"wiseowl.identity.head.v1\0";
pub const LINEAGE_HEAD_LEN: usize = 124;
const HEAD_CHECKSUM_OFFSET: usize = LINEAGE_HEAD_LEN - 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineageSequence(u64);

impl LineageSequence {
    pub const GENESIS: Self = Self(1);
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContinuityGeneration(u64);

impl ContinuityGeneration {
    pub const INITIAL: Self = Self(1);
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineageEventId([u8; 32]);

impl LineageEventId {
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, IdentityError> {
        if bytes == [0; 32] {
            return Err(IdentityError::InvalidEventId);
        }
        Ok(Self(bytes))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IdentityError> {
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| IdentityError::InvalidLength)?;
        Self::from_bytes(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for LineageEventId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fingerprint = short_hex(&self.0);
        let text = core::str::from_utf8(&fingerprint).map_err(|_| fmt::Error)?;
        write!(formatter, "LineageEventId({text}…)")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum GenesisEventKind {
    Created = 1,
    ExistingStateAdopted = 2,
}

impl GenesisEventKind {
    fn decode(value: u8) -> Result<Self, IdentityError> {
        match value {
            1 => Ok(Self::Created),
            2 => Ok(Self::ExistingStateAdopted),
            _ => Err(IdentityError::InvalidEventKind),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineageRecord {
    identity_id: IdentityId,
    sequence: LineageSequence,
    continuity_generation: ContinuityGeneration,
    event_id: LineageEventId,
    event_kind: GenesisEventKind,
    previous_hash: [u8; 32],
    record_hash: [u8; 32],
}

impl LineageRecord {
    pub fn genesis(
        identity_id: IdentityId,
        event_id: LineageEventId,
        event_kind: GenesisEventKind,
    ) -> Self {
        let mut record = Self {
            identity_id,
            sequence: LineageSequence::GENESIS,
            continuity_generation: ContinuityGeneration::INITIAL,
            event_id,
            event_kind,
            previous_hash: [0; 32],
            record_hash: [0; 32],
        };
        record.record_hash = record.calculate_hash();
        record
    }

    pub const fn identity_id(self) -> IdentityId { self.identity_id }
    pub const fn sequence(self) -> LineageSequence { self.sequence }
    pub const fn continuity_generation(self) -> ContinuityGeneration { self.continuity_generation }
    pub const fn event_id(self) -> LineageEventId { self.event_id }
    pub const fn event_kind(self) -> GenesisEventKind { self.event_kind }
    pub const fn record_hash(self) -> [u8; 32] { self.record_hash }

    fn encode_without_hash(self) -> [u8; LINEAGE_HASH_OFFSET] {
        let mut out = [0u8; LINEAGE_HASH_OFFSET];
        out[..8].copy_from_slice(LINEAGE_MAGIC);
        out[8..10].copy_from_slice(&IDENTITY_FORMAT_VERSION.to_le_bytes());
        out[10..12].copy_from_slice(&(LINEAGE_RECORD_LEN as u16).to_le_bytes());
        out[12..44].copy_from_slice(self.identity_id.as_bytes());
        out[44..52].copy_from_slice(&self.sequence.get().to_le_bytes());
        out[52..60].copy_from_slice(&self.continuity_generation.get().to_le_bytes());
        out[60..92].copy_from_slice(self.event_id.as_bytes());
        out[92] = self.event_kind as u8;
        // 93..100 is reserved and must remain zero.
        out[100..132].copy_from_slice(&self.previous_hash);
        out
    }

    fn calculate_hash(self) -> [u8; 32] {
        sha256(LINEAGE_DOMAIN, &self.encode_without_hash())
    }

    pub fn encode(self) -> [u8; LINEAGE_RECORD_LEN] {
        let mut out = [0u8; LINEAGE_RECORD_LEN];
        out[..LINEAGE_HASH_OFFSET].copy_from_slice(&self.encode_without_hash());
        out[LINEAGE_HASH_OFFSET..].copy_from_slice(&self.record_hash);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IdentityError> {
        require_exact(bytes, LINEAGE_RECORD_LEN)?;
        if &bytes[..8] != LINEAGE_MAGIC { return Err(IdentityError::InvalidMagic); }
        if read_u16(bytes, 8) != IDENTITY_FORMAT_VERSION { return Err(IdentityError::UnsupportedVersion); }
        if read_u16(bytes, 10) as usize != LINEAGE_RECORD_LEN { return Err(IdentityError::InvalidLength); }
        if bytes[93..100] != [0; 7] { return Err(IdentityError::UnsupportedVersion); }
        let sequence = read_u64(bytes, 44);
        if sequence != 1 { return Err(IdentityError::InvalidSequence); }
        let generation = read_u64(bytes, 52);
        if generation != 1 { return Err(IdentityError::InvalidContinuityGeneration); }
        if bytes[100..132] != [0; 32] { return Err(IdentityError::InvalidGenesis); }
        let mut record_hash = [0u8; 32];
        record_hash.copy_from_slice(&bytes[LINEAGE_HASH_OFFSET..]);
        let record = Self {
            identity_id: IdentityId::decode(&bytes[12..44])?,
            sequence: LineageSequence::GENESIS,
            continuity_generation: ContinuityGeneration::INITIAL,
            event_id: LineageEventId::decode(&bytes[60..92])?,
            event_kind: GenesisEventKind::decode(bytes[92])?,
            previous_hash: [0; 32],
            record_hash,
        };
        if record.calculate_hash() != record.record_hash { return Err(IdentityError::HashMismatch); }
        Ok(record)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineageHead {
    identity_id: IdentityId,
    sequence: LineageSequence,
    continuity_generation: ContinuityGeneration,
    committed_record_hash: [u8; 32],
}

impl LineageHead {
    pub fn from_record(record: &LineageRecord) -> Self {
        Self {
            identity_id: record.identity_id,
            sequence: record.sequence,
            continuity_generation: record.continuity_generation,
            committed_record_hash: record.record_hash,
        }
    }

    pub const fn identity_id(self) -> IdentityId { self.identity_id }
    pub const fn sequence(self) -> LineageSequence { self.sequence }
    pub const fn continuity_generation(self) -> ContinuityGeneration { self.continuity_generation }
    pub const fn committed_record_hash(self) -> [u8; 32] { self.committed_record_hash }

    pub fn encode(self) -> [u8; LINEAGE_HEAD_LEN] {
        let mut out = [0u8; LINEAGE_HEAD_LEN];
        out[..8].copy_from_slice(HEAD_MAGIC);
        out[8..10].copy_from_slice(&IDENTITY_FORMAT_VERSION.to_le_bytes());
        out[10..12].copy_from_slice(&(LINEAGE_HEAD_LEN as u16).to_le_bytes());
        out[12..44].copy_from_slice(self.identity_id.as_bytes());
        out[44..52].copy_from_slice(&self.sequence.get().to_le_bytes());
        out[52..60].copy_from_slice(&self.continuity_generation.get().to_le_bytes());
        out[60..92].copy_from_slice(&self.committed_record_hash);
        let checksum = sha256(HEAD_DOMAIN, &out[..HEAD_CHECKSUM_OFFSET]);
        out[HEAD_CHECKSUM_OFFSET..].copy_from_slice(&checksum);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, IdentityError> {
        require_exact(bytes, LINEAGE_HEAD_LEN)?;
        if &bytes[..8] != HEAD_MAGIC { return Err(IdentityError::InvalidMagic); }
        if read_u16(bytes, 8) != IDENTITY_FORMAT_VERSION { return Err(IdentityError::UnsupportedVersion); }
        if read_u16(bytes, 10) as usize != LINEAGE_HEAD_LEN { return Err(IdentityError::InvalidLength); }
        if sha256(HEAD_DOMAIN, &bytes[..HEAD_CHECKSUM_OFFSET]) != bytes[HEAD_CHECKSUM_OFFSET..] {
            return Err(IdentityError::ChecksumMismatch);
        }
        let sequence = read_u64(bytes, 44);
        if sequence != 1 { return Err(IdentityError::InvalidSequence); }
        let generation = read_u64(bytes, 52);
        if generation != 1 { return Err(IdentityError::InvalidContinuityGeneration); }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&bytes[60..92]);
        Ok(Self {
            identity_id: IdentityId::decode(&bytes[12..44])?,
            sequence: LineageSequence::GENESIS,
            continuity_generation: ContinuityGeneration::INITIAL,
            committed_record_hash: hash,
        })
    }

    pub fn validate_record(self, record: &LineageRecord) -> Result<(), IdentityError> {
        if self.identity_id != record.identity_id { return Err(IdentityError::IdentityMismatch); }
        if self.sequence != record.sequence { return Err(IdentityError::SequenceMismatch); }
        if self.continuity_generation != record.continuity_generation {
            return Err(IdentityError::GenerationMismatch);
        }
        if self.committed_record_hash != record.record_hash { return Err(IdentityError::HashMismatch); }
        Ok(())
    }
}
