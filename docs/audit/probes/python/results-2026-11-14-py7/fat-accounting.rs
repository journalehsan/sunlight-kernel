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



fn allocated(disk: &Disk) -> usize {
 let image=disk.bytes.borrow();let start=u16::from_le_bytes([image[14],image[15]]) as usize*512;
 let size=u32::from_le_bytes(image[36..40].try_into().unwrap()) as usize*512;
 image[start+8..start+size].chunks_exact(4).filter(|b|u32::from_le_bytes((*b).try_into().unwrap())&0x0fff_ffff!=0).count()
}
fn entry(disk: &Disk)->(usize,u32) {
 let image=disk.bytes.borrow();let reserved=u16::from_le_bytes(image[14..16].try_into().unwrap()) as usize;
 let fat=u32::from_le_bytes(image[36..40].try_into().unwrap()) as usize;
 let at=(reserved+fat)*512+32;let e=&image[at..at+32];assert_eq!(&e[..11],b"LIVE    TXT");
 (at,((u16::from_le_bytes(e[20..22].try_into().unwrap()) as u32)<<16)|u16::from_le_bytes(e[26..28].try_into().unwrap()) as u32)
}
fn main() {
 let (mut vfs,disk)=fixture();mount(&mut vfs,&disk);let initial=allocated(&disk);
 let a=vfs.create_file("/mnt/LIVE.TXT",0,0,0o600).unwrap();vfs.write(a,0,b"old").unwrap();vfs.sync_file(a).unwrap();
 let original=entry(&disk);let after_open=allocated(&disk);assert_eq!(after_open,initial+1);
 vfs.unlink("/mnt/LIVE.TXT").unwrap();vfs.sync_dir("/mnt").unwrap();let after_unlink=allocated(&disk);assert_eq!(after_unlink,initial);
 let b=vfs.create_file("/mnt/LIVE.TXT",0,0,0o600).unwrap();vfs.write(b,0,b"new").unwrap();vfs.sync_file(b).unwrap();let replacement=entry(&disk);
 assert_eq!(original,replacement,"witness must reuse both directory slot and cluster");
 let before=disk.bytes.borrow().clone();assert_eq!(vfs.read(a,0,&mut [0;3]),Err(FsError::BadHandle));assert_eq!(vfs.write(a,0,b"BAD"),Err(FsError::BadHandle));assert_eq!(vfs.truncate(a),Err(FsError::BadHandle));assert_eq!(*disk.bytes.borrow(),before);
 let mut bytes=[0;3];assert_eq!(vfs.read(b,0,&mut bytes),Ok(3));assert_eq!(&bytes,b"new");assert_ne!(a,b);
 let id=vfs.mount_info(1).unwrap().id;assert_eq!(vfs.unmount(id),Err(FsError::Busy));
 vfs.unlink("/mnt/LIVE.TXT").unwrap();vfs.sync_dir("/mnt").unwrap();let before_close=allocated(&disk);assert_eq!(before_close,initial);
 vfs.close(a).unwrap();vfs.close(b).unwrap();let after_close=allocated(&disk);assert_eq!(after_close,initial);assert_eq!(vfs.open_handle_count(),0);vfs.unmount(id).unwrap();
 println!("PY7 RAW ACCOUNTING PASS initial={initial} after_open={after_open} after_unlink={after_unlink} before_close={before_close} after_close={after_close} slot_byte_offset={} reused_cluster={} old_handle={} new_handle={} replacement=new backing_release=immediate_not_deferred", original.0, original.1,a.0,b.0);
}
