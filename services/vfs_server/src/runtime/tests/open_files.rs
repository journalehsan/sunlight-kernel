use super::*;
use crate::open::{Access, Seek};
use sunlight_fs::Credential;
const OWNER: Credential = Credential { uid: 42, gid: 43 };

fn fixture<const N: usize>() -> (Runtime<2, N, 2>, DirectoryNamespace, Receipt) {
    let mut fs = sunlight_fs::Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    fs.mkdir("/jail", 42, 43, 0o755).unwrap();
    fs.mkdir("/jail/cwd", 42, 43, 0o755).unwrap();
    let h = fs
        .create_file_exclusive("/jail/cwd/data", 42, 43, 0o600)
        .unwrap();
    fs.write(h, 0, b"abcdef").unwrap();
    fs.close(h).unwrap();
    let namespace = DirectoryNamespace::mounted(fs).unwrap();
    let mut runtime = Runtime::new();
    runtime.bind(registration()).unwrap();
    runtime
        .install(registration(), scope(), &namespace, "/jail", "/jail/cwd")
        .unwrap();
    let receipt = acquire(&mut runtime, registration(), scope(), 1);
    (runtime, namespace, receipt)
}
fn open<const N: usize>(runtime: &mut Runtime<2, N, 2>, receipt: Receipt, access: Access) -> u64 {
    runtime
        .open_file(registration(), receipt, "data", OWNER, access, || true)
        .unwrap()
}

#[test]
fn duplicates_share_offsets_and_unlinked_inode_until_final_close() {
    let (mut rt, ns, receipt) = fixture::<4>();
    let r = registration();
    let c = scope().caller;
    let baseline = ns.shared().lock().open_handle_count();
    let a = open(&mut rt, receipt, Access::ReadWrite);
    let b = rt.duplicate_file(r, c, a).unwrap();
    assert_ne!(a, b);
    assert_eq!(ns.shared().lock().open_handle_count(), baseline + 1);
    let mut bytes = [0; 2];
    assert_eq!(rt.read_file(r, c, a, &mut bytes), Ok(2));
    assert_eq!(&bytes, b"ab");
    assert_eq!(rt.read_file(r, c, b, &mut bytes), Ok(2));
    assert_eq!(&bytes, b"cd");
    ns.shared().lock().unlink("/jail/cwd/data").unwrap();
    let h = ns
        .shared()
        .lock()
        .create_file_exclusive("/jail/cwd/data", 42, 43, 0o600)
        .unwrap();
    ns.shared().lock().write(h, 0, b"replacement").unwrap();
    ns.shared().lock().close(h).unwrap();
    assert_eq!(rt.stat_file(r, c, a).unwrap().size, 6);
    assert_eq!(rt.seek_file(r, c, b, Seek::End(-2)), Ok(4));
    assert_eq!(rt.write_file(r, c, a, b"XY"), Ok(2));
    assert_eq!(rt.seek_file(r, c, b, Seek::Current(0)), Ok(6));
    rt.close_file(r, c, a).unwrap();
    assert_eq!(
        rt.read_file(r, c, a, &mut bytes),
        Err(Error::Filesystem(FsError::BadHandle))
    );
    assert_eq!(ns.shared().lock().open_handle_count(), baseline + 1);
    rt.seek_file(r, c, b, Seek::Start(4)).unwrap();
    rt.read_file(r, c, b, &mut bytes).unwrap();
    assert_eq!(&bytes, b"XY");
    rt.truncate_file(r, c, b).unwrap();
    assert_eq!(rt.stat_file(r, c, b).unwrap().size, 0);
    assert_eq!(rt.seek_file(r, c, b, Seek::Current(0)), Ok(6));
    rt.close_file(r, c, b).unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), baseline);
}

