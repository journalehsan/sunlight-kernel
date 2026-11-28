//! Local-only unpublished child construction. The scheduler serializes PID
//! claims and ownership decisions. VM operations use PMM outside that guard.
use super::address_space::MappingError;
use super::pending_state::{Error, Identity, Parent, Ticket};
use super::{fd_table::FileDescriptor, spawn, thread_admission, Process};
use crate::sched::{Scheduler, SCHEDULER};
use x86_64::{instructions::interrupts::without_interrupts, VirtAddr};

pub(crate) struct Image<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
    pub argv: &'a [&'a [u8]],
    pub envp: &'a [&'a [u8]],
    pub uid: u32,
    pub gid: u32,
    pub stdio: [Option<i32>; 3],
}

/// The creator keeps this owner until its local work ends. A copied identity
/// does not retain resources. Call and drop outside scheduler and PMM guards.
pub(crate) struct Child {
    identity: Identity,
    hhdm: VirtAddr,
}

impl Child {
    /// Adopt an adapter-built image before releasing its construction locks.
    /// The image still contains only its initial streams.
    #[cfg(feature = "vfs_c1a_candidate")]
    pub(crate) fn adopt(
        parent: Parent,
        mut process: Process,
        sched: &mut Scheduler,
        pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
        hhdm: VirtAddr,
    ) -> Result<Self, Error> {
        let reservation = (|| {
            let incarnation = process.foreground.incarnation;
            if incarnation == 0 || process.pid > 0x00ff_ffff {
                return Err(Error::Exhausted);
            }
            parent_index(sched, parent).ok_or(Error::ParentGone)?;
            let storage = thread_admission::try_box_uninit().map_err(|_| Error::NoMemory)?;
            let occupied = sched.process_index_by_pid(process.pid).is_some();
            let ticket = sched.pending_children.reserve(process.pid, parent, occupied)?;
            Ok((ticket, storage))
        })();
        let (ticket, mut storage) = match reservation {
            Ok(reservation) => reservation,
            Err(error) => {
                unsafe { spawn::rollback_initial_image(&mut process, pmm, hhdm) };
                return Err(error);
            }
        };
        let identity = Identity {
            ticket,
            incarnation: process.foreground.incarnation,
            image_generation: process.address_space.identity().generation,
        };
        storage.write(process);
        let process = unsafe { storage.assume_init() };
        assert!(sched.pending_children.finish(identity, process, |_| true).is_ok());
        Ok(Self { identity, hhdm })
    }


    #[cfg(feature = "vfs_c2e_candidate")]
    pub(crate) fn adopt_worker(
        parent: Parent,
        process: Process,
        sched: &mut Scheduler,
        hhdm: VirtAddr,
    ) -> Result<Self, Error> {
        assert!(!process.owns_address_space && process.foreground.incarnation != 0);
        let mut storage = thread_admission::try_box_uninit().map_err(|_| Error::NoMemory)?;
        let occupied = sched.process_index_by_pid(process.pid).is_some();
        let ticket = sched.pending_children.reserve(process.pid, parent, occupied)?;
        let identity = Identity {
            ticket,
            incarnation: process.foreground.incarnation,
            image_generation: process.address_space.identity().generation,
        };
        storage.write(process);
        assert!(sched.pending_children.finish(identity, unsafe { storage.assume_init() }, |_| true).is_ok());
        Ok(Self { identity, hhdm })
    }

    #[cfg(feature = "vfs_c2e_candidate")]
    pub(crate) fn rollback_unstaged_worker(self, sched: &mut Scheduler) {
        let identity = self.identity;
        assert!(sched.pending_children.remote(identity).is_none());
        let child = sched.pending_children.cancel(identity.ticket, Some(identity))
            .expect("unstaged worker owner").expect("constructed worker");
        assert!(!child.owns_address_space);
        drop(child);
        sched.pending_children.reclaimed(identity.ticket);
        self.disarm();
    }

    #[cfg(feature = "vfs_c1a_candidate")]
    pub(crate) fn rollback_unstaged(
        self,
        sched: &mut Scheduler,
        pmm: &mut crate::memory::pmm::PhysicalMemoryManager,
    ) {
        let identity = self.identity;
        assert!(sched.pending_children.remote(identity).is_none());
        let mut child = sched.pending_children.cancel(identity.ticket, Some(identity))
            .expect("unstaged child owner").expect("constructed child");
        unsafe { spawn::rollback_initial_image(&mut child, pmm, self.hhdm) };
        drop(child);
        sched.pending_children.reclaimed(identity.ticket);
        self.disarm();
    }

    pub(crate) fn private_identity(&self) -> Identity {
        self.identity
    }

    pub(crate) fn disarm(self) {
        core::mem::forget(self);
    }

