//! Opt-in M3.1B bounded page-table retention fixture. Not a ring-3 FastLane
//! peer and not a production path. It drives the ordinary SHM allocation and
//! retirement entry points against a dedicated owner VM so the retention
//! mechanism (empty lower tables left linked by `free_shared_page`) can be
//! observed with per-frame identities rather than aggregate counts.
//!
//! No sleeps, no wall-clock waits and no artificial census thresholds are used.
//! Quiescence is established by the existing synchronous unmap/TLB completion
//! and by walking the owner's own lower-half table tree.
use super::accounting::{class_frame_count, PhysicalMemoryClass};
use super::pmm::PhysicalMemoryManager;
use super::shared::{alloc_shared_region, free_shared_page};
use crate::capability::CapabilityBroker;
use crate::process::Process;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::structures::paging::{PageTable, PageTableFlags};
use x86_64::VirtAddr;

/// One observed lower-half table: `(level, phys, parent_phys, occupied)`.
type TableRecord = (u64, u64, u64, usize);

static RUNNING: AtomicBool = AtomicBool::new(false);

/// True when this image is built with the debug-only pre-repair evidence switch,
/// which omits the SHM empty-table reclaim so retention is reproducible.
const PRE_REPAIR: bool = cfg!(all(
    debug_assertions,
    feature = "fastlane_m31_retention_test",
    feature = "fastlane_m31_prerepair"
));

fn emit(fields: &str) {
    crate::serial_println!("[FL-M31] {}", fields);
}

/// Walk the owner's lower-half page tables (PML4 indices 0..256) and return
/// every reachable table with its parent and current occupied-entry count.
/// SAFETY: `hhdm` correct; caller holds the scheduler-equivalent ownership.
unsafe fn walk_tables(owner: &Process, hhdm: VirtAddr) -> Vec<TableRecord> {
    let mut out: Vec<TableRecord> = Vec::new();
    let pml4_phys = owner.address_space.pml4_phys.as_u64();
    let pml4 = &*((hhdm + pml4_phys).as_ptr::<PageTable>());
    for p4_idx in 0..256 {
        let p4e = &pml4[p4_idx];
        if p4e.is_unused() || !p4e.flags().contains(PageTableFlags::PRESENT) {
            continue;
        }
        let p3_phys = p4e.addr().as_u64();
        let p3 = &*((hhdm + p3_phys).as_ptr::<PageTable>());
        let p3_occupied = p3.iter().filter(|e| !e.is_unused()).count();
        out.push((3, p3_phys, pml4_phys, p3_occupied));
        for p3e in p3.iter() {
            if p3e.is_unused()
                || !p3e.flags().contains(PageTableFlags::PRESENT)
                || p3e.flags().contains(PageTableFlags::HUGE_PAGE)
            {
                continue;
            }
            let p2_phys = p3e.addr().as_u64();
            let p2 = &*((hhdm + p2_phys).as_ptr::<PageTable>());
            let p2_occupied = p2.iter().filter(|e| !e.is_unused()).count();
            out.push((2, p2_phys, p3_phys, p2_occupied));
            for p2e in p2.iter() {
                if p2e.is_unused()
                    || !p2e.flags().contains(PageTableFlags::PRESENT)
                    || p2e.flags().contains(PageTableFlags::HUGE_PAGE)
                {
                    continue;
                }
                let p1_phys = p2e.addr().as_u64();
                let p1 = &*((hhdm + p1_phys).as_ptr::<PageTable>());
                let p1_occupied = p1.iter().filter(|e| !e.is_unused()).count();
                out.push((1, p1_phys, p2_phys, p1_occupied));
            }
        }
    }
    out
}

fn p1_count(tables: &[TableRecord], occupied: usize) -> usize {
    tables
        .iter()
        .filter(|(level, _, _, occ)| *level == 1 && *occ == occupied)
        .count()
}

fn table_ids(tables: &[TableRecord]) -> Vec<u64> {
    let mut ids: Vec<u64> = tables.iter().map(|(_, phys, _, _)| *phys).collect();
    ids.sort_unstable();
    ids
}

