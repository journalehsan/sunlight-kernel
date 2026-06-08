#![no_std]
#![no_main]

use sunlight_fs::{FileHandle, FileType, FsError, RamFs, Vfs, INITRAMFS};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg, VfsMsg,
};

const STATUS_OK: u64 = 0;
const ERR_NOT_FOUND: u64 = 2;
const ERR_BAD_HANDLE: u64 = 9;
const ERR_INVALID: u64 = 22;
const MAX_PATH_BYTES: usize = 32;
const READ_REPLY_BYTES: usize = 16;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[VFS]  VFS server started");

    let ep = endpoint_create();
    nameserver_register("vfs", ep);
    debug_log("[VFS]  Registered as 'vfs'");

    let mut vfs = Vfs::new();
    let _ = vfs.mount_ramfs("/", RamFs::new(INITRAMFS));

    run_boot_self_tests(&mut vfs);

    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle_request(&mut vfs, msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

fn handle_request(vfs: &mut Vfs, msg: IpcMsg) -> IpcMsg {
    match msg.label {
        VfsMsg::OPEN => match decoded_path(&msg.words) {
            Some(path_buf) => match vfs.open(path_buf.as_str()) {
                Ok(handle) => ok_reply().word(1, handle.0 as u64),
                Err(err) => error_reply(err),
            },
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::READ => {
            let handle = FileHandle(msg.words[0] as u32);
            let offset = msg.words[1] as usize;
            let requested = (msg.words[2] as usize).min(READ_REPLY_BYTES);
            let mut buf = [0u8; READ_REPLY_BYTES];
            match vfs.read(handle, offset, &mut buf[..requested]) {
                Ok(read) => {
                    let mut reply = ok_reply().word(1, read as u64);
                    reply.words[2] = pack_bytes(&buf[0..8]);
                    reply.words[3] = pack_bytes(&buf[8..16]);
                    reply.word_count = 4;
                    reply
                }
                Err(err) => error_reply(err),
            }
        }
        VfsMsg::CLOSE => match vfs.close(FileHandle(msg.words[0] as u32)) {
            Ok(()) => ok_reply(),
            Err(err) => error_reply(err),
        },
        VfsMsg::STAT => match decoded_path(&msg.words) {
            Some(path_buf) => match vfs.stat(path_buf.as_str()) {
                Ok(stat) => ok_reply()
                    .word(1, stat.size as u64)
                    .word(2, file_type_code(stat.file_type)),
                Err(err) => error_reply(err),
            },
            None => error_reply(FsError::InvalidPath),
        },
        _ => error_reply(FsError::Unsupported),
    }
}

fn run_boot_self_tests(vfs: &mut Vfs) {
    debug_log("[VFS]  Test open /etc/motd");
    let open_reply = handle_request(vfs, path_msg(VfsMsg::OPEN, "/etc/motd"));
    let motd = if open_reply.label == VfsMsg::REPLY && open_reply.words[0] == STATUS_OK {
        FileHandle(open_reply.words[1] as u32)
    } else {
        return;
    };

    debug_log("[VFS]  Test read /etc/motd");
    let mut buf = [0u8; 32];
    let first = handle_request(vfs, read_msg(motd, 0, READ_REPLY_BYTES));
    let second = handle_request(vfs, read_msg(motd, READ_REPLY_BYTES, READ_REPLY_BYTES));
    if first.label != VfsMsg::REPLY || second.label != VfsMsg::REPLY {
        return;
    }
    let first_len = first.words[1] as usize;
    let second_len = second.words[1] as usize;
    unpack_data(&first, &mut buf[..first_len]);
    unpack_data(&second, &mut buf[first_len..first_len + second_len]);
    if &buf[..first_len + second_len] == b"Welcome to SunlightOS\n" {
        debug_log("[VFS]  Read: \"Welcome to SunlightOS\\n\"");
    } else {
        return;
    }
    let _ = handle_request(
        vfs,
        IpcMsg::with_label(VfsMsg::CLOSE).word(0, motd.0 as u64),
    );

    let missing = handle_request(vfs, path_msg(VfsMsg::OPEN, "/missing"));
    if missing.label == VfsMsg::ERROR && missing.words[0] == ERR_NOT_FOUND {
        debug_log("[VFS]  ENOENT test OK");
    } else {
        return;
    }

    let bad_handle = handle_request(vfs, read_msg(FileHandle(0), 0, 8));
    if bad_handle.label == VfsMsg::ERROR && bad_handle.words[0] == ERR_BAD_HANDLE {
        debug_log("[VFS]  Bad handle test OK");
    } else {
        return;
    }

    let stat = handle_request(vfs, path_msg(VfsMsg::STAT, "/etc/sunlight/session.toml"));
    if stat.label == VfsMsg::REPLY
        && stat.words[1] > 0
        && stat.words[2] == file_type_code(FileType::File)
    {
        debug_log("[VFS]  Stat OK");
        debug_log("[SunlightOS] Phase 3.0 OK");
    }
}

fn ok_reply() -> IpcMsg {
    IpcMsg::with_label(VfsMsg::REPLY).word(0, STATUS_OK)
}

fn error_reply(err: FsError) -> IpcMsg {
    IpcMsg::with_label(VfsMsg::ERROR).word(0, errno(err))
}

fn errno(err: FsError) -> u64 {
    match err {
        FsError::NotFound => ERR_NOT_FOUND,
        FsError::BadHandle => ERR_BAD_HANDLE,
        FsError::InvalidPath => ERR_INVALID,
        _ => ERR_INVALID,
    }
}

fn file_type_code(file_type: FileType) -> u64 {
    match file_type {
        FileType::File => 1,
        FileType::Directory => 2,
    }
}

struct PathBuf {
    bytes: [u8; MAX_PATH_BYTES],
    len: usize,
}

impl PathBuf {
    fn as_str(&self) -> &str {
        // SAFETY: PathBuf is only constructed by decoded_path after UTF-8 validation.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.len]) }
    }
}

fn decoded_path(words: &[u64; 8]) -> Option<PathBuf> {
    let mut bytes = [0u8; MAX_PATH_BYTES];
    let mut idx = 0;
    while idx < 4 {
        bytes[idx * 8..idx * 8 + 8].copy_from_slice(&words[idx].to_le_bytes());
        idx += 1;
    }
    let len = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(MAX_PATH_BYTES);
    if len == 0 {
        return None;
    }
    core::str::from_utf8(&bytes[..len]).ok()?;
    Some(PathBuf { bytes, len })
}

fn pack_bytes(bytes: &[u8]) -> u64 {
    let mut out = 0u64;
    let mut idx = 0;
    while idx < bytes.len() && idx < 8 {
        out |= (bytes[idx] as u64) << (idx * 8);
        idx += 1;
    }
    out
}

fn path_msg(label: u64, path: &str) -> IpcMsg {
    let bytes = path.as_bytes();
    let mut msg = IpcMsg::with_label(label);
    let mut word_idx = 0;
    while word_idx < 4 {
        let start = word_idx * 8;
        let end = (start + 8).min(bytes.len());
        if start < bytes.len() {
            msg = msg.word(word_idx, pack_bytes(&bytes[start..end]));
        }
        word_idx += 1;
    }
    msg
}

fn read_msg(handle: FileHandle, offset: usize, len: usize) -> IpcMsg {
    IpcMsg::with_label(VfsMsg::READ)
        .word(0, handle.0 as u64)
        .word(1, offset as u64)
        .word(2, len as u64)
}

fn unpack_data(msg: &IpcMsg, out: &mut [u8]) {
    let mut idx = 0;
    while idx < out.len() {
        let word = if idx < 8 { msg.words[2] } else { msg.words[3] };
        out[idx] = ((word >> ((idx % 8) * 8)) & 0xff) as u8;
        idx += 1;
    }
}
