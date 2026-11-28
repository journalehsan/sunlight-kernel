//! Trusted attachment transactions at the existing mounted service owner.
use super::*;
use alloc::{string::String, sync::Weak, vec::Vec};
use sunlight_fs::{
    vfs::{AttachmentCapacity, AttachmentRow, ObjectMetadata},
    Credential,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Filesystem(FsError),
    #[cfg(not(target_os = "none"))]
    Lease(crate::device_lease::Error),
    Stale,
    WrongOwner,
}

impl From<FsError> for Error {
    fn from(error: FsError) -> Self {
        Self::Filesystem(error)
    }
}

/// Private backend and optimistic reservation. Dropping it cancels staging.
pub struct StagedAttachment<D: BlockDevice = NullDevice> {
    reservation: Reservation<D>,
    backend: Tmpfs,
}

struct Reservation<D: BlockDevice> {
    owner: Weak<Mutex<Vfs<D>>>,
    base: u64,
    target: String,
    target_identity: ObjectMetadata,
}

/// Owned by the trusted caller binding. Replacement or retirement drops it.
/// Existing open descriptions do not depend on this view's topology revision.
pub struct CallerView<D: BlockDevice = NullDevice> {
    pair: DirectoryPair<D>,
    revision: u64,
    caller: Credential,
}

pub struct Snapshot<'view, D: BlockDevice = NullDevice> {
    view: &'view CallerView<D>,
    rows: Vec<AttachmentRow>,
}

impl<D: BlockDevice> DirectoryNamespace<D> {
    fn attachment_owner(&self) -> Result<&Arc<Mutex<Vfs<D>>>, Error> {
        match &self.filesystem {
            Backend::Mounted(owner) => Ok(owner),
            Backend::Isolated(_) => Err(FsError::Unsupported.into()),
        }
    }

    pub fn stage_attachment(
        &self,
        target: &str,
        mut backend: Tmpfs,
    ) -> Result<StagedAttachment<D>, Error> {
        backend.validate_attachment_backend()?;
        Ok(StagedAttachment {
            reservation: self.reserve_attachment(target)?,
            backend,
        })
    }

    fn reserve_attachment(&self, target: &str) -> Result<Reservation<D>, Error> {
        let owner = self.attachment_owner()?;
        let (base, target_identity) = {
            let mut fs = owner.lock();
            let identity = fs.validate_tmpfs_attachment(target)?;
            (fs.topology_revision(), identity)
        };
        let mut path = String::new();
        path.try_reserve_exact(target.len())
            .map_err(|_| FsError::NoMemory)?;
        path.push_str(target);
        Ok(Reservation {
            owner: Arc::downgrade(owner),
            base,
            target: path,
            target_identity,
        })
    }

    pub fn publish_attachment(&self, staged: StagedAttachment<D>) -> Result<u64, Error> {
        let mut fs = staged.reservation.lock(self)?;
        Ok(fs.attach_tmpfs(&staged.reservation.target, staged.backend)?)
    }

    pub fn withdraw_attachment(&self, id: u64) -> Result<(), Error> {
        Ok(self.attachment_owner()?.lock().unmount(id)?)
    }

    /// Capture trusted policy paths and credentials. The revision sandwich
    /// rejects intervening topology changes and releases both provisional pins.
    pub fn caller_view(
        &self,
        root: &str,
        directory: &str,
        caller: Credential,
    ) -> Result<CallerView<D>, Error> {
        let owner = self.attachment_owner()?;
        let revision = owner.lock().topology_revision();
        let pair = self.pin(root, directory)?;
        check_revision(&owner.lock(), revision)?;
        Ok(CallerView {
            pair,
            revision,
            caller,
        })
    }
}

impl<D: BlockDevice> Reservation<D> {
    fn lock<'a>(
        &self,
        namespace: &'a DirectoryNamespace<D>,
    ) -> Result<spin::MutexGuard<'a, Vfs<D>>, Error> {
        let owner = namespace.attachment_owner()?;
        if !self.owner.ptr_eq(&Arc::downgrade(owner)) {
            return Err(Error::WrongOwner);
        }
        let mut fs = owner.lock();
        check_revision(&fs, self.base)?;
        if fs.validate_tmpfs_attachment(&self.target)? != self.target_identity {
            return Err(Error::Stale);
        }
        Ok(fs)
    }
}

fn check_revision<D: BlockDevice>(fs: &Vfs<D>, revision: u64) -> Result<(), Error> {
    if fs.topology_revision() == revision {
        Ok(())
    } else {
        Err(Error::Stale)
    }
}

