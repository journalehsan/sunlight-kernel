use super::control::Control;
use super::*;
use crate::open::Access;
use sunlight_fs::Credential;
use sunlight_ipc::vfs_rpc::{file_transport as api, retention::wire};
const OWNER: Credential = Credential { uid: 42, gid: 43 };

struct Channel {
    current: Option<[u8; api::FRAME_BYTES]>,
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
        true
    }
}

fn fixture<const N: usize>() -> (Runtime<2, N, 2>, DirectoryNamespace, Channel, Receipt) {
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
    let channel = Channel { current: None };
    (runtime, namespace, channel, receipt)
}

fn io(op: u64, token: u64, arg0: u64, arg1: u64, payload: &[u8]) -> api::Request<'_> {
    api::Request {
        registration: registration(),
        caller: scope().caller,
        request_id: 1,
        op,
        token,
        arg0,
        arg1,
        length: payload.len(),
        namespace_id: 0,
        namespace_generation: 0,
        receipt_id: 0,
        receipt_generation: 0,
        uid: 0,
        gid: 0,
        payload,
    }
}

fn pread(token: u64, offset: u64, length: u64) -> api::Request<'static> {
    io(api::PREAD, token, offset, length, &[])
}

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

/// Report the shared cursor without moving it.
fn cursor<const N: usize>(
    runtime: &mut Runtime<2, N, 2>,
    channel: &mut Channel,
    token: u64,
) -> u64 {
    let request = io(api::SEEK, token, api::SEEK_CURRENT, 0, &[]);
    drive(runtime, channel, request).result0
}

#[test]
fn explicit_offset_reads_leave_the_shared_cursor_alone() {
    let (mut rt, _ns, mut channel, receipt) = fixture::<4>();
    let token = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::ReadWrite,
            || true,
        )
        .unwrap();

    let read = drive(&mut rt, &mut channel, io(api::READ, token, 0, 2, &[]));
    assert_eq!((read.errno, read.result0), (0, 2));
    assert_eq!(read.payload.as_slice(), b"ab");

    for (offset, expected) in [(0u64, b"ab".as_slice()), (4, b"ef"), (1, b"bc")] {
        let at = drive(&mut rt, &mut channel, pread(token, offset, 2));
        assert_eq!((at.errno, at.result0), (0, 2));
        assert_eq!(at.payload.as_slice(), expected);
    }

    // The cursor is exactly where the last ordinary read left it.
    let read = drive(&mut rt, &mut channel, io(api::READ, token, 0, 2, &[]));
    assert_eq!((read.errno, read.result0), (0, 2));
    assert_eq!(read.payload.as_slice(), b"cd");

    // A positional read is not a seek for `SEEK_CURRENT` either.
    assert_eq!(cursor(&mut rt, &mut channel, token), 4);
    assert_eq!(
        drive(
            &mut rt,
            &mut channel,
            io(api::SEEK, token, api::SEEK_START, 1, &[]),
        )
        .result0,
        1
    );
    let at = drive(&mut rt, &mut channel, pread(token, 5, 1));
    assert_eq!(at.payload.as_slice(), b"f");
    assert_eq!(cursor(&mut rt, &mut channel, token), 1);
}

