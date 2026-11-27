//! Isolated, single-owner qualification of a copied FAT32 attachment.
use super::*;
use sunlight_fat::Fat32;
use sunlight_fs::{BlockError, FatFs, BLOCK_SIZE};

const START: u64 = 2048;
const BLOCKS: u64 = 131072 - START;
const TARGET: &str = "/mnt/volume";
const FILE: &str = "/mnt/volume/NEW.TXT";
const PAYLOAD: &[u8] = b"native FAT32 attachment";

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

struct Device(Arc<Lease>);

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
    let mut fat = Fat32::mount(Device(lease.clone())).ok_or(FsError::Io)?;
    fat.validate_root_chain().map_err(|_| FsError::Io)?;
    let mut backend = FatFs::new(fat);
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

    let fs = exclusive(namespace);
    let keep = fs.open("/mnt/volume/KEEP.TXT")?;
    let mut untouched = [0; 9];
    assert_eq!(fs.read(keep, 0, &mut untouched)?, untouched.len());
    assert_eq!(&untouched, b"untouched");
    fs.close(keep)?;
    if first {
        let temporary = "/mnt/volume/TEMP.TXT";
        let handle = fs.create_file_exclusive(temporary, 0, 0, 0o644)?;
        assert_eq!(fs.write(handle, 0, &[0x5a; 512])?, 512);
        assert_eq!(fs.write(handle, 512, &[0x6b; 512])?, 512);
        assert_eq!(fs.stat(temporary)?.size, 1024);
        fs.truncate(handle)?;
        assert_eq!(fs.stat(temporary)?.size, 0);
        assert_eq!(fs.write(handle, 0, b"delete me")?, 9);
        fs.close(handle)?;
        fs.unlink(temporary)?;
        assert_eq!(fs.open(temporary), Err(FsError::NotFound));
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
