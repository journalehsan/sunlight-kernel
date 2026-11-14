use super::*;
use alloc::{rc::Rc, vec::Vec};
use core::cell::{Cell, RefCell};
use sunlight_block::{BlockError, CachedBlockDevice, BLOCK_SIZE};
use sunlight_fat::testimg::FatImageBuilder;

#[derive(Clone)]
struct Disk {
    bytes: Rc<RefCell<Vec<u8>>>,
    fail_sync: Rc<Cell<bool>>,
}
impl BlockDevice for Disk {
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let offset = usize::try_from(lba)
            .ok()
            .and_then(|n| n.checked_mul(BLOCK_SIZE))
            .ok_or(BlockError::OutOfRange)?;
        buf.copy_from_slice(
            self.bytes
                .borrow()
                .get(offset..offset + BLOCK_SIZE)
                .ok_or(BlockError::OutOfRange)?,
        );
        Ok(())
    }
    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let offset = usize::try_from(lba)
            .ok()
            .and_then(|n| n.checked_mul(BLOCK_SIZE))
            .ok_or(BlockError::OutOfRange)?;
        self.bytes
            .borrow_mut()
            .get_mut(offset..offset + BLOCK_SIZE)
            .ok_or(BlockError::OutOfRange)?
            .copy_from_slice(buf);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        if self.fail_sync.get() {
            Err(BlockError::Io)
        } else {
            Ok(())
        }
    }
    fn block_count(&self) -> u64 {
        (self.bytes.borrow().len() / BLOCK_SIZE) as u64
    }
}
type Cache = CachedBlockDevice<Disk, 2>;
fn fixture() -> (Vfs<Cache>, Disk) {
    let mut image = FatImageBuilder::new(1);
    image.add_file(image.root(), "HELLO.TXT", b"fixture\n");
    let disk = Disk {
        bytes: Rc::new(RefCell::new(image.build())),
        fail_sync: Rc::new(Cell::new(false)),
    };
    let mut vfs = Vfs::new();
    static ROOT: &[crate::RamEntry] = &[crate::RamEntry::dir("/mnt", 0, 0, mode::DIR_755)];
    vfs.mount_ramfs("/", RamFs::new(ROOT)).unwrap();
    (vfs, disk)
}
fn mount(vfs: &mut Vfs<Cache>, disk: &Disk) {
    vfs.mount_runtime_fat("/mnt", 42, Fat32::mount(Cache::new(disk.clone())).unwrap())
        .unwrap();
}

#[test]
fn rejects_invalid_system_occupied_and_duplicate_targets() {
    let (mut vfs, disk) = fixture();
    for path in ["/", "/state", "/boot", "/etc", "/state/users"] {
        assert_eq!(
            vfs.validate_runtime_target(path, 42),
            Err(FsError::OperationNotPermitted)
        );
    }
    for path in ["mnt", "/mnt/../state", "/mnt/", "/mnt//one"] {
        assert_eq!(
            vfs.validate_runtime_target(path, 42),
            Err(FsError::InvalidPath)
        );
    }
    assert_eq!(
        vfs.validate_runtime_target("/mnt/missing", 42),
        Err(FsError::NotFound)
    );
    assert_eq!(vfs.unmount(999), Err(FsError::NotFound));
    let root = vfs.mount_info(0).unwrap().id;
    assert_eq!(vfs.unmount(root), Err(FsError::OperationNotPermitted));
    let file = vfs.create_file("/mnt/occupied", 0, 0, 0o600).unwrap();
    vfs.close(file).unwrap();
    assert_eq!(
        vfs.validate_runtime_target("/mnt/occupied", 42),
        Err(FsError::NotDir)
    );
    assert_eq!(vfs.validate_runtime_target("/mnt", 42), Err(FsError::Busy));
    vfs.unlink("/mnt/occupied").unwrap();
    static INSECURE: &[crate::RamEntry] =
        &[crate::RamEntry::dir("/mnt", 0, 0, mode::S_IFDIR | 0o777)];
    let mut insecure: Vfs<Cache> = Vfs::new();
    insecure.mount_ramfs("/", RamFs::new(INSECURE)).unwrap();
    assert_eq!(
        insecure.validate_runtime_target("/mnt", 42),
        Err(FsError::InsecureMetadata)
    );
    mount(&mut vfs, &disk);
    assert_eq!(
        vfs.validate_runtime_target("/mnt", 42),
        Err(FsError::AlreadyExists)
    );
    assert_eq!(
        vfs.validate_runtime_target("/mnt", 43),
        Err(FsError::AlreadyExists)
    );
    assert_eq!(
        vfs.validate_runtime_target("/mnt/other", 42),
        Err(FsError::AlreadyExists)
    );
    assert_eq!(
        vfs.validate_runtime_target("/mnt/other", 43),
        Err(FsError::Busy)
    );
}

