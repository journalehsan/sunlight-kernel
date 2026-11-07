//! Bounded 512-byte-sector partition views and allocation-free table discovery.
use crate::{BlockDevice, BlockError, BLOCK_SIZE};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Partition {
    pub start: u64,
    pub blocks: u64,
    /// Eligible FAT container (MBR FAT32, GPT EFI System or Basic Data).
    pub fat_candidate: bool,
}

pub struct PartitionBlockDevice<D> {
    inner: D,
    partition: Partition,
}

impl<D: BlockDevice> PartitionBlockDevice<D> {
    pub fn new(inner: D, start: u64, blocks: u64) -> Result<Self, BlockError> {
        if blocks == 0
            || start
                .checked_add(blocks)
                .filter(|&end| end <= inner.block_count())
                .is_none()
        {
            return Err(BlockError::OutOfRange);
        }
        Ok(Self {
            inner,
            partition: Partition {
                start,
                blocks,
                fat_candidate: false,
            },
        })
    }

    fn translate(&self, lba: u64) -> Result<u64, BlockError> {
        if lba >= self.partition.blocks {
            return Err(BlockError::OutOfRange);
        }
        self.partition
            .start
            .checked_add(lba)
            .ok_or(BlockError::OutOfRange)
    }
}

impl<D: BlockDevice> BlockDevice for PartitionBlockDevice<D> {
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let disk_lba = self.translate(lba)?;
        self.inner.read_block(disk_lba, buf)
    }
    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let disk_lba = self.translate(lba)?;
        self.inner.write_block(disk_lba, buf)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
    fn block_count(&self) -> u64 {
        self.partition.blocks
    }
}

pub const MAX_PARTITIONS: usize = 128;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Superfloppy,
    Mbr,
    Gpt,
}
#[derive(Debug, PartialEq, Eq)]
pub enum TableError {
    Io(BlockError),
    Malformed,
    Unsupported,
}
impl From<BlockError> for TableError {
    fn from(e: BlockError) -> Self {
        Self::Io(e)
    }
}
pub struct PartitionTable {
    pub layout: Layout,
    entries: [Partition; MAX_PARTITIONS],
    count: usize,
}
impl PartitionTable {
    pub fn partitions(&self) -> &[Partition] {
        &self.entries[..self.count]
    }
    fn push(&mut self, p: Partition, disk_blocks: u64) -> Result<(), TableError> {
        let end = p.start.checked_add(p.blocks).ok_or(TableError::Malformed)?;
        if p.start == 0 || p.blocks == 0 || end > disk_blocks || self.count == MAX_PARTITIONS {
            return Err(TableError::Malformed);
        }
        for q in self.partitions() {
            if p.start < q.start + q.blocks && q.start < end {
                return Err(TableError::Malformed);
            }
        }
        self.entries[self.count] = p;
        self.count += 1;
        Ok(())
    }
}
fn u32le(s: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(s[o..o + 4].try_into().unwrap())
}
fn u64le(s: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(s[o..o + 8].try_into().unwrap())
}
fn crc_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb88320 & (0u32.wrapping_sub(crc & 1)));
        }
    }
    crc
}
fn crc(data: &[u8]) -> u32 {
    !crc_update(!0, data)
}