impl<D: BlockDevice> CallerView<D> {
    fn owner(&self) -> &Mutex<Vfs<D>> {
        match &self.pair.filesystem {
            Backend::Mounted(owner) => owner,
            Backend::Isolated(_) => unreachable!("caller views require a mounted VFS"),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn stat(&self, path: &str) -> Result<ObjectMetadata, Error> {
        let mut fs = self.owner().lock();
        check_revision(&fs, self.revision)?;
        Ok(fs.stat_beneath(self.pair.root, self.pair.directory, path, self.caller)?)
    }

    pub fn capacity(&self, path: &str) -> Result<AttachmentCapacity, Error> {
        let mut fs = self.owner().lock();
        check_revision(&fs, self.revision)?;
        Ok(fs.attachment_capacity_beneath(self.pair.root, self.pair.directory, path, self.caller)?)
    }

    pub fn snapshot(&self) -> Result<Snapshot<'_, D>, Error> {
        let mut fs = self.owner().lock();
        check_revision(&fs, self.revision)?;
        let rows = fs.attachment_rows(self.pair.root, self.caller)?;
        Ok(Snapshot { view: self, rows })
    }
}

#[cfg(feature = "vfs-c1e2-test")]
impl<D: BlockDevice> DirectoryPair<D> {
    pub(crate) fn mount_observation(
        &self,
        caller: Credential,
        epoch: u64,
        path: Option<&str>,
    ) -> Result<(u64, u64, Vec<u8>), Error> {
        let Backend::Mounted(owner) = &self.filesystem else {
            return Err(FsError::Unsupported.into());
        };
        let mut fs = owner.lock();
        let revision = fs.topology_revision();
        let id = |local| sunlight_ipc::storage::service_mount_id(epoch, local);
        if let Some(path) = path {
            let capacity = fs.attachment_capacity_beneath(self.root, self.directory, path, caller)?;
            let mount = id(capacity.id).ok_or(FsError::Unsupported)?;
            let mut payload = Vec::new();
            payload.try_reserve_exact(12).map_err(|_| FsError::NoMemory)?;
            for word in [capacity.unit, capacity.blocks, capacity.free] {
                payload.extend_from_slice(&word.to_le_bytes());
            }
            Ok((revision, mount, payload))
        } else {
            let rows = fs.attachment_rows(self.root, caller)?;
            let text = Vfs::<D>::attachment_mountinfo(
                &rows, id, sunlight_ipc::vfs_rpc::file_transport::IO_BYTES,
            )?;
            Ok((revision, 0, text.into_bytes()))
        }
    }
}

impl<D: BlockDevice> Snapshot<'_, D> {
    pub fn revision(&self) -> u64 {
        self.view.revision
    }

    pub fn rows(&self) -> &[AttachmentRow] {
        &self.rows
    }

    /// The consumer must publish inside this bounded, synchronous callback,
    /// while it still owns the caller binding borrowed by the snapshot. The
    /// callback must not reenter the VFS, drop VFS pins, park, or perform IPC.
    pub fn install<T>(self, publish: impl FnOnce(Vec<AttachmentRow>) -> T) -> Result<T, Error> {
        let mut fs = self.view.owner().lock();
        check_revision(&fs, self.view.revision)?;
        if fs.attachment_rows(self.view.pair.root, self.view.caller)? != self.rows {
            return Err(Error::Stale);
        }
        let installed = publish(self.rows);
        drop(fs);
        Ok(installed)
    }
}

#[cfg(test)]
mod tests;

#[cfg(not(target_os = "none"))]
mod device;
#[cfg(not(target_os = "none"))]
pub use device::*;

#[cfg(all(target_os = "none", feature = "vfs-n1o-fat-test"))]
mod native;
#[cfg(all(target_os = "none", feature = "vfs-n1o-fat-test"))]
pub use native::qualify_native_fat;
#[cfg(all(target_os = "none", feature = "vfs-c1e1-test"))]
pub use native::storage_caller;
#[cfg(all(target_os = "none", feature = "vfs-c1e12-test"))]
pub use native::{fat_mapping_byte, FAT_MAPPING_BYTES};
#[cfg(all(target_os = "none", feature = "vfs-c1e1-test", not(feature = "vfs-c1e2-test")))]
pub use native::qualify_storage;
#[cfg(all(target_os = "none", feature = "vfs-c1e2-test"))]
pub use native::{Device as StorageDevice, RetainedStorage};
