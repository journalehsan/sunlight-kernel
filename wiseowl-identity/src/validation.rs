#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityError {
    InvalidLength,
    InvalidMagic,
    UnsupportedVersion,
    InvalidIdentityId,
    InvalidEventId,
    InvalidSequence,
    InvalidContinuityGeneration,
    InvalidEventKind,
    InvalidGenesis,
    IdentityMismatch,
    SequenceMismatch,
    GenerationMismatch,
    HashMismatch,
    ChecksumMismatch,
    MissingLineageRecord,
    TrailingData,
}

use crate::{
    ContinuityGeneration, GenesisEventKind, IdentityId, IdentityRoot, LineageHead, LineageRecord,
    LineageSequence,
};

/// Read-only result of validating ROOT, the committed genesis record, and HEAD
/// as one inseparable durable identity set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValidatedIdentity {
    pub identity_id: IdentityId,
    pub lineage_sequence: LineageSequence,
    pub continuity_generation: ContinuityGeneration,
    pub genesis_event_kind: GenesisEventKind,
}

pub fn validate_identity_set(
    root: &IdentityRoot,
    lineage: &LineageRecord,
    head: &LineageHead,
) -> Result<ValidatedIdentity, IdentityError> {
    if root.identity_id() != lineage.identity_id() || root.identity_id() != head.identity_id() {
        return Err(IdentityError::IdentityMismatch);
    }
    if root.creation_event_id() != lineage.event_id() {
        return Err(IdentityError::InvalidGenesis);
    }
    if root.creation_event_kind() != lineage.event_kind() {
        return Err(IdentityError::InvalidGenesis);
    }
    head.validate_record(lineage)?;
    Ok(ValidatedIdentity {
        identity_id: root.identity_id(),
        lineage_sequence: lineage.sequence(),
        continuity_generation: lineage.continuity_generation(),
        genesis_event_kind: lineage.event_kind(),
    })
}
