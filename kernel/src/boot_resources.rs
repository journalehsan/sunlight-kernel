use crate::memory::pmm::PhysicalMemoryManager;
use crate::process::{
    address_space::AddressSpace,
    region::{MappingKind, MappingRegion, RegionBacking, RegionPolicy, RegionProtection},
    Process,
};
use limine::memmap::{Entry, MEMMAP_BOOTLOADER_RECLAIMABLE, MEMMAP_EXECUTABLE_AND_MODULES};
use sunlight_fs::boot_resources::{archive_size, BLOCK, USER_ADDRESS};
use x86_64::{
    structures::paging::{Page, PhysFrame},
    PhysAddr, VirtAddr,
};

#[used]
static MODULES: limine::request::ModulesRequest = limine::request::ModulesRequest::new();

pub struct Module {
    pub bytes: &'static [u8],
    phys: u64,
}

impl Module {
    pub fn load(map: &[&Entry], hhdm: VirtAddr) -> Self {
        let [file] = MODULES.response().expect("boot module response").modules() else {
            panic!("expected one boot resource module");
        };
        let bytes = file.data();
        assert_eq!(archive_size(bytes), Ok(bytes.len()), "boot module size");
        let phys = (bytes.as_ptr() as u64)
            .checked_sub(hhdm.as_u64())
            .expect("module HHDM");
        assert_eq!(phys % 4096, 0);
        let end = phys
            .checked_add(bytes.len().next_multiple_of(4096) as u64)
            .unwrap();
        assert!(map
            .iter()
            .any(|entry| entry.type_ == MEMMAP_EXECUTABLE_AND_MODULES
                && entry.base <= phys
                && end <= entry.base + entry.length));
        crate::memory::accounting::set_retained_boot_image_bytes(end - phys);
        crate::serial_println!("[BOOT-RES] module_phys={:#x} module_bytes={} module_pages={} scratch_bytes_per_read={}",
            phys, bytes.len(), (end - phys) / 4096, BLOCK);
        Self { bytes, phys }
    }

    pub fn map_vfs(&self, vfs: &mut Process, pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr) {
        let size = self.bytes.len().next_multiple_of(4096) as u64;
        let protection = RegionProtection::READ_ONLY;
        let flags = AddressSpace::protection_to_pte_flags(protection).expect("resource protection");
        let region = MappingRegion::new(
            USER_ADDRESS,
            USER_ADDRESS + size,
            protection,
            MappingKind::BootSharedData,
            RegionPolicy::SYSTEM.union(RegionPolicy::OWNER_MANAGED),
            RegionBacking::Internal(2),
        )
        .expect("boot resource region");
        let reservation = vfs
            .address_space
            .preflight_region(region)
            .expect("boot resource ledger");
        for offset in (0..size).step_by(4096) {
            let page = Page::from_start_address(VirtAddr::new(USER_ADDRESS + offset)).unwrap();
            let frame = PhysFrame::from_start_address(PhysAddr::new(self.phys + offset)).unwrap();
            unsafe {
                vfs.address_space
                    .map_page(page, frame, flags, pmm, hhdm)
                    .expect("boot resource map");
            }
        }
        vfs.address_space
            .commit_region(reservation)
            .expect("boot resource commit");
    }
}

pub fn initial_accounting(map: &[&Entry], managed: usize, free: usize) {
    let bytes_of = |kind| {
        map.iter()
            .filter(|entry| entry.type_ == kind)
            .map(|entry| entry.length)
            .sum::<u64>()
    };
    crate::serial_println!(
        "[BOOT-RES] managed_frames={} initial_free_frames={} usable_bytes={} loader_reclaimable_bytes={} image_and_module_bytes={}",
        managed, free, bytes_of(limine::memmap::MEMMAP_USABLE),
        bytes_of(MEMMAP_BOOTLOADER_RECLAIMABLE), bytes_of(MEMMAP_EXECUTABLE_AND_MODULES)
    );
}

pub fn checkpoint(stage: &str) {
    #[cfg(feature = "main_stack_census")]
    stack_census(stage);
    use core::sync::atomic::Ordering;
    use crate::memory::accounting::{class_frame_count, PhysicalMemoryClass};
    let (free, minimum, classes) = x86_64::instructions::interrupts::without_interrupts(|| {
        let pmm = crate::PMM.lock();
        let classes: [u64; 14] = core::array::from_fn(|index| {
            class_frame_count(PhysicalMemoryClass::from_u8(index as u8 + 2).unwrap())
        });
        (
            pmm.free_page_count(),
            crate::memory::pmm::BOOT_MIN_FREE.load(Ordering::Relaxed),
            classes,
        )
    });
    crate::serial_println!(
        "[BOOT-RES] stage={} minimum_free_frames={}",
        stage,
        minimum
    );
    let (zram, reclaim, heap) = x86_64::instructions::interrupts::without_interrupts(|| {
        (
            crate::memory::zram::aggregate_stats(),
            crate::memory::swap::diagnostics(),
            crate::memory::heap::heap_stats(),
        )
    });
    crate::serial_println!(
        "[BOOT-MEM] stage={} free={} classes_2_to_15={:?} heap_used={} swap_configured={} pools={} stored={} zbytes={} out_attempts={} out_ok={} in_ok={} activations={} scans={} reclaimed={} no_progress={} candidates={}",
        stage, free, classes, heap.allocated, zram.service_configured,
        zram.active_pool_count, zram.stored_pages, zram.compressed_bytes,
        zram.swap_out_attempts, zram.swap_out_successes, zram.swap_in_successes,
        reclaim.watermark_activations, reclaim.candidate_scans, reclaim.pages_reclaimed,
        reclaim.no_progress_events, reclaim.candidate_count
    );
}

#[cfg(feature = "main_stack_census")]
fn stack_census(stage: &str) {
    use crate::process::{layout::*, ProcessState};
    use x86_64::structures::paging::PageTableFlags as Flags;
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
    x86_64::instructions::interrupts::without_interrupts(|| {
        let sched = crate::sched::SCHEDULER.lock();
        let pmm = crate::PMM.lock();
        let mut leaders = 0;
        let mut total = 0;
        for process in &sched.processes {
            if !process.owns_address_space
                || matches!(process.state, ProcessState::Finished | ProcessState::Reaped)
                || !process.address_space.lookup_region(USER_STACK_TOP - 4096)
                    .is_some_and(|region| region.kind == MappingKind::UserStack)
            {
                continue;
            }
            let mut pages = 0;
            let mut depth = 0;
            for offset in (4096..=USER_STACK_SIZE).step_by(4096) {
                let page = Page::containing_address(VirtAddr::new(USER_STACK_TOP - offset));
                if let Some((frame, flags)) = unsafe { process.address_space.lookup_entry(page, hhdm) } {
                    if flags.contains(Flags::PRESENT) && pmm.owner_of(frame) == Some(process.pid as u32) {
                        pages += 1;
                        if flags.contains(Flags::ACCESSED) {
                            depth = offset / 4096;
                        }
                    }
                }
            }
            leaders += 1;
            total += pages;
            if process.name_str() == "sunlightd" {
                crate::serial_println!("[STACK-1] stage={} sunlightd_pages={} sunlightd_depth={}", stage, pages, depth);
            }
        }
        let (count, cycles, max) = crate::arch::x86_64::interrupts::main_stack_fault_cycles();
        crate::serial_println!("[STACK-1] stage={} leaders={} pages={} count={} cycles={} max={}",
            stage, leaders, total, count, cycles, max);
    });
}
