use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use wiseowl_identity::{ActivationId, ActivationState, InstallationId};
use wiseowl_memorydb::activation::{begin_activation, finish_activation, reconcile_local, LocalActivationRecord};
use wiseowl_memorydb::database::{validate_store_read_only, Database, DbCaller, FsStore};
use wiseowl_memorydb::health::HealthState;
use wiseowl_memorydb::identity::{
    detect_store_state, ensure_identity, host::HostIdentityStorage, AdoptionPolicy,
    DetectedStoreState, StoreDisposition,
};
use wiseowl_memorydb::owlql::parse_owlql;
use wiseowl_memorydb::protocol::{DbRequest, DbResponse};
use wiseowl_memorydb::{DbCapabilitySet, DbQuotaConfig};

fn host_boot_epoch() -> std::io::Result<[u8; 16]> {
    let text = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    let mut digits = [0u8; 32];
    let mut count = 0usize;
    for byte in text.bytes() {
        if byte != b'-' && byte != b'\n' {
            if count == digits.len() { return Err(std::io::ErrorKind::InvalidData.into()); }
            digits[count] = byte;
            count += 1;
        }
    }
    if count != digits.len() { return Err(std::io::ErrorKind::InvalidData.into()); }
    let mut epoch = [0u8; 16];
    for (index, pair) in digits.chunks_exact(2).enumerate() {
        let hex = |b: u8| match b { b'0'..=b'9' => Some(b - b'0'), b'a'..=b'f' => Some(b - b'a' + 10), b'A'..=b'F' => Some(b - b'A' + 10), _ => None };
        epoch[index] = (hex(pair[0]).ok_or(std::io::ErrorKind::InvalidData)? << 4)
            | hex(pair[1]).ok_or(std::io::ErrorKind::InvalidData)?;
    }
    if epoch == [0; 16] { return Err(std::io::ErrorKind::InvalidData.into()); }
    Ok(epoch)
}

