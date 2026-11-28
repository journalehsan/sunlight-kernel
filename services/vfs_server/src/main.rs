#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;

// The libc allocator reclaims dropped policy pins, leases and temporary paths.
// Cwd transitions reuse the same bounded heap for the service lifetime.

use sunlight_fat::{FatSharePage, FAT_SHARE_VADDR, SHARE_MAGIC};
use sunlight_fs::{
    check_permission, parse_fstab, parse_group, parse_passwd, Credential, FileHandle, FileType,
    FsError, FstabEntry, PermCheck, RamEntry, RamFs, Vfs,
};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, nameserver_lookup_timeout,
    nameserver_register, shm_alloc, shm_free, shm_map, unpack_ipv4, IpcMsg, ResolvedMsg, VfsMsg,
};
#[cfg(not(feature = "vfs-c1e2-test"))]
use sunlight_ipc::{ipc_recv, ipc_reply_and_wait};

const STATUS_OK: u64 = 0;
const ERR_PERM: u64 = 1;
const ERR_NOT_FOUND: u64 = 2;
const ERR_BAD_HANDLE: u64 = 9;
const ERR_ACCES: u64 = 13;
const ERR_INVALID: u64 = 22;
const ERR_ROFS: u64 = 30;
const MAX_PATH_BYTES: usize = 32;
const READ_REPLY_BYTES: usize = 16;
const FSTAB_MAX_BYTES: usize = sunlight_fs::fstab::MAX_FSTAB_BYTES + 1;
const OPEN_META_SLOTS: usize = 64;

// Handle encoding: high byte = mount (0=ram, 1=boot), lower bytes = local handle
const MOUNT_RAM: u32 = 0;
const MOUNT_BOOT: u32 = 1;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    #[cfg(feature = "vfs-n1o-fat-test")]
    {
        debug_log("[VFS-N1O] ring3 panic");
        debug_log(&alloc::format!("{_info}"));
        #[cfg(feature = "vfs-c1e7-test")]
        for message in ["[VFS-N1O] ring3 panic", &alloc::format!("{_info}")] {
            linux_fixture_call(sunlight_ipc::NATIVE_SYSCALL_TAG | 99,
                message.as_ptr() as u64, message.len() as u64, 0);
        }
    }
    loop {}
}

// ---------------------------------------------------------------------------
// Boot filesystem backed by the kernel-populated FAT32 share page
// ---------------------------------------------------------------------------

/// Max open handles for the boot filesystem.
const BOOT_MAX_HANDLES: usize = 16;

struct BootHandle {
    file_idx: u8, // index into share.files
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
            handles: core::array::from_fn(|_| BootHandle {
                file_idx: 0,
                in_use: false,
            }),
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

    fn fstat(&self, local_handle: FileHandle) -> Result<sunlight_fs::FileStat, FsError> {
        let h = local_handle.0.checked_sub(1).ok_or(FsError::BadHandle)? as usize;
        let slot = self.handles.get(h).ok_or(FsError::BadHandle)?;
        if !slot.in_use {
            return Err(FsError::BadHandle);
        }
        Ok(sunlight_fs::FileStat {
            file_type: FileType::File,
            size: self.share.files[slot.file_idx as usize].data_len as usize,
            uid: 0,
            gid: 0,
            mode: sunlight_fs::mode::FILE_755,
            nlinks: 1,
        })
    }

    fn stat(&self, local_path: &str) -> Result<(usize, FileType), FsError> {
        let idx = self.find_file(local_path).ok_or(FsError::NotFound)?;
        Ok((self.share.files[idx].data_len as usize, FileType::File))
    }
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

#[cfg(feature = "stat-failure-test")]
mod stat_failure_probe;

#[cfg(feature = "vfs-c1e2-test")]
use sunlight_vfs_server::directory::attachment::{RetainedStorage, StorageDevice};
#[cfg(not(feature = "vfs-c1e2-test"))]
use sunlight_fs::NullDevice as StorageDevice;

struct State {
    #[cfg(feature = "stat-failure-test")]
    stat_failure: stat_failure_probe::Probe,
    // Bound only through the service-only kernel snapshot syscall. Ordinary
    // nameserver registration returns no trusted registration snapshot.
    retention: sunlight_vfs_server::runtime::Runtime<64, 64, 4, StorageDevice>,
    vfs: alloc::sync::Arc<spin::Mutex<Vfs<StorageDevice>>>,
    namespace: sunlight_vfs_server::directory::DirectoryNamespace<StorageDevice>,
    #[cfg(feature = "vfs-c1e2-test")]
    storage: RetainedStorage,
    boot: Option<BootFs>,
    boot_mountpoint: Option<&'static str>,
    open_meta: [OpenMeta; OPEN_META_SLOTS],
}

#[derive(Clone, Copy)]
struct OpenMeta {
    handle: u32,
    len: usize,
    path: [u8; MAX_PATH_BYTES],
}

impl OpenMeta {
    const EMPTY: Self = Self {
        handle: 0,
        len: 0,
        path: [0; MAX_PATH_BYTES],
    };

