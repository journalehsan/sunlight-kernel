#![no_std]
//! Owned 4 KiB frames. The kernel installs its PMM/direct-map provider once.
//! Userspace providers use private mapped page addresses as opaque handles;
//! physical addresses are never exposed outside the kernel.
extern crate alloc;

pub const PAGE_SIZE: usize = 4096;

pub struct FrameAllocator {
    pub allocate: fn() -> Option<u64>,
    pub free: unsafe fn(u64),
    pub direct_map: unsafe fn(u64) -> *mut u8,
}
static ALLOCATOR: spin::Once<&'static FrameAllocator> = spin::Once::new();

/// Install the frame provider before mounting writable filesystems.
///
/// # Safety
/// Callbacks must return exclusive, page-aligned, mapped 4 KiB frames, remain
/// valid forever, and support concurrent callers. `free` must not allocate.
pub unsafe fn install_allocator(provider: &'static FrameAllocator) -> bool {
    core::ptr::eq(*ALLOCATOR.call_once(|| provider), provider)
}

/// Exclusively owned physical page; dropping it returns it to its provider.
/// The address cannot be forged or cloned through the safe API.
pub struct PhysFrame {
    address: u64,
    provider: &'static FrameAllocator,
}
impl PhysFrame {
    pub fn start_address(&self) -> u64 {
        self.address
    }
}
impl Drop for PhysFrame {
    fn drop(&mut self) {
        unsafe { (self.provider.free)(self.address) }
        #[cfg(all(
            feature = "testutil",
            any(target_os = "linux", target_os = "windows", target_os = "macos")
        ))]
        testutil::note_free();
    }
}

pub fn allocate_frame() -> Option<PhysFrame> {
    #[cfg(all(
        feature = "testutil",
        any(target_os = "linux", target_os = "windows", target_os = "macos")
    ))]
    if !testutil::allow_alloc() {
        return None;
    }
    let provider = ALLOCATOR.get().copied().or_else(host_provider)?;
    let address = (provider.allocate)()?;
    #[cfg(all(
        feature = "testutil",
        any(target_os = "linux", target_os = "windows", target_os = "macos")
    ))]
    testutil::note_alloc();
    Some(PhysFrame { address, provider })
}
pub fn free_frame(frame: PhysFrame) {
    drop(frame);
}
pub fn phys_to_virt(frame: &PhysFrame) -> *mut u8 {
    unsafe { (frame.provider.direct_map)(frame.address) }
}

// Hosted verification only: never used in the bare-metal kernel or services.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn host_provider() -> Option<&'static FrameAllocator> {
    use alloc::alloc::{alloc, dealloc, Layout};
    fn allocate() -> Option<u64> {
        let layout = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE).ok()?;
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            None
        } else {
            Some(ptr as u64)
        }
    }
    unsafe fn free(address: u64) {
        if let Ok(layout) = Layout::from_size_align(PAGE_SIZE, PAGE_SIZE) {
            unsafe { dealloc(address as *mut u8, layout) };
        }
    }
    unsafe fn direct_map(address: u64) -> *mut u8 {
        address as *mut u8
    }
    static HOST: FrameAllocator = FrameAllocator {
        allocate,
        free,
        direct_map,
    };
    Some(&HOST)
}
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn host_provider() -> Option<&'static FrameAllocator> {
    None
}

#[cfg(all(
    feature = "testutil",
    any(target_os = "linux", target_os = "windows", target_os = "macos")
))]
extern crate std;
/// Deterministic per-thread fault injection for hosted regression tests only.
#[cfg(all(
    feature = "testutil",
    any(target_os = "linux", target_os = "windows", target_os = "macos")
))]
pub mod testutil {
    use core::cell::Cell;
    std::thread_local! {
        static BUDGET: Cell<Option<usize>> = const { Cell::new(None) };
        static LIVE: Cell<usize> = const { Cell::new(0) };
    }
    pub fn fail_after(pages: Option<usize>) {
        BUDGET.set(pages);
    }
    pub fn live_frames() -> usize {
        LIVE.get()
    }
    pub(crate) fn allow_alloc() -> bool {
        BUDGET.with(|b| match b.get() {
            None => true,
            Some(0) => false,
            Some(n) => {
                b.set(Some(n - 1));
                true
            }
        })
    }
    pub(crate) fn note_alloc() {
        LIVE.set(LIVE.get() + 1);
    }
    pub(crate) fn note_free() {
        LIVE.set(LIVE.get() - 1);
    }
}
