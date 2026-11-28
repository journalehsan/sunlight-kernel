extern crate std;

use super::*;
use crate::open::{Access, Table};
use std::{
    sync::{mpsc, Barrier},
    thread,
};
use sunlight_ipc::vfs_rpc::lifecycle::Caller;

const USER: Credential = Credential { uid: 42, gid: 43 };
const CALLER: Caller = Caller {
    pid: 1,
    incarnation: 1,
    address_space_generation: 1,
};
const TARGET: &str = "/mnt/volume";

#[cfg(feature = "vfs-c1e2-test")]
#[test]
fn mount_observation_uses_retained_root_and_hides_preparations() {
    let namespace = fixture();
    let full = namespace.pin("/", "/").unwrap();
    let rebased = namespace.pin("/restricted", "/restricted").unwrap();
    let pending = namespace.stage_attachment(TARGET, backend()).unwrap();
    let (revision, _, hidden) = full.mount_observation(USER, 7, None).unwrap();
    assert!(!core::str::from_utf8(&hidden).unwrap().contains(TARGET));
    let mount = namespace.publish_attachment(pending).unwrap();
    let (new_revision, _, visible) = full.mount_observation(USER, 7, None).unwrap();
    assert!(new_revision > revision);
    let id = sunlight_ipc::storage::service_mount_id(7, mount).unwrap();
    assert!(core::str::from_utf8(&visible).unwrap()
        .lines().any(|row| row.starts_with(&std::format!("{id} ")) && row.contains(TARGET)));
    let (_, _, scoped) = rebased.mount_observation(USER, 7, None).unwrap();
    assert!(!core::str::from_utf8(&scoped).unwrap().contains(TARGET));
    assert!(matches!(
        rebased.mount_observation(USER, 7, Some("/mnt/volume/data")),
        Err(Error::Filesystem(FsError::NotFound))
    ));
}

fn fixture() -> DirectoryNamespace {
    let mut fs = Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    for path in ["/mnt", TARGET, "/restricted"] {
        fs.mkdir(path, 0, 0, 0o755).unwrap();
    }
    DirectoryNamespace::mounted(fs).unwrap()
}

fn backend() -> Tmpfs {
    let mut backend = Tmpfs::new(&[]);
    let handle = backend.create_file("/data", 42, 43, 0o600).unwrap();
    backend.write(handle, 0, b"retained").unwrap();
    backend.close(handle).unwrap();
    backend
}

#[test]
fn publication_fences_old_views_and_snapshot_installation() {
    let owner = fixture();
    let before = owner.caller_view("/", "/", USER).unwrap();
    let snapshot = before.snapshot().unwrap();
    let staged = owner.stage_attachment(TARGET, backend()).unwrap();
    assert_eq!(
        before.stat("/mnt/volume/data"),
        Err(FsError::NotFound.into())
    );
    assert_eq!(before.snapshot().unwrap().rows(), snapshot.rows());
    let id = owner.publish_attachment(staged).unwrap();
    assert_eq!(before.stat(TARGET), Err(Error::Stale));
    assert!(matches!(before.snapshot(), Err(Error::Stale)));
    assert_eq!(
        snapshot.install(|_| panic!("stale publication")),
        Err(Error::Stale)
    );

    let global = owner.caller_view("/", "/", USER).unwrap();
    let restricted = owner
        .caller_view("/restricted", "/restricted", USER)
        .unwrap();
    let rebased = owner.caller_view("/mnt", "/mnt", USER).unwrap();
    assert_eq!(global.revision(), before.revision() + 1);
    let stat = global.stat("/mnt/volume/data").unwrap();
    let sample = global.snapshot().unwrap();
    let row = &sample.rows()[1];
    assert_eq!(
        (row.id, row.slot, row.incarnation),
        (id, stat.mount_id, stat.mount_generation)
    );
    assert_eq!(
        (row.source, row.protected, row.parent_id),
        (0, false, sample.rows()[0].id)
    );
    assert_eq!(sample.install(|rows| rows.len()), Ok(2));
    assert_eq!(
        restricted.stat("/../mnt/volume/data"),
        Err(FsError::NotFound.into())
    );
    assert_eq!(restricted.snapshot().unwrap().rows().len(), 1);
    assert_eq!(rebased.snapshot().unwrap().rows()[1].mountpoint, "/volume");
    let pending = global.snapshot().unwrap();
    owner.shared().lock().chmod("/mnt", 0o700).unwrap();
    assert_eq!(
        pending.install(|_| panic!("visibility changed")),
        Err(Error::Stale)
    );
}

