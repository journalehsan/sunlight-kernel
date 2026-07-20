use super::pmm::PhysicalMemoryManager;
use x86_64::{
    structures::paging::{
        mapper::{FlagUpdateError, MapToError, MappedFrame, Mapper, TranslateResult},
        page::{Page, Size4KiB},
        FrameAllocator, OffsetPageTable, PageTable, PageTableFlags, PhysFrame, Translate,
    },
    PhysAddr, VirtAddr,
};

pub struct VirtualMemoryManager {
    page_table: OffsetPageTable<'static>,
}

#[derive(Clone, Copy)]
pub struct FramebufferCachePolicy {
    pub pte_flags: PageTableFlags,
    pub leaf_pat: bool,
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

    pub fn mapping_info(
        &self,
        page: Page<Size4KiB>,
    ) -> Option<(PhysFrame<Size4KiB>, PageTableFlags)> {
        match self.page_table.translate(page.start_address()) {
            TranslateResult::Mapped {
                frame: MappedFrame::Size4KiB(frame),
                offset: 0,
                flags,
            } => Some((frame, flags)),
            _ => None,
        }
    }

    pub unsafe fn framebuffer_cache_policy(
        &self,
        addr: VirtAddr,
        hhdm_offset: VirtAddr,
    ) -> Option<FramebufferCachePolicy> {
        let (root, _) = x86_64::registers::control::Cr3::read();
        let p4 = &*((hhdm_offset + root.start_address().as_u64()).as_ptr::<PageTable>());
        let p4_entry = &p4[addr.p4_index()];
        if !p4_entry.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }

        let p3 = &*((hhdm_offset + p4_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p3_entry = &p3[addr.p3_index()];
        if !p3_entry.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        if p3_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Some(framebuffer_cache_policy_from_entry(p3_entry, true));
        }

        let p2 = &*((hhdm_offset + p3_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p2_entry = &p2[addr.p2_index()];
        if !p2_entry.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        if p2_entry.flags().contains(PageTableFlags::HUGE_PAGE) {
            return Some(framebuffer_cache_policy_from_entry(p2_entry, true));
        }

        let p1 = &*((hhdm_offset + p2_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p1_entry = &p1[addr.p1_index()];
        if !p1_entry.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        Some(framebuffer_cache_policy_from_entry(p1_entry, false))
    }

    pub fn update_flags(
        &mut self,
        page: Page<Size4KiB>,
        flags: PageTableFlags,
    ) -> Result<(), FlagUpdateError> {
        let flush = unsafe { self.page_table.update_flags(page, flags) }?;
        flush.flush();
        Ok(())
    }
}

fn framebuffer_cache_policy_from_entry(
    entry: &x86_64::structures::paging::page_table::PageTableEntry,
    huge: bool,
) -> FramebufferCachePolicy {
    let flags = entry.flags();
    let leaf_pat = if huge {
        entry.addr().as_u64() & (1 << 12) != 0
    } else {
        flags.contains(PageTableFlags::HUGE_PAGE)
    };
    FramebufferCachePolicy {
        pte_flags: flags & (PageTableFlags::NO_CACHE | PageTableFlags::WRITE_THROUGH),
        leaf_pat,
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