#[test]
fn rights_and_identity_are_checked_on_every_descriptor_call() {
    let (mut rt, ns, receipt) = fixture::<4>();
    let r = registration();
    let c = scope().caller;
    let read = open(&mut rt, receipt, Access::Read);
    let write = open(&mut rt, receipt, Access::Write);
    assert_eq!(
        rt.write_file(r, c, read, b"x"),
        Err(Error::Filesystem(FsError::PermissionDenied))
    );
    assert_eq!(
        rt.truncate_file(r, c, read),
        Err(Error::Filesystem(FsError::PermissionDenied))
    );
    assert_eq!(
        rt.read_file(r, c, write, &mut [0]),
        Err(Error::Filesystem(FsError::PermissionDenied))
    );
    for wrong in [
        Caller {
            pid: c.pid + 1,
            ..c
        },
        Caller {
            incarnation: c.incarnation + 1,
            ..c
        },
        Caller {
            address_space_generation: c.address_space_generation + 1,
            ..c
        },
    ] {
        assert_eq!(
            rt.read_file(r, wrong, read, &mut [0]),
            Err(Error::Filesystem(FsError::BadHandle))
        );
        assert_eq!(
            rt.write_file(r, wrong, write, b"x"),
            Err(Error::Filesystem(FsError::BadHandle))
        );
        assert_eq!(
            rt.seek_file(r, wrong, read, Seek::Start(0)),
            Err(Error::Filesystem(FsError::BadHandle))
        );
        assert_eq!(
            rt.stat_file(r, wrong, read),
            Err(Error::Filesystem(FsError::BadHandle))
        );
        assert_eq!(
            rt.truncate_file(r, wrong, write),
            Err(Error::Filesystem(FsError::BadHandle))
        );
        assert_eq!(
            rt.duplicate_file(r, wrong, read),
            Err(Error::Filesystem(FsError::BadHandle))
        );
        assert_eq!(
            rt.close_file(r, wrong, read),
            Err(Error::Filesystem(FsError::BadHandle))
        );
        rt.retire_caller(wrong);
        rt.retire_file_owner(wrong);
    }
    let mut wrong = r;
    wrong.epoch += 1;
    assert_eq!(rt.stat_file(wrong, c, read), Err(Error::Identity));
    ns.shared().lock().chmod("/jail/cwd/data", 0).unwrap();
    assert_eq!(
        rt.open_file(r, receipt, "data", OWNER, Access::Read, || true),
        Err(Error::Filesystem(FsError::PermissionDenied))
    );
    assert_eq!(rt.read_file(r, c, read, &mut [0]), Ok(1));
    assert_eq!(
        rt.seek_file(r, c, read, Seek::Current(-2)),
        Err(Error::Filesystem(FsError::InvalidPath))
    );
    assert_eq!(rt.seek_file(r, c, read, Seek::Current(0)), Ok(1));
}

#[test]
fn refused_publication_capacity_and_replay_do_not_leak_or_reuse_tokens() {
    let (mut rt, ns, receipt) = fixture::<1>();
    let r = registration();
    let c = scope().caller;
    let baseline = ns.shared().lock().open_handle_count();
    assert_eq!(
        rt.open_file(r, receipt, "data", OWNER, Access::Read, || false),
        Err(Error::Identity)
    );
    assert_eq!(ns.shared().lock().open_handle_count(), baseline);
    let token = open(&mut rt, receipt, Access::Read);
    assert_eq!(
        rt.open_file(r, receipt, "data", OWNER, Access::Read, || true),
        Err(Error::Filesystem(FsError::TooManyOpenFiles))
    );
    assert_eq!(
        rt.duplicate_file(r, c, token),
        Err(Error::Filesystem(FsError::TooManyOpenFiles))
    );
    assert_eq!(ns.shared().lock().open_handle_count(), baseline + 1);
    rt.close_file(r, c, token).unwrap();
    let next = open(&mut rt, receipt, Access::Read);
    assert!(next > token);
    assert_eq!(
        rt.close_file(r, c, token),
        Err(Error::Filesystem(FsError::BadHandle))
    );
    rt.close_file(r, c, next).unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), baseline);
}

