use core::sync::atomic::{AtomicUsize, Ordering};
use limine::memmap::Entry;
use x86_64::PhysAddr;

use super::accounting::{self, PhysicalMemoryClass};

const FRAME_SIZE: usize = 4096;
const MAX_FRAMES: usize = 4 * 1024 * 1024; // 16 GiB (bitmap-tracked region)
const BITMAP_SIZE: usize = MAX_FRAMES / 8;

static mut BITMAP: [u8; BITMAP_SIZE] = [0; BITMAP_SIZE];
/// Packed: high 8 bits = PhysicalMemoryClass, low 24 bits = owner PID.
/// FREE sentinel is u32::MAX (all bits set).
static mut FRAME_OWNER: [u32; MAX_FRAMES] = [u32::MAX; MAX_FRAMES];
static mut TOTAL_FRAMES: usize = 0;
static mut FREE_FRAMES: usize = 0;

// === Diagnostic counters for memory leak detection ===
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static FREE_COUNT: AtomicUsize = AtomicUsize::new(0);

extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

pub struct PhysicalMemoryManager {
    scan_cursor: usize,
    #[cfg(feature = "mm2a_test_injection")]
    test_allocations_before_failure: Option<usize>,
}

pub const PMM_OWNER_FREE: u32 = u32::MAX;
pub const PMM_OWNER_KERNEL: u32 = 0;

const OWNER_PID_MASK: u32 = 0x00FF_FFFF;
const OWNER_CLASS_SHIFT: u32 = 24;

#[inline]
fn pack_owner(owner_pid: u32, class: PhysicalMemoryClass) -> u32 {
    ((class.as_u8() as u32) << OWNER_CLASS_SHIFT) | (owner_pid & OWNER_PID_MASK)
}

#[inline]
fn unpack_pid(packed: u32) -> u32 {
    packed & OWNER_PID_MASK
}

#[inline]
fn unpack_class(packed: u32) -> PhysicalMemoryClass {
    PhysicalMemoryClass::from_u8((packed >> OWNER_CLASS_SHIFT) as u8)
        .unwrap_or(PhysicalMemoryClass::OtherAccounted)
}

#[derive(Clone, Copy, Debug)]
pub struct KernelReservedSpan {
    pub phys_start: u64,
    pub phys_end: u64,
    pub frame_count: usize,
}

impl PhysicalMemoryManager {
    pub const fn new() -> Self {
        Self {
            scan_cursor: 0,
            #[cfg(feature = "mm2a_test_injection")]
            test_allocations_before_failure: None,
        }
    }