/// Run the controlled retention fixture. SAFETY: boot-time single owner of PMM.
pub fn run(pmm: &mut PhysicalMemoryManager, hhdm: VirtAddr) {
    assert!(
        !RUNNING.swap(true, Ordering::AcqRel),
        "m31 retention fixture re-entered"
    );

    let baseline_free = pmm.free_page_count();
    let baseline_pt = class_frame_count(PhysicalMemoryClass::PageTable);
    let baseline_shm = class_frame_count(PhysicalMemoryClass::SharedMemory);

    let mut caps = CapabilityBroker::new();
    let mut owner = unsafe { Process::new(0xF131, 0, "m31-owner", pmm, hhdm) };
    owner.system_allocation = true; // Explicit trusted kernel fixture policy.
    let identity = owner.address_space.identity();
    emit(&alloc::format!(
        "phase=setup owner_pid={} vm_generation={} root={} baseline_free={} baseline_pt={} baseline_shm={} pre_repair={}",
        owner.pid,
        identity.generation,
        identity.pml4_phys,
        baseline_free,
        baseline_pt,
        baseline_shm,
        u8::from(PRE_REPAIR)
    ));

    let initial = unsafe { walk_tables(&owner, hhdm) };
    emit(&alloc::format!(
        "phase=initial tables={} p1_empty={} ids={:?}",
        initial.len(),
        p1_count(&initial, 0),
        table_ids(&initial)
    ));

    // Each cycle allocates exactly one 2 MiB span of normal SHM and retires it
    // through the verified final-view path. Because the SHM bump is monotonic
    // and 2 MiB-aligned at the window base, every cycle lands on a fresh P1
    // window. Before the M3.1B repair, free_shared_page left the now-empty P1
    // table linked (retained); _retained_* arms below remain to prove the
    // repaired contract: the empty lower tables are reclaimed once their leaf
    // shootdown has completed and the VM's table set returns to its baseline.
    const SPAN_PAGES: usize = 512; // 2 MiB
    const CROSSINGS: usize = 8;
    let mut max_group_tables = 0usize;
    let mut reused_root: Option<u64> = None;
    for cycle in 0..CROSSINGS {
        let size = SPAN_PAGES * super::shared::PAGE_SIZE;
        let (base, token) = alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, size).unwrap();
        let frames = caps
            .resolve_shared_region(token)
            .expect("fresh SHM region resolves")
            .frames
            .clone();
        let leaf = frames[0].start_address().as_u64();
        // The mapped range must be inside the monotonic SHM window and start on
        // the 2 MiB boundary this cycle is meant to exercise.
        assert_eq!(
            base.as_u64() - super::shared::SHARED_REGION_BASE,
            (cycle as u64) * size as u64,
            "SHM bump did not advance by exactly one span"
        );
        assert_eq!(base.as_u64() % (2 * 1024 * 1024), 0);
        // While the 2 MiB span is live it must own exactly one populated P1
        // window plus the tables above it. This is the only table the fixture
        // should ever grow for a crossing.
        let live = unsafe { walk_tables(&owner, hhdm) };
        let live_p1 = p1_count(&live, SPAN_PAGES);
        assert_eq!(live_p1, 1, "live 2 MiB span did not own exactly one P1");
        max_group_tables = max_group_tables.max(live.len());
        let root = owner.address_space.identity().pml4_phys;
        reused_root.get_or_insert(root);
        assert_eq!(Some(root), reused_root, "owner root changed mid-run");
        free_shared_page(&mut owner, token, pmm, &mut caps, hhdm).unwrap();
        assert!(owner.mapped_shared.is_empty(), "view retained after free");
        // Backing frames must be physically released by the final-view path.
        for frame in &frames {
            assert_eq!(
                pmm.class_of(frame.start_address()),
                Some(PhysicalMemoryClass::Free),
                "retired SHM backing frame not physically free"
            );
        }
        let tables = unsafe { walk_tables(&owner, hhdm) };
        let retained_p1 = p1_count(&tables, 0);
        emit(&alloc::format!(
            "phase=crossing index={} base={:#x} leaf={} tables_live={} tables_after_free={} retained_empty_p1={} ids_after_free={:?}",
            cycle,
            base.as_u64(),
            leaf,
            live.len(),
            tables.len(),
            retained_p1,
            table_ids(&tables)
        ));
        if PRE_REPAIR {
            // Pre-repair evidence: each crossing leaves its freshly linked
            // empty P1 table parent-linked, so the empty-P1 count grows by one
            // per crossing while the shared P2/P3 are created once (3 tables on
            // the first crossing, then one P1 per crossing).
            assert_eq!(
                retained_p1,
                cycle + 1,
                "pre-repair crossing did not retain one more empty P1"
            );
            assert_eq!(
                tables.len(),
                initial.len() + 3 + cycle,
                "pre-repair table growth not one P1 chain per crossing"
            );
        } else {
            // Repaired contract: after the synchronous leaf shootdown the
            // covering empty lower tables are reclaimed, so nothing is
            // retained across a crossing and the set does not grow.
            assert_eq!(
                retained_p1, 0,
                "empty P1 retained after free; reclaim did not run"
            );
            assert_eq!(
                tables.len(),
                initial.len(),
                "page-table set grew across a crossing"
            );
        }
    }

    // Reuse witness: two allocations live at once inside one 2 MiB window share
    // exactly one P1 table, and the window is reclaimed only when both leaves
    // are retired. This is the intentional-cache benefit the retention policy
    // must preserve; the repair reclaims only once the table is actually empty.
    let (base_a, token_a) =
        alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, super::shared::PAGE_SIZE).unwrap();
    let (base_b, token_b) =
        alloc_shared_region(&mut owner, pmm, &mut caps, hhdm, super::shared::PAGE_SIZE).unwrap();
    let shared = unsafe { walk_tables(&owner, hhdm) };
    assert_eq!(
        base_b.as_u64(),
        base_a.as_u64() + super::shared::PAGE_SIZE as u64,
        "small allocations were not consecutive inside one span"
    );
    assert_eq!(
        (base_a.as_u64() >> 21),
        (base_b.as_u64() >> 21),
        "reuse witness spanned two P1 windows"
    );
    assert_eq!(
        p1_count(&shared, 2),
        1,
        "two live same-span allocations did not share one P1 table"
    );
    emit(&alloc::format!(
        "phase=reuse first={:#x} second={:#x} tables={} shared_p1=1",
        base_a.as_u64(),
        base_b.as_u64(),
        shared.len()
    ));
    // Retiring only the first view must retain the shared table for the second.
    free_shared_page(&mut owner, token_a, pmm, &mut caps, hhdm).unwrap();
    let after_first = unsafe { walk_tables(&owner, hhdm) };
    assert_eq!(
        p1_count(&after_first, 1),
        1,
        "shared P1 was reclaimed while a live view still used it"
    );
    free_shared_page(&mut owner, token_b, pmm, &mut caps, hhdm).unwrap();
    let after_second = unsafe { walk_tables(&owner, hhdm) };
    // Pre-repair the crossing tables stay linked, so the empty set is the
    // crossing count plus the now-empty reuse window; after repair it is empty.
    let expected_empty_after_reuse = if PRE_REPAIR { CROSSINGS + 1 } else { 0 };
    let expected_tables_after_reuse = if PRE_REPAIR {
        initial.len() + CROSSINGS + 3
    } else {
        initial.len()
    };
    assert_eq!(
        p1_count(&after_second, 0),
        expected_empty_after_reuse,
        "empty P1 retained after the last shared view was retired"
    );
    assert_eq!(
        after_second.len(),
        expected_tables_after_reuse,
        "page-table set did not return to baseline after shared retirement"
    );

    // Retention summary before teardown. The bound is analytic: the SHM window
    // is 4 GiB and each P1 table covers 2 MiB, so one live VM can retain at most
    // 2048 empty P1 tables from address-cursor growth before repair. The repair
    // removes that accumulation; the analytic bound is recorded for contrast.
    let retained = unsafe { walk_tables(&owner, hhdm) };
    let retained_p1 = p1_count(&retained, 0);
    let analytic_bound = (super::shared::SHARED_REGION_MAX_SIZE as u64) / (2 * 1024 * 1024);
    emit(&alloc::format!(
        "phase=retention crossings={} retained_empty_p1={} total_tables={} max_live_group_tables={} window_bytes={} pre_repair_analytic_p1_bound={}",
        CROSSINGS,
        retained_p1,
        retained.len(),
        max_group_tables,
        super::shared::SHARED_REGION_MAX_SIZE,
        analytic_bound
    ));
    assert_eq!(
        retained_p1, expected_empty_after_reuse,
        "retained empty P1 total did not match the expected free-path policy"
    );
    assert_eq!(retained.len(), expected_tables_after_reuse);
    assert_eq!(owner.mapped_shared.len(), 0);
    assert_eq!(owner.owned_shared.len(), 0);

    // The first owner's shared views are all retired. After repair its lower
    // table set is back to baseline (root only); in the pre-repair evidence
    // image the retained empty tables are still linked and must be released
    // here. Either way teardown must release exactly the reachable tables plus
    // the root and no user frames.
    assert_eq!(crate::memory::tlb::active_cpu_mask(identity), 0);
    let live_ids = table_ids(&unsafe { walk_tables(&owner, hhdm) });
    let stats = unsafe { owner.address_space.reclaim_user_space(pmm, hhdm, true) };
    emit(&alloc::format!(
        "phase=teardown owner_pid={} vm_generation={} reachable_lower_tables={} reclaim_user_frames={} reclaim_page_tables={}",
        owner.pid,
        identity.generation,
        live_ids.len(),
        stats.user_frames,
        stats.page_tables
    ));
    assert_eq!(
        live_ids.len(),
        expected_tables_after_reuse,
        "owner lower table set before teardown differed from the free-path policy"
    );
    assert_eq!(
        pmm.class_of(x86_64::PhysAddr::new(identity.pml4_phys)),
        Some(PhysicalMemoryClass::Free),
        "VM root was not reclaimed"
    );
    assert_eq!(
        stats.page_tables,
        live_ids.len() + 1,
        "owner teardown did not release exactly the reachable tables plus the root"
    );
    assert_eq!(stats.user_frames, 0, "unexpected user frames in owner VM");
    assert!(owner.address_space.is_reclaimed());

    // Phase C retention arm: build deliberately retained empty lower tables
    // through the ordinary VM map/unmap path (which, unlike the repaired SHM
    // free path, does not reclaim empty tables), then tear the VM down and
    // prove each retained table identity is physically released exactly once.
    let mut teardown_owner = unsafe { Process::new(0xF132, 0, "m31-teardown", pmm, hhdm) };
    teardown_owner.system_allocation = true; // Explicit trusted kernel fixture policy.
    let teardown_identity = teardown_owner.address_space.identity();
    let mut backing: Vec<u64> = Vec::new();
    const RETAIN_SPANS: usize = 4;
    for span in 0..RETAIN_SPANS {
        for index in 0..2usize {
            let va = super::shared::SHARED_REGION_BASE
                + (span as u64) * (2 * 1024 * 1024)
                + (index as u64) * super::shared::PAGE_SIZE as u64;
            let phys = pmm
                .alloc_frame_owned_class(
                    teardown_owner.pid as u32,
                    PhysicalMemoryClass::UserPrivate,
                )
                .expect("retention arm backing frame");
            backing.push(phys.as_u64());
            let page = x86_64::structures::paging::Page::<x86_64::structures::paging::Size4KiB>::from_start_address(
                VirtAddr::new(va),
            )
            .unwrap();
            let frame = x86_64::structures::paging::PhysFrame::from_start_address(phys).unwrap();
            let flags = x86_64::structures::paging::PageTableFlags::PRESENT
                | x86_64::structures::paging::PageTableFlags::WRITABLE
                | x86_64::structures::paging::PageTableFlags::USER_ACCESSIBLE
                | x86_64::structures::paging::PageTableFlags::NO_EXECUTE;
            unsafe {
                teardown_owner
                    .address_space
                    .map_page(page, frame, flags, pmm, hhdm)
                    .unwrap();
            }
            // unmap_page clears the leaf and completes the synchronous
            // invalidation but never reclaims the now-empty table chain.
            let removed = unsafe { teardown_owner.address_space.unmap_page(page, hhdm) };
            assert_eq!(removed, Some(phys));
            pmm.free_frame(phys);
        }
    }
    let retained = unsafe { walk_tables(&teardown_owner, hhdm) };
    let retained_empty = retained
        .iter()
        .filter(|(level, _, _, occ)| *level == 1 && *occ == 0)
        .count();
    let retained_ids = table_ids(&retained);
    emit(&alloc::format!(
        "phase=teardown-retained owner_pid={} vm_generation={} spans={} retained_empty_p1={} reachable_lower_tables={} ids={:?}",
        teardown_owner.pid,
        teardown_identity.generation,
        RETAIN_SPANS,
        retained_empty,
        retained_ids.len(),
        retained_ids
    ));
    assert_eq!(
        retained_empty, RETAIN_SPANS,
        "retention arm did not leave one empty P1 per crossed span"
    );
    let pre_teardown_ids = retained_ids.clone();
    let reachable_lower = pre_teardown_ids.len();
    let stats = unsafe {
        teardown_owner
            .address_space
            .reclaim_user_space(pmm, hhdm, true)
    };
    emit(&alloc::format!(
        "phase=teardown-retained-complete vm_generation={} released_tables={} reclaim_user_frames={} reclaim_page_tables={}",
        teardown_identity.generation,
        reachable_lower,
        stats.user_frames,
        stats.page_tables
    ));
    // Per-object release: every identity observed before teardown is now in the
    // Free class, and the teardown released exactly those tables plus the root.
    for phys in &pre_teardown_ids {
        assert_eq!(
            pmm.class_of(x86_64::PhysAddr::new(*phys)),
            Some(PhysicalMemoryClass::Free),
            "retained table was not released by teardown"
        );
    }
    assert_eq!(
        pmm.class_of(x86_64::PhysAddr::new(teardown_identity.pml4_phys)),
        Some(PhysicalMemoryClass::Free),
        "retention-arm VM root was not reclaimed"
    );
    assert_eq!(
        stats.page_tables,
        reachable_lower + 1,
        "teardown did not release each retained table plus the root exactly once"
    );
    assert_eq!(stats.user_frames, 0, "retention arm left user frames");
    // All retention-arm backing frames were freed by the arm itself, so the
    // final delta check below also covers them.
    for phys in &backing {
        assert_eq!(
            pmm.class_of(x86_64::PhysAddr::new(*phys)),
            Some(PhysicalMemoryClass::Free),
            "retention-arm backing frame not free"
        );
    }
    assert!(teardown_owner.address_space.is_reclaimed());

    let final_free = pmm.free_page_count();
    let final_pt = class_frame_count(PhysicalMemoryClass::PageTable);
    let final_shm = class_frame_count(PhysicalMemoryClass::SharedMemory);
    emit(&alloc::format!(
        "phase=final free={} pt={} shm={} free_delta={} pt_delta={} shm_delta={}",
        final_free,
        final_pt,
        final_shm,
        final_free as i64 - baseline_free as i64,
        final_pt as i64 - baseline_pt as i64,
        final_shm as i64 - baseline_shm as i64
    ));
    assert_eq!(final_free, baseline_free, "allocator drift after teardown");
    assert_eq!(
        final_pt, baseline_pt,
        "page-table class drift after teardown"
    );
    assert_eq!(
        final_shm, baseline_shm,
        "shared-memory class drift after teardown"
    );

    emit("phase=complete all_owners_reclaimed=true");
}
