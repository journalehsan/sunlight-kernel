#![no_std]

//! Block-device abstraction for SunlightOS storage.
//!
//! Sits below both `sunlight-fat` and `sunlight-fs` so filesystems and the
//! VFS share one device interface. The cache is the Rust equivalent of the
//! direct-mapped write-back scheme in Luxos `lxfs/src/blockio.c`
//! (slot = lba % N, tag = lba / N, dirty bit per slot, flush on evict),
//! with fixed-size slots instead of lazy heap buffers.

#[cfg(test)]
extern crate std;

/// All SunlightOS block devices use 512-byte sectors.
pub const BLOCK_SIZE: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockError {
    Io,
    OutOfRange,
    Unsupported,
}

pub trait BlockDevice {
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError>;
    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError>;
    fn block_count(&self) -> u64;
}

impl<D: BlockDevice + ?Sized> BlockDevice for &mut D {
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        (**self).read_block(lba, buf)
    }

    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        (**self).write_block(lba, buf)
    }

    fn block_count(&self) -> u64 {
        (**self).block_count()
    }
}

/// Placeholder device for VFS instances that only carry RAM-backed mounts.
pub struct NullDevice;

impl BlockDevice for NullDevice {
    fn read_block(&mut self, _lba: u64, _buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        Err(BlockError::Unsupported)
    }

    fn write_block(&mut self, _lba: u64, _buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        Err(BlockError::Unsupported)
    }

    fn block_count(&self) -> u64 {
        0
    }
}

struct CacheSlot {
    tag: u64,
    valid: bool,
    dirty: bool,
    data: [u8; BLOCK_SIZE],
}

impl CacheSlot {
    const EMPTY: Self = Self {
        tag: 0,
        valid: false,
        dirty: false,
        data: [0; BLOCK_SIZE],
    };
}

/// Direct-mapped write-back block cache. `N` slots of one block each;
/// `lba` maps to slot `lba % N` with tag `lba / N`.
pub struct CachedBlockDevice<D: BlockDevice, const N: usize> {
    inner: D,
    slots: [CacheSlot; N],
}

impl<D: BlockDevice, const N: usize> CachedBlockDevice<D, N> {
    pub const fn new(inner: D) -> Self {
        Self {
            inner,
            slots: [const { CacheSlot::EMPTY }; N],
        }
    }

    fn slot_lba(tag: u64, index: usize) -> u64 {
        tag * N as u64 + index as u64
    }

    /// Write one dirty slot back to the underlying device.
    fn flush_slot(&mut self, index: usize) -> Result<(), BlockError> {
        if !self.slots[index].valid || !self.slots[index].dirty {
            return Ok(());
        }
        let lba = Self::slot_lba(self.slots[index].tag, index);
        self.inner.write_block(lba, &self.slots[index].data)?;
        self.slots[index].dirty = false;
        Ok(())
    }

    /// Write all dirty slots back to the underlying device.
    pub fn flush(&mut self) -> Result<(), BlockError> {
        for index in 0..N {
            self.flush_slot(index)?;
        }
        Ok(())
    }

    pub fn inner(&self) -> &D {
        &self.inner
    }

    /// Flush and return the underlying device.
    pub fn into_inner(mut self) -> Result<D, BlockError> {
        self.flush()?;
        Ok(self.inner)
    }
}

impl<D: BlockDevice, const N: usize> BlockDevice for CachedBlockDevice<D, N> {
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let tag = lba / N as u64;
        let index = (lba % N as u64) as usize;

        if self.slots[index].valid && self.slots[index].tag == tag {
            buf.copy_from_slice(&self.slots[index].data);
            return Ok(());
        }

        // Evicting a different block: write it back first.
        self.flush_slot(index)?;

        self.inner.read_block(lba, &mut self.slots[index].data)?;
        self.slots[index].tag = tag;
        self.slots[index].valid = true;
        self.slots[index].dirty = false;
        buf.copy_from_slice(&self.slots[index].data);
        Ok(())
    }

    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let tag = lba / N as u64;
        let index = (lba % N as u64) as usize;

        if !(self.slots[index].valid && self.slots[index].tag == tag) {
            self.flush_slot(index)?;
        }

        self.slots[index].data.copy_from_slice(buf);
        self.slots[index].tag = tag;
        self.slots[index].valid = true;
        self.slots[index].dirty = true;
        Ok(())
    }

    fn block_count(&self) -> u64 {
        self.inner.block_count()
    }
}