#[test]
fn duplicates_keep_one_cursor_and_independent_positional_reads() {
    let (mut rt, _ns, mut channel, receipt) = fixture::<4>();
    let a = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::ReadWrite,
            || true,
        )
        .unwrap();
    let read = drive(&mut rt, &mut channel, io(api::READ, a, 0, 2, &[]));
    assert_eq!(read.payload.as_slice(), b"ab");
    let b = drive(&mut rt, &mut channel, io(api::DUP, a, 0, 0, &[])).result0;
    assert_ne!(a, b);

    // A duplicate starts on the shared cursor, and a positional read on it
    // moves neither descriptor.
    let at = drive(&mut rt, &mut channel, pread(b, 1, 3));
    assert_eq!(at.payload.as_slice(), b"bcd");
    let read = drive(&mut rt, &mut channel, io(api::READ, a, 0, 1, &[]));
    assert_eq!(read.payload.as_slice(), b"c");

    let read = drive(&mut rt, &mut channel, io(api::READ, b, 0, 2, &[]));
    assert_eq!(read.payload.as_slice(), b"de");
    let at = drive(&mut rt, &mut channel, pread(a, 0, 6));
    assert_eq!(at.payload.as_slice(), b"abcdef");
    assert_eq!(cursor(&mut rt, &mut channel, a), 5);
    assert_eq!(cursor(&mut rt, &mut channel, b), 5);

    // Closing one row leaves the other's positional source intact.
    drive(&mut rt, &mut channel, io(api::CLOSE, a, 0, 0, &[]));
    let at = drive(&mut rt, &mut channel, pread(b, 3, 2));
    assert_eq!(at.payload.as_slice(), b"de");
    let stale = drive(&mut rt, &mut channel, pread(a, 0, 1));
    assert_eq!(stale.errno, 9);
}

#[test]
fn eof_partial_and_unrepresentable_offsets_report_exactly() {
    let (mut rt, _ns, mut channel, receipt) = fixture::<4>();
    let token = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::Read,
            || true,
        )
        .unwrap();

    let at = drive(&mut rt, &mut channel, pread(token, 6, 4));
    assert_eq!((at.errno, at.result0), (0, 0));
    assert!(at.payload.is_empty());
    let at = drive(&mut rt, &mut channel, pread(token, 4096, 4));
    assert_eq!((at.errno, at.result0), (0, 0));

    let at = drive(&mut rt, &mut channel, pread(token, 4, 4096));
    assert_eq!((at.errno, at.result0), (0, 2));
    assert_eq!(at.payload.as_slice(), b"ef");

    for offset in [u64::MAX, u64::MAX - 1, (usize::MAX as u64) - 2] {
        let at = drive(&mut rt, &mut channel, pread(token, offset, 4096));
        assert_eq!(at.errno, 22);
        assert!(at.payload.is_empty());
    }

    // A rejected range never disturbs the description.
    let read = drive(&mut rt, &mut channel, io(api::READ, token, 0, 6, &[]));
    assert_eq!((read.errno, read.result0), (0, 6));
    assert_eq!(read.payload.as_slice(), b"abcdef");
}

#[test]
fn rights_identity_and_stale_tokens_are_rechecked_per_request() {
    let (mut rt, _ns, mut channel, receipt) = fixture::<4>();
    let read = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::Read,
            || true,
        )
        .unwrap();
    let write = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::Write,
            || true,
        )
        .unwrap();

    let denied = drive(&mut rt, &mut channel, pread(write, 0, 1));
    assert_eq!(denied.errno, 13);

    for wrong in [
        Caller {
            pid: scope().caller.pid + 1,
            ..scope().caller
        },
        Caller {
            incarnation: scope().caller.incarnation + 1,
            ..scope().caller
        },
        Caller {
            address_space_generation: scope().caller.address_space_generation + 1,
            ..scope().caller
        },
    ] {
        let mut foreign = pread(read, 0, 1);
        foreign.caller = wrong;
        assert_eq!(drive(&mut rt, &mut channel, foreign).errno, 9);
        assert_eq!(
            rt.read_at_file(registration(), wrong, read, 0, &mut [0]),
            Err(Error::Filesystem(FsError::BadHandle))
        );
    }

    // The frame must carry the exact authenticated epoch before dispatch.
    let mut stale_epoch = pread(read, 0, 1);
    stale_epoch.registration = Registration {
        epoch: stale_epoch.registration.epoch + 1,
        ..stale_epoch.registration
    };
    let bytes = stale_epoch.encode().unwrap();
    channel.current = Some(bytes);
    assert!(rt
        .file_request(&mut channel, registration(), &bytes)
        .is_none());

    rt.close_file(registration(), scope().caller, read).unwrap();
    assert_eq!(drive(&mut rt, &mut channel, pread(read, 0, 1)).errno, 9);
    let at = drive(&mut rt, &mut channel, pread(write, 0, 1));
    assert_eq!(at.errno, 13);
}

