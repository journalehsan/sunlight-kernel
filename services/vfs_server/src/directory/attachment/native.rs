//! Isolated, single-owner qualification of a copied FAT32 attachment.
use super::*;
use sunlight_fat::Fat32;
use sunlight_fs::{BlockError, FatFs, BLOCK_SIZE};

const START: u64 = 2048;
const BLOCKS: u64 = 131072 - START;
const TARGET: &str = "/mnt/volume";
const FILE: &str = "/mnt/volume/NEW.TXT";
const PAYLOAD: &[u8] = b"native FAT32 attachment";
const ROOT: Credential = Credential { uid: 0, gid: 0 };

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

struct Lease(u64);

impl Lease {
    fn control(&self, op: u64) -> Result<(), FsError> {
        (call(op, 0, 0, self.0) == 0)
            .then_some(())
            .ok_or(FsError::Io)
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.control(0x108);
    }
}

pub struct Device(Arc<Lease>, bool);

impl Device {
    fn sector(&self, op: u64, ptr: u64, lba: u64) -> Result<(), BlockError> {
        if lba >= BLOCKS {
            return Err(BlockError::OutOfRange);
        }
        (call(op, ptr, START + lba, self.0 .0) == 0)
            .then_some(())
            .ok_or(BlockError::Io)
    }
}

impl BlockDevice for Device {
    fn read_block(&mut self, lba: u64, bytes: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        if self.1 {
            return Err(BlockError::Io);
        }
        self.sector(0x101, bytes.as_mut_ptr() as u64, lba)
    }

    fn write_block(&mut self, lba: u64, bytes: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        self.sector(0x102, bytes.as_ptr() as u64, lba)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.0.control(0x103).map_err(|_| BlockError::Io)
    }

    fn block_count(&self) -> u64 {
        BLOCKS
    }
}

struct Ready {
    reservation: Reservation<Device>,
    backend: FatFs<Device>,
    lease: Arc<Lease>,
}

fn stage(namespace: &DirectoryNamespace<Device>, lease: Arc<Lease>) -> Result<Ready, Error> {
    let reservation = namespace.reserve_attachment(TARGET)?;
    stage_reserved(reservation, lease)
}

fn stage_reserved(reservation: Reservation<Device>, lease: Arc<Lease>) -> Result<Ready, Error> {
    let fail_read = cfg!(feature = "vfs-c1e2-test") && reservation.target == "/mnt/bad";
    let mut fat = Fat32::mount(Device(lease.clone(), fail_read)).ok_or(FsError::Io)?;
    fat.validate_root_chain().map_err(|_| FsError::Io)?;
    fat.sample_capacity(1024).map_err(|_| FsError::Io)?;
    let mut backend = FatFs::new_attachment(fat);
    if backend.stat("/")?.file_type != FileType::Directory {
        return Err(FsError::NotDir.into());
    }
    backend.read_dir("/", &mut |_| true)?;
    Ok(Ready {
        reservation,
        backend,
        lease,
    })
}

#[cfg(feature = "vfs-c1e1-test")]
fn storage_call(op: u64, source: u64, pointer: u64) -> u64 {
    let result;
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") 168u64 => result,
            in("rdi") op, in("rsi") source, in("rdx") pointer,
            lateout("rcx") _, lateout("r11") _,
        );
    }
    result
}

