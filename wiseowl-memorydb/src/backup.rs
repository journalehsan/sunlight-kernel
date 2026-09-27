//! Host-only, passive Wise Owl package. Creation is deliberately an internal API:
//! the current host socket gives every peer admin capabilities and cannot safely
//! authorize an export of personal data.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use wiseowl_identity::{
    validate_identity_set, IdentityId, IdentityRoot, LineageHead, LineageRecord,
};

use crate::database::{validate_store_read_only, Database, DbCaller, FsStore};
use crate::quotas::DbQuotaConfig;

const MAGIC: &[u8; 8] = b"WOBKUP01";
// V1 was a host prototype without the required Wise Owl KV component.
// It must never be accepted as a complete continuity package.
const FORMAT: u16 = 2;
const COMMIT_MAGIC: &[u8; 8] = b"WOBSEAL2";
const COMMIT_LEN: usize = 8 + 2 + 1 + 16 + 32;
const MAX_COMPONENTS: usize = 256;
const MAX_PATH: usize = 96;
const MAX_MANIFEST: usize = 40_000;
const MAX_TOTAL: u64 = 1 << 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PackagePurpose {
    Backup = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ComponentKind {
    IdentityRoot = 1,
    IdentityLineage = 2,
    IdentityHead = 3,
    MemoryDbManifest = 4,
    MemoryDbWal = 5,
    MemoryDbSegment = 6,
    MemoryDbRelationshipSnapshot = 7,
    PersonalityKvExport = 8,
    IndexState = 9,
    ActionReceipts = 10,
    PassiveGuard = 11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ComponentRole {
    IdentityAuthority = 1,
    MemoryDbAuthority = 2,
    PassiveGuard = 3,
    OptionalWiseOwlState = 4,
    Rebuildable = 5,
    WiseOwlAuthority = 6,
}

impl ComponentRole {
    fn decode(value: u8) -> io::Result<Self> {
        Ok(match value {
            1 => Self::IdentityAuthority,
            2 => Self::MemoryDbAuthority,
            3 => Self::PassiveGuard,
            4 => Self::OptionalWiseOwlState,
            5 => Self::Rebuildable,
            6 => Self::WiseOwlAuthority,
            _ => return Err(invalid("component role")),
        })
    }
}

fn role_for(kind: ComponentKind) -> ComponentRole {
    match kind {
        ComponentKind::IdentityRoot
        | ComponentKind::IdentityLineage
        | ComponentKind::IdentityHead => ComponentRole::IdentityAuthority,
        ComponentKind::MemoryDbManifest
        | ComponentKind::MemoryDbWal
        | ComponentKind::MemoryDbSegment
        | ComponentKind::MemoryDbRelationshipSnapshot => ComponentRole::MemoryDbAuthority,
        ComponentKind::PassiveGuard => ComponentRole::PassiveGuard,
        ComponentKind::PersonalityKvExport => ComponentRole::WiseOwlAuthority,
        ComponentKind::ActionReceipts => ComponentRole::OptionalWiseOwlState,
        ComponentKind::IndexState => ComponentRole::Rebuildable,
    }
}

impl ComponentKind {
    fn decode(v: u8) -> io::Result<Self> {
        Ok(match v {
            1 => Self::IdentityRoot,
            2 => Self::IdentityLineage,
            3 => Self::IdentityHead,
            4 => Self::MemoryDbManifest,
            5 => Self::MemoryDbWal,
            6 => Self::MemoryDbSegment,
            7 => Self::MemoryDbRelationshipSnapshot,
            8 => Self::PersonalityKvExport,
            9 => Self::IndexState,
            10 => Self::ActionReceipts,
            11 => Self::PassiveGuard,
            _ => return Err(invalid("component kind")),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackupId(pub [u8; 16]);

impl BackupId {
    pub fn generate() -> io::Result<Self> {
        let mut bytes = [0; 16];
        wiseowl_identity::fill_host_entropy(&mut bytes).map_err(|_| invalid("entropy"))?;
        Self::new(bytes)
    }
    pub fn new(bytes: [u8; 16]) -> io::Result<Self> {
        if bytes == [0; 16] {
            return Err(invalid("zero backup id"));
        }
        Ok(Self(bytes))
    }
    pub fn fingerprint(self) -> String {
        hex(&self.0[..4])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentSnapshot {
    pub kind: ComponentKind,
    pub role: ComponentRole,
    pub path: String,
    pub required: bool,
    pub present: bool,
    pub schema: u16,
    pub size: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackupManifest {
    pub purpose: PackagePurpose,
    pub activation_allowed: bool,
    pub backup_id: BackupId,
    pub identity_id: IdentityId,
    pub lineage_sequence: u64,
    pub continuity_generation: u64,
    pub head_digest: [u8; 32],
    pub identity_format_version: u16,
    pub components: Vec<ComponentSnapshot>,
}

#[derive(Clone, Debug)]
pub struct BackupInspection {
    pub manifest: BackupManifest,
    pub total_bytes: u64,
    pub storage_independent: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackageInspectionStatus {
    ValidCommittedBackup,
    ValidButUncommittedPackage,
    CorruptPackage,
    UnsupportedPackage,
}

/// Classify package bytes without opening the active store. This reports the
/// obsolete V1 prototype as unsupported, even if its own hashes are intact.
pub fn inspect_package_status(package: &Path) -> PackageInspectionStatus {
    let manifest = match fs::read(package.join("BACKUP.MANIFEST")) {
        Ok(bytes) => bytes,
        Err(_) => return PackageInspectionStatus::CorruptPackage,
    };
    if manifest.len() >= 10 && &manifest[..8] == MAGIC {
        let version = u16::from_le_bytes([manifest[8], manifest[9]]);
        if version != FORMAT {
            return PackageInspectionStatus::UnsupportedPackage;
        }
    }
    if inspect_components(package).is_err() {
        return PackageInspectionStatus::CorruptPackage;
    }
    if !package.join("COMMITTED").exists() {
        return PackageInspectionStatus::ValidButUncommittedPackage;
    }
    if inspect_backup(package).is_ok() {
        PackageInspectionStatus::ValidCommittedBackup
    } else {
        PackageInspectionStatus::CorruptPackage
    }
}

#[derive(Clone, Debug)]
pub struct BackupResult {
    pub path: PathBuf,
    pub backup_id: BackupId,
    pub quiesce: Duration,
    pub checkpoint: Duration,
    pub component_copy: Duration,
    pub sha256: Duration,
    pub total: Duration,
    pub hashing: Duration,
    pub bytes: u64,
    /// Same filesystem is definitely not independent; a different filesystem
    /// does not prove a different physical disk.
    pub storage_independent: Option<bool>,
}

#[derive(Default)]
struct CopyMetrics {
    copy: Duration,
    sha256: Duration,
}

/// A trusted coordinator must stop new Wise Owl durable operations and drain
/// in-flight operations before `begin` returns. `end` must be idempotent and
/// must always thaw them, including after a partially failed `begin`.
/// The enumerated export must be captured while the barrier is held.
pub trait WiseOwlBackupBarrier {
    fn begin(&mut self) -> io::Result<()>;
    fn export_kv(&mut self) -> io::Result<Vec<(String, Vec<u8>)>>;
    fn end(&mut self);
}

struct FrozenBarrier<'a>(&'a mut dyn WiseOwlBackupBarrier);

impl<'a> FrozenBarrier<'a> {
    fn acquire(barrier: &'a mut dyn WiseOwlBackupBarrier) -> io::Result<Self> {
        if let Err(error) = barrier.begin() {
            barrier.end();
            return Err(error);
        }
        Ok(Self(barrier))
    }

    fn export_kv(&mut self) -> io::Result<Vec<(String, Vec<u8>)>> {
        self.0.export_kv()
    }
}

impl Drop for FrozenBarrier<'_> {
    fn drop(&mut self) {
        self.0.end();
    }
}

const KV_MAGIC: &[u8; 8] = b"WOKVSNP2";
const KV_MAX_ENTRIES: usize = 4096;
const KV_MAX_VALUE: usize = 64;
const KV_MAX_BYTES: usize =
    8 + 2 + 1 + 16 + 32 + 2 + KV_MAX_ENTRIES * (4 + 1 + 2 + 2 + KV_MAX_VALUE);
// Explicitly records that historical keys used the low 32 bits of a UID.
const KV_OWNER_LEGACY_U32: u8 = 1;

fn kv_kind(code: &str) -> Option<u8> {
    Some(match code {
        "vc" => 1,
        "gen" => 2,
        "lp" => 3,
        "gs" => 4,
        "sms" => 5,
        "sis" => 6,
        _ => return None,
    })
}

fn parse_kv_key(key: &str) -> io::Result<(u32, u8)> {
    let rest = key
        .strip_prefix("wb1:")
        .ok_or_else(|| invalid("KV namespace"))?;
    let (owner, code) = rest
        .split_once(':')
        .ok_or_else(|| invalid("KV key layout"))?;
    if owner.is_empty()
        || owner.len() > 8
        || !owner
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("KV owner"));
    }
    let owner_id = u32::from_str_radix(owner, 16).map_err(|_| invalid("KV owner"))?;
    if format!("{owner_id:x}") != owner {
        return Err(invalid("noncanonical KV owner"));
    }
    Ok((
        owner_id,
        kv_kind(code).ok_or_else(|| invalid("unknown Wise Owl KV key"))?,
    ))
}

fn encode_kv_snapshot(
    backup_id: BackupId,
    identity_id: IdentityId,
    raw: Vec<(String, Vec<u8>)>,
) -> io::Result<Vec<u8>> {
    if raw.len() > KV_MAX_ENTRIES {
        return Err(invalid("KV entry count"));
    }
    let mut entries = Vec::with_capacity(raw.len());
    let mut seen = BTreeSet::new();
    for (key, value) in raw {
        let (owner, kind) = parse_kv_key(&key)?;
        if !seen.insert((owner, kind)) || value.len() > KV_MAX_VALUE {
            return Err(invalid("duplicate/oversized KV entry"));
        }
        entries.push((owner, kind, value));
    }
    entries.sort_by_key(|(owner, kind, _)| (*owner, *kind));
    let mut out = Vec::new();
    out.extend_from_slice(KV_MAGIC);
    out.extend_from_slice(&1u16.to_le_bytes());
    out.push(KV_OWNER_LEGACY_U32);
    out.extend_from_slice(&backup_id.0);
    out.extend_from_slice(identity_id.as_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (owner, kind, value) in entries {
        out.extend_from_slice(&owner.to_le_bytes());
        out.push(kind);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(&value);
    }
    Ok(out)
}

fn validate_kv_snapshot(
    bytes: &[u8],
    backup_id: BackupId,
    identity_id: IdentityId,
) -> io::Result<()> {
    if bytes.len() < 61
        || bytes.len() > KV_MAX_BYTES
        || &bytes[..8] != KV_MAGIC
        || u16::from_le_bytes(bytes[8..10].try_into().unwrap()) != 1
        || bytes[10] != KV_OWNER_LEGACY_U32
        || bytes[11..27] != backup_id.0
        || bytes[27..59] != *identity_id.as_bytes()
    {
        return Err(invalid("KV snapshot header"));
    }
    let count = u16::from_le_bytes(bytes[59..61].try_into().unwrap()) as usize;
    if count > KV_MAX_ENTRIES {
        return Err(invalid("KV snapshot count"));
    }
    let mut pos = 61;
    let mut last = None;
    for _ in 0..count {
        if pos + 9 > bytes.len() {
            return Err(invalid("truncated KV entry"));
        }
        let owner = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap());
        let kind = bytes[pos + 4];
        let value_schema = u16::from_le_bytes(bytes[pos + 5..pos + 7].try_into().unwrap());
        let len = u16::from_le_bytes(bytes[pos + 7..pos + 9].try_into().unwrap()) as usize;
        pos += 9;
        if !(1..=6).contains(&kind)
            || value_schema != 1
            || len > KV_MAX_VALUE
            || pos + len > bytes.len()
            || last.is_some_and(|previous| (owner, kind) <= previous)
        {
            return Err(invalid("KV entry"));
        }
        last = Some((owner, kind));
        pos += len;
    }
    if pos != bytes.len() {
        return Err(invalid("trailing KV bytes"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackupBoundary {
    BeforeStageCreate,
    AfterStageCreate,
    DuringFirstComponent,
    AfterFirstComponentWrite,
    AfterFirstComponentSync,
    BeforeMemoryDbExport,
    DuringKvSnapshot,
    AfterKvSnapshotSync,
    BeforeManifestWrite,
    AfterManifestWrite,
    AfterManifestSync,
    BeforeStageSync,
    AfterStageSync,
    BeforePublish,
    AfterPublish,
    BeforeCommitMarker,
    AfterCommitMarkerWrite,
    AfterCommitMarkerSync,
    BeforeFinalDirSync,
    BeforeParentSync,
    AfterParentSync,
    AfterFinalDirSync,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn io_other(message: &'static str) -> io::Error {
    io::Error::other(message)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn expected_path(kind: ComponentKind, path: &str) -> bool {
    match kind {
        ComponentKind::IdentityRoot => path == "IDENTITY/ROOT",
        ComponentKind::IdentityLineage => path == "IDENTITY/LINEAGE",
        ComponentKind::IdentityHead => path == "IDENTITY/HEAD",
        ComponentKind::MemoryDbManifest => path == "MEMORYDB/MANIFEST",
        ComponentKind::MemoryDbWal => path == "MEMORYDB/WAL/wal-000001",
        ComponentKind::MemoryDbRelationshipSnapshot => path == "MEMORYDB/INDEX/relationships.bin",
        ComponentKind::MemoryDbSegment => path
            .strip_prefix("MEMORYDB/SEGMENTS/data-")
            .and_then(|s| s.strip_suffix(".owlseg"))
            .is_some_and(|s| (6..=20).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())),
        ComponentKind::PersonalityKvExport => path == "KV/wiseowl-mtm.bin",
        ComponentKind::IndexState => path == "INDEX/state.bin",
        ComponentKind::ActionReceipts => path == "RECEIPTS/sealed.bin",
        ComponentKind::PassiveGuard => path == "MEMORYDB/BACKUP.PASSIVE",
    }
}

fn check_component(c: &ComponentSnapshot) -> io::Result<()> {
    if c.role != role_for(c.kind) {
        return Err(invalid("component role mismatch"));
    }
    if c.path.is_empty()
        || c.path.len() > MAX_PATH
        || !expected_path(c.kind, &c.path)
        || Path::new(&c.path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || c.path
            .split('/')
            .any(|part| part == "." || part == ".." || part.is_empty())
    {
        return Err(invalid("component path"));
    }
    if c.required && !c.present {
        return Err(invalid("required component absent"));
    }
    if !c.present && (c.size != 0 || c.sha256 != [0; 32]) {
        return Err(invalid("absent component data"));
    }
    if c.schema != 1 {
        return Err(invalid("component schema"));
    }
    if c.kind == ComponentKind::MemoryDbSegment && (!c.required || !c.present) {
        return Err(invalid("segment classification"));
    }
    Ok(())
}

fn check_manifest(m: &BackupManifest) -> io::Result<()> {
    if m.purpose != PackagePurpose::Backup
        || m.activation_allowed
        || m.backup_id.0 == [0; 16]
        || m.identity_format_version != wiseowl_identity::IDENTITY_FORMAT_VERSION
        || m.lineage_sequence == 0
        || m.continuity_generation == 0
        || m.components.len() < 7
        || m.components.len() > MAX_COMPONENTS
    {
        return Err(invalid("manifest header"));
    }
    let mut paths = BTreeSet::new();
    let mut singletons = BTreeSet::new();
    let mut segment_ids = BTreeSet::new();
    let mut total = 0u64;
    for c in &m.components {
        check_component(c)?;
        if !paths.insert(c.path.as_str()) {
            return Err(invalid("duplicate component path"));
        }
        if c.kind != ComponentKind::MemoryDbSegment && !singletons.insert(c.kind as u8) {
            return Err(invalid("duplicate component kind"));
        }
        if c.kind == ComponentKind::MemoryDbSegment {
            let number = c
                .path
                .strip_prefix("MEMORYDB/SEGMENTS/data-")
                .and_then(|s| s.strip_suffix(".owlseg"))
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| invalid("segment number"))?;
            if number == 0 || !segment_ids.insert(number) {
                return Err(invalid("duplicate segment number"));
            }
        }
        total = total
            .checked_add(c.size)
            .ok_or_else(|| invalid("size overflow"))?;
        if total > MAX_TOTAL {
            return Err(invalid("package too large"));
        }
    }
    for kind in [
        ComponentKind::IdentityRoot,
        ComponentKind::IdentityLineage,
        ComponentKind::IdentityHead,
        ComponentKind::MemoryDbManifest,
        ComponentKind::MemoryDbWal,
        ComponentKind::MemoryDbRelationshipSnapshot,
        ComponentKind::PersonalityKvExport,
        ComponentKind::PassiveGuard,
    ] {
        if !m
            .components
            .iter()
            .any(|c| c.kind == kind && c.required && c.present)
        {
            return Err(invalid("missing required kind"));
        }
    }
    for kind in [ComponentKind::IndexState, ComponentKind::ActionReceipts] {
        if !m.components.iter().any(|c| c.kind == kind && !c.required) {
            return Err(invalid("missing optional declaration"));
        }
    }
    Ok(())
}

impl BackupManifest {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        check_manifest(self)?;
        let mut b = Vec::new();
        b.extend_from_slice(MAGIC);
        b.extend_from_slice(&FORMAT.to_le_bytes());
        b.push(self.purpose as u8);
        b.push(u8::from(self.activation_allowed));
        b.extend_from_slice(&self.backup_id.0);
        b.extend_from_slice(self.identity_id.as_bytes());
        b.extend_from_slice(&self.lineage_sequence.to_le_bytes());
        b.extend_from_slice(&self.continuity_generation.to_le_bytes());
        b.extend_from_slice(&self.head_digest);
        b.extend_from_slice(&self.identity_format_version.to_le_bytes());
        b.extend_from_slice(&(self.components.len() as u16).to_le_bytes());
        for c in &self.components {
            b.extend_from_slice(&[
                c.kind as u8,
                c.role as u8,
                u8::from(c.required),
                u8::from(c.present),
                c.path.len() as u8,
            ]);
            b.extend_from_slice(&c.schema.to_le_bytes());
            b.extend_from_slice(&c.size.to_le_bytes());
            b.extend_from_slice(&c.sha256);
            b.extend_from_slice(c.path.as_bytes());
        }
        if b.len() + 32 > MAX_MANIFEST {
            return Err(invalid("manifest too large"));
        }
        let digest = Sha256::digest(&b);
        b.extend_from_slice(&digest);
        Ok(b)
    }

    pub fn decode(b: &[u8]) -> io::Result<Self> {
        if b.len() < 144 || b.len() > MAX_MANIFEST || &b[..8] != MAGIC {
            return Err(invalid("manifest magic/length"));
        }
        let (body, checksum) = b.split_at(b.len() - 32);
        if Sha256::digest(body).as_slice() != checksum {
            return Err(invalid("manifest SHA-256"));
        }
        let mut p = 8;
        fn take<'a>(b: &'a [u8], p: &mut usize, n: usize) -> io::Result<&'a [u8]> {
            let end = p.checked_add(n).ok_or_else(|| invalid("manifest length"))?;
            if end > b.len() {
                return Err(invalid("truncated manifest"));
            }
            let out = &b[*p..end];
            *p = end;
            Ok(out)
        }
        fn u16_at(b: &[u8], p: &mut usize) -> io::Result<u16> {
            Ok(u16::from_le_bytes(take(b, p, 2)?.try_into().unwrap()))
        }
        fn u64_at(b: &[u8], p: &mut usize) -> io::Result<u64> {
            Ok(u64::from_le_bytes(take(b, p, 8)?.try_into().unwrap()))
        }
        if u16_at(body, &mut p)? != FORMAT {
            return Err(invalid("backup version"));
        }
        if take(body, &mut p, 1)?[0] != PackagePurpose::Backup as u8 {
            return Err(invalid("package purpose"));
        }
        if take(body, &mut p, 1)?[0] != 0 {
            return Err(invalid("activation allowed"));
        }
        let backup_id = BackupId::new(take(body, &mut p, 16)?.try_into().unwrap())?;
        let identity_id =
            IdentityId::decode(take(body, &mut p, 32)?).map_err(|_| invalid("identity id"))?;
        let lineage_sequence = u64_at(body, &mut p)?;
        let continuity_generation = u64_at(body, &mut p)?;
        let head_digest = take(body, &mut p, 32)?.try_into().unwrap();
        let identity_format_version = u16_at(body, &mut p)?;
        let count = u16_at(body, &mut p)? as usize;
        if count > MAX_COMPONENTS {
            return Err(invalid("component count"));
        }
        let mut components = Vec::with_capacity(count);
        for _ in 0..count {
            let kind = ComponentKind::decode(take(body, &mut p, 1)?[0])?;
            let role = ComponentRole::decode(take(body, &mut p, 1)?[0])?;
            let required = match take(body, &mut p, 1)?[0] {
                0 => false,
                1 => true,
                _ => return Err(invalid("required flag")),
            };
            let present = match take(body, &mut p, 1)?[0] {
                0 => false,
                1 => true,
                _ => return Err(invalid("present flag")),
            };
            let path_len = take(body, &mut p, 1)?[0] as usize;
            let schema = u16_at(body, &mut p)?;
            let size = u64_at(body, &mut p)?;
            let sha256 = take(body, &mut p, 32)?.try_into().unwrap();
            let path = std::str::from_utf8(take(body, &mut p, path_len)?)
                .map_err(|_| invalid("path UTF-8"))?
                .to_owned();
            components.push(ComponentSnapshot {
                kind,
                role,
                path,
                required,
                present,
                schema,
                size,
                sha256,
            });
        }
        if p != body.len() {
            return Err(invalid("trailing manifest data"));
        }
        let m = Self {
            purpose: PackagePurpose::Backup,
            activation_allowed: false,
            backup_id,
            identity_id,
            lineage_sequence,
            continuity_generation,
            head_digest,
            identity_format_version,
            components,
        };
        check_manifest(&m)?;
        Ok(m)
    }
}

fn hash_file(path: &Path) -> io::Result<(u64, [u8; 32])> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .ok_or_else(|| invalid("size overflow"))?;
        if size > MAX_TOTAL {
            return Err(invalid("component too large"));
        }
        hasher.update(&buf[..n]);
    }
    Ok((size, hasher.finalize().into()))
}

fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(target_os = "linux")]
fn package_lock(directory: &File, shared: bool) -> io::Result<()> {
    let operation = if shared { libc::LOCK_SH } else { libc::LOCK_EX };
    if unsafe { libc::flock(directory.as_raw_fd(), operation) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
fn package_lock(_directory: &File, _shared: bool) -> io::Result<()> {
    Err(io_other("package lock unavailable"))
}

struct UncommittedSeal {
    package: PathBuf,
    committed: bool,
}

impl Drop for UncommittedSeal {
    fn drop(&mut self) {
        if !self.committed {
            // An ordinary failure must not leave a seal that can be inspected.
            // A process/VM crash bypasses this cleanup and has a weaker model.
            let _ = fs::remove_file(self.package.join("COMMITTED"));
            let _ = File::open(&self.package).and_then(|dir| dir.sync_all());
        }
    }
}

fn private_component_dirs(stage: &Path, parent: &Path) -> io::Result<()> {
    let relative = parent
        .strip_prefix(stage)
        .map_err(|_| invalid("component parent"))?;
    let mut path = stage.to_path_buf();
    for part in relative.components() {
        if !matches!(part, Component::Normal(_)) {
            return Err(invalid("component directory"));
        }
        path.push(part);
        if !path.exists() {
            fs::create_dir(&path)?;
        }
        if !fs::symlink_metadata(&path)?.file_type().is_dir() {
            return Err(invalid("component directory type"));
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn copy_component(
    source: &Path,
    stage: &Path,
    kind: ComponentKind,
    rel: &str,
    first: bool,
    metrics: &mut CopyMetrics,
    fault: &mut impl FnMut(BackupBoundary) -> io::Result<()>,
) -> io::Result<ComponentSnapshot> {
    let metadata = fs::symlink_metadata(source)?;
    if !metadata.file_type().is_file() {
        return Err(invalid("source is not a regular file"));
    }
    let target = stage.join(rel);
    private_component_dirs(
        stage,
        target.parent().ok_or_else(|| invalid("component parent"))?,
    )?;
    let mut input = File::open(source)?;
    let mut output = private_file(&target)?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let copy_started = Instant::now();
        let n = input.read(&mut buf)?;
        if n == 0 {
            metrics.copy += copy_started.elapsed();
            break;
        }
        size = size
            .checked_add(n as u64)
            .ok_or_else(|| invalid("size overflow"))?;
        if size > MAX_TOTAL {
            return Err(invalid("component too large"));
        }
        output.write_all(&buf[..n])?;
        metrics.copy += copy_started.elapsed();
        let hash_started = Instant::now();
        hasher.update(&buf[..n]);
        metrics.sha256 += hash_started.elapsed();
    }
    if first {
        fault(BackupBoundary::AfterFirstComponentWrite)?;
    }
    output.sync_all()?;
    if first {
        fault(BackupBoundary::AfterFirstComponentSync)?;
    }
    let hash_started = Instant::now();
    let digest: [u8; 32] = hasher.finalize().into();
    metrics.sha256 += hash_started.elapsed();
    let hash_started = Instant::now();
    let reread = hash_file(&target)?;
    metrics.sha256 += hash_started.elapsed();
    if reread != (size, digest) {
        return Err(invalid("copy hash mismatch"));
    }
    Ok(ComponentSnapshot {
        kind,
        role: role_for(kind),
        path: rel.to_owned(),
        required: true,
        present: true,
        schema: 1,
        size,
        sha256: digest,
    })
}

fn optional_absent(kind: ComponentKind, path: &str) -> ComponentSnapshot {
    ComponentSnapshot {
        kind,
        role: role_for(kind),
        path: path.to_owned(),
        required: false,
        present: false,
        schema: 1,
        size: 0,
        sha256: [0; 32],
    }
}

fn identity_bytes(root: &Path) -> io::Result<([Vec<u8>; 3], IdentityId, u64, u64, [u8; 32])> {
    let files = ["ROOT", "LINEAGE", "HEAD"];
    let lengths = [
        wiseowl_identity::root::IDENTITY_ROOT_LEN,
        wiseowl_identity::lineage::LINEAGE_RECORD_LEN,
        wiseowl_identity::lineage::LINEAGE_HEAD_LEN,
    ];
    for (file, length) in files.iter().zip(lengths) {
        if fs::metadata(root.join("IDENTITY").join(file))?.len() != length as u64 {
            return Err(invalid("identity file length"));
        }
    }
    let bytes = files
        .map(|file| fs::read(root.join("IDENTITY").join(file)))
        .into_iter()
        .collect::<io::Result<Vec<_>>>()?;
    let bytes: [Vec<u8>; 3] = bytes
        .try_into()
        .map_err(|_| invalid("identity file count"))?;
    let r = IdentityRoot::decode(&bytes[0]).map_err(|_| invalid("ROOT"))?;
    let l = LineageRecord::decode(&bytes[1]).map_err(|_| invalid("LINEAGE"))?;
    let h = LineageHead::decode(&bytes[2]).map_err(|_| invalid("HEAD"))?;
    let validated = validate_identity_set(&r, &l, &h).map_err(|_| invalid("identity set"))?;
    Ok((
        bytes,
        validated.identity_id,
        validated.lineage_sequence.get(),
        validated.continuity_generation.get(),
        h.committed_record_hash(),
    ))
}

fn sync_tree_dirs(root: &Path) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if fs::symlink_metadata(&path)?.file_type().is_dir() {
            sync_tree_dirs(&path)?;
        }
    }
    File::open(root)?.sync_all()
}

#[cfg(target_os = "linux")]
fn publish_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let a = CString::new(from.as_os_str().as_bytes()).map_err(|_| invalid("path NUL"))?;
    let b = CString::new(to.as_os_str().as_bytes()).map_err(|_| invalid("path NUL"))?;
    let rc = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
fn publish_no_replace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io_other("no-replace publication unavailable"))
}

/// Trusted internal caller must own all durable mutation coordination. This
/// function takes the MemoryDB lock for its entire lifetime and rejects open
/// transactions before the snapshot boundary. It does not alter identity.
pub fn create_backup(
    db: &mut Database<FsStore>,
    source: &Path,
    destination: &Path,
    barrier: &mut dyn WiseOwlBackupBarrier,
) -> io::Result<BackupResult> {
    create_backup_with_id(db, source, destination, BackupId::generate()?, barrier)
}

fn create_backup_with_id(
    db: &mut Database<FsStore>,
    source: &Path,
    destination: &Path,
    backup_id: BackupId,
    barrier: &mut dyn WiseOwlBackupBarrier,
) -> io::Result<BackupResult> {
    create_backup_with_fault(db, source, destination, backup_id, barrier, &mut |_| Ok(()))
}

fn create_backup_with_fault(
    db: &mut Database<FsStore>,
    source: &Path,
    destination: &Path,
    backup_id: BackupId,
    barrier: &mut dyn WiseOwlBackupBarrier,
    fault: &mut impl FnMut(BackupBoundary) -> io::Result<()>,
) -> io::Result<BackupResult> {
    let started = Instant::now();
    let quiesce_started = Instant::now();
    for relative in ["", "IDENTITY", "WAL", "SEGMENTS", "INDEX"] {
        let path = source.join(relative);
        if !fs::symlink_metadata(path)?.file_type().is_dir() {
            return Err(invalid("source directory type"));
        }
    }
    if db.has_open_transactions() {
        return Err(io_other("open transactions"));
    }
    let identity = db
        .identity_context()
        .ok_or_else(|| io_other("identity unavailable"))?;
    let before = identity_bytes(source)?;
    let local_before = fs::read(source.join("IDENTITY/LOCAL"))?;
    if before.1 != identity.identity_id()
        || before.2 != identity.lineage_sequence().get()
        || before.3 != identity.continuity_generation().get()
    {
        return Err(invalid("identity context mismatch"));
    }
    if fs::symlink_metadata(destination)?.file_type().is_symlink() {
        return Err(invalid("symlink destination"));
    }
    let canonical_source = fs::canonicalize(source)?;
    let canonical_destination = fs::canonicalize(destination)?;
    if canonical_destination.starts_with(&canonical_source) {
        return Err(invalid("backup destination inside active store"));
    }
    let source_meta = fs::metadata(source)?;
    let dest_meta = fs::metadata(destination)?;
    if dest_meta.permissions().mode() & 0o077 != 0 {
        return Err(io_other("destination permissions are not private"));
    }
    let storage_independent = if source_meta.dev() == dest_meta.dev() {
        Some(false)
    } else {
        None
    };
    let mut frozen = FrozenBarrier::acquire(barrier)?;
    let checkpoint_started = Instant::now();
    db.create_checkpoint(&DbCaller::admin())
        .map_err(|_| io_other("checkpoint failed"))?;
    let checkpoint = checkpoint_started.elapsed();
    let id_hex = hex(&backup_id.0);
    let final_path = destination.join(format!("backup-{id_hex}"));
    let stage = destination.join(format!(".backup-{id_hex}.stage"));
    fault(BackupBoundary::BeforeStageCreate)?;
    fs::create_dir(&stage)?;
    fs::set_permissions(&stage, fs::Permissions::from_mode(0o700))?;
    fault(BackupBoundary::AfterStageCreate)?;
    let mut components = Vec::new();
    let mut copy_metrics = CopyMetrics::default();
    let hashing_started = Instant::now();
    for (kind, rel, source_rel) in [
        (
            ComponentKind::IdentityRoot,
            "IDENTITY/ROOT",
            "IDENTITY/ROOT",
        ),
        (
            ComponentKind::IdentityLineage,
            "IDENTITY/LINEAGE",
            "IDENTITY/LINEAGE",
        ),
        (
            ComponentKind::IdentityHead,
            "IDENTITY/HEAD",
            "IDENTITY/HEAD",
        ),
        (
            ComponentKind::MemoryDbManifest,
            "MEMORYDB/MANIFEST",
            "MANIFEST",
        ),
        (
            ComponentKind::MemoryDbWal,
            "MEMORYDB/WAL/wal-000001",
            "WAL/wal-000001",
        ),
        (
            ComponentKind::MemoryDbRelationshipSnapshot,
            "MEMORYDB/INDEX/relationships.bin",
            "INDEX/relationships.bin",
        ),
    ] {
        let first = components.is_empty();
        if first {
            fault(BackupBoundary::DuringFirstComponent)?;
        }
        if kind == ComponentKind::MemoryDbManifest {
            fault(BackupBoundary::BeforeMemoryDbExport)?;
        }
        components.push(copy_component(
            &source.join(source_rel),
            &stage,
            kind,
            rel,
            first,
            &mut copy_metrics,
            fault,
        )?);
    }
    for entry in fs::read_dir(source.join("SEGMENTS"))? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("data-") {
            let rel = format!("MEMORYDB/SEGMENTS/{name}");
            if !expected_path(ComponentKind::MemoryDbSegment, &rel) {
                return Err(invalid("segment name"));
            }
            components.push(copy_component(
                &entry.path(),
                &stage,
                ComponentKind::MemoryDbSegment,
                &rel,
                false,
                &mut copy_metrics,
                fault,
            )?);
        }
    }
    let guard_path = stage.join("MEMORYDB/BACKUP.PASSIVE");
    let mut guard = private_file(&guard_path)?;
    let guard_bytes = [b"WO-PASSIVE-BACKUP-v1\0".as_slice(), &backup_id.0].concat();
    guard.write_all(&guard_bytes)?;
    guard.sync_all()?;
    components.push(ComponentSnapshot {
        kind: ComponentKind::PassiveGuard,
        role: ComponentRole::PassiveGuard,
        path: "MEMORYDB/BACKUP.PASSIVE".to_owned(),
        required: true,
        present: true,
        schema: 1,
        size: guard_bytes.len() as u64,
        sha256: Sha256::digest(&guard_bytes).into(),
    });
    fault(BackupBoundary::DuringKvSnapshot)?;
    let kv_bytes = encode_kv_snapshot(backup_id, before.1, frozen.export_kv()?)?;
    validate_kv_snapshot(&kv_bytes, backup_id, before.1)?;
    let kv_rel = "KV/wiseowl-mtm.bin";
    private_component_dirs(&stage, &stage.join("KV"))?;
    let mut kv_file = private_file(&stage.join(kv_rel))?;
    kv_file.write_all(&kv_bytes)?;
    kv_file.sync_all()?;
    fault(BackupBoundary::AfterKvSnapshotSync)?;
    components.push(ComponentSnapshot {
        kind: ComponentKind::PersonalityKvExport,
        role: role_for(ComponentKind::PersonalityKvExport),
        path: kv_rel.to_owned(),
        required: true,
        present: true,
        schema: 1,
        size: kv_bytes.len() as u64,
        sha256: Sha256::digest(&kv_bytes).into(),
    });
    components.push(optional_absent(
        ComponentKind::IndexState,
        "INDEX/state.bin",
    ));
    components.push(optional_absent(
        ComponentKind::ActionReceipts,
        "RECEIPTS/sealed.bin",
    ));
    if identity_bytes(source)? != before {
        return Err(invalid("identity changed during snapshot"));
    }
    if fs::read(source.join("IDENTITY/LOCAL"))? != local_before {
        return Err(invalid("LOCAL changed during snapshot"));
    }
    let manifest = BackupManifest {
        purpose: PackagePurpose::Backup,
        activation_allowed: false,
        backup_id,
        identity_id: before.1,
        lineage_sequence: before.2,
        continuity_generation: before.3,
        head_digest: before.4,
        identity_format_version: wiseowl_identity::IDENTITY_FORMAT_VERSION,
        components,
    };
    let bytes = manifest.encode()?;
    fault(BackupBoundary::BeforeManifestWrite)?;
    let mut file = private_file(&stage.join("BACKUP.MANIFEST"))?;
    file.write_all(&bytes)?;
    fault(BackupBoundary::AfterManifestWrite)?;
    file.sync_all()?;
    fault(BackupBoundary::AfterManifestSync)?;
    fault(BackupBoundary::BeforeStageSync)?;
    sync_tree_dirs(&stage)?;
    fault(BackupBoundary::AfterStageSync)?;
    let hashing = hashing_started.elapsed();
    let bytes = inspect_components(&stage)?.total_bytes;
    fault(BackupBoundary::BeforePublish)?;
    publish_no_replace(&stage, &final_path)?;
    let package_dir = File::open(&final_path)?;
    package_lock(&package_dir, false)?;
    fault(BackupBoundary::AfterPublish)?;
    fault(BackupBoundary::BeforeParentSync)?;
    File::open(destination)?.sync_all()?;
    fault(BackupBoundary::AfterParentSync)?;
    fault(BackupBoundary::BeforeCommitMarker)?;
    let mut seal = UncommittedSeal {
        package: final_path.clone(),
        committed: false,
    };
    let marker = Sha256::digest(fs::read(final_path.join("BACKUP.MANIFEST"))?);
    let mut commit = private_file(&final_path.join("COMMITTED"))?;
    commit.write_all(COMMIT_MAGIC)?;
    commit.write_all(&FORMAT.to_le_bytes())?;
    commit.write_all(&[PackagePurpose::Backup as u8])?;
    commit.write_all(&backup_id.0)?;
    commit.write_all(&marker)?;
    fault(BackupBoundary::AfterCommitMarkerWrite)?;
    commit.sync_all()?;
    fault(BackupBoundary::AfterCommitMarkerSync)?;
    fault(BackupBoundary::BeforeFinalDirSync)?;
    package_dir.sync_all()?;
    seal.committed = true;
    fault(BackupBoundary::AfterFinalDirSync)?;
    let quiesce = quiesce_started.elapsed();
    Ok(BackupResult {
        path: final_path,
        backup_id,
        quiesce,
        checkpoint,
        component_copy: copy_metrics.copy,
        sha256: copy_metrics.sha256,
        total: started.elapsed(),
        hashing,
        bytes,
        storage_independent,
    })
}

/// Pure read-only inspection. No local installation or activation is created.
pub fn inspect_backup(package: &Path) -> io::Result<BackupInspection> {
    if !fs::symlink_metadata(package)?.file_type().is_dir() {
        return Err(invalid("package directory"));
    }
    let directory = File::open(package)?;
    package_lock(&directory, true)?;
    let commit_path = package.join("COMMITTED");
    let commit_metadata = fs::symlink_metadata(&commit_path)?;
    if !commit_metadata.file_type().is_file() || commit_metadata.len() != COMMIT_LEN as u64 {
        return Err(invalid("commit marker"));
    }
    let marker = fs::read(commit_path)?;
    let manifest_bytes = fs::read(package.join("BACKUP.MANIFEST"))?;
    let manifest = BackupManifest::decode(&manifest_bytes)?;
    if marker.len() != COMMIT_LEN
        || &marker[..8] != COMMIT_MAGIC
        || marker[8..10] != FORMAT.to_le_bytes()
        || marker[10] != PackagePurpose::Backup as u8
        || marker[11..27] != manifest.backup_id.0
        || &marker[27..] != Sha256::digest(&manifest_bytes).as_slice()
    {
        return Err(invalid("commit marker digest"));
    }
    inspect_components(package)
}

fn inspect_components(package: &Path) -> io::Result<BackupInspection> {
    if !fs::symlink_metadata(package)?.file_type().is_dir() {
        return Err(invalid("package directory"));
    }
    let manifest_path = package.join("BACKUP.MANIFEST");
    if !fs::symlink_metadata(&manifest_path)?.file_type().is_file() {
        return Err(invalid("manifest file"));
    }
    let file = File::open(&manifest_path)?;
    if file.metadata()?.len() as usize > MAX_MANIFEST {
        return Err(invalid("manifest too large"));
    }
    let manifest = BackupManifest::decode(&fs::read(manifest_path)?)?;
    let quotas = DbQuotaConfig::default();
    let mut db_bytes = 0u64;
    for c in &manifest.components {
        if !c.present {
            continue;
        }
        match c.kind {
            ComponentKind::MemoryDbManifest
            | ComponentKind::MemoryDbWal
            | ComponentKind::MemoryDbSegment
            | ComponentKind::MemoryDbRelationshipSnapshot => {
                db_bytes = db_bytes
                    .checked_add(c.size)
                    .ok_or_else(|| invalid("MemoryDB size overflow"))?;
                if c.kind == ComponentKind::MemoryDbWal && c.size > quotas.max_wal_bytes {
                    return Err(invalid("WAL size limit"));
                }
                if c.kind == ComponentKind::MemoryDbSegment
                    && c.size > quotas.max_segment_bytes as u64
                {
                    return Err(invalid("segment size limit"));
                }
            }
            _ => {}
        }
    }
    if db_bytes > quotas.max_database_bytes || db_bytes > quotas.max_recovery_bytes {
        return Err(invalid("MemoryDB recovery size limit"));
    }
    let guard_path = package.join("MEMORYDB/BACKUP.PASSIVE");
    let guard_metadata = fs::symlink_metadata(&guard_path)?;
    if !guard_metadata.file_type().is_file() || guard_metadata.len() != 37 {
        return Err(invalid("passive guard length/type"));
    }
    let guard = fs::read(guard_path)?;
    let expected_guard = [b"WO-PASSIVE-BACKUP-v1\0".as_slice(), &manifest.backup_id.0].concat();
    if guard != expected_guard {
        return Err(invalid("passive guard"));
    }
    let mut expected: BTreeSet<String> = manifest
        .components
        .iter()
        .filter(|c| c.present)
        .map(|c| c.path.clone())
        .collect();
    expected.insert("BACKUP.MANIFEST".to_owned());
    if package.join("COMMITTED").exists() {
        expected.insert("COMMITTED".to_owned());
    }
    fn check_tree(root: &Path, dir: &Path, expected: &BTreeSet<String>) -> io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let kind = fs::symlink_metadata(&path)?.file_type();
            if kind.is_symlink() {
                return Err(invalid("package symlink"));
            }
            if kind.is_dir() {
                check_tree(root, &path, expected)?;
            } else if kind.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|_| invalid("package path"))?
                    .to_str()
                    .ok_or_else(|| invalid("package path UTF-8"))?
                    .to_owned();
                if !expected.contains(&rel) {
                    return Err(invalid("unexpected package file"));
                }
            } else {
                return Err(invalid("unexpected package entry"));
            }
        }
        Ok(())
    }
    check_tree(package, package, &expected)?;
    let mut total = 0u64;
    for c in &manifest.components {
        if !c.present {
            continue;
        }
        let path = package.join(&c.path);
        if !fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(invalid("component not regular file"));
        }
        let (size, digest) = hash_file(&path)?;
        if (size, digest) != (c.size, c.sha256) {
            return Err(invalid("component SHA-256/size"));
        }
        if c.kind == ComponentKind::MemoryDbSegment {
            let expected_id = c
                .path
                .strip_prefix("MEMORYDB/SEGMENTS/data-")
                .and_then(|s| s.strip_suffix(".owlseg"))
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| invalid("segment number"))?;
            let bytes = fs::read(&path)?;
            let (header, _) = crate::segment::open_segment(&bytes, &quotas)
                .map_err(|_| invalid("segment format"))?;
            if header.segment_id != expected_id {
                return Err(invalid("segment id/path mismatch"));
            }
        }
        if c.kind == ComponentKind::PersonalityKvExport {
            if c.size > KV_MAX_BYTES as u64 {
                return Err(invalid("KV snapshot size"));
            }
            validate_kv_snapshot(&fs::read(&path)?, manifest.backup_id, manifest.identity_id)?;
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| invalid("total size overflow"))?;
    }
    let (_, identity_id, sequence, generation, head) = identity_bytes(package)?;
    if identity_id != manifest.identity_id
        || sequence != manifest.lineage_sequence
        || generation != manifest.continuity_generation
        || head != manifest.head_digest
    {
        return Err(invalid("manifest identity binding"));
    }
    let memorydb = package.join("MEMORYDB");
    let store = FsStore::open_read_only(&memorydb).map_err(|_| invalid("MemoryDB layout"))?;
    validate_store_read_only(&store, DbQuotaConfig::default())
        .map_err(|_| invalid("MemoryDB contents"))?;
    Ok(BackupInspection {
        manifest,
        total_bytes: total,
        storage_independent: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activation::LocalActivationRecord;
    use crate::database::InsertRequest;
    use crate::identity::{
        detect_store_state, ensure_identity, host::HostIdentityStorage, AdoptionPolicy,
        CreationBoundary, DetectedStoreState, IdentityStartupError, StoreDisposition,
    };
    use crate::provenance::{DerivationKind, LongTermProvenance};
    use crate::query::DedupPolicy;
    use crate::record::{LongTermMemoryKind, MemoryScope};
    use wiseowl_identity::{ActivationId, ActivationState, InstallationId};
    use wiseowl_memory::{SourceKind, TrustLevel};

    #[derive(Default)]
    struct TestBarrier {
        frozen: bool,
    }

    impl WiseOwlBackupBarrier for TestBarrier {
        fn begin(&mut self) -> io::Result<()> {
            assert!(!self.frozen);
            self.frozen = true;
            Ok(())
        }
        fn export_kv(&mut self) -> io::Result<Vec<(String, Vec<u8>)>> {
            assert!(self.frozen);
            Ok(vec![
                ("wb1:2a:vc".to_owned(), b"7".to_vec()),
                ("wb1:2a:gs".to_owned(), b"concise".to_vec()),
            ])
        }
        fn end(&mut self) {
            self.frozen = false;
        }
    }

    fn create_backup(
        db: &mut Database<FsStore>,
        source: &Path,
        destination: &Path,
    ) -> io::Result<BackupResult> {
        super::create_backup(db, source, destination, &mut TestBarrier::default())
    }

    fn create_backup_with_id(
        db: &mut Database<FsStore>,
        source: &Path,
        destination: &Path,
        id: BackupId,
    ) -> io::Result<BackupResult> {
        super::create_backup_with_id(db, source, destination, id, &mut TestBarrier::default())
    }

    fn create_backup_with_fault(
        db: &mut Database<FsStore>,
        source: &Path,
        destination: &Path,
        id: BackupId,
        fault: &mut impl FnMut(BackupBoundary) -> io::Result<()>,
    ) -> io::Result<BackupResult> {
        super::create_backup_with_fault(
            db,
            source,
            destination,
            id,
            &mut TestBarrier::default(),
            fault,
        )
    }

    fn fixture() -> (tempfile::TempDir, tempfile::TempDir, Database<FsStore>) {
        let source = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        fs::set_permissions(dest.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut storage = HostIdentityStorage::open(source.path()).unwrap();
        let identity = ensure_identity(
            &mut storage,
            StoreDisposition::Fresh,
            AdoptionPolicy::Disabled,
            |bytes| {
                bytes.fill(0x53);
                Ok(())
            },
            |_: CreationBoundary| -> Result<(), IdentityStartupError> { Ok(()) },
        )
        .unwrap();
        let mut db = Database::open_fs(source.path(), DbQuotaConfig::default()).unwrap();
        db.bind_identity_context(identity);
        let activation = ActivationId::from_bytes([0x22; 16]).unwrap();
        let local = LocalActivationRecord {
            identity_id: identity.identity_id(),
            installation_id: InstallationId::from_bytes([0x11; 16]).unwrap(),
            state: ActivationState::Active,
            activation_id: Some(activation),
            clean_stop: false,
            boot_epoch: [0x33; 16],
            activation_sequence: 1,
        };
        fs::write(source.path().join("IDENTITY/LOCAL"), local.encode()).unwrap();
        db.bind_activation(activation.fingerprint());
        (source, dest, db)
    }

    #[test]
    fn kv_snapshot_is_typed_bounded_and_identity_bound() {
        let (_, _, db) = fixture();
        let identity = db.identity_context().unwrap().identity_id();
        let id = BackupId::new([0x45; 16]).unwrap();
        let approved = ["vc", "gen", "lp", "gs", "sms", "sis"]
            .into_iter()
            .map(|kind| (format!("wb1:2a:{kind}"), b"1".to_vec()))
            .collect();
        let bytes = encode_kv_snapshot(id, identity, approved).unwrap();
        validate_kv_snapshot(&bytes, id, identity).unwrap();
        assert!(
            validate_kv_snapshot(&bytes, BackupId::new([0x46; 16]).unwrap(), identity).is_err()
        );
        for key in [
            "other:2a:vc",
            "wb1:02a:vc",
            "wb1:2A:vc",
            "wb1:2a:unknown",
            "wb1:100000000:vc",
        ] {
            assert!(
                encode_kv_snapshot(id, identity, vec![(key.to_owned(), b"1".to_vec())]).is_err(),
                "{key}"
            );
        }
        assert!(encode_kv_snapshot(
            id,
            identity,
            vec![
                ("wb1:2a:vc".to_owned(), b"1".to_vec()),
                ("wb1:2a:vc".to_owned(), b"2".to_vec())
            ]
        )
        .is_err());
        assert!(
            encode_kv_snapshot(id, identity, vec![("wb1:2a:vc".to_owned(), vec![0; 65])]).is_err()
        );
    }

    #[test]
    fn missing_kv_aborts_and_thaws_barrier() {
        struct Unavailable(bool);
        impl WiseOwlBackupBarrier for Unavailable {
            fn begin(&mut self) -> io::Result<()> {
                self.0 = true;
                Ok(())
            }
            fn export_kv(&mut self) -> io::Result<Vec<(String, Vec<u8>)>> {
                Err(io_other("KV unavailable"))
            }
            fn end(&mut self) {
                self.0 = false;
            }
        }
        let (source, dest, mut db) = fixture();
        let mut barrier = Unavailable(false);
        assert!(super::create_backup(&mut db, source.path(), dest.path(), &mut barrier).is_err());
        assert!(!barrier.0);
        assert!(fs::read_dir(dest.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("backup-")));
    }

    #[test]
    fn partial_barrier_acquisition_failure_thaws() {
        struct Partial(bool);
        impl WiseOwlBackupBarrier for Partial {
            fn begin(&mut self) -> io::Result<()> {
                self.0 = true;
                Err(io_other("participant failed"))
            }
            fn export_kv(&mut self) -> io::Result<Vec<(String, Vec<u8>)>> {
                unreachable!()
            }
            fn end(&mut self) {
                self.0 = false;
            }
        }
        let (source, dest, mut db) = fixture();
        let mut barrier = Partial(false);
        assert!(super::create_backup(&mut db, source.path(), dest.path(), &mut barrier).is_err());
        assert!(!barrier.0);
    }

    #[test]
    fn creates_passive_package_without_identity_change() {
        let (source, dest, mut db) = fixture();
        let before = identity_bytes(source.path()).unwrap();
        let local_before = fs::read(source.path().join("IDENTITY/LOCAL")).unwrap();
        let status_before = db.identity_status().unwrap();
        let result = create_backup(&mut db, source.path(), dest.path()).unwrap();
        eprintln!(
            "backup metrics: bytes={} quiesce_ms={} checkpoint_ms={} copy_ms={} sha256_ms={} total_ms={} copy_hash_ms={}",
            result.bytes,
            result.quiesce.as_millis(),
            result.checkpoint.as_millis(),
            result.component_copy.as_millis(),
            result.sha256.as_millis(),
            result.total.as_millis(),
            result.hashing.as_millis()
        );
        let view = inspect_backup(&result.path).unwrap();
        assert_eq!(view.manifest.purpose, PackagePurpose::Backup);
        assert!(!view.manifest.activation_allowed);
        assert_eq!(view.manifest.identity_id, before.1);
        assert_eq!(view.manifest.lineage_sequence, 1);
        assert_eq!(view.manifest.continuity_generation, 1);
        assert_eq!(view.manifest.head_digest, before.4);
        assert_eq!(identity_bytes(source.path()).unwrap(), before);
        assert_eq!(
            fs::read(source.path().join("IDENTITY/LOCAL")).unwrap(),
            local_before
        );
        assert_eq!(db.identity_status().unwrap(), status_before);
        assert_eq!(result.storage_independent, Some(false));
        assert!(view
            .manifest
            .components
            .iter()
            .filter(|c| c.required)
            .all(|c| c.present));
        assert!(view
            .manifest
            .components
            .iter()
            .filter(|c| !c.required)
            .all(|c| !c.present));
        assert!(!result.path.join("IDENTITY/LOCAL").exists());
        assert_eq!(
            fs::metadata(&result.path).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            fs::metadata(result.path.join("MEMORYDB"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        assert_eq!(
            fs::metadata(result.path.join("IDENTITY/ROOT"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        let passive_store = HostIdentityStorage::open(result.path.join("MEMORYDB")).unwrap();
        assert_eq!(
            detect_store_state(&passive_store).unwrap(),
            DetectedStoreState::Uncertain
        );
        let copy = dest.path().join("copied-passive-package");
        fs::create_dir(&copy).unwrap();
        fn copy_tree(a: &Path, b: &Path) {
            for entry in fs::read_dir(a).unwrap() {
                let entry = entry.unwrap();
                let from = entry.path();
                let to = b.join(entry.file_name());
                if from.is_dir() {
                    fs::create_dir(&to).unwrap();
                    copy_tree(&from, &to);
                } else {
                    fs::copy(from, to).unwrap();
                }
            }
        }
        copy_tree(&result.path, &copy);
        drop(db);
        drop(source);
        assert_eq!(
            inspect_backup(&copy).unwrap().manifest.backup_id,
            result.backup_id
        );
    }

    #[test]
    fn checkpointed_segment_and_owner_survive_package_validation() {
        let (source, dest, mut db) = fixture();
        let caller = DbCaller::user(1001);
        let record = InsertRequest {
            kind: LongTermMemoryKind::Observation,
            scope: MemoryScope::User,
            owner: 1001,
            payload: b"durable promoted memory".to_vec(),
            provenance: LongTermProvenance {
                source_kind: SourceKind::UserInput,
                source_id: None,
                producer_service: "backup-test".to_owned(),
                original_memory_ids: Vec::new(),
                parent_lt_ids: Vec::new(),
                insertion_time_ns: 1,
                trust: TrustLevel::Untrusted,
                source_content_hash: None,
                external_ref: None,
                derivation: DerivationKind::DirectImport,
            },
            confidence: 900,
            importance: 100,
            trust: TrustLevel::Untrusted,
            valid_from_ns: None,
            valid_until_ns: None,
            tokens: None,
            attributes: crate::attributes::AttributeSet::default(),
            supersedes: None,
            relationships: Vec::new(),
            dedup: DedupPolicy::Allow,
            id: None,
            revision: 1,
        };
        let memory_id = db.insert_one(&caller, record).unwrap();
        let backup = create_backup(&mut db, source.path(), dest.path()).unwrap();
        let view = inspect_backup(&backup.path).unwrap();
        let segment = view
            .manifest
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::MemoryDbSegment)
            .unwrap();
        assert!(segment.required && segment.present && segment.size > 0);
        assert_eq!(
            db.get_record(&caller, memory_id, false).unwrap().owner,
            1001
        );
        let mut byte = fs::read(backup.path.join(&segment.path)).unwrap();
        byte[0] ^= 1;
        fs::write(backup.path.join(&segment.path), byte).unwrap();
        assert!(inspect_backup(&backup.path).is_err());
    }

    #[test]
    fn multi_segment_relationship_fixture_reports_development_metrics() {
        use crate::provenance::RelationshipProvenance;
        use crate::relationship::{MemoryRelationship, RelationshipKind};
        let (source, dest, mut db) = fixture();
        let caller = DbCaller::admin();
        let mut ids = Vec::new();
        for n in 0..64u64 {
            let mut seed = n + 1;
            let payload = (0..2048)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            let record = InsertRequest {
                kind: LongTermMemoryKind::Observation,
                scope: MemoryScope::User,
                owner: 1001,
                payload,
                provenance: LongTermProvenance {
                    source_kind: SourceKind::UserInput,
                    source_id: None,
                    producer_service: "backup-metric-fixture".to_owned(),
                    original_memory_ids: Vec::new(),
                    parent_lt_ids: Vec::new(),
                    insertion_time_ns: n + 1,
                    trust: TrustLevel::Untrusted,
                    source_content_hash: None,
                    external_ref: None,
                    derivation: DerivationKind::DirectImport,
                },
                confidence: 900,
                importance: 100,
                trust: TrustLevel::Untrusted,
                valid_from_ns: None,
                valid_until_ns: None,
                tokens: None,
                attributes: crate::attributes::AttributeSet::default(),
                supersedes: None,
                relationships: Vec::new(),
                dedup: DedupPolicy::Allow,
                id: None,
                revision: 1,
            };
            ids.push(db.insert_one(&caller, record).unwrap());
            if n % 16 == 15 {
                db.create_checkpoint(&caller).unwrap();
            }
        }
        let tx = db.begin_transaction(&caller).unwrap();
        db.insert_relationship(
            &caller,
            tx,
            MemoryRelationship {
                source: ids[0],
                target: ids[1],
                kind: RelationshipKind::RelatedTo,
                confidence: 800,
                created_at_ns: 100,
                provenance: RelationshipProvenance {
                    producer_service: "backup-metric-fixture".to_owned(),
                    created_at_ns: 100,
                    trust: TrustLevel::Untrusted,
                },
                tombstoned: false,
            },
        )
        .unwrap();
        db.commit_transaction(&caller, tx).unwrap();
        let before: Vec<_> = ids
            .iter()
            .map(|id| db.get_record(&caller, *id, false).unwrap())
            .collect();
        let relationships_before = db.get_relationships(&caller, ids[0]).unwrap();
        let identity_before = identity_bytes(source.path()).unwrap();
        let local_before = fs::read(source.path().join("IDENTITY/LOCAL")).unwrap();
        let status_before = db.identity_status().unwrap();
        let result = create_backup(&mut db, source.path(), dest.path()).unwrap();
        let inspection = inspect_backup(&result.path).unwrap();
        let after: Vec<_> = ids
            .iter()
            .map(|id| db.get_record(&caller, *id, false).unwrap())
            .collect();
        assert_eq!(before, after);
        assert_eq!(
            db.get_relationships(&caller, ids[0]).unwrap(),
            relationships_before
        );
        assert_eq!(identity_bytes(source.path()).unwrap(), identity_before);
        assert_eq!(
            fs::read(source.path().join("IDENTITY/LOCAL")).unwrap(),
            local_before
        );
        assert_eq!(db.identity_status().unwrap(), status_before);
        let segments = inspection
            .manifest
            .components
            .iter()
            .filter(|c| c.kind == ComponentKind::MemoryDbSegment)
            .count();
        assert!(segments >= 2, "expected multiple segments, got {segments}");
        eprintln!("phase-d-metric records=64 relationships=1 segments={} bytes={} freeze_us={} checkpoint_us={} copy_us={} sha256_us={} total_us={}",
            segments, result.bytes, result.quiesce.as_micros(), result.checkpoint.as_micros(),
            result.component_copy.as_micros(), result.sha256.as_micros(), result.total.as_micros());
    }

    #[test]
    fn rejects_corruption_missing_required_and_interrupted_staging() {
        let (source, dest, mut db) = fixture();
        let result = create_backup(&mut db, source.path(), dest.path()).unwrap();
        let stage = dest.path().join(".unfinished.stage");
        fs::create_dir(&stage).unwrap();
        fs::write(
            stage.join("BACKUP.MANIFEST"),
            fs::read(result.path.join("BACKUP.MANIFEST")).unwrap(),
        )
        .unwrap();
        assert!(inspect_backup(&stage).is_err());
        fs::write(result.path.join("MEMORYDB/WAL/wal-000001"), b"corrupt").unwrap();
        assert!(inspect_backup(&result.path).is_err());
        fs::remove_file(result.path.join("MEMORYDB/MANIFEST")).unwrap();
        assert!(inspect_backup(&result.path).is_err());
        assert_eq!(
            identity_bytes(source.path()).unwrap().1,
            db.identity_context().unwrap().identity_id()
        );
    }

    #[test]
    fn inspection_distinguishes_uncommitted_and_legacy_packages() {
        let (source, dest, mut db) = fixture();
        let result = create_backup(&mut db, source.path(), dest.path()).unwrap();
        assert_eq!(
            inspect_package_status(&result.path),
            PackageInspectionStatus::ValidCommittedBackup
        );
        fs::remove_file(result.path.join("COMMITTED")).unwrap();
        assert_eq!(
            inspect_package_status(&result.path),
            PackageInspectionStatus::ValidButUncommittedPackage
        );
        let manifest_path = result.path.join("BACKUP.MANIFEST");
        let mut bytes = fs::read(&manifest_path).unwrap();
        bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
        fs::write(&manifest_path, bytes).unwrap();
        assert_eq!(
            inspect_package_status(&result.path),
            PackageInspectionStatus::UnsupportedPackage
        );
    }

    #[test]
    fn rejects_manifest_malformation_and_unsafe_paths() {
        let (source, dest, mut db) = fixture();
        let result = create_backup(&mut db, source.path(), dest.path()).unwrap();
        let mut m = inspect_backup(&result.path).unwrap().manifest;
        m.components[0].path = "../ROOT".to_owned();
        assert!(m.encode().is_err());
        m.components[0].path = "IDENTITY/ROOT".to_owned();
        m.components.push(m.components[0].clone());
        assert!(m.encode().is_err());
        let mut bytes = fs::read(result.path.join("BACKUP.MANIFEST")).unwrap();
        bytes[0] ^= 1;
        assert!(BackupManifest::decode(&bytes).is_err());
        bytes.truncate(10);
        assert!(BackupManifest::decode(&bytes).is_err());
    }

    #[test]
    fn inspection_rejects_unexpected_files_and_does_not_write() {
        use std::os::unix::fs::symlink;
        let (source, dest, mut db) = fixture();
        let result = create_backup(&mut db, source.path(), dest.path()).unwrap();
        fn file_list(root: &Path) -> Vec<(String, Vec<u8>)> {
            fn visit(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
                for entry in fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        visit(root, &path, out);
                    } else {
                        out.push((
                            path.strip_prefix(root)
                                .unwrap()
                                .to_string_lossy()
                                .into_owned(),
                            fs::read(path).unwrap(),
                        ));
                    }
                }
            }
            let mut out = Vec::new();
            visit(root, root, &mut out);
            out.sort();
            out
        }
        let before = file_list(&result.path);
        inspect_backup(&result.path).unwrap();
        assert_eq!(file_list(&result.path), before);
        fs::write(result.path.join("EXTRA"), b"unexpected").unwrap();
        assert!(inspect_backup(&result.path).is_err());
        fs::remove_file(result.path.join("EXTRA")).unwrap();
        symlink("IDENTITY/ROOT", result.path.join("LINK")).unwrap();
        assert!(inspect_backup(&result.path).is_err());
    }

    #[test]
    fn open_transaction_and_active_store_destination_are_rejected() {
        let (source, dest, mut db) = fixture();
        let before = identity_bytes(source.path()).unwrap();
        let tx = db.begin_transaction(&DbCaller::admin()).unwrap();
        assert!(create_backup(&mut db, source.path(), dest.path()).is_err());
        db.abort_transaction(&DbCaller::admin(), tx).unwrap();
        assert!(create_backup(&mut db, source.path(), source.path()).is_err());
        assert_eq!(identity_bytes(source.path()).unwrap(), before);
        assert!(create_backup(&mut db, source.path(), dest.path()).is_ok());
    }

    #[test]
    fn existing_backup_is_never_replaced() {
        let (source, dest, mut db) = fixture();
        let id = BackupId::new([0xA5; 16]).unwrap();
        let first = create_backup_with_id(&mut db, source.path(), dest.path(), id).unwrap();
        let manifest_before = fs::read(first.path.join("BACKUP.MANIFEST")).unwrap();
        assert!(create_backup_with_id(&mut db, source.path(), dest.path(), id).is_err());
        assert_eq!(
            fs::read(first.path.join("BACKUP.MANIFEST")).unwrap(),
            manifest_before
        );
        assert!(inspect_backup(&first.path).is_ok());
    }

    #[test]
    fn publication_fault_boundaries_never_change_identity() {
        let boundaries = [
            BackupBoundary::BeforeStageCreate,
            BackupBoundary::AfterStageCreate,
            BackupBoundary::DuringFirstComponent,
            BackupBoundary::AfterFirstComponentWrite,
            BackupBoundary::AfterFirstComponentSync,
            BackupBoundary::BeforeMemoryDbExport,
            BackupBoundary::DuringKvSnapshot,
            BackupBoundary::AfterKvSnapshotSync,
            BackupBoundary::BeforeManifestWrite,
            BackupBoundary::AfterManifestWrite,
            BackupBoundary::AfterManifestSync,
            BackupBoundary::BeforeStageSync,
            BackupBoundary::AfterStageSync,
            BackupBoundary::BeforePublish,
            BackupBoundary::AfterPublish,
            BackupBoundary::BeforeParentSync,
            BackupBoundary::AfterParentSync,
            BackupBoundary::BeforeCommitMarker,
            BackupBoundary::AfterCommitMarkerWrite,
            BackupBoundary::AfterCommitMarkerSync,
            BackupBoundary::BeforeFinalDirSync,
            BackupBoundary::AfterFinalDirSync,
        ];
        for (index, boundary) in boundaries.into_iter().enumerate() {
            let (source, dest, mut db) = fixture();
            let before = identity_bytes(source.path()).unwrap();
            let id = BackupId::new([index as u8 + 1; 16]).unwrap();
            let mut fault = |seen| {
                if seen == boundary {
                    Err(io::ErrorKind::Interrupted.into())
                } else {
                    Ok(())
                }
            };
            assert!(
                create_backup_with_fault(&mut db, source.path(), dest.path(), id, &mut fault)
                    .is_err(),
                "{boundary:?}"
            );
            assert_eq!(
                identity_bytes(source.path()).unwrap(),
                before,
                "{boundary:?}"
            );
            let name = format!("backup-{}", hex(&id.0));
            let final_path = dest.path().join(name);
            let valid = inspect_backup(&final_path).is_ok();
            // Ordinary injected failures remove an uncommitted seal. A VM
            // stop bypasses cleanup and needs its own native fault test.
            assert_eq!(
                valid,
                boundary == BackupBoundary::AfterFinalDirSync,
                "{boundary:?}"
            );
        }
    }

    #[test]
    fn concurrent_validator_waits_for_final_directory_sync() {
        use std::sync::mpsc;
        let (source, dest, mut db) = fixture();
        let id = BackupId::new([0x91; 16]).unwrap();
        let final_path = dest.path().join(format!("backup-{}", hex(&id.0)));
        let (ready_tx, ready_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let mut senders = Some((ready_tx, result_tx));
        let mut handle = None;
        let mut fault = |boundary| {
            if boundary == BackupBoundary::AfterCommitMarkerSync {
                let package = final_path.clone();
                let (ready_tx, result_tx) = senders.take().unwrap();
                handle = Some(std::thread::spawn(move || {
                    ready_tx.send(()).unwrap();
                    result_tx.send(inspect_backup(&package).is_ok()).unwrap();
                }));
                ready_rx.recv().unwrap();
                assert!(result_rx.recv_timeout(Duration::from_millis(50)).is_err());
            }
            Ok(())
        };
        super::create_backup_with_fault(
            &mut db,
            source.path(),
            dest.path(),
            id,
            &mut TestBarrier::default(),
            &mut fault,
        )
        .unwrap();
        assert!(result_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        handle.unwrap().join().unwrap();
    }

    #[test]
    fn concurrent_validator_is_gated_at_every_publication_boundary() {
        use std::sync::mpsc;

        let boundaries = [
            BackupBoundary::BeforeStageCreate,
            BackupBoundary::AfterStageCreate,
            BackupBoundary::DuringFirstComponent,
            BackupBoundary::AfterFirstComponentWrite,
            BackupBoundary::AfterFirstComponentSync,
            BackupBoundary::BeforeMemoryDbExport,
            BackupBoundary::DuringKvSnapshot,
            BackupBoundary::AfterKvSnapshotSync,
            BackupBoundary::BeforeManifestWrite,
            BackupBoundary::AfterManifestWrite,
            BackupBoundary::AfterManifestSync,
            BackupBoundary::BeforeStageSync,
            BackupBoundary::AfterStageSync,
            BackupBoundary::BeforePublish,
            BackupBoundary::AfterPublish,
            BackupBoundary::BeforeParentSync,
            BackupBoundary::AfterParentSync,
            BackupBoundary::BeforeCommitMarker,
            BackupBoundary::AfterCommitMarkerWrite,
            BackupBoundary::AfterCommitMarkerSync,
            BackupBoundary::BeforeFinalDirSync,
            BackupBoundary::AfterFinalDirSync,
        ];

        for (index, boundary) in boundaries.into_iter().enumerate() {
            let (source, dest, mut db) = fixture();
            let id = BackupId::new([index as u8 + 1; 16]).unwrap();
            let final_path = dest.path().join(format!("backup-{}", hex(&id.0)));
            let marker_visible = matches!(
                boundary,
                BackupBoundary::AfterCommitMarkerWrite
                    | BackupBoundary::AfterCommitMarkerSync
                    | BackupBoundary::BeforeFinalDirSync
                    | BackupBoundary::AfterFinalDirSync
            );
            let (ready_tx, ready_rx) = mpsc::channel();
            let (result_tx, result_rx) = mpsc::channel();
            let mut senders = Some((ready_tx, result_tx));
            let mut handle = None;
            let mut fault = |seen| {
                if seen == boundary {
                    let package = final_path.clone();
                    let (ready_tx, result_tx) = senders.take().unwrap();
                    handle = Some(std::thread::spawn(move || {
                        ready_tx.send(()).unwrap();
                        result_tx.send(inspect_package_status(&package)).unwrap();
                    }));
                    ready_rx.recv().unwrap();
                    if !marker_visible {
                        assert_ne!(
                            result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
                            PackageInspectionStatus::ValidCommittedBackup,
                            "{boundary:?}"
                        );
                    } else {
                        assert!(result_rx.recv_timeout(Duration::from_millis(50)).is_err());
                    }
                }
                Ok(())
            };

            create_backup_with_fault(&mut db, source.path(), dest.path(), id, &mut fault)
                .unwrap_or_else(|error| panic!("{boundary:?}: {error}"));

            if marker_visible {
                assert_eq!(
                    result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
                    PackageInspectionStatus::ValidCommittedBackup,
                    "{boundary:?}"
                );
            }
            assert_eq!(
                inspect_package_status(&final_path),
                PackageInspectionStatus::ValidCommittedBackup,
                "{boundary:?}"
            );
            handle.unwrap().join().unwrap();
        }
    }

    #[test]
    fn failures_leave_identity_intact() {
        let (source, dest, mut db) = fixture();
        let before = identity_bytes(source.path()).unwrap();
        let missing = dest.path().join("no-such-destination");
        assert!(create_backup(&mut db, source.path(), &missing).is_err());
        assert_eq!(identity_bytes(source.path()).unwrap(), before);
        assert!(fs::read_dir(dest.path()).unwrap().next().is_none());
    }
}
