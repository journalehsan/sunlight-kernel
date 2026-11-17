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
