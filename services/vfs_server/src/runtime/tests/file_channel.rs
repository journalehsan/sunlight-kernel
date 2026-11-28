use super::control::Control;
use super::*;
use crate::open::Access;
use sunlight_fs::Credential;
use sunlight_ipc::vfs_rpc::{file_transport as api, retention::wire};
const OWNER: Credential = Credential { uid: 42, gid: 43 };

struct Channel {
    current: Option<[u8; api::FRAME_BYTES]>,
    replies: usize,
}
impl Control for Channel {
    fn snapshot(&mut self) -> Option<Registration> {
        None
    }
    fn status(&mut self, _: u64) -> Result<(Registration, bool), ()> {
        Err(())
    }
    fn next(&mut self, _: u64) -> Result<Option<[u8; wire::FRAME_BYTES]>, ()> {
        Ok(None)
    }
    fn reply(&mut self, _: u64, _: &[u8; wire::FRAME_BYTES]) -> bool {
        false
    }
    fn file_next(&mut self, _: u64) -> Result<Option<[u8; api::FRAME_BYTES]>, ()> {
        Ok(self.current)
    }
    fn file_reply(&mut self, _: u64, _: &[u8; api::FRAME_BYTES]) -> bool {
        self.replies += 1;
        true
    }
}

fn fixture<const N: usize>() -> (Runtime<2, N, 2>, Channel, Receipt) {
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
    let channel = Channel {
        current: None,
        replies: 0,
    };
    (runtime, channel, receipt)
}

fn request(op: u64, token: u64, arg0: u64, arg1: u64, payload: &[u8]) -> api::Request<'_> {
    api::Request {
        registration: registration(),
        caller: scope().caller,
        request_id: 1,
        op,
        token,
        arg0,
        arg1,
        length: payload.len(),
        namespace_id: scope().namespace_id,
        namespace_generation: scope().namespace_generation,
        receipt_id: 1,
        receipt_generation: 1,
        directory_generation: 0,
        uid: OWNER.uid,
        gid: OWNER.gid,
        payload,
    }
}

fn io(op: u64, token: u64, arg0: u64, arg1: u64, payload: &[u8]) -> api::Request<'_> {
    api::Request {
        namespace_id: 0,
        namespace_generation: 0,
        receipt_id: 0,
        receipt_generation: 0,
        directory_generation: 0,
        uid: 0,
        gid: 0,
        ..request(op, token, arg0, arg1, payload)
    }
}

/// Owned view of one dispatched reply, so tests never borrow a transient frame.
struct Reply {
    errno: u64,
    result0: u64,
    payload: alloc::vec::Vec<u8>,
}

fn drive<const N: usize>(
    runtime: &mut Runtime<2, N, 2>,
    channel: &mut Channel,
    request: api::Request<'_>,
) -> Reply {
    let bytes = request.encode().unwrap();
    channel.current = Some(bytes);
    let reply = runtime
        .file_request(channel, registration(), &bytes)
        .unwrap();
    let reply = api::Reply::decode(&reply).unwrap();
    Reply {
        errno: reply.errno,
        result0: reply.result0,
        payload: reply.payload.to_vec(),
    }
}