#[test]
fn withdrawal_preserves_open_files_but_caller_and_registration_retirement_close_them() {
    let (mut rt, ns, receipt) = fixture::<4>();
    let r = registration();
    let c = scope().caller;
    let a = open(&mut rt, receipt, Access::Read);
    rt.duplicate_file(r, c, a).unwrap();
    rt.withdraw(r, scope()).unwrap();
    assert_eq!(
        rt.open_file(r, receipt, "data", OWNER, Access::Read, || true),
        Err(Error::Scope)
    );
    assert_eq!(rt.read_file(r, c, a, &mut [0]), Ok(1));
    rt.retire_caller(c); // Metadata/credential withdrawal preserves existing rights.
    assert_eq!(rt.read_file(r, c, a, &mut [0]), Ok(1));
    rt.retire_file_owner(c);
    rt.retire_file_owner(c);
    assert_eq!(
        rt.stat_file(r, c, a),
        Err(Error::Filesystem(FsError::BadHandle))
    );
    // Lease still owns its two anchors until release, independently of files.
    assert_eq!(ns.shared().lock().open_handle_count(), 2);
    rt.request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), 0);

    let (mut rt, ns, receipt) = fixture::<4>();
    let a = open(&mut rt, receipt, Access::Read);
    let mut next = r;
    next.epoch += 1;
    rt.observe_active(next).unwrap();
    assert_eq!(rt.stat_file(r, c, a), Err(Error::Identity));
    assert_eq!(
        rt.stat_file(next, c, a),
        Err(Error::Filesystem(FsError::BadHandle))
    );
    assert_eq!(ns.shared().lock().open_handle_count(), 2);
    rt.request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), 0);
}

#[test]
fn open_uses_retained_objects_after_rename_and_rejects_nonregular_or_unsearchable_paths() {
    let (mut rt, ns, receipt) = fixture::<4>();
    let r = registration();
    ns.shared().lock().rename("/jail", "/moved").unwrap();
    ns.shared().lock().mkdir("/jail", 42, 43, 0o755).unwrap();
    ns.shared()
        .lock()
        .mkdir("/jail/cwd", 42, 43, 0o755)
        .unwrap();
    let token = open(&mut rt, receipt, Access::Read);
    assert_eq!(rt.stat_file(r, scope().caller, token).unwrap().size, 6);
    assert!(rt
        .open_file(r, receipt, "/../cwd/data", OWNER, Access::Read, || true)
        .is_ok());
    assert_eq!(
        rt.open_file(r, receipt, "..", OWNER, Access::Read, || true),
        Err(Error::Filesystem(FsError::IsDir))
    );
    ns.shared().lock().chmod("/moved/cwd", 0o700).unwrap();
    assert_eq!(
        rt.open_file(
            r,
            receipt,
            "../cwd/data",
            Credential { uid: 99, gid: 99 },
            Access::Read,
            || true
        ),
        Err(Error::Filesystem(FsError::PermissionDenied))
    );
    assert_eq!(
        rt.open_file(r, receipt, "data/", OWNER, Access::Read, || true),
        Err(Error::Filesystem(FsError::NotDir))
    );
    ns.shared()
        .lock()
        .mount_tmpfs("/moved/cwd", Tmpfs::new(&[]))
        .unwrap();
    assert_eq!(
        rt.open_file(r, receipt, "data", OWNER, Access::Read, || true),
        Err(Error::Filesystem(FsError::BadHandle))
    );
}

#[test]
fn separate_opens_have_independent_offsets_and_runtime_drop_closes_every_description() {
    let (mut rt, ns, receipt) = fixture::<4>();
    let r = registration();
    let c = scope().caller;
    let a = open(&mut rt, receipt, Access::ReadWrite);
    let b = open(&mut rt, receipt, Access::ReadWrite);
    let dup = rt.duplicate_file(r, c, a).unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), 4);
    rt.read_file(r, c, a, &mut [0; 2]).unwrap();
    assert_eq!(rt.seek_file(r, c, dup, Seek::Current(0)), Ok(2));
    assert_eq!(rt.seek_file(r, c, b, Seek::Current(0)), Ok(0));
    rt.seek_file(r, c, a, Seek::Start(u64::MAX)).unwrap();
    assert_eq!(
        rt.read_file(r, c, dup, &mut [0]),
        Err(Error::Filesystem(FsError::InvalidPath))
    );
    assert_eq!(
        rt.write_file(r, c, dup, b"x"),
        Err(Error::Filesystem(FsError::InvalidPath))
    );
    drop(rt);
    assert_eq!(ns.shared().lock().open_handle_count(), 0);
}

