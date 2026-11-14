use sunlight_fs::{vfs::*, *};
use std::{rc::Rc,cell::{Cell,RefCell}};
use sunlight_block::{BlockError,CachedBlockDevice,BLOCK_SIZE};
use sunlight_fat::{Fat32,testimg::FatImageBuilder};
#[derive(Clone)]
struct Disk {
    bytes: Rc<RefCell<Vec<u8>>>,
    fail_sync: Rc<Cell<bool>>,
}
impl BlockDevice for Disk {
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let offset = usize::try_from(lba)
            .ok()
            .and_then(|n| n.checked_mul(BLOCK_SIZE))
            .ok_or(BlockError::OutOfRange)?;
        buf.copy_from_slice(
            self.bytes
                .borrow()
                .get(offset..offset + BLOCK_SIZE)
                .ok_or(BlockError::OutOfRange)?,
        );
        Ok(())
    }
    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let offset = usize::try_from(lba)
            .ok()
            .and_then(|n| n.checked_mul(BLOCK_SIZE))
            .ok_or(BlockError::OutOfRange)?;
        self.bytes
            .borrow_mut()
            .get_mut(offset..offset + BLOCK_SIZE)
            .ok_or(BlockError::OutOfRange)?
            .copy_from_slice(buf);
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        if self.fail_sync.get() {
            Err(BlockError::Io)
        } else {
            Ok(())
        }
    }
    fn block_count(&self) -> u64 {
        (self.bytes.borrow().len() / BLOCK_SIZE) as u64
    }
}
type Cache = CachedBlockDevice<Disk, 2>;
fn fixture() -> (Vfs<Cache>, Disk) {
    let mut image = FatImageBuilder::new(1);
    image.add_file(image.root(), "HELLO.TXT", b"fixture\n");
    let disk = Disk {
        bytes: Rc::new(RefCell::new(image.build())),
        fail_sync: Rc::new(Cell::new(false)),
    };
    let mut vfs = Vfs::new();
    static ROOT: &[crate::RamEntry] = &[crate::RamEntry::dir("/mnt", 0, 0, mode::DIR_755)];
    vfs.mount_ramfs("/", RamFs::new(ROOT)).unwrap();
    (vfs, disk)
}
fn mount(vfs: &mut Vfs<Cache>, disk: &Disk) {
    vfs.mount_runtime_fat("/mnt", 42, Fat32::mount(Cache::new(disk.clone())).unwrap())
        .unwrap();
}


fn main() {
 let (mut vfs,disk)=fixture(); mount(&mut vfs,&disk);
 let a=vfs.create_file("/mnt/LIVE.TXT",0,0,0o600).unwrap();
 if std::env::args().nth(1).as_deref()==Some("size") {
   let b=vfs.open("/mnt/LIVE.TXT").unwrap();
   vfs.write(a,0,b"updated").unwrap();
   let observed=vfs.fstat_handle(b).unwrap().size;
   println!("other descriptor size={observed}, actual path size={}",vfs.stat("/mnt/LIVE.TXT").unwrap().size);
   assert_eq!(observed,7,"FAT descriptor metadata must refer to the current object");
 } else {
   vfs.write(a,0,b"old").unwrap();
   vfs.unlink("/mnt/LIVE.TXT").unwrap();
   let b=vfs.create_file("/mnt/LIVE.TXT",0,0,0o600).unwrap();
   vfs.write(b,0,b"new").unwrap();
   vfs.write(a,0,b"BAD").unwrap();
   let mut out=[0;3];vfs.read(b,0,&mut out).unwrap();
   println!("replacement after write through unlinked fd={out:?}");
   assert_eq!(&out,b"new","unlinked FAT descriptor must not modify replacement pathname");
 }
}
