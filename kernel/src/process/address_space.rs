use crate::memory::pmm::PhysicalMemoryManager;
use x86_64::{
    PhysAddr, VirtAddr,
    structures::paging::{
        Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
    },
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
            let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
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
