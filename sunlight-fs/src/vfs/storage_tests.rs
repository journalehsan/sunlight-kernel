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
fn attached_fat_keeps_renamed_roots_and_descendants() {
    let (mut vfs, disk) = fixture();
    let backend = || FatFs::new(Fat32::mount(Cache::new(disk.clone())).unwrap());
    let id = vfs.publish_prepared_device("/mnt", backend()).unwrap();
    let caller = crate::Credential { uid: 0, gid: 0 };
    let outer = vfs.open("/").unwrap();
    vfs.mkdir("/mnt/TREE", 0, 0, 0o755).unwrap();
    let file = vfs.create_file_exclusive("/mnt/TREE/DATA", 0, 0, 0o644).unwrap();
    let (root, cwd) = vfs.pin_root_beneath(outer, outer, "/mnt/tree", caller).unwrap();
    let before = vfs.stat_beneath(root, cwd, "/DATA", caller).unwrap();
    assert_eq!(vfs.handle_object_id(file).unwrap(), Some(before.object_id - 1));
    vfs.rename("/mnt/tree", "/mnt/Renamed Branch").unwrap();
    vfs.mkdir("/mnt/TREE", 0, 0, 0o755).unwrap();
    assert_eq!(vfs.stat_beneath(root, cwd, "/../../data", caller).unwrap(), before);
    assert_eq!(vfs.rename("/mnt/Renamed Branch", "/mnt/TREE/OTHER"), Err(FsError::Unsupported));
    vfs.write(file, 0, b"retained").unwrap();
    let mut output = [0; 8];
    assert_eq!(vfs.read(file, 0, &mut output), Ok(8));
    assert_eq!(&output, b"retained");
    for handle in [file, root, cwd] {
        vfs.close(handle).unwrap();
    }
    let mut work = vfs.prepare_device_withdrawal(id).unwrap();
    work.synchronize().unwrap();
    vfs.finish_device_withdrawal(&work).unwrap();
    drop(work);
    let remount = vfs.publish_prepared_device("/mnt", backend()).unwrap();
    assert_ne!(remount, id);
    assert_eq!(vfs.read(file, 0, &mut output), Err(FsError::BadHandle));
    let after = vfs.stat_beneath(outer, outer, "/mnt/Renamed Branch/DATA", caller).unwrap();
    assert_ne!(before.mount_generation, after.mount_generation);
    vfs.close(outer).unwrap();
    assert_eq!(vfs.open_handle_count(), 0);
}

#[test]
fn attached_fat_uncertain_rename_cannot_authorize_io() {
    let (_, disk) = fixture();
    let mut fs = FatFs::new(Fat32::mount(disk.clone()).unwrap());
    fs.stable_objects = true;
    let handle = fs.open("/HELLO.TXT").unwrap();
    let identity = fs.handle_object_id(handle).unwrap();
    disk.fail_sync.set(true);
    assert_eq!(fs.rename("/HELLO.TXT", "/MOVED.TXT"), Err(FsError::Io));
    assert_eq!(fs.identities.values().copied().map(Some).collect::<Vec<_>>(), [identity]);
    assert_eq!(fs.stable_id("/MOVED.TXT"), Err(FsError::Io));
    assert_eq!(fs.read(handle, 0, &mut [0; 8]), Err(FsError::BadHandle));
    fs.close(handle).unwrap();
    assert!(fs.handles.iter().all(Option::is_none));
    assert!(fs.fat.stat_path(b"/MOVED.TXT").is_some());
    assert!(fs.fat.stat_path(b"/HELLO.TXT").is_some());
}

#[test]
fn mountinfo_samples_one_fat_incarnation_and_requires_its_source() {
    let (mut vfs, disk) = fixture();
    let root = "1 1 0:0 / / rw - tmpfs tmpfs rw\n";
    mount(&mut vfs, &disk);
    let first = vfs.mount_info(1).unwrap().id;
    let expected = alloc::format!("{first} 1 240:42 / /mnt rw - vfat helios-block:42 rw\n{root}");
    let bytes = vfs.linux_mountinfo(expected.len(), |id| id == 42).unwrap();
    assert_eq!(bytes, expected);
    assert_eq!(
        vfs.linux_mountinfo(8192, |_| false),
        Err(FsError::Unsupported)
    );
    assert_eq!(
        vfs.linux_mountinfo(bytes.len() - 1, |_| true),
        Err(FsError::NoMemory)
    );
    vfs.unmount(first).unwrap();
    assert_eq!(vfs.linux_mountinfo(8192, |_| false).unwrap(), root);
    mount(&mut vfs, &disk);
    let second = vfs.mount_info(1).unwrap().id;
    assert_eq!(second, first + MAX_MOUNTS as u64);
    assert_eq!(vfs.unmount(first), Err(FsError::NotFound));
    assert_eq!(
        vfs.linux_mountinfo(8192, |id| id == 42).unwrap(),
        alloc::format!("{second} 1 240:42 / /mnt rw - vfat helios-block:42 rw\n{root}")
    );
    assert_eq!(bytes, expected);
}

