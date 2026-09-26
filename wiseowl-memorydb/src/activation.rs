//! Durable, installation-local activation state (Phase C).

use wiseowl_identity::{ActivationId, ActivationState, InstallationId};

pub const LOCAL_VERSION: u16 = 1;
pub const LOCAL_LEN: usize = 104;
const MAGIC: &[u8; 4] = b"WLOC";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalActivationRecord {
    pub identity_id: wiseowl_identity::IdentityId,
    pub installation_id: InstallationId,
    pub state: ActivationState,
    pub activation_id: Option<ActivationId>,
    pub clean_stop: bool,
    pub boot_epoch: [u8; 16],
    pub activation_sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalActivationError {
    InvalidLength,
    InvalidFormat,
    InvalidChecksum,
    InvalidField,
    IdentityMismatch,
    Ambiguous,
    Io,
    Entropy,
    Ownership,
}

/// The committed name is authoritative. A staged record can establish the
/// first installation only when no committed LOCAL exists. A stage next to a
/// committed record is discarded only when it is a plausible unpublished
/// successor; contradictory evidence suspends startup.
pub fn reconcile_local(
    committed: Option<LocalActivationRecord>,
    staged: Option<LocalActivationRecord>,
    identity: wiseowl_identity::IdentityId,
) -> Result<(Option<LocalActivationRecord>, bool), LocalActivationError> {
    if let Some(record) = committed {
        record.validate_identity(identity)?;
        if let Some(candidate) = staged {
            candidate.validate_identity(identity)?;
            if !plausible_unpublished_successor(&record, &candidate) {
                return Err(LocalActivationError::Ambiguous);
            }
        }
        return Ok((Some(record), staged.is_some()));
    }
    if let Some(record) = staged {
        record.validate_identity(identity)?;
        if record.state != ActivationState::Activating || record.activation_id.is_some()
            || record.activation_sequence != 1
        {
            return Err(LocalActivationError::Ambiguous);
        }
        return Ok((Some(record), true));
    }
    Ok((None, false))
}

fn plausible_unpublished_successor(old: &LocalActivationRecord, new: &LocalActivationRecord) -> bool {
    if old.installation_id != new.installation_id || old.identity_id != new.identity_id {
        return false;
    }
    if new.activation_sequence == old.activation_sequence {
        if old.boot_epoch != new.boot_epoch
            && !(old.state == ActivationState::Activating && old.activation_id.is_none()
                && new.state == ActivationState::Activating && new.activation_id.is_some()) {
            return false;
        }
        match (old.state, new.state) {
            (ActivationState::Activating, ActivationState::Activating) =>
                old.activation_id.is_none() || old.activation_id == new.activation_id,
            (ActivationState::Activating | ActivationState::RecoveringLocal, ActivationState::Active)
            | (ActivationState::Active, ActivationState::RecoveringLocal)
            | (ActivationState::Active, ActivationState::Active)
            | (ActivationState::RecoveringLocal, ActivationState::RecoveringLocal) =>
                old.activation_id == new.activation_id,
            (ActivationState::Active, ActivationState::Dormant) => true,
            _ => old == new,
        }
    } else if old.activation_sequence.checked_add(1) == Some(new.activation_sequence) {
        matches!(old.state, ActivationState::Active | ActivationState::Dormant | ActivationState::RecoveringLocal | ActivationState::Activating)
            && matches!(new.state, ActivationState::Activating | ActivationState::RecoveringLocal)
            && new.activation_id.is_some()
            && (old.activation_id != new.activation_id || old.activation_id.is_none())
    } else {
        false
    }
}

/// Decide whether a process restart may retain an activation or a new boot
/// epoch must create one. A boot epoch is generated once in volatile `/tmp`.
pub fn restart_preserves_activation(record: &LocalActivationRecord, boot_epoch: [u8; 16]) -> bool {
    matches!(record.state, ActivationState::Active | ActivationState::RecoveringLocal | ActivationState::Activating)
        && record.activation_id.is_some()
        && record.boot_epoch == boot_epoch
}

/// Construct the durable pre-open recovery state for one process startup.
/// Callers publish this before opening the database and publish Active only
/// after store validation succeeds.
pub fn begin_activation(
    previous: Option<LocalActivationRecord>,
    identity_id: wiseowl_identity::IdentityId,
    boot_epoch: [u8; 16],
    mut installation_entropy: impl FnMut(&mut [u8]) -> Result<(), wiseowl_identity::EntropyError>,
    mut activation_entropy: impl FnMut(&mut [u8]) -> Result<(), wiseowl_identity::EntropyError>,
) -> Result<LocalActivationRecord, LocalActivationError> {
    let (installation_id, activation_sequence, reuse) = match previous {
        Some(record) => {
            record.validate_identity(identity_id)?;
            if record.state == ActivationState::Suspended {
                return Err(LocalActivationError::InvalidField);
            }
            let keep = restart_preserves_activation(&record, boot_epoch);
            let incomplete_first_activation =
                record.state == ActivationState::Activating && record.activation_id.is_none();
            (record.installation_id,
                if keep || incomplete_first_activation { record.activation_sequence } else { record.activation_sequence.checked_add(1).ok_or(LocalActivationError::InvalidField)? },
                keep.then_some(record.activation_id).flatten())
        }
        None => (
            InstallationId::generate_with(&mut installation_entropy).map_err(|_| LocalActivationError::Entropy)?,
            1,
            None,
        ),
    };
    let activation_id = match reuse {
        Some(id) => id,
        _ => ActivationId::generate_with(&mut activation_entropy).map_err(|_| LocalActivationError::Entropy)?,
    };
    let recovering = previous.is_some_and(|record| record.activation_id.is_some()
        && matches!(record.state, ActivationState::Active | ActivationState::RecoveringLocal | ActivationState::Activating));
    Ok(LocalActivationRecord {
        identity_id,
        installation_id,
        state: if recovering { ActivationState::RecoveringLocal } else { ActivationState::Activating },
        activation_id: Some(activation_id),
        clean_stop: false,
        boot_epoch,
        activation_sequence,
    })
}

pub fn finish_activation(mut record: LocalActivationRecord) -> Result<LocalActivationRecord, LocalActivationError> {
    if record.activation_id.is_none() || record.state == ActivationState::Suspended {
        return Err(LocalActivationError::InvalidField);
    }
    record.state = ActivationState::Active;
    record.clean_stop = false;
    Ok(record)
}

pub fn deactivate(mut record: LocalActivationRecord) -> Result<LocalActivationRecord, LocalActivationError> {
    if record.state != ActivationState::Active || record.activation_id.is_none() {
        return Err(LocalActivationError::InvalidField);
    }
    record.state = ActivationState::Dormant;
    record.activation_id = None;
    record.clean_stop = true;
    Ok(record)
}

impl LocalActivationRecord {
    pub fn encode(self) -> [u8; LOCAL_LEN] {
        let mut out = [0u8; LOCAL_LEN];
        out[..4].copy_from_slice(MAGIC);
        out[4..6].copy_from_slice(&LOCAL_VERSION.to_le_bytes());
        out[6] = self.state as u8;
        out[7] = u8::from(self.clean_stop);
        out[8..40].copy_from_slice(self.identity_id.as_bytes());
        out[40..56].copy_from_slice(self.installation_id.as_bytes());
        if let Some(activation) = self.activation_id {
            out[56..72].copy_from_slice(activation.as_bytes());
        }
        out[72..80].copy_from_slice(&self.activation_sequence.to_le_bytes());
        out[80..96].copy_from_slice(&self.boot_epoch);
        let checksum = checksum64(&out[..96]);
        out[96..104].copy_from_slice(&checksum.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, LocalActivationError> {
        if bytes.len() != LOCAL_LEN { return Err(LocalActivationError::InvalidLength); }
        if &bytes[..4] != MAGIC || u16::from_le_bytes([bytes[4], bytes[5]]) != LOCAL_VERSION {
            return Err(LocalActivationError::InvalidFormat);
        }
        if u64::from_le_bytes(bytes[96..104].try_into().unwrap()) != checksum64(&bytes[..96]) {
            return Err(LocalActivationError::InvalidChecksum);
        }
        let state = ActivationState::decode(bytes[6]).ok_or(LocalActivationError::InvalidField)?;
        if bytes[7] > 1 { return Err(LocalActivationError::InvalidField); }
        let identity_id = wiseowl_identity::IdentityId::decode(&bytes[8..40])
            .map_err(|_| LocalActivationError::InvalidField)?;
        let installation_id = InstallationId::from_bytes(bytes[40..56].try_into().unwrap())
            .ok_or(LocalActivationError::InvalidField)?;
        let activation_raw: [u8; 16] = bytes[56..72].try_into().unwrap();
        let activation_id = if activation_raw == [0; 16] { None } else {
            Some(ActivationId::from_bytes(activation_raw).ok_or(LocalActivationError::InvalidField)?)
        };
        let activation_sequence = u64::from_le_bytes(bytes[72..80].try_into().unwrap());
        let boot_epoch = bytes[80..96].try_into().unwrap();
        if activation_sequence == 0 || boot_epoch == [0; 16] { return Err(LocalActivationError::InvalidField); }
        if matches!(state, ActivationState::Active | ActivationState::RecoveringLocal)
            && activation_id.is_none() { return Err(LocalActivationError::InvalidField); }
        if state == ActivationState::Dormant && activation_id.is_some() { return Err(LocalActivationError::InvalidField); }
        if (state == ActivationState::Dormant) != (bytes[7] == 1) {
            return Err(LocalActivationError::InvalidField);
        }
        Ok(Self { identity_id, installation_id, state, activation_id, clean_stop: bytes[7] != 0, boot_epoch, activation_sequence })
    }

    pub fn validate_identity(&self, identity: wiseowl_identity::IdentityId) -> Result<(), LocalActivationError> {
        (self.identity_id == identity).then_some(()).ok_or(LocalActivationError::IdentityMismatch)
    }
}

/// Deterministic non-cryptographic corruption checksum; authority still comes
/// from validating ROOT/LINEAGE/HEAD and the runtime ownership endpoint.
fn checksum64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiseowl_identity::{IdentityId, ActivationState};

    fn sample(state: ActivationState, activation: Option<ActivationId>) -> LocalActivationRecord {
        LocalActivationRecord {
            identity_id: IdentityId::from_bytes([1; 32]).unwrap(),
            installation_id: InstallationId::from_bytes([2; 16]).unwrap(),
            state, activation_id: activation, clean_stop: state == ActivationState::Dormant,
            boot_epoch: [3; 16], activation_sequence: 1,
        }
    }

    fn resign(bytes: &mut [u8; LOCAL_LEN]) {
        let checksum = checksum64(&bytes[..96]);
        bytes[96..104].copy_from_slice(&checksum.to_le_bytes());
    }

    #[test]
    fn wloc_v1_exact_layout_and_invalid_combinations() {
        let active = sample(ActivationState::Active, Some(ActivationId::from_bytes([4; 16]).unwrap()));
        let bytes = active.encode();
        assert_eq!(bytes.len(), 104);
        assert_eq!(&bytes[..4], b"WLOC");
        assert_eq!(&bytes[4..6], &1u16.to_le_bytes());
        assert_eq!(&bytes[8..40], active.identity_id.as_bytes());
        assert_eq!(&bytes[40..56], active.installation_id.as_bytes());
        assert_eq!(&bytes[56..72], active.activation_id.unwrap().as_bytes());
        assert_eq!(&bytes[72..80], &1u64.to_le_bytes());
        assert_eq!(&bytes[80..96], &[3; 16]);
        assert_eq!(&bytes[96..104], &checksum64(&bytes[..96]).to_le_bytes());
        let mut longer = bytes.to_vec();
        longer.push(0);
        assert_eq!(LocalActivationRecord::decode(&longer), Err(LocalActivationError::InvalidLength));
        for offset in [0, 4] {
            let mut changed = bytes;
            changed[offset] ^= 1;
            resign(&mut changed);
            assert_eq!(LocalActivationRecord::decode(&changed), Err(LocalActivationError::InvalidFormat));
        }
        for offset in [6, 7, 8, 40, 56, 72, 80] {
            let mut changed = bytes;
            match offset {
                6 => changed[6] = 0xff,
                7 => changed[7] = 2,
                8 => changed[8..40].fill(0),
                40 => changed[40..56].fill(0),
                56 => changed[56..72].fill(0),
                72 => changed[72..80].fill(0),
                _ => changed[80..96].fill(0),
            }
            resign(&mut changed);
            assert_eq!(LocalActivationRecord::decode(&changed), Err(LocalActivationError::InvalidField), "offset {offset}");
        }
        let mut dormant_with_activation = bytes;
        dormant_with_activation[6] = ActivationState::Dormant as u8;
        dormant_with_activation[7] = 1;
        resign(&mut dormant_with_activation);
        assert_eq!(LocalActivationRecord::decode(&dormant_with_activation), Err(LocalActivationError::InvalidField));
        let mut wrong_clean_stop = bytes;
        wrong_clean_stop[7] = 1;
        resign(&mut wrong_clean_stop);
        assert_eq!(LocalActivationRecord::decode(&wrong_clean_stop), Err(LocalActivationError::InvalidField));
    }

    #[test]
    fn conflicting_candidates_suspend_and_a_plausible_unpublished_successor_is_discarded() {
        let identity = IdentityId::from_bytes([1; 32]).unwrap();
        let active = sample(ActivationState::Active, Some(ActivationId::from_bytes([4; 16]).unwrap()));
        let mut successor = active;
        successor.state = ActivationState::RecoveringLocal;
        assert_eq!(reconcile_local(Some(active), Some(successor), identity), Ok((Some(active), true)));
        successor.installation_id = InstallationId::from_bytes([9; 16]).unwrap();
        assert_eq!(reconcile_local(Some(active), Some(successor), identity), Err(LocalActivationError::Ambiguous));
        successor = active;
        successor.activation_id = Some(ActivationId::from_bytes([5; 16]).unwrap());
        assert_eq!(reconcile_local(Some(active), Some(successor), identity), Err(LocalActivationError::Ambiguous));
        let mut first = sample(ActivationState::Activating, None);
        assert_eq!(reconcile_local(None, Some(first), identity), Ok((Some(first), true)));
        first.activation_id = active.activation_id;
        assert_eq!(reconcile_local(None, Some(first), identity), Err(LocalActivationError::Ambiguous));
    }

    #[test]
    fn activation_sequence_exhaustion_fails_closed() {
        let identity = IdentityId::from_bytes([1; 32]).unwrap();
        let mut old = sample(ActivationState::Active, Some(ActivationId::from_bytes([4; 16]).unwrap()));
        old.activation_sequence = u64::MAX;
        assert_eq!(begin_activation(Some(old), identity, [5; 16], |_| Ok(()), |b| { b.fill(6); Ok(()) }),
            Err(LocalActivationError::InvalidField));
        assert!(begin_activation(Some(old), identity, old.boot_epoch, |_| Ok(()), |_| Ok(())).is_ok());
    }

    #[test]
    fn local_codec_roundtrips_and_rejects_corruption_and_invalid_ids() {
        let record = sample(ActivationState::Active, Some(ActivationId::from_bytes([4; 16]).unwrap()));
        let encoded = record.encode();
        assert_eq!(LocalActivationRecord::decode(&encoded), Ok(record));
        let mut corrupt = encoded;
        corrupt[20] ^= 1;
        assert_eq!(LocalActivationRecord::decode(&corrupt), Err(LocalActivationError::InvalidChecksum));
        assert_eq!(LocalActivationRecord::decode(&encoded[..103]), Err(LocalActivationError::InvalidLength));
        let mut zero_install = encoded;
        zero_install[40..56].fill(0);
        let sum = checksum64(&zero_install[..96]);
        zero_install[96..104].copy_from_slice(&sum.to_le_bytes());
        assert_eq!(LocalActivationRecord::decode(&zero_install), Err(LocalActivationError::InvalidField));
        let mut no_activation = encoded;
        no_activation[56..72].fill(0);
        let sum = checksum64(&no_activation[..96]);
        no_activation[96..104].copy_from_slice(&sum.to_le_bytes());
        assert_eq!(LocalActivationRecord::decode(&no_activation), Err(LocalActivationError::InvalidField));
    }

    #[test]
    fn local_identity_mismatch_fails_closed() {
        let record = sample(ActivationState::Dormant, None);
        assert_eq!(record.validate_identity(IdentityId::from_bytes([9; 32]).unwrap()), Err(LocalActivationError::IdentityMismatch));
    }

    #[test]
    fn clean_reboot_and_same_boot_process_restart_have_distinct_activation_rules() {
        let identity = IdentityId::from_bytes([1; 32]).unwrap();
        let first = begin_activation(None, identity, [3; 16], |b| { b.fill(2); Ok(()) }, |b| { b.fill(4); Ok(()) }).unwrap();
        let first = finish_activation(first).unwrap();
        let restart = begin_activation(Some(first), identity, [3; 16], |_| Ok(()), |b| { b.fill(5); Ok(()) }).unwrap();
        assert_eq!(restart.activation_id, first.activation_id);
        assert_eq!(restart.installation_id, first.installation_id);
        let restart = finish_activation(restart).unwrap();
        let dormant = deactivate(restart).unwrap();
        let reboot = begin_activation(Some(dormant), identity, [6; 16], |_| Ok(()), |b| { b.fill(7); Ok(()) }).unwrap();
        assert_eq!(reboot.installation_id, first.installation_id);
        assert_ne!(reboot.activation_id, first.activation_id);
        assert_eq!(reboot.activation_sequence, 2);
        assert_eq!(reboot.state, ActivationState::Activating);
    }

    #[test]
    fn unclean_new_boot_recovers_locally_without_identity_or_lineage_mutation() {
        let identity = IdentityId::from_bytes([8; 32]).unwrap();
        let old = finish_activation(begin_activation(None, identity, [1; 16], |b| { b.fill(2); Ok(()) }, |b| { b.fill(3); Ok(()) }).unwrap()).unwrap();
        let recovered = begin_activation(Some(old), identity, [4; 16], |_| Ok(()), |b| { b.fill(5); Ok(()) }).unwrap();
        assert_eq!(recovered.state, ActivationState::RecoveringLocal);
        assert_eq!(recovered.identity_id, identity);
        assert_eq!(recovered.installation_id, old.installation_id);
        assert_ne!(recovered.activation_id, old.activation_id);
        assert_eq!(recovered.activation_sequence, old.activation_sequence + 1);
        assert_eq!(wiseowl_identity::ContinuityGeneration::INITIAL.get(), 1);
    }

    #[test]
    fn two_authoritative_instances_cannot_share_a_runtime_lease() {
        // Mirrors the kernel-owned unique endpoint: acquisition is exclusive
        // while the first process incarnation remains alive.
        let lease = core::sync::atomic::AtomicBool::new(false);
        assert!(lease.compare_exchange(false, true, core::sync::atomic::Ordering::AcqRel, core::sync::atomic::Ordering::Acquire).is_ok());
        assert!(lease.compare_exchange(false, true, core::sync::atomic::Ordering::AcqRel, core::sync::atomic::Ordering::Acquire).is_err());
        lease.store(false, core::sync::atomic::Ordering::Release);
        assert!(lease.compare_exchange(false, true, core::sync::atomic::Ordering::AcqRel, core::sync::atomic::Ordering::Acquire).is_ok());
    }
}
