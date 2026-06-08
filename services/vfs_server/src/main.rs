#![no_std]
#![no_main]

use sunlight_fat::{FatSharePage, FAT_SHARE_VADDR, SHARE_MAGIC};
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

// Handle encoding: high byte = mount (0=ram, 1=boot), lower bytes = local handle
const MOUNT_RAM: u32 = 0;
const MOUNT_BOOT: u32 = 1;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

// ---------------------------------------------------------------------------
// Boot filesystem backed by the kernel-populated FAT32 share page
// ---------------------------------------------------------------------------

/// Max open handles for the boot filesystem.
const BOOT_MAX_HANDLES: usize = 16;

struct BootHandle {
    file_idx: u8,  // index into share.files
    in_use: bool,
}

struct BootFs {
    share: &'static FatSharePage,
    handles: [BootHandle; BOOT_MAX_HANDLES],
}

impl BootFs {
    /// Read share page at FAT_SHARE_VADDR. Returns None if magic is wrong or
    /// no files were loaded (block device not present).
    ///
    /// SAFETY: The kernel must have mapped the share page at FAT_SHARE_VADDR before
    /// this process starts. The page is read-only from the vfs_server's perspective.
    unsafe fn new() -> Option<Self> {
        let share = &*(FAT_SHARE_VADDR as *const FatSharePage);
        if share.magic != SHARE_MAGIC || share.count == 0 {
            return None;
        }
        Some(BootFs {
            share,
            handles: core::array::from_fn(|_| BootHandle { file_idx: 0, in_use: false }),
        })
    }

    /// Look up a local path (e.g. "/HELLO.TXT") in the share page.
    fn find_file(&self, local_path: &str) -> Option<usize> {
        let needle = local_path.as_bytes();
        for idx in 0..self.share.count as usize {
            let f = &self.share.files[idx];
            if f.path_bytes() == needle {
                return Some(idx);
            }
        }
        None
    }

    fn open(&mut self, local_path: &str) -> Result<FileHandle, FsError> {
        let idx = self.find_file(local_path).ok_or(FsError::NotFound)?;
        for (h, slot) in self.handles.iter_mut().enumerate() {
            if !slot.in_use {
                slot.file_idx = idx as u8;
                slot.in_use = true;
                return Ok(pack_handle(MOUNT_BOOT, FileHandle((h + 1) as u32)));
            }
        }
        Err(FsError::TooManyOpenFiles)
    }

    fn read(
        &mut self,
        local_handle: FileHandle,
        offset: usize,
        buf: &mut [u8],
    ) -> Result<usize, FsError> {
        let h = local_handle.0.checked_sub(1).ok_or(FsError::BadHandle)? as usize;
        let slot = self.handles.get(h).ok_or(FsError::BadHandle)?;
        if !slot.in_use {
            return Err(FsError::BadHandle);
        }
        let data = self.share.files[slot.file_idx as usize].data_bytes();
        if offset >= data.len() {
            return Ok(0);
        }
        let src = &data[offset..];
        let len = src.len().min(buf.len());
        buf[..len].copy_from_slice(&src[..len]);
        Ok(len)
    }

    fn close(&mut self, local_handle: FileHandle) -> Result<(), FsError> {
        let h = local_handle.0.checked_sub(1).ok_or(FsError::BadHandle)? as usize;
        let slot = self.handles.get_mut(h).ok_or(FsError::BadHandle)?;
        if !slot.in_use {
            return Err(FsError::BadHandle);
        }
        slot.in_use = false;
        Ok(())
    }

