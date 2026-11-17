use super::*;
use crate::Tmpfs;
const OWNER: Credential = Credential { uid: 42, gid: 42 };
const OTHER: Credential = Credential { uid: 43, gid: 43 };

#[test]
fn cwd_reporting_is_root_relative_bounded_and_rejects_escaped_or_hidden_anchors() {
    let (mut fs, root, cwd) = fixture();
    let mut bytes = [0; 32];
    let len = fs.cwd_beneath(root, cwd, &mut bytes).unwrap();
    assert_eq!(&bytes[..len], b"/cwd");
    assert_eq!(
        fs.cwd_beneath(root, cwd, &mut bytes[..3]),
        Err(FsError::InvalidPath)
    );
    let len = fs.cwd_beneath(root, root, &mut bytes).unwrap();
    assert_eq!(&bytes[..len], b"/");
    fs.rename("/jail", "/renamed").unwrap();
    let len = fs.cwd_beneath(root, cwd, &mut bytes).unwrap();
    assert_eq!(&bytes[..len], b"/cwd");
    fs.rename("/renamed/cwd", "/outside").unwrap();
    assert_eq!(
        fs.cwd_beneath(root, cwd, &mut bytes),
        Err(FsError::PermissionDenied)
    );
    fs.rename("/outside", "/renamed/cwd").unwrap();
    fs.mount_tmpfs("/renamed/cwd", Tmpfs::new(&[])).unwrap();
    assert_eq!(
        fs.cwd_beneath(root, cwd, &mut bytes),
        Err(FsError::BadHandle)
    );
    fs.close(root).unwrap();
    fs.close(cwd).unwrap();
    assert_eq!(fs.open_handle_count(), 0);
}

#[test]
fn root_pinning_rejects_files_and_checks_search_before_dotdot() {
    let (mut fs, root, cwd) = fixture();
    let count = fs.open_handle_count();
    assert_eq!(
        fs.pin_root_beneath(root, cwd, "data", OWNER),
        Err(FsError::NotDir)
    );
    fs.chmod("/jail/cwd", 0o700).unwrap();
    for path in [".", "..", "../cwd"] {
        assert_eq!(
            fs.pin_root_beneath(root, cwd, path, OTHER),
            Err(FsError::PermissionDenied)
        );
    }
    assert_eq!(fs.open_handle_count(), count);
    let (new_root, new_cwd) = fs.pin_root_beneath(root, cwd, ".", OWNER).unwrap();
    assert_ne!(new_root, new_cwd);
    let destination = fs.stat_beneath(root, cwd, ".", OWNER).unwrap();
    assert_eq!(
        fs.stat_beneath(new_root, new_cwd, "../../..", OWNER)
            .unwrap(),
        destination
    );
    for handle in [root, cwd, new_root, new_cwd] {
        fs.close(handle).unwrap();
    }
    assert_eq!(fs.open_handle_count(), 0);
}

fn fixture() -> (Vfs, FileHandle, FileHandle) {
    let mut fs = Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    fs.mkdir("/jail", 42, 42, 0o755).unwrap();
    fs.mkdir("/jail/cwd", 42, 42, 0o755).unwrap();
    let h = fs
        .create_file_exclusive("/jail/cwd/data", 42, 42, 0o000)
        .unwrap();
    fs.write(h, 0, b"hello").unwrap();
    fs.close(h).unwrap();
    let root = fs.open("/jail").unwrap();
    let cwd = fs.open("/jail/cwd").unwrap();
    (fs, root, cwd)
}

#[test]
fn relative_absolute_dotdot_and_trailing_separator_use_retained_root() {
    let (mut fs, root, cwd) = fixture();
    let a = fs.stat_beneath(root, cwd, "data", OWNER).unwrap();
    let b = fs.stat_beneath(root, cwd, "/cwd/data", OWNER).unwrap();
    assert_eq!(a, b);
    assert_eq!(a.stat.size, 5); // No read permission needed on the final file.
    assert_eq!(
        fs.stat_beneath(root, cwd, "../../../../", OWNER)
            .unwrap()
            .stat
            .file_type,
        FileType::Directory
    );
    assert_eq!(fs.stat_beneath(root, cwd, "////cwd//./data", OWNER), Ok(a));
    assert_eq!(
        fs.stat_beneath(root, cwd, "data/", OWNER),
        Err(FsError::NotDir)
    );
    assert_eq!(
        fs.stat_beneath(root, cwd, "data/../data", OWNER),
        Err(FsError::NotDir)
    );
    assert_eq!(
        fs.stat_beneath(root, cwd, "missing/../cwd", OWNER),
        Err(FsError::NotFound)
    );
    for path in ["", "bad\0path"] {
        assert_eq!(
            fs.stat_beneath(root, cwd, path, OWNER),
            Err(FsError::InvalidPath)
        );
    }
}