#[test]
fn copied_open_read_write_seek_stat_dup_and_close_share_one_description() {
    let (mut rt, mut channel, _receipt) = fixture::<4>();
    let open = drive(
        &mut rt,
        &mut channel,
        request(
            api::OPEN,
            0,
            api::ACCESS_READ | api::ACCESS_WRITE,
            0,
            b"data",
        ),
    );
    assert_eq!(open.errno, 0);
    let token = open.result0;
    assert_ne!(token, 0);
    assert_eq!(channel.replies, 0);

    let read = drive(&mut rt, &mut channel, io(api::READ, token, 0, 2, &[]));
    assert_eq!((read.errno, read.result0), (0, 2));
    assert_eq!(read.payload.as_slice(), b"ab");

    let dup = drive(&mut rt, &mut channel, io(api::DUP, token, 0, 0, &[]));
    assert_eq!(dup.errno, 0);
    assert_ne!(dup.result0, token);

    // Duplicates share the offset: the second read continues at 2.
    let read = drive(&mut rt, &mut channel, io(api::READ, dup.result0, 0, 2, &[]));
    assert_eq!((read.errno, read.result0), (0, 2));
    assert_eq!(read.payload.as_slice(), b"cd");

    let seek = drive(
        &mut rt,
        &mut channel,
        io(api::SEEK, token, api::SEEK_END, (-2i64) as u64, &[]),
    );
    assert_eq!((seek.errno, seek.result0), (0, 4));

    let write = drive(&mut rt, &mut channel, io(api::WRITE, token, 0, 0, b"XY"));
    assert_eq!((write.errno, write.result0), (0, 2));

    let stat = drive(&mut rt, &mut channel, io(api::STAT, token, 0, 0, &[]));
    assert_eq!(stat.errno, 0);
    assert_eq!(stat.payload.len(), api::STAT_BYTES);
    assert_eq!(&stat.payload[..8], &6u64.to_le_bytes());

    let close = drive(&mut rt, &mut channel, io(api::CLOSE, token, 0, 0, &[]));
    assert_eq!((close.errno, close.result0), (0, 0));
    // The description stays open through the duplicate; the offset is shared.
    let read = drive(&mut rt, &mut channel, io(api::READ, dup.result0, 0, 2, &[]));
    assert_eq!(read.errno, 0);
    let close = drive(
        &mut rt,
        &mut channel,
        io(api::CLOSE, dup.result0, 0, 0, &[]),
    );
    assert_eq!((close.errno, close.result0), (0, 0));
    let stale = drive(&mut rt, &mut channel, io(api::STAT, token, 0, 0, &[]));
    assert_eq!(stale.errno, 9);
}

#[test]
fn copied_channel_rejects_foreign_registration_and_access_mode() {
    let (mut rt, mut channel, _) = fixture::<4>();
    let open = drive(
        &mut rt,
        &mut channel,
        request(api::OPEN, 0, api::ACCESS_READ, 0, b"data"),
    );
    let token = open.result0;
    let write = drive(&mut rt, &mut channel, io(api::WRITE, token, 0, 0, b"x"));
    assert_eq!(write.errno, 13);
    let read = drive(&mut rt, &mut channel, io(api::READ, token, 0, 16, &[]));
    assert_eq!(read.errno, 0);

    let mut foreign = io(api::STAT, token, 0, 0, &[]);
    foreign.registration = Registration {
        epoch: foreign.registration.epoch + 1,
        ..foreign.registration
    };
    let bytes = foreign.encode().unwrap();
    channel.current = Some(bytes);
    assert!(rt
        .file_request(&mut channel, registration(), &bytes)
        .is_none());

    let mut wrong_receipt = request(api::OPEN, 0, api::ACCESS_READ, 0, b"data");
    wrong_receipt.receipt_id = 99;
    let bytes = wrong_receipt.encode().unwrap();
    channel.current = Some(bytes);
    let reply = rt
        .file_request(&mut channel, registration(), &bytes)
        .unwrap();
    assert_eq!(api::Reply::decode(&reply).unwrap().errno, 5);
}

fn open_read_only<const N: usize>(runtime: &mut Runtime<2, N, 2>, receipt: Receipt) -> u64 {
    runtime
        .open_file(registration(), receipt, "data", OWNER, Access::Read, || {
            true
        })
        .unwrap()
}

#[test]
fn copied_teardown_preserves_descriptions_like_local_owner_operations() {
    let (mut rt, mut channel, receipt) = fixture::<4>();
    let token = open_read_only(&mut rt, receipt);
    let read = drive(&mut rt, &mut channel, io(api::READ, token, 0, 1, &[]));
    assert_eq!(read.errno, 0);
    let bytes = io(api::STAT, token, 0, 0, &[]).encode().unwrap();
    channel.current = Some(bytes);
    rt.retire_caller(scope().caller);
    let reply = rt
        .file_request(&mut channel, registration(), &bytes)
        .unwrap();
    assert_eq!(api::Reply::decode(&reply).unwrap().errno, 0);
    rt.retire_file_owner(scope().caller);
    let reply = rt
        .file_request(&mut channel, registration(), &bytes)
        .unwrap();
    assert_eq!(api::Reply::decode(&reply).unwrap().errno, 9);
}