fn sync_directory(path: &std::path::Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalBoundary {
    BeforeCreate, AfterCreate, AfterWrite, AfterFileSync, BeforeStageSync,
    AfterStageSync, BeforeRename, AfterRename, BeforeParentSync, AfterParentSync,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LocalTransition { FirstInstallation, BeginActivation, FinishActivation }

fn publish_local_with_fault(
    path: &std::path::Path,
    record: LocalActivationRecord,
    transition: LocalTransition,
    fault: &mut impl FnMut(LocalTransition, LocalBoundary) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let temp = parent.join("LOCAL.tmp");
    fault(transition, LocalBoundary::BeforeCreate)?;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temp)?;
    fault(transition, LocalBoundary::AfterCreate)?;
    file.write_all(&record.encode())?;
    fault(transition, LocalBoundary::AfterWrite)?;
    file.sync_all()?;
    fault(transition, LocalBoundary::AfterFileSync)?;
    fault(transition, LocalBoundary::BeforeStageSync)?;
    sync_directory(parent)?;
    fault(transition, LocalBoundary::AfterStageSync)?;
    fault(transition, LocalBoundary::BeforeRename)?;
    std::fs::rename(&temp, path)?;
    fault(transition, LocalBoundary::AfterRename)?;
    fault(transition, LocalBoundary::BeforeParentSync)?;
    sync_directory(parent)?;
    fault(transition, LocalBoundary::AfterParentSync)
}

fn load_local(path: &std::path::Path) -> std::io::Result<Option<LocalActivationRecord>> {
    if !path.exists() { return Ok(None); }
    let bytes = std::fs::read(path)?;
    LocalActivationRecord::decode(&bytes)
        .map(Some)
        .map_err(|_| std::io::ErrorKind::InvalidData.into())
}

fn start_local_activation(
    identity: wiseowl_identity::IdentityId,
    identity_dir: &std::path::Path,
    boot_epoch: [u8; 16],
) -> std::io::Result<ActivationId> {
    start_local_activation_with_fault(identity, identity_dir, boot_epoch, &mut |_, _| Ok(()))
}

fn start_local_activation_with_fault(
    identity: wiseowl_identity::IdentityId,
    identity_dir: &std::path::Path,
    boot_epoch: [u8; 16],
    fault: &mut impl FnMut(LocalTransition, LocalBoundary) -> std::io::Result<()>,
) -> std::io::Result<ActivationId> {
    let local = identity_dir.join("LOCAL");
    let temp = identity_dir.join("LOCAL.tmp");
    for entry in std::fs::read_dir(identity_dir)? {
        let name = entry?.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("LOCAL.") && name != "LOCAL.tmp" {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    }
    let committed = load_local(&local)?;
    let staged = if temp.exists() { load_local(&temp).ok().flatten() } else { None };
    let (mut previous, had_stage) = reconcile_local(committed, staged, identity)
        .map_err(|_| std::io::ErrorKind::InvalidData)?;
    if temp.exists() {
        if committed.is_some() || !had_stage {
            std::fs::remove_file(&temp)?;
            sync_directory(identity_dir)?;
        } else {
            std::fs::rename(&temp, &local)?;
            sync_directory(identity_dir)?;
        }
    }
    if previous.is_none() {
        let install = InstallationId::generate().map_err(|_| std::io::ErrorKind::Other)?;
        let initial = LocalActivationRecord {
            identity_id: identity,
            installation_id: install,
            state: ActivationState::Activating,
            activation_id: None,
            clean_stop: false,
            boot_epoch,
            activation_sequence: 1,
        };
        publish_local_with_fault(&local, initial, LocalTransition::FirstInstallation, fault)?;
        previous = Some(initial);
    }
    let activating = begin_activation(
        previous,
        identity,
        boot_epoch,
        wiseowl_identity::fill_host_entropy,
        wiseowl_identity::fill_host_entropy,
    ).map_err(|_| std::io::ErrorKind::InvalidData)?;
    let activation = activating.activation_id.ok_or(std::io::ErrorKind::InvalidData)?;
    publish_local_with_fault(&local, activating, LocalTransition::BeginActivation, fault)?;
    Ok(activation)
}

fn finish_local_activation(
    identity: wiseowl_identity::IdentityId,
    identity_dir: &std::path::Path,
) -> std::io::Result<ActivationId> {
    finish_local_activation_with_fault(identity, identity_dir, &mut |_, _| Ok(()))
}

fn finish_local_activation_with_fault(
    identity: wiseowl_identity::IdentityId,
    identity_dir: &std::path::Path,
    fault: &mut impl FnMut(LocalTransition, LocalBoundary) -> std::io::Result<()>,
) -> std::io::Result<ActivationId> {
    let local = identity_dir.join("LOCAL");
    let record = load_local(&local)?.ok_or(std::io::ErrorKind::NotFound)?;
    record.validate_identity(identity).map_err(|_| std::io::ErrorKind::InvalidData)?;
    let activation = record.activation_id.ok_or(std::io::ErrorKind::InvalidData)?;
    let active = finish_activation(record).map_err(|_| std::io::ErrorKind::InvalidData)?;
    publish_local_with_fault(&local, active, LocalTransition::FinishActivation, fault)?;
    Ok(activation)
}

fn main() {
    let socket = std::env::var("WISEOWL_MEMORYDB_SOCKET")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-memorydb.sock".to_string());
    let data_dir = std::env::var("WISEOWL_MEMORYDB_DIR")
        .unwrap_or_else(|_| "/tmp/sunlight/wiseowl-memorydb".to_string());

    if let Some(parent) = PathBuf::from(&socket).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::create_dir_all(&data_dir);
    let _writer_authority = match wiseowl_memorydb::host_authority::HostWriterAuthority::acquire(&PathBuf::from(&data_dir)) {
        Ok(guard) => guard,
        Err(error) => {
            eprintln!("Wise Owl local writer authority unavailable: {error}");
            return;
        }
    };
    if PathBuf::from(&socket).exists() {
        // The lock proves no current daemon owns the store; this can only be
        // a leftover socket from a crashed previous process.
        let _ = std::fs::remove_file(&socket);
    }

    let mut identity_storage = match HostIdentityStorage::open(&data_dir) {
        Ok(storage) => storage,
        Err(error) => {
            eprintln!("Wise Owl identity storage unavailable: {error:?}");
            return;
        }
    };
    let detected = match detect_store_state(&identity_storage) {
        Ok(state) => state,
        Err(error) => {
            eprintln!("Wise Owl identity state detection failed: {error:?}");
            return;
        }
    };
    let disposition = match detected {
        DetectedStoreState::Fresh => StoreDisposition::Fresh,
        DetectedStoreState::Uncertain => StoreDisposition::ExistingInvalidOrUncertain,
        DetectedStoreState::Existing => {
            let validation_store = match FsStore::open(&data_dir) {
                Ok(store) => store,
                Err(_) => {
                    eprintln!("Wise Owl pre-identity store could not be opened for validation");
                    return;
                }
            };
            if validate_store_read_only(&validation_store, DbQuotaConfig::default()).is_ok() {
                StoreDisposition::ExistingValidated
            } else {
                StoreDisposition::ExistingInvalidOrUncertain
            }
        }
    };
    let adoption_policy = AdoptionPolicy::AllowValidatedExistingState;
    let identity = match ensure_identity(
        &mut identity_storage,
        disposition,
        adoption_policy,
        wiseowl_identity::fill_host_entropy,
        |_| Ok(()),
    ) {
        Ok(identity) => identity,
        Err(error) => {
            eprintln!("Wise Owl identity suspended: {error:?}");
            return;
        }
    };
    let fingerprint = identity.diagnostic_fingerprint();
    eprintln!(
        "Wise Owl identity loaded: woid fingerprint {}",
        std::str::from_utf8(&fingerprint).unwrap_or("????????")
    );
    if identity.lineage_sequence().get() != 1 || identity.continuity_generation().get() != 1 {
        eprintln!("Wise Owl Phase C continuity invariant failed; activation suspended");
        return;
    }
    let identity_dir = PathBuf::from(&data_dir).join("IDENTITY");
    let boot_epoch = match host_boot_epoch() {
        Ok(epoch) => epoch,
        Err(error) => {
            eprintln!("Wise Owl boot epoch unavailable: {error}");
            return;
        }
    };
    let activation = match start_local_activation(identity.identity_id(), &identity_dir, boot_epoch) {
        Ok(activation) => activation,
        Err(error) => {
            eprintln!("Wise Owl local activation suspended: {error}");
            return;
        }
    };
    eprintln!(
        "Wise Owl activation recovering locally: {}",
        std::str::from_utf8(&activation.fingerprint()).unwrap_or("????????")
    );

    let store = FsStore::open(&data_dir).expect("open wiseowl-memorydb store");
    let mut db =
        Database::open_with_store(store, DbQuotaConfig::default()).expect("open wiseowl-memorydb");
    db.bind_identity_context(identity);
    db.bind_activation(activation.fingerprint());
    if db.identity_context().is_none() {
        eprintln!("Wise Owl identity context unavailable; service startup suspended");
        return;
    }
    match finish_local_activation(identity.identity_id(), &identity_dir) {
        Ok(active) if active == activation => {}
        Ok(_) => {
            eprintln!("Wise Owl activation changed during startup; service suspended");
            return;
        }
        Err(error) => {
            eprintln!("Wise Owl activation commit failed: {error}");
            return;
        }
    }
    let db = Arc::new(Mutex::new(db));

    let listener = UnixListener::bind(&socket).expect("bind socket");
    eprintln!("wiseowl-memorydb listening on {socket} (data {data_dir})");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let db = Arc::clone(&db);
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream, db) {
                        eprintln!("client error: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept error: {e}"),
        }
    }
}

