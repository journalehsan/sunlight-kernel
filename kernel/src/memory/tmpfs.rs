//! PMM-backed Tmpfs frames. Registration precedes every writable VFS mount.
use core::sync::atomic::{AtomicU64, Ordering};
use sunlight_mm::FrameAllocator;
use x86_64::PhysAddr;

static DIRECT_MAP: AtomicU64 = AtomicU64::new(0);
fn allocate() -> Option<u64> {
    crate::PMM
        .lock()
        .alloc_frame_class(super::accounting::PhysicalMemoryClass::RamFsFileData)
        .map(|address| address.as_u64())
}
unsafe fn free(address: u64) {
    crate::PMM.lock().free_frame(PhysAddr::new(address));
}
unsafe fn direct_map(address: u64) -> *mut u8 {
    (DIRECT_MAP.load(Ordering::Acquire) + address) as *mut u8
}
static PROVIDER: FrameAllocator = FrameAllocator {
    allocate,
    free,
    direct_map,
};
pub fn init(hhdm: u64) {
    DIRECT_MAP.store(hhdm, Ordering::Release);
    // SAFETY: boot supplied the permanent direct map; PMM is synchronized and
    // returns exclusive pages. No callback allocates kernel heap memory.
    unsafe {
        sunlight_mm::install_allocator(&PROVIDER);
    }
}

#[cfg(feature = "tmpfs_test")]
pub fn run_gate() {
    use super::accounting::{class_frame_count, PhysicalMemoryClass};
    use sunlight_fs::{FileSystem, FsError, Tmpfs, TmpfsLimits};
    let class = PhysicalMemoryClass::RamFsFileData;
    let before = class_frame_count(class);
    let free_before = crate::PMM.lock().free_page_count();
    let run = || -> Result<(), FsError> {
        let mut fs = Tmpfs::with_limits(&[], TmpfsLimits::new(8192, 3))?;
        fs.mkdir("/tmp", 0, 0, 0o755)?;
        let file = fs.create_file("/tmp/data", 0, 0, 0o600)?;
        if fs.create_file("/extra", 0, 0, 0o600) != Err(FsError::NoSpace) {
            return Err(FsError::Io);
        }
        fs.write(file, 4094, b"pages!")?;
        let mut data = [1u8; 4100];
        if fs.read(file, 0, &mut data)? != 4100
            || data[..4094].iter().any(|b| *b != 0)
            || &data[4094..] != b"pages!"
            || fs.usage() != (8192, 3)
            || class_frame_count(class) != before + 2
            || crate::PMM.lock().free_page_count() + 2 != free_before
            || fs.write(file, 8192, b"quota") != Err(FsError::NoSpace)
        {
            return Err(FsError::Io);
        }
        fs.truncate(file)?;
        if fs.usage() != (0, 3)
            || class_frame_count(class) != before
            || crate::PMM.lock().free_page_count() != free_before
        {
            return Err(FsError::Io);
        }
        fs.write(file, 0, b"reclaimed")?;
        fs.unlink("/tmp/data")?;
        if fs.usage() != (0, 2)
            || class_frame_count(class) != before
            || crate::PMM.lock().free_page_count() != free_before
        {
            return Err(FsError::Io);
        }
        Ok(())
    };
    if run().is_ok()
        && class_frame_count(class) == before
        && crate::PMM.lock().free_page_count() == free_before
    {
        crate::serial_println!("[TMPFS] native page/quota/zero-fill/reclaim PASS");
    } else {
        crate::serial_println!("[TMPFS] native page/quota/zero-fill/reclaim FAIL");
    }
}