#[cfg(feature = "vfs-c1e1-test")]
pub fn storage_caller(log: fn(&str)) -> ! {
    use sunlight_ipc::{nameserver_lookup, process_yield, storage as api};
    let endpoint = loop {
        if let Some(endpoint) = nameserver_lookup("vfs") {
            break endpoint;
        }
        process_yield();
    };
    let mut record = [0; api::INVENTORY_RECORD_SIZE];
    let mut index = 0;
    let source = loop {
        assert_eq!(storage_call(api::INVENTORY, index, record.as_mut_ptr() as u64), 1);
        if u64::from_le_bytes(record[8..16].try_into().unwrap()) != 0 {
            break u64::from_le_bytes(record[..8].try_into().unwrap());
        }
        index += 1;
    };
    let prepare = |source, path: &[u8]| storage_call(api::MOUNT_FAT32, source, path.as_ptr() as u64);
    if cfg!(feature = "vfs-c1e2-test") {
        if sunlight_ipc::getpid() == api::FIXTURE_RETIRING_PID {
            let _ = prepare(source, b"/mnt/volume\0");
            panic!("retired StorageCtl caller resumed");
        }
        while !sunlight_ipc::kill(api::FIXTURE_RETIRING_PID, 0) {
            process_yield();
        }
    }
    assert_eq!(prepare(source, b"/mnt/missing\0"), (-5i64) as u64);
    let unknown_source_errno = if cfg!(feature = "vfs-c1e2-test") { -2i64 } else { -5i64 };
    assert_eq!(prepare(u64::MAX, b"/mnt/volume\0"), unknown_source_errno as u64);
    if cfg!(feature = "vfs-c1e2-test") {
        assert_eq!(prepare(source, b"/mnt/bad\0"), (-5i64) as u64);
        let mut previous = 0;
        for _ in 0..3 {
            let token = prepare(source, b"/mnt/volume\0");
            assert!(token > previous && token <= i64::MAX as u64);
            let stat = |path: &[u8]| {
                let mut msg = sunlight_ipc::IpcMsg::with_label(sunlight_ipc::VfsMsg::STAT);
                for (index, chunk) in path.chunks(8).enumerate() {
                    let mut word = [0; 8];
                    word[..chunk.len()].copy_from_slice(chunk);
                    msg = msg.word(index, u64::from_le_bytes(word));
                }
                sunlight_ipc::ipc_call(endpoint, msg)
            };
            let target = stat(b"/mnt/volume");
            assert_eq!((target.label, target.words[0], target.words[2]),
                (sunlight_ipc::VfsMsg::REPLY, 0, 2));
            let hidden = stat(b"/mnt/volume/KEEP.TXT");
            assert_eq!((hidden.label, hidden.words[0]), (sunlight_ipc::VfsMsg::ERROR, 2));
            log("[VFS-C1E4] ordinary STAT sees target directory and cannot see prepared FAT file");
            let mut mount = [0; api::MOUNT_RECORD_SIZE];
            for op in [api::LIST_MOUNTS, api::INVENTORY_MOUNTS] {
                let mut end = false;
                for index in 0..8 {
                    let result = storage_call(op, index, mount.as_mut_ptr() as u64);
                    if result == 0 {
                        end = true;
                        break;
                    }
                    assert_eq!(result, 1);
                    assert_ne!(&mount[32..43], b"/mnt/volume");
                }
                assert!(end);
            }
            let stale = api::service_mount_id((1 << 31) - 1, 2).unwrap();
            assert_eq!(storage_call(api::UNMOUNT, stale, 0), (-116i64) as u64);
            log("[VFS-C1E5] prepared mount absent; stale service identity rejected");
            assert_eq!(prepare(source, b"/mnt/other\0"), (-5i64) as u64);
            assert_eq!(storage_call(api::PUBLISH_PREPARATION, token + 100, 0), (-2i64) as u64);
            if !cfg!(feature = "vfs-c1e10-test") {
                assert_eq!(storage_call(api::PUBLISH_PREPARATION, token, 0), (-95i64) as u64);
                assert_eq!(storage_call(api::PUBLISH_PREPARATION, token, 0), (-95i64) as u64);
            }
            assert_eq!(storage_call(api::ABORT_PREPARATION, token + 100, 0), (-2i64) as u64);
            assert_eq!(storage_call(api::ABORT_PREPARATION, token, 0), 0);
            assert_eq!(storage_call(api::ABORT_PREPARATION, token, 0), (-2i64) as u64);
            assert_eq!(storage_call(api::PUBLISH_PREPARATION, token, 0), (-2i64) as u64);
            previous = token;
        }
        #[cfg(not(feature = "vfs-c1e10-test"))]
        log("[VFS-C1E3] PASS wrong cookies rejected; publish blocked and preparation retryable");
        #[cfg(feature = "vfs-c1e10-test")]
        committed_fixture(source, log);
        let interrupted = prepare(source, b"/mnt/exit\0");
        assert!((interrupted as i64) < 0);
        let mut mount = [0; api::MOUNT_RECORD_SIZE];
        assert!((storage_call(api::INVENTORY_MOUNTS, 0, mount.as_mut_ptr() as u64) as i64) < 0);
        log("[VFS-C1E5] retired service inventory fails closed");
        log("[VFS-C1E2] PASS StorageCtl retained and aborted three preparations; service exit rejected");
        sunlight_ipc::ProcessExit::exit(0);
    } else {
        for _ in 0..2 {
            assert_eq!(prepare(source, b"/mnt/volume\0"), 0);
        }
        log("[VFS-C1E1] PASS StorageCtl completed two unpublished preparations");
    }
    loop {
        process_yield();
    }
}

#[cfg(feature = "vfs-c1e10-test")]
fn committed_fixture(source: u64, log: fn(&str)) {
    use sunlight_ipc::storage as api;
    use sunlight_libc::{chdir, close, dup, lseek, open, read, stat};
    let mut previous_mount = None;
    for _cycle in 0..2 {
        let mut cookie = 0;
        api::mount_fat32(source, b"/mnt/volume\0".as_ptr() as u64, |op, value, pointer| {
            let result = storage_call(op, value, pointer) as i64;
            if result < 0 {
                return Err(result);
            }
            if op == api::MOUNT_FAT32 {
                cookie = result as u64;
            }
            Ok(result as u64)
        }).unwrap();
        assert_ne!(cookie, 0);
        assert_eq!(storage_call(api::PUBLISH_PREPARATION, cookie, 0), (-2i64) as u64);
        assert_eq!(storage_call(api::ABORT_PREPARATION, cookie, 0), (-2i64) as u64);
        let mut record = [0; api::MOUNT_RECORD_SIZE];
        let mut mount = None;
        for op in [api::LIST_MOUNTS, api::INVENTORY_MOUNTS] {
            let mut found = None;
            for index in 0..8 {
                let result = storage_call(op, index, record.as_mut_ptr() as u64);
                if result == 0 {
                    break;
                }
                assert_eq!(result, 1);
                let id = u64::from_le_bytes(record[..8].try_into().unwrap());
                if id & api::SERVICE_MOUNT_BIT != 0 {
                    assert_eq!(u64::from_le_bytes(record[8..16].try_into().unwrap()), source);
                    assert_eq!(&record[32..43], b"/mnt/volume");
                    assert!(found.replace(id).is_none());
                }
            }
            let id = found.unwrap();
            assert_eq!(mount.replace(id).unwrap_or(id), id);
        }
        let mount = mount.unwrap();
        assert_ne!(previous_mount.replace(mount), Some(mount));
        stat(b"/mnt/volume/KEEP.TXT").unwrap();
        chdir(b"/mnt/volume").unwrap();
        let fd = open(b"KEEP.TXT").unwrap();
        let alias = dup(fd).unwrap();
        lseek(fd, 1, 0).unwrap();
        let mut payload = [0; 8];
        assert_eq!(read(alias, &mut payload).unwrap(), 8);
        assert_eq!(&payload, b"ntouched");
        close(alias).unwrap();
        close(fd).unwrap();
        #[cfg(feature = "vfs-c1e11-test")]
        committed_writes(_cycle, log);
        chdir(b"/").unwrap();
        #[cfg(feature = "vfs-c1e12-test")]
        committed_mapping(_cycle, mount, source, log);
        assert_eq!(storage_call(api::UNMOUNT, mount, 0), 0);
        assert_eq!(storage_call(api::UNMOUNT, mount, 0), (-2i64) as u64);
        assert!(stat(b"/mnt/volume/KEEP.TXT").is_err());
        log("[VFS-C1E10] exactly one attachment committed and withdrawn; payload verified");
    }
}