/// RAM-backed block device over a borrowed byte slice. Used by filesystem
/// unit tests; also usable for initrd-style images.
pub struct MemDisk<'a> {
    data: &'a mut [u8],
}

impl<'a> MemDisk<'a> {
    pub fn new(data: &'a mut [u8]) -> Self {
        Self { data }
    }

    fn range(&self, lba: u64) -> Result<core::ops::Range<usize>, BlockError> {
        let start = (lba as usize).checked_mul(BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
        let end = start.checked_add(BLOCK_SIZE).ok_or(BlockError::OutOfRange)?;
        if end > self.data.len() {
            return Err(BlockError::OutOfRange);
        }
        Ok(start..end)
    }
}

impl BlockDevice for MemDisk<'_> {
    fn read_block(&mut self, lba: u64, buf: &mut [u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let range = self.range(lba)?;
        buf.copy_from_slice(&self.data[range]);
        Ok(())
    }

    fn write_block(&mut self, lba: u64, buf: &[u8; BLOCK_SIZE]) -> Result<(), BlockError> {
        let range = self.range(lba)?;
        self.data[range].copy_from_slice(buf);
        Ok(())
    }

    fn block_count(&self) -> u64 {
        (self.data.len() / BLOCK_SIZE) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    fn pattern_block(seed: u8) -> [u8; BLOCK_SIZE] {
        let mut block = [0u8; BLOCK_SIZE];
        for (i, b) in block.iter_mut().enumerate() {
            *b = seed.wrapping_add(i as u8);
        }
        block
    }

    #[test]
    fn memdisk_round_trips_and_bounds_checks() {
        let mut backing = vec![0u8; BLOCK_SIZE * 4];
        let mut disk = MemDisk::new(&mut backing);

        let block = pattern_block(7);
        disk.write_block(2, &block).unwrap();

        let mut readback = [0u8; BLOCK_SIZE];
        disk.read_block(2, &mut readback).unwrap();
        assert_eq!(readback, block);
        assert_eq!(disk.block_count(), 4);
        assert_eq!(disk.read_block(4, &mut readback), Err(BlockError::OutOfRange));
    }

    #[test]
    fn cache_serves_repeat_reads_and_writes_back_on_flush() {
        let mut backing = vec![0u8; BLOCK_SIZE * 8];
        let block = pattern_block(3);

        {
            let mut cached: CachedBlockDevice<_, 4> =
                CachedBlockDevice::new(MemDisk::new(&mut backing));
            cached.write_block(1, &block).unwrap();

            // Still dirty: readable through the cache...
            let mut readback = [0u8; BLOCK_SIZE];
            cached.read_block(1, &mut readback).unwrap();
            assert_eq!(readback, block);

            cached.flush().unwrap();
        }

        // ...and after flush the backing store has it.
        assert_eq!(&backing[BLOCK_SIZE..2 * BLOCK_SIZE], &block[..]);
    }

    #[test]
    fn cache_flushes_dirty_slot_on_collision_evict() {
        let mut backing = vec![0u8; BLOCK_SIZE * 8];
        let block = pattern_block(9);

        let mut cached: CachedBlockDevice<_, 4> =
            CachedBlockDevice::new(MemDisk::new(&mut backing));

        // LBA 1 and LBA 5 collide in a 4-slot cache (both map to slot 1).
        cached.write_block(1, &block).unwrap();
        let mut readback = [0u8; BLOCK_SIZE];
        cached.read_block(5, &mut readback).unwrap();

        // The dirty write to LBA 1 must have been evicted to the backing store.
        let mut direct = CachedBlockDevice::<_, 4>::new(MemDisk::new(&mut backing));
        direct.read_block(1, &mut readback).unwrap();
        assert_eq!(readback, block);
    }

    #[test]
    fn null_device_rejects_everything() {
        let mut dev = NullDevice;
        let mut buf = [0u8; BLOCK_SIZE];
        assert_eq!(dev.read_block(0, &mut buf), Err(BlockError::Unsupported));
        assert_eq!(dev.block_count(), 0);
    }
}