#[cfg(test)]
mod phase_c_host_tests {
    use super::*;

    const BOUNDARIES: [LocalBoundary; 10] = [
        LocalBoundary::BeforeCreate, LocalBoundary::AfterCreate,
        LocalBoundary::AfterWrite, LocalBoundary::AfterFileSync,
        LocalBoundary::BeforeStageSync, LocalBoundary::AfterStageSync,
        LocalBoundary::BeforeRename, LocalBoundary::AfterRename,
        LocalBoundary::BeforeParentSync, LocalBoundary::AfterParentSync,
    ];

    fn fail_at(transition: LocalTransition, boundary: LocalBoundary)
        -> impl FnMut(LocalTransition, LocalBoundary) -> std::io::Result<()> {
        move |seen_transition, seen_boundary| {
            if seen_transition == transition && seen_boundary == boundary {
                Err(std::io::ErrorKind::Interrupted.into())
            } else { Ok(()) }
        }
    }

    #[test]
    fn every_local_publication_boundary_recovers_without_changing_identity_or_installation() {
        let identity = wiseowl_identity::IdentityId::from_bytes([0x31; 32]).unwrap();
        for transition in [LocalTransition::FirstInstallation, LocalTransition::BeginActivation,
            LocalTransition::FinishActivation] {
            for boundary in BOUNDARIES {
                let root = tempfile::tempdir().unwrap();
                let dir = root.path().join("IDENTITY");
                std::fs::create_dir(&dir).unwrap();
                let boot = [0x41; 16];
                let mut original_install = None;
                if transition == LocalTransition::FinishActivation {
                    start_local_activation(identity, &dir, boot).unwrap();
                    original_install = load_local(&dir.join("LOCAL")).unwrap().map(|r| r.installation_id);
                }
                let result = if transition == LocalTransition::FinishActivation {
                    finish_local_activation_with_fault(identity, &dir, &mut fail_at(transition, boundary)).map(|_| ())
                } else {
                    start_local_activation_with_fault(identity, &dir, boot, &mut fail_at(transition, boundary)).map(|_| ())
                };
                assert!(result.is_err(), "{transition:?} {boundary:?}");
                let recoverable = load_local(&dir.join("LOCAL")).ok().flatten()
                    .or_else(|| load_local(&dir.join("LOCAL.tmp")).ok().flatten());
                let activation = start_local_activation(identity, &dir, boot).unwrap();
                assert_eq!(finish_local_activation(identity, &dir).unwrap(), activation);
                let recovered = load_local(&dir.join("LOCAL")).unwrap().unwrap();
                assert_eq!(recovered.identity_id, identity);
                assert_eq!(recovered.state, ActivationState::Active);
                if let Some(expected) = original_install.or(recoverable.map(|r| r.installation_id)) {
                    assert_eq!(recovered.installation_id, expected, "{transition:?} {boundary:?}");
                }
                assert_eq!(recovered.activation_sequence, 1);
                assert_eq!(wiseowl_identity::ContinuityGeneration::INITIAL.get(), 1);
                assert!(!dir.join("LOCAL.tmp").exists());
            }
        }
    }