    /// Initialize from Limine memory map entries.
    /// SAFETY: Must be called exactly once before any alloc/free operations.
    pub unsafe fn init(&mut self, entries: &[&Entry]) {
        // Mark all as used initially (not free; class will be set for kernel span only).
        BITMAP.fill(0xFF);
        FRAME_OWNER.fill(PMM_OWNER_FREE);

        let mut total = 0usize;
        let mut free = 0usize;
        let mut installed: u64 = 0;
        let mut usable: u64 = 0;
        let mut reserved_firmware: u64 = 0;

        for entry in entries {
            let len = entry.length;
            // Track installed conventional-ish RAM coverage.
            match entry.type_ {
                limine::memmap::MEMMAP_USABLE
                | limine::memmap::MEMMAP_BOOTLOADER_RECLAIMABLE
                | limine::memmap::MEMMAP_ACPI_RECLAIMABLE
                | limine::memmap::MEMMAP_ACPI_NVS
                | limine::memmap::MEMMAP_EXECUTABLE_AND_MODULES => {
                    installed = installed.saturating_add(len);
                }
                limine::memmap::MEMMAP_RESERVED | limine::memmap::MEMMAP_BAD_MEMORY => {
                    reserved_firmware = reserved_firmware.saturating_add(len);
                    installed = installed.saturating_add(len);
                }
                _ => {
                    // Framebuffer / MMIO etc.: do not count as ordinary RAM.
                }
            }

            if entry.type_ == limine::memmap::MEMMAP_USABLE {
                usable = usable.saturating_add(len);
                let start_frame = (entry.base / FRAME_SIZE as u64) as usize;
                let end_frame = ((entry.base + entry.length + FRAME_SIZE as u64 - 1)
                    / FRAME_SIZE as u64) as usize;

                for f in start_frame..end_frame {
                    // Only frames the bitmap can actually track count toward
                    // total/free. Counting frames beyond MAX_FRAMES in `total`
                    // (but never in `free`, since the bitmap can't represent
                    // them) would make `used = total - free` report that RAM as
                    // permanently allocated — the source of the bogus ~80% on
                    // machines with more than MAX_FRAMES of RAM.
                    if f < MAX_FRAMES {
                        BITMAP[f / 8] &= !(1 << (f % 8));
                        FRAME_OWNER[f] = PMM_OWNER_FREE;
                        free += 1;
                        total += 1;
                    }
                }
            }
        }

        accounting::set_installed_bytes(installed.max(usable));
        accounting::set_reserved_firmware_bytes(reserved_firmware);

        // Mark kernel frames as used (primary class: ReservedKernelImage).
        let kernel_start = core::ptr::addr_of!(__kernel_start) as usize;
        let kernel_end = core::ptr::addr_of!(__kernel_end) as usize;
        // The kernel is linked in the higher half at 0xFFFF_FFFF_8000_0000 and
        // Limine loads it contiguously starting at physical address 0. Reserve
        // the full loaded image span instead of a stale fixed 16 MiB window.
        let kernel_phys_base = 0xFFFF_FFFF_8000_0000usize;
        let kernel_phys_start = kernel_start.saturating_sub(kernel_phys_base);
        let kernel_phys_end = kernel_end.saturating_sub(kernel_phys_base);
        let start_frame = kernel_phys_start / FRAME_SIZE;
        let end_frame = kernel_phys_end.div_ceil(FRAME_SIZE);

        // Only frames that were previously free (i.e. in the managed usable set)
        // participate in managed accounting. Kernel pages outside MEMMAP_USABLE
        // must not inflate class counters relative to TOTAL_FRAMES.
        let mut kernel_managed_frames = 0u64;
        for f in start_frame..end_frame {
            if f < MAX_FRAMES {
                let was_free = BITMAP[f / 8] & (1 << (f % 8)) == 0;
                if was_free {
                    free -= 1;
                    kernel_managed_frames += 1;
                }
                BITMAP[f / 8] |= 1 << (f % 8);
                FRAME_OWNER[f] = pack_owner(
                    PMM_OWNER_KERNEL,
                    PhysicalMemoryClass::ReservedKernelImage,
                );
            }
        }
        accounting::note_alloc(
            PhysicalMemoryClass::ReservedKernelImage,
            kernel_managed_frames,
        );

        TOTAL_FRAMES = total;
        FREE_FRAMES = free;
    }

    /// Allocate one 4 KiB physical frame. Returns physical address.
    /// Default class: OtherAccounted (callers that know better should use
    /// `alloc_frame_class` / `alloc_frame_owned_class`).
    pub fn alloc_frame(&mut self) -> Option<PhysAddr> {
        self.alloc_frame_owned_class(PMM_OWNER_KERNEL, PhysicalMemoryClass::OtherAccounted)
    }

    /// Allocate one frame with an explicit primary accounting class.
    pub fn alloc_frame_class(&mut self, class: PhysicalMemoryClass) -> Option<PhysAddr> {
        self.alloc_frame_owned_class(PMM_OWNER_KERNEL, class)
    }

    /// Allocate one 4 KiB physical frame and record an owner PID.
    /// Default class for process-owned frames: UserPrivate.
    pub fn alloc_frame_owned(&mut self, owner_pid: u32) -> Option<PhysAddr> {
        self.alloc_frame_owned_class(owner_pid, PhysicalMemoryClass::UserPrivate)
    }

    /// Allocate one frame with owner PID and primary accounting class.
    pub fn alloc_frame_owned_class(
        &mut self,
        owner_pid: u32,
        class: PhysicalMemoryClass,
    ) -> Option<PhysAddr> {
        #[cfg(feature = "mm2a_test_injection")]
        if let Some(remaining) = self.test_allocations_before_failure.as_mut() {
            if *remaining == 0 {
                return None;
            }
            *remaining -= 1;
        }

        let free = unsafe { FREE_FRAMES };
        if free == 0 {
            return None;
        }

        unsafe {
            let cursor = self.scan_cursor;
            let (left, right) = BITMAP.split_at_mut(cursor);

            // First pass: scan from cursor to end
            for (rel_idx, byte) in right.iter_mut().enumerate() {
                if *byte != 0xFF {
                    let byte_idx = cursor + rel_idx;
                    for bit in 0..8 {
                        if *byte & (1 << bit) == 0 {
                            return self.claim_frame(byte_idx, bit, owner_pid, class);
                        }
                    }
                }
            }

            // Second pass: wrap around and scan from 0 to cursor
            for (byte_idx, byte) in left.iter_mut().enumerate() {
                if *byte != 0xFF {
                    for bit in 0..8 {
                        if *byte & (1 << bit) == 0 {
                            return self.claim_frame(byte_idx, bit, owner_pid, class);
                        }
                    }
                }
            }
        }

        None
    }