#[test]
fn released_or_foreign_lease_cannot_publish_a_description() {
    let (mut rt, ns, receipt) = fixture::<4>();
    let r = registration();
    let mut foreign = r;
    foreign.epoch += 1;
    assert_eq!(
        rt.open_file(foreign, receipt, "data", OWNER, Access::Read, || true),
        Err(Error::Identity)
    );
    rt.request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(
        rt.open_file(r, receipt, "data", OWNER, Access::Read, || true),
        Err(Error::Scope)
    );
    assert_eq!(ns.shared().lock().open_handle_count(), 2);
}

#[test]
fn copied_file_owner_barrier_authenticates_origin_and_preserves_other_lifetimes() {
    let (mut rt, ns, receipt) = fixture::<4>();
    let r = registration();
    let c = scope().caller;
    let a = open(&mut rt, receipt, Access::Read);
    rt.duplicate_file(r, c, a).unwrap();
    let event = wire::Frame::FileOwnerRetired {
        registration: r,
        caller: c,
        event_id: 7,
    };
    let bytes = event.encode().unwrap();
    assert!(rt.request(r, &bytes).is_err());
    for field in 0..5 {
        let mut wrong = r;
        match field {
            0 => wrong.owner.endpoint += 1,
            1 => wrong.owner.pid += 1,
            2 => wrong.owner.incarnation += 1,
            3 => wrong.owner.address_space_generation += 1,
            _ => wrong.epoch += 1,
        }
        assert_eq!(rt.control_request(wrong, &bytes), Err(Error::Identity));
        assert!(rt
            .control_request(
                r,
                &wire::Frame::FileOwnerRetired {
                    registration: wrong,
                    caller: c,
                    event_id: 7,
                }
                .encode()
                .unwrap()
            )
            .is_err());
    }
    for caller in [
        Caller {
            pid: c.pid + 1,
            ..c
        },
        Caller {
            incarnation: c.incarnation + 1,
            ..c
        },
        Caller {
            address_space_generation: c.address_space_generation + 1,
            ..c
        },
    ] {
        rt.control_request(
            r,
            &wire::Frame::FileOwnerRetired {
                registration: r,
                caller,
                event_id: 8,
            }
            .encode()
            .unwrap(),
        )
        .unwrap();
        assert!(rt.stat_file(r, c, a).is_ok());
    }
    let next = Scope {
        caller: Caller {
            address_space_generation: c.address_space_generation + 2,
            ..c
        },
        namespace_generation: scope().namespace_generation + 1,
        ..scope()
    };
    rt.install(r, next, &ns, "/jail", "/jail/cwd").unwrap();
    let next_receipt = acquire(&mut rt, r, next, 2);
    let next_file = open(&mut rt, next_receipt, Access::Read);
    let ack = wire::Frame::FileOwnerRetiredAck {
        registration: r,
        caller: c,
        event_id: 7,
    };
    assert!(rt.control_request(r, &ack.encode().unwrap()).is_err());
    assert_eq!(rt.control_request(r, &bytes), Ok(ack));
    assert_eq!(rt.control_request(r, &bytes), Ok(ack));
    assert_eq!(
        rt.stat_file(r, c, a),
        Err(Error::Filesystem(FsError::BadHandle))
    );
    assert_eq!(
        rt.open_file(r, receipt, "data", OWNER, Access::Read, || true),
        Err(Error::Scope)
    );
    assert!(rt.stat_file(r, next.caller, next_file).is_ok());
    assert_eq!(ns.shared().lock().open_handle_count(), 5);
    rt.request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), 3);
    rt.close_file(r, next.caller, next_file).unwrap();
    rt.request(r, &wire::Frame::Release(next_receipt).encode().unwrap())
        .unwrap();
    rt.withdraw(r, next).unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), 0);
}

