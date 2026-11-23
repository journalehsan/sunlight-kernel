//! Storage lifecycle is serialized by KERNEL_VFS, including FAT probing and sync.
use sunlight_block::busy_time::BusyTime;
use sunlight_block::partition::MAX_PARTITIONS;
use sunlight_block::registry::{DeviceInfo, DeviceRegistry, FAT32, WRITE};
use sunlight_fs::{FsError, Vfs};

// Bound extra synchronous reads before the mount is visible. Larger
// filesystems and removable USB media do not publish capacity in this tier.
pub const MAX_CAPACITY_FAT_SECTORS: u32 = 1024;

pub static DEVICES: spin::Mutex<DeviceRegistry> = spin::Mutex::new(DeviceRegistry::new());

pub(crate) fn virtio_partition(parent: u64, start: u64, blocks: u64) -> Option<DeviceInfo> {
    DEVICES.lock().find_extent(parent, start, blocks).filter(|info| info.parent == parent)
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