/// No EBRs, hybrid MBRs, GPT recovery, >128 entries, or non-128-byte GPT
/// entries. Both GPT copies must validate and agree. No writes during discovery.
pub fn discover<D: BlockDevice>(disk: &mut D) -> Result<PartitionTable, TableError> {
    let mut s = [0; BLOCK_SIZE];
    disk.read_block(0, &mut s)?;
    let mut t = PartitionTable {
        layout: Layout::Mbr,
        entries: [Partition::default(); MAX_PARTITIONS],
        count: 0,
    };
    // Historical FAT images may deliberately clear the BIOS boot signature.
    if &s[82..90] == b"FAT32   " && s[11..13] == [0, 2] && (s[0] == 0xeb || s[0] == 0xe9) {
        // Never let FAT-like bytes hide an MBR/protective table. This
        // milestone supports the existing mkfs.fat superfloppy fixtures,
        // whose partition-entry area is empty.
        if s[446..510].iter().any(|&b| b != 0) {
            return Err(TableError::Unsupported);
        }
        let mut next = [0; BLOCK_SIZE];
        if disk.block_count() > 1 {
            disk.read_block(1, &mut next)?;
            if &next[..8] == b"EFI PART" {
                return Err(TableError::Malformed);
            }
        }
        t.layout = Layout::Superfloppy;
        return Ok(t);
    }
    if s[510..512] != [0x55, 0xaa] {
        return Err(TableError::Malformed);
    }
    let mut protective = false;
    for e in s[446..510].chunks_exact(16) {
        if e.iter().all(|&b| b == 0) {
            continue;
        }
        if e[0] != 0 && e[0] != 0x80 {
            return Err(TableError::Malformed);
        }
        let kind = e[4];
        if kind == 0 {
            return Err(TableError::Malformed);
        }
        if matches!(kind, 0x05 | 0x0f | 0x85) {
            return Err(TableError::Unsupported);
        }
        let p = Partition {
            start: u32le(e, 8) as u64,
            blocks: u32le(e, 12) as u64,
            fat_candidate: matches!(kind, 0x0b | 0x0c),
        };
        if kind == 0xee {
            if protective
                || e[0] != 0
                || p.start != 1
                || p.blocks != disk.block_count().saturating_sub(1).min(u32::MAX as u64)
            {
                return Err(TableError::Malformed);
            }
            protective = true;
        }
        t.push(p, disk.block_count())?;
    }
    if protective {
        if t.count != 1 {
            return Err(TableError::Unsupported);
        }
        t.count = 0;
        t.layout = Layout::Gpt;
        read_gpt(disk, &mut t)?;
    } else {
        // A GPT signature without a protective MBR must not become ordinary MBR.
        disk.read_block(1, &mut s)?;
        if &s[..8] == b"EFI PART" {
            return Err(TableError::Malformed);
        }
        if t.count == 0 {
            return Err(TableError::Unsupported);
        }
    }
    Ok(t)
}