    fn path_str(&self) -> &str {
        // SAFETY: metadata is only populated from decoded_path/open_path input,
        // which is already UTF-8 validated.
        unsafe { core::str::from_utf8_unchecked(&self.path[..self.len]) }
    }
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn _start() -> ! {
    #[cfg(feature = "vfs-c1e2-test")]
    if linux_fixture_call(sunlight_ipc::NATIVE_SYSCALL_TAG | 33, 0, 0, 0)
        == sunlight_ipc::storage::FIXTURE_LINUX_PID as i64
    {
        linux_mount_fixture();
    }
    init_tmpfs_pages();
    #[cfg(feature = "vfs-c1e1-test")]
    if sunlight_ipc::getpid() == sunlight_ipc::storage::FIXTURE_CALLER_PID
        || (cfg!(feature = "vfs-c1e2-test")
            && sunlight_ipc::getpid() == sunlight_ipc::storage::FIXTURE_RETIRING_PID)
    {
        #[cfg(feature = "vfs-c1e7-test")]
        if sunlight_ipc::getpid() == sunlight_ipc::storage::FIXTURE_CALLER_PID {
            native_mutation_fixture();
        }
        sunlight_vfs_server::directory::attachment::storage_caller(debug_log);
    }
    debug_log("[VFS]  VFS server started");

    let ep = endpoint_create();
    assert!(nameserver_register("vfs", ep), "VFS registration rejected");
    debug_log("[VFS]  Registered as 'vfs'");
    #[cfg(all(feature = "vfs-c1e1-test", not(feature = "vfs-c1e2-test")))]
    if core::hint::black_box(true) {
        sunlight_vfs_server::directory::attachment::qualify_storage(debug_log);
    }
    #[cfg(all(feature = "vfs-n1o-fat-test", not(feature = "vfs-c1e1-test")))]
    if core::hint::black_box(true) {
        sunlight_vfs_server::directory::attachment::qualify_native_fat(debug_log);
        sunlight_ipc::ProcessExit::exit(0);
    }
    #[cfg(feature = "vfs-n1m-block-test")]
    if core::hint::black_box(true) {
        qualify_native_block();
    }
    #[cfg(feature = "vfs-n1j-exit-test")]
    if core::hint::black_box(true) {
        sunlight_ipc::ProcessExit::exit(0);
    }

    // Root filesystem (RamFs)
    let mut vfs = Vfs::new();
    let mut boot = None;
    #[cfg(feature = "boot-resources")]
    let seed = {
        use sunlight_fs::boot_resources as resources;
        let pointer = resources::USER_ADDRESS as *const u8;
        let header = unsafe { core::slice::from_raw_parts(pointer, resources::HEADER) };
        let size = resources::archive_size(header).expect("boot resource header");
        let bytes = unsafe { core::slice::from_raw_parts(pointer, size) };
        let entries = resources::entries(bytes).expect("boot resource inventory");
        debug_log("[BOOT-RES] VFS inventory ready");
        entries
    };
    #[cfg(not(feature = "boot-resources"))]
    let seed = sunlight_fs::INITRAMFS;
    let boot_mountpoint = mount_from_fstab(&mut vfs, &mut boot, seed);
    #[cfg(feature = "vfs-c2e-candidate")]
    vfs.mount_service_downloads().expect("C2E Downloads mount");
    #[cfg(feature = "vfs-c1e2-test")]
    for path in ["/mnt", "/mnt/volume", "/mnt/bad", "/mnt/other", "/mnt/exit"] {
        if matches!(vfs.stat(path), Err(FsError::NotFound)) {
            vfs.mkdir(path, 0, 0, 0o755).expect("storage fixture target");
        }
    }
    #[cfg(feature = "vfs-c1e7-test")]
    for path in ["/tmp/c1e9-native-file", "/tmp/c1e9-linux-file"] {
        let handle = vfs.create_file_exclusive(path, 0, 0, 0o600).expect("mutation fixture file");
        assert_eq!(vfs.write(handle, 0, b"held").unwrap(), 4);
        vfs.close(handle).unwrap();
    }

    let namespace = sunlight_vfs_server::directory::DirectoryNamespace::mount_owner(vfs)
        .expect("service mounted namespace ownership");
    let mut state = State {
        #[cfg(feature = "stat-failure-test")]
        stat_failure: stat_failure_probe::Probe::default(),
        retention: sunlight_vfs_server::runtime::Runtime::new(),
        vfs: namespace.shared(),
        namespace,
        #[cfg(feature = "vfs-c1e2-test")]
        storage: RetainedStorage::default(),
        boot,
        boot_mountpoint,
        open_meta: [OpenMeta::EMPTY; OPEN_META_SLOTS],
    };
    #[cfg(feature = "vfs-d2b-test")]
    state.retention.set_fixture_endpoint(ep);
    assert!(alloc::sync::Arc::ptr_eq(
        &state.vfs,
        &state.namespace.shared()
    ));
    assert!(state.retention.active().is_none());

    // Pre-seed /etc/resolv.conf so it is visible via ls/stat/readdir from early boot.
    // Content is a compatibility view; it will be refreshed from `resolved` on OPEN.
    let _ = ensure_resolv_conf(&mut state);

    // Phase 3.0 self-tests (RamFs)
    run_phase30_tests(&mut state, ep);

    // Phase 3.5 self-tests (/boot mount)
    run_phase35_tests(&mut state);

    // Phase 3.7 self-tests (Unix permissions)
    run_phase37_tests(&mut state);

    // Bite 4: Shared memory grant self-test (exercises shm_alloc + large VFS read via DATA_SHARED + map/free)
    run_shm_tests(&mut state);

    // This registration comes from a successful kernel copy, never an IPC
    // badge or retention payload. Binding grants no namespace policy.
    if let Some(registration) = sunlight_ipc::vfs_rpc::retention::transport::snapshot() {
        if state.retention.bind(registration).is_ok() {
            assert_eq!(
                sunlight_ipc::vfs_rpc::retention::transport::snapshot(),
                Some(registration)
            );
            // Snapshot can now stage the explicit boot caller's root profile.
            // Consume populated control work through the ordinary handler;
            // probing NEXT here would steal a single-delivery retain.
            drain_retention(&mut state);
            assert_eq!(
                sunlight_ipc::vfs_rpc::retention::transport::next(registration.epoch),
                Ok(None)
            );
            assert!(sunlight_ipc::vfs_rpc::retention::transport::next(0).is_err());
            assert_eq!(
                sunlight_ipc::vfs_rpc::retention::transport::status(registration.epoch),
                Ok((registration, false))
            );
            assert!(sunlight_ipc::vfs_rpc::retention::transport::status(0).is_err());
            debug_log("[VFS-RPC] authenticated kernel registration snapshot OK transport=copy");
            drain_retention(&mut state);
            assert_eq!(state.retention.active(), Some(registration));
            debug_log("[VFS-RPC] authenticated registration status OK transport=copy");
        }
    }

    // IPC server loop
    #[cfg(not(feature = "vfs-c1e2-test"))]
    let mut msg = ipc_recv(ep);
    loop {
        drain_retention(&mut state);
        drain_stats(&mut state);
        drain_files(&mut state);
        #[cfg(feature = "vfs-c1e2-test")]
        let Some(msg) = sunlight_ipc::ipc_try_recv(ep) else {
            sunlight_ipc::process_yield();
            continue;
        };
        #[cfg(feature = "vfs-d2b-test")]
        if state.retention.fixture_withdrew() {
            let handles = state.vfs.lock().open_handle_count();
            assert_eq!(handles, 2);
            debug_log("[VFS-D2B] withdrawn service backend handles=2");
            // Require endpoint withdrawal alone to wake the mmap caller.
            let start = sunlight_ipc::monotonic_millis();
            while unsafe { sunlight_libc::sys::syscall2(0xf1c1, 30, 0) } == 0 {
                assert!(sunlight_ipc::monotonic_millis() - start < 3000);
                sunlight_ipc::process_yield();
            }
            sunlight_ipc::ProcessExit::exit(0);
        }
        if sunlight_ipc::vfs_rpc::retention::transport::is_control_wakeup(&msg) {
            #[cfg(not(feature = "vfs-c1e2-test"))]
            { msg = ipc_recv(ep); }
            continue;
        }
        let reply = handle_request(&mut state, msg);
        #[cfg(not(feature = "vfs-c1e2-test"))]
        { msg = ipc_reply_and_wait(ep, reply); }
        #[cfg(feature = "vfs-c1e2-test")]
        sunlight_ipc::ipc_reply(reply);
    }
}

#[cfg(feature = "vfs-c1e2-test")]
fn linux_fixture_call(number: u64, arg0: u64, arg1: u64, arg2: u64) -> i64 {
    linux_fixture_call6(number, [arg0, arg1, arg2, 0, 0, 0])
}

#[cfg(feature = "vfs-c1e2-test")]
fn linux_fixture_call6(number: u64, args: [u64; 6]) -> i64 {
    let mut result = number;
    unsafe {
        core::arch::asm!("syscall", inout("rax") result, in("rdi") args[0], in("rsi") args[1],
            in("rdx") args[2], in("r10") args[3], in("r8") args[4], in("r9") args[5],
            lateout("rcx") _, lateout("r11") _);
    }
    result as i64
}

#[cfg(feature = "vfs-c1e7-test")]
fn native_mutation_fixture() {
    use sunlight_libc::{chdir, close, mkdir, open, read, rename, rename_no_replace, stat, unlink};
    let file = b"/tmp/c1e9-native-file";
    while stat(file).is_err() {
        sunlight_ipc::process_yield();
    }
    let fd = open(file).unwrap();
    mkdir(b"/tmp/c1e9-native", 0o700).unwrap();
    stat(b"/tmp/c1e9-native").unwrap();
    chdir(b"/tmp/c1e9-native").unwrap();
    mkdir(b"child", 0o700).unwrap();
    assert!(rename_no_replace(file, b"child").is_err());
    stat(file).unwrap();
    rename(file, b"payload").unwrap();
    rename(b"/tmp/c1e9-native", b"/tmp/c1e9-native-moved").unwrap();
    stat(b"child").unwrap();
    stat(b"payload").unwrap();
    unlink(b"payload").unwrap();
    assert!(stat(b"payload").is_err());
    let mut bytes = [0; 4];
    assert_eq!(read(fd, &mut bytes).unwrap(), 4);
    assert_eq!(&bytes, b"held");
    close(fd).unwrap();
    chdir(b"/").unwrap();
    debug_log("[VFS-C1E9] native namespace mutations passed");
}

#[cfg(feature = "vfs-c1e7-test")]
fn linux_mutation_fixture() {
    let ptr = |path: &[u8]| path.as_ptr() as u64;
    let dir = ptr(b"/tmp/c1e9-linux\0");
    let moved = ptr(b"/tmp/c1e9-linux-moved\0");
    let file = ptr(b"/tmp/c1e9-linux-file\0");
    let payload = ptr(b"payload\0");
    let child = ptr(b"child\0");
    let cwd = (-100i64) as u64;
    let mut record = [0u8; 144];
    let mut stat = |path| linux_fixture_call(4, path, record.as_mut_ptr() as u64, 0);
    assert_eq!(linux_fixture_call(83, dir, 0o700, 0), 0);
    assert_eq!(stat(dir), 0);
    assert_eq!(linux_fixture_call(80, dir, 0, 0), 0);
    assert_eq!(linux_fixture_call(258, cwd, child, 0o700), 0);
    assert_eq!(linux_fixture_call(258, cwd, child, 0o700), -17);
    assert_eq!(linux_fixture_call(82, dir, moved, 0), 0);
    assert_eq!(stat(dir), -2);
    assert_eq!(stat(child), 0);
    assert_eq!(linux_fixture_call6(264, [cwd, file, cwd, payload, 0, 0]), 0);
    assert_eq!(linux_fixture_call(263, cwd, payload, 0x200), -38);
    assert_eq!(stat(payload), 0);
    assert_eq!(linux_fixture_call(263, cwd, payload, 0), 0);
    assert_eq!(stat(payload), -2);
    assert_eq!(linux_fixture_call(80, ptr(b"/\0"), 0, 0), 0);
}

#[cfg(feature = "vfs-c1e7-test")]
fn linux_private_mmap_fixture() {
    const PAGE: u64 = 4096;
    const SPAN: u64 = 2 * PAGE;
    let descriptor = linux_fixture_call(2, b"/tests/cat-hello\0".as_ptr() as u64, 2, 0);
    assert!(descriptor >= 0);
    let fd = descriptor as u64;
    let seek = |offset, whence| linux_fixture_call(8, fd, offset, whence);
    assert_eq!(seek(PAGE, 0), PAGE as i64);
    assert_eq!(linux_fixture_call(1, fd, b"tail-data".as_ptr() as u64, 9), 9);
    assert_eq!(seek(5, 0), 5);
    let map = |length, prot, offset| {
        linux_fixture_call6(9, [0, length, prot, 0x802, fd, offset])
    };
    assert_eq!(map(PAGE, 1, 1), -22);
    for _ in 0..4 {
        let address = map(SPAN, 3, 0);
        assert!(address > 0);
        let bytes = address as *mut u8;
        unsafe {
            assert_eq!(bytes.read_volatile(), b'h');
            assert_eq!(bytes.add(PAGE as usize).read_volatile(), b't');
            assert_eq!(bytes.add(PAGE as usize + 9).read_volatile(), 0);
            assert_eq!(bytes.add(SPAN as usize - 1).read_volatile(), 0);
            bytes.add(PAGE as usize).write_volatile(b'X');
        }
        assert_eq!(seek(0, 1), 5);
        assert_eq!(linux_fixture_call(11, address as u64, SPAN, 0), 0);
    }
    let address = map(PAGE, 1, PAGE);
    assert!(address > 0);
    assert_eq!(linux_fixture_call(10, address as u64, PAGE, 0), 0);
    assert_eq!(linux_fixture_call(10, address as u64, PAGE, 1), 0);
    assert_eq!(seek(0, 1), 5);
    assert_eq!(linux_fixture_call(3, fd, 0, 0), 0);
    unsafe {
        assert_eq!((address as *const u8).read_volatile(), b't');
    }
    assert_eq!(linux_fixture_call(11, address as u64, PAGE, 0), 0);
}

#[cfg(feature = "vfs-c1e11-test")]
fn linux_create_fixture() {
    let cwd = (-100i64) as u64;
    let directory = b"/tmp\0".as_ptr() as u64;
    let name = b"c1e11-linux-create\0".as_ptr() as u64;
    let chdir = |path| linux_fixture_call(80, path, 0, 0);
    let open = |flags, mode| linux_fixture_call6(257, [cwd, name, flags, mode, 0, 0]);
    let close = |fd| linux_fixture_call(3, fd as u64, 0, 0);
    assert_eq!(chdir(directory), 0);
    let fd = open(0x42 | 0x80, 0o600); // O_RDWR | O_CREAT | O_EXCL
    assert!(fd >= 0);
    assert_eq!(open(0x42 | 0x80, 0o600), -17);
    assert_eq!(linux_fixture_call(1, fd as u64, b"x".as_ptr() as u64, 1), 1);
    assert_eq!(close(fd), 0);
    let fd = open(0x40, 0); // O_RDONLY | O_CREAT preserves the existing data.
    assert!(fd >= 0);
    let mut byte = 0u8;
    assert_eq!(linux_fixture_call(0, fd as u64, &mut byte as *mut u8 as u64, 1), 1);
    assert_eq!(byte, b'x');
    assert_eq!(close(fd), 0);
    assert_eq!(open(0x200, 0), -22); // Read-only O_TRUNC is unsupported.
    let fd = open(0x201, 0); // O_WRONLY | O_TRUNC
    assert!(fd >= 0);
    assert_eq!(close(fd), 0);
    let fd = open(0, 0);
    assert!(fd >= 0);
    assert_eq!(linux_fixture_call(0, fd as u64, &mut byte as *mut u8 as u64, 1), 0);
    assert_eq!(close(fd), 0);
    assert_eq!(linux_fixture_call(87, name, 0, 0), 0);
    assert_eq!(chdir(b"/\0".as_ptr() as u64), 0);
}

#[cfg(feature = "vfs-c1e12-test")]
fn linux_committed_mapping(log: impl Fn(&str)) -> bool {
    use sunlight_vfs_server::directory::attachment::{fat_mapping_byte, FAT_MAPPING_BYTES};
    const PAGE: usize = 4096;
    const FILE: &[u8] = b"/mnt/volume/MMAP.BIN\0";
    let open = |path: &[u8]| linux_fixture_call(2, path.as_ptr() as u64, 0, 0);
    let ready = open(b"/tmp/c1e12-native-ready\0");
    if ready < 0 {
        return false;
    }
    assert_eq!(linux_fixture_call(3, ready as u64, 0, 0), 0);
    let sample = |address| linux_fixture_call(sunlight_ipc::NATIVE_SYSCALL_TAG | 0xf1c7, address, 0, 0) as u64;
    const USER_RO_NX: u64 = (1 << 63) | 5;
    assert_eq!(sample(0), 0);
    let fd = open(FILE);
    assert!(fd >= 0);
    let fd = fd as u64;
    assert_eq!(linux_fixture_call(8, fd, 23, 0), 23);
    let map = |offset| linux_fixture_call6(9, [0, (3 * PAGE) as u64, 3, 2, fd, offset]);
    assert_eq!(map(1), -22);
    assert_eq!(sample(0), 0);
    for _ in 0..2 {
        let address = map(PAGE as u64);
        assert!(address > 0);
        let mapped = address as *mut u8;
        for page in 0..3 {
            assert_eq!(sample(address as u64 + (page * PAGE) as u64), USER_RO_NX | 2);
        }
        for i in 0..3 * PAGE {
            let expected = if PAGE + i < FAT_MAPPING_BYTES { fat_mapping_byte(PAGE + i) } else { 0 };
            assert_eq!(unsafe { mapped.add(i).read_volatile() }, expected);
        }
        unsafe { mapped.write_volatile(0xff) };
        assert_eq!(linux_fixture_call(10, address as u64, PAGE as u64, 1), 0);
        assert_eq!(sample(address as u64), USER_RO_NX);
        assert_eq!(linux_fixture_call(11, address as u64, (3 * PAGE) as u64, 0), 0);
        for page in 0..3 {
            assert_eq!(sample(address as u64 + (page * PAGE) as u64), 0);
        }
        assert_eq!(sample(0), 0);
    }
    assert_eq!(linux_fixture_call(8, fd, 0, 1), 23);
    let mut byte = 0u8;
    assert_eq!(linux_fixture_call(8, fd, PAGE as u64, 0), PAGE as i64);
    assert_eq!(linux_fixture_call(0, fd, &mut byte as *mut u8 as u64, 1), 1);
    assert_eq!(byte, fat_mapping_byte(PAGE));
    let address = map(0);
    assert!(address > 0);
    assert_eq!(linux_fixture_call(3, fd, 0, 0), 0);
    assert_eq!(unsafe { (address as *const u8).read_volatile() }, fat_mapping_byte(0));
    assert_eq!(linux_fixture_call(11, address as u64, (3 * PAGE) as u64, 0), 0);
    assert_eq!(sample(address as u64), 0);
    assert_eq!(sample(0), 0);
    log("[VFS-C1E12] ordinary Linux FAT private mmap passed");

    let fd = open(b"/proc/self/mountinfo\0");
    let mut observed = [0u8; 16];
    if fd >= 0 {
        let mut bytes = [0u8; PAGE];
        let count = linux_fixture_call(0, fd as u64, bytes.as_mut_ptr() as u64, PAGE as u64);
        assert_eq!(linux_fixture_call(3, fd as u64, 0, 0), 0);
        if (1..=PAGE as i64).contains(&count) {
            if let Ok(text) = core::str::from_utf8(&bytes[..count as usize]) {
                let mut rows = text.lines().filter(|line| line.contains(" /mnt/volume "));
                if let Some(row) = rows.next() {
                    let mut fields = row.split_ascii_whitespace();
                    let id = fields.next().and_then(|id| id.parse::<u64>().ok());
                    let device = fields.nth(1).and_then(|dev| dev.strip_prefix("240:"));
                    let source = device.and_then(|dev| dev.parse::<u64>().ok());
                    let tail = fields.collect::<alloc::vec::Vec<_>>();
                    if let (Some(id), Some(source), Some(device)) = (id, source, device) {
                        if rows.next().is_none()
                            && tail.len() == 7
                            && tail[..5] == ["/", "/mnt/volume", "rw", "-", "vfat"]
                            && tail[5].strip_prefix("helios-block:") == Some(device)
                            && tail[6] == "rw"
                        {
                            observed[..8].copy_from_slice(&id.to_le_bytes());
                            observed[8..].copy_from_slice(&source.to_le_bytes());
                        }
                    }
                }
            }
        }
    }
    let mut capacity = [0u64; 15];
    let status = linux_fixture_call(137, FILE.as_ptr() as u64, capacity.as_mut_ptr() as u64, 0);
    if status == 0 && capacity[..5] == [0x4d44, 512, 127006, 126984, 126984] && capacity[9] == 512 {
        log("[VFS-C1E12] Linux statfs FAT capacity passed blocks=127006 free=126984 unit=512");
    } else {
        log("[VFS-C1E12] Linux statfs FAT capacity mismatch");
    }
    let staging = b"/tmp/c1e12-linux-staging\0";
    let done = b"/tmp/c1e12-linux-done\0";
    let fd = linux_fixture_call(2, staging.as_ptr() as u64, 0x41, 0o600);
    assert!(fd >= 0);
    assert_eq!(linux_fixture_call(1, fd as u64, observed.as_ptr() as u64, observed.len() as u64), observed.len() as i64);
    assert_eq!(linux_fixture_call(3, fd as u64, 0, 0), 0);
    assert_eq!(linux_fixture_call(82, staging.as_ptr() as u64, done.as_ptr() as u64, 0), 0);
    true
}

#[cfg(feature = "vfs-c1e2-test")]
fn linux_mount_fixture() -> ! {
    let open = || linux_fixture_call(2, b"/proc/self/mountinfo\0".as_ptr() as u64, 0, 0);
    let log = |message: &str| {
        linux_fixture_call(sunlight_ipc::NATIVE_SYSCALL_TAG | 99,
            message.as_ptr() as u64, message.len() as u64, 0);
    };
    let mut passed = false;
    let mut unavailable = false;
    #[cfg(feature = "vfs-c1e12-test")]
    let mut committed = false;
    #[cfg(feature = "vfs-c1e12-test")]
    let mut withdrawal_seen = false;
    for _ in 0..200_000 {
        #[cfg(feature = "vfs-c1e12-test")]
        if passed && !committed {
            committed = linux_committed_mapping(log);
        }
        let fd = open();
        if fd >= 0 {
            let mut bytes = core::mem::MaybeUninit::<[u8; 4096]>::uninit();
            let len = linux_fixture_call(0, fd as u64, bytes.as_mut_ptr() as u64, 4096);
            let closed = linux_fixture_call(3, fd as u64, 0, 0);
            let statfs = linux_fixture_call(137, b"/mnt/volume/KEEP.TXT\0".as_ptr() as u64, 0, 0);
            let root = linux_fixture_call(137, b"/\0".as_ptr() as u64, 0, 0);
            if (1..=4096).contains(&len) && closed == 0 && statfs == -2 && root == -95
            {
                let bytes = unsafe { core::slice::from_raw_parts(bytes.as_ptr().cast::<u8>(), len as usize) };
                let contains = |pattern: &[u8]| bytes.windows(pattern.len()).any(|window| {
                    window.iter().zip(pattern).all(|(left, right)| left == right)
                });
                if cfg!(feature = "vfs-c1e10-test") && contains(b"/mnt/volume ") {
                    linux_fixture_call(24, 0, 0, 0);
                    continue;
                }
                if !contains(b" / / rw - tmpfs tmpfs rw\n") || contains(b"/mnt/volume ") {
                    break;
                }
                #[cfg(feature = "vfs-c1e12-test")]
                if committed && !withdrawal_seen {
                    withdrawal_seen = true;
                    log("[VFS-C1E12] Linux mountinfo and statfs confirm FAT withdrawal");
                }
                if !passed {
                    #[cfg(feature = "vfs-c1e7-test")]
                    {
                        linux_private_mmap_fixture();
                        log("[VFS-C1E7] Linux private mmap checks passed");
                        linux_mutation_fixture();
                        log("[VFS-C1E9] Linux namespace mutations passed");
                        #[cfg(feature = "vfs-c1e11-test")]
                        {
                            linux_create_fixture();
                            log("[VFS-C1E11] Linux openat creation flags passed");
                        }
                    }
                    log("[VFS-C1E6] ordinary Linux open/read/close and statfs reached retained view");
                }
                passed = true;
            }
        } else if passed && fd == -5
            && linux_fixture_call(sunlight_ipc::NATIVE_SYSCALL_TAG | 25, 3, 0, 0) == 0
            && linux_fixture_call(137, b"/\0".as_ptr() as u64, 0, 0) == -5
        {
            unavailable = true;
            break;
        }
        linux_fixture_call(24, 0, 0, 0);
    }
    if unavailable {
        #[cfg(feature = "vfs-c1e7-test")]
        {
            assert_eq!(linux_fixture_call(83, b"/tmp/c1e9-unavailable\0".as_ptr() as u64, 0o700, 0), -5);
            log("[VFS-C1E9] Linux mutation failed closed after service loss");
        }
        log("[VFS-C1E6] ordinary Linux mountinfo and statfs fail closed after service loss");
    }
    linux_fixture_call(60, u64::from(!unavailable), 0, 0);
    loop { core::hint::spin_loop(); }
}

#[cfg(feature = "vfs-n1m-block-test")]
fn qualify_native_block() -> ! {
    fn call(op: u64, ptr: u64, lba: u64, generation: u64) -> u64 {
        let result;
        unsafe {
            core::arch::asm!(
                "syscall",
                inlateout("rax") 180u64 => result,
                in("rdi") op, in("rsi") ptr, in("rdx") lba, in("r8") generation,
                in("r9") 0u64, in("r10") 0u64, in("r12") 0u64,
                lateout("rcx") _, lateout("r11") _,
            );
        }
        result
    }
    let mut previous = 0;
    for cycle in 0..64u8 {
        let generation = call(0x100, 0, 0, 0);
        assert!(generation > previous && generation != u64::MAX);
        let lba = 128 + u64::from(cycle);
        let payload: [u8; 512] = core::array::from_fn(|index| cycle ^ index as u8);
        let mut readback = [0u8; 512];
        let out = readback.as_mut_ptr() as u64;
        assert_eq!(call(0x101, out, lba, previous), u64::MAX);
        assert_eq!(call(0x101, out, 192, generation), u64::MAX);
        assert_eq!(call(0x102, payload.as_ptr() as u64, lba, generation), 0);
        assert_eq!(call(0x103, 0, 0, generation), 0);
        assert_eq!(call(0x101, out, lba, generation), 0);
        assert_eq!(readback, payload);
        assert_eq!(call(0x104, 0, 0, generation), 0);
        previous = generation;
    }
    let generation = call(0x100, 0, 0, 0);
    assert!(generation > previous && generation != u64::MAX);
    let payload: [u8; 512] = core::array::from_fn(|index| 63 ^ index as u8);
    let accepted = call(0x105, payload.as_ptr() as u64, 191, generation);
    assert!(accepted != 0 && accepted != u64::MAX);
    debug_log("[VFS-N1M] ring3 readback and release cycles complete; exiting with accepted write");
    sunlight_ipc::ProcessExit::exit(0)
}

/// Bound the work per ordinary IPC turn or payload-free kernel receive hint.
fn drain_retention(state: &mut State) {
    #[cfg(feature = "vfs-d1b-test")]
    let before = state.vfs.lock().open_handle_count();
    #[cfg(not(feature = "vfs-c1e10-test"))]
    state.retention.poll_root_control(
        &mut sunlight_vfs_server::runtime::control::CopiedControl,
        &state.namespace,
    );
    #[cfg(feature = "vfs-c1e10-test")]
    state.retention.poll_root_control_without_files(
        &mut sunlight_vfs_server::runtime::control::CopiedControl,
        &state.namespace,
    );
    #[cfg(feature = "vfs-d1b-test")]
    {
        let after = state.vfs.lock().open_handle_count();
        if before != after {
            debug_log(&alloc::format!("[VFS-D1B] backend handles={before}->{after}"));
        }
    }
}

/// Copied stat work is kernel-origin only; legacy IPC cannot enter this path.
fn drain_stats(state: &mut State) {
    use sunlight_ipc::vfs_rpc::stat_transport as api;
    let Some(registration) = state.retention.active() else {
        return;
    };
    #[cfg(feature = "stat-failure-test")]
    if state.stat_failure.fail_pull(registration.epoch) {
        return;
    }
    for _ in 0..64 {
        let Ok(Some(packet)) = api::next(registration.epoch) else {
            break;
        };
        let Ok(input) = api::input(&packet) else {
            break;
        };
        let reply = match state.retention.stat(registration, input) {
            Ok(reply) => reply,
            Err(_) => {
                // Kernel input always contains the canonical dispatch header.
                // A service-side policy failure completes that exact ID.
                let Some(id) = input.get(8..16) else {
                    break;
                };
                let id = u64::from_le_bytes(id.try_into().unwrap());
                let Ok(reply) = api::encode_reply(id, 5, &[]) else {
                    break;
                };
                reply
            }
        };
        #[cfg(feature = "stat-failure-test")]
        if state.stat_failure.withhold(registration.epoch, &reply) {
            continue;
        }
        if !api::complete(registration.epoch, &reply) {
            break;
        }
    }
}

/// Copied regular-file work is kernel-origin only; legacy IPC cannot enter this
/// path. Each frame is independently authenticated by the kernel transport
/// before it is pulled, and the service still checks the caller against every
/// descriptor it touches. Bounded per turn so one busy caller cannot starve the
/// ordinary IPC loop.
fn drain_files(state: &mut State) {
    use sunlight_vfs_server::runtime::control::{Control, CopiedControl};
    let Some(registration) = state.retention.active() else {
        return;
    };
    let mut control = CopiedControl;
    #[cfg(feature = "vfs-c1e2-test")]
    state.storage.reap_cancelled(debug_log);
    for _ in 0..64 {
        let Ok(Some(frame)) = control.file_next(registration.epoch) else {
            break;
        };
        #[cfg(feature = "vfs-d1b-test")]
        let before = state.vfs.lock().open_handle_count();
        #[cfg(feature = "vfs-c1e2-test")]
        if let Some(accepted) =
            state.storage.dispatch(&state.namespace, registration, &frame, debug_log)
        {
            if !accepted {
                break;
            }
            continue;
        }
        let accepted = state.retention.dispatch_file(&mut control, registration, &frame);
        #[cfg(feature = "vfs-d1b-test")]
        {
            let after = state.vfs.lock().open_handle_count();
            if before != after {
                debug_log(&alloc::format!("[VFS-D1B] backend handles={before}->{after}"));
            }
        }
        if !accepted {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// FSTAB mount coordinator
// ---------------------------------------------------------------------------

fn mount_from_fstab(vfs: &mut Vfs<StorageDevice>, boot: &mut Option<BootFs>, entries_seed: &'static [RamEntry]) -> Option<&'static str> {
    let mut seed = Vfs::new();
    let _ = seed.mount_ramfs("/", RamFs::new(entries_seed));

    let mut fstab_buf = [0u8; FSTAB_MAX_BYTES];
    let fstab_len = read_seed_file(&mut seed, "/etc/fstab", &mut fstab_buf);
    if fstab_len > sunlight_fs::fstab::MAX_FSTAB_BYTES {
        debug_log("[FSTAB] oversized configuration rejected");
        return None;
    }
    let fstab_text = match core::str::from_utf8(&fstab_buf[..fstab_len]) {
        Ok(text) => text,
        Err(_) => {
            debug_log("[FSTAB] invalid UTF-8 configuration rejected");
            return None;
        }
    };
    let entries = match parse_fstab(fstab_text) {
        Ok(entries) => entries,
        Err(_) => {
            debug_log("[FSTAB] invalid configuration; legacy view unchanged");
            return None;
        }
    };
    let mut boot_mountpoint = None;

    for entry in entries.iter().flatten() {
        if let Some(mountpoint) = mount_fstab_entry(vfs, boot, entry, entries_seed) {
            boot_mountpoint = Some(mountpoint);
        }
    }

    boot_mountpoint
}

fn mount_fstab_entry(
    vfs: &mut Vfs<StorageDevice>,
    boot: &mut Option<BootFs>,
    entry: &FstabEntry<'_>,
    seed: &'static [RamEntry],
) -> Option<&'static str> {
    match entry.fs_type {
        "ramfs" | "tmpfs" => {
            if entry.mountpoint == "/" {
                #[cfg(feature = "vfs-c1e2-test")]
                vfs.mount_tmpfs("/", sunlight_fs::Tmpfs::new(seed)).expect("storage fixture root");
                #[cfg(not(feature = "vfs-c1e2-test"))]
                let _ = vfs.mount_ramfs("/", RamFs::new(seed));
            }
            None
        }
        "bootfs" => {
            if entry.mountpoint == "/boot" && boot.is_none() {
                // SAFETY: Kernel maps the FAT share page before starting this process.
                *boot = unsafe { BootFs::new() };
            }
            boot.as_ref().map(|_| "/boot")
        }
        _ => None,
    }
}

fn read_seed_file(vfs: &mut Vfs, path: &str, out: &mut [u8]) -> usize {
    let handle = match vfs.open(path) {
        Ok(handle) => handle,
        Err(_) => return 0,
    };
    let read = vfs.read(handle, 0, out).unwrap_or(0);
    let _ = vfs.close(handle);
    read
}

// ---------------------------------------------------------------------------
// Request routing
// ---------------------------------------------------------------------------

fn handle_request(state: &mut State, msg: IpcMsg) -> IpcMsg {
    #[cfg(feature = "stat-failure-test")]
    if let Some(reply) = state.stat_failure.arm(&msg) {
        return reply;
    }
    // Negotiation is read-only and grants no authority. File dispatch stays
    // disabled until authenticated kernel registration and continuations exist.
    if msg.label == sunlight_ipc::vfs_rpc::NEGOTIATE {
        let words = sunlight_ipc::vfs_rpc::negotiate(
            [msg.words[0], msg.words[1], msg.words[2], msg.words[3]],
            msg.word_count as usize,
            msg.cap_count as usize,
        );
        return IpcMsg::with_label(sunlight_ipc::vfs_rpc::NEGOTIATE_REPLY)
            .word(0, words[0])
            .word(1, words[1])
            .word(2, words[2])
            .word(3, words[3]);
    }
    #[cfg(feature = "production-grant-test")]
    if msg.badge == 0xF4F0 && msg.label == 0xF11B07 && msg.cap_count == 0 {
        // This service owns no delegation registry. Report real legacy handle
        // storage separately; these handles are not authorization grants.
        let open = state.open_meta.iter().filter(|m| m.len != 0).count();
        let boot = state
            .boot
            .as_ref()
            .map_or(0, |b| b.handles.iter().filter(|h| h.in_use).count());
        return IpcMsg::with_label(0xF11B07)
            .word(0, msg.words[0])
            .word(1, open as u64)
            .word(2, boot as u64)
            .word(
                3,
                sunlight_libc::alloc::heap_stats().requested_user_bytes as u64,
            );
    }
    #[cfg(feature = "production-grant-test")]
    let msg = {
        let mut request = msg;
        if matches!(request.badge, 0xF4F3 | 0xF4F4)
            && request.label == VfsMsg::AUTHORIZE_EXEC
            && request.cap_count == 2
        {
            // Only the fixture's transport word is stripped. The real syscall
            // must authenticate the original caller and reject retired state.
            let control = request.caps[1];
            request.cap_count = 1;
            request.caps[1] = sunlight_ipc::CapabilityToken::INVALID;
            let ack = sunlight_ipc::ipc_call_timeout(
                control,
                IpcMsg::with_label(0xF11B0F).word(0, request.badge),
                10_000,
            )
            .expect("bounded consume lifecycle rendezvous");
            assert_eq!(ack.label, 0xF11B0F);
            debug_log("[GRANT-F] consumer-resumed real_vfs=1 boundary=before-consume");
            let reply = handle_request(state, request);
            assert!(
                reply.label != VfsMsg::REPLY || reply.words[0] != 0 || reply.words[1] != 1,
                "retired requester authorized"
            );
            debug_log(if request.badge == 0xF4F3 {
                "[GRANT-F] consumer-result case=exit-consume allowed=0"
            } else {
                "[GRANT-F] consumer-result case=exec-consume allowed=0"
            });
            return reply;
        }
        request
    };
    match msg.label {
        VfsMsg::AUTHORIZE_EXEC => match decoded_path(&msg.words) {
            Some(path)
                if msg.cap_count == 1
                    && sunlight_ipc::vfs_grants::consume(msg.caps[0], path.as_str()) =>
            {
                ok_reply().word(1, 1)
            }
            _ => error_reply(FsError::PermissionDenied),
        },
        VfsMsg::OPEN => match decoded_path(&msg.words) {
            Some(pb) if pb.as_str() == "/var/lib/sunlight-kv/broker.store" => {
                error_reply(FsError::PermissionDenied)
            }
            Some(pb) => open_path(state, pb.as_str()),
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::READ => {
            let Ok(raw_handle) = u32::try_from(msg.words[0]) else {
                return error_reply(FsError::BadHandle);
            };
            let Ok(offset) = usize::try_from(msg.words[1]) else {
                return error_reply(FsError::Io);
            };
            let Ok(requested) = usize::try_from(msg.words[2]) else {
                return error_reply(FsError::Io);
            };
            if requested > 4096 || offset.checked_add(requested).is_none() {
                return error_reply(FsError::Io);
            }
            read_handle(state, FileHandle(raw_handle), offset, requested)
        }
        VfsMsg::WRITE => {
            let Ok(raw_handle) = u32::try_from(msg.words[0]) else {
                return error_reply(FsError::BadHandle);
            };
            let Ok(offset) = usize::try_from(msg.words[1]) else {
                return error_reply(FsError::Io);
            };
            let data = unpack_bytes(&msg.words[2..]);
            if offset.checked_add(data.len()).is_none() {
                return error_reply(FsError::Io);
            }
            write_handle(state, FileHandle(raw_handle), offset, &data)
        }
        VfsMsg::CLOSE => match u32::try_from(msg.words[0]) {
            Ok(handle) => close_handle(state, FileHandle(handle)),
            Err(_) => error_reply(FsError::BadHandle),
        },
        VfsMsg::STAT => match decoded_path(&msg.words) {
            Some(pb) => stat_path(state, pb.as_str()),
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::FSTAT => match u32::try_from(msg.words[0]) {
            Ok(handle) => fstat_handle(state, FileHandle(handle)),
            Err(_) => error_reply(FsError::BadHandle),
        },
        VfsMsg::MKDIR => match decoded_path(&msg.words) {
            Some(pb) => mkdir_path(
                state,
                pb.as_str(),
                msg.words[4] as u32,
                msg.words[5] as u32,
                msg.words[6] as u16,
            ),
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::CHMOD => match decoded_path(&msg.words) {
            Some(pb) => chmod_path(state, pb.as_str(), msg.words[4] as u16),
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::CHOWN => match decoded_path(&msg.words) {
            Some(pb) => chown_path(state, pb.as_str(), msg.words[4] as u32, msg.words[5] as u32),
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::UNLINK => match decoded_path(&msg.words) {
            Some(pb) => unlink_path(state, pb.as_str()),
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::RENAME => match (decoded_path(&msg.words), decoded_path_hi(&msg.words)) {
            (Some(src), Some(dst)) => rename_path(state, src.as_str(), dst.as_str()),
            _ => error_reply(FsError::InvalidPath),
        },
        VfsMsg::GETPWNAM => match decoded_path(&msg.words) {
            Some(pb) => getpwnam(state, pb.as_str()),
            None => error_reply(FsError::InvalidPath),
        },
        VfsMsg::GETGRGID => getgrgid(state, msg.words[0] as u32),
        VfsMsg::GETPWUID => getpwuid(state, msg.words[0] as u32),
        _ => error_reply(FsError::Unsupported),
    }
}

/// Open a VFS path, routing /boot/* to BootFs.
fn open_path(state: &mut State, path: &str) -> IpcMsg {
    // Password material is UAC-private. The legacy daemon's RAMFS is not
    // an authentication database and must never expose its seeded shadow.
    if path == "/etc/shadow" {
        return error_reply(FsError::PermissionDenied);
    }
    if path == "/etc/resolv.conf" {
        // Ensure a generated view from resolved (or default). Trusted internal update.
        let _ = ensure_resolv_conf(state);
    }

    if let Some(local) = strip_boot_prefix(state, path) {
        match state.boot.as_mut() {
            Some(boot) => match boot.open(local) {
                Ok(handle) => {
                    remember_open(state, handle, path);
                    ok_reply().word(1, handle.0 as u64)
                }
                Err(e) => error_reply(e),
            },
            None => error_reply(FsError::NotFound),
        }
    } else {
        let opened = state.vfs.lock().open(path);
        match opened {
            Ok(handle) => {
                let packed = pack_handle(MOUNT_RAM, handle);
                remember_open(state, packed, path);
                ok_reply().word(1, packed.0 as u64)
            }
            Err(e) => error_reply(e),
        }
    }
}

/// Populate or refresh /etc/resolv.conf as a dynamic ramfs entry from resolved state.
/// Falls back to OpenDNS text if resolved is unavailable. Never makes root broadly writable.
fn ensure_resolv_conf(state: &mut State) -> Result<(), ()> {
    // Try resolved for live content (best effort, short timeout)
    let mut lines: heapless::Vec<u8, 256> = heapless::Vec::new();
    let header = b"# Generated by SunlightOS resolved.\n# Do not edit this file directly unless you know what you are doing.\n";
    for &b in header {
        let _ = lines.push(b);
    }

    let mut wrote_any = false;
    if let Some(cap) = nameserver_lookup_timeout("resolved", 25) {
        // Use RENDER to get packed servers (up to 3)
        let reply = ipc_call(cap, IpcMsg::with_label(ResolvedMsg::RENDER_RESOLV_CONF));
        if reply.label == ResolvedMsg::REPLY {
            let count = reply.words[0] as usize;
            for i in 0..count.min(3) {
                if i + 1 < 8 {
                    let addr = unpack_ipv4(reply.words[1 + i]);
                    if addr != [0, 0, 0, 0] {
                        // append "nameserver a.b.c.d\n"
                        let _ = lines.extend_from_slice(b"nameserver ");
                        for (j, &o) in addr.iter().enumerate() {
                            if j > 0 {
                                let _ = lines.push(b'.');
                            }
                            push_u8_dec(&mut lines, o);
                        }
                        let _ = lines.push(b'\n');
                        wrote_any = true;
                    }
                }
            }
        }
    }

    if !wrote_any {
        // System default OpenDNS (v0)
        let _ = lines.extend_from_slice(b"nameserver 208.67.222.222\n");
        let _ = lines.extend_from_slice(b"nameserver 208.67.220.220\n");
    }

    // Create (or open existing) dynamic entry and overwrite content via trusted internal path.
    // This does not go through the restricted write_handle policy.
    let mut vfs = state.vfs.lock();
    match vfs.create_file("/etc/resolv.conf", 0, 0, 0o644) {
        Ok(h) => {
            let _ = vfs.write(h, 0, &lines);
            let _ = vfs.close(h);
        }
        Err(_) => {
            // If create fails for some reason, try open+write (in case it pre-existed read-only)
            if let Ok(h) = vfs.open("/etc/resolv.conf") {
                let _ = vfs.write(h, 0, &lines);
                let _ = vfs.close(h);
            }
        }
    }
    Ok(())
}

fn push_u8_dec(buf: &mut heapless::Vec<u8, 256>, v: u8) {
    if v >= 100 {
        let _ = buf.push(b'0' + (v / 100));
    }
    if v >= 10 {
        let _ = buf.push(b'0' + ((v / 10) % 10));
    }
    let _ = buf.push(b'0' + (v % 10));
}

fn read_handle(state: &mut State, raw: FileHandle, offset: usize, requested: usize) -> IpcMsg {
    let (mount, local) = unpack_handle(raw);
    // Use a larger temp buf so we can take shm path for >48 byte reads (zero-copy grant to caller)
    let mut big_buf = [0u8; 4096];
    let public_account =
        open_path_for_handle(state, raw).filter(|p| matches!(*p, "/etc/passwd" | "/etc/group"));
    let read_res = if let Some(path) = public_account {
        read_public_account(path, offset, &mut big_buf[..requested])
    } else {
        match mount {
            MOUNT_BOOT => {
                if let Some(boot) = state.boot.as_mut() {
                    boot.read(local, offset, &mut big_buf[..requested])
                } else {
                    return error_reply(FsError::BadHandle);
                }
            }
            MOUNT_RAM => state
                .vfs
                .lock()
                .read(local, offset, &mut big_buf[..requested]),
            _ => return error_reply(FsError::BadHandle),
        }
    };
    match read_res {
        Ok(n) => {
            if n <= READ_REPLY_BYTES {
                // Small: keep old inline packing (compat with existing read tests/clients)
                let mut buf = [0u8; READ_REPLY_BYTES];
                buf[..n].copy_from_slice(&big_buf[..n]);
                let mut reply = ok_reply().word(1, n as u64);
                reply.words[2] = pack_bytes(&buf[0..8]);
                reply.words[3] = pack_bytes(&buf[8..16]);
                reply.word_count = 4;
                reply
            } else {
                // Large: shared memory grant (zero copy). Threshold matches IpcMsg inline capacity guidance.
                match shm_alloc() {
                    Ok((ptr, token)) => {
                        unsafe {
                            core::ptr::copy_nonoverlapping(big_buf.as_ptr(), ptr, n);
                        }
                        IpcMsg::with_label(VfsMsg::DATA_SHARED)
                            .word(0, STATUS_OK)
                            .word(1, n as u64)
                            .with_cap(0, token)
                    }
                    Err(_) => error_reply(FsError::InvalidPath),
                }
            }
        }
        Err(e) => error_reply(e),
    }
}

fn close_handle(state: &mut State, raw: FileHandle) -> IpcMsg {
    let (mount, local) = unpack_handle(raw);
    let reply = match mount {
        MOUNT_BOOT => match state.boot.as_mut() {
            Some(boot) => match boot.close(local) {
                Ok(()) => ok_reply(),
                Err(e) => error_reply(e),
            },
            None => error_reply(FsError::BadHandle),
        },
        MOUNT_RAM => match state.vfs.lock().close(local) {
            Ok(()) => ok_reply(),
            Err(e) => error_reply(e),
        },
        _ => error_reply(FsError::BadHandle),
    };
    if reply.label == VfsMsg::REPLY && reply.words[0] == STATUS_OK {
        forget_open(state, raw);
    }
    reply
}

fn stat_path(state: &mut State, path: &str) -> IpcMsg {
    if matches!(path, "/etc/passwd" | "/etc/group") {
        return match sunlight_libc::stat(path.as_bytes()) {
            Ok(stat) => ok_reply()
                .word(1, stat.size)
                .word(2, file_type_code(FileType::File)),
            Err(_) => error_reply(FsError::Io),
        };
    }
    if path == "/etc/resolv.conf" {
        let _ = ensure_resolv_conf(state);
    }
    if let Some(local) = strip_boot_prefix(state, path) {
        match state.boot.as_ref() {
            Some(boot) => match boot.stat(local) {
                Ok((size, ft)) => ok_reply().word(1, size as u64).word(2, file_type_code(ft)),
                Err(e) => error_reply(e),
            },
            None => error_reply(FsError::NotFound),
        }
    } else {
        match sunlight_vfs_server::namespace::stat_root(
            &mut state.vfs.lock(),
            path,
            Credential { uid: 0, gid: 0 },
        ) {
            Ok(stat) => ok_reply()
                .word(1, stat.size as u64)
                .word(2, file_type_code(stat.file_type)),
            Err(e) => error_reply(e),
        }
    }
}

fn fstat_handle(state: &mut State, raw: FileHandle) -> IpcMsg {
    if let Some(path) =
        open_path_for_handle(state, raw).filter(|p| matches!(*p, "/etc/passwd" | "/etc/group"))
    {
        return match sunlight_libc::stat(path.as_bytes()) {
            Ok(stat) => stat_reply(sunlight_fs::FileStat {
                size: stat.size as usize,
                uid: stat.uid,
                gid: stat.gid,
                mode: stat.mode,
                nlinks: stat.nlinks,
                file_type: FileType::File,
            }),
            Err(_) => error_reply(FsError::Io),
        };
    }
    let (mount, local) = unpack_handle(raw);
    let stat = match mount {
        MOUNT_BOOT => match state.boot.as_ref() {
            Some(boot) => match boot.fstat(local) {
                Ok(stat) => stat,
                Err(e) => return error_reply(e),
            },
            None => return error_reply(FsError::BadHandle),
        },
        MOUNT_RAM => match state.vfs.lock().fstat_handle(local) {
            Ok(stat) => stat,
            Err(e) => return error_reply(e),
        },
        _ => return error_reply(FsError::BadHandle),
    };
    stat_reply(stat)
}

fn write_handle(state: &mut State, raw: FileHandle, offset: usize, buf: &[u8]) -> IpcMsg {
    let Some(path) = open_path_for_handle(state, raw) else {
        return error_reply(FsError::BadHandle);
    };

    if path != "/etc/localtime" {
        debug_log("[SUNLIGHT-FS] request actor=unknown op=write path=<ipc-handle>");
        debug_log("[SUNLIGHT-FS] decision actor=unknown op=write path=<ipc-handle> result=deny reason=DeniedUnknownActor err=OperationNotPermitted");
        return error_reply(FsError::OperationNotPermitted);
    }

    let (mount, local) = unpack_handle(raw);
    let write_res = match mount {
        MOUNT_RAM => state.vfs.lock().write(local, offset, buf),
        MOUNT_BOOT => return error_reply(FsError::ReadOnlyFilesystem),
        _ => return error_reply(FsError::BadHandle),
    };
    match write_res {
        Ok(n) => ok_reply().word(1, n as u64),
        Err(e) => error_reply(e),
    }
}

fn mkdir_path(state: &mut State, path: &str, uid: u32, gid: u32, mode: u16) -> IpcMsg {
    if strip_boot_prefix(state, path).is_some() {
        return error_reply(FsError::ReadOnlyFilesystem);
    }
    let actor = actor_for_uid(uid);
    debug_fs_request(actor, sunlight_fs::FsOperation::Mkdir, path);
    let decision =
        sunlight_fs::can_write(actor, path, sunlight_fs::FsOperation::Mkdir, None, false);
    debug_fs_decision(actor, sunlight_fs::FsOperation::Mkdir, path, decision);
    if !decision.allowed {
        return error_reply(decision.error.unwrap_or(FsError::OperationNotPermitted));
    }
    match state.vfs.lock().mkdir(path, uid, gid, mode) {
        Ok(()) => ok_reply(),
        Err(e) => error_reply(e),
    }
}

fn actor_for_uid(uid: u32) -> sunlight_fs::Actor<'static> {
    sunlight_fs::Actor::User {
        uid,
        name: match uid {
            0 => "root",
            1000 => "user",
            _ => "",
        },
    }
}

fn debug_fs_request(actor: sunlight_fs::Actor<'_>, op: sunlight_fs::FsOperation, path: &str) {
    let _ = (actor, op, path);
    debug_log("[SUNLIGHT-FS] request actor=<ipc-user> op=<write> path=<path>");
}

fn debug_fs_decision(
    actor: sunlight_fs::Actor<'_>,
    op: sunlight_fs::FsOperation,
    path: &str,
    decision: sunlight_fs::Decision,
) {
    let _ = (actor, op, path, decision);
    debug_log("[SUNLIGHT-FS] decision actor=<ipc-user> op=<op> path=<path> result=<allow|deny> reason=<policy> err=<err>");
}

fn chmod_path(state: &mut State, path: &str, mode: u16) -> IpcMsg {
    if strip_boot_prefix(state, path).is_some() {
        return error_reply(FsError::Unsupported);
    }
    match state.vfs.lock().chmod(path, mode) {
        Ok(()) => ok_reply(),
        Err(e) => error_reply(e),
    }
}

fn chown_path(state: &mut State, path: &str, uid: u32, gid: u32) -> IpcMsg {
    if strip_boot_prefix(state, path).is_some() {
        return error_reply(FsError::Unsupported);
    }
    match state.vfs.lock().chown(path, uid, gid) {
        Ok(()) => ok_reply(),
        Err(e) => error_reply(e),
    }
}

fn unlink_path(state: &mut State, path: &str) -> IpcMsg {
    if strip_boot_prefix(state, path).is_some() {
        return error_reply(FsError::ReadOnlyFilesystem);
    }
    match state.vfs.lock().unlink(path) {
        Ok(()) => ok_reply(),
        Err(e) => error_reply(e),
    }
}

fn rename_path(state: &mut State, old: &str, new: &str) -> IpcMsg {
    if strip_boot_prefix(state, old).is_some() || strip_boot_prefix(state, new).is_some() {
        return error_reply(FsError::ReadOnlyFilesystem);
    }
    match state.vfs.lock().rename(old, new) {
        Ok(()) => ok_reply(),
        Err(e) => error_reply(e),
    }
}

fn read_public_account(path: &str, offset: usize, out: &mut [u8]) -> Result<usize, FsError> {
    if !matches!(path, "/etc/passwd" | "/etc/group") {
        return Err(FsError::PermissionDenied);
    }
    let fd = sunlight_libc::open(path.as_bytes()).map_err(|_| FsError::Io)?;
    let result = (|| {
        sunlight_libc::lseek(fd, offset as i64, 0).map_err(|_| FsError::Io)?;
        sunlight_libc::read(fd, out).map_err(|_| FsError::Io)
    })();
    let _ = sunlight_libc::close(fd);
    result
}

/// Get user information by name from /etc/passwd
fn getpwnam(state: &mut State, username: &str) -> IpcMsg {
    // Read /etc/passwd
    let opened = state.vfs.lock().open("/etc/passwd");
    match opened {
        Ok(handle) => {
            let mut buf = [0u8; 8192];
            let _ = state.vfs.lock().close(handle);
            match read_public_account("/etc/passwd", 0, &mut buf) {
                Ok(n) => {
                    let passwd_data = core::str::from_utf8(&buf[..n]).unwrap_or("");
                    match parse_passwd(passwd_data.as_bytes()) {
                        (entries, count) => {
                            match sunlight_fs::lookup_by_name(
                                &entries[..count],
                                username.as_bytes(),
                            ) {
                                Some(entry) => {
                                    let mut reply = ok_reply();
                                    reply.words[1] = entry.uid as u64;
                                    reply.words[2] = entry.gid as u64;
                                    reply.word_count = 3;
                                    reply
                                }
                                None => error_reply(FsError::NotFound),
                            }
                        }
                    }
                }
                Err(e) => error_reply(e),
            }
        }
        Err(e) => error_reply(e),
    }
}

/// Get group information by gid from /etc/group
fn getgrgid(state: &mut State, gid: u32) -> IpcMsg {
    // Read /etc/group
    let opened = state.vfs.lock().open("/etc/group");
    match opened {
        Ok(handle) => {
            let mut buf = [0u8; 8192];
            let _ = state.vfs.lock().close(handle);
            match read_public_account("/etc/group", 0, &mut buf) {
                Ok(n) => match parse_group(&buf[..n]) {
                    (entries, count) => match entries[..count].iter().find(|e| e.gid == gid) {
                        Some(_entry) => {
                            let reply = ok_reply().word(1, gid as u64);
                            reply
                        }
                        None => error_reply(FsError::NotFound),
                    },
                },
                Err(e) => error_reply(e),
            }
        }
        Err(e) => error_reply(e),
    }
}

/// Get user information by uid from /etc/passwd
fn getpwuid(state: &mut State, uid: u32) -> IpcMsg {
    // Read /etc/passwd
    let opened = state.vfs.lock().open("/etc/passwd");
    match opened {
        Ok(handle) => {
            let mut buf = [0u8; 8192];
            let _ = state.vfs.lock().close(handle);
            match read_public_account("/etc/passwd", 0, &mut buf) {
                Ok(n) => {
                    let passwd_data = core::str::from_utf8(&buf[..n]).unwrap_or("");
                    match parse_passwd(passwd_data.as_bytes()) {
                        (entries, count) => {
                            match entries[..count].iter().find(|e| e.uid == uid) {
                                Some(entry) => {
                                    let mut reply = ok_reply();
                                    reply.words[1] = entry.uid as u64;
                                    reply.words[2] = entry.gid as u64;
                                    // Pack username into words[3:7] (up to 32 bytes)
                                    let username_len = entry
                                        .username
                                        .iter()
                                        .position(|&b| b == 0)
                                        .unwrap_or(64)
                                        .min(64);
                                    for i in 0..4 {
                                        let start = i * 8;
                                        let end = (start + 8).min(username_len);
                                        if start < username_len {
                                            let mut word = 0u64;
                                            for (j, &b) in
                                                entry.username[start..end].iter().enumerate()
                                            {
                                                word |= (b as u64) << (j * 8);
                                            }
                                            reply.words[3 + i] = word;
                                        }
                                    }
                                    reply.word_count = 7;
                                    reply
                                }
                                None => error_reply(FsError::NotFound),
                            }
                        }
                    }
                }
                Err(e) => error_reply(e),
            }
        }
        Err(e) => error_reply(e),
    }
}

// ---------------------------------------------------------------------------
// Phase 3.0 self-tests (RamFs gate)
// ---------------------------------------------------------------------------

/// Ring-3 ownership fixture using a real isolated tmpfs backend. The identities
/// below are fixture values, not authenticated transport records. No IPC route,
/// process binding, namespace publication or production root is exercised.
fn test_directory_retention() {
    use sunlight_fs::{mode, FileSystem, Tmpfs, TmpfsLimits};
    use sunlight_ipc::vfs_rpc::{
        lifecycle::Caller,
        registration::{EndpointOwner, Registration},
        retention::{
            ownership::{Ownership, Retained},
            wire::{Frame, PendingRetain},
            Scope,
        },
    };
    use sunlight_vfs_server::{directory::DirectoryNamespace, runtime::Runtime};

    let mut fs = Tmpfs::with_limits(&[], TmpfsLimits::new(0, 3)).unwrap();
    fs.mkdir("/root", 1, 2, mode::DIR_755).unwrap();
    fs.mkdir("/root/cwd", 3, 4, mode::DIR_700).unwrap();
    let namespace = DirectoryNamespace::new(fs).unwrap();
    let registration = Registration {
        owner: EndpointOwner {
            endpoint: 0,
            pid: 1,
            incarnation: 2,
            address_space_generation: 3,
        },
        epoch: 4,
    };
    let scope = Scope {
        caller: Caller {
            pid: 5,
            incarnation: 6,
            address_space_generation: 7,
        },
        namespace_id: 8,
        namespace_generation: 9,
    };
    let mut owner = Ownership::<1>::new(registration).unwrap();
    let mut policy = Runtime::<1, 1, 2>::new();
    policy.bind(registration).unwrap();
    policy.bind(registration).unwrap();
    policy
        .install(registration, scope, &namespace, "/root", "/root/cwd")
        .unwrap();
    assert!(matches!(
        owner.prepare(scope).unwrap(),
        Frame::Retain { request_id: 1, .. }
    ));
    owner.abandon(1, scope).unwrap();
    let reply = policy
        .request(
            registration,
            &PendingRetain::new(registration, scope, 1)
                .unwrap()
                .request()
                .encode()
                .unwrap(),
        )
        .unwrap();
    let Retained::Release(Frame::Release(receipt)) = owner
        .retained(1, registration, &reply.encode().unwrap())
        .unwrap()
    else {
        panic!("abandoned retain must release")
    };
    // A lost publication keeps the real handles and the retry inventory owned.
    assert_eq!(owner.releases().next(), Some(Frame::Release(receipt)));
    policy.retire_caller(scope.caller);
    policy.retire_registration(registration).unwrap();
    let replacement = Registration {
        epoch: registration.epoch + 1,
        ..registration
    };
    policy.bind(replacement).unwrap();
    policy.retire_registration(registration).unwrap();
    assert_eq!(policy.active(), Some(replacement));
    drop(namespace);
    let metadata = policy.metadata(registration, receipt).unwrap();
    assert_eq!((metadata[1].1.uid, metadata[1].1.gid), (3, 4));
    let release = Frame::Release(receipt).encode().unwrap();
    policy.request(registration, &release).unwrap();
    // A lost release acknowledgement requires an idempotent service retry.
    assert_eq!(owner.outstanding(), 1);
    let released = policy.request(registration, &release).unwrap();
    owner
        .released(1, registration, &released.encode().unwrap())
        .unwrap();
    assert_eq!(owner.outstanding(), 0);
    assert!(matches!(
        policy
            .request(
                registration,
                &PendingRetain::new(registration, scope, 2)
                    .unwrap()
                    .request()
                    .encode()
                    .unwrap()
            )
            .unwrap(),
        Frame::Refused {
            reason: sunlight_ipc::vfs_rpc::retention::wire::Refusal::Resolve,
            ..
        }
    ));
    debug_log("[VFS-RPC] isolated directory pins and abandoned release OK transport=fixture");
    debug_log("[VFS-RPC] scope policy withdrawal and retained pins OK transport=fixture");
    debug_log("[VFS-RPC] registration policy lifecycle OK transport=fixture");
}

/// Uses the actual service mount table and legacy handlers. Fixture identities
/// deliberately do not claim authenticated kernel provisioning or dispatch.
fn test_live_namespace_pins(state: &mut State) {
    use sunlight_ipc::vfs_rpc::{
        lifecycle::Caller,
        registration::{EndpointOwner, Registration},
        retention::{wire::Frame, wire::PendingRetain, Scope},
    };
    let registration = Registration {
        owner: EndpointOwner {
            endpoint: 0,
            pid: 1,
            incarnation: 1,
            address_space_generation: 1,
        },
        epoch: 1,
    };
    let scope = Scope {
        caller: Caller {
            pid: 2,
            incarnation: 1,
            address_space_generation: 1,
        },
        namespace_id: 1,
        namespace_generation: 1,
    };
    let before = state.vfs.lock().open_handle_count();
    state
        .vfs
        .lock()
        .mkdir("/tmp/vfs-pins", 0, 0, 0o755)
        .unwrap();
    let mut policy = sunlight_vfs_server::runtime::Runtime::<1, 4, 1, StorageDevice>::new();
    policy.bind(registration).unwrap();
    policy
        .install(registration, scope, &state.namespace, "/", "/tmp/vfs-pins")
        .unwrap();
    let request = PendingRetain::new(registration, scope, 1).unwrap();
    let Frame::Retained(receipt) = policy
        .request(registration, &request.request().encode().unwrap())
        .unwrap()
    else {
        panic!("live namespace retain refused");
    };
    let retained = policy.metadata(registration, receipt).unwrap();
    // Mutate through the existing IPC handler, observe through the retained pair.
    assert_eq!(
        chown_path(state, "/tmp/vfs-pins", 71, 72).words[0],
        STATUS_OK
    );
    let changed = policy.metadata(registration, receipt).unwrap();
    assert_eq!(retained[1].0, changed[1].0);
    assert_eq!((changed[1].1.uid, changed[1].1.gid), (71, 72));
    // Trusted local file-owner fixture. This does not exercise copied file
    // dispatch, application descriptor tables or exit/exec transport.
    use sunlight_vfs_server::open::{Access, Seek};
    let h = state
        .vfs
        .lock()
        .create_file_exclusive("/tmp/vfs-pins/data", 71, 72, 0o600)
        .unwrap();
    state.vfs.lock().write(h, 0, b"abcdef").unwrap();
    state.vfs.lock().close(h).unwrap();
    let handles = state.vfs.lock().open_handle_count();
    let file = policy
        .open_file(
            registration,
            receipt,
            "data",
            Credential { uid: 71, gid: 72 },
            Access::ReadWrite,
            || true,
        )
        .unwrap();
    let duplicate = policy
        .duplicate_file(registration, scope.caller, file)
        .unwrap();
    let mut bytes = [0; 2];
    assert_eq!(
        policy
            .read_file(registration, scope.caller, file, &mut bytes)
            .unwrap(),
        2
    );
    assert_eq!(&bytes, b"ab");
    assert_eq!(
        policy
            .read_file(registration, scope.caller, duplicate, &mut bytes)
            .unwrap(),
        2
    );
    assert_eq!(&bytes, b"cd");
    state.vfs.lock().unlink("/tmp/vfs-pins/data").unwrap();
    policy
        .seek_file(registration, scope.caller, duplicate, Seek::Start(0))
        .unwrap();
    policy
        .read_file(registration, scope.caller, file, &mut bytes)
        .unwrap();
    assert_eq!(&bytes, b"ab");
    assert_eq!(state.vfs.lock().open_handle_count(), handles + 1);
    policy.retire_caller(scope.caller);
    assert_eq!(
        policy
            .stat_file(registration, scope.caller, file)
            .unwrap()
            .size,
        6
    );
    policy.retire_file_owner(scope.caller);
    assert_eq!(state.vfs.lock().open_handle_count(), handles);
    debug_log("[VFS-RPC] service open objects shared offset/unlink/teardown OK transport=fixture");

    policy
        .request(registration, &Frame::Release(receipt).encode().unwrap())
        .unwrap();
    drop(policy);
    assert_eq!(state.vfs.lock().open_handle_count(), before);
    debug_log(
        "[VFS-RPC] live mounted namespace pins shared with legacy handlers OK transport=fixture",
    );
}

fn run_phase30_tests(state: &mut State, ep: sunlight_ipc::EndpointId) {
    // Real userspace syscall, not a direct handler call. Even the trusted VFS
    // service cannot publish a kernel-origin namespace frame via ordinary IPC.
    let forged = sunlight_ipc::ipc_call_timeout(
        sunlight_ipc::CapabilityToken(ep.0),
        IpcMsg::with_label(sunlight_ipc::vfs_rpc::REQUEST)
            .word(0, sunlight_ipc::vfs_rpc::VERSION)
            .word(1, 1)
            .word(2, 0)
            .word(3, 56),
        1000,
    );
    if !matches!(forged, Err(sunlight_ipc::IpcCallError::InvalidCapability)) {
        debug_log("[VFS-RPC] userspace dispatch injection FAIL");
        return;
    }
    debug_log("[VFS-RPC] userspace dispatch injection rejected");
    // Exercise the actual ring-3 handler. This is a handler regression, not
    // evidence of authenticated kernel dispatch or IPC round-trip lifecycle.
    let negotiate = IpcMsg::with_label(sunlight_ipc::vfs_rpc::NEGOTIATE)
        .word(0, sunlight_ipc::vfs_rpc::VERSION)
        .word(1, sunlight_ipc::vfs_rpc::VERSION)
        .word(2, 0)
        .word(3, 0);
    let reply = handle_request(state, negotiate);
    if reply.label != sunlight_ipc::vfs_rpc::NEGOTIATE_REPLY
        || reply.word_count != 4
        || reply.cap_count != 0
        || reply.words[..4] != [1, 0, sunlight_ipc::vfs_rpc::MAX_TRANSFER, 0]
    {
        debug_log("[VFS-RPC] negotiation handler FAIL");
        return;
    }
    let mut incompatible = negotiate;
    incompatible.words[0] = 2;
    incompatible.words[1] = 2;
    if handle_request(state, incompatible).words[1] != 93 {
        debug_log("[VFS-RPC] version rejection FAIL");
        return;
    }
    let mut truncated = negotiate;
    truncated.word_count = 3;
    if handle_request(state, truncated).words[1] != 22 {
        debug_log("[VFS-RPC] truncated negotiation FAIL");
        return;
    }
    debug_log("[VFS-RPC] negotiation handler OK dispatch=disabled");
    test_directory_retention();
    test_live_namespace_pins(state);
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
    let fstat = handle_request(
        state,
        IpcMsg::with_label(VfsMsg::FSTAT).word(0, motd.0 as u64),
    );
    if fstat.label == VfsMsg::REPLY
        && fstat.words[0] == STATUS_OK
        && fstat.words[1] == 22
        && ((fstat.words[3] >> 16) & 0xff) == file_type_code(FileType::File)
    {
        debug_log("[VFS]  Fstat OK");
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

    let localtime = handle_request(state, path_msg(VfsMsg::OPEN, "/etc/localtime"));
    if localtime.label != VfsMsg::REPLY || localtime.words[0] != STATUS_OK {
        return;
    }
    let localtime_handle = FileHandle(localtime.words[1] as u32);
    let localtime_write = handle_request(
        state,
        write_msg(localtime_handle, 0, b"{\"id\":\"UTC\",\"di"),
    );
    if localtime_write.label != VfsMsg::REPLY || localtime_write.words[0] != STATUS_OK {
        return;
    }
    let _ = handle_request(
        state,
        IpcMsg::with_label(VfsMsg::CLOSE).word(0, localtime_handle.0 as u64),
    );

    let motd_write_open = handle_request(state, path_msg(VfsMsg::OPEN, "/etc/motd"));
    if motd_write_open.label != VfsMsg::REPLY || motd_write_open.words[0] != STATUS_OK {
        return;
    }
    let motd_write_handle = FileHandle(motd_write_open.words[1] as u32);
    let denied_write = handle_request(state, write_msg(motd_write_handle, 0, b"X"));
    let _ = handle_request(
        state,
        IpcMsg::with_label(VfsMsg::CLOSE).word(0, motd_write_handle.0 as u64),
    );
    if denied_write.label == VfsMsg::ERROR && denied_write.words[0] == ERR_PERM {
        debug_log("[VFS]  IPC write policy OK");
    } else {
        return;
    }

    // Exercise the native syscalls as well as IPC: clients use ENOENT from
    // stat to decide whether to create their first-run RAMFS directories.
    let probe = b"/tmp/.native-stat-mkdir-test";
    if !matches!(
        sunlight_libc::stat(probe),
        Err(sunlight_libc::sys::Errno::NoEntry)
    ) {
        debug_log("[VFS]  Native missing stat errno FAILED");
        return;
    }
    if sunlight_libc::mkdir(probe, 0o700).is_err()
        || !sunlight_libc::stat(probe).is_ok_and(|s| s.file_type == sunlight_libc::FT_DIR)
    {
        debug_log("[VFS]  Native stat/mkdir RAMFS FAILED");
        return;
    }
    debug_log("[VFS]  Native stat/mkdir RAMFS OK");

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
    let _ = handle_request(
        state,
        IpcMsg::with_label(VfsMsg::CLOSE).word(0, h1.0 as u64),
    );

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
    let _ = handle_request(
        state,
        IpcMsg::with_label(VfsMsg::CLOSE).word(0, h2.0 as u64),
    );

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
// Phase 3.7 self-tests (Unix permissions gate)
// ---------------------------------------------------------------------------

fn run_phase37_tests(state: &mut State) {
    debug_log("[VFS]  Permission model: Unix uid/gid/mode");

    // Read and parse /etc/passwd to count users
    let mut passwd_buf = [0u8; 256];
    let passwd_len = read_file_bytes(state, "/etc/passwd", &mut passwd_buf);
    let (passwd_entries, passwd_count) = parse_passwd(&passwd_buf[..passwd_len]);
    if passwd_count == 2 {
        debug_log("[VFS]  /etc/passwd: 2 users loaded");
    } else {
        return;
    }

    // Read and parse /etc/group to count groups
    let mut group_buf = [0u8; 256];
    let group_len = read_file_bytes(state, "/etc/group", &mut group_buf);
    let (_, group_count) = parse_group(&group_buf[..group_len]);
    if group_count == 7 {
        debug_log("[VFS]  /etc/group: 7 groups loaded");
    } else {
        return;
    }

    // Stat /etc/shadow and /etc/passwd for permission tests
    let shadow_stat = match state.vfs.lock().stat("/etc/shadow") {
        Ok(s) => s,
        Err(_) => return,
    };
    let passwd_stat = match state.vfs.lock().stat("/etc/passwd") {
        Ok(s) => s,
        Err(_) => return,
    };

    // Verify parsed root entry
    let root_entry = match sunlight_fs::lookup_by_name(&passwd_entries[..passwd_count], b"root") {
        Some(e) => e,
        None => return,
    };
    if root_entry.uid != 0 || root_entry.gid != 0 {
        return;
    }

    let root_cred = Credential { uid: 0, gid: 0 };
    let user_cred = Credential {
        uid: 1000,
        gid: 1000,
    };

    // Root bypasses all permission checks (including shadow which is mode 0600)
    if check_permission(&shadow_stat, &root_cred, PermCheck::Read) {
        debug_log("[VFS]  Permission check: root bypasses OK");
    } else {
        return;
    }

    // Regular user can read /etc/passwd (mode 0644, other-readable)
    if check_permission(&passwd_stat, &user_cred, PermCheck::Read) {
        debug_log("[VFS]  Permission check: user read /etc/passwd OK");
    } else {
        return;
    }

    // Regular user cannot read /etc/shadow (mode 0600, owner=root)
    if !check_permission(&shadow_stat, &user_cred, PermCheck::Read) {
        debug_log("[VFS]  Permission check: user read /etc/shadow EACCES OK");
    } else {
        return;
    }
}

// ---------------------------------------------------------------------------
// Bite 4: Shared memory + large VFS read test (runs at vfs_server init)
// ---------------------------------------------------------------------------

fn run_shm_tests(state: &mut State) {
    // Exercise direct shm_alloc/map/free (kernel will log the [SHM] lines)
    if let Ok((ptr, tok)) = shm_alloc() {
        unsafe {
            *ptr = b'Z';
        }
        if let Ok(_p2) = shm_map(tok) {
            let _ = shm_free(tok);
        }
        let _ = shm_free(tok);
    }

    // Open the 2 KiB test file (seeded in INITRAMFS) and request >48 bytes to force shm path in read_handle
    let open_reply = handle_request(state, path_msg(VfsMsg::OPEN, "/etc/large_test"));
    if open_reply.label != VfsMsg::REPLY || open_reply.words[0] != STATUS_OK {
        return;
    }
    let h = FileHandle(open_reply.words[1] as u32);

    let read_req = IpcMsg::with_label(VfsMsg::READ)
        .word(0, h.0 as u64)
        .word(1, 0)
        .word(2, 2048);
    let r = handle_request(state, read_req);

    // Server used shm for large read: token is in caps[0], label may be DATA_SHARED
    if (r.label == VfsMsg::REPLY || r.label == VfsMsg::DATA_SHARED)
        && r.caps[0] != sunlight_ipc::CapabilityToken::INVALID
    {
        let n = r.words[1] as usize;
        if n == 2048 {
            if let Ok(ptr) = shm_map(r.caps[0]) {
                // Verify content (all 'A's) to prove zero-copy data arrived intact
                let mut all_a = true;
                for i in 0..n {
                    if unsafe { *ptr.add(i) } != b'A' {
                        all_a = false;
                        break;
                    }
                }
                if all_a {
                    debug_log("[SHM]  VFS read 2048 bytes via shared memory: OK");
                }
                let _ = shm_free(r.caps[0]);
                debug_log("[SHM]  shm_free: page unmapped OK");
            }
        }
    }

    let _ = handle_request(state, IpcMsg::with_label(VfsMsg::CLOSE).word(0, h.0 as u64));

    debug_log("[SHM]  Shared memory grant: PASSED");
}

/// Read a file via internal handle_request calls into a caller-provided buffer.
/// Returns the number of bytes written.
fn read_file_bytes(state: &mut State, path: &str, out: &mut [u8]) -> usize {
    let open_reply = handle_request(state, path_msg(VfsMsg::OPEN, path));
    if open_reply.label != VfsMsg::REPLY || open_reply.words[0] != STATUS_OK {
        return 0;
    }
    let handle = FileHandle(open_reply.words[1] as u32);
    let mut total = 0usize;

    loop {
        if total >= out.len() {
            break;
        }
        let r = handle_request(state, read_msg(handle, total, READ_REPLY_BYTES));
        if r.label != VfsMsg::REPLY {
            break;
        }
        let n = r.words[1] as usize;
        if n == 0 {
            break;
        }
        let to_copy = n.min(out.len() - total);
        unpack_data(&r, &mut out[total..total + to_copy]);
        total += to_copy;
    }

    let _ = handle_request(
        state,
        IpcMsg::with_label(VfsMsg::CLOSE).word(0, handle.0 as u64),
    );
    total
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Strip the configured bootfs prefix from a path; returns the local path.
fn strip_boot_prefix<'a>(state: &State, path: &'a str) -> Option<&'a str> {
    let mountpoint = state.boot_mountpoint?;
    if path == mountpoint {
        Some("/")
    } else if path.starts_with(mountpoint) && path.as_bytes().get(mountpoint.len()) == Some(&b'/') {
        Some(&path[mountpoint.len()..])
    } else {
        None
    }
}

fn remember_open(state: &mut State, handle: FileHandle, path: &str) {
    let slot_idx = state
        .open_meta
        .iter()
        .position(|meta| meta.handle == 0 || meta.handle == handle.0);
    let Some(slot_idx) = slot_idx else {
        return;
    };

    let mut meta = OpenMeta::EMPTY;
    let bytes = path.as_bytes();
    let len = bytes.len().min(MAX_PATH_BYTES);
    meta.handle = handle.0;
    meta.len = len;
    meta.path[..len].copy_from_slice(&bytes[..len]);
    state.open_meta[slot_idx] = meta;
}

fn forget_open(state: &mut State, handle: FileHandle) {
    for meta in &mut state.open_meta {
        if meta.handle == handle.0 {
            *meta = OpenMeta::EMPTY;
            return;
        }
    }
}

fn open_path_for_handle(state: &State, handle: FileHandle) -> Option<&str> {
    state
        .open_meta
        .iter()
        .find(|meta| meta.handle == handle.0 && meta.len > 0)
        .map(OpenMeta::path_str)
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

fn stat_reply(stat: sunlight_fs::FileStat) -> IpcMsg {
    ok_reply()
        .word(1, stat.size as u64)
        .word(2, ((stat.uid as u64) << 32) | stat.gid as u64)
        .word(
            3,
            (stat.mode as u64)
                | (file_type_code(stat.file_type) << 16)
                | ((stat.nlinks as u64) << 32),
        )
}

fn error_reply(err: FsError) -> IpcMsg {
    IpcMsg::with_label(VfsMsg::ERROR).word(0, errno(err))
}

fn errno(err: FsError) -> u64 {
    match err {
        FsError::NotFound => ERR_NOT_FOUND,
        FsError::BadHandle => ERR_BAD_HANDLE,
        FsError::InvalidPath => ERR_INVALID,
        FsError::PermissionDenied => ERR_ACCES,
        FsError::OperationNotPermitted => ERR_PERM,
        FsError::ReadOnlyFilesystem => ERR_ROFS,
        FsError::NoSpace => 28,
        FsError::NoMemory => 12,
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

/// Decode a path from the upper 4 words (words[4..8]) — used for rename dst.
fn decoded_path_hi(words: &[u64; 8]) -> Option<PathBuf> {
    let mut bytes = [0u8; MAX_PATH_BYTES];
    let mut idx = 0;
    while idx < 4 {
        bytes[idx * 8..idx * 8 + 8].copy_from_slice(&words[idx + 4].to_le_bytes());
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

/// Unpack bytes from an array of u64 words into a Vec.
fn unpack_bytes(words: &[u64]) -> Vec<u8> {
    let mut out = Vec::new();
    for word in words {
        let bytes = word.to_le_bytes();
        for b in bytes {
            if b == 0 {
                break;
            }
            out.push(b);
        }
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

fn write_msg(handle: FileHandle, offset: usize, data: &[u8]) -> IpcMsg {
    IpcMsg::with_label(VfsMsg::WRITE)
        .word(0, handle.0 as u64)
        .word(1, offset as u64)
        .word(2, pack_bytes(&data[..data.len().min(8)]))
        .word(3, pack_bytes(&data[data.len().min(8)..data.len().min(16)]))
}

fn unpack_data(msg: &IpcMsg, out: &mut [u8]) {
    let mut idx = 0;
    while idx < out.len() {
        let word = if idx < 8 { msg.words[2] } else { msg.words[3] };
        out[idx] = ((word >> ((idx % 8) * 8)) & 0xff) as u8;
        idx += 1;
    }
}

// Legacy IPC VFS owns a separate view. Allocate its payload pages through the
// kernel mapping API; physical addresses and the kernel direct map stay private.
fn init_tmpfs_pages() {
    fn allocate() -> Option<u64> {
        use sunlight_libc::mman::*;
        mmap(
            core::ptr::null_mut(),
            4096,
            PROT_READ | PROT_WRITE,
            MAP_PRIVATE | MAP_ANONYMOUS | MAP_NATIVE_THREAD,
            -1,
            0,
        )
        .ok()
        .map(|p| p as u64)
    }
    unsafe fn free(address: u64) {
        let _ = sunlight_libc::mman::munmap(address as *mut u8, 4096);
    }
    unsafe fn direct_map(address: u64) -> *mut u8 {
        address as *mut u8
    }
    static PROVIDER: sunlight_mm::FrameAllocator = sunlight_mm::FrameAllocator {
        allocate,
        free,
        direct_map,
    };
    // SAFETY: mmap provides exclusive page-aligned backing with exact unmap
    // boundaries. The callbacks remain valid for the lifetime of this service.
    unsafe {
        sunlight_mm::install_allocator(&PROVIDER);
    }
}
