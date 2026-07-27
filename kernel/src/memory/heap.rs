use super::{pmm::PhysicalMemoryManager, vmm::VirtualMemoryManager};
use linked_list_allocator::LockedHeap;
use x86_64::{
    structures::paging::{Page, PageSize, PageTableFlags, PhysFrame, Size4KiB},
    VirtAddr,
};

pub const HEAP_START: VirtAddr = VirtAddr::new_truncate(0xFFFF_FFFF_9000_0000);
pub const HEAP_SIZE: usize = 8 * 1024 * 1024; // 8 MiB
pub const HEAP_PAGES: usize = HEAP_SIZE / Size4KiB::SIZE as usize;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub fn init_heap(vmm: &mut VirtualMemoryManager, pmm: &mut PhysicalMemoryManager) {
    use super::accounting::PhysicalMemoryClass;
    for i in 0..HEAP_PAGES {
        let page = Page::from_start_address(HEAP_START + i as u64 * Size4KiB::SIZE).unwrap();
        let frame = pmm
            .alloc_frame_class(PhysicalMemoryClass::KernelHeap)
            .expect("heap allocation failed");
        // SAFETY: frame address is valid and page-aligned; mapping new pages is safe.
        let phys = unsafe { PhysFrame::from_start_address_unchecked(frame) };
        vmm.map_page(
            page,
            phys,
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
            pmm,
        )
        .expect("heap page map failed");
    }

    // SAFETY: all heap pages are mapped with correct permissions.
    unsafe {
        ALLOCATOR.lock().init(HEAP_START.as_mut_ptr(), HEAP_SIZE);
    }
}

pub fn heap_diagnostic() {
    let heap = ALLOCATOR.lock();
    crate::serial_println!(
        "[HEAP-DIAG] total={} used={} free={}",
        heap.size(),
        heap.used(),
        heap.free(),
    );
}

/// Lightweight heap accounting snapshot for telemetry.
#[derive(Clone, Copy, Default)]
pub struct HeapStats {
    pub allocated: usize,
    pub free: usize,
    pub reusable: usize,
}

pub fn heap_stats() -> HeapStats {
    let heap = ALLOCATOR.lock();
    let used = heap.used();
    let free = heap.free();
    // "reusable" is the free pool; treat used as allocated.
    HeapStats {
        allocated: used,
        free,
        reusable: free,
    }
}