    #[cfg(feature = "mm2a_test_injection")]
    pub fn fail_test_allocations_after(&mut self, successful_allocations: usize) {
        self.test_allocations_before_failure = Some(successful_allocations);
    }

    #[cfg(feature = "mm2a_test_injection")]
    pub fn clear_test_allocation_failure(&mut self) {
        self.test_allocations_before_failure = None;
    }

    unsafe fn claim_frame(
        &mut self,
        byte_idx: usize,
        bit: usize,
        owner_pid: u32,
        class: PhysicalMemoryClass,
    ) -> Option<PhysAddr> {
        let frame = byte_idx * 8 + bit;
        BITMAP[byte_idx] |= 1 << bit;
        FRAME_OWNER[frame] = pack_owner(owner_pid, class);
        FREE_FRAMES -= 1;
        self.scan_cursor = byte_idx;
        accounting::note_alloc(class, 1);
        let count = ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        if cfg!(feature = "verbose_diag") && (count % 100 == 0 || count < 10) {
            crate::serial_println!(
                "[PMM] ALLOC #{} addr={:#x} free_now={} class={}",
                count + 1,
                frame as u64 * FRAME_SIZE as u64,
                FREE_FRAMES,
                class.as_u8()
            );
        }
        Some(PhysAddr::new(frame as u64 * FRAME_SIZE as u64))
    }

    /// Allocate `count` physically-contiguous 4 KiB frames.
    /// Returns the physical address of the first frame on success.
    pub fn alloc_frames(&mut self, count: usize) -> Option<PhysAddr> {
        self.alloc_frames_owned_class(count, PMM_OWNER_KERNEL, PhysicalMemoryClass::OtherAccounted)
    }

    pub fn alloc_frames_class(
        &mut self,
        count: usize,
        class: PhysicalMemoryClass,
    ) -> Option<PhysAddr> {
        self.alloc_frames_owned_class(count, PMM_OWNER_KERNEL, class)
    }

    /// Allocate `count` physically-contiguous 4 KiB frames and record an owner PID.
    pub fn alloc_frames_owned(&mut self, count: usize, owner_pid: u32) -> Option<PhysAddr> {
        self.alloc_frames_owned_class(count, owner_pid, PhysicalMemoryClass::UserPrivate)
    }

