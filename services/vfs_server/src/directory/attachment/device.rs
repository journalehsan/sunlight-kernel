//! Host qualification over the lease authority's bounded sector transport.
use super::*;
use crate::device_lease::{
    Completion, DeviceLease, Extent, Lease, Media, Outcome, Rights, Session, Transition,
};
use sunlight_fat::Fat32;
use sunlight_fs::{BlockError, FatFs, BLOCK_SIZE};

pub struct AttachmentDevice(Arc<Binding>);

struct Binding {
    authority: Arc<DeviceLease>,
    session: Session,
    lease: Lease,
    blocks: u64,
}

impl Binding {
    fn sector(&self, lba: u64) -> Result<Extent, BlockError> {
        if lba >= self.blocks {
            return Err(BlockError::OutOfRange);
        }
        Ok(Extent {
            offset: lba * BLOCK_SIZE as u64,
            length: BLOCK_SIZE as u64,
        })
    }

    fn confirm(&self, transition: &Transition) -> Result<(), crate::device_lease::Error> {
        let flush = self.authority.start_flush(transition)?;
        self.authority.complete_flush(&flush, Outcome::Success)
    }

    fn prepare_release(&self) -> Result<Transition, crate::device_lease::Error> {
        let transition = self.authority.begin_release(&self.lease)?;
        self.confirm(&transition)?;
        Ok(transition)
    }

    fn release(&self) -> Result<(), crate::device_lease::Error> {
        self.authority.finish(&self.prepare_release()?)?;
        Ok(())
    }

    fn fence(&self) {
        let _ = self.authority.transport_lost(&self.lease);
    }
}

impl BlockDevice for AttachmentDevice {
    fn read_block(&mut self, lba: u64, bytes: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let binding = &self.0;
        let operation = binding
            .authority
            .read_authenticated(&binding.session, &binding.lease, binding.sector(lba)?)
            .map_err(|_| BlockError::Io)?;
        match binding.authority.complete(&operation, Outcome::Success) {
            Ok(Completion::Read(data)) => {
                *bytes = data;
                Ok(())
            }
            _ => {
                binding.fence();
                Err(BlockError::Uncertain)
            }
        }
    }

    fn write_block(&mut self, lba: u64, bytes: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let binding = &self.0;
        let operation = binding
            .authority
            .write_authenticated(
                &binding.session,
                &binding.lease,
                binding.sector(lba)?,
                *bytes,
            )
            .map_err(|_| BlockError::Io)?;
        match binding.authority.complete(&operation, Outcome::Success) {
            Ok(Completion::Written) => Ok(()),
            _ => {
                binding.fence();
                Err(BlockError::Uncertain)
            }
        }
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let binding = &self.0;
        let transition = binding
            .authority
            .request_flush(&binding.session, &binding.lease)
            .map_err(|_| BlockError::Io)?;
        binding.confirm(&transition).map_err(|_| {
            binding.fence();
            BlockError::Uncertain
        })
    }

    fn block_count(&self) -> u64 {
        self.0.blocks
    }
}

struct Unpublished(Option<Arc<Binding>>);

impl Drop for Unpublished {
    fn drop(&mut self) {
        if let Some(binding) = self.0.take() {
            if binding.release().is_err() {
                binding.fence();
            }
        }
    }
}

pub struct StagedDeviceAttachment {
    reservation: Reservation<AttachmentDevice>,
    backend: FatFs<AttachmentDevice>,
    cleanup: Unpublished,
}

pub struct DeviceAttachment {
    owner: Weak<Mutex<Vfs<AttachmentDevice>>>,
    id: u64,
    binding: Option<Arc<Binding>>,
}

impl DeviceAttachment {
    pub fn id(&self) -> u64 {
        self.id
    }
}

impl Drop for DeviceAttachment {
    fn drop(&mut self) {
        if let Some(binding) = &self.binding {
            binding.fence();
        }
    }
}

impl DirectoryNamespace<AttachmentDevice> {
    pub fn mounted_for_device(filesystem: Vfs<AttachmentDevice>) -> Result<Self, FsError> {
        Self::mount_owner(filesystem)
    }

    pub fn stage_device_attachment(
        &self,
        target: &str,
        authority: Arc<DeviceLease>,
        session: &Session,
        media: &Media,
    ) -> Result<StagedDeviceAttachment, Error> {
        let reservation = self.reserve_attachment(target)?;
        let blocks = authority.attachment_blocks(media).map_err(Error::Lease)?;
        let lease = authority
            .acquire_authenticated(
                media,
                session,
                Extent {
                    offset: 0,
                    length: blocks * BLOCK_SIZE as u64,
                },
                Rights {
                    read: true,
                    write: true,
                    flush: true,
                },
            )
            .map_err(Error::Lease)?;
        let binding = Arc::new(Binding {
            authority,
            session: session.clone(),
            lease,
            blocks,
        });
        let cleanup = Unpublished(Some(binding.clone()));
        let mut fat = Fat32::mount(AttachmentDevice(binding)).ok_or(FsError::Io)?;
        fat.validate_root_chain().map_err(|_| FsError::Io)?;
        let _ = fat.sample_capacity(1024);
        let mut backend = FatFs::new(fat);
        if backend.stat("/")?.file_type != FileType::Directory {
            return Err(FsError::NotDir.into());
        }
        backend.read_dir("/", &mut |_| true)?;
        Ok(StagedDeviceAttachment {
            reservation,
            backend,
            cleanup,
        })
    }

    pub fn publish_device_attachment(
        &self,
        staged: StagedDeviceAttachment,
    ) -> Result<DeviceAttachment, Error> {
        let StagedDeviceAttachment {
            reservation,
            backend,
            mut cleanup,
        } = staged;
        let mut fs = reservation.lock(self)?;
        let binding = cleanup.0.as_ref().unwrap();
        let id = binding
            .authority
            .with_attachment_pin(&binding.session, &binding.lease, || {
                fs.publish_prepared_device(&reservation.target, backend)
            })
            .map_err(Error::Lease)??;
        Ok(DeviceAttachment {
            owner: reservation.owner,
            id,
            binding: cleanup.0.take(),
        })
    }

    pub fn withdraw_device_attachment(&self, attached: &mut DeviceAttachment) -> Result<(), Error> {
        self.withdraw_device_attachment_with(attached, |work| work.synchronize())
    }

    fn withdraw_device_attachment_with(
        &self,
        attached: &mut DeviceAttachment,
        synchronize: impl FnOnce(&mut sunlight_fs::vfs::DeviceWithdrawal<AttachmentDevice>) -> Result<(), FsError>,
    ) -> Result<(), Error> {
        let owner = self.attachment_owner()?;
        if !attached.owner.ptr_eq(&Arc::downgrade(owner)) {
            return Err(Error::WrongOwner);
        }
        let binding = attached.binding.as_ref().ok_or(Error::Stale)?;
        let mut work = owner.lock().prepare_device_withdrawal(attached.id)?;
        let result = (|| {
            synchronize(&mut work)?;
            let transition = binding.prepare_release().map_err(Error::Lease)?;
            let mut fs = owner.lock();
            binding
                .authority
                .finish_attachment_release(&transition, || fs.finish_device_withdrawal(&work))
                .map_err(Error::Lease)??;
            Ok(())
        })();
        if result.is_err() {
            binding.fence();
            return result;
        }
        attached.binding = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
