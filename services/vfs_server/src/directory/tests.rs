use super::*;
use sunlight_fs::{mode, FileSystem, TmpfsLimits};
use sunlight_ipc::vfs_rpc::{
    lifecycle::Caller,
    registration::{EndpointOwner, Registration},
    retention::{self, wire, Scope, Table},
};

fn namespace() -> DirectoryNamespace {
    let mut fs = Tmpfs::with_limits(&[], TmpfsLimits::new(0, 8)).unwrap();
    fs.mkdir("/root", 1, 2, mode::DIR_755).unwrap();
    fs.mkdir("/root/cwd", 3, 4, mode::DIR_700).unwrap();
    fs.mkdir("/outside", 5, 6, mode::DIR_755).unwrap();
    DirectoryNamespace::new(fs).unwrap()
}

fn registration() -> Registration {
    Registration {
        owner: EndpointOwner {
            endpoint: 0,
            pid: 2,
            incarnation: 3,
            address_space_generation: 4,
        },
        epoch: 5,
    }
}

fn scope() -> Scope {
    Scope {
        caller: Caller {
            pid: 6,
            incarnation: 7,
            address_space_generation: 8,
        },
        namespace_id: 9,
        namespace_generation: 10,
    }
}

#[test]
fn pins_follow_real_directory_objects_and_observe_live_metadata() {
    let namespace = namespace();
    let pair = namespace.pin("/root", "/root/cwd").unwrap();
    let before = pair.metadata().unwrap();
    {
        let mut fs = namespace.isolated().lock();
        fs.rename("/root", "/moved").unwrap();
        fs.mkdir("/root", 11, 12, mode::DIR_755).unwrap();
        fs.mkdir("/root/cwd", 13, 14, mode::DIR_755).unwrap();
        fs.chmod("/moved/cwd", mode::DIR_755).unwrap();
        fs.chown("/moved/cwd", 15, 16).unwrap();
    }
    let after = pair.metadata().unwrap();
    assert_eq!(before[0], after[0]);
    assert_eq!(before[1].0, after[1].0);
    assert_eq!((after[1].1.uid, after[1].1.gid), (15, 16));
    assert_eq!(after[1].1.mode, mode::DIR_755);
    let replacement = namespace.pin("/root", "/root/cwd").unwrap();
    assert_ne!(replacement.metadata().unwrap()[1].0, after[1].0);
    // Removing another dynamic inode shifts backend indexes; handles follow.
    {
        let mut fs = namespace.isolated().lock();
        let file = fs.create_file("/gone", 0, 0, mode::FILE_644).unwrap();
        fs.close(file).unwrap();
        fs.mkdir("/later", 0, 0, mode::DIR_755).unwrap();
    }
    let later = namespace.pin("/", "/later").unwrap();
    let later_metadata = later.metadata().unwrap();
    namespace.isolated().lock().unlink("/gone").unwrap();
    assert_eq!(later.metadata().unwrap(), later_metadata);
    assert_eq!(pair.metadata().unwrap(), after);
}

#[test]
fn policy_rejects_escapes_and_files_and_rolls_back_partial_acquisition() {
    let namespace = namespace();
    for (root, directory) in [
        ("/root", "/outside"),
        ("/root", "/rooted"),
        ("/root", "/root/../outside"),
        ("/root", "/root//cwd"),
        ("/root/.", "/root/cwd"),
        ("/root", "/root/cwd/"),
        ("/root", "root/cwd"),
    ] {
        assert!(namespace.pin(root, directory).is_err());
    }
    let mut fs = namespace.isolated().lock();
    let file = fs.create_file("/root/file", 0, 0, mode::FILE_644).unwrap();
    fs.close(file).unwrap();
    drop(fs);
    for _ in 0..64 {
        assert!(matches!(
            namespace.pin("/root", "/root/file"),
            Err(FsError::NotDir)
        ));
        assert!(matches!(
            namespace.pin("/root", "/root/missing"),
            Err(FsError::NotFound)
        ));
    }
    // Fill all 32 slots; failed second acquisition must return its first slot.
    let pairs: alloc::vec::Vec<_> = (0..15)
        .map(|_| namespace.pin("/root", "/root/cwd").unwrap())
        .collect();
    let spare = namespace.isolated().lock().open("/").unwrap();
    for _ in 0..64 {
        assert!(matches!(
            namespace.pin("/root", "/root/cwd"),
            Err(FsError::TooManyOpenFiles)
        ));
    }
    namespace.isolated().lock().close(spare).unwrap();
    let last = namespace.pin("/root", "/root/cwd").unwrap();
    drop(last);
    drop(pairs);
    let all: alloc::vec::Vec<_> = (0..16).map(|_| namespace.pin("/", "/").unwrap()).collect();
    assert_eq!(all[0].metadata().unwrap()[0].0, 0); // synthetic root
}