    #[test]
    fn active_recovery_fault_matrix_preserves_same_boot_activation_and_rotates_new_boot_activation() {
        let identity = wiseowl_identity::IdentityId::from_bytes([0x61; 32]).unwrap();
        for new_boot in [false, true] {
            for boundary in BOUNDARIES {
                let root = tempfile::tempdir().unwrap();
                let dir = root.path().join("IDENTITY");
                std::fs::create_dir(&dir).unwrap();
                let first = start_local_activation(identity, &dir, [0x71; 16]).unwrap();
                finish_local_activation(identity, &dir).unwrap();
                let before = load_local(&dir.join("LOCAL")).unwrap().unwrap();
                let boot = if new_boot { [0x72; 16] } else { [0x71; 16] };
                assert!(start_local_activation_with_fault(identity, &dir, boot,
                    &mut fail_at(LocalTransition::BeginActivation, boundary)).is_err());
                let next = start_local_activation(identity, &dir, boot).unwrap();
                finish_local_activation(identity, &dir).unwrap();
                let after = load_local(&dir.join("LOCAL")).unwrap().unwrap();
                assert_eq!(after.identity_id, before.identity_id);
                assert_eq!(after.installation_id, before.installation_id);
                assert_eq!(after.activation_sequence, if new_boot { 2 } else { 1 });
                if new_boot { assert_ne!(next, first); } else { assert_eq!(next, first); }
            }
        }
    }