#[test]
fn downloads_directory_description_survives_rename_until_final_close() {
    use sunlight_ipc::vfs_rpc::file_transport as api;
    let mut fs: sunlight_fs::Vfs = sunlight_fs::Vfs::new();
    const SEED: &[sunlight_fs::RamEntry] = &[
        sunlight_fs::RamEntry::dir("/root", 0, 0, 0o40700),
        sunlight_fs::RamEntry::dir("/root/Downloads", 0, 0, 0o40700),
    ];
    fs.mount_tmpfs("/", Tmpfs::new(SEED)).unwrap();
    fs.mount_service_downloads().unwrap();
    fs.mkdir("/root/Downloads/old", 0, 0, 0o700).unwrap();
    let ns = DirectoryNamespace::mounted(fs).unwrap();
    let mut rt = Runtime::<2, 4, 2>::new();
    let r = registration();
    let caller = scope().caller;
    let root = Credential { uid: 0, gid: 0 };
    rt.bind(r).unwrap();
    rt.install(r, scope(), &ns, "/", "/").unwrap();
    let receipt = acquire(&mut rt, r, scope(), 1);
    let baseline = ns.shared().lock().open_handle_count();
    let directory = rt.open_file_with_options(
        r, receipt, "/root/Downloads/old", root, Access::Read,
        Some((api::ACCESS_READ | api::OPEN_DIRECTORY, 0, 0)), || true,
    ).unwrap();
    let duplicate = rt.duplicate_file(r, caller, directory).unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), baseline + 1);
    assert_eq!(rt.stat_file(r, caller, directory).unwrap().file_type, sunlight_fs::FileType::Directory);
    assert_eq!(rt.read_file(r, caller, directory, &mut [0; 1]), Err(Error::Filesystem(FsError::IsDir)));
    let outside = ns.pin("/", "/root").unwrap();
    let cwd = outside.change_directory("Downloads/old", root).unwrap();
    assert_eq!(cwd.cwd().unwrap(), "/root/Downloads/old");
    ns.shared().lock().rename("/root/Downloads/old", "/root/Downloads/new").unwrap();
    assert_eq!(cwd.cwd().unwrap(), "/root/Downloads/new");
    let outside_again = cwd.change_directory("../..", root).unwrap();
    assert_eq!(outside_again.cwd().unwrap(), "/root");
    drop(outside_again);
    drop(cwd);
    drop(outside);
    rt.close_file(r, caller, directory).unwrap();
    let create = api::ACCESS_WRITE | api::OPEN_CREATE;
    assert_eq!(rt.open_file_with_options(
        r, receipt, "data", root, Access::Write, Some((create, 0o600, directory)), || true,
    ), Err(Error::Filesystem(FsError::BadHandle)));
    assert_eq!(rt.open_file_with_options(
        r, receipt, "../..", root, Access::Read,
        Some((api::ACCESS_READ, 0, duplicate)), || true,
    ), Err(Error::Filesystem(FsError::Unsupported)));
    let file = rt.open_file_with_options(
        r, receipt, "data", root, Access::Write, Some((create, 0o600, duplicate)), || true,
    ).unwrap();
    rt.write_file(r, caller, file, b"retained").unwrap();
    rt.close_file(r, caller, file).unwrap();
    assert_eq!(ns.shared().lock().stat("/root/Downloads/new/data").unwrap().size, 8);
    rt.close_file(r, caller, duplicate).unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), baseline);

    let directory = rt.open_file_with_options(
        r, receipt, "/root/Downloads/new", root, Access::Read,
        Some((api::ACCESS_READ | api::OPEN_DIRECTORY, 0, 0)), || true,
    ).unwrap();
    rt.retire_registration(r).unwrap();
    assert_eq!(rt.stat_file(r, caller, directory), Err(Error::Identity));
    assert!(ns.shared().lock().open_handle_count() < baseline + 1);
}
