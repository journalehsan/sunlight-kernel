use super::*;
use crate::device_lease::Status;
use sunlight_fat::testimg::FatImageBuilder;
use sunlight_ipc::vfs_rpc::registration::{EndpointOwner, Registration};

const TARGET: &str = "/mnt/volume";
const FILE: &str = "/mnt/volume/NEW.TXT";
const ROOT: Credential = Credential { uid: 0, gid: 0 };

#[test]
fn block_capable_runtime_shares_attachment_namespace() {
    use crate::{open::{Access, Seek}, runtime::Runtime};
    use sunlight_ipc::vfs_rpc::{
        lifecycle::Caller,
        retention::{wire, Scope},
    };

    let namespace = namespace();
    let (authority, media, session) = device(true);
    let registration = Registration {
        owner: EndpointOwner {
            endpoint: 7,
            pid: 12,
            incarnation: 2,
            address_space_generation: 3,
        },
        epoch: 11,
    };
    let scope = Scope {
        caller: Caller {
            pid: 13,
            incarnation: 4,
            address_space_generation: 5,
        },
        namespace_id: 1,
        namespace_generation: 1,
    };
    let mut runtime = Runtime::<1, 2, 4, AttachmentDevice>::new();
    runtime.bind(registration).unwrap();
    runtime
        .install(registration, scope, &namespace, "/", "/")
        .unwrap();
    let request = wire::PendingRetain::new(registration, scope, 1)
        .unwrap()
        .request()
        .encode()
        .unwrap();
    let wire::Frame::Retained(receipt) = runtime.request(registration, &request).unwrap() else {
        panic!("expected retained caller scope");
    };

    let mut attached = namespace
        .publish_device_attachment(
            namespace
                .stage_device_attachment(TARGET, authority.clone(), &session, &media)
                .unwrap(),
        )
        .unwrap();
    let view = namespace.caller_view("/", "/", ROOT).unwrap();
    assert_eq!(
        view.capacity("/mnt/volume/KEEP.TXT").unwrap().id,
        attached.id()
    );
    let token = runtime.open_file(
            registration,
            receipt,
            "/mnt/volume/KEEP.TXT",
            ROOT,
            Access::Read,
            || true
        ).unwrap();
    let alias = runtime.duplicate_file(registration, scope.caller, token).unwrap();
    let shared = namespace.shared();
    {
        let mut fs = shared.lock();
        assert_eq!(fs.unlink("/mnt/volume/KEEP.TXT"), Err(FsError::Busy));
        fs.rename("/mnt/volume/KEEP.TXT", "/mnt/volume/MOVED.TXT").unwrap();
        let handle = fs.create_file_exclusive("/other/data", 0, 0, 0o600).unwrap();
        fs.write(handle, 0, b"ordinary").unwrap();
        fs.close(handle).unwrap();
    }
    assert_eq!(runtime.seek_file(registration, scope.caller, alias, Seek::Start(2)), Ok(2));
    let mut part = [0; 3];
    assert_eq!(runtime.read_at_file(registration, scope.caller, token, 0, &mut part), Ok(3));
    assert_eq!(&part, b"unt");
    runtime.close_file(registration, scope.caller, token).unwrap();
    assert_eq!(runtime.read_file(registration, scope.caller, alias, &mut part), Ok(3));
    assert_eq!(&part, b"tou");
    runtime.close_file(registration, scope.caller, alias).unwrap();
    let token = runtime
        .open_file(registration, receipt, "/other/data", ROOT, Access::Read, || true)
        .unwrap();
    namespace.withdraw_device_attachment(&mut attached).unwrap();
    let mut bytes = [0; 8];
    assert_eq!(
        runtime.read_file(registration, scope.caller, token, &mut bytes),
        Ok(8)
    );
    assert_eq!(&bytes, b"ordinary");
    runtime
        .close_file(registration, scope.caller, token)
        .unwrap();
    vacant(&authority);
}

fn namespace() -> DirectoryNamespace<AttachmentDevice> {
    let mut fs = Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    fs.mkdir("/mnt", 0, 0, 0o755).unwrap();
    fs.mkdir(TARGET, 0, 0, 0o755).unwrap();
    fs.mkdir("/other", 0, 0, 0o755).unwrap();
    DirectoryNamespace::mounted_for_device(fs).unwrap()
}