#[test]
fn failed_staging_and_publish_do_not_spend_revision_or_slot() {
    let owner = fixture();
    let other = fixture();
    let view = owner.caller_view("/", "/", USER).unwrap();
    let mut busy_backend = backend();
    busy_backend.open("/data").unwrap();
    assert!(matches!(
        owner.stage_attachment(TARGET, busy_backend),
        Err(Error::Filesystem(FsError::Busy))
    ));
    assert!(matches!(
        owner.stage_attachment("/missing", backend()),
        Err(Error::Filesystem(FsError::OperationNotPermitted))
    ));
    let staged = owner.stage_attachment(TARGET, backend()).unwrap();
    assert_eq!(other.publish_attachment(staged), Err(Error::WrongOwner));
    let staged = owner.stage_attachment(TARGET, backend()).unwrap();
    let fs = owner.shared();
    {
        let mut fs = fs.lock();
        let file = fs.create_file("/mnt/volume/obstacle", 0, 0, 0o600).unwrap();
        fs.close(file).unwrap();
    }
    assert_eq!(owner.publish_attachment(staged), Err(FsError::Busy.into()));
    assert_eq!(view.snapshot().unwrap().rows().len(), 1);
    assert_eq!(fs.lock().open_handle_count(), 2);
    fs.lock().unlink("/mnt/volume/obstacle").unwrap();
    let replaced = owner.stage_attachment(TARGET, backend()).unwrap();
    {
        let mut fs = fs.lock();
        fs.rename(TARGET, "/mnt/old").unwrap();
        fs.mkdir(TARGET, 0, 0, 0o755).unwrap();
    }
    assert_eq!(owner.publish_attachment(replaced), Err(Error::Stale));
    let first = owner.stage_attachment(TARGET, backend()).unwrap();
    let competing = owner.stage_attachment(TARGET, backend()).unwrap();
    assert_eq!(owner.publish_attachment(first), Ok(2));
    assert_eq!(owner.publish_attachment(competing), Err(Error::Stale));
    assert!(matches!(
        owner.stage_attachment(TARGET, backend()),
        Err(Error::Filesystem(FsError::AlreadyExists))
    ));
    assert_eq!(fs.lock().topology_revision(), view.revision() + 1);
    assert_eq!(fs.lock().rename("/mnt", "/moved"), Err(FsError::Busy));
}

#[test]
fn withdrawal_waits_for_retained_root_and_last_description() {
    let owner = fixture();
    let id = owner
        .publish_attachment(owner.stage_attachment(TARGET, backend()).unwrap())
        .unwrap();
    let retained = owner.caller_view(TARGET, TARGET, USER).unwrap();
    let mut files = Table::<2>::new();
    let token = files
        .insert(
            CALLER,
            retained.pair.open_file("/data", USER, sunlight_fs::vfs::OpenOptions {
                read: true,
                ..Default::default()
            }, false, false, false, None).unwrap(),
            Access::Read,
        )
        .unwrap();
    let duplicate = files.duplicate(CALLER, token).unwrap();
    let fs = owner.shared();
    let revision = retained.revision();
    assert_eq!(fs.lock().open_handle_count(), 3);
    assert_eq!(owner.withdraw_attachment(id), Err(FsError::Busy.into()));
    drop(retained);
    assert_eq!(fs.lock().open_handle_count(), 1);
    assert_eq!(owner.withdraw_attachment(id), Err(FsError::Busy.into()));
    files.close(CALLER, token).unwrap();
    let mut data = [0; 3];
    assert_eq!(files.read_at(CALLER, duplicate, 2, &mut data), Ok(3));
    assert_eq!(&data, b"tai");
    assert_eq!(fs.lock().topology_revision(), revision);
    files.close(CALLER, duplicate).unwrap();
    assert_eq!(fs.lock().open_handle_count(), 0);
    owner.withdraw_attachment(id).unwrap();
    assert_eq!(fs.lock().topology_revision(), revision + 1);
    assert!(fs.lock().mount_info(1).is_none());
    assert_eq!(fs.lock().stat("/mnt/volume/data"), Err(FsError::NotFound));
    let next = owner
        .publish_attachment(owner.stage_attachment(TARGET, Tmpfs::new(&[])).unwrap())
        .unwrap();
    assert_ne!(next, id);
    assert_eq!(owner.withdraw_attachment(id), Err(FsError::NotFound.into()));
    let view = owner.caller_view("/", "/", USER).unwrap();
    assert_eq!(view.revision(), revision + 2);
    let snapshot = view.snapshot().unwrap();
    assert_eq!(
        (snapshot.rows()[1].slot, snapshot.rows()[1].incarnation),
        (2, 2)
    );
    assert_eq!(view.stat("/mnt/volume/data"), Err(FsError::NotFound.into()));
    let weak = Arc::downgrade(&fs);
    drop((fs, owner));
    assert!(weak.upgrade().is_some());
    drop(view);
    assert!(weak.upgrade().is_none());
}