#[test]
fn transfer_rekeys_one_description_and_survives_old_owner_teardown() {
    let (mut rt, mut channel, receipt) = fixture::<4>();
    let token = open_read_only(&mut rt, receipt);
    let read = drive(&mut rt, &mut channel, io(api::READ, token, 0, 1, &[]));
    assert_eq!((read.errno, read.result0), (0, 1));

    let new_owner = sunlight_ipc::vfs_rpc::lifecycle::Caller {
        pid: scope().caller.pid,
        incarnation: scope().caller.incarnation,
        address_space_generation: scope().caller.address_space_generation + 1,
    };
    let payload = api::encode_caller(new_owner);
    let transfer = api::Request {
        request_id: 2,
        op: api::TRANSFER,
        token,
        arg0: 0,
        arg1: 0,
        length: payload.len(),
        namespace_id: 0,
        namespace_generation: 0,
        receipt_id: 0,
        receipt_generation: 0,
        directory_generation: 0,
        uid: 0,
        gid: 0,
        payload: &payload,
        ..request(api::TRANSFER, token, 0, 0, &payload)
    };
    let transferred = drive(&mut rt, &mut channel, transfer);
    assert_eq!((transferred.errno, transferred.result0), (0, token));

    // The old owner no longer names the description.
    let stale = drive(&mut rt, &mut channel, io(api::STAT, token, 0, 0, &[]));
    assert_eq!(stale.errno, 9);

    // The replacement owner continues at the shared offset.
    let mut next = io(api::READ, token, 0, 1, &[]);
    next.caller = new_owner;
    let read = drive(&mut rt, &mut channel, next);
    assert_eq!((read.errno, read.result0), (0, 1));
    assert_eq!(read.payload.as_slice(), b"b");

    // Old-owner teardown cannot close a transferred description.
    rt.retire_file_owner(scope().caller);
    let mut stat = io(api::STAT, token, 0, 0, &[]);
    stat.caller = new_owner;
    let stat = drive(&mut rt, &mut channel, stat);
    assert_eq!(stat.errno, 0);
}