#[cfg(feature = "vfs-c1e11-test")]
fn committed_writes(cycle: usize, log: fn(&str)) {
    use sunlight_libc::{
        chdir, close, dup, fstat, lseek, mkdir, open, open_with_flags_mode, read,
        rename, stat, unlink, write, Errno, O_CREAT, O_EXCL, O_RDWR, O_TRUNC, O_WRONLY,
    };
    if cycle == 0 {
        mkdir(b"DIR", 0o755).unwrap();
        stat(b"DIR").unwrap();
        chdir(b"DIR").unwrap();
        mkdir(b"CHILD", 0o755).unwrap();
        let fd = open_with_flags_mode(b"DATA.TXT", O_RDWR | O_CREAT | O_EXCL, 0o600).unwrap();
        assert_eq!(
            open_with_flags_mode(b"DATA.TXT", O_RDWR | O_CREAT | O_EXCL, 0o600),
            Err(Errno::Exists)
        );
        assert_eq!(write(fd, b"retained").unwrap(), 8);
        assert_eq!(lseek(fd, 520, 0).unwrap(), 520);
        assert_eq!(write(fd, b"!").unwrap(), 1);
        assert_eq!(fstat(fd).unwrap().size, 521);
        assert_eq!(unlink(b"DATA.TXT"), Err(Errno::Busy));
        rename(b"DATA.TXT", b"MOVED.TXT").unwrap();
        rename(b"/mnt/volume/DIR", b"/mnt/volume/LIVE").unwrap();
        stat(b"CHILD").unwrap();
        let alias = dup(fd).unwrap();
        close(fd).unwrap();
        lseek(alias, 0, 0).unwrap();
        let mut retained = [0; 8];
        assert_eq!(read(alias, &mut retained).unwrap(), 8);
        assert_eq!(&retained, b"retained");
        close(alias).unwrap();
        let fd = open_with_flags_mode(b"MOVED.TXT", O_RDWR | O_CREAT, 0o600).unwrap();
        assert_eq!(fstat(fd).unwrap().size, 521);
        close(fd).unwrap();
        let fd = open_with_flags_mode(b"MOVED.TXT", O_WRONLY | O_TRUNC, 0).unwrap();
        assert_eq!(fstat(fd).unwrap().size, 0);
        assert_eq!(read(fd, &mut retained), Err(Errno::Access));
        assert_eq!(write(fd, b"persistent FAT data").unwrap(), 19);
        close(fd).unwrap();
        let temporary = open_with_flags_mode(b"TEMP.TXT", O_CREAT, 0o644).unwrap();
        close(temporary).unwrap();
        unlink(b"TEMP.TXT").unwrap();
        assert_eq!(stat(b"TEMP.TXT").err(), Some(Errno::NoEntry));
        log("[VFS-C1E11] ordinary committed FAT mutations passed");
    }
    chdir(b"/mnt/volume/LIVE").unwrap();
    stat(b"CHILD").unwrap();
    let fd = open(b"MOVED.TXT").unwrap();
    let mut payload = [0; 20];
    assert_eq!(read(fd, &mut payload).unwrap(), 19);
    assert_eq!(&payload[..19], b"persistent FAT data");
    close(fd).unwrap();
    if cycle == 1 {
        log("[VFS-C1E11] reattached payload and directory verified");
    }
}

#[cfg(feature = "vfs-c1e12-test")]
pub const FAT_MAPPING_BYTES: usize = 8192 + 9;

#[cfg(feature = "vfs-c1e12-test")]
pub fn fat_mapping_byte(offset: usize) -> u8 {
    ((offset * 29 + offset / 512 * 17) % 251) as u8
}

