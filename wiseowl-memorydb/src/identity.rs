//! MemoryDB-owned persistence and startup policy for Wise Owl identity.
//!
//! Identity files are adjacent to, but never embedded in, MemoryDB formats.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_identity::{
    validate_identity_set, ContinuityGeneration, EntropyError, GenesisEventKind, IdentityId,
    IdentityRoot, LineageEventId, LineageHead, LineageRecord, LineageSequence,
};

const COMMITTED_DIR: &str = "IDENTITY";
const STAGE_PREFIX: &str = "IDENTITY.STAGE";
const PRIMARY_STAGE: &str = "IDENTITY.STAGE";
const REJECTED_STAGE: &str = "IDENTITY.REJECTED";
const ROOT_FILE: &str = "ROOT";
const LINEAGE_FILE: &str = "LINEAGE";
const HEAD_FILE: &str = "HEAD";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreDisposition {
    Fresh,
    ExistingValidated,
    ExistingInvalidOrUncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectedStoreState {
    Fresh,
    Existing,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdoptionPolicy {
    Disabled,
    AllowValidatedExistingState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityStartupError {
    Io,
    EntropyUnavailable,
    CorruptCommittedIdentity,
    CorruptStagedIdentity,
    AmbiguousStagedIdentity,
    ExistingStateRequiresAdoption,
    ExistingStateInvalidOrUncertain,
    InjectedCrash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreationBoundary {
    BeforeIdGeneration,
    AfterIdGeneration,
    AfterRootWrite,
    AfterRootFlush,
    AfterLineageWrite,
    AfterLineageFlush,
    AfterHeadWrite,
    AfterHeadFlush,
    BeforeStagingDirectoryFlush,
    AfterStagingDirectoryFlush,
    BeforePublishRename,
    AfterPublishRename,
    BeforeParentDirectoryFlush,
    AfterParentDirectoryFlush,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadedIdentity {
    identity_id: IdentityId,
    lineage_sequence: LineageSequence,
    continuity_generation: ContinuityGeneration,
    genesis_event_kind: GenesisEventKind,
    recovered_from_staging: bool,
}

impl LoadedIdentity {
    pub const fn identity_id(self) -> IdentityId { self.identity_id }
    pub const fn lineage_sequence(self) -> LineageSequence { self.lineage_sequence }
    pub const fn continuity_generation(self) -> ContinuityGeneration { self.continuity_generation }
    pub const fn genesis_event_kind(self) -> GenesisEventKind { self.genesis_event_kind }
    pub const fn recovered_from_staging(self) -> bool { self.recovered_from_staging }
    pub fn diagnostic_fingerprint(self) -> [u8; 8] { self.identity_id.diagnostic_fingerprint() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityDirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Minimal filesystem contract required for crash-safe identity publication.
pub trait IdentityStorage {
    fn exists(&self, relative: &str) -> Result<bool, IdentityStartupError>;
    fn create_dir(&mut self, relative: &str) -> Result<(), IdentityStartupError>;
    fn list_dir(&self, relative: &str) -> Result<Vec<IdentityDirEntry>, IdentityStartupError>;
    fn read_file(&self, relative: &str) -> Result<Option<Vec<u8>>, IdentityStartupError>;
    fn write_file(&mut self, relative: &str, bytes: &[u8]) -> Result<(), IdentityStartupError>;
    fn sync_file(&mut self, relative: &str) -> Result<(), IdentityStartupError>;
    fn sync_dir(&mut self, relative: &str) -> Result<(), IdentityStartupError>;
    fn rename_no_replace(&mut self, old: &str, new: &str) -> Result<(), IdentityStartupError>;
}

/// Conservatively detect pre-identity durable state without relying on
/// MANIFEST alone. Unknown top-level names and stale temporary content are
/// uncertain, never fresh.
pub fn detect_store_state<S: IdentityStorage>(
    storage: &S,
) -> Result<DetectedStoreState, IdentityStartupError> {
    let entries = storage.list_dir("")?;
    let mut existing = false;
    for entry in entries {
        if entry.name == COMMITTED_DIR
            || entry.name.starts_with(STAGE_PREFIX)
            || entry.name == REJECTED_STAGE
        {
            continue;
        }
        if !entry.is_dir {
            if entry.name == "MANIFEST" {
                existing = true;
                continue;
            }
            return Ok(DetectedStoreState::Uncertain);
        }
        match entry.name.as_str() {
            "WAL" | "SEGMENTS" | "INDEX" | "SNAPSHOTS" | "ACTION_RECEIPTS"
            | "QUARANTINE" => {
                if !storage.list_dir(&entry.name)?.is_empty() {
                    existing = true;
                }
            }
            "TMP" => {
                if !storage.list_dir(&entry.name)?.is_empty() {
                    return Ok(DetectedStoreState::Uncertain);
                }
            }
            _ => return Ok(DetectedStoreState::Uncertain),
        }
    }
    Ok(if existing {
        DetectedStoreState::Existing
    } else {
        DetectedStoreState::Fresh
    })
}

fn path(directory: &str, file: &str) -> String {
    alloc::format!("{directory}/{file}")
}

fn load_set<S: IdentityStorage>(
    storage: &S,
    directory: &str,
) -> Result<LoadedIdentity, IdentityStartupError> {
    let root = storage
        .read_file(&path(directory, ROOT_FILE))?
        .ok_or(IdentityStartupError::CorruptStagedIdentity)
        .and_then(|bytes| {
            IdentityRoot::decode(&bytes).map_err(|_| IdentityStartupError::CorruptStagedIdentity)
        })?;
    let lineage = storage
        .read_file(&path(directory, LINEAGE_FILE))?
        .ok_or(IdentityStartupError::CorruptStagedIdentity)
        .and_then(|bytes| {
            LineageRecord::decode(&bytes)
                .map_err(|_| IdentityStartupError::CorruptStagedIdentity)
        })?;
    let head = storage
        .read_file(&path(directory, HEAD_FILE))?
        .ok_or(IdentityStartupError::CorruptStagedIdentity)
        .and_then(|bytes| {
            LineageHead::decode(&bytes).map_err(|_| IdentityStartupError::CorruptStagedIdentity)
        })?;
    let validated = validate_identity_set(&root, &lineage, &head)
        .map_err(|_| IdentityStartupError::CorruptStagedIdentity)?;
    Ok(LoadedIdentity {
        identity_id: validated.identity_id,
        lineage_sequence: validated.lineage_sequence,
        continuity_generation: validated.continuity_generation,
        genesis_event_kind: validated.genesis_event_kind,
        recovered_from_staging: false,
    })
}

fn event_kind(
    disposition: StoreDisposition,
    policy: AdoptionPolicy,
) -> Result<GenesisEventKind, IdentityStartupError> {
    match (disposition, policy) {
        (StoreDisposition::Fresh, _) => Ok(GenesisEventKind::Created),
        (StoreDisposition::ExistingValidated, AdoptionPolicy::AllowValidatedExistingState) => {
            Ok(GenesisEventKind::ExistingStateAdopted)
        }
        (StoreDisposition::ExistingValidated, AdoptionPolicy::Disabled) => {
            Err(IdentityStartupError::ExistingStateRequiresAdoption)
        }
        (StoreDisposition::ExistingInvalidOrUncertain, _) => {
            Err(IdentityStartupError::ExistingStateInvalidOrUncertain)
        }
    }
}

fn hit(
    hook: &mut impl FnMut(CreationBoundary) -> Result<(), IdentityStartupError>,
    boundary: CreationBoundary,
) -> Result<(), IdentityStartupError> {
    hook(boundary)
}

fn finish_stage<S: IdentityStorage>(
    storage: &mut S,
    stage: &str,
    hook: &mut impl FnMut(CreationBoundary) -> Result<(), IdentityStartupError>,
) -> Result<LoadedIdentity, IdentityStartupError> {
    let mut loaded = load_set(storage, stage)?;
    hit(hook, CreationBoundary::BeforeStagingDirectoryFlush)?;
    storage.sync_dir(stage)?;
    hit(hook, CreationBoundary::AfterStagingDirectoryFlush)?;
    hit(hook, CreationBoundary::BeforePublishRename)?;
    storage.rename_no_replace(stage, COMMITTED_DIR)?;
    hit(hook, CreationBoundary::AfterPublishRename)?;
    hit(hook, CreationBoundary::BeforeParentDirectoryFlush)?;
    storage.sync_dir("")?;
    hit(hook, CreationBoundary::AfterParentDirectoryFlush)?;
    loaded.recovered_from_staging = true;
    Ok(loaded)
}

fn complete_or_create_stage<S: IdentityStorage>(
    storage: &mut S,
    stage: &str,
    kind: GenesisEventKind,
    fill_entropy: &mut impl FnMut(&mut [u8]) -> Result<(), EntropyError>,
    hook: &mut impl FnMut(CreationBoundary) -> Result<(), IdentityStartupError>,
) -> Result<LoadedIdentity, IdentityStartupError> {
    let root_path = path(stage, ROOT_FILE);
    let lineage_path = path(stage, LINEAGE_FILE);
    let head_path = path(stage, HEAD_FILE);

    let root = match storage.read_file(&root_path)? {
        Some(bytes) => IdentityRoot::decode(&bytes)
            .map_err(|_| IdentityStartupError::CorruptStagedIdentity)?,
        None => {
            if storage.exists(&lineage_path)? || storage.exists(&head_path)? {
                return Err(IdentityStartupError::CorruptStagedIdentity);
            }
            hit(hook, CreationBoundary::BeforeIdGeneration)?;
            let identity_id = IdentityId::generate_with(&mut *fill_entropy)
                .map_err(|_| IdentityStartupError::EntropyUnavailable)?;
            hit(hook, CreationBoundary::AfterIdGeneration)?;
            let mut event_bytes = [0u8; 32];
            fill_entropy(&mut event_bytes)
                .map_err(|_| IdentityStartupError::EntropyUnavailable)?;
            let event_id = LineageEventId::from_bytes(event_bytes)
                .map_err(|_| IdentityStartupError::EntropyUnavailable)?;
            let root = IdentityRoot::new(identity_id, event_id, kind);
            storage.write_file(&root_path, &root.encode())?;
            hit(hook, CreationBoundary::AfterRootWrite)?;
            storage.sync_file(&root_path)?;
            hit(hook, CreationBoundary::AfterRootFlush)?;
            root
        }
    };

    if root.creation_event_kind() != kind {
        return Err(IdentityStartupError::CorruptStagedIdentity);
    }
    let lineage = match storage.read_file(&lineage_path)? {
        Some(bytes) => LineageRecord::decode(&bytes)
            .map_err(|_| IdentityStartupError::CorruptStagedIdentity)?,
        None => {
            let lineage = LineageRecord::genesis(
                root.identity_id(),
                root.creation_event_id(),
                root.creation_event_kind(),
            );
            storage.write_file(&lineage_path, &lineage.encode())?;
            hit(hook, CreationBoundary::AfterLineageWrite)?;
            storage.sync_file(&lineage_path)?;
            hit(hook, CreationBoundary::AfterLineageFlush)?;
            lineage
        }
    };
    let head = match storage.read_file(&head_path)? {
        Some(bytes) => LineageHead::decode(&bytes)
            .map_err(|_| IdentityStartupError::CorruptStagedIdentity)?,
        None => {
            let head = LineageHead::from_record(&lineage);
            storage.write_file(&head_path, &head.encode())?;
            hit(hook, CreationBoundary::AfterHeadWrite)?;
            storage.sync_file(&head_path)?;
            hit(hook, CreationBoundary::AfterHeadFlush)?;
            head
        }
    };
    validate_identity_set(&root, &lineage, &head)
        .map_err(|_| IdentityStartupError::CorruptStagedIdentity)?;
    finish_stage(storage, stage, hook)
}

/// Load the committed identity or create/finalize exactly one staged genesis.
pub fn ensure_identity<S: IdentityStorage>(
    storage: &mut S,
    disposition: StoreDisposition,
    adoption_policy: AdoptionPolicy,
    mut fill_entropy: impl FnMut(&mut [u8]) -> Result<(), EntropyError>,
    mut hook: impl FnMut(CreationBoundary) -> Result<(), IdentityStartupError>,
) -> Result<LoadedIdentity, IdentityStartupError> {
    if storage.exists(COMMITTED_DIR)? {
        return load_set(storage, COMMITTED_DIR).map_err(|_| {
            IdentityStartupError::CorruptCommittedIdentity
        });
    }

    let mut stages = storage
        .list_dir("")?
        .into_iter()
        .filter(|entry| entry.is_dir && entry.name.starts_with(STAGE_PREFIX))
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    stages.sort();
    if stages.len() > 1 {
        return Err(IdentityStartupError::AmbiguousStagedIdentity);
    }
    let stage = if let Some(stage) = stages.first() {
        stage.clone()
    } else {
        storage.create_dir(PRIMARY_STAGE)?;
        PRIMARY_STAGE.to_string()
    };
    // Once ROOT is durable it is the authoritative creation/adoption choice.
    // A later retry must not depend on a changed command-line policy.
    let kind = match storage.read_file(&path(&stage, ROOT_FILE))? {
        Some(bytes) => match IdentityRoot::decode(&bytes) {
            Ok(root) => root.creation_event_kind(),
            Err(_) => event_kind(disposition, adoption_policy)?,
        },
        None => event_kind(disposition, adoption_policy)?,
    };

    match complete_or_create_stage(
        storage,
        &stage,
        kind,
        &mut fill_entropy,
        &mut hook,
    ) {
        Ok(identity) => Ok(identity),
        Err(IdentityStartupError::CorruptStagedIdentity)
            if disposition == StoreDisposition::Fresh && stage == PRIMARY_STAGE =>
        {
            if storage.exists(REJECTED_STAGE)? {
                return Err(IdentityStartupError::AmbiguousStagedIdentity);
            }
            storage.rename_no_replace(&stage, REJECTED_STAGE)?;
            storage.sync_dir("")?;
            storage.create_dir(PRIMARY_STAGE)?;
            complete_or_create_stage(
                storage,
                PRIMARY_STAGE,
                kind,
                &mut fill_entropy,
                &mut hook,
            )
        }
        Err(error) => Err(error),
    }
}

#[cfg(feature = "host")]
pub mod host {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::{Read, Write};
    use std::path::{Path, PathBuf};

    pub struct HostIdentityStorage {
        root: PathBuf,
    }

    impl HostIdentityStorage {
        pub fn open(root: impl AsRef<Path>) -> Result<Self, IdentityStartupError> {
            fs::create_dir_all(root.as_ref()).map_err(|_| IdentityStartupError::Io)?;
            Ok(Self { root: root.as_ref().to_path_buf() })
        }

        fn path(&self, relative: &str) -> Result<PathBuf, IdentityStartupError> {
            if relative.split('/').any(|part| part == "..") {
                return Err(IdentityStartupError::Io);
            }
            Ok(if relative.is_empty() { self.root.clone() } else { self.root.join(relative) })
        }
    }

    impl IdentityStorage for HostIdentityStorage {
        fn exists(&self, relative: &str) -> Result<bool, IdentityStartupError> {
            Ok(self.path(relative)?.exists())
        }

        fn create_dir(&mut self, relative: &str) -> Result<(), IdentityStartupError> {
            fs::create_dir(self.path(relative)?).map_err(|_| IdentityStartupError::Io)
        }

        fn list_dir(&self, relative: &str) -> Result<Vec<IdentityDirEntry>, IdentityStartupError> {
            let mut entries = Vec::new();
            for entry in fs::read_dir(self.path(relative)?).map_err(|_| IdentityStartupError::Io)? {
                let entry = entry.map_err(|_| IdentityStartupError::Io)?;
                entries.push(IdentityDirEntry {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    is_dir: entry.file_type().map_err(|_| IdentityStartupError::Io)?.is_dir(),
                });
            }
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(entries)
        }

        fn read_file(&self, relative: &str) -> Result<Option<Vec<u8>>, IdentityStartupError> {
            let path = self.path(relative)?;
            if !path.exists() { return Ok(None); }
            let mut file = fs::File::open(path).map_err(|_| IdentityStartupError::Io)?;
            let mut bytes = Vec::new();
            file.take(4097).read_to_end(&mut bytes).map_err(|_| IdentityStartupError::Io)?;
            Ok(Some(bytes))
        }

        fn write_file(&mut self, relative: &str, bytes: &[u8]) -> Result<(), IdentityStartupError> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.path(relative)?)
                .map_err(|_| IdentityStartupError::Io)?;
            file.write_all(bytes).map_err(|_| IdentityStartupError::Io)
        }

        fn sync_file(&mut self, relative: &str) -> Result<(), IdentityStartupError> {
            fs::File::open(self.path(relative)?)
                .and_then(|file| file.sync_all())
                .map_err(|_| IdentityStartupError::Io)
        }

        fn sync_dir(&mut self, relative: &str) -> Result<(), IdentityStartupError> {
            fs::File::open(self.path(relative)?)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| IdentityStartupError::Io)
        }

        fn rename_no_replace(&mut self, old: &str, new: &str) -> Result<(), IdentityStartupError> {
            let destination = self.path(new)?;
            if destination.exists() { return Err(IdentityStartupError::Io); }
            fs::rename(self.path(old)?, destination).map_err(|_| IdentityStartupError::Io)
        }
    }
}

#[cfg(all(test, feature = "host"))]
mod tests {
    use super::host::HostIdentityStorage;
    use super::*;
    use std::fs;
    use std::path::Path;

    fn entropy(start: u8) -> impl FnMut(&mut [u8]) -> Result<(), EntropyError> {
        let mut value = start.max(1);
        move |bytes| {
            bytes.fill(value);
            value = value.wrapping_add(1).max(1);
            Ok(())
        }
    }

    fn no_fault(_: CreationBoundary) -> Result<(), IdentityStartupError> {
        Ok(())
    }

    fn boot(root: &Path, seed: u8) -> LoadedIdentity {
        let mut storage = HostIdentityStorage::open(root).unwrap();
        ensure_identity(
            &mut storage,
            StoreDisposition::Fresh,
            AdoptionPolicy::Disabled,
            entropy(seed),
            no_fault,
        )
        .unwrap()
    }

    #[test]
    fn restart_and_reboot_keep_one_identity_and_genesis() {
        let temp = tempfile::tempdir().unwrap();
        let first = boot(temp.path(), 1);
        let second = boot(temp.path(), 80);
        let third = boot(temp.path(), 160);
        assert_eq!(first.identity_id(), second.identity_id());
        assert_eq!(second.identity_id(), third.identity_id());
        assert_eq!(third.lineage_sequence().get(), 1);
        assert_eq!(third.continuity_generation().get(), 1);
        assert_eq!(third.genesis_event_kind(), GenesisEventKind::Created);
    }

    #[test]
    fn every_creation_boundary_converges_without_replacing_recoverable_id() {
        let boundaries = [
            CreationBoundary::BeforeIdGeneration,
            CreationBoundary::AfterIdGeneration,
            CreationBoundary::AfterRootWrite,
            CreationBoundary::AfterRootFlush,
            CreationBoundary::AfterLineageWrite,
            CreationBoundary::AfterLineageFlush,
            CreationBoundary::AfterHeadWrite,
            CreationBoundary::AfterHeadFlush,
            CreationBoundary::BeforeStagingDirectoryFlush,
            CreationBoundary::AfterStagingDirectoryFlush,
            CreationBoundary::BeforePublishRename,
            CreationBoundary::AfterPublishRename,
            CreationBoundary::BeforeParentDirectoryFlush,
            CreationBoundary::AfterParentDirectoryFlush,
        ];

        for boundary in boundaries {
            let temp = tempfile::tempdir().unwrap();
            let mut storage = HostIdentityStorage::open(temp.path()).unwrap();
            let mut fired = false;
            let result = ensure_identity(
                &mut storage,
                StoreDisposition::Fresh,
                AdoptionPolicy::Disabled,
                entropy(1),
                |at| {
                    if at == boundary && !fired {
                        fired = true;
                        Err(IdentityStartupError::InjectedCrash)
                    } else {
                        Ok(())
                    }
                },
            );
            assert_eq!(result, Err(IdentityStartupError::InjectedCrash), "{boundary:?}");

            let durable_root = ["IDENTITY/ROOT", "IDENTITY.STAGE/ROOT"]
                .iter()
                .find_map(|relative| fs::read(temp.path().join(relative)).ok())
                .and_then(|bytes| IdentityRoot::decode(&bytes).ok())
                .map(IdentityRoot::identity_id);

            let recovered = boot(temp.path(), 90);
            if let Some(durable_id) = durable_root {
                assert_eq!(recovered.identity_id(), durable_id, "{boundary:?}");
            }
            let again = boot(temp.path(), 170);
            assert_eq!(again.identity_id(), recovered.identity_id(), "{boundary:?}");
            assert_eq!(again.lineage_sequence().get(), 1);
            assert_eq!(again.continuity_generation().get(), 1);
        }
    }

    #[test]
    fn adoption_is_explicit_and_does_not_rewrite_existing_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let manifest = b"pre-identity-memorydb-manifest";
        fs::write(temp.path().join("MANIFEST"), manifest).unwrap();

        let mut denied = HostIdentityStorage::open(temp.path()).unwrap();
        assert_eq!(
            ensure_identity(
                &mut denied,
                StoreDisposition::ExistingValidated,
                AdoptionPolicy::Disabled,
                entropy(1),
                no_fault,
            ),
            Err(IdentityStartupError::ExistingStateRequiresAdoption)
        );
        assert!(!temp.path().join("IDENTITY").exists());

        let mut allowed = HostIdentityStorage::open(temp.path()).unwrap();
        let adopted = ensure_identity(
            &mut allowed,
            StoreDisposition::ExistingValidated,
            AdoptionPolicy::AllowValidatedExistingState,
            entropy(3),
            no_fault,
        )
        .unwrap();
        assert_eq!(adopted.genesis_event_kind(), GenesisEventKind::ExistingStateAdopted);
        assert_eq!(fs::read(temp.path().join("MANIFEST")).unwrap(), manifest);

        let mut restart = HostIdentityStorage::open(temp.path()).unwrap();
        let same = ensure_identity(
            &mut restart,
            StoreDisposition::ExistingInvalidOrUncertain,
            AdoptionPolicy::Disabled,
            entropy(99),
            no_fault,
        )
        .unwrap();
        assert_eq!(same.identity_id(), adopted.identity_id());
    }

    fn copy_identity(source: &Path, destination: &Path) {
        fs::create_dir(destination).unwrap();
        for file in [ROOT_FILE, LINEAGE_FILE, HEAD_FILE] {
            fs::copy(source.join(file), destination.join(file)).unwrap();
        }
    }

    #[test]
    fn multiple_valid_candidates_fail_closed_without_third_identity() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        boot(first.path(), 1);
        boot(second.path(), 9);

        let target = tempfile::tempdir().unwrap();
        copy_identity(
            &first.path().join(COMMITTED_DIR),
            &target.path().join("IDENTITY.STAGE.A"),
        );
        copy_identity(
            &second.path().join(COMMITTED_DIR),
            &target.path().join("IDENTITY.STAGE.B"),
        );
        let before = fs::read_dir(target.path()).unwrap().count();
        let mut storage = HostIdentityStorage::open(target.path()).unwrap();
        assert_eq!(
            ensure_identity(
                &mut storage,
                StoreDisposition::Fresh,
                AdoptionPolicy::Disabled,
                entropy(50),
                no_fault,
            ),
            Err(IdentityStartupError::AmbiguousStagedIdentity)
        );
        assert_eq!(fs::read_dir(target.path()).unwrap().count(), before);
        assert!(!target.path().join(COMMITTED_DIR).exists());
    }

    #[test]
    fn committed_corruption_and_missing_lineage_never_create_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let original = boot(temp.path(), 7);
        fs::remove_file(temp.path().join("IDENTITY/LINEAGE")).unwrap();
        let mut storage = HostIdentityStorage::open(temp.path()).unwrap();
        assert_eq!(
            ensure_identity(
                &mut storage,
                StoreDisposition::Fresh,
                AdoptionPolicy::Disabled,
                entropy(99),
                no_fault,
            ),
            Err(IdentityStartupError::CorruptCommittedIdentity)
        );
        assert!(!temp.path().join(PRIMARY_STAGE).exists());
        let root = IdentityRoot::decode(&fs::read(temp.path().join("IDENTITY/ROOT")).unwrap()).unwrap();
        assert_eq!(root.identity_id(), original.identity_id());
    }
}