    #[test]
    fn dormant_start_and_recovering_to_active_cover_every_publication_boundary() {
        let identity = wiseowl_identity::IdentityId::from_bytes([0x81; 32]).unwrap();
        for boundary in BOUNDARIES {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("IDENTITY");
            std::fs::create_dir(&dir).unwrap();
            let first = start_local_activation(identity, &dir, [0x11; 16]).unwrap();
            finish_local_activation(identity, &dir).unwrap();
            let active = load_local(&dir.join("LOCAL")).unwrap().unwrap();
            let dormant = wiseowl_memorydb::activation::deactivate(active).unwrap();
            std::fs::write(dir.join("LOCAL"), dormant.encode()).unwrap();
            assert!(start_local_activation_with_fault(identity, &dir, [0x12; 16],
                &mut fail_at(LocalTransition::BeginActivation, boundary)).is_err());
            let second = start_local_activation(identity, &dir, [0x12; 16]).unwrap();
            assert_ne!(first, second);
            assert_eq!(finish_local_activation(identity, &dir).unwrap(), second);
            let after = load_local(&dir.join("LOCAL")).unwrap().unwrap();
            assert_eq!(after.installation_id, active.installation_id);
            assert_eq!(after.activation_sequence, 2);

            let recovering = start_local_activation(identity, &dir, [0x12; 16]).unwrap();
            assert_eq!(recovering, second);
            assert!(finish_local_activation_with_fault(identity, &dir,
                &mut fail_at(LocalTransition::FinishActivation, boundary)).is_err());
            assert_eq!(start_local_activation(identity, &dir, [0x12; 16]).unwrap(), second);
            assert_eq!(finish_local_activation(identity, &dir).unwrap(), second);
            let final_record = load_local(&dir.join("LOCAL")).unwrap().unwrap();
            assert_eq!(final_record.installation_id, active.installation_id);
            assert_eq!(final_record.activation_sequence, 2);
        }
    }

