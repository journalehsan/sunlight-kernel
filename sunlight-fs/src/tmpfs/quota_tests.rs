use super::*;
use sunlight_mm::testutil;

fn bounded(bytes: usize, inodes: usize) -> Tmpfs {
    Tmpfs::with_limits(&[], TmpfsLimits::new(bytes, inodes)).unwrap()
}

#[test]
fn open_unlink_retains_inode_until_last_close() {
    let baseline = testutil::live_frames();
    let mut fs = bounded(2 * PAGE_SIZE, 4);
    let first = fs
        .create_file_exclusive("/temp", 1000, 1000, 0o600)
        .unwrap();
    let second = fs.open("/temp").unwrap();
    fs.unlink("/temp").unwrap();
    assert_eq!(fs.open("/temp"), Err(FsError::NotFound));
    assert_eq!(fs.lookup("/temp"), Err(FsError::NotFound));
    assert_eq!(fs.stat("/temp"), Err(FsError::NotFound));
    let mut count = 0;
    fs.read_dir("/", &mut |_| {
        count += 1;
        true
    })
    .unwrap();
    assert_eq!(count, 0);
    assert_eq!(fs.write(first, 0, b"retained"), Ok(8));
    let mut out = [0; 8];
    assert_eq!(fs.read(second, 0, &mut out), Ok(8));
    assert_eq!(&out, b"retained");
    assert_eq!(fs.fstat_handle(first).unwrap().nlinks, 0);
    assert_eq!(fs.usage(), (PAGE_SIZE, 2));
    assert_eq!(testutil::live_frames(), baseline + 1);
    // Recreating the name must produce an independent inode.
    let replacement = fs
        .create_file_exclusive("/temp", 1000, 1000, 0o600)
        .unwrap();
    fs.write(replacement, 0, b"new").unwrap();
    fs.close(first).unwrap();
    assert_eq!(fs.usage(), (2 * PAGE_SIZE, 3));
    assert_eq!(fs.read(second, 0, &mut out), Ok(8));
    assert_eq!(&out, b"retained");
    fs.close(second).unwrap();
    assert_eq!(fs.usage(), (PAGE_SIZE, 2));
    assert_eq!(testutil::live_frames(), baseline + 1);
    assert_eq!(fs.read(second, 0, &mut out), Err(FsError::BadHandle));
    assert_eq!(fs.read(replacement, 0, &mut out), Ok(3));
    assert_eq!(&out[..3], b"new");
    fs.unlink("/temp").unwrap();
    fs.close(replacement).unwrap();
    assert_eq!(fs.usage(), (0, 1));
    assert_eq!(testutil::live_frames(), baseline);
}

#[test]
fn unlinked_quota_and_compaction_preserve_other_handles() {
    let baseline = testutil::live_frames();
    let mut fs = bounded(3 * PAGE_SIZE, 4);
    let root = fs.open("/").unwrap();
    let a = fs.create_file_exclusive("/a", 0, 0, 0o600).unwrap();
    let b = fs.create_file_exclusive("/b", 0, 0, 0o600).unwrap();
    let c = fs.create_file_exclusive("/c", 0, 0, 0o600).unwrap();
    fs.unlink("/a").unwrap();
    fs.unlink("/b").unwrap();
    assert_eq!(
        fs.create_file_exclusive("/d", 0, 0, 0o600),
        Err(FsError::NoSpace)
    );
    fs.write(a, 0, b"a").unwrap();
    fs.write(b, 0, b"b").unwrap();
    fs.close(b).unwrap();
    assert_eq!(
        fs.fstat_handle(root).unwrap().file_type,
        FileType::Directory
    );
    assert_eq!(fs.write(c, 0, b"c"), Ok(1));
    let d = fs.create_file_exclusive("/d", 0, 0, 0o600).unwrap();
    assert_eq!(fs.write(d, 0, b"d"), Ok(1));
    assert_eq!(fs.usage(), (3 * PAGE_SIZE, 4));
    fs.close(a).unwrap();
    assert_eq!(
        fs.fstat_handle(root).unwrap().file_type,
        FileType::Directory
    );
    let mut out = [0];
    assert_eq!(fs.read(c, 0, &mut out), Ok(1));
    assert_eq!(&out, b"c");
    assert_eq!(fs.read(d, 0, &mut out), Ok(1));
    assert_eq!(&out, b"d");
    drop(fs);
    assert_eq!(testutil::live_frames(), baseline);
}

