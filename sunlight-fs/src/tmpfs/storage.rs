use crate::{mode, FsError};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicUsize, Ordering};
use sunlight_mm::{allocate_frame, free_frame, phys_to_virt, PhysFrame, PAGE_SIZE};

pub const TMPFS_DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;
pub const TMPFS_DEFAULT_MAX_INODES: usize = 4096;
pub const TMPFS_PATH_MAX: usize = 4096;

/// Per-mount quotas. Bytes count allocated pages, including reserved capacity.
/// Embedded read-only initramfs payloads are part of the kernel image and are
/// excluded; their inodes are counted at mount time.
pub struct TmpfsLimits {
    pub max_bytes: usize,
    pub max_inodes: usize,
    used_bytes: AtomicUsize,
    pub(super) used_inodes: AtomicUsize,
}
impl TmpfsLimits {
    pub const fn new(max_bytes: usize, max_inodes: usize) -> Self {
        Self {
            max_bytes,
            max_inodes,
            used_bytes: AtomicUsize::new(0),
            used_inodes: AtomicUsize::new(0),
        }
    }
    fn charge(counter: &AtomicUsize, amount: usize, max: usize) -> Result<(), FsError> {
        counter
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                old.checked_add(amount).filter(|new| *new <= max)
            })
            .map(|_| ())
            .map_err(|_| FsError::NoSpace)
    }
    pub fn try_alloc_bytes(&self, bytes: usize) -> Result<(), FsError> {
        Self::charge(&self.used_bytes, bytes, self.max_bytes)
    }
    pub fn free_bytes(&self, bytes: usize) {
        let _ = self
            .used_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                old.checked_sub(bytes)
            });
    }
    pub fn try_alloc_inode(&self) -> Result<(), FsError> {
        Self::charge(&self.used_inodes, 1, self.max_inodes)
    }
    pub fn free_inode(&self) {
        let _ = self
            .used_inodes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |old| {
                old.checked_sub(1)
            });
    }
    pub fn used_bytes(&self) -> usize {
        self.used_bytes.load(Ordering::Acquire)
    }
    pub fn used_inodes(&self) -> usize {
        self.used_inodes.load(Ordering::Acquire)
    }
}

pub struct TmpfsInode {
    pub id: u64,
    pub mode: u16,
    pub(super) size: usize,
    pub(super) pages: Vec<PhysFrame>,
    pub(super) limits: Arc<TmpfsLimits>,
    pub(super) charged_inode: bool,
}
impl TmpfsInode {
    pub fn new(id: u64, mode: u16, limits: Arc<TmpfsLimits>) -> Result<Self, FsError> {
        limits.try_alloc_inode()?;
        Ok(Self {
            id,
            mode,
            size: 0,
            pages: Vec::new(),
            limits,
            charged_inode: true,
        })
    }
    pub fn size(&self) -> usize {
        self.size
    }
    pub fn allocated_pages(&self) -> usize {
        self.pages.len()
    }
    pub fn read_at(&self, offset: usize, buffer: &mut [u8]) -> Result<usize, FsError> {
        if self.mode & mode::S_IFDIR != 0 {
            return Err(FsError::IsDir);
        }
        let count = buffer.len().min(self.size.saturating_sub(offset));
        let mut done = 0;
        while done < count {
            let pos = offset + done;
            let n = (PAGE_SIZE - pos % PAGE_SIZE).min(count - done);
            // SAFETY: the frame is exclusively owned, mapped, and pos is in size.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    phys_to_virt(&self.pages[pos / PAGE_SIZE]).add(pos % PAGE_SIZE),
                    buffer.as_mut_ptr().add(done),
                    n,
                );
            }
            done += n;
        }
        Ok(count)
    }
    pub(super) fn reserve_pages(&mut self, size: usize) -> Result<(), FsError> {
        let count = size / PAGE_SIZE + usize::from(size % PAGE_SIZE != 0);
        if count <= self.pages.len() {
            return Ok(());
        }
        let extra = count - self.pages.len();
        let bytes = extra.checked_mul(PAGE_SIZE).ok_or(FsError::NoSpace)?;
        self.limits.try_alloc_bytes(bytes)?;
        if self.pages.try_reserve_exact(extra).is_err() {
            self.limits.free_bytes(bytes);
            return Err(FsError::NoMemory);
        }
        let original = self.pages.len();
        for _ in 0..extra {
            let Some(frame) = allocate_frame() else {
                while self.pages.len() > original {
                    self.pages.pop();
                }
                self.limits.free_bytes(bytes);
                return Err(FsError::NoMemory);
            };
            // SAFETY: a fresh exclusive mapped 4 KiB page. Clear before exposure.
            unsafe {
                core::ptr::write_bytes(phys_to_virt(&frame), 0, PAGE_SIZE);
            }
            self.pages.push(frame);
        }
        Ok(())
    }
    pub fn write_at(
        &mut self,
        offset: usize,
        buffer: &[u8],
        limits: &TmpfsLimits,
    ) -> Result<usize, FsError> {
        if self.mode & mode::S_IFDIR != 0 {
            return Err(FsError::IsDir);
        }
        if !core::ptr::eq(limits, self.limits.as_ref()) {
            return Err(FsError::InvalidPath);
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        let end = offset.checked_add(buffer.len()).ok_or(FsError::NoSpace)?;
        self.reserve_pages(end)?;
        let mut done = 0;
        while done < buffer.len() {
            let pos = offset + done;
            let n = (PAGE_SIZE - pos % PAGE_SIZE).min(buffer.len() - done);
            // SAFETY: reserve_pages established exclusive backing for the range.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    buffer.as_ptr().add(done),
                    phys_to_virt(&self.pages[pos / PAGE_SIZE]).add(pos % PAGE_SIZE),
                    n,
                );
            }
            done += n;
        }
        self.size = self.size.max(end);
        Ok(done)
    }
    /// Shrink or extend with zeros. Allocation failure leaves size/data intact.
    pub fn truncate_to(&mut self, size: usize) -> Result<(), FsError> {
        if self.mode & mode::S_IFDIR != 0 {
            return Err(FsError::IsDir);
        }
        if size > self.size {
            self.reserve_pages(size)?;
        }
        let count = size / PAGE_SIZE + usize::from(size % PAGE_SIZE != 0);
        while self.pages.len() > count {
            if let Some(frame) = self.pages.pop() {
                free_frame(frame);
                self.limits.free_bytes(PAGE_SIZE);
            }
        }
        if size % PAGE_SIZE != 0 {
            // Clear the retained tail so later extensions cannot reveal old data.
            unsafe {
                core::ptr::write_bytes(
                    phys_to_virt(&self.pages[count - 1]).add(size % PAGE_SIZE),
                    0,
                    PAGE_SIZE - size % PAGE_SIZE,
                );
            }
        }
        self.size = size;
        Ok(())
    }
}
impl Drop for TmpfsInode {
    fn drop(&mut self) {
        let bytes = self.pages.len() * PAGE_SIZE;
        self.pages.clear();
        self.limits.free_bytes(bytes);
        if self.charged_inode {
            self.limits.free_inode();
        }
    }
}
