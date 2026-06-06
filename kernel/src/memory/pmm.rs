use limine::memmap::Entry;
use x86_64::PhysAddr;

const FRAME_SIZE: usize = 4096;
const MAX_FRAMES: usize = 1024 * 1024; // 4 GiB
const BITMAP_SIZE: usize = MAX_FRAMES / 8;

static mut BITMAP: [u8; BITMAP_SIZE] = [0; BITMAP_SIZE];
static mut TOTAL_FRAMES: usize = 0;
static mut FREE_FRAMES: usize = 0;

extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

pub struct PhysicalMemoryManager;

impl PhysicalMemoryManager {
    pub const fn new() -> Self {
        Self
    }

    /// Initialize from Limine memory map entries.
    /// SAFETY: Must be called exactly once before any alloc/free operations.
    pub unsafe fn init(&mut self, entries: &[&Entry]) {
        // Mark all as used initially.
        BITMAP.fill(0xFF);

        let mut total = 0usize;
        let mut free = 0usize;

        for entry in entries {
            if entry.type_ == limine::memmap::MEMMAP_USABLE {
                let start_frame = (entry.base / FRAME_SIZE as u64) as usize;
                let end_frame = ((entry.base + entry.length + FRAME_SIZE as u64 - 1) / FRAME_SIZE as u64) as usize;

                for f in start_frame..end_frame {
                    if f < MAX_FRAMES {
                        BITMAP[f / 8] &= !(1 << (f % 8));
                        free += 1;
                    }
                    total += 1;
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
            }
        }

        TOTAL_FRAMES = total;
        FREE_FRAMES = free;
    }

    /// Allocate one 4 KiB physical frame. Returns physical address.
    pub fn alloc_frame(&mut self) -> Option<PhysAddr> {
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
                            FREE_FRAMES -= 1;
                            return Some(PhysAddr::new(frame as u64 * FRAME_SIZE as u64));
                        }
                    }
                }
            }
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
                FREE_FRAMES += 1;
            }
        }
    }

    /// Return (total_frames, free_frames) for diagnostics.
    pub fn stats(&self) -> (usize, usize) {
        unsafe { (TOTAL_FRAMES, FREE_FRAMES) }
    }
}

impl Default for PhysicalMemoryManager {
    fn default() -> Self {
        Self::new()
    }
}