#[test]
fn page_quota_failure_preserves_content_and_reclaims_on_truncate() {
    let mut fs = bounded(PAGE_SIZE, 3);
    let h = fs.create_file("/a", 0, 0, mode::FILE_644).unwrap();
    fs.write(h, 0, b"kept").unwrap();
    assert_eq!(fs.write(h, PAGE_SIZE, b"x"), Err(FsError::NoSpace));
    assert_eq!(fs.write(h, usize::MAX, b"x"), Err(FsError::NoSpace));
    assert_eq!(fs.write(h, usize::MAX, b""), Ok(0));
    assert_eq!(fs.usage(), (PAGE_SIZE, 2));
    let mut out = [0; 4];
    assert_eq!(fs.read(h, 0, &mut out), Ok(4));
    assert_eq!(&out, b"kept");
    fs.truncate(h).unwrap();
    assert_eq!(fs.usage(), (0, 2));
    fs.write(h, PAGE_SIZE - 1, b"z").unwrap();
    fs.unlink("/a").unwrap();
    assert_eq!(fs.usage(), (PAGE_SIZE, 2));
    fs.close(h).unwrap();
    assert_eq!(fs.usage(), (0, 1));
}

#[test]
fn inode_quota_counts_directories_and_recovers_after_unlink() {
    let mut fs = bounded(PAGE_SIZE, 3);
    fs.mkdir("/dir", 0, 0, mode::DIR_755).unwrap();
    let h = fs.create_file("/dir/a", 0, 0, mode::FILE_644).unwrap();
    let id = fs.lookup("/dir/a").unwrap();
    fs.close(h).unwrap();
    assert_eq!(fs.mkdir("/b", 0, 0, mode::DIR_755), Err(FsError::NoSpace));
    assert_eq!(
        fs.create_file("/b", 0, 0, mode::FILE_644),
        Err(FsError::NoSpace)
    );
    fs.unlink("/dir/a").unwrap();
    fs.create_file("/b", 0, 0, mode::FILE_644).unwrap();
    assert!(fs.lookup("/b").unwrap() > id);
    assert_eq!(fs.usage(), (0, 3));
}

#[test]
fn handle_exhaustion_does_not_create_an_inode() {
    let mut fs = bounded(PAGE_SIZE, 100);
    for i in 0..RAMFS_MAX_HANDLES {
        fs.create_file(&alloc::format!("/f{i}"), 0, 0, mode::FILE_644)
            .unwrap();
    }
    let before = fs.usage();
    assert_eq!(
        fs.create_file("/extra", 0, 0, mode::FILE_644),
        Err(FsError::TooManyOpenFiles)
    );
    assert_eq!(fs.lookup("/extra"), Err(FsError::NotFound));
    assert_eq!(fs.usage(), before);
}

#[test]
fn partial_frame_failure_rolls_back_pages_quota_and_length() {
    let baseline = testutil::live_frames();
    let mut fs = bounded(4 * PAGE_SIZE, 3);
    let h = fs.create_file("/a", 0, 0, mode::FILE_644).unwrap();
    fs.write(h, 0, b"old").unwrap();
    testutil::fail_after(Some(1));
    let result = fs.write(h, 3 * PAGE_SIZE, b"new");
    testutil::fail_after(None);
    assert_eq!(result, Err(FsError::NoMemory));
    assert_eq!(fs.usage(), (PAGE_SIZE, 2));
    assert_eq!(testutil::live_frames(), baseline + 1);
    assert_eq!(fs.stat("/a").unwrap().size, 3);
    let mut out = [0; 3];
    fs.read(h, 0, &mut out).unwrap();
    assert_eq!(&out, b"old");
    drop(fs);
    assert_eq!(testutil::live_frames(), baseline);
}

#[test]
fn failed_static_materialization_preserves_borrowed_payload() {
    static SEED: &[RamEntry] = &[RamEntry::file("/a", 0, 0, mode::FILE_644, b"original")];
    let mut fs = Tmpfs::with_limits(SEED, TmpfsLimits::new(PAGE_SIZE * 2, 2)).unwrap();
    let h = fs.open("/a").unwrap();
    testutil::fail_after(Some(1));
    let result = fs.write(h, PAGE_SIZE, b"new");
    testutil::fail_after(None);
    assert_eq!(result, Err(FsError::NoMemory));
    assert_eq!(fs.usage(), (0, 2));
    let mut out = [0; 8];
    fs.read(h, 0, &mut out).unwrap();
    assert_eq!(&out, b"original");
    fs.truncate(h).unwrap();
    assert_eq!(fs.usage(), (0, 2));
    assert_eq!(fs.stat("/a").unwrap().size, 0);
}

#[test]
fn shrink_tail_hole_and_regrowth_are_zeroed() {
    let quota = Arc::try_new(TmpfsLimits::new(PAGE_SIZE * 3, 1)).unwrap();
    let mut inode = TmpfsInode::new(1, mode::FILE_644, quota.clone()).unwrap();
    inode
        .write_at(0, &alloc::vec![0xa5; PAGE_SIZE * 2], &quota)
        .unwrap();
    inode.truncate_to(PAGE_SIZE - 1).unwrap();
    assert_eq!(quota.used_bytes(), PAGE_SIZE);
    inode.truncate_to(PAGE_SIZE + 5).unwrap();
    let mut out = [1; 6];
    assert_eq!(inode.read_at(PAGE_SIZE - 1, &mut out), Ok(6));
    assert_eq!(out, [0; 6]);
    inode.truncate_to(0).unwrap();
    inode.reserve_pages(PAGE_SIZE * 2).unwrap();
    inode.write_at(PAGE_SIZE + 1, b"x", &quota).unwrap();
    let mut out = alloc::vec![1; PAGE_SIZE + 2];
    inode.read_at(0, &mut out).unwrap();
    assert!(out[..PAGE_SIZE + 1].iter().all(|b| *b == 0));
    assert_eq!(out[PAGE_SIZE + 1], b'x');
    drop(inode);
    assert_eq!((quota.used_bytes(), quota.used_inodes()), (0, 0));
}

