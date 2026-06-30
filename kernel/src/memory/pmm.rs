use core::sync::atomic::{AtomicUsize, Ordering};
use limine::memmap::Entry;
use x86_64::PhysAddr;

const FRAME_SIZE: usize = 4096;
const MAX_FRAMES: usize = 4 * 1024 * 1024; // 16 GiB (bitmap-tracked region)
const BITMAP_SIZE: usize = MAX_FRAMES / 8;

static mut BITMAP: [u8; BITMAP_SIZE] = [0; BITMAP_SIZE];
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

pub struct PhysicalMemoryManager;

pub const PMM_OWNER_FREE: u32 = u32::MAX;
pub const PMM_OWNER_KERNEL: u32 = 0;

impl PhysicalMemoryManager {
    pub const fn new() -> Self {
        Self
    }

    /// Initialize from Limine memory map entries.
    /// SAFETY: Must be called exactly once before any alloc/free operations.
    pub unsafe fn init(&mut self, entries: &[&Entry]) {
        // Mark all as used initially.
        BITMAP.fill(0xFF);
        FRAME_OWNER.fill(PMM_OWNER_KERNEL);

        let mut total = 0usize;
        let mut free = 0usize;

        for entry in entries {
            if entry.type_ == limine::memmap::MEMMAP_USABLE {
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

        // Mark kernel frames as used.
        let _kernel_start = core::ptr::addr_of!(__kernel_start) as usize;
        let _kernel_end = core::ptr::addr_of!(__kernel_end) as usize;
        // The kernel is loaded at higher-half VA; subtract to get physical offset.
        // Limine loads the kernel base at 0xFFFFFFFF80000000, but the actual
        // physical address is not known. Use a conservative estimate:
        // mark the first 16 MiB of physical memory as used (covers kernel + bootloader + page tables).
        let start_frame = 0;
        let end_frame = (16 * 1024 * 1024) / FRAME_SIZE; // 16 MiB

        for f in start_frame..end_frame {
            if f < MAX_FRAMES {
                if BITMAP[f / 8] & (1 << (f % 8)) == 0 {
                    free -= 1;
                }
                BITMAP[f / 8] |= 1 << (f % 8);
                FRAME_OWNER[f] = PMM_OWNER_KERNEL;
            }
        }

        TOTAL_FRAMES = total;
        FREE_FRAMES = free;
    }

    /// Allocate one 4 KiB physical frame. Returns physical address.
    pub fn alloc_frame(&mut self) -> Option<PhysAddr> {
        self.alloc_frame_owned(PMM_OWNER_KERNEL)
    }

    /// Allocate one 4 KiB physical frame and record an owner PID.
    pub fn alloc_frame_owned(&mut self, owner_pid: u32) -> Option<PhysAddr> {
        let free = unsafe { FREE_FRAMES };
        if free == 0 {
            return None;
        }

        unsafe {
            for (byte_idx, byte) in BITMAP.iter_mut().enumerate() {
                if *byte != 0xFF {
                    for bit in 0..8 {
                        if *byte & (1 << bit) == 0 {
                            let frame = byte_idx * 8 + bit;
                            *byte |= 1 << bit;
                            FRAME_OWNER[frame] = owner_pid;
                            FREE_FRAMES -= 1;
                            let count = ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
                            if cfg!(feature = "verbose_diag") && (count % 100 == 0 || count < 10) {
                                crate::serial_println!(
                                    "[PMM] ALLOC #{} addr={:#x} free_now={}",
                                    count + 1,
                                    frame as u64 * FRAME_SIZE as u64,
                                    FREE_FRAMES
                                );
                            }
                            return Some(PhysAddr::new(frame as u64 * FRAME_SIZE as u64));
                        }
                    }
                }
            }
        }

        None
    }

    /// Allocate `count` physically-contiguous 4 KiB frames.
    /// Returns the physical address of the first frame on success.
    pub fn alloc_frames(&mut self, count: usize) -> Option<PhysAddr> {
        self.alloc_frames_owned(count, PMM_OWNER_KERNEL)
    }

    /// Allocate `count` physically-contiguous 4 KiB frames and record an owner PID.
    pub fn alloc_frames_owned(&mut self, count: usize, owner_pid: u32) -> Option<PhysAddr> {
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
                    FRAME_OWNER[f] = owner_pid;
                    FREE_FRAMES -= 1;
                }
            }
            return Some(PhysAddr::new(start as u64 * FRAME_SIZE as u64));
        }
        None
    }

    /// Free a previously allocated frame.
    #[allow(dead_code)]
    pub fn free_frame(&mut self, addr: PhysAddr) {
        let frame = (addr.as_u64() / FRAME_SIZE as u64) as usize;
        if frame < MAX_FRAMES {
            unsafe {
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

    /// Return (total_frames, free_frames) for diagnostics.
    pub fn stats(&self) -> (usize, usize) {
        unsafe { (TOTAL_FRAMES, FREE_FRAMES) }
    }

    pub fn owner_of(&self, addr: PhysAddr) -> Option<u32> {
        let frame = (addr.as_u64() / FRAME_SIZE as u64) as usize;
        if frame >= MAX_FRAMES {
            return None;
        }
        let owner = unsafe { FRAME_OWNER[frame] };
        (owner != PMM_OWNER_FREE).then_some(owner)
    }

    pub fn owned_frame_count(&self, owner_pid: u32) -> usize {
        unsafe {
            FRAME_OWNER
                .iter()
                .filter(|&&owner| owner == owner_pid)
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

impl Default for PhysicalMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}