#[cfg(feature = "vfs-c1e12-test")]
fn committed_mapping(cycle: usize, mount: u64, source: u64, log: fn(&str)) {
    use sunlight_ipc::storage as api;
    use sunlight_libc::mman::{c_mprotect, mmap, munmap, MAP_PRIVATE, PROT_EXEC, PROT_READ, PROT_WRITE};
    use sunlight_libc::{close, dup, fstat, lseek, open, open_with_flags_mode, read, unlink, write};
    use sunlight_libc::{O_CREAT, O_EXCL, O_RDWR};
    const FILE: &[u8] = b"/mnt/volume/MMAP.BIN";
    const DONE: &[u8] = b"/tmp/c1e12-linux-done";
    const PAGE: usize = 4096;
    let sample = |address: *mut u8| unsafe { sunlight_libc::sys::syscall1(0xf1c7, address as u64) };
    const USER_RO_NX: u64 = (1 << 63) | 5;
    let mut buffer = [0u8; PAGE];
    if cycle != 0 {
        let fd = open(FILE).unwrap();
        for offset in (0..FAT_MAPPING_BYTES).step_by(PAGE) {
            let size = PAGE.min(FAT_MAPPING_BYTES - offset);
            assert_eq!(read(fd, &mut buffer).unwrap(), size);
            assert!(buffer[..size].iter().enumerate().all(|(i, &b)| b == fat_mapping_byte(offset + i)));
        }
        assert_eq!(read(fd, &mut buffer).unwrap(), 0);
        close(fd).unwrap();
        log("[VFS-C1E12] reattached original mmap payload verified");
        return;
    }

    assert_eq!(sample(core::ptr::null_mut()), 0);
    let fd = open_with_flags_mode(FILE, O_RDWR | O_CREAT | O_EXCL, 0o600).unwrap();
    for offset in (0..FAT_MAPPING_BYTES).step_by(PAGE) {
        let size = PAGE.min(FAT_MAPPING_BYTES - offset);
        for (i, byte) in buffer[..size].iter_mut().enumerate() {
            *byte = fat_mapping_byte(offset + i);
        }
        assert_eq!(write(fd, &buffer[..size]).unwrap(), size);
    }
    assert_eq!(fstat(fd).unwrap().size, FAT_MAPPING_BYTES as u64);
    let alias = dup(fd).unwrap();
    assert_eq!(lseek(alias, 37, 0).unwrap(), 37);
    assert_eq!(sample(core::ptr::null_mut()), 0);
    for _ in 0..3 {
        let address = mmap(core::ptr::null_mut(), 3 * PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE, fd.0 as i32, PAGE as u64).unwrap();
        for page in 0..3 {
            assert_eq!(sample(address.wrapping_add(page * PAGE)), USER_RO_NX | 2);
        }
        for i in 0..3 * PAGE {
            let expected = if PAGE + i < FAT_MAPPING_BYTES { fat_mapping_byte(PAGE + i) } else { 0 };
            assert_eq!(unsafe { address.add(i).read_volatile() }, expected);
        }
        unsafe { address.write_volatile(0xff) };
        assert_eq!(unsafe { c_mprotect(address, PAGE, (PROT_READ | PROT_EXEC) as i32) }, 0);
        assert_eq!(sample(address), 5);
        assert_eq!(unsafe { c_mprotect(address, PAGE, PROT_READ as i32) }, 0);
        assert_eq!(sample(address), USER_RO_NX);
        munmap(address, 3 * PAGE).unwrap();
        for page in 0..3 {
            assert_eq!(sample(address.wrapping_add(page * PAGE)), 0);
        }
        assert_eq!(sample(core::ptr::null_mut()), 0);
    }
    assert_eq!(lseek(fd, 0, 1).unwrap(), 37);
    assert_eq!(read(alias, &mut buffer[..1]).unwrap(), 1);
    assert_eq!(buffer[0], fat_mapping_byte(37));
    assert_eq!(lseek(alias, PAGE as i64, 0).unwrap(), PAGE as u64);
    assert_eq!(read(fd, &mut buffer[..1]).unwrap(), 1);
    assert_eq!(buffer[0], fat_mapping_byte(PAGE));

    let address = mmap(core::ptr::null_mut(), PAGE, PROT_READ, MAP_PRIVATE, fd.0 as i32, 0).unwrap();
    let ready = open_with_flags_mode(b"/tmp/c1e12-native-ready", O_CREAT | O_EXCL, 0o600).unwrap();
    close(ready).unwrap();
    let linux = (0..8192).find_map(|_| {
        let result = open(DONE).ok();
        if result.is_none() {
            sunlight_ipc::process_yield();
        }
        result
    }).expect("Linux FAT mapping probe did not finish");
    let mut observation = [0u8; 16];
    assert_eq!(read(linux, &mut observation).unwrap(), observation.len());
    close(linux).unwrap();
    unlink(DONE).unwrap();
    unlink(b"/tmp/c1e12-native-ready").unwrap();
    if u64::from_le_bytes(observation[..8].try_into().unwrap()) == mount
        && u64::from_le_bytes(observation[8..].try_into().unwrap()) == source
    {
        log(&alloc::format!(
            "[VFS-C1E12] StorageCtl and Linux mountinfo identity and source agree mount={mount} source={source}",
        ));
    } else {
        log("[VFS-C1E12] Linux mountinfo identity/source mismatch");
    }

    assert_eq!(storage_call(api::UNMOUNT, mount, 0), (-16i64) as u64);
    assert_eq!(lseek(alias, 0, 0).unwrap(), 0);
    assert_eq!(read(alias, &mut buffer[..1]).unwrap(), 1);
    assert_eq!(buffer[0], fat_mapping_byte(0));
    close(alias).unwrap();
    close(fd).unwrap();
    assert_eq!(unsafe { address.read_volatile() }, fat_mapping_byte(0));
    munmap(address, PAGE).unwrap();
    assert_eq!(sample(address), 0);
    assert_eq!(sample(core::ptr::null_mut()), 0);
    log("[VFS-C1E12] ordinary native FAT private mmap and busy withdrawal passed");
}