#[test]
fn search_permissions_are_checked_before_dot_and_dotdot_and_for_captured_gid() {
    let (mut fs, root, cwd) = fixture();
    fs.chmod("/jail/cwd", 0o710).unwrap();
    let group = Credential { uid: 43, gid: 42 };
    assert!(fs.stat_beneath(root, cwd, "data", group).is_ok());
    assert!(fs.stat_beneath(root, cwd, "/cwd", OTHER).is_ok());
    for path in ["data", ".", "..", "/cwd/../cwd/data"] {
        assert_eq!(
            fs.stat_beneath(root, cwd, path, OTHER),
            Err(FsError::PermissionDenied)
        );
    }
    fs.chmod("/jail/cwd", 0).unwrap();
    assert!(fs
        .stat_beneath(root, cwd, "data", Credential { uid: 0, gid: 0 })
        .is_ok());
}

#[test]
fn anchors_follow_rename_and_refuse_cwd_moved_outside_root() {
    let (mut fs, root, cwd) = fixture();
    let original = fs.stat_beneath(root, cwd, "data", OWNER).unwrap();
    fs.rename("/jail/cwd", "/jail/moved").unwrap();
    assert_eq!(fs.stat_beneath(root, cwd, "data", OWNER), Ok(original));
    fs.rename("/jail", "/renamed").unwrap();
    assert_eq!(
        fs.stat_beneath(root, cwd, "/moved/data", OWNER),
        Ok(original)
    );
    fs.rename("/renamed/moved", "/outside").unwrap();
    assert_eq!(
        fs.stat_beneath(root, cwd, "data", OWNER),
        Err(FsError::PermissionDenied)
    );
    assert_eq!(
        fs.stat_beneath(root, cwd, "/", OWNER)
            .unwrap()
            .stat
            .file_type,
        FileType::Directory
    );
}

#[test]
fn crossing_mounts_preserves_distinct_identity_and_dotdot_returns_to_parent() {
    let (mut fs, root, cwd) = fixture();
    fs.mount_tmpfs("/jail/mount", Tmpfs::new(&[])).unwrap();
    fs.mkdir("/jail/mount/sub", 42, 42, 0o755).unwrap();
    let first = fs.stat_beneath(root, cwd, "/", OWNER).unwrap();
    let mounted = fs.stat_beneath(root, cwd, "/mount", OWNER).unwrap();
    assert_ne!(
        (first.mount_id, first.mount_generation),
        (mounted.mount_id, mounted.mount_generation)
    );
    assert_eq!(
        fs.stat_beneath(root, cwd, "/mount/sub/../..", OWNER),
        Ok(first)
    );
    // Overmounting a retained cwd must fail rather than redirecting its pin.
    fs.mount_tmpfs("/jail/cwd", Tmpfs::new(&[])).unwrap();
    assert_eq!(
        fs.stat_beneath(root, cwd, "data", OWNER),
        Err(FsError::BadHandle)
    );
}

#[test]
fn identities_survive_writes_and_rename_but_never_alias_recreated_objects_or_mounts() {
    let (mut fs, root, cwd) = fixture();
    let old = fs.stat_beneath(root, cwd, "data", OWNER).unwrap();
    let h = fs.open("/jail/cwd/data").unwrap();
    fs.write(h, 5, b"!").unwrap();
    fs.close(h).unwrap();
    fs.rename("/jail/cwd/data", "/jail/cwd/other").unwrap();
    let renamed = fs.stat_beneath(root, cwd, "other", OWNER).unwrap();
    assert_eq!(
        (old.object_id, old.object_generation),
        (renamed.object_id, renamed.object_generation)
    );
    fs.unlink("/jail/cwd/other").unwrap();
    let h = fs
        .create_file_exclusive("/jail/cwd/other", 42, 42, 0o600)
        .unwrap();
    fs.close(h).unwrap();
    assert_ne!(
        fs.stat_beneath(root, cwd, "other", OWNER)
            .unwrap()
            .object_id,
        old.object_id
    );
    fs.mount_tmpfs("/jail/mount", Tmpfs::new(&[])).unwrap();
    let old_mount = fs.stat_beneath(root, cwd, "/mount", OWNER).unwrap();
    // Simulate administrative mount retirement; ordinary tmpfs mounts are
    // protected and do not implement the runtime FAT unmount barrier.
    fs.mounts[(old_mount.mount_id - 1) as usize] = None;
    fs.count -= 1;
    fs.mount_tmpfs("/jail/mount", Tmpfs::new(&[])).unwrap();
    let new_mount = fs.stat_beneath(root, cwd, "/mount", OWNER).unwrap();
    assert_ne!(
        (new_mount.mount_id, new_mount.mount_generation),
        (old_mount.mount_id, old_mount.mount_generation)
    );
}