fn header<D: BlockDevice>(
    disk: &mut D,
    lba: u64,
    alternate: u64,
) -> Result<[u8; BLOCK_SIZE], TableError> {
    let mut h = [0; BLOCK_SIZE];
    disk.read_block(lba, &mut h)?;
    if &h[..8] != b"EFI PART"
        || u32le(&h, 8) != 0x10000
        || u32le(&h, 20) != 0
        || u64le(&h, 24) != lba
        || u64le(&h, 32) != alternate
    {
        return Err(TableError::Malformed);
    }
    let size = u32le(&h, 12) as usize;
    if size != 92 {
        return Err(TableError::Unsupported);
    }
    let expected = u32le(&h, 16);
    h[16..20].fill(0);
    if crc(&h[..size]) != expected {
        return Err(TableError::Malformed);
    }
    if u32le(&h, 80) == 0 || u32le(&h, 80) > MAX_PARTITIONS as u32 || u32le(&h, 84) != 128 {
        return Err(TableError::Unsupported);
    }
    Ok(h)
}
fn read_gpt<D: BlockDevice>(disk: &mut D, t: &mut PartitionTable) -> Result<(), TableError> {
    let last = disk
        .block_count()
        .checked_sub(1)
        .ok_or(TableError::Malformed)?;
    let h = header(disk, 1, last)?;
    let b = header(disk, last, 1)?;
    if h[40..72] != b[40..72] || h[80..92] != b[80..92] || h[56..72].iter().all(|&v| v == 0) {
        return Err(TableError::Malformed);
    }
    let first = u64le(&h, 40);
    let end = u64le(&h, 48);
    let count = u32le(&h, 80) as usize;
    let sectors = (count * 128).div_ceil(BLOCK_SIZE) as u64;
    let primary = u64le(&h, 72);
    let backup = u64le(&b, 72);
    if first > end
        || end >= last
        || primary < 2
        || primary
            .checked_add(sectors)
            .filter(|&v| v <= first)
            .is_none()
        || backup <= end
        || backup.checked_add(sectors).filter(|&v| v <= last).is_none()
    {
        return Err(TableError::Malformed);
    }
    let mut sum = !0;
    let mut ids = [[0u8; 16]; MAX_PARTITIONS];
    for sector in 0..sectors {
        let mut s = [0; BLOCK_SIZE];
        let mut mirror = [0; BLOCK_SIZE];
        disk.read_block(primary + sector, &mut s)?;
        disk.read_block(backup + sector, &mut mirror)?;
        let bytes = (count * 128 - sector as usize * BLOCK_SIZE).min(BLOCK_SIZE);
        if s[..bytes] != mirror[..bytes] {
            return Err(TableError::Malformed);
        }
        sum = crc_update(sum, &s[..bytes]);
        for e in s[..bytes].chunks_exact(128) {
            if e[..16].iter().all(|&v| v == 0) {
                continue;
            }
            let id: [u8; 16] = e[16..32].try_into().unwrap();
            if id == [0; 16] || ids[..t.count].contains(&id) {
                return Err(TableError::Malformed);
            }
            ids[t.count] = id;
            let start = u64le(e, 32);
            let finish = u64le(e, 40);
            if start < first || finish > end || finish < start {
                return Err(TableError::Malformed);
            }
            // Microsoft Basic Data and EFI System GUIDs in on-disk byte order.
            let basic = [
                0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26,
                0x99, 0xc7,
            ];
            let esp = [
                0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0, 0xa0, 0xc9, 0x3e,
                0xc9, 0x3b,
            ];
            t.push(
                Partition {
                    start,
                    blocks: finish - start + 1,
                    fat_candidate: (e[..16] == basic || e[..16] == esp) && u64le(e, 48) == 0,
                },
                disk.block_count(),
            )?;
        }
    }
    if !sum != u32le(&h, 88) {
        return Err(TableError::Malformed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CachedBlockDevice, MemDisk};
    use std::vec;
    use std::vec::Vec;
    fn put32(s: &mut [u8], o: usize, v: u32) {
        s[o..o + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn put64(s: &mut [u8], o: usize, v: u64) {
        s[o..o + 8].copy_from_slice(&v.to_le_bytes());
    }
    fn mbr() -> Vec<u8> {
        let mut v = vec![0; 512 * 100];
        v[510..512].copy_from_slice(&[0x55, 0xaa]);
        v[450] = 0x0c;
        put32(&mut v, 454, 10);
        put32(&mut v, 458, 20);
        v
    }
    fn gpt() -> Vec<u8> {
        let mut v = mbr();
        v[450] = 0xee;
        put32(&mut v, 454, 1);
        put32(&mut v, 458, 99);
        let mut entries = [0; 512];
        entries[..16].copy_from_slice(&[
            0xa2, 0xa0, 0xd0, 0xeb, 0xe5, 0xb9, 0x33, 0x44, 0x87, 0xc0, 0x68, 0xb6, 0xb7, 0x26,
            0x99, 0xc7,
        ]);
        entries[16] = 1;
        put64(&mut entries, 32, 10);
        put64(&mut entries, 40, 29);
        for (lba, other, array) in [(1, 99, 2), (99, 1, 98)] {
            let h = &mut v[lba * 512..(lba + 1) * 512];
            h[..8].copy_from_slice(b"EFI PART");
            put32(h, 8, 0x10000);
            put32(h, 12, 92);
            put64(h, 24, lba as u64);
            put64(h, 32, other);
            put64(h, 40, 3);
            put64(h, 48, 97);
            h[56] = 42;
            put64(h, 72, array as u64);
            put32(h, 80, 4);
            put32(h, 84, 128);
            put32(h, 88, crc(&entries));
            let c = crc(&h[..92]);
            put32(h, 16, c);
            v[array * 512..(array + 1) * 512].copy_from_slice(&entries);
        }
        v
    }
    fn parse(v: &mut [u8]) -> Result<PartitionTable, TableError> {
        discover(&mut MemDisk::new(v))
    }
    fn bad(v: &mut [u8]) {
        assert!(parse(v).is_err());
    }
    fn repair_header(v: &mut [u8], lba: usize) {
        let h = &mut v[lba * 512..(lba + 1) * 512];
        put32(h, 16, 0);
        let c = crc(&h[..92]);
        put32(h, 16, c);
    }
    fn repair_arrays(v: &mut [u8]) {
        let c = crc(&v[1024..1536]);
        v.copy_within(1024..1536, 98 * 512);
        for lba in [1, 99] {
            put32(v, lba * 512 + 88, c);
            repair_header(v, lba);
        }
    }
    #[test]
    fn bounded_reads_writes_and_cache() {
        let mut v = vec![0x55; 512 * 8];
        {
            let mut disk = MemDisk::new(&mut v);
            assert!(PartitionBlockDevice::new(&mut disk, u64::MAX, 2).is_err());
            assert!(PartitionBlockDevice::new(&mut disk, 7, 2).is_err());
            assert!(PartitionBlockDevice::new(&mut disk, 0, 0).is_err());
            let view = PartitionBlockDevice::new(&mut disk, 2, 3).unwrap();
            let mut cache = CachedBlockDevice::<_, 2>::new(view);
            cache.write_block(0, &[1; 512]).unwrap();
            cache.write_block(2, &[2; 512]).unwrap();
            let mut out = [0; 512];
            for lba in [3, u64::MAX] {
                assert_eq!(cache.write_block(lba, &out), Err(BlockError::OutOfRange));
                assert_eq!(cache.read_block(lba, &mut out), Err(BlockError::OutOfRange));
            }
            cache.flush().unwrap();
            cache.read_block(2, &mut out).unwrap();
            assert_eq!(out, [2; 512]);
        }
        assert!(v[..1024].iter().all(|&v| v == 0x55));
        assert!(v[5 * 512..].iter().all(|&v| v == 0x55));
        let mut disk = MemDisk::new(&mut v);
        assert_eq!(
            disk.write_block(u64::MAX, &[0; 512]),
            Err(BlockError::OutOfRange)
        );
    }
    #[test]
    fn adapter_rejects_relative_boundary_before_forwarding() {
        let mut bytes = vec![0x55; 8 * BLOCK_SIZE];
        {
            let mut view = PartitionBlockDevice::new(MemDisk::new(&mut bytes), 2, 3).unwrap();
            assert_eq!(view.block_count(), 3);
            view.write_block(0, &[1; BLOCK_SIZE]).unwrap();
            view.write_block(2, &[2; BLOCK_SIZE]).unwrap();
            let mut out = [0; BLOCK_SIZE];
            view.read_block(0, &mut out).unwrap();
            assert_eq!(out, [1; BLOCK_SIZE]);
            view.read_block(2, &mut out).unwrap();
            assert_eq!(out, [2; BLOCK_SIZE]);
            for lba in [3, u64::MAX] {
                assert_eq!(view.read_block(lba, &mut out), Err(BlockError::OutOfRange));
                assert_eq!(view.write_block(lba, &out), Err(BlockError::OutOfRange));
            }
            view.flush().unwrap();
        }
        assert!(bytes[..2 * BLOCK_SIZE].iter().all(|&b| b == 0x55));
        assert!(bytes[5 * BLOCK_SIZE..].iter().all(|&b| b == 0x55));
    }

    #[test]
    fn flush_propagates_failure() {
        struct Barrier;
        impl BlockDevice for Barrier {
            fn read_block(&mut self, _: u64, _: &mut [u8; 512]) -> Result<(), BlockError> {
                Ok(())
            }
            fn write_block(&mut self, _: u64, _: &[u8; 512]) -> Result<(), BlockError> {
                Ok(())
            }
            fn flush(&mut self) -> Result<(), BlockError> {
                Err(BlockError::Unsupported)
            }
            fn block_count(&self) -> u64 {
                u64::MAX
            }
        }
        let mut p = PartitionBlockDevice::new(Barrier, u64::MAX - 1, 1).unwrap();
        assert_eq!(p.flush(), Err(BlockError::Unsupported));
        assert!(PartitionBlockDevice::new(Barrier, u64::MAX, 1).is_err());
    }
    #[test]
    fn valid_mbr_and_gpt() {
        for (mut v, layout) in [(mbr(), Layout::Mbr), (gpt(), Layout::Gpt)] {
            let t = parse(&mut v).unwrap();
            assert_eq!(t.layout, layout);
            assert_eq!(
                t.partitions(),
                &[Partition {
                    start: 10,
                    blocks: 20,
                    fat_candidate: true
                }]
            );
        }
    }
    #[test]
    fn rejects_bad_mbr_layouts() {
        for (offset, value) in [(510, 0), (450, 0), (446, 1), (450, 5)] {
            let mut v = mbr();
            v[offset] = value;
            bad(&mut v);
        }
        for (start, size) in [(0, 20), (90, 20), (10, 0), (u32::MAX, u32::MAX)] {
            let mut v = mbr();
            put32(&mut v, 454, start);
            put32(&mut v, 458, size);
            bad(&mut v);
        }
        let mut v = mbr();
        v.copy_within(446..462, 462);
        bad(&mut v);
        let mut v = gpt();
        v[466] = 0x0c;
        put32(&mut v, 470, 10);
        put32(&mut v, 474, 1);
        bad(&mut v);
        let mut v = gpt();
        v[450] = 0x0c;
        bad(&mut v);
        let mut v = gpt();
        put32(&mut v, 458, 98);
        bad(&mut v);
    }
    #[test]
    fn rejects_gpt_corruption_in_both_copies() {
        for offset in [
            512,
            512 + 16,
            512 + 40,
            1024 + 60,
            98 * 512 + 60,
            99 * 512 + 16,
        ] {
            let mut v = gpt();
            v[offset] ^= 1;
            bad(&mut v);
        }
    }
    #[test]
    fn rejects_crc_valid_unsafe_gpt_geometry() {
        for (offset, value) in [(72, 99), (40, 0), (48, 99), (32, 98)] {
            let mut v = gpt();
            put64(&mut v, 512 + offset, value);
            repair_header(&mut v, 1);
            bad(&mut v);
        }
        let mut v = gpt();
        put32(&mut v, 512 + 80, 129);
        repair_header(&mut v, 1);
        bad(&mut v);
        let mut v = gpt();
        put64(&mut v, 1024 + 40, u64::MAX);
        repair_arrays(&mut v);
        bad(&mut v);
        let mut v = gpt();
        v.copy_within(1024..1152, 1152);
        v[1152 + 16] = 2;
        repair_arrays(&mut v);
        bad(&mut v);
        let mut v = gpt();
        v[1024 + 16] = 0;
        repair_arrays(&mut v);
        bad(&mut v);
    }
    #[test]
    fn legacy_superfloppy_and_sector_constraint() {
        let mut v = vec![0; 1024];
        v[0] = 0xeb;
        v[11..13].copy_from_slice(&512u16.to_le_bytes());
        v[82..90].copy_from_slice(b"FAT32   ");
        assert_eq!(parse(&mut v).unwrap().layout, Layout::Superfloppy);
        v[450] = 0xee;
        bad(&mut v);
        v[450] = 0;
        v[12] = 16;
        bad(&mut v);
    }
    #[test]
    fn truncated_tables_and_all_single_byte_gpt_mutations_fail_closed() {
        let mut v = gpt();
        v.truncate(99 * 512);
        bad(&mut v);
        for offset in (512..604).chain(1024..1152).chain(99 * 512..99 * 512 + 92) {
            let mut v = gpt();
            v[offset] ^= 0x80;
            bad(&mut v);
        }
    }
}
