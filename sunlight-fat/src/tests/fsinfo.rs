use super::*;
use crate::FatError;
use sunlight_block::{BlockDevice, BlockError, BLOCK_SIZE};

const ROOT: u64 = 10;
const FAT: u64 = 8;
const INFO: [u64; 2] = [1, 7];

fn fixture(unknown: bool) -> Vec<u8> {
    let original = FatImageBuilder::new(1).build();
    let mut image = vec![0; original.len() + 8 * BLOCK_SIZE];
    image[..BLOCK_SIZE].copy_from_slice(&original[..BLOCK_SIZE]);
    image[FAT as usize * BLOCK_SIZE..9 * BLOCK_SIZE]
        .copy_from_slice(&original[BLOCK_SIZE..2 * BLOCK_SIZE]);
    image[9 * BLOCK_SIZE..ROOT as usize * BLOCK_SIZE]
        .copy_from_slice(&original[BLOCK_SIZE..2 * BLOCK_SIZE]);
    image[ROOT as usize * BLOCK_SIZE..].copy_from_slice(&original[2 * BLOCK_SIZE..]);
    let total = (image.len() / BLOCK_SIZE) as u32;
    image[32..36].copy_from_slice(&total.to_le_bytes());
    image[14..16].copy_from_slice(&8u16.to_le_bytes());
    image[16] = 2;
    image[48..50].copy_from_slice(&1u16.to_le_bytes());
    image[50..52].copy_from_slice(&6u16.to_le_bytes());
    image.copy_within(..BLOCK_SIZE, 6 * BLOCK_SIZE);
    for lba in INFO {
        let start = lba as usize * BLOCK_SIZE;
        let sector = &mut image[start..start + BLOCK_SIZE];
        sector[..4].copy_from_slice(b"RRaA");
        sector[484..488].copy_from_slice(b"rrAa");
        sector[488..492].copy_from_slice(&(if unknown { u32::MAX } else { 64 }).to_le_bytes());
        sector[492..496].copy_from_slice(&(if unknown { u32::MAX } else { 3 }).to_le_bytes());
        sector[510..].copy_from_slice(&[0x55, 0xaa]);
    }
    image
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Event {
    Write(u64),
    Flush,
}

struct Disk {
    bytes: Vec<u8>,
    durable: Vec<u8>,
    fault: Option<Event>,
    events: Vec<Event>,
}

impl Disk {
    fn new(unknown: bool) -> Self {
        let bytes = fixture(unknown);
        Self {
            durable: bytes.clone(),
            bytes,
            fault: None,
            events: Vec::new(),
        }
    }

    fn event(&mut self, event: Event) -> Result<(), BlockError> {
        self.events.push(event);
        if self.fault == Some(event) {
            self.fault = None;
            return Err(BlockError::Io);
        }
        Ok(())
    }
}

impl BlockDevice for Disk {
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        MemDisk::new(&mut self.bytes).read_block(lba, buf)
    }

    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        self.event(Event::Write(lba))?;
        if (FAT..ROOT).contains(&lba) {
            assert_unknown(&self.durable);
        }
        MemDisk::new(&mut self.bytes).write_block(lba, buf)
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.event(Event::Flush)?;
        self.durable.copy_from_slice(&self.bytes);
        Ok(())
    }

    fn block_count(&self) -> u64 {
        (self.bytes.len() / BLOCK_SIZE) as u64
    }
}

fn assert_unknown(image: &[u8]) {
    for lba in INFO {
        let start = lba as usize * BLOCK_SIZE + 488;
        assert_eq!(image[start..start + 8], [0xff; 8]);
    }
}