fn device(valid: bool) -> (Arc<DeviceLease>, Media, Session) {
    let mut fixture = FatImageBuilder::new(1);
    fixture.add_file(fixture.root(), "KEEP.TXT", b"untouched");
    let mut image = fixture.build();
    if !valid {
        image[82] = 0;
    }
    let (device, media) = DeviceLease::disposable_image(
        image
            .chunks_exact(BLOCK_SIZE)
            .map(|sector| sector.try_into().unwrap())
            .collect(),
    )
    .unwrap();
    let session = device
        .bind_registration(Registration {
            owner: EndpointOwner {
                endpoint: 7,
                pid: 12,
                incarnation: 2,
                address_space_generation: 3,
            },
            epoch: 11,
        })
        .unwrap();
    (Arc::new(device), media, session)
}

fn vacant(device: &DeviceLease) {
    let settlement = device.snapshot();
    assert_eq!(settlement.status, Status::Vacant);
    assert!(settlement.lease.is_none());
    assert_eq!(settlement.outstanding, 0);
    assert_eq!(settlement.retained_data, 0);
    assert!(!settlement.flush_pending);
}

#[test]
fn authenticated_fat_attachment_survives_reopen_and_fences_closing_views() {
    let namespace = namespace();
    let (device, media, session) = device(true);
    let shared = namespace.shared();
    let payload = b"leased bytes";
    let mut previous_id = 0;
    for cycle in 0..4 {
        let before = namespace.caller_view("/", "/", ROOT).unwrap();
        let staged = namespace
            .stage_device_attachment(TARGET, device.clone(), &session, &media)
            .unwrap();
        assert_eq!(before.snapshot().unwrap().rows().len(), 1);
        let mut attached = namespace.publish_device_attachment(staged).unwrap();
        assert!(attached.id() > previous_id);
        if previous_id != 0 {
            assert!(matches!(
                shared.lock().prepare_device_withdrawal(previous_id),
                Err(FsError::NotFound)
            ));
        }
        previous_id = attached.id();
        assert!(matches!(before.snapshot(), Err(Error::Stale)));
        let view = namespace.caller_view("/", "/", ROOT).unwrap();
        let rows = view.snapshot().unwrap();
        assert_eq!(rows.rows()[1].id, attached.id());
        assert_eq!(rows.rows()[1].mountpoint, TARGET);
        assert_eq!(before.capacity(TARGET), Err(Error::Stale));
        let capacity = view.capacity(TARGET).unwrap();
        assert_eq!(capacity.id, rows.rows()[1].id);
        assert_eq!(view.capacity("/mnt/volume/keep.txt"), Ok(capacity));
        assert_eq!(
            view.capacity("/mnt/volume/absent/.."),
            Err(FsError::NotFound.into())
        );
        assert_eq!(view.capacity("/other"), Err(FsError::Unsupported.into()));
        assert_eq!(
            namespace
                .caller_view("/other", "/other", ROOT)
                .unwrap()
                .capacity("../mnt/volume"),
            Err(FsError::NotFound.into())
        );
        assert_eq!(
            namespace
                .caller_view("/other", "/other", ROOT)
                .unwrap()
                .snapshot()
                .unwrap()
                .rows()
                .len(),
            1
        );
        let handle = {
            let mut fs = shared.lock();
            let keep = fs.open("/mnt/volume/KEEP.TXT").unwrap();
            let mut original = [0; 9];
            assert_eq!(fs.read(keep, 0, &mut original).unwrap(), original.len());
            assert_eq!(&original, b"untouched");
            fs.close(keep).unwrap();
            let handle = if cycle == 0 {
                let handle = fs.create_file_exclusive(FILE, 0, 0, 0o644).unwrap();
                assert_eq!(fs.write(handle, 0, payload).unwrap(), payload.len());
                handle
            } else {
                fs.open(FILE).unwrap()
            };
            assert_eq!(fs.stat(FILE).unwrap().size as usize, payload.len());
            handle
        };
        let after = view.capacity(FILE).unwrap();
        assert_eq!(after.id, capacity.id);
        assert_eq!(after.free, capacity.free - u32::from(cycle == 0));
        assert_eq!(
            namespace.withdraw_device_attachment(&mut attached),
            Err(FsError::Busy.into())
        );
        assert_eq!(
            rows.install(|_| panic!("stale snapshot")),
            Err(Error::Stale)
        );
        assert_eq!(view.capacity(FILE), Err(Error::Stale));
        assert_eq!(
            namespace
                .caller_view("/", "/", ROOT)
                .unwrap()
                .capacity(FILE),
            Err(FsError::Busy.into())
        );
        assert_eq!(
            namespace
                .caller_view("/", "/", ROOT)
                .unwrap()
                .snapshot()
                .unwrap()
                .rows()
                .len(),
            1
        );
        {
            let mut fs = shared.lock();
            assert_eq!(fs.open(FILE), Err(FsError::Busy));
            let mut bytes = [0; 5];
            assert_eq!(fs.read(handle, 2, &mut bytes).unwrap(), bytes.len());
            assert_eq!(&bytes, &payload[2..7]);
            fs.close(handle).unwrap();
        }
        namespace.withdraw_device_attachment(&mut attached).unwrap();
        vacant(&device);
        assert_eq!(
            namespace
                .caller_view("/", "/", ROOT)
                .unwrap()
                .snapshot()
                .unwrap()
                .rows()
                .len(),
            1
        );
    }
}