    fn resign(bytes: &mut [u8; 104]) {
        let hash = bytes[..96].iter().fold(0xcbf29ce484222325u64,
            |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3));
        bytes[96..104].copy_from_slice(&hash.to_le_bytes());
    }

    #[test]
    fn invalid_committed_local_and_conflicting_stages_suspend_without_global_mutation() {
        let identity = wiseowl_identity::IdentityId::from_bytes([0x31; 32]).unwrap();
        for case in 0..11 {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("IDENTITY");
            std::fs::create_dir(&dir).unwrap();
            for name in ["ROOT", "LINEAGE", "HEAD"] {
                std::fs::write(dir.join(name), name.as_bytes()).unwrap();
            }
            let _ = start_local_activation(identity, &dir, [0x41; 16]).unwrap();
            finish_local_activation(identity, &dir).unwrap();
            let before = std::fs::read(dir.join("LOCAL")).unwrap();
            let mut bytes: [u8; 104] = before.as_slice().try_into().unwrap();
            match case {
                0 => { bytes[8..40].fill(0x99); resign(&mut bytes); }
                1 => { bytes[40..56].fill(0); resign(&mut bytes); }
                2 => { bytes[56..72].fill(0); resign(&mut bytes); }
                3 => { bytes[6] = ActivationState::Dormant as u8; bytes[7] = 1; resign(&mut bytes); }
                4 => { bytes[56..72].fill(0); bytes[6] = ActivationState::Activating as u8; bytes[40..56].fill(0); resign(&mut bytes); }
                5 => { bytes[56..72].fill(0); bytes[6] = ActivationState::Active as u8; resign(&mut bytes); }
                6 => bytes[20] ^= 1,
                7 => { bytes[4] = 2; resign(&mut bytes); }
                8 => { bytes[56..72].fill(0); resign(&mut bytes); }
                9 => { bytes[72..80].fill(0); resign(&mut bytes); }
                _ => { bytes[80..96].fill(0); resign(&mut bytes); }
            }
            let replacement: &[u8] = if case == 8 { &bytes[..103] } else { &bytes };
            std::fs::write(dir.join("LOCAL"), replacement).unwrap();
            assert!(start_local_activation(identity, &dir, [0x41; 16]).is_err(), "case {case}");
            assert_eq!(std::fs::read(dir.join("LOCAL")).unwrap(), replacement);
            for name in ["ROOT", "LINEAGE", "HEAD"] {
                assert_eq!(std::fs::read(dir.join(name)).unwrap(), name.as_bytes());
            }
        }

        for case in 0..3 {
            let root = tempfile::tempdir().unwrap();
            let dir = root.path().join("IDENTITY");
            std::fs::create_dir(&dir).unwrap();
            let _ = start_local_activation(identity, &dir, [0x41; 16]).unwrap();
            finish_local_activation(identity, &dir).unwrap();
            let committed = std::fs::read(dir.join("LOCAL")).unwrap();
            let mut staged: [u8; 104] = committed.as_slice().try_into().unwrap();
            match case {
                0 => { staged[40..56].fill(0x99); resign(&mut staged); std::fs::write(dir.join("LOCAL.tmp"), staged).unwrap(); }
                1 => { staged[56..72].fill(0x99); resign(&mut staged); std::fs::write(dir.join("LOCAL.tmp"), staged).unwrap(); }
                _ => { std::fs::write(dir.join("LOCAL.tmp"), staged).unwrap(); std::fs::write(dir.join("LOCAL.other"), staged).unwrap(); }
            }
            assert!(start_local_activation(identity, &dir, [0x41; 16]).is_err(), "stage case {case}");
            assert_eq!(std::fs::read(dir.join("LOCAL")).unwrap(), committed);
        }

        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("IDENTITY");
        std::fs::create_dir(&dir).unwrap();
        let first = LocalActivationRecord {
            identity_id: identity,
            installation_id: InstallationId::from_bytes([0x11; 16]).unwrap(),
            state: ActivationState::Activating,
            activation_id: None,
            clean_stop: false,
            boot_epoch: [0x41; 16],
            activation_sequence: 1,
        };
        let mut second = first;
        second.installation_id = InstallationId::from_bytes([0x22; 16]).unwrap();
        std::fs::write(dir.join("LOCAL.tmp"), first.encode()).unwrap();
        std::fs::write(dir.join("LOCAL.other"), second.encode()).unwrap();
        assert!(start_local_activation(identity, &dir, [0x41; 16]).is_err());
        assert!(!dir.join("LOCAL").exists());

        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("IDENTITY");
        std::fs::create_dir(&dir).unwrap();
        let _ = start_local_activation(identity, &dir, [0x41; 16]).unwrap();
        finish_local_activation(identity, &dir).unwrap();
        let committed = std::fs::read(dir.join("LOCAL")).unwrap();
        std::fs::write(dir.join("LOCAL.tmp"), &committed).unwrap();
        start_local_activation(identity, &dir, [0x41; 16]).unwrap();
        assert!(!dir.join("LOCAL.tmp").exists());
    }

    #[test]
    fn phase_ab_identity_without_local_is_upgraded_without_global_rewrite() {
        let root = tempfile::tempdir().unwrap();
        let mut storage = HostIdentityStorage::open(root.path()).unwrap();
        let identity = ensure_identity(&mut storage, StoreDisposition::Fresh,
            AdoptionPolicy::Disabled, |bytes| { bytes.fill(0x42); Ok(()) }, |_| Ok(())).unwrap();
        let dir = root.path().join("IDENTITY");
        assert!(!dir.join("LOCAL").exists());
        let globals: Vec<_> = ["ROOT", "LINEAGE", "HEAD"].iter()
            .map(|name| std::fs::read(dir.join(name)).unwrap()).collect();
        let first = start_local_activation(identity.identity_id(), &dir, [0x21; 16]).unwrap();
        finish_local_activation(identity.identity_id(), &dir).unwrap();
        let first_local = load_local(&dir.join("LOCAL")).unwrap().unwrap();
        let second = start_local_activation(identity.identity_id(), &dir, [0x22; 16]).unwrap();
        finish_local_activation(identity.identity_id(), &dir).unwrap();
        let second_local = load_local(&dir.join("LOCAL")).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(first_local.installation_id, second_local.installation_id);
        assert_eq!(first_local.identity_id, second_local.identity_id);
        assert_eq!(identity.continuity_generation().get(), 1);
        assert_eq!(identity.lineage_sequence().get(), 1);
        for (name, before) in ["ROOT", "LINEAGE", "HEAD"].iter().zip(globals) {
            assert_eq!(std::fs::read(dir.join(name)).unwrap(), before);
        }
    }

    #[test]
    fn host_restart_retains_activation_and_new_boot_retains_installation() {
        let root = tempfile::tempdir().unwrap();
        let identity_dir = root.path().join("IDENTITY");
        std::fs::create_dir(&identity_dir).unwrap();
        let identity = wiseowl_identity::IdentityId::from_bytes([0x31; 32]).unwrap();
        let boot_a = [0x41; 16];
        let first = start_local_activation(identity, &identity_dir, boot_a).unwrap();
        assert_eq!(finish_local_activation(identity, &identity_dir).unwrap(), first);
        let first_record = load_local(&identity_dir.join("LOCAL")).unwrap().unwrap();

        let process_restart = start_local_activation(identity, &identity_dir, boot_a).unwrap();
        assert_eq!(process_restart, first);
        finish_local_activation(identity, &identity_dir).unwrap();

        let reboot = start_local_activation(identity, &identity_dir, [0x42; 16]).unwrap();
        assert_ne!(reboot, first);
        let reboot_record = load_local(&identity_dir.join("LOCAL")).unwrap().unwrap();
        assert_eq!(reboot_record.installation_id, first_record.installation_id);
        assert_eq!(reboot_record.identity_id, identity);
        assert_eq!(reboot_record.activation_sequence, 2);
        assert_eq!(wiseowl_identity::ContinuityGeneration::INITIAL.get(), 1);
    }

    #[test]
    fn recoverable_first_local_stage_reuses_installation_id() {
        let root = tempfile::tempdir().unwrap();
        let identity_dir = root.path().join("IDENTITY");
        std::fs::create_dir(&identity_dir).unwrap();
        let identity = wiseowl_identity::IdentityId::from_bytes([0x51; 32]).unwrap();
        let install = InstallationId::from_bytes([0x52; 16]).unwrap();
        let staged = LocalActivationRecord {
            identity_id: identity,
            installation_id: install,
            state: ActivationState::Activating,
            activation_id: None,
            clean_stop: false,
            boot_epoch: [0x53; 16],
            activation_sequence: 1,
        };
        std::fs::write(identity_dir.join("LOCAL.tmp"), staged.encode()).unwrap();
        let _ = start_local_activation(identity, &identity_dir, [0x54; 16]).unwrap();
        let recovered = load_local(&identity_dir.join("LOCAL")).unwrap().unwrap();
        assert_eq!(recovered.installation_id, install);
    }
}

