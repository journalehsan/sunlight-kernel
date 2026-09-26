use wiseowl_identity::{
    validate_identity_set, GenesisEventKind, IdentityError, IdentityId, IdentityRoot,
    LineageEventId, LineageHead, LineageRecord,
};
use wiseowl_identity::lineage::{LINEAGE_HEAD_LEN, LINEAGE_RECORD_LEN};
use wiseowl_identity::root::IDENTITY_ROOT_LEN;

fn identity(seed: u8) -> IdentityId {
    IdentityId::from_bytes([seed; 32]).unwrap()
}

fn event(seed: u8) -> LineageEventId {
    LineageEventId::from_bytes([seed; 32]).unwrap()
}

fn set(kind: GenesisEventKind) -> (IdentityRoot, LineageRecord, LineageHead) {
    let id = identity(0x11);
    let event_id = event(0x22);
    let root = IdentityRoot::new(id, event_id, kind);
    let record = LineageRecord::genesis(id, event_id, kind);
    let head = LineageHead::from_record(&record);
    (root, record, head)
}

#[test]
fn root_roundtrip_and_strict_failures() {
    let (root, _, _) = set(GenesisEventKind::Created);
    let encoded = root.encode();
    assert_eq!(IdentityRoot::decode(&encoded).unwrap(), root);
    assert_eq!(IdentityRoot::decode(&encoded[..IDENTITY_ROOT_LEN - 1]), Err(IdentityError::InvalidLength));
    let mut corrupt = encoded;
    corrupt[20] ^= 1;
    assert_eq!(IdentityRoot::decode(&corrupt), Err(IdentityError::ChecksumMismatch));
    let mut newer = encoded;
    newer[8..10].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(IdentityRoot::decode(&newer), Err(IdentityError::UnsupportedVersion));
    let mut trailing = encoded.to_vec();
    trailing.push(0);
    assert_eq!(IdentityRoot::decode(&trailing), Err(IdentityError::TrailingData));
}

#[test]
fn both_phase_a_genesis_kinds_validate() {
    for kind in [GenesisEventKind::Created, GenesisEventKind::ExistingStateAdopted] {
        let (root, record, head) = set(kind);
        let decoded_record = LineageRecord::decode(&record.encode()).unwrap();
        let decoded_head = LineageHead::decode(&head.encode()).unwrap();
        let valid = validate_identity_set(&root, &decoded_record, &decoded_head).unwrap();
        assert_eq!(valid.genesis_event_kind, kind);
        assert_eq!(valid.lineage_sequence.get(), 1);
        assert_eq!(valid.continuity_generation.get(), 1);
    }
}

#[test]
fn lineage_rejects_sequence_generation_kind_hash_and_truncation() {
    let (_, record, _) = set(GenesisEventKind::Created);
    let encoded = record.encode();
    let mut wrong_sequence = encoded;
    wrong_sequence[44..52].copy_from_slice(&2u64.to_le_bytes());
    assert_eq!(LineageRecord::decode(&wrong_sequence), Err(IdentityError::InvalidSequence));
    let mut wrong_generation = encoded;
    wrong_generation[52..60].copy_from_slice(&2u64.to_le_bytes());
    assert_eq!(LineageRecord::decode(&wrong_generation), Err(IdentityError::InvalidContinuityGeneration));
    let mut invalid_kind = encoded;
    invalid_kind[92] = 9;
    assert_eq!(LineageRecord::decode(&invalid_kind), Err(IdentityError::InvalidEventKind));
    let mut corrupt_hash = encoded;
    corrupt_hash[LINEAGE_RECORD_LEN - 1] ^= 1;
    assert_eq!(LineageRecord::decode(&corrupt_hash), Err(IdentityError::HashMismatch));
    assert_eq!(LineageRecord::decode(&encoded[..LINEAGE_RECORD_LEN - 1]), Err(IdentityError::InvalidLength));
}

#[test]
fn set_rejects_wrong_identity_and_head_hash() {
    let (root, record, head) = set(GenesisEventKind::Created);
    let other_record = LineageRecord::genesis(identity(0x33), event(0x44), GenesisEventKind::Created);
    let other_head = LineageHead::from_record(&other_record);
    assert_eq!(validate_identity_set(&root, &other_record, &other_head), Err(IdentityError::IdentityMismatch));

    let other_same_identity = LineageRecord::genesis(identity(0x11), event(0x55), GenesisEventKind::Created);
    let mismatched_head = LineageHead::from_record(&other_same_identity);
    assert_eq!(
        validate_identity_set(&root, &record, &mismatched_head),
        Err(IdentityError::HashMismatch)
    );

    let encoded = head.encode();
    assert_eq!(LineageHead::decode(&encoded).unwrap(), head);
    assert_eq!(LineageHead::decode(&encoded[..LINEAGE_HEAD_LEN - 1]), Err(IdentityError::InvalidLength));
    let mut corrupt = encoded;
    corrupt[70] ^= 1;
    assert_eq!(LineageHead::decode(&corrupt), Err(IdentityError::ChecksumMismatch));
}