#[test]
fn authenticated_fat_attachment_compensates_probe_and_publication_failures() {
    let namespace = namespace();
    let initial = namespace.shared().lock().topology_revision();
    let (invalid, media, session) = device(false);
    assert!(matches!(
        namespace.stage_device_attachment(TARGET, invalid.clone(), &session, &media),
        Err(Error::Filesystem(FsError::Io))
    ));
    vacant(&invalid);
    assert_eq!(namespace.shared().lock().topology_revision(), initial);

    let (valid, media, session) = device(true);
    let staged = namespace
        .stage_device_attachment(TARGET, valid.clone(), &session, &media)
        .unwrap();
    valid.withdraw_registration(&session).unwrap();
    assert!(matches!(
        namespace.publish_device_attachment(staged),
        Err(Error::Lease(crate::device_lease::Error::Unauthorized))
    ));
    vacant(&valid);
    assert_eq!(namespace.shared().lock().topology_revision(), initial);

    let (valid, media, session) = device(true);
    let staged = namespace
        .stage_device_attachment(TARGET, valid.clone(), &session, &media)
        .unwrap();
    let other = namespace.stage_attachment(TARGET, Tmpfs::new(&[])).unwrap();
    let other_id = namespace.publish_attachment(other).unwrap();
    assert!(matches!(
        namespace.publish_device_attachment(staged),
        Err(Error::Stale)
    ));
    vacant(&valid);
    assert_eq!(namespace.shared().lock().topology_revision(), initial + 1);
    namespace.withdraw_attachment(other_id).unwrap();
}

#[test]
fn authenticated_fat_attachment_fences_unsettled_withdrawal() {
    let namespace = namespace();
    let (device, media, session) = device(true);
    let staged = namespace
        .stage_device_attachment(TARGET, device.clone(), &session, &media)
        .unwrap();
    let mut attached = namespace.publish_device_attachment(staged).unwrap();
    let lease = device.snapshot().lease.unwrap();
    let retained = device
        .read_authenticated(
            &session,
            &lease,
            Extent {
                offset: 0,
                length: BLOCK_SIZE as u64,
            },
        )
        .unwrap();
    assert_eq!(
        namespace.withdraw_device_attachment(&mut attached),
        Err(FsError::Io.into())
    );
    assert!(matches!(device.snapshot().status, Status::Fenced(_)));
    assert_eq!(device.snapshot().retained_data, 1);
    assert_eq!(
        namespace.shared().lock().mount_info(1).unwrap().id,
        attached.id()
    );
    assert!(matches!(
        device.complete(&retained, Outcome::Success),
        Ok(Completion::Read(_))
    ));
    assert_eq!(device.snapshot().retained_data, 0);
    assert!(matches!(device.snapshot().status, Status::Fenced(_)));
}

#[test]
fn split_withdrawal_keeps_topology_available_and_rejects_duplicate_work() {
    let namespace = namespace();
    let (authority, media, session) = device(true);
    let mut attached = namespace
        .publish_device_attachment(
            namespace
                .stage_device_attachment(TARGET, authority.clone(), &session, &media)
                .unwrap(),
        )
        .unwrap();
    let id = attached.id();
    let shared = namespace.shared();
    let mut reached_sync = false;
    namespace
        .withdraw_device_attachment_with(&mut attached, |work| {
            reached_sync = true;
            {
                let mut topology = shared.try_lock().expect("writeback holds topology");
                assert_eq!(topology.stat(TARGET).unwrap_err(), FsError::Busy);
                assert!(matches!(
                    topology.prepare_device_withdrawal(id),
                    Err(FsError::Busy)
                ));
                assert_eq!(
                    topology.validate_tmpfs_attachment(TARGET).unwrap_err(),
                    FsError::AlreadyExists
                );
                assert_eq!(topology.finish_device_withdrawal(work), Err(FsError::Busy));
            }
            assert!(authority.snapshot().lease.is_some());
            work.synchronize()
        })
        .unwrap();
    assert!(reached_sync);
    assert_eq!(shared.lock().open_handle_count(), 0);
    assert_eq!(
        namespace.withdraw_device_attachment(&mut attached),
        Err(Error::Stale)
    );
    vacant(&authority);
}
