//! Explicit build-time fixture gate; writes only the selected test volume.
use crate::{KernelDisk, KERNEL_VFS};
use sunlight_fs::{FsError, Vfs};

pub fn run() {
    let result = exercise();
    match result {
        Ok(()) => {
            crate::serial_println!("[STORAGE] PASS");
        }
        Err(e) => {
            crate::serial_println!("[STORAGE] FAIL {:?}", e);
        }
    }
}
fn exercise() -> Result<(), FsError> {
    let mut guard = KERNEL_VFS.lock();
    let vfs = guard.as_mut().ok_or(FsError::NotFound)?;
    let base = if crate::BLOCK_VOLUME_KIND.load(core::sync::atomic::Ordering::Acquire)
        == crate::BLOCK_VOLUME_PARTITION
    {
        "/mnt"
    } else {
        "/boot"
    };
    let path = if base == "/mnt" {
        "/mnt/PROBE.TXT"
    } else {
        "/boot/PROBE.TXT"
    };
    let input = if base == "/mnt" {
        "/mnt/HELLO.TXT"
    } else {
        "/boot/HELLO.TXT"
    };
    let h = vfs.open(input)?;
    let mut buf = [0u8; 64];
    let n = vfs.read(h, 0, &mut buf)?;
    vfs.close(h)?;
    assert_eq!(&buf[..n], b"partition fixture\n");
    crate::serial_println!("[STORAGE] fixture read OK");
    match vfs.open(path) {
        Ok(h) => {
            let n = vfs.read(h, 0, &mut buf)?;
            assert_eq!(&buf[..n], b"guest durable contents\n");
            vfs.close(h)?;
            crate::serial_println!("[STORAGE] restart contents OK");
        }
        Err(FsError::NotFound) => {
            let h = vfs.create_file(path, 0, 0, 0o600)?;
            assert_eq!(vfs.write(h, 0, b"guest durable contents\n")?, 23);
            vfs.sync_file(h)?;
            vfs.close(h)?;
            vfs.sync_dir(base)?;
            crate::serial_println!("[STORAGE] write sync OK");
        }
        Err(e) => return Err(e),
    }
    // Drop the mounted cache and remount the same bounded device in KERNEL_VFS.
    let p = crate::BLOCK_PARTITION.lock().unwrap();
    let disk =
        sunlight_block::PartitionBlockDevice::new(crate::KernelBlkDev::Virtio, p.start, p.blocks).unwrap();
    let fat = sunlight_fat::Fat32::mount(crate::KernelDisk::new(disk, false))
        .ok_or(FsError::NotFound)?;
    let mut remounted: Vfs<KernelDisk> = Vfs::new();
    remounted.mount_ramfs("/", sunlight_fs::RamFs::new(sunlight_fs::INITRAMFS))?;
    remounted.mount_fat(base, fat)?;
    *vfs = remounted;
    let h = vfs.open(path)?;
    let n = vfs.read(h, 0, &mut buf)?;
    assert_eq!(&buf[..n], b"guest durable contents\n");
    vfs.close(h)?;
    vfs.sync_dir(base)?;
    crate::serial_println!("[STORAGE] remount contents OK");
    Ok(())
}