#[test]
fn racing_snapshot_observes_one_complete_revision() {
    let owner = fixture();
    let start = Barrier::new(2);
    thread::scope(|scope| {
        let publisher = scope.spawn(|| {
            let mut fixture = Tmpfs::new(&[]);
            let file = fixture.create_file("/data", 42, 43, 0o600).unwrap();
            fixture.close(file).unwrap();
            let staged = owner.stage_attachment(TARGET, fixture).unwrap();
            start.wait();
            owner.publish_attachment(staged).unwrap()
        });
        start.wait();
        for _ in 0..64 {
            let view = match owner.caller_view("/", "/", USER) {
                Ok(view) => view,
                Err(Error::Stale) => continue,
                Err(error) => panic!("view capture failed: {error:?}"),
            };
            let snapshot = match view.snapshot() {
                Ok(snapshot) => snapshot,
                Err(Error::Stale) => continue,
                Err(error) => panic!("snapshot failed: {error:?}"),
            };
            match snapshot.revision() {
                1 => assert_eq!(snapshot.rows().len(), 1),
                2 => {
                    assert_eq!(snapshot.rows().len(), 2);
                    let row = &snapshot.rows()[1];
                    assert_eq!(
                        (row.mountpoint.as_str(), row.id, row.source, row.protected),
                        (TARGET, 2, 0, false)
                    );
                    match view.stat("/mnt/volume/data") {
                        Ok(data) => assert_eq!(
                            (data.mount_id, data.mount_generation),
                            (row.slot, row.incarnation)
                        ),
                        result => panic!("published backend missing: {result:?}"),
                    }
                }
                revision => panic!("unexpected revision {revision}"),
            }
        }
        assert_eq!(publisher.join().unwrap(), 2);
    });
    assert_eq!(
        owner
            .caller_view("/", "/", USER)
            .unwrap()
            .snapshot()
            .unwrap()
            .rows()
            .len(),
        2
    );
}

#[test]
fn install_holds_publication_fence_across_consumer_commit() {
    let owner = fixture();
    let view = owner.caller_view("/", "/", USER).unwrap();
    let staged = owner.stage_attachment(TARGET, backend()).unwrap();
    let snapshot = view.snapshot().unwrap();
    let (installing, begun) = mpsc::channel();
    let (checked, release) = mpsc::channel();
    let shared = owner.shared();
    thread::scope(|scope| {
        let writer = scope.spawn(move || {
            begun.recv().unwrap();
            assert!(shared.try_lock().is_none());
            checked.send(()).unwrap();
            owner.publish_attachment(staged).unwrap()
        });
        assert_eq!(
            snapshot.install(|rows| {
                installing.send(()).unwrap();
                release.recv().unwrap();
                rows.len()
            }),
            Ok(1)
        );
        assert_eq!(writer.join().unwrap(), 2);
    });
}