#[cfg(all(feature = "vfs-c1e1-test", not(feature = "vfs-c1e2-test")))]
pub fn qualify_storage(log: fn(&str)) -> ! {
    use sunlight_ipc::vfs_rpc::retention::transport;
    use sunlight_ipc::{process_yield, vfs_rpc::file_transport as api};
    let mut fs = Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    fs.mkdir("/mnt", 0, 0, 0o755).unwrap();
    fs.mkdir(TARGET, 0, 0, 0o755).unwrap();
    let namespace = DirectoryNamespace::mount_owner(fs).unwrap();
    let registration = transport::snapshot().unwrap();
    loop {
        let Some(bytes) = api::next(registration.epoch).unwrap() else {
            process_yield();
            continue;
        };
        let request = api::Request::decode(&bytes).unwrap();
        assert_eq!(request.registration, registration);
        assert_eq!(request.op, api::PREPARE_STORAGE);
        let target = core::str::from_utf8(request.payload).unwrap();
        let prepared = (|| {
            let reservation = namespace.reserve_attachment(target)?;
            assert_eq!(call(0x10a, request.request_id, 0, registration.epoch + 1), u64::MAX);
            let generation = call(0x10a, request.request_id, 0, registration.epoch);
            if generation == u64::MAX {
                return Err(Error::Stale);
            }
            let lease = Arc::new(Lease(generation));
            let staged = stage_reserved(reservation, lease.clone());
            let validation = staged.and_then(|ready| {
                drop(ready.reservation.lock(&namespace)?);
                drop(ready);
                Ok(())
            });
            lease.control(0x106)?;
            lease.control(0x107)?;
            validation?;
            Ok(generation)
        })();
        let (errno, result0, result1) = match prepared {
            Ok(generation) => (0, generation, request.request_id),
            Err(_) => (5, 0, 0),
        };
        let mut reply = api::Reply {
            registration, caller: request.caller, request_id: request.request_id,
            op: request.op, errno, result0, result1, length: 0, payload: &[],
        };
        if errno == 0 {
            reply.caller.incarnation += 1;
            assert!(!api::reply(registration.epoch, &reply.encode().unwrap()));
            reply.caller = request.caller;
            reply.result0 += 1;
            assert!(!api::reply(registration.epoch, &reply.encode().unwrap()));
            reply.result0 = result0;
            reply.request_id += 100;
            reply.result1 = reply.request_id;
            assert!(!api::reply(registration.epoch, &reply.encode().unwrap()));
            reply.request_id = request.request_id;
            reply.result1 = result1;
        }
        let encoded = reply.encode().unwrap();
        if api::reply(registration.epoch, &encoded) {
            assert!(!api::reply(registration.epoch, &encoded));
            if errno == 0 {
                log("[VFS-C1E1] authenticated reply accepted; lease vacant; no publication");
            }
        }
    }
}

#[cfg(feature = "vfs-c1e2-test")]
fn settle(lease: &Lease) -> Result<(), Error> {
    lease.control(0x106)?;
    lease.control(0x107)?;
    Ok(())
}

#[cfg(feature = "vfs-c1e2-test")]
#[derive(Default)]
pub struct RetainedStorage {
    retained: Option<(sunlight_ipc::vfs_rpc::lifecycle::Caller, u64, u64, Ready)>,
}

#[cfg(feature = "vfs-c1e2-test")]
impl RetainedStorage {
    fn inventory(
        namespace: &DirectoryNamespace<Device>,
        request: &sunlight_ipc::vfs_rpc::file_transport::Request<'_>,
    ) -> Result<(u64, Option<[u8; sunlight_ipc::storage::MOUNT_RECORD_SIZE]>), u64> {
        use sunlight_ipc::storage as api;
        let view = namespace
            .caller_view("/", "/", Credential { uid: request.uid, gid: request.gid })
            .map_err(storage_errno)?;
        let snapshot = view.snapshot().map_err(storage_errno)?;
        snapshot.install(|rows| {
            let mut attachments = rows.iter().filter(|row| row.mountpoint != "/");
            let count = attachments.clone().count() as u64;
            let Some(row) = attachments.nth(request.token as usize) else {
                return Ok((count, None));
            };
            if row.source == 0 {
                return Err(95u64);
            }
            let id = api::service_mount_id(request.registration.epoch, row.id).ok_or(75u64)?;
            let record = api::mount_record(id, row.source, row.protected, &row.mountpoint)
                .ok_or(75u64)?;
            Ok((count, Some(record)))
        }).map_err(storage_errno)?
    }

    fn unmount(
        namespace: &DirectoryNamespace<Device>,
        epoch: u64,
        token: u64,
    ) -> Result<(), u64> {
        let id = sunlight_ipc::storage::local_mount_id(epoch, token).ok_or(116u64)?;
        let owner = namespace.attachment_owner().map_err(storage_errno)?;
        let mut work = owner.lock().prepare_device_withdrawal(id)
            .map_err(|error| storage_errno(error.into()))?;
        let lease = work.device().0.clone();
        let withdrawal = (|| {
            work.synchronize()?;
            lease.control(0x106)?;
            owner.lock().finish_device_withdrawal(&work)?;
            lease.control(0x107)
        })();
        if withdrawal.is_err() {
            let _ = lease.control(0x108);
        }
        withdrawal.map_err(|error| storage_errno(error.into()))
    }

    pub fn reap_cancelled(&mut self, log: fn(&str)) {
        let retained = &mut self.retained;
        if let Some((_, _, _, ready)) = retained.as_ref() {
            if call(0x10b, 0, 0, ready.lease.0) == 1 {
                let (_, _, _, ready) = retained.take().unwrap();
                abort_ready(ready, log).expect("cancelled preparation settlement");
            }
        }
    }