#[test]
fn reserve_is_charged_without_changing_visible_size() {
    let mut fs = bounded(PAGE_SIZE * 2, 2);
    let h = fs.create_file("/a", 0, 0, mode::FILE_644).unwrap();
    fs.reserve(h, PAGE_SIZE + 1).unwrap();
    assert_eq!(fs.usage(), (PAGE_SIZE * 2, 2));
    assert_eq!(fs.stat("/a").unwrap().size, 0);
    assert_eq!(fs.reserve(h, PAGE_SIZE * 3), Err(FsError::NoSpace));
    assert_eq!(fs.usage(), (PAGE_SIZE * 2, 2));
}

#[test]
fn mount_root_limits_and_invalid_paths() {
    assert!(matches!(
        Tmpfs::with_limits(&[], TmpfsLimits::new(0, 0)),
        Err(FsError::NoSpace)
    ));
    let mut fs = bounded(0, 3);
    assert_eq!(fs.stat("/").unwrap().file_type, FileType::Directory);
    assert_eq!(fs.chmod("/", 0o700), Err(FsError::ReadOnlyFilesystem));
    assert_eq!(fs.chown("/", 1, 1), Err(FsError::ReadOnlyFilesystem));
    assert_eq!(fs.rename("/", "/root"), Err(FsError::ReadOnlyFilesystem));
    for path in ["/a//b", "/../a", "/a/", "relative"] {
        assert_eq!(
            fs.create_file(path, 0, 0, mode::FILE_644),
            Err(FsError::InvalidPath)
        );
    }
    let h = fs.create_file("/a", 0, 0, mode::FILE_644).unwrap();
    assert_eq!(fs.write(h, 0, b"x"), Err(FsError::NoSpace));
    assert_eq!(fs.usage(), (0, 2));
    let mut seed = Tmpfs::with_limits(crate::INITRAMFS, TmpfsLimits::new(0, 4096)).unwrap();
    assert_eq!(seed.stat("/").unwrap().file_type, FileType::Directory);
}

#[test]
fn quota_atomic_reservations_never_exceed_limits() {
    let quota = std::sync::Arc::new(TmpfsLimits::new(7 * PAGE_SIZE, 7));
    let winners = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..24 {
            scope.spawn(|| {
                if quota.try_alloc_bytes(PAGE_SIZE).is_ok() {
                    winners.fetch_add(1, Ordering::Relaxed);
                }
                let _ = quota.try_alloc_inode();
            });
        }
    });
    assert_eq!(winners.load(Ordering::Relaxed), 7);
    assert_eq!(quota.used_bytes(), 7 * PAGE_SIZE);
    assert_eq!(quota.used_inodes(), 7);
    assert_eq!(quota.try_alloc_bytes(usize::MAX), Err(FsError::NoSpace));
}

#[test]
fn rename_reclaims_replaced_pages_and_preserves_source_inode() {
    static SEED: &[RamEntry] = &[RamEntry::file("/seed", 0, 0, mode::FILE_644, b"static")];
    let mut fs = Tmpfs::with_limits(SEED, TmpfsLimits::new(3 * PAGE_SIZE, 4)).unwrap();
    let a = fs.create_file("/a", 0, 0, mode::FILE_644).unwrap();
    let b = fs.create_file("/b", 0, 0, mode::FILE_644).unwrap();
    fs.write(a, 0, b"source").unwrap();
    fs.write(b, 0, b"replaced").unwrap();
    let id = fs.lookup("/a").unwrap();
    fs.rename("/a", "/b").unwrap();
    assert_eq!(fs.lookup("/b"), Ok(id));
    assert_eq!(fs.usage(), (PAGE_SIZE, 3));
    let seed = fs.open("/seed").unwrap();
    fs.write(seed, 0, b"overlay").unwrap();
    assert_eq!(fs.usage(), (2 * PAGE_SIZE, 3));
    fs.rename("/b", "/seed").unwrap();
    assert_eq!(fs.lookup("/seed"), Ok(id));
    assert_eq!(fs.usage(), (PAGE_SIZE, 2));
    let mut out = [0; 6];
    assert_eq!(fs.read(a, 0, &mut out), Ok(6));
    assert_eq!(&out, b"source");
}
