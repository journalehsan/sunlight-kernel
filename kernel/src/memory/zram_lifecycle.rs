//! Read-only accounting checkpoints for the native exit qualification.

use crate::memory::accounting::{class_frame_count, PhysicalMemoryClass};
use crate::memory::{swap, zram};
use crate::process::{ChildWait, Process, ProcessState};
use crate::sched::SCHEDULER;
use x86_64::{structures::paging::Page, VirtAddr};

pub fn checkpoint(stage: u64, value: u64) -> u64 {
    let sched = SCHEDULER.lock();
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
        1 | 2 if current.ppid == 1 && current.trusted_zram_diagnostic => {
            let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
            for index in 0..16 {
                let page = Page::from_start_address(VirtAddr::new(value + index * 4096)).unwrap();
                let vm = &current.address_space;
                unsafe {
                    if stage == 2 && index < 12 {
                        let slot = vm
                            .swapped_block_id(page, hhdm)
                            .expect("populated swapped PTE");
                        assert!(zram::block_exists(slot));
                    } else {
                        assert!(vm.lookup_phys(page, hhdm).is_some());
                    }
                }
            }
            (
                if stage == 1 { "resident" } else { "populated" },
                Some(current),
            )
        }
        _ => return u64::MAX,
    };
    snapshot(phase, process, &sched);
    1
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
    let (candidates, owned_candidates) = swap::lifecycle_candidates(pid);
    match phase {
        "resident" => assert_eq!(owned_candidates, 16),
        "populated" => assert_eq!(owned_candidates, 4),
        "settled" => assert_eq!(
            (owned, regions, descriptors, owned_candidates),
            (0, 0, 0, 0)
        ),
        _ => {}
    }
    crate::serial_println!(
        "[ZRAM-LIFE] phase={} pid={} cpus={} pools={} frames={} user={} pt={} compressed_frames={} slots={} compressed={} backing={} candidates={} owned_candidates={} swapins={} swapouts={} owned={} regions={} fds={} ipc_bytes={} stack={} reclaimed={} rows={} row_capacity={} heap={}",
        phase, pid, sched.online_cores, stats.active_pool_count, pmm.free_page_count(),
        class_frame_count(PhysicalMemoryClass::UserPrivate),
        class_frame_count(PhysicalMemoryClass::PageTable),
        class_frame_count(PhysicalMemoryClass::CompressedMemory),
        stats.stored_pages, stats.compressed_bytes, stats.allocator_consumed_bytes,
        candidates, owned_candidates, stats.swap_in_attempts, stats.swap_out_successes,
        owned, regions, descriptors, ipc_bytes, u8::from(stack), u8::from(reclaimed),
        sched.processes.len(), sched.processes.capacity(),
        crate::memory::heap::heap_stats().allocated,
    );
    zram::log_lifecycle_pools(phase, pid);
}

fn buffer_bytes<T>(queue: &alloc::collections::VecDeque<T>) -> usize {
    queue.capacity() * core::mem::size_of::<T>()
}