    pub fn dispatch(
        &mut self,
        namespace: &DirectoryNamespace<Device>,
        registration: sunlight_ipc::vfs_rpc::registration::Registration,
        bytes: &[u8],
        log: fn(&str),
    ) -> Option<bool> {
        use sunlight_ipc::vfs_rpc::file_transport as api;
        let request = api::Request::decode(bytes).ok()?;
        if request.registration != registration
            || !matches!(request.op, api::PREPARE_STORAGE | api::ABORT_STORAGE | api::PUBLISH_STORAGE
                | api::INVENTORY_STORAGE | api::UNMOUNT_STORAGE)
        {
            return None;
        }
        let retained = &mut self.retained;
        let mut reply = api::Reply {
            registration,
            caller: request.caller,
            request_id: request.request_id,
            op: request.op,
            errno: 0,
            result0: 0,
            result1: 0,
            length: 0,
            payload: &[],
        };
        let record;
        match request.op {
            api::INVENTORY_STORAGE => {
                match Self::inventory(namespace, &request) {
                    Ok((_, Some(row))) => {
                        record = row;
                        reply.payload = &record;
                        reply.length = record.len();
                        reply.result0 = 1;
                    }
                    Ok((count, None)) => reply.result1 = count,
                    Err(errno) => reply.errno = errno,
                }
                log("[VFS-C1E5] ordinary mount inventory reached runtime owner");
            }
            api::UNMOUNT_STORAGE => {
                if let Err(errno) = Self::unmount(namespace, registration.epoch, request.token) {
                    reply.errno = errno;
                }
            }
            api::ABORT_STORAGE => {
                if retained.as_ref().is_some_and(|(caller, id, _, _)| {
                    *caller == request.caller && *id == request.token
                }) {
                    let (_, _, _, ready) = retained.take().unwrap();
                    if abort_ready(ready, log).is_err() {
                        reply.errno = 5;
                    }
                } else {
                    reply.errno = 2;
                }
            }
            #[cfg(feature = "vfs-c1e10-test")]
            api::PUBLISH_STORAGE => {
                use sunlight_ipc::storage::service_mount_id;
                match retained.take() {
                    Some((caller, cookie, source, ready))
                        if caller == request.caller && cookie == request.token =>
                    {
                        let Ready { reservation, backend, lease } = ready;
                        let mut backend = Some(backend);
                        let installed = (|| {
                            service_mount_id(registration.epoch, 256).ok_or(Error::Stale)?;
                            if call(0x10b, 0, 0, lease.0) != 0 {
                                return Err(Error::Stale);
                            }
                            let mut fs = reservation.lock(namespace)?;
                            Ok(fs.publish_retained_device(&reservation.target, source, false,
                                || backend.take().unwrap())?)
                        })();
                        match installed {
                            Ok(id) => {
                                reply.result0 = service_mount_id(registration.epoch, id).unwrap();
                                reply.result1 = lease.0;
                            }
                            Err(error) => {
                                *retained = Some((caller, cookie, source, Ready {
                                    reservation, backend: backend.unwrap(), lease,
                                }));
                                reply.errno = storage_errno(error);
                            }
                        }
                    }
                    other => {
                        *retained = other;
                        reply.errno = 2;
                    }
                }
            }
            #[cfg(not(feature = "vfs-c1e10-test"))]
            api::PUBLISH_STORAGE => {
                reply.errno = match retained.as_ref() {
                    Some((caller, cookie, _, ready))
                        if *caller == request.caller && *cookie == request.token =>
                    {
                        match ready.reservation.lock(namespace) {
                            Ok(guard) => {
                                drop(guard);
                                // The service has the exact cookie but lacks StorageAdmin.
                                assert_eq!(
                                    storage_call(sunlight_ipc::storage::PUBLISH_PREPARATION, *cookie, 0),
                                    (-1i64) as u64
                                );
                                let id = sunlight_ipc::storage::service_mount_id(registration.epoch, 2)
                                    .expect("fixture service epoch is representable");
                                assert_eq!(storage_call(sunlight_ipc::storage::UNMOUNT, id, 0),
                                    (-1i64) as u64);
                                assert_eq!(storage_call(sunlight_ipc::storage::LIST_MOUNTS, 0, 0),
                                    (-1i64) as u64);
                                log("[VFS-C1E5] unauthorized inventory and unmount denied");
                                log("[VFS-C1E4] runtime reservation revalidated; unauthorized publish denied");
                                publication_gate()
                            }
                            Err(_) => 116,
                        }
                    }
                    _ => 2,
                };
            }
            api::PREPARE_STORAGE => {
                match prepare_retained(namespace, &request, log) {
                    Ok(ready) => {
                        assert!(retained.is_none());
                        reply.result0 = ready.lease.0;
                        reply.result1 = request.request_id;
                        if request.caller.pid == sunlight_ipc::storage::FIXTURE_RETIRING_PID {
                            assert_eq!(call(0x10c, request.request_id, 0, registration.epoch), 0);
                            assert_eq!(call(0x10b, 0, 0, ready.lease.0), 1);
                            assert!(!api::reply(registration.epoch, &reply.encode().unwrap()));
                            abort_ready(ready, log).expect("retired caller settlement");
                            log("[VFS-C1E2] parked caller retired; late reply rejected");
                            return Some(true);
                        }
                        *retained = Some((request.caller, request.request_id, request.token, ready));
                    }
                    Err(_) => reply.errno = 5,
                }
            }
            _ => unreachable!(),
        }
        if reply.op == api::PREPARE_STORAGE && reply.errno == 0 {
            reply.caller.incarnation += 1;
            assert!(!api::reply(registration.epoch, &reply.encode().unwrap()));
            reply.caller = request.caller;
            reply.result0 += 1;
            assert!(!api::reply(registration.epoch, &reply.encode().unwrap()));
            reply.result0 -= 1;
        }
        let encoded = reply.encode().unwrap();
        let accepted = api::reply(registration.epoch, &encoded);
        if accepted {
            assert!(!api::reply(registration.epoch, &encoded));
        } else if reply.op == api::PREPARE_STORAGE && reply.errno == 0 {
            let (_, _, _, ready) = retained.take().unwrap();
            abort_ready(ready, log).expect("unacknowledged preparation settlement");
        } else if reply.op == api::PUBLISH_STORAGE && reply.errno == 0 {
            if Self::unmount(namespace, registration.epoch, reply.result0).is_err() {
                log("[VFS-C1E10] compensation uncertain; retiring runtime owner");
                sunlight_ipc::ProcessExit::exit(1);
            }
            log("[VFS-C1E10] unacknowledged insertion withdrawn and settled");
        }
        Some(accepted)
    }
}