#[cfg(feature = "vfs-c2e-candidate")]
#[test]
fn copied_downloads_mutations_require_a_live_directory_description() {
    let mut fs: sunlight_fs::Vfs = sunlight_fs::Vfs::new();
    const SEED: &[sunlight_fs::RamEntry] = &[
        sunlight_fs::RamEntry::dir("/root", 0, 0, 0o40700),
        sunlight_fs::RamEntry::dir("/root/Downloads", 0, 0, 0o40700),
    ];
    fs.mount_tmpfs("/", Tmpfs::new(SEED)).unwrap();
    fs.mount_service_downloads().unwrap();
    let namespace = DirectoryNamespace::mounted(fs).unwrap();
    let mut rt = Runtime::<2, 4, 2>::new();
    rt.bind(registration()).unwrap();
    rt.install(registration(), scope(), &namespace, "/", "/").unwrap();
    acquire(&mut rt, registration(), scope(), 1);
    let mut channel = Channel { current: None, replies: 0 };
    let root_request = |op, token, arg0, arg1, payload| api::Request {
        uid: 0,
        gid: 0,
        ..request(op, token, arg0, arg1, payload)
    };
    let baseline = namespace.shared().lock().open_handle_count();
    let opened = drive(&mut rt, &mut channel, root_request(
        api::OPEN, 0, api::ACCESS_READ | api::OPEN_DIRECTORY, 0, b"/root/Downloads",
    ));
    assert_eq!(opened.errno, 0);
    let directory = opened.result0;
    assert_eq!(drive(&mut rt, &mut channel, root_request(
        api::MKDIR, directory, 0o700, 0, b"subdir",
    )).errno, 0);
    let file = drive(&mut rt, &mut channel, root_request(
        api::OPEN, directory, api::ACCESS_WRITE | api::OPEN_CREATE, 0o600, b"subdir/old",
    ));
    assert_eq!(file.errno, 0);
    assert_eq!(drive(&mut rt, &mut channel, root_request(
        api::MKDIR, file.result0, 0o700, 0, b"denied",
    )).errno, 20);
    assert_eq!(drive(&mut rt, &mut channel, io(api::CLOSE, file.result0, 0, 0, b"")).errno, 0);
    let target = drive(&mut rt, &mut channel, root_request(
        api::OPEN, directory, api::ACCESS_READ | api::OPEN_DIRECTORY, 0, b"subdir",
    ));
    assert_eq!(target.errno, 0);
    let stale = drive(&mut rt, &mut channel, root_request(
        api::OPEN, directory, api::ACCESS_READ | api::OPEN_DIRECTORY, 0, b"subdir",
    ));
    assert_eq!(stale.errno, 0);
    assert_eq!(drive(&mut rt, &mut channel, io(api::CLOSE, stale.result0, 0, 0, b"")).errno, 0);
    assert_eq!(drive(&mut rt, &mut channel, root_request(
        api::RENAME, directory, stale.result0, api::RENAME_DISTINCT_DIRS, b"subdir/old\0new",
    )).errno, 9);
    assert_eq!(drive(&mut rt, &mut channel, root_request(
        api::STAT_AT, target.result0, 0, 0, b"old",
    )).errno, 0);
    assert_eq!(drive(&mut rt, &mut channel, root_request(
        api::RENAME, directory, target.result0, api::RENAME_DISTINCT_DIRS, b"subdir/old\0new",
    )).errno, 0);
    assert_eq!(drive(&mut rt, &mut channel, root_request(
        api::RENAME, target.result0, directory, api::RENAME_DISTINCT_DIRS, b"new\0../escaped",
    )).errno, 18);
    let stat = drive(&mut rt, &mut channel, root_request(
        api::STAT_AT, target.result0, 0, 0, b"./new",
    ));
    assert_eq!(stat.errno, 0);
    assert_eq!(stat.payload[18], 1);
    assert_eq!(drive(&mut rt, &mut channel, root_request(
        api::UNLINK, target.result0, 0, 0, b"new",
    )).errno, 0);
    assert_eq!(drive(&mut rt, &mut channel, io(api::CLOSE, target.result0, 0, 0, b"")).errno, 0);
    assert_eq!(drive(&mut rt, &mut channel, io(api::CLOSE, directory, 0, 0, b"")).errno, 0);
    assert_eq!(drive(&mut rt, &mut channel, root_request(
        api::MKDIR, directory, 0o700, 0, b"stale",
    )).errno, 9);
    assert_eq!(namespace.shared().lock().open_handle_count(), baseline);
}

