//! Checkpoints and scoped reclaim for the native ZRAM qualifications.

use crate::memory::accounting::{class_frame_count, PhysicalMemoryClass};
use crate::memory::{swap, zram};
use crate::process::{ChildWait, Process, ProcessState};
use crate::sched::SCHEDULER;
use x86_64::{
    structures::paging::{Page, PageTableFlags as Flags},
    VirtAddr,
};

pub fn checkpoint(stage: u64, value: u64) -> u64 {
    #[cfg(feature = "zram_pressure_test")]
    if (7..=18).contains(&stage) {
        return pressure_checkpoint(stage, value);
    }
    #[cfg(feature = "zram_fault_test")]
    let mut sched = SCHEDULER.lock();
    #[cfg(not(feature = "zram_fault_test"))]
    let sched = SCHEDULER.lock();
    #[cfg(feature = "zram_fault_test")]
    if stage == 6 {
        let current = sched.current_process();
        if current.ppid != 1 || !current.trusted_zram_diagnostic {
            return u64::MAX;
        }
        let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
        let mut pmm = crate::PMM.lock();
        let pid = current.pid;
        let before = swap::diagnostics();
        let deferred = swap::reclaim_lifecycle_range(value, &mut sched, hhdm, &mut pmm);
        let reclaimed = swap::reclaim_lifecycle_range(value, &mut sched, hhdm, &mut pmm);
        let after = swap::diagnostics();
        assert_eq!((deferred, reclaimed), (0, 12));
        assert_eq!(after.candidate_scans - before.candidate_scans, 24);
        assert_eq!(after.pages_reclaimed - before.pages_reclaimed, 12);
        crate::serial_println!(
            "[ZRAM-REPEAT] pid={} first_pass_evicted={} second_pass_evicted={} scans={}",
            pid,
            deferred,
            reclaimed,
            after.candidate_scans - before.candidate_scans,
        );
    }
    let current = sched.current_process();
    let (phase, process) = match stage {
        0 if current.pid == 1 => {
            let Some(child) = sched.processes.iter().find(|p| p.pid as u64 == value) else {
                return u64::MAX;
            };
            assert_eq!(child.ppid, current.pid);
            assert_eq!(child.child_wait, ChildWait::Consumed);
            if child.state != ProcessState::Reaped {
                assert_eq!(child.state, ProcessState::Finished);
                return 0;
            }
            assert!(child.address_space.is_reclaimed());
            assert!(child.kernel_stack.is_none());
            assert!(!child.exit_cleanup_pending);
            assert!(child.ipc_queue.is_empty() && child.deferred_reply_targets.is_empty());
            assert!(!crate::capability::CAP_BROKER
                .lock()
                .has_owned_endpoints(child.pid));
            if child.name_str() == "sunlight-swapd" {
                assert_eq!(child.exit_code, 0);
                assert!(!zram::aggregate_stats().admin_owner_alive);
                ("baseline", None)
            } else {
                assert!(child.trusted_zram_diagnostic);
                assert_eq!(child.exit_code, 37);
                ("settled", Some(child))
            }
        }
        1..=6
            if current.ppid == 1
                && current.trusted_zram_diagnostic
                && (stage <= 2 || cfg!(feature = "zram_fault_test")) =>
        {
            let phase = match stage {
                1 | 3 => "resident",
                2 | 4 | 6 => "populated",
                _ => "restored",
            };
            let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
            let pmm = crate::PMM.lock();
            for index in 0..16 {
                let page = Page::from_start_address(VirtAddr::new(value + index * 4096)).unwrap();
                let vm = &current.address_space;
                unsafe {
                    if phase == "populated" && index < 12 {
                        let slot = vm
                            .swapped_block_id(page, hhdm)
                            .expect("populated swapped PTE");
                        assert!(zram::block_exists(slot));
                        assert_eq!(
                            swap::lifecycle_page_candidates(
                                current.pid,
                                page.start_address(),
                                None
                            ),
                            0
                        );
                    } else {
                        let (frame, flags) = vm.lookup_entry(page, hhdm).expect("resident PTE");
                        assert_eq!(pmm.owner_of(frame), Some(current.pid as u32));
                        assert_eq!(
                            swap::lifecycle_page_candidates(
                                current.pid,
                                page.start_address(),
                                Some(frame)
                            ),
                            1,
                        );
                        if stage >= 3 {
                            let mask = Flags::PRESENT
                                | Flags::USER_ACCESSIBLE
                                | Flags::WRITABLE
                                | Flags::NO_EXECUTE;
                            let mut expected =
                                Flags::PRESENT | Flags::USER_ACCESSIBLE | Flags::NO_EXECUTE;
                            if index >= 6 {
                                expected |= Flags::WRITABLE;
                            }
                            assert_eq!(flags & mask, expected);
                            let region = vm.lookup_region(page.start_address().as_u64()).unwrap();
                            assert_eq!(
                                crate::process::address_space::AddressSpace::protection_from_pte_flags(flags)
                                    .unwrap(),
                                region.protection,
                            );
                        }
                    }
                }
            }
            if stage >= 3 {
                crate::serial_println!(
                    "[ZRAM-FAULT] phase={} pid={} base={:#x}",
                    phase,
                    current.pid,
                    value,
                );
            }
            (phase, Some(current))
        }
        _ => return u64::MAX,
    };
    snapshot(phase, process, &sched);
    1
}