#[test]
fn busy_close_sync_failure_retry_and_exact_remount() {
    let (mut vfs, disk) = fixture();
    mount(&mut vfs, &disk);
    let id = vfs.mount_info(1).unwrap().id;
    let file = vfs.create_file("/mnt/PROBE.TXT", 0, 0, 0o600).unwrap();
    let content = b"exact durable bytes\0\xff\n";
    assert_eq!(vfs.write(file, 0, content), Ok(content.len()));
    assert_eq!(vfs.unmount(id), Err(FsError::Busy));
    vfs.close(file).unwrap();
    let directory = vfs.open("/mnt").unwrap();
    assert_eq!(vfs.unmount(id), Err(FsError::Busy));
    vfs.close(directory).unwrap();
    disk.fail_sync.set(true);
    assert_eq!(vfs.unmount(id), Err(FsError::Io));
    assert_eq!(vfs.mount_info(1).unwrap().id, id);
    let still_live = vfs.open("/mnt/PROBE.TXT").unwrap();
    let mut bytes = [0; 64];
    let n = vfs.read(still_live, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], content);
    vfs.close(still_live).unwrap();
    disk.fail_sync.set(false);
    vfs.unmount(id).unwrap();
    assert!(vfs.mount_info(1).is_none());
    assert_eq!(vfs.unmount(id), Err(FsError::NotFound));
    mount(&mut vfs, &disk);
    assert_ne!(vfs.mount_info(1).unwrap().id, id);
    let reopened = vfs.open("/mnt/PROBE.TXT").unwrap();
    for stale in [file, directory, still_live] {
        assert_eq!(vfs.read(stale, 0, &mut bytes), Err(FsError::BadHandle));
        assert_eq!(vfs.close(stale), Err(FsError::BadHandle));
    }
    let n = vfs.read(reopened, 0, &mut bytes).unwrap();
    assert_eq!(&bytes[..n], content);
    vfs.close(reopened).unwrap();
    vfs.unmount(vfs.mount_info(1).unwrap().id).unwrap();
}

#[test]
fn exhausted_mount_incarnations_retire_without_aliasing() {
    let (mut vfs, disk) = fixture();
    let mut oldest = None;
    // Seven runtime slots, each with 32 nonwrapping incarnations.
    for _ in 0..224 {
        mount(&mut vfs, &disk);
        let handle = vfs.open("/mnt/HELLO.TXT").unwrap();
        assert_eq!(
            handle.0 & !0x3fff_ffff,
            0,
            "kernel fd kind bits must stay clear"
        );
        if let Some(old) = oldest {
            assert_eq!(vfs.close(old), Err(FsError::BadHandle));
        } else {
            oldest = Some(handle);
        }
        vfs.close(handle).unwrap();
        vfs.unmount(vfs.mount_info(1).unwrap().id).unwrap();
    }
    let fat = Fat32::mount(Cache::new(disk)).unwrap();
    assert_eq!(
        vfs.mount_runtime_fat("/mnt", 42, fat),
        Err(FsError::TooManyOpenFiles)
    );
}