#[test]
fn boot_root_seed_precedes_registration_and_writable_service_publication() {
    use crate::boot_root::{read_seed_file, BootRoot};
    use sunlight_fs::{Vfs, INITRAMFS};

    let mut vfs = Vfs::new();
    BootRoot::Writable.mount(&mut vfs, INITRAMFS).unwrap();
    let mut bytes = [0; 256];
    let count = read_seed_file(INITRAMFS, "/etc/resolv.conf", &mut bytes);
    assert!(core::str::from_utf8(&bytes[..count])
        .unwrap()
        .contains("nameserver 208.67.222.222"));
    let count = read_seed_file(INITRAMFS, "/etc/passwd", &mut bytes);
    assert_eq!(sunlight_fs::parse_passwd(&bytes[..count]).1, 2);
    let count = read_seed_file(INITRAMFS, "/etc/group", &mut bytes);
    assert_eq!(sunlight_fs::parse_group(&bytes[..count]).1, 7);
    let count = read_seed_file(INITRAMFS, "/etc/fstab", &mut bytes);
    assert!(count > 0);
    assert_eq!(vfs.open_handle_count(), 0);

    let namespace = DirectoryNamespace::mounted(vfs).unwrap();
    let shared = namespace.shared();
    let r = registration();
    let s = scope();
    let mut runtime = Runtime::<2, 2, 2>::new();
    assert_eq!(
        runtime.install(r, s, &namespace, "/", "/tmp"),
        Err(Error::Identity)
    );
    runtime.bind(r).unwrap();
    runtime.install(r, s, &namespace, "/", "/tmp").unwrap();
    let receipt = acquire(&mut runtime, r, s, 1);
    let mut channel = Channel {
        current: None,
        replies: 0,
    };
    let reply = drive(
        &mut runtime,
        &mut channel,
        request(
            api::OPEN,
            0,
            api::ACCESS_WRITE | api::OPEN_CREATE,
            0o600,
            b"c2i-test",
        ),
    );
    assert_eq!(reply.errno, 0);
    runtime.write_file(r, s.caller, reply.result0, b"root").unwrap();

    let stat = shared.lock().stat("/tmp/c2i-test").unwrap();
    assert_eq!((stat.uid, stat.gid, stat.size), (42, 43, 4));
    let read_only = runtime
        .open_file(r, receipt, "c2i-test", OWNER, Access::Read, || true)
        .unwrap();
    let mut contents = [0; 3];
    assert_eq!(
        runtime.read_at_file(r, s.caller, read_only, 1, &mut contents).unwrap(),
        3
    );
    assert_eq!(&contents, b"oot");

    runtime.retire_registration(r).unwrap();
    assert!(runtime.read_file(r, s.caller, read_only, &mut contents).is_err());
    // Withdrawal closes file descriptions. The delivered namespace lease
    // owns its two directory pins until the corresponding release arrives.
    assert_eq!(shared.lock().open_handle_count(), 2);
    runtime
        .request(r, &wire::Frame::Release(receipt).encode().unwrap())
        .unwrap();
    assert_eq!(shared.lock().open_handle_count(), 0);
    shared.lock().unlink("/tmp/c2i-test").unwrap();
    assert!(INITRAMFS.iter().all(|entry| entry.path != "/tmp/c2i-test"));
}