#[test]
fn runtime_publication_keeps_capacity_and_source_bound_to_one_incarnation() {
    let (mut vfs, disk) = fixture();
    let mut fat = Fat32::mount(Cache::new(disk.clone())).unwrap();
    fat.sample_capacity(1024).unwrap();
    let capacity = fat.capacity().unwrap();
    assert_eq!(
        vfs.mount_runtime_fat("/state", 42, fat),
        Err(FsError::OperationNotPermitted)
    );
    assert!(vfs.mount_info(1).is_none());

    let mut fat = Fat32::mount(Cache::new(disk.clone())).unwrap();
    fat.sample_capacity(1024).unwrap();
    vfs.mount_runtime_fat("/mnt", 42, fat).unwrap();
    let published = vfs.mount_info(1).unwrap();
    assert_eq!(published.id, 2); // Failed staging did not consume the slot tag.
    assert_eq!(published.source, 42);
    assert!(!published.protected);
    assert_eq!(published.path, "/mnt");
    let first = published.id;
    let resolved = vfs
        .resolve_existing("/", "/mnt/HELLO.TXT", &crate::Credential { uid: 0, gid: 0 })
        .unwrap();
    assert_eq!(resolved.metadata.mount_id, first);
    assert_eq!(vfs.fat_capacity(first), Ok(capacity));

    disk.fail_sync.set(true);
    assert_eq!(vfs.unmount(first), Err(FsError::Io));
    assert_eq!(vfs.fat_capacity(first), Ok(capacity));
    disk.fail_sync.set(false);
    vfs.unmount(first).unwrap();
    assert!(vfs.mount_info(1).is_none());
    mount(&mut vfs, &disk);
    let second = vfs.mount_info(1).unwrap().id;
    assert_eq!(second, first + MAX_MOUNTS as u64);
    assert_eq!(vfs.fat_capacity(first), Err(FsError::NotFound));
    assert_eq!(vfs.fat_capacity(second), Err(FsError::Unsupported));
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

#[test]
fn canonical_fat_spelling_and_honest_identity_capabilities() {
    let (mut vfs, disk) = fixture();
    mount(&mut vfs, &disk);
    let cred = crate::Credential { uid: 0, gid: 0 };
    let resolved = vfs.resolve_existing("/", "/mnt//hello.txt", &cred).unwrap();
    assert_eq!(resolved.path, "/mnt/HELLO.TXT");
    assert_eq!(resolved.metadata.object_id, None);
    assert_eq!(resolved.metadata.capabilities, BackendCapabilities::FAT);
    assert_eq!(
        vfs.resolve_existing("/", "/mnt/hello.txt/..", &cred),
        Err(FsError::NotDir)
    );
    let stale = vfs.open("/mnt/HELLO.TXT").unwrap();
    vfs.close(stale).unwrap();
    for _ in 0..HANDLE_GENERATION_MAX {
        let live = vfs.open("/mnt/HELLO.TXT").unwrap();
        assert_eq!(vfs.metadata_handle(stale), Err(FsError::BadHandle));
        vfs.close(live).unwrap();
    }
}

#[test]
fn fat_revokes_unlinked_aliases_before_slot_and_cluster_reuse() {
    let (mut vfs, disk) = fixture();
    mount(&mut vfs, &disk);
    let a = vfs.create_file("/mnt/LIVE.TXT", 0, 0, 0o600).unwrap();
    vfs.write(a, 0, b"old").unwrap();
    let alias = vfs.open("/mnt//live.txt").unwrap();
    vfs.unlink("/mnt/live.txt").unwrap();
    let b = vfs.create_file("/mnt/LIVE.TXT", 0, 0, 0o600).unwrap();
    vfs.write(b, 0, b"new").unwrap();
    for stale in [a, alias] {
        assert_eq!(vfs.read(stale, 0, &mut [0; 3]), Err(FsError::BadHandle));
        assert_eq!(vfs.write(stale, 0, b"BAD"), Err(FsError::BadHandle));
        assert_eq!(vfs.truncate(stale), Err(FsError::BadHandle));
        assert_eq!(vfs.fstat_handle(stale), Err(FsError::BadHandle));
        assert_eq!(
            vfs.handle_path(stale, &mut [0; 128]),
            Err(FsError::BadHandle)
        );
        vfs.close(stale).unwrap();
        assert_eq!(vfs.close(stale), Err(FsError::BadHandle));
    }
    let mut bytes = [0; 3];
    assert_eq!(vfs.read(b, 0, &mut bytes), Ok(3));
    assert_eq!(&bytes, b"new");
    assert_eq!(vfs.open_handle_count(), 1);
    vfs.close(b).unwrap();
    assert_eq!(vfs.open_handle_count(), 0);
    let id = vfs.mount_info(1).unwrap().id;
    vfs.unmount(id).unwrap();
}

#[test]
fn fat_rename_replacement_rebinds_source_and_revokes_destination_alias() {
    let (mut vfs, disk) = fixture();
    mount(&mut vfs, &disk);
    let a = vfs.create_file("/mnt/A.TXT", 0, 0, 0o600).unwrap();
    let a2 = vfs.open("/mnt/a.txt").unwrap();
    let b = vfs.create_file("/mnt/B.TXT", 0, 0, 0o600).unwrap();
    vfs.write(a, 0, b"source").unwrap();
    vfs.write(b, 0, b"victim").unwrap();
    vfs.rename("/mnt//a.txt", "/mnt/b.txt").unwrap();
    assert_eq!(vfs.write(b, 0, b"BAD"), Err(FsError::BadHandle));
    assert_eq!(vfs.read(b, 0, &mut [0; 6]), Err(FsError::BadHandle));
    assert_eq!(vfs.fstat_handle(a2).unwrap().size, 6);
    vfs.write(a2, 0, b"SOURCE").unwrap();
    let mut bytes = [0; 6];
    vfs.read(a, 0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"SOURCE");
    vfs.rename("/mnt/B.TXT", "/mnt/b.txt").unwrap(); // alias-to-self must not unlink
    assert_eq!(vfs.stat("/mnt/B.TXT").unwrap().size, 6);
    for h in [a, a2, b] {
        vfs.close(h).unwrap();
    }
}

#[test]
fn fat_truncate_and_extension_refresh_all_open_snapshots() {
    let (mut vfs, disk) = fixture();
    mount(&mut vfs, &disk);
    let a = vfs.create_file("/mnt/A.TXT", 0, 0, 0o600).unwrap();
    let a2 = vfs.open("/mnt/a.txt").unwrap();
    vfs.write(a, 0, b"original").unwrap();
    assert_eq!(vfs.fstat_handle(a2).unwrap().size, 8);
    vfs.truncate(a).unwrap();
    let b = vfs.create_file("/mnt/B.TXT", 0, 0, 0o600).unwrap();
    vfs.write(b, 0, b"private!").unwrap();
    assert_eq!(vfs.read(a2, 0, &mut [0; 8]), Ok(0));
    vfs.write(a2, 0, b"fresh").unwrap();
    let mut bytes = [0; 8];
    vfs.read(b, 0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"private!");
    for h in [a, a2, b] {
        vfs.close(h).unwrap();
    }
}

#[test]
fn fat_partial_retirement_poison_preserves_close_and_mount_ownership() {
    let (mut vfs, disk) = fixture();
    mount(&mut vfs, &disk);
    let a = vfs.create_file("/mnt/A.TXT", 0, 0, 0o600).unwrap();
    vfs.write(a, 0, b"old").unwrap();
    let b = vfs.open("/mnt/HELLO.TXT").unwrap();
    disk.fail_sync.set(true);
    assert_eq!(vfs.unlink("/mnt/A.TXT"), Err(FsError::Io));
    disk.fail_sync.set(false);
    assert_eq!(vfs.write(a, 0, b"BAD"), Err(FsError::BadHandle));
    assert_eq!(vfs.read(b, 0, &mut [0; 8]), Err(FsError::BadHandle));
    assert_eq!(vfs.open("/mnt/HELLO.TXT"), Err(FsError::Io));
    let id = vfs.mount_info(1).unwrap().id;
    assert_eq!(vfs.unmount(id), Err(FsError::Busy));
    vfs.close(a).unwrap();
    vfs.close(b).unwrap();
    assert_eq!(vfs.open_handle_count(), 0);
    // No claim that poisoned disk state is recoverable by remounting.
}