#[test]
fn allocation_extension_truncate_and_delete_survive_remount() {
    for unknown in [false, true] {
        let mut disk = Disk::new(unknown);
        let path = b"/DATA.BIN";
        let mut fs = Fat32::mount(&mut disk).unwrap();
        fs.sample_capacity(u32::MAX).unwrap();
        assert_eq!(fs.capacity().unwrap().2, 64);
        fs.create_file_path(path).unwrap();
        fs.write_path(path, 0, &[0x5a; 512]).unwrap();
        fs.sync().unwrap();
        fs.write_path(path, 512, b"tail").unwrap();
        fs.sync().unwrap();
        assert_eq!(fs.capacity().unwrap().2, 62);
        assert_unknown(&disk.durable);
        assert_eq!(
            disk.events
                .iter()
                .filter(|event| matches!(event, Event::Write(lba) if INFO.contains(lba)))
                .count(),
            if unknown { 0 } else { 2 }
        );

        let mut fs = Fat32::mount(&mut disk).unwrap();
        fs.sample_capacity(u32::MAX).unwrap();
        assert_eq!(fs.capacity().unwrap().2, 62);
        let mut data = [0; 516];
        assert_eq!(fs.read_file(path, &mut data), Some(516));
        assert_eq!(data[..512], [0x5a; 512]);
        assert_eq!(&data[512..], b"tail");
        fs.truncate_path(path).unwrap();
        assert_eq!(fs.stat_path(path).unwrap().size, 0);
        assert_eq!(fs.capacity().unwrap().2, 64);
        fs.write_path(path, 0, b"again").unwrap();
        fs.unlink_path(path).unwrap();
        fs.sync().unwrap();

        let mut fs = Fat32::mount(MemDisk::new(&mut disk.durable)).unwrap();
        assert!(fs.stat_path(path).is_none());
        fs.sample_capacity(u32::MAX).unwrap();
        assert_eq!(fs.capacity().unwrap().2, 64);
    }
}

#[test]
fn fsinfo_write_and_barrier_failures_block_fat_mutation_until_retry() {
    for fault in [Event::Write(1), Event::Write(7), Event::Flush] {
        let mut disk = Disk::new(false);
        disk.fault = Some(fault);
        let before = disk.bytes.clone();
        let mut fs = Fat32::mount(&mut disk).unwrap();
        assert_eq!(fs.mkdir_path(b"/DIR"), Err(FatError::Io));
        assert_eq!(
            disk.bytes[FAT as usize * BLOCK_SIZE..],
            before[FAT as usize * BLOCK_SIZE..]
        );
        assert_eq!(disk.durable, before);
        assert!(!disk
            .events
            .iter()
            .any(|event| matches!(event, Event::Write(lba) if *lba >= FAT)));

        let mut fs = Fat32::mount(&mut disk).unwrap();
        fs.mkdir_path(b"/DIR").unwrap();
        fs.sync().unwrap();
        assert_unknown(&disk.durable);
    }
}

#[test]
fn failed_directory_publication_rolls_back_allocation_without_restoring_hints() {
    let mut disk = Disk::new(false);
    disk.fault = Some(Event::Write(ROOT));
    let before = disk.bytes.clone();
    let mut fs = Fat32::mount(&mut disk).unwrap();
    fs.sample_capacity(u32::MAX).unwrap();
    assert_eq!(fs.mkdir_path(b"/DIR"), Err(FatError::Io));
    assert_eq!(fs.capacity().unwrap().2, 64);
    assert!(fs.stat_path(b"/DIR").is_none());
    fs.sync().unwrap();
    assert_unknown(&disk.durable);
    assert_eq!(
        disk.durable[FAT as usize * BLOCK_SIZE..],
        before[FAT as usize * BLOCK_SIZE..]
    );
}

#[test]
fn deallocation_as_first_mutation_invalidates_a_populated_summary() {
    for truncate in [false, true] {
        let mut disk = Disk::new(false);
        let path = b"/EXISTING.BIN";
        let mut fs = Fat32::mount(&mut disk).unwrap();
        fs.create_file_path(path).unwrap();
        fs.write_path(path, 0, b"existing").unwrap();
        fs.sync().unwrap();

        for lba in INFO {
            let start = lba as usize * BLOCK_SIZE + 488;
            disk.bytes[start..start + 4].copy_from_slice(&63u32.to_le_bytes());
        }
        disk.durable.copy_from_slice(&disk.bytes);
        let mut fs = Fat32::mount(&mut disk).unwrap();
        fs.sample_capacity(u32::MAX).unwrap();
        assert_eq!(fs.capacity().unwrap().2, 63);
        if truncate {
            fs.truncate_path(path).unwrap();
            assert_eq!(fs.stat_path(path).unwrap().size, 0);
        } else {
            fs.unlink_path(path).unwrap();
            assert!(fs.stat_path(path).is_none());
        }
        assert_eq!(fs.capacity().unwrap().2, 64);
        fs.sync().unwrap();
        assert_unknown(&disk.durable);
    }
}