#[cfg(feature = "vfs-c2e-candidate")]
#[test]
fn protected_staging_publication_and_retirement_use_the_same_service_objects() {
    struct PublicationChannel {
        acknowledged: bool,
        errno: Option<u64>,
    }

    impl Control for PublicationChannel {
        fn snapshot(&mut self) -> Option<Registration> {
            None
        }
        fn status(&mut self, _: u64) -> Result<(Registration, bool), ()> {
            Err(())
        }
        fn next(&mut self, _: u64) -> Result<Option<[u8; wire::FRAME_BYTES]>, ()> {
            Ok(None)
        }
        fn reply(&mut self, _: u64, _: &[u8; wire::FRAME_BYTES]) -> bool {
            false
        }
        fn file_reply(&mut self, _: u64, bytes: &[u8; api::FRAME_BYTES]) -> bool {
            assert!(self.errno.is_none());
            self.errno = Some(api::Reply::decode(bytes).unwrap().errno);
            self.acknowledged
        }
    }

    const DESTINATION: &[u8] = b"/etc/sunlight/test-private";
    const TEMPORARY: &[u8] = b"/etc/sunlight/.test-private.tmp.synthetic";
    const PATHS: &[u8] = b"/etc/sunlight/.test-private.tmp.synthetic\0/etc/sunlight/test-private";
    let mut fs = sunlight_fs::Vfs::new();
    let tmpfs = Tmpfs::with_limits(&[], TmpfsLimits::new(2 * 4096, 16)).unwrap();
    fs.mount_tmpfs("/", tmpfs).unwrap();
    fs.mkdir("/etc", 0, 0, 0o755).unwrap();
    fs.mkdir("/etc/sunlight", 0, 0, 0o755).unwrap();
    let namespace = DirectoryNamespace::mounted(fs).unwrap();
    let shared = namespace.shared();
    let mut runtime = Runtime::<2, 5, 2>::new();
    let registration = registration();
    runtime.bind(registration).unwrap();
    runtime
        .install(registration, scope(), &namespace, "/", "/etc")
        .unwrap();
    let receipt = acquire(&mut runtime, registration, scope(), 1);
    let mut channel = Channel {
        current: None,
        replies: 0,
    };
    let stage_request = || {
        request(
            api::OPEN,
            0,
            api::OPEN_PRIVATE | api::ACCESS_WRITE | api::OPEN_CREATE | api::OPEN_EXCLUSIVE,
            0o600,
            TEMPORARY,
        )
    };
    let reopen_request = || {
        request(
            api::OPEN,
            0,
            api::OPEN_PRIVATE | api::ACCESS_READ,
            0,
            DESTINATION,
        )
    };
    let publish = |runtime: &mut Runtime<2, 5, 2>, token, replace, accepted, paths| {
        let request = api::Request {
            uid: OWNER.uid,
            gid: OWNER.gid,
            ..io(api::PUBLISH_PRIVATE, token, u64::from(replace), 0, paths)
        };
        let mut control = PublicationChannel {
            acknowledged: accepted,
            errno: None,
        };
        assert_eq!(
            runtime.dispatch_file(&mut control, registration, &request.encode().unwrap()),
            accepted
        );
        control.errno.unwrap()
    };
    let baseline = shared.lock().open_handle_count();
    assert_eq!(
        drive(
            &mut runtime,
            &mut channel,
            request(
                api::OPEN,
                0,
                api::ACCESS_WRITE | api::OPEN_CREATE,
                0o600,
                TEMPORARY
            )
        )
        .errno,
        13,
    );
    assert_eq!(drive(&mut runtime, &mut channel, reopen_request()).errno, 2);

    // An OPEN whose revalidation is refused rolls back the provisional inode.
    let bytes = stage_request().encode().unwrap();
    channel.current = None;
    let reply = runtime
        .file_request(&mut channel, registration, &bytes)
        .unwrap();
    assert_ne!(api::Reply::decode(&reply).unwrap().errno, 0);
    assert_eq!(shared.lock().open_handle_count(), baseline);
    assert!(matches!(
        shared.lock().stat(core::str::from_utf8(TEMPORARY).unwrap()),
        Err(FsError::NotFound)
    ));

    let stage = drive(&mut runtime, &mut channel, stage_request());
    assert_eq!(stage.errno, 0);
    assert_eq!(drive(&mut runtime, &mut channel, stage_request()).errno, 17);
    let token = stage.result0;
    let stat = drive(&mut runtime, &mut channel, io(api::STAT, token, 0, 0, &[]));
    assert_eq!(stat.errno, 0);
    assert_eq!(
        u16::from_le_bytes(stat.payload[16..18].try_into().unwrap()),
        0o100600
    );
    assert_eq!(
        drive(&mut runtime, &mut channel, io(api::READ, token, 0, 1, &[])).errno,
        13
    );
    assert_eq!(
        drive(
            &mut runtime,
            &mut channel,
            io(api::WRITE, token, 0, 0, &[1])
        )
        .result0,
        1
    );
    assert_eq!(publish(&mut runtime, token, false, true, PATHS), 0);
    assert_eq!(
        drive(
            &mut runtime,
            &mut channel,
            io(api::WRITE, token, 0, 0, &[2])
        )
        .errno,
        13
    );
    assert_eq!(
        drive(&mut runtime, &mut channel, io(api::CLOSE, token, 0, 0, &[])).errno,
        0
    );
    assert_eq!(publish(&mut runtime, token, true, true, PATHS), 9);

    let old = drive(&mut runtime, &mut channel, reopen_request()).result0;
    let wrong_credentials = api::Request {
        uid: OWNER.uid + 1,
        gid: OWNER.gid + 1,
        ..reopen_request()
    };
    assert_eq!(
        drive(&mut runtime, &mut channel, wrong_credentials).errno,
        13
    );
    let ordinary_read = request(api::OPEN, 0, api::ACCESS_READ, 0, b"sunlight/test-private");
    assert_eq!(drive(&mut runtime, &mut channel, ordinary_read).errno, 13);
    for (replace, accepted, expected) in [(false, true, 17), (true, false, 0), (true, true, 0)] {
        let stage = drive(&mut runtime, &mut channel, stage_request());
        assert_eq!(stage.errno, 0);
        assert_eq!(
            drive(
                &mut runtime,
                &mut channel,
                io(api::WRITE, stage.result0, 0, 0, &[2])
            )
            .result0,
            1
        );
        assert_eq!(
            publish(&mut runtime, stage.result0, replace, accepted, PATHS),
            expected
        );
        assert_eq!(
            drive(
                &mut runtime,
                &mut channel,
                io(api::CLOSE, stage.result0, 0, 0, &[])
            )
            .errno,
            0
        );
        let latest = drive(&mut runtime, &mut channel, reopen_request()).result0;
        let data = drive(&mut runtime, &mut channel, io(api::READ, latest, 0, 1, &[]));
        assert!(data.payload == [if replace && accepted { 2 } else { 1 }]);
        assert_eq!(
            drive(
                &mut runtime,
                &mut channel,
                io(api::CLOSE, latest, 0, 0, &[])
            )
            .errno,
            0
        );
        assert_eq!(shared.lock().open_handle_count(), baseline + 1);
    }
    // The pinned old inode and the committed replacement exhaust this
    // fixture's page budget. Closing an unwritten stage drops its inode.
    let unwritten = drive(&mut runtime, &mut channel, stage_request());
    assert_eq!(unwritten.errno, 0);
    assert_eq!(
        drive(
            &mut runtime,
            &mut channel,
            io(api::WRITE, unwritten.result0, 0, 0, &[3])
        )
        .errno,
        28
    );
    assert_eq!(
        drive(
            &mut runtime,
            &mut channel,
            io(api::CLOSE, unwritten.result0, 0, 0, &[])
        )
        .errno,
        0
    );
    assert_eq!(shared.lock().open_handle_count(), baseline + 1);
    assert!(drive(&mut runtime, &mut channel, io(api::READ, old, 0, 1, &[])).payload == [1]);
    assert_eq!(
        drive(&mut runtime, &mut channel, io(api::CLOSE, old, 0, 0, &[])).errno,
        0
    );

    // Abrupt retirement of the exact owner drops its stage. A later cleanup
    // for that token must never remove a replacement at the staging name.
    let stage = drive(&mut runtime, &mut channel, stage_request());
    assert_eq!(stage.errno, 0);
    let temporary = core::str::from_utf8(TEMPORARY).unwrap();
    {
        let mut fs = shared.lock();
        fs.unlink(temporary).unwrap();
        let replacement = fs
            .create_private_file_exclusive(temporary, OWNER.uid, OWNER.gid, 0o600)
            .unwrap();
        fs.close(replacement).unwrap();
    }
    assert_ne!(publish(&mut runtime, stage.result0, true, true, PATHS), 0);
    let different_incarnation = Caller {
        incarnation: scope().caller.incarnation + 1,
        ..scope().caller
    };
    runtime.retire_file_owner(different_incarnation);
    assert_eq!(shared.lock().open_handle_count(), baseline + 1);
    runtime.retire_file_owner(scope().caller);
    assert_eq!(shared.lock().open_handle_count(), baseline);
    assert!(shared.lock().stat(temporary).is_ok());
    shared.lock().unlink(temporary).unwrap();

    assert_eq!(drive(&mut runtime, &mut channel, stage_request()).errno, 0);
    runtime.retire_registration(registration).unwrap();
    assert!(shared.lock().stat(temporary).is_err());
    assert_eq!(shared.lock().open_handle_count(), baseline);
    runtime
        .request(
            registration,
            &wire::Frame::Release(receipt).encode().unwrap(),
        )
        .unwrap();
    assert_eq!(shared.lock().open_handle_count(), 0);
    let bytes = reopen_request().encode().unwrap();
    assert!(!runtime.dispatch_file(&mut channel, registration, &bytes));
}
