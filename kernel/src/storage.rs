//! Storage lifecycle is serialized by KERNEL_VFS, including FAT probing and sync.
use sunlight_block::registry::{DeviceRegistry, FAT32, WRITE};
use sunlight_fs::{FsError, Vfs};

pub static DEVICES: spin::Mutex<DeviceRegistry> = spin::Mutex::new(DeviceRegistry::new());

pub fn mount(vfs: &mut Vfs<crate::KernelDisk>, device: u64, target: &str) -> Result<(), FsError> {
    let info = DEVICES.lock().get(device).ok_or(FsError::NotFound)?;
    if info.operations & (FAT32 | WRITE) != FAT32 | WRITE {
        return Err(FsError::Unsupported);
    }
    vfs.validate_runtime_target(target, device)?;
    let parent = if info.parent == 0 {
        info.id
    } else {
        info.parent
    };
    let usb = parent == crate::usb_storage::DEVICE_ID.load(core::sync::atomic::Ordering::Acquire);
    let transport = if usb {
        crate::KernelBlkDev::Usb
    } else if parent == crate::VIRTIO_DEVICE_ID.load(core::sync::atomic::Ordering::Acquire) {
        crate::KernelBlkDev::Virtio
    } else {
        return Err(FsError::NotFound);
    };
    let view = sunlight_block::PartitionBlockDevice::new(transport, info.start, info.blocks)
        .map_err(|_| FsError::Io)?;
    let fat = sunlight_fat::Fat32::mount(crate::KernelDisk::new(view, usb))
        .ok_or(FsError::Unsupported)?;
    vfs.mount_runtime_fat(target, device, fat)
}

/// StorageCtl uses ordinary negative errno values, independent of the older
/// native file syscall error encoding. All records are explicitly serialized.
pub fn error(error: FsError) -> u64 {
    let errno: i64 = match error {
        FsError::PermissionDenied | FsError::OperationNotPermitted | FsError::InsecureMetadata => 1,
        FsError::NotFound => 2,
        FsError::Busy => 16,
        FsError::AlreadyExists => 17,
        FsError::NotDir => 20,
        FsError::InvalidPath => 22,
        FsError::TooManyOpenFiles => 28,
        FsError::Unsupported => 95,
        _ => 5,
    };
    (-errno) as u64
}
