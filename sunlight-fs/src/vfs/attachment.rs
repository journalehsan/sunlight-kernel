use super::*;
use crate::{check_permission, Credential, PermCheck, Tmpfs};
use alloc::{sync::Arc, vec::Vec};

pub(super) enum DeviceState {
    Active,
    Closing,
    Settling(Arc<()>),
}

impl DeviceState {
    pub(super) fn closed(&self) -> bool {
        !matches!(self, Self::Active)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct AttachmentRow {
    pub slot: u64,
    pub incarnation: u64,
    pub id: u64,
    pub parent_id: u64,
    pub source: u64,
    pub protected: bool,
    pub fat: bool,
    pub mountpoint: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentCapacity {
    /// Full mount ID, including its incarnation, as in AttachmentRow::id.
    pub id: u64,
    pub unit: u32,
    pub blocks: u32,
    pub free: u32,
}

/// Unique work item for one closing mount incarnation.
pub struct DeviceWithdrawal<D: BlockDevice> {
    id: u64,
    transaction: Arc<()>,
    backend: FatFs<D>,
    synchronized: bool,
}

impl<D: BlockDevice> DeviceWithdrawal<D> {
    pub fn device(&self) -> &D {
        self.backend.fat.device()
    }

    pub fn synchronize(&mut self) -> Result<(), FsError> {
        self.synchronized = false;
        self.backend.sync_dir("/")?;
        self.synchronized = true;
        Ok(())
    }
}

fn copy_path(path: &str) -> Result<String, FsError> {
    let mut owned = String::new();
    owned
        .try_reserve_exact(path.len())
        .map_err(|_| FsError::NoMemory)?;
    owned.push_str(path);
    Ok(owned)
}

impl<D: BlockDevice> Vfs<D> {
    /// Resolve and sample capacity under the same exclusive namespace borrow.
    /// Capacity needs a live mount incarnation, so the lookup only needs
    /// existence and search permissions for objects inside that mount.
    pub fn attachment_capacity_beneath(
        &mut self,
        root: FileHandle,
        cwd: FileHandle,
        path: &str,
        caller: Credential,
    ) -> Result<AttachmentCapacity, FsError> {
        let resolved = self.walk_beneath(root, cwd, path, caller, Self::stat)?;
        let stat = self.stat(resolved.as_str())?;
        if path.ends_with('/') && stat.file_type != FileType::Directory {
            return Err(FsError::NotDir);
        }
        let (slot, _) = self.resolve_mount(resolved.as_str())?;
        let mount = self.mounts[slot].as_ref().ok_or(FsError::NotFound)?;
        if mount.device_attachment.is_none() {
            return Err(FsError::Unsupported);
        }
        let id = u64::from(mount.tag) + 1;
        let (unit, blocks, free) = self.fat_capacity(id)?;
        Ok(AttachmentCapacity {
            id,
            unit,
            blocks,
            free,
        })
    }

    pub fn topology_revision(&self) -> u64 {
        self.topology_revision
    }

    pub(super) fn next_topology_revision(&self) -> Result<u64, FsError> {
        self.topology_revision
            .checked_add(1)
            .ok_or(FsError::NoSpace)
    }

    pub(super) fn advance_topology(&mut self) -> Result<(), FsError> {
        self.topology_revision = self.next_topology_revision()?;
        Ok(())
    }

    pub(super) fn check_attachment_ancestor(&self, path: &str) -> Result<(), FsError> {
        if self.mounts.iter().flatten().any(|mount| {
            !mount.protected
                && (matches!(mount.fs, FsNode::Ram(_)) || mount.device_attachment.is_some())
                && path::strip_mount(&mount.path, path).is_some()
        }) {
            return Err(FsError::Busy);
        }
        Ok(())
    }

    pub fn validate_tmpfs_attachment(&mut self, target: &str) -> Result<ObjectMetadata, FsError> {
        self.next_topology_revision()?;
        self.validate_attachment_target(target, None)?;
        let parent = parent_path(target)?;
        let name = target.rsplit('/').next().ok_or(FsError::InvalidPath)?;
        let mut listed = false;
        self.read_dir(parent, &mut |entry| {
            listed =
                entry.name_bytes() == name.as_bytes() && entry.file_type == FileType::Directory;
            !listed
        })?;
        if !listed {
            return Err(FsError::NotFound);
        }
        if !self
            .mounts
            .iter()
            .enumerate()
            .any(|(slot, mount)| mount.is_none() && self.next_tags[slot] <= u16::from(u8::MAX))
        {
            return Err(FsError::TooManyOpenFiles);
        }
        self.lookup_metadata(target)
    }

    pub fn attach_tmpfs(&mut self, target: &str, mut fs: Tmpfs) -> Result<u64, FsError> {
        self.validate_tmpfs_attachment(target)?;
        fs.validate_attachment_backend()?;
        let slot = self
            .mounts
            .iter()
            .enumerate()
            .position(|(slot, mount)| mount.is_none() && self.next_tags[slot] <= u16::from(u8::MAX))
            .ok_or(FsError::TooManyOpenFiles)?;
        let id = u64::from(self.next_tags[slot]) + 1;
        self.mount_with_metadata(target, FsNode::Ram(fs), 0, false)?;
        Ok(id)
    }

    /// Caller holds the namespace reservation and the device admission pin.
    /// Target validation must already have completed; this step performs no I/O.
    pub fn publish_prepared_device(&mut self, target: &str, fs: FatFs<D>) -> Result<u64, FsError> {
        self.publish_prepared_device_with_metadata(target, fs, 0, false)
    }

    pub fn publish_prepared_device_with_metadata(
        &mut self,
        target: &str,
        mut fs: FatFs<D>,
        source: u64,
        protected: bool,
    ) -> Result<u64, FsError> {
        let slot = self
            .mounts
            .iter()
            .enumerate()
            .position(|(slot, mount)| mount.is_none() && self.next_tags[slot] <= u16::from(u8::MAX))
            .ok_or(FsError::TooManyOpenFiles)?;
        let id = u64::from(self.next_tags[slot]) + 1;
        fs.stable_objects = true;
        self.mount_with_metadata(target, FsNode::Fat(fs), source, protected)?;
        self.mounts[slot].as_mut().unwrap().device_attachment = Some(DeviceState::Active);
        Ok(id)
    }

    fn device_attachment_index(&self, id: u64) -> Result<usize, FsError> {
        self.mounts
            .iter()
            .position(|mount| {
                mount.as_ref().is_some_and(|mount| {
                    u64::from(mount.tag) + 1 == id && mount.device_attachment.is_some()
                })
            })
            .ok_or(FsError::NotFound)
    }

    pub fn prepare_device_withdrawal(&mut self, id: u64) -> Result<DeviceWithdrawal<D>, FsError> {
        let index = self.device_attachment_index(id)?;
        if self.mounts[index].as_ref().unwrap().protected {
            return Err(FsError::PermissionDenied);
        }
        if matches!(
            self.mounts[index].as_ref().unwrap().device_attachment,
            Some(DeviceState::Active)
        ) {
            self.topology_revision
                .checked_add(2)
                .ok_or(FsError::NoSpace)?;
            self.advance_topology()?;
            self.mounts[index].as_mut().unwrap().device_attachment = Some(DeviceState::Closing);
        }
        self.next_topology_revision()?;
        let mount = self.mounts[index].as_mut().unwrap();
        if mount.open_handles != 0 {
            return Err(FsError::Busy);
        }
        if !matches!(mount.device_attachment, Some(DeviceState::Closing)) {
            return Err(FsError::Busy);
        }
        let transaction = Arc::try_new(()).map_err(|_| FsError::NoMemory)?;
        let FsNode::Fat(backend) =
            core::mem::replace(&mut mount.fs, FsNode::Ram(RamFs::new(&[])))
        else {
            unreachable!()
        };
        mount.device_attachment = Some(DeviceState::Settling(transaction.clone()));
        Ok(DeviceWithdrawal {
            id,
            transaction,
            backend,
            synchronized: false,
        })
    }

    /// Trusted coordinator commits under a covering device release pin.
    pub fn finish_device_withdrawal(&mut self, work: &DeviceWithdrawal<D>) -> Result<(), FsError> {
        let index = self.device_attachment_index(work.id)?;
        let revision = self.next_topology_revision()?;
        let mount = self.mounts[index].as_ref().unwrap();
        if !matches!(&mount.device_attachment, Some(DeviceState::Settling(transaction))
            if Arc::ptr_eq(transaction, &work.transaction))
        {
            return Err(FsError::BadHandle);
        }
        if !work.synchronized || mount.open_handles != 0 {
            return Err(FsError::Busy);
        }
        self.mounts[index] = None;
        self.count -= 1;
        self.topology_revision = revision;
        Ok(())
    }

    pub fn attachment_rows(
        &mut self,
        root: FileHandle,
        caller: Credential,
    ) -> Result<Vec<AttachmentRow>, FsError> {
        let (root_slot, _) = self.unpack_handle(root)?;
        let root_path = self.lookup_anchor(root)?;
        let root_path = root_path.as_str();
        let metadata = self.stat_beneath(root, root, "/", caller)?;
        if !check_permission(&metadata.stat, &caller, PermCheck::Execute) {
            return Err(FsError::PermissionDenied);
        }
        let mut rows = Vec::new();
        rows.try_reserve_exact(self.count)
            .map_err(|_| FsError::NoMemory)?;
        // Parents precede descendants even after slot reuse.
        let mut order = [0; MAX_MOUNTS];
        let mut count = 0;
        for (slot, mount) in self.mounts.iter().enumerate() {
            if mount.is_some() {
                order[count] = slot;
                count += 1;
            }
        }
        order[..count].sort_unstable_by_key(|&slot| self.mounts[slot].as_ref().unwrap().path.len());
        for &slot in &order[..count] {
            let mount = self.mounts[slot].as_ref().unwrap();
            if mount.device_attachment.as_ref().is_some_and(DeviceState::closed) {
                continue;
            }
            let mountpoint = if slot == root_slot {
                copy_path("/")?
            } else {
                let Some(relative) = path::strip_mount(&mount.path, root_path) else {
                    continue;
                };
                if relative == "/" {
                    return Err(FsError::BadHandle);
                }
                copy_path(relative)?
            };
            let id = u64::from(mount.tag) + 1;
            let incarnation = u64::from(mount.tag) / MAX_MOUNTS as u64 + 1;
            let source = mount.source;
            let protected = mount.protected;
            let fat = matches!(mount.fs, FsNode::Fat(_));
            let supported = matches!(mount.fs, FsNode::Ram(_)) || mount.device_attachment.is_some();
            let stat = match self.stat_beneath(root, root, &mountpoint, caller) {
                Ok(stat) => stat,
                Err(FsError::PermissionDenied) => continue,
                Err(error) => return Err(error),
            };
            if !check_permission(&stat.stat, &caller, PermCheck::Execute) {
                continue;
            }
            if !supported
                || stat.mount_id != slot as u64 + 1
                || stat.mount_generation != incarnation
            {
                return Err(FsError::Unsupported);
            }
            let parent_id = if slot == root_slot {
                id
            } else {
                rows.iter()
                    .rev()
                    .find(|row: &&AttachmentRow| {
                        path::strip_mount(&mountpoint, &row.mountpoint).is_some()
                    })
                    .ok_or(FsError::BadHandle)?
                    .id
            };
            rows.push(AttachmentRow {
                slot: slot as u64 + 1,
                incarnation,
                id,
                parent_id,
                source,
                protected,
                fat,
                mountpoint,
            });
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_revision_exhaustion_preserves_mounts_and_slot_credit() {
        let mut fs = Vfs::<NullDevice>::new();
        fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
        fs.mkdir("/mnt", 0, 0, 0o755).unwrap();
        fs.mkdir("/mnt/a", 0, 0, 0o755).unwrap();
        fs.mkdir("/mnt/b", 0, 0, 0o755).unwrap();
        fs.topology_revision = u64::MAX - 1;
        let id = fs.attach_tmpfs("/mnt/a", Tmpfs::new(&[])).unwrap();
        let tags = fs.next_tags;
        assert_eq!(fs.topology_revision(), u64::MAX);
        assert_eq!(
            fs.attach_tmpfs("/mnt/b", Tmpfs::new(&[])),
            Err(FsError::NoSpace)
        );
        assert_eq!(fs.unmount(id), Err(FsError::NoSpace));
        assert_eq!(fs.next_tags, tags);
        assert_eq!(fs.mount_info(1).unwrap().id, id);
        assert!(fs.mount_info(2).is_none());
        assert_eq!(fs.topology_revision(), u64::MAX);
    }

    #[test]
    fn attachment_backend_pages_return_on_failed_publish_and_withdrawal() {
        let mut fs = Vfs::<NullDevice>::new();
        fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
        fs.mkdir("/mnt", 0, 0, 0o755).unwrap();
        let baseline = sunlight_mm::testutil::live_frames();
        let make = || {
            let mut backend = Tmpfs::new(&[]);
            let handle = backend.create_file("/data", 0, 0, 0o600).unwrap();
            backend.write(handle, 0, b"owned").unwrap();
            backend.close(handle).unwrap();
            assert_eq!(sunlight_mm::testutil::live_frames(), baseline + 1);
            backend
        };
        assert_eq!(
            fs.attach_tmpfs("/mnt/missing", make()),
            Err(FsError::NotFound)
        );
        assert_eq!(sunlight_mm::testutil::live_frames(), baseline);
        let id = fs.attach_tmpfs("/mnt", make()).unwrap();
        let pin = fs.open("/mnt/data").unwrap();
        assert_eq!(fs.unmount(id), Err(FsError::Busy));
        assert_eq!(sunlight_mm::testutil::live_frames(), baseline + 1);
        fs.close(pin).unwrap();
        fs.unmount(id).unwrap();
        assert_eq!(sunlight_mm::testutil::live_frames(), baseline);
    }
}