#[cfg(feature = "vfs-c1e2-test")]
fn storage_errno(error: Error) -> u64 {
    match error {
        Error::Filesystem(FsError::Busy) => 16,
        Error::Filesystem(FsError::NotFound) => 2,
        Error::Filesystem(FsError::PermissionDenied) => 13,
        Error::Stale => 116,
        _ => 5,
    }
}

#[cfg(all(feature = "vfs-c1e2-test", not(feature = "vfs-c1e10-test")))]
fn publication_gate() -> u64 {
    // Inventory and withdrawal are routed. Commit still needs ordinary
    // mountinfo, statfs, mmap and the FAT ownership handoff.
    95
}

#[cfg(feature = "vfs-c1e2-test")]
fn prepare_retained(
    namespace: &DirectoryNamespace<Device>,
    request: &sunlight_ipc::vfs_rpc::file_transport::Request<'_>,
    log: fn(&str),
) -> Result<Ready, Error> {
    let target = core::str::from_utf8(request.payload).map_err(|_| Error::Stale)?;
    let reservation = namespace.reserve_attachment(target)?;
    let generation = call(0x10a, request.request_id, 0, request.registration.epoch);
    if generation == u64::MAX {
        return Err(Error::Stale);
    }
    let lease = Arc::new(Lease(generation));
    let prepared = stage_reserved(reservation, lease.clone()).and_then(|mut ready| {
        if target == "/mnt/exit" {
            log("[VFS-C1E2] exiting with private FAT and active lease before reply");
            sunlight_ipc::ProcessExit::exit(0);
        }
        let backend_stat = ready.backend.stat("/KEEP.TXT")?;
        let handle = ready.backend.open("/KEEP.TXT")?;
        let identity = ready.backend.handle_object_id(handle);
        ready.backend.close(handle)?;
        let identity = identity?.ok_or(Error::Stale)?;
        let alias = ready.backend.open("/keep.txt")?;
        let alias_identity = ready.backend.handle_object_id(alias);
        ready.backend.close(alias)?;
        if alias_identity? != Some(identity) {
            return Err(Error::Stale);
        }
        let mut fs = ready.reservation.lock(namespace)?;
        assert!(matches!(
            fs.stat(&alloc::format!("{target}/KEEP.TXT")),
            Err(FsError::NotFound)
        ));
        assert_eq!(backend_stat.size, 9);
        drop(fs);
        ready.lease.control(0x109)?;
        Ok(ready)
    });
    match prepared {
        Ok(ready) => {
            assert_eq!(Arc::strong_count(&lease), 3);
            log("[VFS-C1E2] retained reservations=1 backends=1 lease=active descriptors=0");
            Ok(ready)
        }
        Err(error) => {
            assert_eq!(Arc::strong_count(&lease), 1);
            settle(&lease)?;
            log("[VFS-C1E2] compensated reservations=0 backends=0 lease=vacant descriptors=0");
            Err(error)
        }
    }
}

#[cfg(feature = "vfs-c1e2-test")]
fn abort_ready(ready: Ready, log: fn(&str)) -> Result<(), Error> {
    let Ready { reservation, backend, lease } = ready;
    drop(backend);
    drop(reservation);
    assert_eq!(Arc::strong_count(&lease), 1);
    settle(&lease)?;
    log("[VFS-C1E2] aborted reservations=0 backends=0 lease=vacant descriptors=0");
    Ok(())
}

fn publish(namespace: &DirectoryNamespace<Device>, ready: Ready) -> Result<u64, Error> {
    let mut fs = ready.reservation.lock(namespace)?;
    ready.lease.control(0x109)?;
    Ok(fs.publish_prepared_device(&ready.reservation.target, ready.backend)?)
}

fn exclusive(namespace: &mut DirectoryNamespace<Device>) -> &mut Vfs<Device> {
    let Backend::Mounted(owner) = &mut namespace.filesystem else {
        unreachable!();
    };
    Arc::get_mut(owner)
        .expect("probe exclusively owns its namespace")
        .get_mut()
}