#[cfg(feature = "zram_pressure_test")]
static PRESSURE_COLD: spin::Mutex<(usize, u64, u64, u64)> = spin::Mutex::new((0, 0, 0, 0));

#[cfg(feature = "zram_pressure_test")]
pub(super) fn pressure_page_deferred(pid: usize, address: u64) {
    let mut witness = PRESSURE_COLD.lock();
    let (owner, base, _, _) = *witness;
    if owner == pid && (base..base + 2 * 1024 * 1024).contains(&address) {
        witness.3 += 1;
        crate::serial_println!("[ZRAM-NATURAL-PAGE] defer pid={} va={:#x}", pid, address);
    }
}

#[cfg(feature = "zram_pressure_test")]
pub(super) fn pressure_page_event(
    pid: usize,
    address: u64,
    restored: Option<Flags>,
) {
    let mut witness = PRESSURE_COLD.lock();
    let (owner, base, _, _) = *witness;
    if owner != pid || !(base..base + 2 * 1024 * 1024).contains(&address) {
        return;
    }
    if let Some(flags) = restored {
        let required = Flags::PRESENT | Flags::USER_ACCESSIBLE | Flags::NO_EXECUTE;
        assert!(flags.contains(required));
        assert_eq!(flags.contains(Flags::WRITABLE), address >= base + 1024 * 1024);
        crate::serial_println!("[ZRAM-NATURAL-PAGE] restore pid={} va={:#x} writable={}", pid, address, u8::from(flags.contains(Flags::WRITABLE)));
    } else {
        witness.2 += 1;
        crate::serial_println!("[ZRAM-NATURAL-PAGE] evict pid={} va={:#x}", pid, address);
    }
}

#[cfg(feature = "zram_pressure_test")]
fn pressure_checkpoint(stage: u64, value: u64) -> u64 {
    use core::sync::atomic::{AtomicBool, Ordering};
    static READY: AtomicBool = AtomicBool::new(false);
    static DONE: AtomicBool = AtomicBool::new(false);
    let sched = SCHEDULER.lock();
    let current = sched.current_process();
    let child = current.ppid == 1 && current.trusted_zram_diagnostic;
    if stage == 10 && current.pid == 1 {
        return u64::from(READY.load(Ordering::Relaxed));
    }
    if stage == 11 && child {
        READY.store(true, Ordering::Relaxed);
        return u64::from(DONE.load(Ordering::Relaxed));
    }
    if stage == 14 && child {
        let (total, free) = crate::PMM.lock().stats();
        return free.saturating_sub(swap::watermarks(total).0) as u64;
    }
    if stage == 15 && child && value.checked_add(2 * 1024 * 1024).is_some() {
        let mut witness = PRESSURE_COLD.lock();
        if witness.0 != 0 {
            return 0;
        }
        *witness = (current.pid, value, 0, 0);
        return 1;
    }
    if stage == 16 && child {
        let (owner, _, evicted, deferred) = *PRESSURE_COLD.lock();
        return if owner != current.pid {
            u64::MAX
        } else if value == 1 {
            deferred
        } else {
            evicted
        };
    }
    let (phase, process) = match stage {
        7 if child => ("start", current),
        8 if child => ("sample", current),
        9 if current.pid == 1 => {
            let Some(worker) = sched.processes.iter().find(|p| p.pid as u64 == value) else {
                return u64::MAX;
            };
            if worker.state != ProcessState::Reaped {
                return 0;
            }
            assert_eq!(worker.child_wait, ChildWait::Consumed);
            assert!(worker.address_space.is_reclaimed() && worker.kernel_stack.is_none());
            assert!(!worker.exit_cleanup_pending);
            DONE.store(true, Ordering::Relaxed);
            ("reaped", worker)
        }
        12 if child => ("verified", current),
        13 if child => ("initialized", current),
        17 if child => ("held_for_exit", current),
        18 if child => ("turnover", current),
        _ => return u64::MAX,
    };
    let pmm = crate::PMM.lock();
    let (total, free) = pmm.stats();
    let (start, target) = swap::watermarks(total);
    let z = zram::aggregate_stats();
    let d = swap::diagnostics();
    let (candidates, owned_candidates, capacity) = swap::lifecycle_candidates(process.pid);
    assert!(capacity >= candidates + z.stored_pages as usize);
    let owned = pmm.owned_frame_count(process.pid as u32);
    if stage == 9 {
        assert_eq!((owned, owned_candidates, process.address_space.region_count()), (0, 0, 0));
    }
    crate::serial_println!(
        "[ZRAM-PRESSURE] phase={} pid={} value={} cpus={} pools={} total={} free={} start={} target={} user={} pt={} owned={} candidates={} owned_candidates={} capacity={} heap={} slots={} payload={} backing={} activations={} scans={} reclaimed={} no_progress={} out={} in_attempts={} in_success={} in_failed={}",
        phase, process.pid, value, sched.online_cores, z.active_pool_count, total, free, start, target,
        class_frame_count(PhysicalMemoryClass::UserPrivate),
        class_frame_count(PhysicalMemoryClass::PageTable), owned, candidates, owned_candidates,
        capacity, crate::memory::heap::heap_stats().allocated, z.stored_pages, z.compressed_bytes,
        z.allocator_consumed_bytes, d.watermark_activations, d.candidate_scans, d.pages_reclaimed,
        d.no_progress_events, z.swap_out_successes, z.swap_in_attempts, z.swap_in_successes,
        z.swap_in_failures,
    );
    zram::log_lifecycle_pools(phase, process.pid);
    if stage == 7 {
        (free.saturating_sub(start).saturating_add(1024).min(384 * 1024)) as u64
    } else {
        1
    }
}

