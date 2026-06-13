use crate::memory::pmm::PhysicalMemoryManager;
use x86_64::{
    structures::paging::{Page, PageTable, PageTableFlags, PhysFrame, Size4KiB},
    PhysAddr, VirtAddr,
};

pub struct AddressSpace {
    pub pml4_phys: PhysAddr,
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

        Self { pml4_phys }
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

    /// Switch to this address space (write PML4 phys addr to CR3).
    /// SAFETY: `pml4_phys` must be a valid, page-aligned physical address.
    pub unsafe fn activate(&self) {
        x86_64::registers::control::Cr3::write(
            PhysFrame::from_start_address_unchecked(self.pml4_phys),
            x86_64::registers::control::Cr3Flags::empty(),
        );
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