#[test]
fn retained_positional_source_survives_rename_and_unlink() {
    let (mut rt, ns, mut channel, receipt) = fixture::<4>();
    let baseline = ns.shared().lock().open_handle_count();
    let token = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::ReadWrite,
            || true,
        )
        .unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), baseline + 1);

    ns.shared()
        .lock()
        .rename("/jail/cwd/data", "/jail/cwd/moved")
        .unwrap();
    let at = drive(&mut rt, &mut channel, pread(token, 0, 6));
    assert_eq!(at.payload.as_slice(), b"abcdef");

    ns.shared().lock().unlink("/jail/cwd/moved").unwrap();
    let replacement = ns
        .shared()
        .lock()
        .create_file_exclusive("/jail/cwd/data", 42, 43, 0o600)
        .unwrap();
    ns.shared()
        .lock()
        .write(replacement, 0, b"replacement")
        .unwrap();
    ns.shared().lock().close(replacement).unwrap();

    let at = drive(&mut rt, &mut channel, pread(token, 0, 4096));
    assert_eq!((at.errno, at.result0), (0, 6));
    assert_eq!(at.payload.as_slice(), b"abcdef");
    let stat = drive(&mut rt, &mut channel, io(api::STAT, token, 0, 0, &[]));
    assert_eq!(&stat.payload[..8], &6u64.to_le_bytes());
    // The replacement is a different object on the ordinary path.
    let fresh = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::Read,
            || true,
        )
        .unwrap();
    let at = drive(&mut rt, &mut channel, pread(fresh, 0, 2));
    assert_eq!(at.payload.as_slice(), b"re");

    drive(&mut rt, &mut channel, io(api::CLOSE, token, 0, 0, &[]));
    drive(&mut rt, &mut channel, io(api::CLOSE, fresh, 0, 0, &[]));
    assert_eq!(ns.shared().lock().open_handle_count(), baseline);
}

#[test]
fn caller_and_file_owner_retirement_release_descriptions_and_backend_pins() {
    let (mut rt, ns, mut channel, receipt) = fixture::<4>();
    let baseline = ns.shared().lock().open_handle_count();
    let token = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::Read,
            || true,
        )
        .unwrap();
    rt.duplicate_file(registration(), scope().caller, token)
        .unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), baseline + 1);
    assert_eq!(
        drive(&mut rt, &mut channel, pread(token, 0, 2))
            .payload
            .as_slice(),
        b"ab"
    );

    // Metadata/credential withdrawal is not a file teardown.
    rt.retire_caller(scope().caller);
    assert_eq!(drive(&mut rt, &mut channel, pread(token, 2, 2)).errno, 0);
    assert_eq!(ns.shared().lock().open_handle_count(), baseline + 1);

    rt.retire_file_owner(scope().caller);
    assert_eq!(drive(&mut rt, &mut channel, pread(token, 0, 2)).errno, 9);
    assert_eq!(ns.shared().lock().open_handle_count(), baseline);
}

#[test]
fn epoch_withdrawal_releases_descriptions_and_refuses_the_old_epoch() {
    let (mut rt, ns, mut channel, receipt) = fixture::<4>();
    let baseline = ns.shared().lock().open_handle_count();
    let token = rt
        .open_file(
            registration(),
            receipt,
            "data",
            OWNER,
            Access::Read,
            || true,
        )
        .unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), baseline + 1);
    assert_eq!(drive(&mut rt, &mut channel, pread(token, 0, 1)).errno, 0);

    // A newer authenticated snapshot retires the old epoch with its files.
    let next = Registration {
        epoch: registration().epoch + 1,
        ..registration()
    };
    rt.observe_active(next).unwrap();
    assert_eq!(ns.shared().lock().open_handle_count(), baseline);
    assert_eq!(
        rt.read_at_file(registration(), scope().caller, token, 0, &mut [0]),
        Err(Error::Identity)
    );
    assert_eq!(drive(&mut rt, &mut channel, pread(token, 0, 1)).errno, 5);
}
