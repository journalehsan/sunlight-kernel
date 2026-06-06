use super::{pmm::PhysicalMemoryManager, vmm::VirtualMemoryManager};
use linked_list_allocator::LockedHeap;
use x86_64::{
    VirtAddr,
    structures::paging::{
        Page, PageSize, Size4KiB,
        PageTableFlags,
        PhysFrame,
    },
};

pub const HEAP_START: VirtAddr = VirtAddr::new_truncate(0xFFFF_FFFF_9000_0000);
pub const HEAP_SIZE: usize = 1024 * 1024; // 1 MiB
pub const HEAP_PAGES: usize = HEAP_SIZE / Size4KiB::SIZE as usize;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();

pub fn init_heap(vmm: &mut VirtualMemoryManager, pmm: &mut PhysicalMemoryManager) {
    for i in 0..HEAP_PAGES {
        let page = Page::from_start_address(HEAP_START + i as u64 * Size4KiB::SIZE).unwrap();
        let frame = pmm.alloc_frame().expect("heap allocation failed");
        // SAFETY: frame address is valid and page-aligned; mapping new pages is safe.
        let phys = unsafe { PhysFrame::from_start_address_unchecked(frame) };
        vmm.map_page(
            page,
            phys,
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
            pmm,
        ).expect("heap page map failed");
    }

    // SAFETY: all heap pages are mapped with correct permissions.
    unsafe {
        ALLOCATOR.lock().init(HEAP_START.as_mut_ptr(), HEAP_SIZE);
    }
}