    fn stat(&self, local_path: &str) -> Result<(usize, FileType), FsError> {
        let idx = self.find_file(local_path).ok_or(FsError::NotFound)?;
        Ok((self.share.files[idx].data_len as usize, FileType::File))
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

struct State {
    vfs: Vfs,
    boot: Option<BootFs>,
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[VFS]  VFS server started");

    let ep = endpoint_create();
    nameserver_register("vfs", ep);
    debug_log("[VFS]  Registered as 'vfs'");

    // Root filesystem (RamFs)
    let mut vfs = Vfs::new();
    let _ = vfs.mount_ramfs("/", RamFs::new(INITRAMFS));

    // Boot filesystem from kernel-populated FAT32 share page
    // SAFETY: Kernel mapped the share page before starting this process.
    let boot = unsafe { BootFs::new() };

    let mut state = State { vfs, boot };

    // Phase 3.0 self-tests (RamFs)
    run_phase30_tests(&mut state);

    // Phase 3.5 self-tests (/boot mount)
    run_phase35_tests(&mut state);

    // IPC server loop
    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle_request(&mut state, msg);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

// ---------------------------------------------------------------------------
// Request routing
// ---------------------------------------------------------------------------

fn handle_request(state: &mut State, msg: IpcMsg) -> IpcMsg {
    match msg.label {
        VfsMsg::OPEN => match decoded_path(&msg.words) {
            Some(pb) => open_path(state, pb.as_str()),
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::READ => {
            let raw_handle = FileHandle(msg.words[0] as u32);
            let offset = msg.words[1] as usize;
            let requested = (msg.words[2] as usize).min(READ_REPLY_BYTES);
            read_handle(state, raw_handle, offset, requested)
        }
        VfsMsg::CLOSE => close_handle(state, FileHandle(msg.words[0] as u32)),
        VfsMsg::STAT => match decoded_path(&msg.words) {
            Some(pb) => stat_path(state, pb.as_str()),
            None => error_reply(FsError::InvalidPath),
        },
        _ => error_reply(FsError::Unsupported),
    }
}

/// Open a VFS path, routing /boot/* to BootFs.
fn open_path(state: &mut State, path: &str) -> IpcMsg {
    if let Some(local) = strip_boot_prefix(path) {
        match state.boot.as_mut() {
            Some(boot) => match boot.open(local) {
                Ok(handle) => ok_reply().word(1, handle.0 as u64),
                Err(e) => error_reply(e),
            },
            None => error_reply(FsError::NotFound),
        }
    } else {
        match state.vfs.open(path) {
            Ok(handle) => ok_reply().word(1, pack_handle(MOUNT_RAM, handle).0 as u64),
            Err(e) => error_reply(e),
        }
    }
}

fn read_handle(state: &mut State, raw: FileHandle, offset: usize, requested: usize) -> IpcMsg {
    let (mount, local) = unpack_handle(raw);
    match mount {
        MOUNT_BOOT => {
            if let Some(boot) = state.boot.as_mut() {
                let mut buf = [0u8; READ_REPLY_BYTES];
                match boot.read(local, offset, &mut buf[..requested]) {
                    Ok(n) => {
                        let mut reply = ok_reply().word(1, n as u64);
                        reply.words[2] = pack_bytes(&buf[0..8]);
                        reply.words[3] = pack_bytes(&buf[8..16]);
                        reply.word_count = 4;
                        reply
                    }
                    Err(e) => error_reply(e),
                }
            } else {
                error_reply(FsError::BadHandle)
            }
        }
        MOUNT_RAM => {
            let mut buf = [0u8; READ_REPLY_BYTES];
            match state.vfs.read(local, offset, &mut buf[..requested]) {
                Ok(n) => {
                    let mut reply = ok_reply().word(1, n as u64);
                    reply.words[2] = pack_bytes(&buf[0..8]);
                    reply.words[3] = pack_bytes(&buf[8..16]);
                    reply.word_count = 4;
                    reply
                }
                Err(e) => error_reply(e),
            }
        }
        _ => error_reply(FsError::BadHandle),
    }
}

fn close_handle(state: &mut State, raw: FileHandle) -> IpcMsg {
    let (mount, local) = unpack_handle(raw);
    match mount {
        MOUNT_BOOT => match state.boot.as_mut() {
            Some(boot) => match boot.close(local) {
                Ok(()) => ok_reply(),
                Err(e) => error_reply(e),
            },
            None => error_reply(FsError::BadHandle),
        },
        MOUNT_RAM => match state.vfs.close(local) {
            Ok(()) => ok_reply(),
            Err(e) => error_reply(e),
        },
        _ => error_reply(FsError::BadHandle),
    }
}

fn stat_path(state: &mut State, path: &str) -> IpcMsg {
    if let Some(local) = strip_boot_prefix(path) {
        match state.boot.as_ref() {
            Some(boot) => match boot.stat(local) {
                Ok((size, ft)) => ok_reply()
                    .word(1, size as u64)
                    .word(2, file_type_code(ft)),
                Err(e) => error_reply(e),
            },
            None => error_reply(FsError::NotFound),
        }
    } else {
        match state.vfs.stat(path) {
            Ok(stat) => ok_reply()
                .word(1, stat.size as u64)
                .word(2, file_type_code(stat.file_type)),
            Err(e) => error_reply(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 3.0 self-tests (RamFs gate)
// ---------------------------------------------------------------------------

fn run_phase30_tests(state: &mut State) {
    debug_log("[VFS]  Test open /etc/motd");
    let open_reply = handle_request(state, path_msg(VfsMsg::OPEN, "/etc/motd"));
    let motd = if open_reply.label == VfsMsg::REPLY && open_reply.words[0] == STATUS_OK {
        FileHandle(open_reply.words[1] as u32)
    } else {
        return;
    };

    debug_log("[VFS]  Test read /etc/motd");
    let mut buf = [0u8; 32];
    let first = handle_request(state, read_msg(motd, 0, READ_REPLY_BYTES));
    let second = handle_request(state, read_msg(motd, READ_REPLY_BYTES, READ_REPLY_BYTES));
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
        state,
        IpcMsg::with_label(VfsMsg::CLOSE).word(0, motd.0 as u64),
    );

    let missing = handle_request(state, path_msg(VfsMsg::OPEN, "/missing"));
    if missing.label == VfsMsg::ERROR && missing.words[0] == ERR_NOT_FOUND {
        debug_log("[VFS]  ENOENT test OK");
    } else {
        return;
    }

    let bad_handle = handle_request(state, read_msg(FileHandle(0), 0, 8));
    if bad_handle.label == VfsMsg::ERROR && bad_handle.words[0] == ERR_BAD_HANDLE {
        debug_log("[VFS]  Bad handle test OK");
    } else {
        return;
    }

    let stat = handle_request(state, path_msg(VfsMsg::STAT, "/etc/sunlight/session.toml"));
    if stat.label == VfsMsg::REPLY
        && stat.words[1] > 0
        && stat.words[2] == file_type_code(FileType::File)
    {
        debug_log("[VFS]  Stat OK");
        debug_log("[SunlightOS] Phase 3.0 OK");
    }
}

// ---------------------------------------------------------------------------
// Phase 3.5 self-tests (/boot gate)
// ---------------------------------------------------------------------------

fn run_phase35_tests(state: &mut State) {
    if state.boot.is_none() {
        return;
    }
    debug_log("[VFS]  /boot OK");

    // Read /boot/HELLO.TXT → "SunlightOS FAT32 boot volume\n"
    let open1 = handle_request(state, path_msg(VfsMsg::OPEN, "/boot/HELLO.TXT"));
    if open1.label != VfsMsg::REPLY || open1.words[0] != STATUS_OK {
        return;
    }
    let h1 = FileHandle(open1.words[1] as u32);
    let mut buf1 = [0u8; 64];
    let read1a = handle_request(state, read_msg(h1, 0, READ_REPLY_BYTES));
    let read1b = handle_request(state, read_msg(h1, READ_REPLY_BYTES, READ_REPLY_BYTES));
    if read1a.label == VfsMsg::REPLY && read1b.label == VfsMsg::REPLY {
        let la = read1a.words[1] as usize;
        let lb = read1b.words[1] as usize;
        unpack_data(&read1a, &mut buf1[..la]);
        unpack_data(&read1b, &mut buf1[la..la + lb]);
        let total = la + lb;
        if &buf1[..total] == b"SunlightOS FAT32 boot volume\n" {
            debug_log("[VFS]  Read: \"SunlightOS FAT32 boot volume\\n\"");
        }
    }
    let _ = handle_request(state, IpcMsg::with_label(VfsMsg::CLOSE).word(0, h1.0 as u64));

    // Read /boot/BOOT/PHASE35.TXT → "Phase 3.5 FAT32 OK\n" (19 bytes, two read calls)
    let open2 = handle_request(state, path_msg(VfsMsg::OPEN, "/boot/BOOT/PHASE35.TXT"));
    if open2.label != VfsMsg::REPLY || open2.words[0] != STATUS_OK {
        return;
    }
    let h2 = FileHandle(open2.words[1] as u32);
    let mut buf2 = [0u8; 32];
    let read2a = handle_request(state, read_msg(h2, 0, READ_REPLY_BYTES));
    let read2b = handle_request(state, read_msg(h2, READ_REPLY_BYTES, READ_REPLY_BYTES));
    if read2a.label == VfsMsg::REPLY && read2b.label == VfsMsg::REPLY {
        let na = read2a.words[1] as usize;
        let nb = read2b.words[1] as usize;
        unpack_data(&read2a, &mut buf2[..na]);
        unpack_data(&read2b, &mut buf2[na..na + nb]);
        let total = na + nb;
        if &buf2[..total] == b"Phase 3.5 FAT32 OK\n" {
            debug_log("[VFS]  Read: \"Phase 3.5 FAT32 OK\\n\"");
        }
    }
    let _ = handle_request(state, IpcMsg::with_label(VfsMsg::CLOSE).word(0, h2.0 as u64));

    // ENOENT test for /boot/MISSING.TXT
    let missing = handle_request(state, path_msg(VfsMsg::OPEN, "/boot/MISSING.TXT"));
    if missing.label == VfsMsg::ERROR && missing.words[0] == ERR_NOT_FOUND {
        debug_log("[VFS]  /boot/MISSING.TXT ENOENT OK");
    } else {
        return;
    }

    debug_log("[SunlightOS] Phase 3.5 OK");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Strip the "/boot" prefix from a path; returns the local path (e.g. "/HELLO.TXT").
/// Returns None if path does not start with "/boot".
fn strip_boot_prefix(path: &str) -> Option<&str> {
    if path == "/boot" {
        Some("/")
    } else if path.starts_with("/boot/") {
        // path[5..] starts with '/' → gives the local path e.g. "/HELLO.TXT"
        Some(&path[5..])
    } else {
        None
    }
}

fn pack_handle(mount: u32, local: FileHandle) -> FileHandle {
    FileHandle((mount << 28) | (local.0 & 0x0FFF_FFFF))
}

fn unpack_handle(handle: FileHandle) -> (u32, FileHandle) {
    let mount = handle.0 >> 28;
    let local = handle.0 & 0x0FFF_FFFF;
    (mount, FileHandle(local))
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
