//! Opt-in legacy SHM prediction regression. Not restricted-grant isolation.
use super::accounting::{class_frame_count, PhysicalMemoryClass};
use super::pmm::PhysicalMemoryManager;
use super::shared::{alloc_shared_region, cleanup_shared_pages, map_shared_page, SharedMemError};
use crate::capability::{CapabilityBroker, CapabilityToken};
use crate::process::Process;
use x86_64::VirtAddr;

pub fn run(pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr) {
    for cycle in 0..32 {
        let baseline = pmm.free_page_count();
        let data_before = class_frame_count(PhysicalMemoryClass::SharedMemory);
        let tables_before = class_frame_count(PhysicalMemoryClass::PageTable);
        let mut owner = unsafe { Process::new(0xF110, 0, "f11-owner", pmm, hhdm) };
        let mut attacker = unsafe { Process::new(0xF111, 0, "f11-attacker", pmm, hhdm) };
        assert_ne!(
            owner.address_space.identity(),
            attacker.address_space.identity()
        );
        let mut caps = CapabilityBroker::new();
        let (_, private_token) =
            alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, 4096).unwrap();
        let (_, attacker_token) =
            alloc_shared_region(&mut attacker, pmm, &mut caps, hhdm, 4096).unwrap();
        for bits in 1..=62 {
            let guess = CapabilityToken(attacker_token.as_u64() ^ ((1u64 << bits) - 1));
            let free = pmm.free_page_count();
            assert_eq!(
                map_shared_page(&mut attacker, guess, pmm, &mut caps, hhdm),
                Err(SharedMemError::InvalidToken)
            );
            assert_eq!(pmm.free_page_count(), free);
            assert_eq!(attacker.mapped_shared.len(), 1);
        }
        // Explicit delegation still works for a legacy bearer. This is not
        // recipient binding and must not be described as a restricted grant.
        map_shared_page(&mut attacker, private_token, pmm, &mut caps, hhdm).unwrap();
        assert_eq!(caps.shared_region_map_count(private_token), Some(2));
        assert!(caps.revoke_shared(private_token).is_none());
        assert_eq!(caps.shared_region_map_count(private_token), Some(2));
        cleanup_shared_pages(&mut owner, pmm, &mut caps);
        assert_eq!(caps.shared_region_map_count(private_token), Some(1));
        cleanup_shared_pages(&mut attacker, pmm, &mut caps);
        assert_eq!(caps.shared_region_count(), 0);
        assert!(owner.mapped_shared.is_empty() && attacker.mapped_shared.is_empty());
        unsafe {
            owner.address_space.reclaim_user_space(pmm, hhdm, true);
            attacker.address_space.reclaim_user_space(pmm, hhdm, true);
        }
        assert_eq!(pmm.free_page_count(), baseline);
        assert_eq!(
            class_frame_count(PhysicalMemoryClass::SharedMemory),
            data_before
        );
        assert_eq!(
            class_frame_count(PhysicalMemoryClass::PageTable),
            tables_before
        );
        crate::serial_println!("[CAP-P] shm-cycle={} free_before={} free_after={} data_before={} data_after={} pt_before={} pt_after={} views=0 objects=0", cycle, baseline, pmm.free_page_count(), data_before, class_frame_count(PhysicalMemoryClass::SharedMemory), tables_before, class_frame_count(PhysicalMemoryClass::PageTable));
    }
    crate::serial_println!("[CAP-P] SHM prediction regression: 1984 denied; 32 delegated legacy maps; zero frame drift");
}