fn snapshot(phase: &str, process: Option<&Process>, sched: &crate::sched::Scheduler) {
    let pmm = crate::PMM.lock();
    let stats = zram::aggregate_stats();
    let (pid, owned, regions, descriptors, ipc_bytes, stack, reclaimed) =
        process.map_or((0, 0, 0, 0, 0, false, false), |p| {
            (
                p.pid,
                pmm.owned_frame_count(p.pid as u32),
                p.address_space.region_count(),
                (0..256).filter(|&fd| p.fd_table.get(fd).is_some()).count(),
                buffer_bytes(&p.ipc_queue) + buffer_bytes(&p.deferred_reply_targets),
                p.kernel_stack.is_some(),
                p.address_space.is_reclaimed(),
            )
        });
    let (candidates, owned_candidates, candidate_capacity) = swap::lifecycle_candidates(pid);
    assert!(candidate_capacity >= candidates + stats.stored_pages as usize);
    match phase {
        "resident" | "restored" => assert_eq!(owned_candidates, 16),
        "populated" => assert_eq!(owned_candidates, 4),
        "settled" => assert_eq!(
            (owned, regions, descriptors, owned_candidates),
            (0, 0, 0, 0)
        ),
        _ => {}
    }
    crate::serial_println!(
        "[ZRAM-LIFE] phase={} pid={} cpus={} pools={} frames={} user={} pt={} compressed_frames={} slots={} compressed={} backing={} candidates={} owned_candidates={} candidate_capacity={} swapins={} swapouts={} owned={} regions={} fds={} ipc_bytes={} stack={} reclaimed={} rows={} row_capacity={} heap={}",
        phase, pid, sched.online_cores, stats.active_pool_count, pmm.free_page_count(),
        class_frame_count(PhysicalMemoryClass::UserPrivate),
        class_frame_count(PhysicalMemoryClass::PageTable),
        class_frame_count(PhysicalMemoryClass::CompressedMemory),
        stats.stored_pages, stats.compressed_bytes, stats.allocator_consumed_bytes,
        candidates, owned_candidates, candidate_capacity, stats.swap_in_attempts, stats.swap_out_successes,
        owned, regions, descriptors, ipc_bytes, u8::from(stack), u8::from(reclaimed),
        sched.processes.len(), sched.processes.capacity(),
        crate::memory::heap::heap_stats().allocated,
    );
    zram::log_lifecycle_pools(phase, pid);
}

fn buffer_bytes<T>(queue: &alloc::collections::VecDeque<T>) -> usize {
    queue.capacity() * core::mem::size_of::<T>()
}
