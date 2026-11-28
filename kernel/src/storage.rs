//! Storage lifecycle is serialized by KERNEL_VFS, including FAT probing and sync.
use sunlight_block::busy_time::BusyTime;
use sunlight_block::partition::MAX_PARTITIONS;
use sunlight_block::registry::{DeviceInfo, DeviceRegistry, FAT32, WRITE};
use sunlight_fs::{FsError, Vfs};

// Bound extra synchronous reads before the mount is visible. Larger
// filesystems and removable USB media do not publish capacity in this tier.
pub const MAX_CAPACITY_FAT_SECTORS: u32 = 1024;

pub static DEVICES: spin::Mutex<DeviceRegistry> = spin::Mutex::new(DeviceRegistry::new());

pub(crate) fn register_virtio_disk(blk: &sunlight_virtio::VirtioBlk) -> Option<u64> {
    use sunlight_block::registry::{FLUSH, READ};

    let operations = READ
        | if blk.supports_write() { WRITE } else { 0 }
        | if blk.supports_flush() { FLUSH } else { 0 };
    DEVICES.lock().register_disk(blk.block_count(), operations)
}

pub(crate) fn discover_virtio_partitions(
    blk: &mut sunlight_virtio::VirtioBlk,
    parent: u64,
) -> Result<sunlight_block::partition::PartitionTable, sunlight_block::partition::TableError> {
    let table = sunlight_block::partition::discover(&mut crate::VirtioBootDisk {
        blk,
        partition: None,
    })?;
    DEVICES.lock().register_partitions(parent, &table)
        .expect("VirtIO partition registry capacity");
    Ok(table)
}

#[cfg(all(feature = "vfs_c1c_test", not(feature = "vfs_c2b_test")))]
pub(crate) fn register_disposable(blk: &mut sunlight_virtio::VirtioBlk) -> u64 {
    use sunlight_block::registry::{FLUSH, READ};

    assert!(blk.supports_write() && blk.supports_flush());
    let mut marker = [0; 512];
    assert!(unsafe { blk.read_block(0, &mut marker) });
    assert!(marker.starts_with(b"VFS-N1O-DISPOSABLE-ONLY!"));
    let table = sunlight_block::partition::discover(&mut crate::VirtioBootDisk {
        blk,
        partition: None,
    })
    .expect("C1C disposable partition table");
    let [partition] = table.partitions() else {
        panic!("C1C requires one disposable FAT32 partition");
    };
    assert!(partition.fat_candidate);
    let mut registry = DEVICES.lock();
    let disk = registry
        .register_disk(blk.block_count(), READ | WRITE | FLUSH)
        .expect("C1C physical registry slot");
    registry
        .register_partitions(disk, &table)
        .expect("C1C partition registry slot");
    registry
        .find_extent(disk, partition.start, partition.blocks)
        .expect("C1C registered FAT32 extent")
        .id
}

pub(crate) fn virtio_partition(parent: u64, start: u64, blocks: u64) -> Option<DeviceInfo> {
    DEVICES.lock().find_extent(parent, start, blocks).filter(|info| info.parent == parent)
}

#[cfg(feature = "vfs_c2b_test")]
pub(crate) fn verify_early_reservation(mut try_mount: impl FnMut(u64) -> Result<(), FsError>) {
    use core::sync::atomic::Ordering;
    use sunlight_block::registry::MAX_DEVICES;

    let boot = crate::VIRTIO_DEVICE_ID.load(Ordering::Acquire);
    assert_ne!(boot, 0);
    assert!(!crate::vfs_n1j_test::reserves_storage_parent(boot));
    let mut fenced = 0;
    for row in (0..MAX_DEVICES).filter_map(|index| DEVICES.lock().enumerate(index)) {
        let parent = if row.parent == 0 { row.id } else { row.parent };
        if crate::vfs_n1j_test::reserves_storage_parent(parent) {
            assert!(matches!(try_mount(row.id), Err(FsError::Busy)));
            fenced += 1;
        }
    }
    assert!(fenced >= 2);
    crate::serial_println!("[VFS-C2B] legacy mount fenced rows={} boot_parent={}", fenced, boot);
}

