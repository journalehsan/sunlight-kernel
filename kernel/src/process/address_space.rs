use crate::memory::pmm::PhysicalMemoryManager;
use x86_64::{
    structures::paging::{Page, PageTable, PageTableFlags, PhysFrame, Size4KiB},
    PhysAddr, VirtAddr,
};

pub struct AddressSpace {
    pub pml4_phys: PhysAddr,
    shared_bump: u64,
}

impl AddressSpace {
    /// Create a new address space, copying kernel higher-half mappings.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn new(pmm: &mut PhysicalMemoryManager, hhdm_offset: VirtAddr) -> Self {
        let pml4_phys = pmm.alloc_frame().expect("PML4 allocation failed");

        // Map the PML4 via HHDM to initialize it.
        let pml4_virt = hhdm_offset + pml4_phys.as_u64();
        let pml4 = &mut *(pml4_virt.as_mut_ptr::<PageTable>());

        // Zero the PML4.
        for entry in pml4.iter_mut() {
            entry.set_unused();
        }

        // Copy kernel higher-half mappings (indices 256..512).
        let current_pml4 = &*get_current_pml4(hhdm_offset);
        for i in 256..512 {
            pml4[i].set_addr(current_pml4[i].addr(), current_pml4[i].flags());
        }

        Self {
            pml4_phys,
            shared_bump: 0x0000_0003_0000_0000,
        }
    }

    /// Construct an AddressSpace wrapper for an already-allocated PML4 (used by fork CoW clone).
    /// Shared bump region is reset for the child (no inherited grants).
    pub fn from_pml4(pml4_phys: PhysAddr) -> Self {
        Self {
            pml4_phys,
            shared_bump: 0x0000_0003_0000_0000,
        }
    }

    /// Map a page in this address space.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        phys: PhysFrame<Size4KiB>,
        flags: PageTableFlags,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) {
        let pml4 = &mut *((hhdm_offset + self.pml4_phys.as_u64()).as_mut_ptr::<PageTable>());

        let p4_entry = &mut pml4[page.p4_index()];
        let p3_table = Self::create_next_table(p4_entry, pmm, hhdm_offset);

        let p3_entry = &mut p3_table[page.p3_index()];
        let p2_table = Self::create_next_table(p3_entry, pmm, hhdm_offset);

        let p2_entry = &mut p2_table[page.p2_index()];
        let p1_table = Self::create_next_table(p2_entry, pmm, hhdm_offset);

        let p1_entry = &mut p1_table[page.p1_index()];
        p1_entry.set_frame(phys, flags);
    }

    /// Look up the physical address mapped for `page`, if any.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn lookup_phys(
        &self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<PhysAddr> {
        unsafe { self.lookup_entry(page, hhdm_offset).map(|(phys, _)| phys) }
    }

    /// Look up the physical address and flags mapped for `page`, if any.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn lookup_entry(
        &self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<(PhysAddr, PageTableFlags)> {
        let pml4 = &*((hhdm_offset + self.pml4_phys.as_u64()).as_ptr::<PageTable>());
        let p4_entry = &pml4[page.p4_index()];
        if p4_entry.is_unused() {
            return None;
        }
        let p3_table = &*((hhdm_offset + p4_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p3_entry = &p3_table[page.p3_index()];
        if p3_entry.is_unused() {
            return None;
        }
        let p2_table = &*((hhdm_offset + p3_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p2_entry = &p2_table[page.p2_index()];
        if p2_entry.is_unused() {
            return None;
        }
        let p1_table = &*((hhdm_offset + p2_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p1_entry = &p1_table[page.p1_index()];
        if p1_entry.is_unused() {
            return None;
        }
        Some((p1_entry.addr(), p1_entry.flags()))
    }

    /// Replace the flags of an already-mapped page, keeping its frame.
    /// Returns false if the page is not mapped.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn update_flags(
        &mut self,
        page: Page<Size4KiB>,
        flags: PageTableFlags,
        hhdm_offset: VirtAddr,
    ) -> bool {
        let pml4 = &mut *((hhdm_offset + self.pml4_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p4_entry = &mut pml4[page.p4_index()];
        if p4_entry.is_unused() {
            return false;
        }
        let p3_table = &mut *((hhdm_offset + p4_entry.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p3_entry = &mut p3_table[page.p3_index()];
        if p3_entry.is_unused() {
            return false;
        }
        let p2_table = &mut *((hhdm_offset + p3_entry.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p2_entry = &mut p2_table[page.p2_index()];
        if p2_entry.is_unused() {
            return false;
        }
        let p1_table = &mut *((hhdm_offset + p2_entry.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p1_entry = &mut p1_table[page.p1_index()];
        if p1_entry.is_unused() {
            return false;
        }
        let frame = p1_entry.addr();
        p1_entry.set_addr(frame, flags);
        true
    }

    /// Walk to the leaf (P1) entry for `page`, returning a raw pointer to it
    /// if every higher-level table is already present. Works for both
    /// present and not-present (e.g. swapped) leaf entries.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    unsafe fn p1_entry_ptr(
        &self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<*mut x86_64::structures::paging::page_table::PageTableEntry> {
        let pml4 = &*((hhdm_offset + self.pml4_phys.as_u64()).as_ptr::<PageTable>());
        let p4_entry = &pml4[page.p4_index()];
        if p4_entry.is_unused() {
            return None;
        }
        let p3_table = &*((hhdm_offset + p4_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p3_entry = &p3_table[page.p3_index()];
        if p3_entry.is_unused() {
            return None;
        }
        let p2_table = &*((hhdm_offset + p3_entry.addr().as_u64()).as_ptr::<PageTable>());
        let p2_entry = &p2_table[page.p2_index()];
        if p2_entry.is_unused() {
            return None;
        }
        let p1_table = &*((hhdm_offset + p2_entry.addr().as_u64()).as_ptr::<PageTable>());
        Some(&p1_table[page.p1_index()] as *const _ as *mut _)
    }

    /// Replace a present leaf entry with a swapped marker: PRESENT cleared,
    /// address field repurposed to hold `block_id` (shifted into the
    /// page-aligned address bits). Returns false if `page` isn't mapped.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn mark_swapped(
        &mut self,
        page: Page<Size4KiB>,
        block_id: u64,
        hhdm_offset: VirtAddr,
    ) -> bool {
        match self.p1_entry_ptr(page, hhdm_offset) {
            Some(ptr) => {
                (*ptr).set_addr(PhysAddr::new(block_id << 12), PageTableFlags::empty());
                true
            }
            None => false,
        }
    }

    /// If `page`'s leaf entry is a swapped marker (not present, non-zero
    /// address field), return the encoded ZRAM block id.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn swapped_block_id(&self, page: Page<Size4KiB>, hhdm_offset: VirtAddr) -> Option<u64> {
        let entry = &*self.p1_entry_ptr(page, hhdm_offset)?;
        if entry.is_unused() || entry.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        Some(entry.addr().as_u64() >> 12)
    }

    /// Re-map `page` to `frame` with `flags` (used to fault a swapped page
    /// back in). Returns false if `page`'s leaf entry doesn't exist.
    /// SAFETY: `hhdm_offset` must be the correct HHDM base.
    pub unsafe fn remap_present(
        &mut self,
        page: Page<Size4KiB>,
        frame: PhysFrame<Size4KiB>,
        flags: PageTableFlags,
        hhdm_offset: VirtAddr,
    ) -> bool {
        match self.p1_entry_ptr(page, hhdm_offset) {
            Some(ptr) => {
                (*ptr).set_addr(frame.start_address(), flags);
                true
            }
            None => false,
        }
    }

    /// Switch to this address space (write PML4 phys addr to CR3).
    /// SAFETY: `pml4_phys` must be a valid, page-aligned physical address.
    pub unsafe fn activate(&self) {
        x86_64::registers::control::Cr3::write(
            PhysFrame::from_start_address_unchecked(self.pml4_phys),
            x86_64::registers::control::Cr3Flags::empty(),
        );
    }

    /// Map a shared physical frame into the dedicated shared region (0x3_0000_0000_0000+).
    /// The caller is responsible for capability tracking and frame lifetime.
    /// SAFETY: hhdm_offset must be correct HHDM base; phys must be a valid allocated frame.
    pub unsafe fn map_shared_page(
        &mut self,
        phys: PhysAddr,
        pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> Result<VirtAddr, crate::memory::shared::SharedMemError> {
        const PAGE_SIZE: u64 = 4096;
        if self.shared_bump >= 0x0000_0004_0000_0000 {
            return Err(crate::memory::shared::SharedMemError::OutOfMemory);
        }
        let virt = VirtAddr::new(self.shared_bump);
        let page = Page::<Size4KiB>::from_start_address(virt)
            .map_err(|_| crate::memory::shared::SharedMemError::InvalidAddress)?;
        let frame = PhysFrame::<Size4KiB>::from_start_address(phys)
            .map_err(|_| crate::memory::shared::SharedMemError::InvalidAddress)?;
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE;
        self.map_page(page, frame, flags, pmm, hhdm_offset);
        self.shared_bump += PAGE_SIZE;
        Ok(virt)
    }

    /// Unmap a page previously mapped in this address space. Returns the phys addr if it was present.
    /// Does not free page tables or the frame itself.
    /// SAFETY: hhdm_offset correct.
    pub unsafe fn unmap_page(
        &mut self,
        page: Page<Size4KiB>,
        hhdm_offset: VirtAddr,
    ) -> Option<PhysAddr> {
        let pml4 = &mut *((hhdm_offset + self.pml4_phys.as_u64()).as_mut_ptr::<PageTable>());
        let p4e = &mut pml4[page.p4_index()];
        if p4e.is_unused() {
            return None;
        }
        let p3 = &mut *((hhdm_offset + p4e.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p3e = &mut p3[page.p3_index()];
        if p3e.is_unused() {
            return None;
        }
        let p2 = &mut *((hhdm_offset + p3e.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p2e = &mut p2[page.p2_index()];
        if p2e.is_unused() {
            return None;
        }
        let p1 = &mut *((hhdm_offset + p2e.addr().as_u64()).as_mut_ptr::<PageTable>());
        let p1e = &mut p1[page.p1_index()];
        if p1e.is_unused() || !p1e.flags().contains(PageTableFlags::PRESENT) {
            return None;
        }
        let phys = p1e.addr();
        p1e.set_unused();
        Some(phys)
    }

    /// Count the number of present user-space 4 KiB pages mapped in this
    /// address space (an RSS-like measure). Walks only the lower half of the
    /// PML4 (indices 0..256); the kernel higher half is shared and excluded.
    /// Huge pages are counted by the number of 4 KiB pages they span.
    ///
    /// SAFETY: `hhdm_offset` must be the correct HHDM base and the page tables
    /// must be quiescent (caller holds the scheduler lock).
    pub unsafe fn count_user_pages(&self, hhdm_offset: VirtAddr) -> usize {
        let mut total = 0usize;
        let pml4 = &*((hhdm_offset + self.pml4_phys.as_u64()).as_ptr::<PageTable>());
        for p4e in pml4.iter().take(256) {
            if p4e.is_unused() {
                continue;
            }
            let p3 = &*((hhdm_offset + p4e.addr().as_u64()).as_ptr::<PageTable>());
            for p3e in p3.iter() {
                if p3e.is_unused() {
                    continue;
                }
                // 1 GiB huge page
                if p3e.flags().contains(PageTableFlags::HUGE_PAGE) {
                    total += 512 * 512;
                    continue;
                }
                let p2 = &*((hhdm_offset + p3e.addr().as_u64()).as_ptr::<PageTable>());
                for p2e in p2.iter() {
                    if p2e.is_unused() {
                        continue;
                    }
                    // 2 MiB huge page
                    if p2e.flags().contains(PageTableFlags::HUGE_PAGE) {
                        total += 512;
                        continue;
                    }
                    let p1 = &*((hhdm_offset + p2e.addr().as_u64()).as_ptr::<PageTable>());
                    for p1e in p1.iter() {
                        if !p1e.is_unused() && p1e.flags().contains(PageTableFlags::PRESENT) {
                            total += 1;
                        }
                    }
                }
            }
        }
        total
    }

    /// Create or get the next-level page table for an entry.
    fn create_next_table(
        entry: &mut x86_64::structures::paging::page_table::PageTableEntry,
        pmm: &mut PhysicalMemoryManager,
        hhdm_offset: VirtAddr,
    ) -> &'static mut PageTable {
        if entry.is_unused() {
            let frame_addr = pmm.alloc_frame().expect("page table alloc failed");
            let flags = PageTableFlags::PRESENT
                | PageTableFlags::WRITABLE
                | PageTableFlags::USER_ACCESSIBLE;
            entry.set_addr(frame_addr, flags);
            let virt = hhdm_offset + frame_addr.as_u64();
            let table = unsafe { &mut *(virt.as_mut_ptr::<PageTable>()) };
            for e in table.iter_mut() {
                e.set_unused();
            }
            table
        } else {
            let phys = entry.addr();
            let virt = hhdm_offset + phys.as_u64();
            unsafe { &mut *(virt.as_mut_ptr::<PageTable>()) }
        }
    }
}

/// Get the currently active PML4 as a mutable pointer via HHDM.
/// SAFETY: `hhdm_offset` must be the correct HHDM base.
unsafe fn get_current_pml4(hhdm_offset: VirtAddr) -> *mut PageTable {
    let phys = x86_64::registers::control::Cr3::read().0.start_address();
    let virt = hhdm_offset + phys.as_u64();
    virt.as_mut_ptr::<PageTable>()
}