#[test]
fn unknown_identity_backend_refuses_lookup() {
    use sunlight_block::MemDisk;
    use sunlight_fat::testimg::FatImageBuilder;
    let mut fs: Vfs<MemDisk> = Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    let root = fs.open("/").unwrap();
    let mut image = FatImageBuilder::new(1);
    image.add_file(image.root(), "DATA.TXT", b"data");
    let mut bytes = image.build();
    let disk = MemDisk::new(&mut bytes);
    let fat = sunlight_fat::Fat32::mount(disk).unwrap();
    fs.mount_fat("/fat", fat).unwrap();
    assert_eq!(
        fs.stat_beneath(root, root, "/fat/DATA.TXT", OWNER),
        Err(FsError::Unsupported)
    );
}

#[test]
fn directory_transition_checks_final_search_and_rolls_back_partial_opens() {
    let (mut fs, root, cwd) = fixture();
    fs.mkdir("/jail/private", OWNER.uid, OWNER.gid, 0o700)
        .unwrap();
    let count = fs.open_handle_count();
    for (path, caller, error) in [
        ("/private", OTHER, FsError::PermissionDenied),
        ("data", OWNER, FsError::NotDir),
        ("missing", OWNER, FsError::NotFound),
        ("", OWNER, FsError::InvalidPath),
    ] {
        assert_eq!(
            fs.pin_directory_beneath(root, cwd, path, caller),
            Err(error)
        );
        assert_eq!(fs.open_handle_count(), count);
    }
    // Exhaust backend handles, leaving only one free slot for the first open.
    let mut held = alloc::vec::Vec::new();
    while let Ok(handle) = fs.open("/jail") {
        held.push(handle);
    }
    fs.close(held.pop().unwrap()).unwrap();
    let count = fs.open_handle_count();
    for _ in 0..3 {
        assert_eq!(
            fs.pin_directory_beneath(root, cwd, ".", OWNER),
            Err(FsError::TooManyOpenFiles)
        );
        assert_eq!(fs.open_handle_count(), count);
    }
    for handle in held {
        fs.close(handle).unwrap();
    }
    let (new_root, new_cwd) = fs
        .pin_directory_beneath(root, cwd, "../../private", OWNER)
        .unwrap();
    assert_eq!(
        fs.stat_beneath(new_root, new_cwd, ".", OWNER)
            .unwrap()
            .stat
            .mode
            & 0o777,
        0o700
    );
    fs.close(new_root).unwrap();
    fs.close(new_cwd).unwrap();
    assert_eq!(fs.open_handle_count(), 2);
}

#[test]
fn regular_open_checks_final_access_and_directory_search_without_leaking_handles() {
    let (mut fs, root, cwd) = fixture();
    let count = fs.open_handle_count();
    assert_eq!(
        fs.open_file_beneath(root, cwd, "data", OWNER, true, false),
        Err(FsError::PermissionDenied)
    );
    assert_eq!(
        fs.open_file_beneath(root, cwd, "data", OWNER, false, false),
        Err(FsError::InvalidPath)
    );
    assert_eq!(
        fs.open_file_beneath(root, cwd, ".", OWNER, true, false),
        Err(FsError::IsDir)
    );
    fs.chmod("/jail/cwd/data", 0o400).unwrap();
    assert_eq!(
        fs.open_file_beneath(root, cwd, "data", OWNER, true, true),
        Err(FsError::PermissionDenied)
    );
    let handle = fs
        .open_file_beneath(root, cwd, "data", OWNER, true, false)
        .unwrap();
    fs.close(handle).unwrap();
    fs.chmod("/jail/cwd", 0o000).unwrap();
    assert_eq!(
        fs.open_file_beneath(root, cwd, "../cwd/data", OWNER, true, false),
        Err(FsError::PermissionDenied)
    );
    assert_eq!(fs.open_handle_count(), count);
}

#[test]
fn regular_open_is_confined_after_anchor_rename_and_keeps_unlinked_inode() {
    let (mut fs, root, cwd) = fixture();
    fs.chmod("/jail/cwd/data", 0o600).unwrap();
    fs.rename("/jail", "/moved").unwrap();
    let handle = fs
        .open_file_beneath(root, cwd, "/../../cwd/data", OWNER, true, true)
        .unwrap();
    fs.unlink("/moved/cwd/data").unwrap();
    let mut bytes = [0; 5];
    assert_eq!(fs.read(handle, 0, &mut bytes), Ok(5));
    assert_eq!(&bytes, b"hello");
    fs.close(handle).unwrap();
    assert_eq!(
        fs.open_file_beneath(root, cwd, "data", OWNER, true, false),
        Err(FsError::NotFound)
    );
}