pub(crate) unsafe fn read_virtio(
    blk: &mut sunlight_virtio::VirtioBlk,
    part: Option<DeviceInfo>,
    retained: bool,
    lba: u64,
    buf: &mut [u8; sunlight_block::BLOCK_SIZE],
) -> Result<(), sunlight_block::BlockError> {
    let Some(part) = part else {
        return blk.read_typed(lba, buf).legacy();
    };
    let mut start = None;
    let mut busy = None;
    let result = blk.read_observed(lba, buf, || {
        busy = retained.then(|| ReadBusy::begin(part, lba));
        start = Some(crate::timekeeping::monotonic_ns());
    }).legacy();
    let end = crate::timekeeping::monotonic_ns();
    drop(busy);
    if let Some(start) = start {
        let bytes = if result.is_ok() { buf.len() as u64 } else { 0 };
        DEVICES.lock().retire_read(part.id, bytes, end.saturating_sub(start));
        if option_env!("SUNLIGHT_BTOP16_TRACE") == Some("1") {
            crate::serial_println!(
                "[BTOP16] read parent={} id={} lba={} start={} end={} ok={} bytes={}",
                part.parent, part.id, lba, start, end, u8::from(result.is_ok()), bytes
            );
            trace_read_totals(part.id);
        }
    }
    result
}

pub(crate) fn trace_read_totals(id: u64) {
    let totals = DEVICES.lock().read_totals(id).expect("registered child");
    crate::serial_println!(
        "[BTOP16] totals id={} completions={} sectors={} ns={}",
        id, totals.completions, totals.sectors_512, totals.elapsed_ns
    );
}

static READ_BUSY: spin::Mutex<[BusyTime; MAX_PARTITIONS]> =
    spin::Mutex::new([BusyTime::new(); MAX_PARTITIONS]);

/// Only the retained VirtIO attachment uses this ledger. Its validated
/// partition table is registered once and its ordinals are never recycled.
pub(crate) struct ReadBusy {
    part: DeviceInfo,
    lba: u64,
    start: u64,
}

impl ReadBusy {
    pub(crate) fn begin(part: DeviceInfo, lba: u64) -> Self {
        let mut ledger = READ_BUSY.lock();
        let start = crate::timekeeping::monotonic_ns();
        ledger[part.partition as usize - 1].begin(start);
        Self { part, lba, start }
    }
}

impl Drop for ReadBusy {
    fn drop(&mut self) {
        let mut ledger = READ_BUSY.lock();
        let end = crate::timekeeping::monotonic_ns();
        let entry = &mut ledger[self.part.partition as usize - 1];
        let active = entry.snapshot(end);
        entry.finish(end);
        let idle = entry.snapshot(end);
        drop(ledger);
        if option_env!("SUNLIGHT_BTOP14_TRACE") == Some("1") {
            use core::sync::atomic::{AtomicUsize, Ordering};
            static EVENTS: AtomicUsize = AtomicUsize::new(0);
            let sequence = EVENTS.fetch_add(1, Ordering::Relaxed) + 1;
            if sequence <= 4096 {
                crate::serial_println!(
                    "[BTOP14] seq={} parent={} partition={} lba={} start={} end={} active={} idle={}",
                    sequence, self.part.parent, self.part.id, self.lba, self.start, end, active, idle
                );
            } else if sequence == 4097 {
                crate::serial_println!("[BTOP14] trace exhausted");
            }
        }
    }
}

pub fn mount(vfs: &mut Vfs<crate::KernelDisk>, device: u64, target: &str) -> Result<(), FsError> {
    let info = DEVICES.lock().get(device).ok_or(FsError::NotFound)?;
    let parent = if info.parent == 0 {
        info.id
    } else {
        info.parent
    };
    #[cfg(feature = "vfs_c1e2_test")]
    if crate::vfs_n1j_test::reserves_storage_parent(parent) {
        return Err(FsError::Busy);
    }
    if info.operations & (FAT32 | WRITE) != FAT32 | WRITE {
        return Err(FsError::Unsupported);
    }
    vfs.validate_runtime_target(target, device)?;
    let usb = parent == crate::usb_storage::DEVICE_ID.load(core::sync::atomic::Ordering::Acquire);
    let transport = if usb {
        crate::KernelBlkDev::Usb
    } else if parent == crate::VIRTIO_DEVICE_ID.load(core::sync::atomic::Ordering::Acquire) {
        crate::KernelBlkDev::Virtio((info.parent != 0).then_some(info))
    } else {
        return Err(FsError::NotFound);
    };
    let view = sunlight_block::PartitionBlockDevice::new(transport, info.start, info.blocks)
        .map_err(|_| FsError::Io)?;
    let mut fat = sunlight_fat::Fat32::mount(crate::KernelDisk::new(view, usb))
        .ok_or(FsError::Unsupported)?;
    if !usb {
        let _ = fat.sample_capacity(MAX_CAPACITY_FAT_SECTORS);
    }
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