fn handle_client(mut stream: UnixStream, db: Arc<Mutex<Database<FsStore>>>) -> io::Result<()> {
    let caller = DbCaller {
        caps: DbCapabilitySet::admin(),
        owner: 0,
        is_system: true,
    };

    loop {
        let req: DbRequest = match recv_msg(&mut stream)? {
            Some(r) => r,
            None => return Ok(()),
        };
        let response = {
            let mut guard = db.lock().expect("db mutex");
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(1);
            guard.set_now_ns(now.max(1));
            dispatch(&mut guard, &caller, req)
        };
        send_msg(&mut stream, &response)?;
    }
}

fn dispatch(db: &mut Database<FsStore>, caller: &DbCaller, req: DbRequest) -> DbResponse {
    match req {
        DbRequest::BeginTransaction => match db.begin_transaction(caller) {
            Ok(id) => DbResponse::TxId(id),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::InsertRecord { tx_id, req } => match req.into_request() {
            Ok(r) => match db.insert_record(caller, tx_id, r) {
                Ok(id) => DbResponse::MemoryId(id),
                Err(e) => DbResponse::from_error(e),
            },
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::InsertRelationship { tx_id, rel } => {
            match db.insert_relationship(caller, tx_id, rel) {
                Ok(()) => DbResponse::Ok,
                Err(e) => DbResponse::from_error(e),
            }
        }
        DbRequest::Tombstone { tx_id, id } => match db.tombstone_record(caller, tx_id, id) {
            Ok(()) => DbResponse::Ok,
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Commit { tx_id } => match db.commit_transaction(caller, tx_id) {
            Ok(seq) => DbResponse::Sequence(seq),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Abort { tx_id } => match db.abort_transaction(caller, tx_id) {
            Ok(()) => DbResponse::Ok,
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Get { id, payload } => match db.get_record(caller, id, payload) {
            Ok(r) => DbResponse::Record(r),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::History { id } => match db.list_revisions(caller, id) {
            Ok(r) => DbResponse::Revisions(r),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Source {
            source_id,
            offset,
            limit,
        } => match db.source_lookup(caller, source_id, offset as usize, limit as usize) {
            Ok((ids, more)) => DbResponse::SourcePage { ids, more },
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Relationships { id } => match db.get_relationships(caller, id) {
            Ok(r) => DbResponse::Relationships(r),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Query { query } => match db.query(caller, query) {
            Ok(r) => DbResponse::Query(r),
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::OwlQl { text } => match parse_owlql(&text) {
            Ok(q) => match db.query(caller, q) {
                Ok(r) => DbResponse::Query(r),
                Err(e) => DbResponse::from_error(e),
            },
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::DeleteSource { source_id, batch } => {
            match db.delete_source(caller, source_id, batch) {
                Ok((deleted, more)) => DbResponse::SourceDelete { deleted, more },
                Err(e) => DbResponse::from_error(e),
            }
        }
        DbRequest::DeleteSourceDryRun { source_id } => {
            match db.delete_source_dry_run(caller, source_id) {
                Ok(n) => DbResponse::SourceCount(n),
                Err(e) => DbResponse::from_error(e),
            }
        }
        DbRequest::Checkpoint => match db.create_checkpoint(caller) {
            Ok(()) => DbResponse::Ok,
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Compact => match db.run_compaction(caller) {
            Ok(reclaimed) => DbResponse::Compacted { reclaimed },
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::RebuildIndexes => match db.rebuild_indexes(caller) {
            Ok(()) => DbResponse::Ok,
            Err(e) => DbResponse::from_error(e),
        },
        DbRequest::Stats => DbResponse::Stats(db.stats()),
        DbRequest::Health => {
            let h = db.health();
            DbResponse::Health {
                ready: h.ready,
                state: match h.state {
                    HealthState::Starting => "starting".into(),
                    HealthState::Ready => "ready".into(),
                    HealthState::Degraded => "degraded".into(),
                    HealthState::Failed => "failed".into(),
                },
                reasons: h.reasons.clone(),
            }
        }
        DbRequest::GetIdentityStatus => db
            .identity_status()
            .and_then(|status| status.encode())
            .map(|bytes| {
                DbResponse::IdentityStatus(
                    wiseowl_memorydb::identity_status::IdentityStatusWire::new(bytes),
                )
            })
            .unwrap_or_else(|| DbResponse::Error {
                code: "IdentityUnavailable".into(),
                message: "validated identity context unavailable".into(),
            }),
        DbRequest::Verify { max_segments } => match db.verify_bounded(max_segments) {
            Ok((ok, bad)) => DbResponse::Verify { ok, bad },
            Err(e) => DbResponse::from_error(e),
        },
    }
}

fn recv_msg<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> io::Result<Option<T>> {
    let mut len_buf = [0u8; 4];
    match stream.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 4 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let msg =
        bincode::deserialize(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(msg))
}

fn send_msg<T: serde::Serialize>(stream: &mut UnixStream, msg: &T) -> io::Result<()> {
    let bytes =
        bincode::serialize(msg).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(&bytes)?;
    Ok(())
}