fn cycle(
    namespace: &mut DirectoryNamespace<Device>,
    lease: &Arc<Lease>,
    first: bool,
    log: fn(&str),
) -> Result<u64, Error> {
    let ready = stage(namespace, lease.clone())?;
    log("[VFS-N1O] ring3 FAT initialized");
    let id = publish(namespace, ready)?;
    log("[VFS-N1O] ring3 N1D published");
    let initial = namespace.caller_view("/", "/", ROOT)?.capacity(TARGET)?;
    assert_eq!(initial.id, id);

    let fs = exclusive(namespace);
    let keep = fs.open("/mnt/volume/KEEP.TXT")?;
    let mut untouched = [0; 9];
    assert_eq!(fs.read(keep, 0, &mut untouched)?, untouched.len());
    assert_eq!(&untouched, b"untouched");
    fs.close(keep)?;
    if first {
        let temporary = "/mnt/volume/TEMP.TXT";
        let handle = fs.create_file_exclusive(temporary, 0, 0, 0o644)?;
        let root = fs.open("/")?;
        let original = fs.stat_beneath(root, root, temporary, ROOT)?;
        let empty = fs.create_file_exclusive("/mnt/volume/EMPTY.TXT", 0, 0, 0o644)?;
        assert_ne!(fs.handle_object_id(empty)?, fs.handle_object_id(handle)?);
        fs.close(empty)?;
        fs.unlink("/mnt/volume/EMPTY.TXT")?;
        let alias = fs.open_file_beneath(root, root, "/mnt/volume/temp.txt", ROOT, true, false)?;
        assert_eq!(fs.handle_object_id(alias)?, fs.handle_object_id(handle)?);
        let moved = "/mnt/volume/MOVED.TXT";
        fs.rename(temporary, moved)?;
        assert_eq!(fs.stat_beneath(root, root, moved, ROOT)?.object_id, original.object_id);
        assert_eq!(fs.unlink(moved), Err(FsError::Busy));
        assert_eq!(fs.write(handle, 0, &[0x5a; 512])?, 512);
        assert_eq!(fs.write(handle, 512, &[0x6b; 512])?, 512);
        assert_eq!(fs.fstat_handle(alias)?.size, 1024);
        fs.truncate(handle)?;
        assert_eq!(fs.fstat_handle(alias)?.size, 0);
        assert_eq!(fs.write(handle, 0, b"delete me")?, 9);
        let mut part = [0; 2];
        assert_eq!(fs.read(alias, 7, &mut part)?, 2);
        assert_eq!(&part, b"me");
        fs.close(alias)?;
        fs.close(handle)?;
        fs.unlink(moved)?;
        assert_eq!(fs.open(moved), Err(FsError::NotFound));
        let reused = fs.create_file_exclusive(moved, 0, 0, 0o644)?;
        assert_ne!(fs.stat_beneath(root, root, moved, ROOT)?.object_id, original.object_id);
        fs.close(reused)?;
        fs.unlink(moved)?;
        fs.close(root)?;
        assert_eq!(fs.open_handle_count(), 0);
        log("[VFS-C1D] ring3 FAT identity rename truncate slot reuse handles=0");
        log("[VFS-N1P] ring3 FAT extended truncated unlinked");
        let handle = fs.create_file_exclusive(FILE, 0, 0, 0o644)?;
        assert_eq!(fs.write(handle, 0, PAYLOAD)?, PAYLOAD.len());
        fs.close(handle)?;
    }
    let handle = fs.open(FILE)?;
    assert_eq!(fs.stat(FILE)?.size as usize, PAYLOAD.len());
    let mut contents = [0; PAYLOAD.len()];
    assert_eq!(fs.read(handle, 0, &mut contents)?, PAYLOAD.len());
    assert_eq!(&contents, PAYLOAD);
    let after = namespace.caller_view("/", "/", ROOT)?.capacity(FILE)?;
    assert_eq!(after.id, id);
    assert_eq!(after.blocks, initial.blocks);
    assert_eq!(after.free, initial.free - u32::from(first));
    log("[VFS-C1] ring3 FAT capacity matched attachment and allocation");
    let fs = exclusive(namespace);
    assert_eq!(fs.prepare_device_withdrawal(id).err(), Some(FsError::Busy));
    assert_eq!(fs.open(FILE), Err(FsError::Busy));
    let mut partial = [0; 5];
    assert_eq!(fs.read(handle, 2, &mut partial)?, partial.len());
    assert_eq!(&partial, &PAYLOAD[2..7]);
    fs.close(handle)?;

    let owner = namespace.attachment_owner()?;
    let mut work = owner.lock().prepare_device_withdrawal(id)?;
    work.synchronize()?;
    assert_eq!(call(0x107, 0, 0, lease.0), u64::MAX);
    lease.control(0x106)?;
    assert_eq!(call(0x100, 0, 0, 0), u64::MAX);
    let mut sector = [0; BLOCK_SIZE];
    assert_eq!(
        call(0x101, sector.as_mut_ptr() as u64, START, lease.0),
        u64::MAX
    );
    owner.lock().finish_device_withdrawal(&work)?;
    log("[VFS-N1O] ring3 topology withdrawn");
    lease.control(0x107)?;
    Ok(id)
}

pub fn qualify_native_fat(log: fn(&str)) {
    let mut fs = Vfs::new();
    fs.mount_tmpfs("/", Tmpfs::new(&[])).unwrap();
    fs.mkdir("/mnt", 0, 0, 0o755).unwrap();
    fs.mkdir(TARGET, 0, 0, 0o755).unwrap();
    let mut namespace = DirectoryNamespace::mount_owner(fs).unwrap();
    let mut previous_generation = 0;
    let mut previous_id = 0;
    for index in 0..4 {
        let generation = call(0x100, 0, 0, 0);
        assert!(generation > previous_generation && generation != u64::MAX);
        let lease = Arc::new(Lease(generation));
        let mut sector = [0; BLOCK_SIZE];
        let ptr = sector.as_mut_ptr() as u64;
        assert_eq!(call(0x101, ptr, START, previous_generation), u64::MAX);
        assert_eq!(call(0x101, ptr, START - 1, generation), u64::MAX);
        assert_eq!(call(0x101, ptr, START + BLOCKS, generation), u64::MAX);
        let result = cycle(&mut namespace, &lease, index == 0, log);
        if result.is_err() {
            let _ = lease.control(0x108);
        }
        let id = result.expect("native attachment lifecycle failed");
        assert!(id > previous_id);
        previous_id = id;
        previous_generation = generation;
    }
    log("[VFS-N1O] PASS ring3 FAT lifecycle cycles=4");
}