#[test]
fn real_pins_survive_lost_replies_release_and_registration_drain() {
    let namespace = namespace();
    let weak = Arc::downgrade(namespace.isolated());
    let registration = registration();
    let mut table = Table::<DirectoryPair, 1>::new(registration).unwrap();
    let pending = wire::PendingRetain::new(registration, scope(), 1).unwrap();
    let wire::Frame::Retained(receipt) = table
        .retain_reply(registration, pending, || {
            namespace.pin("/root", "/root/cwd")
        })
        .unwrap()
    else {
        panic!("retain must own real pins")
    };
    let metadata = table
        .get(registration, receipt)
        .unwrap()
        .metadata()
        .unwrap();
    // Lost success/replay cannot reacquire handles or destroy the lease.
    assert!(matches!(
        table.retain_reply(
            registration,
            pending,
            || -> Result<DirectoryPair, FsError> { panic!("replay must not resolve") }
        ),
        Err(retention::Error::Replay)
    ));
    let next = wire::PendingRetain::new(registration, scope(), 2).unwrap();
    assert!(matches!(
        table
            .retain_reply(registration, next, || -> Result<DirectoryPair, FsError> {
                panic!("capacity must not resolve")
            })
            .unwrap(),
        wire::Frame::Refused {
            reason: wire::Refusal::Capacity,
            ..
        }
    ));
    let mut stale = registration;
    stale.epoch += 1;
    assert_eq!(
        table.release(stale, receipt),
        Err(retention::Error::Identity)
    );
    assert_eq!(
        table
            .get(registration, receipt)
            .unwrap()
            .metadata()
            .unwrap(),
        metadata
    );
    table.release(registration, receipt).unwrap();
    // Lost release acknowledgement: retries close no additional handles.
    let pair = namespace.pin("/root", "/root/cwd").unwrap();
    table.release(registration, receipt).unwrap();
    assert_eq!(pair.metadata().unwrap(), metadata);
    drop(pair);
    let receipt = table
        .retain(registration, 3, scope(), || {
            namespace.pin("/root", "/root/cwd")
        })
        .unwrap();
    drop(namespace);
    assert!(weak.upgrade().is_some());
    assert_eq!(
        table
            .get(registration, receipt)
            .unwrap()
            .metadata()
            .unwrap(),
        metadata
    );
    // Registration table teardown is the final owner of the backend and pins.
    drop(table);
    assert!(weak.upgrade().is_none());
}

#[test]
fn policy_retains_installed_objects_after_pathname_replacement() {
    use crate::policy::Policy;
    let namespace = namespace();
    let mut policy = Policy::<1, 16>::new(registration()).unwrap();
    policy
        .install(scope(), &namespace, "/root", "/root/cwd")
        .unwrap();
    let weak = Arc::downgrade(namespace.isolated());
    {
        let mut fs = namespace.isolated().lock();
        fs.rename("/root", "/moved").unwrap();
        fs.mkdir("/root", 11, 12, mode::DIR_755).unwrap();
        fs.mkdir("/root/cwd", 13, 14, mode::DIR_755).unwrap();
    }
    let mut receipts = alloc::vec::Vec::new();
    for id in 1..=16 {
        let request = wire::PendingRetain::new(registration(), scope(), id).unwrap();
        let wire::Frame::Retained(receipt) = policy
            .request(registration(), &request.request().encode().unwrap())
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(
            policy.metadata(registration(), receipt).unwrap()[1].1.uid,
            3
        );
        receipts.push(receipt);
    }
    // Sixteen leases share the original pair; they consume no more handles.
    let pairs: alloc::vec::Vec<_> = (0..15).map(|_| namespace.pin("/", "/").unwrap()).collect();
    assert!(matches!(
        namespace.pin("/", "/"),
        Err(FsError::TooManyOpenFiles)
    ));
    drop(pairs);
    policy.withdraw(scope()).unwrap();
    drop(namespace);
    assert!(weak.upgrade().is_some());
    for receipt in receipts {
        policy
            .request(
                registration(),
                &wire::Frame::Release(receipt).encode().unwrap(),
            )
            .unwrap();
    }
    assert!(weak.upgrade().is_none());
}

#[test]
fn mounted_pins_share_legacy_state_and_account_for_open_handles() {
    let mut vfs = Vfs::new();
    vfs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    vfs.mkdir("/volume", 0, 0, mode::DIR_755).unwrap();
    vfs.mount_tmpfs("/volume", Tmpfs::new(&[])).unwrap();
    vfs.mkdir("/volume/cwd", 7, 8, mode::DIR_700).unwrap();
    let mount_id = vfs.mount_info(1).unwrap().id;
    let namespace = DirectoryNamespace::mounted(vfs).unwrap();
    let legacy = namespace.shared();
    let pair = namespace.pin("/", "/volume/cwd").unwrap();
    assert_eq!(legacy.lock().open_handle_count(), 2);
    let before = pair.metadata().unwrap();
    legacy
        .lock()
        .rename("/volume/cwd", "/volume/moved")
        .unwrap();
    legacy
        .lock()
        .mkdir("/volume/cwd", 9, 10, mode::DIR_755)
        .unwrap();
    legacy.lock().chown("/volume/moved", 11, 12).unwrap();
    let after = pair.metadata().unwrap();
    assert_eq!(before[1].0, after[1].0);
    assert_eq!((after[1].1.uid, after[1].1.gid), (11, 12));
    assert_ne!(
        after[1].0,
        namespace
            .pin("/", "/volume/cwd")
            .unwrap()
            .metadata()
            .unwrap()[1]
            .0
    );
    assert!(legacy.lock().unmount(mount_id).is_err());
    drop(namespace);
    assert_eq!(pair.metadata().unwrap(), after);
    drop(pair);
    assert_eq!(legacy.lock().open_handle_count(), 0);
    // Boot mounts remain protected after pins are released.
    assert_eq!(
        legacy.lock().unmount(mount_id),
        Err(FsError::OperationNotPermitted)
    );
}
