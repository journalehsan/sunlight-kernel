//! Private service-secret storage with atomic live-system publication.
//!
//! The kernel-backed staging and publish calls deliberately only operate
//! beneath `/etc/sunlight` and require the process to have the explicit
//! `HostKeyAdmin` service capability.  This avoids turning a helper that is
//! convenient for host keys into an arbitrary-path privileged writer.
//!
//! The supported durability level is atomic visibility only: while the system
//! remains running, publication leaves readers with either the complete old
//! file or the complete new file.  There is no `fsync` or directory-sync
//! primitive yet, so a host/VM/power crash can lose recent data or retain
//! either complete version.  `RequireDurability` therefore fails closed.

use crate::{self as libc, Errno, Fd, Stat, FT_FILE, MAX_PATH};

pub const PRIVATE_SECRET_DIRECTORY: &[u8] = b"/etc/sunlight/";
pub const DEFAULT_PRIVATE_MODE: u16 = 0o600;
pub const DEFAULT_MAX_SECRET_SIZE: usize = 4 * 1024;
const TEMPORARY_RETRIES: usize = 8;
const TEMPORARY_TOKEN_BYTES: usize = 16;
const TEMPORARY_TOKEN_HEX_LEN: usize = TEMPORARY_TOKEN_BYTES * 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretPublishMode {
    CreateIfAbsent,
    ReplaceExisting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Durability {
    AtomicVisibility,
    DurableWhenSupported,
    RequireDurability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecretFileOptions<'a> {
    pub destination: &'a [u8],
    pub owner_uid: u32,
    pub owner_gid: u32,
    pub mode: u16,
    pub maximum_size: usize,
    pub publish_mode: SecretPublishMode,
    pub durability: Durability,
}

impl<'a> SecretFileOptions<'a> {
    pub const fn system(destination: &'a [u8]) -> Self {
        Self {
            destination,
            owner_uid: 0,
            owner_gid: 0,
            mode: DEFAULT_PRIVATE_MODE,
            maximum_size: DEFAULT_MAX_SECRET_SIZE,
            publish_mode: SecretPublishMode::CreateIfAbsent,
            durability: Durability::AtomicVisibility,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretStoreError {
    InvalidDestination,
    DestinationOutsidePolicy,
    DestinationParentMissing,
    DestinationParentNotDirectory,
    InsecureParent,
    DestinationAlreadyExists,
    DestinationMissing,
    UnexpectedTargetType,
    UnexpectedOwner,
    InsecurePermissions,
    InvalidSecretFormat,
    SecretTooLarge,
    SecretEmpty,
    SecureRandomUnavailable,
    TemporaryNameCollision,
    CreateFailed,
    WriteFailed,
    ShortWrite,
    CloseFailed,
    RenameFailed,
    CleanupFailed,
    PermissionDenied,
    DurabilityUnsupported,
    ReadFailed,
    InternalError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateResult {
    Created,
    Existing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecretStoreCounters {
    pub secret_create_attempt_total: u64,
    pub secret_create_success_total: u64,
    pub secret_create_existing_total: u64,
    pub secret_create_race_lost_total: u64,
    pub secret_replace_attempt_total: u64,
    pub secret_replace_success_total: u64,
    pub secret_replace_failure_total: u64,
    pub secret_load_success_total: u64,
    pub secret_load_failure_total: u64,
    pub temporary_create_total: u64,
    pub temporary_cleanup_total: u64,
    pub temporary_cleanup_failure_total: u64,
    pub temporary_collision_total: u64,
    pub stale_temporary_removed_total: u64,
    pub permission_rejection_total: u64,
    pub ownership_rejection_total: u64,
    pub unexpected_type_rejection_total: u64,
    pub validation_failure_total: u64,
    pub rename_failure_total: u64,
    pub close_failure_total: u64,
}

/// Validator receives bounded private bytes and must not log or retain them.
pub type SecretValidator = fn(&[u8]) -> bool;

/// Stateless private-secret primitive. Counters are process-local so callers
/// can export them through their own bounded service diagnostics.
pub struct SecretStore {
    counters: SecretStoreCounters,
}

impl SecretStore {
    pub const fn new() -> Self {
        Self {
            counters: SecretStoreCounters {
                secret_create_attempt_total: 0,
                secret_create_success_total: 0,
                secret_create_existing_total: 0,
                secret_create_race_lost_total: 0,
                secret_replace_attempt_total: 0,
                secret_replace_success_total: 0,
                secret_replace_failure_total: 0,
                secret_load_success_total: 0,
                secret_load_failure_total: 0,
                temporary_create_total: 0,
                temporary_cleanup_total: 0,
                temporary_cleanup_failure_total: 0,
                temporary_collision_total: 0,
                stale_temporary_removed_total: 0,
                permission_rejection_total: 0,
                ownership_rejection_total: 0,
                unexpected_type_rejection_total: 0,
                validation_failure_total: 0,
                rename_failure_total: 0,
                close_failure_total: 0,
            },
        }
    }

    pub const fn counters(&self) -> SecretStoreCounters {
        self.counters
    }

    /// Create and publish only if the destination remains absent.  A peer
    /// winning the final publication race is reported as `Existing`; callers
    /// should then call `load` rather than treating it as corruption.
    pub fn create_if_absent(
        &mut self,
        options: SecretFileOptions<'_>,
        secret: &mut [u8],
        validator: SecretValidator,
    ) -> Result<CreateResult, SecretStoreError> {
        self.counters.secret_create_attempt_total += 1;
        let result = self.store(
            SecretFileOptions {
                publish_mode: SecretPublishMode::CreateIfAbsent,
                ..options
            },
            secret,
            validator,
        );
        wipe(secret);
        result
    }

    /// Replace only a pre-existing, valid private regular destination.
    pub fn replace(
        &mut self,
        options: SecretFileOptions<'_>,
        secret: &mut [u8],
        validator: SecretValidator,
    ) -> Result<(), SecretStoreError> {
        self.counters.secret_replace_attempt_total += 1;
        let result = match self.store(
            SecretFileOptions {
                publish_mode: SecretPublishMode::ReplaceExisting,
                ..options
            },
            secret,
            validator,
        ) {
            Ok(CreateResult::Created) => {
                self.counters.secret_replace_success_total += 1;
                Ok(())
            }
            Ok(CreateResult::Existing) => {
                self.counters.secret_replace_failure_total += 1;
                Err(SecretStoreError::InternalError)
            }
            Err(error) => {
                self.counters.secret_replace_failure_total += 1;
                Err(error)
            }
        };
        wipe(secret);
        result
    }

    /// Load into a caller-owned bounded buffer after descriptor-based metadata
    /// validation.  The returned slice is valid until the caller overwrites
    /// `out`; callers should clear it when their cryptographic parser is done.
    pub fn load<'a>(
        &mut self,
        options: SecretFileOptions<'_>,
        out: &'a mut [u8],
        validator: SecretValidator,
    ) -> Result<&'a [u8], SecretStoreError> {
        if let Err(error) = validate_options(options) {
            self.counters.secret_load_failure_total += 1;
            return Err(error);
        }
        let fd = match libc::open(options.destination) {
            Ok(fd) => fd,
            Err(_) => {
                self.counters.secret_load_failure_total += 1;
                return Err(SecretStoreError::DestinationMissing);
            }
        };
        let stat = match libc::fstat(fd) {
            Ok(stat) => stat,
            Err(_) => {
                let _ = libc::close(fd);
                self.counters.secret_load_failure_total += 1;
                return Err(SecretStoreError::ReadFailed);
            }
        };
        if let Err(error) = self.validate_stat(stat, options, true) {
            let _ = libc::close(fd);
            self.counters.secret_load_failure_total += 1;
            return Err(error);
        }
        let len = stat.size as usize;
        if out.len() < len {
            let _ = libc::close(fd);
            self.counters.secret_load_failure_total += 1;
            return Err(SecretStoreError::SecretTooLarge);
        }
        let read = read_exact(fd, &mut out[..len]);
        let close = libc::close(fd);
        if read.is_err() || close.is_err() {
            wipe(&mut out[..len]);
            self.counters.secret_load_failure_total += 1;
            return Err(if close.is_err() {
                self.counters.close_failure_total += 1;
                SecretStoreError::CloseFailed
            } else {
                SecretStoreError::ReadFailed
            });
        }
        if !validator(&out[..len]) {
            wipe(&mut out[..len]);
            self.counters.validation_failure_total += 1;
            self.counters.secret_load_failure_total += 1;
            return Err(SecretStoreError::InvalidSecretFormat);
        }
        self.counters.secret_load_success_total += 1;
        Ok(&out[..len])
    }

    /// Remove only clearly-owned stale staging entries for `destination`.
    ///
    /// The destination itself is never touched. Unknown siblings are
    /// preserved, and a stale file is never treated as a candidate secret.
    /// This is best effort because the current ABI has no descriptor-relative
    /// unlink or directory descriptor; each candidate is re-statted before
    /// removal and the kernel still verifies the temp-name grammar.
    pub fn cleanup_stale_temps(
        &mut self,
        options: SecretFileOptions<'_>,
    ) -> Result<usize, SecretStoreError> {
        validate_options(options)?;
        let base = basename(options.destination).ok_or(SecretStoreError::InvalidDestination)?;
        let mut prefix = [0u8; MAX_PATH];
        let prefix_len = PRIVATE_SECRET_DIRECTORY
            .len()
            .checked_add(1)
            .and_then(|len| len.checked_add(base.len()))
            .and_then(|len| len.checked_add(b".tmp.".len()))
            .ok_or(SecretStoreError::InvalidDestination)?;
        if prefix_len >= prefix.len() {
            return Err(SecretStoreError::InvalidDestination);
        }
        prefix[..PRIVATE_SECRET_DIRECTORY.len()].copy_from_slice(PRIVATE_SECRET_DIRECTORY);
        let mut offset = PRIVATE_SECRET_DIRECTORY.len();
        prefix[offset] = b'.';
        offset += 1;
        prefix[offset..offset + base.len()].copy_from_slice(base);
        offset += base.len();
        prefix[offset..prefix_len].copy_from_slice(b".tmp.");

        let mut entries = [crate::DirEntry::zeroed(); 32];
        let count = libc::read_dir(PRIVATE_SECRET_DIRECTORY, &mut entries)
            .map_err(|_| SecretStoreError::DestinationParentMissing)?;
        let mut removed = 0usize;
        for entry in entries.into_iter().take(count) {
            let name = entry.name_bytes();
            if entry.file_type != FT_FILE
                || !name.starts_with(&prefix[PRIVATE_SECRET_DIRECTORY.len()..prefix_len])
            {
                continue;
            }
            let full_len = PRIVATE_SECRET_DIRECTORY
                .len()
                .checked_add(name.len())
                .ok_or(SecretStoreError::InvalidDestination)?;
            if full_len >= MAX_PATH {
                continue;
            }
            let mut full = [0u8; MAX_PATH];
            full[..PRIVATE_SECRET_DIRECTORY.len()].copy_from_slice(PRIVATE_SECRET_DIRECTORY);
            full[PRIVATE_SECRET_DIRECTORY.len()..full_len].copy_from_slice(name);
            let Ok(stat) = libc::stat(&full[..full_len]) else {
                continue;
            };
            if self.validate_stat(stat, options, false).is_err() {
                continue;
            }
            if libc::secret_remove_temp(&full[..full_len]).is_ok() {
                self.counters.temporary_cleanup_total += 1;
                self.counters.stale_temporary_removed_total += 1;
                removed += 1;
            } else {
                self.counters.temporary_cleanup_failure_total += 1;
            }
        }
        Ok(removed)
    }

    fn store(
        &mut self,
        options: SecretFileOptions<'_>,
        secret: &mut [u8],
        validator: SecretValidator,
    ) -> Result<CreateResult, SecretStoreError> {
        validate_options(options)?;
        if secret.is_empty() {
            return Err(SecretStoreError::SecretEmpty);
        }
        if secret.len() > options.maximum_size {
            return Err(SecretStoreError::SecretTooLarge);
        }
        if !validator(secret) {
            self.counters.validation_failure_total += 1;
            return Err(SecretStoreError::InvalidSecretFormat);
        }
        match options.publish_mode {
            SecretPublishMode::CreateIfAbsent => {
                if let Some(existing) = self.existing_status(options, validator)? {
                    if existing {
                        self.counters.secret_create_existing_total += 1;
                        return Ok(CreateResult::Existing);
                    }
                }
            }
            SecretPublishMode::ReplaceExisting => {
                self.validate_existing(options, validator)?;
            }
        }

        let mut temporary = [0u8; MAX_PATH];
        let (fd, temporary_len) = self.create_temp(options.destination, &mut temporary)?;

        let result = self.write_validate_close(fd, secret, options, validator);
        if let Err(error) = result {
            self.cleanup_temp(&temporary[..temporary_len]);
            return Err(error);
        }

        match libc::secret_publish(
            &temporary[..temporary_len],
            options.destination,
            options.mode,
            matches!(options.publish_mode, SecretPublishMode::ReplaceExisting),
        ) {
            Ok(()) => {
                match options.publish_mode {
                    SecretPublishMode::CreateIfAbsent => {
                        self.counters.secret_create_success_total += 1;
                    }
                    SecretPublishMode::ReplaceExisting => {}
                }
                Ok(CreateResult::Created)
            }
            Err(Errno::Again)
                if matches!(options.publish_mode, SecretPublishMode::CreateIfAbsent) =>
            {
                self.cleanup_temp(&temporary[..temporary_len]);
                self.validate_existing(options, validator)?;
                self.counters.secret_create_existing_total += 1;
                self.counters.secret_create_race_lost_total += 1;
                Ok(CreateResult::Existing)
            }
            Err(_) => {
                self.counters.rename_failure_total += 1;
                self.cleanup_temp(&temporary[..temporary_len]);
                Err(SecretStoreError::RenameFailed)
            }
        }
    }

    fn existing_status(
        &mut self,
        options: SecretFileOptions<'_>,
        validator: SecretValidator,
    ) -> Result<Option<bool>, SecretStoreError> {
        let fd = match libc::open(options.destination) {
            Ok(fd) => fd,
            Err(_) => match libc::stat(options.destination) {
                Err(_) => return Ok(None),
                Ok(stat) => {
                    self.validate_stat(stat, options, true)?;
                    return Err(SecretStoreError::ReadFailed);
                }
            },
        };
        let result = self.validate_open_existing(fd, options, validator);
        match result {
            Ok(()) => Ok(Some(true)),
            Err(error) => Err(error),
        }
    }

    fn validate_existing(
        &mut self,
        options: SecretFileOptions<'_>,
        validator: SecretValidator,
    ) -> Result<(), SecretStoreError> {
        let fd =
            libc::open(options.destination).map_err(|_| SecretStoreError::DestinationMissing)?;
        self.validate_open_existing(fd, options, validator)
    }

    fn validate_open_existing(
        &mut self,
        fd: Fd,
        options: SecretFileOptions<'_>,
        validator: SecretValidator,
    ) -> Result<(), SecretStoreError> {
        let result = (|| {
            let stat = libc::fstat(fd).map_err(|_| SecretStoreError::ReadFailed)?;
            self.validate_stat(stat, options, true)?;
            let len = stat.size as usize;
            let mut existing = [0u8; DEFAULT_MAX_SECRET_SIZE];
            let read = read_exact(fd, &mut existing[..len]);
            let valid = read.is_ok() && validator(&existing[..len]);
            wipe(&mut existing[..len]);
            if valid {
                Ok(())
            } else {
                self.counters.validation_failure_total += 1;
                Err(SecretStoreError::InvalidSecretFormat)
            }
        })();
        let close = libc::close(fd);
        if close.is_err() {
            self.counters.close_failure_total += 1;
            return Err(SecretStoreError::CloseFailed);
        }
        result
    }

    fn create_temp(
        &mut self,
        destination: &[u8],
        temporary: &mut [u8; MAX_PATH],
    ) -> Result<(Fd, usize), SecretStoreError> {
        for _ in 0..TEMPORARY_RETRIES {
            let temporary_len = self.create_temp_path(destination, temporary)?;
            match libc::secret_create_temp(&temporary[..temporary_len], DEFAULT_PRIVATE_MODE) {
                Ok(fd) => {
                    self.counters.temporary_create_total += 1;
                    let validation = match libc::fstat(fd) {
                        Ok(stat) => self.validate_temp_stat(stat),
                        Err(_) => Err(SecretStoreError::CreateFailed),
                    };
                    if let Err(error) = validation {
                        let _ = libc::close(fd);
                        self.cleanup_temp(&temporary[..temporary_len]);
                        return Err(error);
                    }
                    return Ok((fd, temporary_len));
                }
                Err(Errno::Again) => {
                    self.counters.temporary_collision_total += 1;
                    continue;
                }
                Err(_) => return Err(SecretStoreError::CreateFailed),
            }
        }
        Err(SecretStoreError::TemporaryNameCollision)
    }

    fn write_validate_close(
        &mut self,
        fd: Fd,
        secret: &[u8],
        options: SecretFileOptions<'_>,
        validator: SecretValidator,
    ) -> Result<(), SecretStoreError> {
        let result = (|| {
            write_exact(fd, secret)?;
            let stat = libc::fstat(fd).map_err(|_| SecretStoreError::WriteFailed)?;
            self.validate_stat(stat, options, false)?;
            if stat.size as usize != secret.len() {
                return Err(SecretStoreError::ShortWrite);
            }
            if !validator(secret) {
                self.counters.validation_failure_total += 1;
                return Err(SecretStoreError::InvalidSecretFormat);
            }
            Ok(())
        })();
        let close = libc::close(fd);
        if close.is_err() {
            self.counters.close_failure_total += 1;
            return Err(SecretStoreError::CloseFailed);
        }
        result
    }

    fn validate_temp_stat(&mut self, stat: Stat) -> Result<(), SecretStoreError> {
        self.validate_stat(
            stat,
            SecretFileOptions {
                destination: PRIVATE_SECRET_DIRECTORY,
                owner_uid: libc::getuid() as u32,
                owner_gid: libc::getgid() as u32,
                mode: DEFAULT_PRIVATE_MODE,
                maximum_size: DEFAULT_MAX_SECRET_SIZE,
                publish_mode: SecretPublishMode::CreateIfAbsent,
                durability: Durability::AtomicVisibility,
            },
            false,
        )
    }

    fn validate_stat(
        &mut self,
        stat: Stat,
        options: SecretFileOptions<'_>,
        require_nonempty: bool,
    ) -> Result<(), SecretStoreError> {
        if stat.file_type != FT_FILE || stat.nlinks != 1 {
            self.counters.unexpected_type_rejection_total += 1;
            return Err(SecretStoreError::UnexpectedTargetType);
        }
        if stat.uid != options.owner_uid || stat.gid != options.owner_gid {
            self.counters.ownership_rejection_total += 1;
            return Err(SecretStoreError::UnexpectedOwner);
        }
        if stat.mode != (0o100_000 | options.mode) {
            self.counters.permission_rejection_total += 1;
            return Err(SecretStoreError::InsecurePermissions);
        }
        let size = stat.size as usize;
        if size > options.maximum_size {
            return Err(SecretStoreError::SecretTooLarge);
        }
        if require_nonempty && size == 0 {
            return Err(SecretStoreError::SecretEmpty);
        }
        Ok(())
    }

    fn create_temp_path(
        &self,
        destination: &[u8],
        out: &mut [u8; MAX_PATH],
    ) -> Result<usize, SecretStoreError> {
        let base = basename(destination).ok_or(SecretStoreError::InvalidDestination)?;
        let prefix_len = PRIVATE_SECRET_DIRECTORY.len();
        let needed = prefix_len
            .checked_add(1)
            .and_then(|len| len.checked_add(base.len()))
            .and_then(|len| len.checked_add(b".tmp.".len()))
            .and_then(|len| len.checked_add(TEMPORARY_TOKEN_HEX_LEN))
            .ok_or(SecretStoreError::InvalidDestination)?;
        if needed >= out.len() {
            return Err(SecretStoreError::InvalidDestination);
        }
        let mut token = [0u8; TEMPORARY_TOKEN_BYTES];
        if libc::getrandom(&mut token, 0) != token.len() as isize {
            wipe(&mut token);
            return Err(SecretStoreError::SecureRandomUnavailable);
        }
        out[..prefix_len].copy_from_slice(PRIVATE_SECRET_DIRECTORY);
        let mut offset = prefix_len;
        out[offset] = b'.';
        offset += 1;
        out[offset..offset + base.len()].copy_from_slice(base);
        offset += base.len();
        out[offset..offset + b".tmp.".len()].copy_from_slice(b".tmp.");
        offset += b".tmp.".len();
        for byte in token {
            out[offset] = hex(byte >> 4);
            out[offset + 1] = hex(byte & 0x0f);
            offset += 2;
        }
        wipe(&mut token);
        Ok(offset)
    }

    fn cleanup_temp(&mut self, temporary: &[u8]) {
        match libc::secret_remove_temp(temporary) {
            Ok(()) => self.counters.temporary_cleanup_total += 1,
            Err(_) => self.counters.temporary_cleanup_failure_total += 1,
        }
    }
}

fn validate_options(options: SecretFileOptions<'_>) -> Result<(), SecretStoreError> {
    if options.mode != DEFAULT_PRIVATE_MODE || options.mode & 0o077 != 0 {
        return Err(SecretStoreError::InsecurePermissions);
    }
    if options.maximum_size == 0 || options.maximum_size > DEFAULT_MAX_SECRET_SIZE {
        return Err(SecretStoreError::SecretTooLarge);
    }
    if options.owner_uid != libc::getuid() as u32 || options.owner_gid != libc::getgid() as u32 {
        return Err(SecretStoreError::PermissionDenied);
    }
    if matches!(options.durability, Durability::RequireDurability) {
        return Err(SecretStoreError::DurabilityUnsupported);
    }
    if !options.destination.starts_with(PRIVATE_SECRET_DIRECTORY) {
        return Err(SecretStoreError::DestinationOutsidePolicy);
    }
    let base = basename(options.destination).ok_or(SecretStoreError::InvalidDestination)?;
    if base.is_empty()
        || base == b"."
        || base == b".."
        || base.starts_with(b".")
        || base.contains(&0)
        || options.destination.len() >= MAX_PATH
        || options.destination[PRIVATE_SECRET_DIRECTORY.len()..].contains(&b'/')
    {
        return Err(SecretStoreError::InvalidDestination);
    }
    Ok(())
}

fn basename(path: &[u8]) -> Option<&[u8]> {
    path.rsplit(|byte| *byte == b'/').next()
}

fn read_exact(fd: Fd, mut out: &mut [u8]) -> Result<(), ()> {
    while !out.is_empty() {
        let read = libc::read(fd, out).map_err(|_| ())?;
        if read == 0 || read > out.len() {
            return Err(());
        }
        out = &mut out[read..];
    }
    Ok(())
}

fn write_exact(fd: Fd, mut bytes: &[u8]) -> Result<(), SecretStoreError> {
    while !bytes.is_empty() {
        let written = libc::write(fd, bytes).map_err(|_| SecretStoreError::WriteFailed)?;
        if written == 0 || written > bytes.len() {
            return Err(SecretStoreError::ShortWrite);
        }
        bytes = &bytes[written..];
    }
    Ok(())
}

fn hex(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'a' + (value - 10),
    }
}

/// Best effort only: it clears this exact caller-owned buffer but cannot
/// promise that compiler temporaries, VFS copies, shared memory, or page cache
/// copies were eliminated.
pub fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        // Volatile prevents the optimizer from dropping this store.
        unsafe {
            core::ptr::write_volatile(byte, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(bytes: &[u8]) -> bool {
        !bytes.is_empty() && bytes[0] == 0xa5
    }

    #[test]
    fn default_options_are_private_and_bounded() {
        let options = SecretFileOptions::system(b"/etc/sunlight/test.key");
        assert_eq!(options.mode, 0o600);
        assert_eq!(options.maximum_size, DEFAULT_MAX_SECRET_SIZE);
        assert_eq!(validate_options(options), Ok(()));
    }

    #[test]
    fn path_policy_rejects_traversal_and_staging_names() {
        assert_eq!(
            validate_options(SecretFileOptions::system(b"/etc/sunlight/../x")),
            Err(SecretStoreError::InvalidDestination)
        );
        assert_eq!(
            validate_options(SecretFileOptions::system(b"/tmp/test.key")),
            Err(SecretStoreError::DestinationOutsidePolicy)
        );
        assert_eq!(
            validate_options(SecretFileOptions::system(b"/etc/sunlight/.key.tmp.x")),
            Err(SecretStoreError::InvalidDestination)
        );
    }

    #[test]
    fn validator_and_wipe_do_not_expose_contents() {
        let mut secret = [0xa5, 0x11, 0x22];
        assert!(valid(&secret));
        wipe(&mut secret);
        assert_eq!(secret, [0; 3]);
    }
}