    pub fn identity(&self) -> Result<Identity, Error> {
        without_interrupts(|| {
            let sched = SCHEDULER.lock();
            let identity = sched.pending_children.identity(self.identity.ticket)?;
            let parent = sched.pending_children.parent(self.identity.ticket)?;
            parent_index(&sched, parent).ok_or(Error::ParentGone)?;
            Ok(identity)
        })
    }

    pub fn cancel(&self) -> Result<(), Error> {
        cancel(self.identity, self.hhdm)
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

fn parent_index(sched: &Scheduler, parent: Parent) -> Option<usize> {
    sched.processes.iter().position(|process| {
        process.pid == parent.pid
            && parent.incarnation != 0
            && process.foreground.incarnation == parent.incarnation
            && process.address_space.identity().generation == parent.image_generation
            && process.process_lifetime_live()
    })
}

fn release(ticket: Ticket) {
    without_interrupts(|| SCHEDULER.lock().pending_children.reclaimed(ticket));
}

fn destroy(process: &mut Process, hhdm: VirtAddr) {
    assert!(process.mapped_shared.is_empty() && process.owned_shared.is_empty());
    assert!(process.ipc_endpoint.is_none());
    if !process.owns_address_space {
        // Pending workers have an unretained descriptor snapshot and borrow the VM.
        return;
    }
    assert!(process.account_membership.is_none());
    // Only stdio descriptions without backend leases can reach this path.
    unsafe {
        spawn::rollback_initial_image(process, &mut crate::PMM.lock(), hhdm);
    }
}

fn cancel(identity: Identity, hhdm: VirtAddr) -> Result<(), Error> {
    without_interrupts(|| {
        let process = {
            let mut sched = SCHEDULER.lock();
            let process = sched.pending_children.cancel(identity.ticket, Some(identity))?;
            if let Some(claim) = sched.pending_children.remote(identity) {
                crate::ipc::vfs_registration::spawn::cancel_child(claim);
            }
            process
        };
        let mut process = process.expect("cancellation requires a constructed identity");
        destroy(&mut process, hhdm);
        drop(process);
        release(identity.ticket);
        #[cfg(all(debug_assertions, feature = "vfs_l1c_r4_test"))]
        if crate::ipc::vfs_registration::spawn::fixture::tracks_child(identity.ticket.pid) {
            crate::serial_println!("[VFS-R4] dropped child={} process_box=1 fd_table_box=1", identity.ticket.pid);
        }
        Ok(())
    })
}

/// The caller authenticates the parent. This interface permits no external
/// service operations keyed by the child and requires initialized boot memory.
pub(crate) fn construct(parent: Parent, image: Image<'_>) -> Result<Child, Error> {
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().expect("boot direct map").offset);
    construct_inner(parent, image, hhdm, || Ok(()))
}

fn construct_inner(
    parent: Parent,
    image: Image<'_>,
    hhdm: VirtAddr,
    mut checkpoint: impl FnMut() -> Result<(), MappingError>,
) -> Result<Child, Error> {
    without_interrupts(|| {
        let ticket = {
            let mut sched = SCHEDULER.lock();
            parent_index(&sched, parent).ok_or(Error::ParentGone)?;
            let pid = sched
                .next_process_pid()
                .filter(|pid| *pid <= 0x00ff_ffff)
                .ok_or(Error::Exhausted)?;
            let occupied = sched.process_index_by_pid(pid).is_some();
            sched.pending_children.reserve(pid, parent, occupied)?
        };

        let built = (|| {
            checkpoint().map_err(|_| Error::NoMemory)?;
            let mut storage = thread_admission::try_box_uninit().map_err(|_| Error::NoMemory)?;
            let capabilities = {
                let sched = SCHEDULER.lock();
                let idx = parent_index(&sched, parent).ok_or(Error::ParentGone)?;
                checkpoint().map_err(|_| Error::NoMemory)?;
                thread_admission::try_copy(&sched.processes[idx].capabilities)
                    .map_err(|_| Error::NoMemory)?
            };
            {
                let mut pmm = crate::PMM.lock();
                let mut process = unsafe {
                    Process::try_new_with_checkpoint(
                        ticket.pid,
                        parent.pid,
                        image.name,
                        &mut pmm,
                        hhdm,
                        &mut checkpoint,
                    )
                }
                .map_err(|_| Error::NoMemory)?;
                process.foreground.parent_generation = parent.incarnation;
                process.uid = image.uid;
                process.gid = image.gid;
                process.capabilities = capabilities;
                let error = if process.foreground.incarnation == 0 {
                    Some(Error::Exhausted)
                } else {
                    spawn::load_pending_image(
                        image.bytes,
                        &mut process,
                        &mut pmm,
                        hhdm,
                        image.argv,
                        image.envp,
                        &mut || checkpoint().map_err(|_| spawn::SpawnError::NoMemory),
                    )
                    .err()
                    .map(|error| match error {
                        spawn::SpawnError::NoMemory => Error::NoMemory,
                        _ => Error::Image,
                    })
                };
                if let Some(error) = error {
                    unsafe { spawn::rollback_initial_image(&mut process, &mut pmm, hhdm) };
                    return Err(error);
                }
                storage.write(process);
            }
            Ok(unsafe { storage.assume_init() })
        })();
        let mut process = match built {
            Ok(process) => process,
            Err(error) => {
                release(ticket);
                return Err(error);
            }
        };
        let identity = Identity {
            ticket,
            incarnation: process.foreground.incarnation,
            image_generation: process.address_space.identity().generation,
        };

        let result = {
            let mut sched = SCHEDULER.lock();
            let parent = parent_index(&sched, parent).map(|idx| &sched.processes[idx]);
            let mut sources: [Option<FileDescriptor>; 3] = [None; 3];
            let mut supported = true;
            if let Some(parent) = parent {
                for (output, (source, fd)) in sources.iter_mut().zip(image.stdio).enumerate() {
                    if let Some(fd) = fd {
                        *source = parent
                            .fd_table
                            .get(fd)
                            .copied()
                            .filter(|entry| entry.handle.0 == output as u32);
                        supported &= source.is_some();
                    }
                }
            }
            let parent_live = parent.is_some();
            if supported {
                for (fd, source) in sources.into_iter().enumerate() {
                    if let Some(source) = source {
                        process.fd_table.inherit_initial_stream(fd as i32, source);
                    }
                }
                sched
                    .pending_children
                    .finish(identity, process, |_| parent_live)
            } else {
                Err((Error::UnsupportedDescriptor, process))
            }
        };
        match result {
            Ok(()) => Ok(Child { identity, hhdm }),
            Err((error, mut process)) => {
                destroy(&mut process, hhdm);
                drop(process);
                release(ticket);
                Err(error)
            }
        }
    })
}

#[cfg(all(debug_assertions, feature = "vfs_l1p_test"))]
pub(crate) fn native_fixture(hhdm: VirtAddr) {
    use super::fd_table::FdTable;
    use crate::memory::{
        accounting::{class_frame_count, PhysicalMemoryClass},
        heap, swap,
    };
    let (parent, cpus, published, ready) = {
        let mut sched = SCHEDULER.lock();
        let idx = sched.process_index_by_pid(1).expect("fixture parent");
        let init = &sched.processes[idx];
        let parent = Parent {
            pid: init.pid,
            incarnation: init.foreground.incarnation,
            image_generation: init.address_space.identity().generation,
        };
        let normal_count = sched.processes.len();
        let ready = sched.pending_test_ready_count();
        // Reserve all table entries without creating a process. Test collision
        // with a published PID and with the first privately claimed PID.
        let pid = sched.next_process_pid().unwrap();
        assert_eq!(
            sched.pending_children.reserve(parent.pid, parent, true),
            Err(Error::Collision)
        );
        let tickets: [_; 8] = core::array::from_fn(|offset| {
            sched
                .pending_children
                .reserve(pid + offset, parent, false)
                .unwrap()
        });
        assert_eq!(
            sched.pending_children.reserve(pid, parent, false),
            Err(Error::Collision)
        );
        assert_eq!(
            sched.pending_children.reserve(pid + 8, parent, false),
            Err(Error::Capacity)
        );
        for ticket in tickets {
            sched.pending_children.reclaimed(ticket);
        }
        (parent, sched.online_cores, normal_count, ready)
    };
    let image = || Image {
        name: "vfs-l1p-private",
        bytes: crate::TIMER_SERVER_ELF_BYTES,
        argv: &[b"vfs-l1p-private"],
        envp: &[b"VFS_L1P=1"],
        uid: 1000,
        gid: 1001,
        stdio: [Some(0), Some(1), None],
    };
    let counts = || {
        (
            crate::PMM.lock().free_page_count(),
            heap::heap_stats().allocated,
            FdTable::description_counts(),
            swap::candidate_count(),
            class_frame_count(PhysicalMemoryClass::UserPrivate),
            class_frame_count(PhysicalMemoryClass::PageTable),
        )
    };
    // Warm existing descriptor and image ledger pools before asserting exact
    // allocator accounting.
    drop(construct(parent, image()).expect("fixture warm-up"));
    let baseline = counts();
    let mut previous: Option<Identity> = None;
    for _ in 0..100 {
        let child = construct(parent, image()).expect("private child");
        let id = child.identity().unwrap();
        if let Some(old) = previous {
            assert_eq!(id.ticket.pid, old.ticket.pid);
            assert_ne!(id, old);
            assert_eq!(cancel(old, hhdm), Err(Error::Stale));
        }
        {
            let mut sched = SCHEDULER.lock();
            assert!(sched.process_index_by_pid(id.ticket.pid).is_none());
            assert!(sched.process_mut_by_pid(id.ticket.pid).is_none());
            assert_eq!(sched.processes.len(), published);
            assert!(sched.next_process_pid().unwrap() > id.ticket.pid);
            assert!((0..cpus).all(|cpu| sched.diagnostic_current_pid(cpu) != Some(id.ticket.pid)));
            assert_eq!(sched.pending_test_ready_count(), ready);
        }
        let during = counts();
        assert!(during.0 < baseline.0 && during.1 > baseline.1);
        assert_eq!(during.2, (baseline.2 .0 + 1, baseline.2 .1 + 3));
        assert_eq!(during.3, baseline.3);
        assert!(during.4 > baseline.4 && during.5 > baseline.5);
        child.cancel().unwrap();
        assert!(child.cancel().is_err());
        drop(child);
        assert_eq!(counts(), baseline);
        previous = Some(id);
    }
    let stale_parent = Parent {
        incarnation: 0,
        ..parent
    };
    assert!(matches!(
        construct(stale_parent, image()),
        Err(Error::ParentGone)
    ));
    for failure in 0..12 {
        let mut step = 0;
        let result = construct_inner(parent, image(), hhdm, || {
            let current = step;
            step += 1;
            if current == failure {
                Err(MappingError::FrameAllocationFailed)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(Error::NoMemory)));
        assert_eq!(counts(), baseline);
        assert_eq!(SCHEDULER.lock().pending_children.pids().count(), 0);
    }
    let mut invalid_image = image();
    invalid_image.bytes = b"not an ELF";
    assert!(matches!(
        construct(parent, invalid_image),
        Err(Error::Image)
    ));
    let mut unsupported = image();
    unsupported.stdio[1] = Some(0);
    assert!(matches!(
        construct(parent, unsupported),
        Err(Error::UnsupportedDescriptor)
    ));
    assert_eq!(counts(), baseline);

    // The VM ledger has fixed capacity. Test rollback at both the initial
    // empty root and the replacement image root, using real ledger slots.
    {
        use super::address_space::{AddressSpace, MAX_ADDRESS_SPACE_LEDGERS};
        let mut blockers = alloc::vec::Vec::with_capacity(MAX_ADDRESS_SPACE_LEDGERS);
        {
            let mut pmm = crate::PMM.lock();
            loop {
                match unsafe { AddressSpace::try_new(&mut pmm, hhdm) } {
                    Ok(space) => blockers.push(space),
                    Err(MappingError::LedgerCapacityExhausted) => break,
                    Err(error) => panic!("fixture ledger setup failed: {:?}", error),
                }
            }
        }
        for _ in 0..2 {
            let before = counts();
            assert!(matches!(construct(parent, image()), Err(Error::NoMemory)));
            assert_eq!(counts(), before);
            assert_eq!(SCHEDULER.lock().pending_children.pids().count(), 0);
            let mut released = blockers.pop().expect("fixture ledger blocker");
            unsafe { released.reclaim_user_space(&mut crate::PMM.lock(), hhdm, true) };
        }
        for mut space in blockers {
            unsafe { space.reclaim_user_space(&mut crate::PMM.lock(), hhdm, true) };
        }
    }
    assert_eq!(counts(), baseline);

    let max_pmm_failures = 1024;
    let mut pmm_failures = 0;
    for allowed in 0..=max_pmm_failures {
        crate::PMM.lock().fail_test_allocations_after(allowed);
        let result = construct(parent, image());
        crate::PMM.lock().clear_test_allocation_failure();
        match result {
            Ok(child) => {
                drop(child);
                assert_eq!(counts(), baseline);
                break;
            }
            Err(Error::NoMemory | Error::Image) => pmm_failures += 1,
            Err(other) => panic!("unexpected pending failure: {:?}", other),
        }
        assert_eq!(counts(), baseline);
        assert_eq!(SCHEDULER.lock().pending_children.pids().count(), 0);
    }
    assert!((2..=max_pmm_failures).contains(&pmm_failures));
    crate::serial_println!(
        "[VFS-L1P] PASS cpus={} cycles=100 heap_faults=12 ledger_faults=2 pmm_faults={} frames={} heap={} descriptions={} refs={} swap_candidates={} published={} ready={}",
        cpus, pmm_failures, baseline.0, baseline.1, baseline.2.0, baseline.2.1,
        baseline.3, published, ready,
    );
    crate::serial_println!(
        "[VFS-ACCT] private baseline user_frames={} page_tables={}",
        baseline.4, baseline.5,
    );
}
