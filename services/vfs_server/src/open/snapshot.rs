use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, Ordering};
use sunlight_fs::FsError;
use sunlight_ipc::vfs_rpc::file_transport::MAX_EXECUTABLE_BYTES;

struct Slot(Arc<AtomicBool>);

impl Slot {
    fn acquire(live: &Arc<AtomicBool>) -> Result<Self, FsError> {
        live.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| FsError::NoMemory)?;
        Ok(Self(Arc::clone(live)))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(crate) struct Snapshot {
    #[cfg(any(target_os = "none", target_os = "sunlightos"))]
    mapping: Mapping,
    #[cfg(not(any(target_os = "none", target_os = "sunlightos")))]
    bytes: alloc::vec::Vec<u8>,
    _slot: Slot,
}

impl Snapshot {
    pub(crate) fn allocate(size: usize, live: &Arc<AtomicBool>) -> Result<Self, FsError> {
        if size > MAX_EXECUTABLE_BYTES {
            return Err(FsError::NoMemory);
        }
        let slot = Slot::acquire(live)?;
        #[cfg(not(any(target_os = "none", target_os = "sunlightos")))]
        let bytes = {
            let mut bytes = alloc::vec::Vec::new();
            bytes.try_reserve_exact(size).map_err(|_| FsError::NoMemory)?;
            bytes.resize(size, 0);
            bytes
        };
        Ok(Self {
            #[cfg(any(target_os = "none", target_os = "sunlightos"))]
            mapping: Mapping::allocate(size)?,
            #[cfg(not(any(target_os = "none", target_os = "sunlightos")))]
            bytes,
            _slot: slot,
        })
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        #[cfg(any(target_os = "none", target_os = "sunlightos"))]
        return unsafe { core::slice::from_raw_parts(self.mapping.base as *const u8, self.mapping.size) };
        #[cfg(not(any(target_os = "none", target_os = "sunlightos")))]
        &self.bytes
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [u8] {
        #[cfg(any(target_os = "none", target_os = "sunlightos"))]
        return unsafe { core::slice::from_raw_parts_mut(self.mapping.base as *mut u8, self.mapping.size) };
        #[cfg(not(any(target_os = "none", target_os = "sunlightos")))]
        &mut self.bytes
    }
}

#[cfg(any(target_os = "none", target_os = "sunlightos"))]
struct Mapping {
    base: usize,
    size: usize,
}

#[cfg(any(target_os = "none", target_os = "sunlightos"))]
impl Mapping {
    fn allocate(size: usize) -> Result<Self, FsError> {
        use sunlight_libc::mman::*;
        let base = if size == 0 {
            core::ptr::NonNull::<u8>::dangling().as_ptr()
        } else {
            mmap(
                core::ptr::null_mut(),
                size,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS | MAP_NATIVE_THREAD,
                -1,
                0,
            )
            .map_err(|_| FsError::NoMemory)?
        };
        Ok(Self { base: base as usize, size })
    }
}

#[cfg(any(target_os = "none", target_os = "sunlightos"))]
impl Drop for Mapping {
    fn drop(&mut self) {
        if self.size != 0 {
            sunlight_libc::mman::munmap(self.base as *mut u8, self.size)
                .expect("owned executable mapping must unmap");
        }
    }
}