    pub fn alloc_frames_owned_class(
        &mut self,
        count: usize,
        owner_pid: u32,
        class: PhysicalMemoryClass,
    ) -> Option<PhysAddr> {
        if count == 0 {
            return None;
        }
        let total = unsafe { TOTAL_FRAMES };
        'search: for start in 0..total {
            let end = start + count;
            if end > total {
                break;
            }
            // Verify all frames in [start, end) are free
            for f in start..end {
                // SAFETY: BITMAP is only mutated under PMM lock (single-threaded kernel).
                if unsafe { BITMAP[f / 8] & (1 << (f % 8)) != 0 } {
                    continue 'search;
                }
            }
            // Mark all frames as allocated
            for f in start..end {
                // SAFETY: same as above.
                unsafe {
                    BITMAP[f / 8] |= 1 << (f % 8);
                    FRAME_OWNER[f] = pack_owner(owner_pid, class);
                    FREE_FRAMES -= 1;
                }
            }
            accounting::note_alloc(class, count as u64);
            return Some(PhysAddr::new(start as u64 * FRAME_SIZE as u64));
        }
        None
    }

    /// Free a previously allocated frame. Decrements the stored primary class.
    #[allow(dead_code)]
    pub fn free_frame(&mut self, addr: PhysAddr) {
        let frame = (addr.as_u64() / FRAME_SIZE as u64) as usize;
        if frame < MAX_FRAMES {
            unsafe {
                let packed = FRAME_OWNER[frame];
                if packed != PMM_OWNER_FREE {
                    let class = unpack_class(packed);
                    accounting::note_free(class, 1);
                }
                BITMAP[frame / 8] &= !(1 << (frame % 8));
                FRAME_OWNER[frame] = PMM_OWNER_FREE;
                FREE_FRAMES += 1;
            }
            let count = FREE_COUNT.fetch_add(1, Ordering::Relaxed);
            if cfg!(feature = "verbose_diag") && (count % 100 == 0 || count < 10) {
                crate::serial_println!(
                    "[PMM] FREE #{} addr={:#x} free_now={}",
                    count + 1,
                    addr.as_u64(),
                    unsafe { FREE_FRAMES }
                );
            }
        }
    }

    /// Explicit class transition for an allocated frame that stays allocated.
    pub fn reclass_frame(&mut self, addr: PhysAddr, new_class: PhysicalMemoryClass) {
        let frame = (addr.as_u64() / FRAME_SIZE as u64) as usize;
        if frame >= MAX_FRAMES {
            return;
        }
        unsafe {
            let packed = FRAME_OWNER[frame];
            if packed == PMM_OWNER_FREE {
                return;
            }
            let old = unpack_class(packed);
            let pid = unpack_pid(packed);
            if old == new_class {
                return;
            }
            accounting::note_reclass(old, new_class, 1);
            FRAME_OWNER[frame] = pack_owner(pid, new_class);
        }
    }

    pub fn class_of(&self, addr: PhysAddr) -> Option<PhysicalMemoryClass> {
        let frame = (addr.as_u64() / FRAME_SIZE as u64) as usize;
        if frame >= MAX_FRAMES {
            return None;
        }
        let packed = unsafe { FRAME_OWNER[frame] };
        if packed == PMM_OWNER_FREE {
            return Some(PhysicalMemoryClass::Free);
        }
        Some(unpack_class(packed))
    }

    /// Return (total_frames, free_frames) for diagnostics.
    pub fn stats(&self) -> (usize, usize) {
        unsafe { (TOTAL_FRAMES, FREE_FRAMES) }
    }

    /// Convenience: free page count (4KiB frames).
    pub fn free_page_count(&self) -> usize {
        unsafe { FREE_FRAMES }
    }

    /// Owner PID of a frame (class bits stripped).
    pub fn owner_of(&self, addr: PhysAddr) -> Option<u32> {
        let frame = (addr.as_u64() / FRAME_SIZE as u64) as usize;
        if frame >= MAX_FRAMES {
            return None;
        }
        let packed = unsafe { FRAME_OWNER[frame] };
        (packed != PMM_OWNER_FREE).then_some(unpack_pid(packed))
    }

    pub fn owned_frame_count(&self, owner_pid: u32) -> usize {
        let pid = owner_pid & OWNER_PID_MASK;
        unsafe {
            FRAME_OWNER
                .iter()
                .filter(|&&packed| packed != PMM_OWNER_FREE && unpack_pid(packed) == pid)
                .count()
        }
    }

    pub fn diagnostic_report_pid(&self, owner_pid: u32) {
        crate::serial_println!(
            "[PMM-DIAG] pid={} owned_frames={}",
            owner_pid,
            self.owned_frame_count(owner_pid)
        );
    }

    /// Print diagnostic information about memory allocation
    pub fn diagnostic_report(&self) {
        let (total, free) = self.stats();
        let allocated = total.saturating_sub(free);
        let alloc_ops = ALLOC_COUNT.load(Ordering::Relaxed);
        let free_ops = FREE_COUNT.load(Ordering::Relaxed);
        crate::serial_println!(
            "[PMM-DIAG] total={} free={} allocated={} alloc_ops={} free_ops={} delta={}",
            total,
            free,
            allocated,
            alloc_ops,
            free_ops,
            alloc_ops.saturating_sub(free_ops)
        );
    }
}

pub fn kernel_reserved_span() -> KernelReservedSpan {
    let kernel_start = core::ptr::addr_of!(__kernel_start) as usize;
    let kernel_end = core::ptr::addr_of!(__kernel_end) as usize;
    let kernel_phys_base = 0xFFFF_FFFF_8000_0000usize;
    let phys_start = kernel_start.saturating_sub(kernel_phys_base);
    let phys_end = kernel_end.saturating_sub(kernel_phys_base);
    let frame_count = phys_end.saturating_sub(phys_start).div_ceil(FRAME_SIZE);
    KernelReservedSpan {
        phys_start: phys_start as u64,
        phys_end: phys_end as u64,
        frame_count,
    }
}

impl Default for PhysicalMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}
