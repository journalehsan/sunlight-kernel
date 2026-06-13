use super::pmm::PhysicalMemoryManager;
use x86_64::{
    structures::paging::{
        mapper::{MapToError, Mapper},
        page::{Page, Size4KiB},
        FrameAllocator, OffsetPageTable, PageTable, PageTableFlags, PhysFrame, Translate,
    },
    PhysAddr, VirtAddr,
};

pub struct VirtualMemoryManager {
    page_table: OffsetPageTable<'static>,
}

impl VirtualMemoryManager {
    /// Initialize from Limine HHDM offset and current CR3.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base. `cr3` must point to valid page tables.
    pub unsafe fn init(hhdm_offset: VirtAddr) -> Self {
        let level_4_table = {
            let phys = x86_64::registers::control::Cr3::read().0.start_address();
            let virt = hhdm_offset + phys.as_u64();
            &mut *(virt.as_mut_ptr::<PageTable>())
        };

        let page_table = OffsetPageTable::new(level_4_table, hhdm_offset);

        Self { page_table }
    }

    /// Map a virtual page to a physical frame with given flags.
    pub fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        phys: PhysFrame<Size4KiB>,
        flags: PageTableFlags,
        pmm: &mut PhysicalMemoryManager,
    ) -> Result<(), MapToError<Size4KiB>> {
        let mut alloc = PmmFrameAllocator { pmm };
        // SAFETY: caller ensures the mapping does not cause UB.
        let flush = unsafe { self.page_table.map_to(page, phys, flags, &mut alloc) }?;
        flush.flush();
        Ok(())
    }

    /// Unmap a virtual page, returning the physical frame.
    #[allow(dead_code)]
    pub fn unmap_page(
        &mut self,
        page: Page<Size4KiB>,
        _pmm: &mut PhysicalMemoryManager,
    ) -> Result<PhysFrame<Size4KiB>, x86_64::structures::paging::mapper::UnmapError> {
        let (frame, flush) = self.page_table.unmap(page)?;
        flush.flush();
        Ok(frame)
    }

    #[allow(dead_code)]
    pub fn translate(&self, addr: VirtAddr) -> Option<PhysAddr> {
        self.page_table.translate_addr(addr)
    }
}

struct PmmFrameAllocator<'a> {
    pmm: &'a mut PhysicalMemoryManager,
}

unsafe impl<'a> FrameAllocator<Size4KiB> for PmmFrameAllocator<'a> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.pmm.alloc_frame().map(|addr| {
            // SAFETY: PMM guarantees the address is aligned and within valid range.
            unsafe { PhysFrame::from_start_address_unchecked(addr) }
        })
    }
}
