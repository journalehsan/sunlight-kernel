//! Isolated service-file continuation. No user leaf exists during IPC.
use super::*;
#[path = "read_progress.rs"]
mod read_progress;
#[cfg(feature = "vfs_d2b_test")]
pub(crate) mod lifecycle;
use crate::process::fd_table::{release_service, retain_service, service_description};
use read_progress::{apply_reply, ReadProgress};
use sunlight_ipc::vfs_rpc::{file_transport::IO_BYTES, lifecycle::Caller};

const MAX_SPAN: u64 = 1024 * 1024;
static NEXT_FAULT: spin::Mutex<Option<(Caller, u64)>> = spin::Mutex::new(None);
static PENDING: spin::Mutex<Option<Transaction>> = spin::Mutex::new(None);

struct RetainedFile {
    handle: FileHandle,
    token: u64,
    epoch: u64,
    released: bool,
}

impl RetainedFile {
    fn release(&mut self) -> bool {
        self.released = true;
        release_service(self.handle)
    }
}

impl Drop for RetainedFile {
    fn drop(&mut self) {
        if !self.released {
            // Exact image retirement owns the remote file-owner barrier.
            self.release();
        }
    }
}

struct Transaction {
    caller: Caller,
    file: RetainedFile,
    args: [u64; 6],
    base: u64,
    bytes: Vec<u8>,
    done: usize,
    fault: u64,
}

pub(crate) fn retire(caller: Caller) {
    let mut fault = NEXT_FAULT.lock();
    if fault.is_some_and(|(owner, _)| owner == caller) {
        fault.take();
    }
    drop(fault);
    let mut pending = PENDING.lock();
    if pending.as_ref().is_some_and(|tx| tx.caller == caller) {
        let _retired = pending.take();
        #[cfg(feature = "vfs_d2b_test")]
        if let Some(tx) = _retired {
            let bytes = tx.done;
            drop(tx);
            lifecycle::retired(caller, bytes);
        }
    }
}

fn preflight(args: [u64; 6], sched: &mut Scheduler) -> Result<(u64, u64), MmapError> {
    let [addr, length, prot, flags, _, offset] = args;
    let flags = flags as u32;
    if flags != MAP_PRIVATE
        && flags != MAP_PRIVATE | MAP_FIXED
        && flags != MAP_PRIVATE | MAP_FIXED_NOREPLACE
    {
        return Err(MmapError::InvalidFlags);
    }
    let protection = normalize_protection(prot as u32)?;
    let (_, span) = checked_page_layout(length).map_err(|_| MmapError::InvalidAddress)?;
    if offset & 4095 != 0
        || offset
            .checked_add(span)
            .is_none_or(|end| end > i64::MAX as u64)
    {
        return Err(MmapError::InvalidAddress);
    }
    if span > MAX_SPAN {
        return Err(MmapError::NoMemory);
    }
    let owner = sched.current_memory_owner();
    let identity = owner.address_space.identity();
    if owner.pid != sched.current_pid()
        || sched.current_process().is_exiting()
        || sched.processes.iter().any(|p| {
            p.pid != owner.pid && !p.is_exiting() && p.address_space.identity() == identity
        })
    {
        return Err(MmapError::Unsupported);
    }
    let base = if flags == MAP_PRIVATE {
        DeferredCursor::new(owner.mmap_next, MMAP_REGION_BASE, span)
            .map_err(|_| MmapError::InvalidAddress)?
            .base()
    } else {
        addr
    };
    if base & 4095 != 0 {
        return Err(MmapError::InvalidAddress);
    }
    crate::memory::user::UserRange::new(base, span as usize)
        .map_err(|_| MmapError::InvalidAddress)?;
    let region = MappingRegion::new(
        base,
        base.checked_add(span).ok_or(MmapError::InvalidAddress)?,
        protection,
        MappingKind::Anonymous,
        RegionPolicy::MAY_UNMAP
            .union(RegionPolicy::MAY_REPLACE)
            .union(RegionPolicy::MAY_CHANGE_PROTECTION)
            .union(RegionPolicy::OWNER_MANAGED),
        RegionBacking::AnonymousOwner(owner.pid as u32),
    )
    .map_err(|_| MmapError::InvalidAddress)?;
    let space = &owner.address_space;
    if flags & MAP_FIXED != 0 {
        space.preflight_replace(region).map_err(mapping_error)?;
    } else {
        let hhdm = VirtAddr::new(
            crate::HHDM_REQ
                .response()
                .ok_or(MmapError::InternalInvariant)?
                .offset,
        );
        for address in (base..region.end).step_by(4096) {
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(address));
            if unsafe { space.is_occupied(page, hhdm) } {
                return Err(MmapError::AlreadyMapped);
            }
        }
        let reservation = space.preflight_region(region).map_err(mapping_error)?;
        // The exact image has one executor. No live ledger token is needed
        // across IPC. Recheck placement and reserve again before installation.
        space.cancel_region(reservation);
    }
    Ok((base, span))
}

impl Transaction {
    fn new(
        args: [u64; 6],
        caller: Caller,
        epoch: u64,
        sched: &mut Scheduler,
    ) -> Result<Self, MmapError> {
        let entry = sched
            .current_shared_process()
            .fd_table
            .get(args[4] as i32)
            .copied()
            .ok_or(MmapError::BadFd)?;
        if !entry.handle.is_service() {
            return Err(MmapError::Unsupported);
        }
        if !entry.rights.contains(CapRights::new(CapRights::READ)) {
            return Err(MmapError::PermissionDenied);
        }
        let (token, file_epoch) =
            service_description(entry.handle.service_index()).ok_or(MmapError::BadFd)?;
        if file_epoch != epoch {
            return Err(MmapError::FileIo);
        }
        let (base, span) = preflight(args, sched)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(span as usize)
            .map_err(|_| MmapError::NoMemory)?;
        bytes.resize(span as usize, 0);
        retain_service(entry.handle);
        Ok(Self {
            caller,
            file: RetainedFile {
                handle: entry.handle,
                token,
                epoch,
                released: false,
            },
            args,
            base,
            bytes,
            done: 0,
            fault: NEXT_FAULT
                .lock()
                .take()
                .filter(|(owner, _)| *owner == caller)
                .map_or(0, |(_, fault)| fault),
        })
    }

    fn populate(
        &mut self,
        sched: &mut Scheduler,
        caps: &crate::capability::CapabilityBroker,
    ) -> Result<Option<u64>, MmapError> {
        loop {
            let length = (self.bytes.len() - self.done).min(IO_BYTES);
            if length == 0 {
                break;
            }
            match crate::ipc::vfs_file_call(
                sched.current_pid(),
                crate::ipc::FileOperation::Pread {
                    token: self.file.token,
                    offset: self.args[5] + self.done as u64,
                    length,
                },
                Some((self.caller, self.file.epoch)),
                sched,
                caps,
                |_, _, _| Err(5),
            )
            .map_err(|error| {
                crate::serial_println!(
                    "[VFS-D2A] PREAD transport pid={} bytes={} error={:?}",
                    self.caller.pid,
                    self.done,
                    error
                );
                MmapError::FileIo
            })? {
                crate::ipc::FileCall::Pending => return Ok(None),
                crate::ipc::FileCall::Complete(reply) => {
                    if reply.errno != 0 || reply.registration_epoch != self.file.epoch {
                        crate::serial_println!(
                            "[VFS-D2A] PREAD completion pid={} bytes={} errno={} epoch={}",
                            self.caller.pid,
                            self.done,
                            reply.errno,
                            reply.registration_epoch
                        );
                        return Err(MmapError::FileIo);
                    }
                    if self.fault == 1 && self.done > 0 {
                        return Err(MmapError::FileIo);
                    }
                    if apply_reply(
                        &mut self.bytes,
                        &mut self.done,
                        length,
                        reply.result0,
                        &reply.payload,
                    )
                    .map_err(|_| MmapError::FileIo)?
                        == ReadProgress::Complete
                    {
                        break;
                    }
                }
            }
        }
        let [addr, length, prot, flags, _, _] = self.args;
        let epoch = crate::ipc::vfs_mapping_epoch(sched, caps);
        let expected_epoch = if self.fault == 4 { 0 } else { self.file.epoch };
        if epoch != Some(expected_epoch) || preflight(self.args, sched)?.0 != self.base {
            return Err(MmapError::FileIo);
        }
        let flags = flags as u32;
        let addr = if flags == MAP_PRIVATE { 0 } else { addr };
        let mut populate = |index: u64, frame: PhysAddr, hhdm: VirtAddr| {
            let start = index as usize * 4096;
            // The eager mapper owns this frame exclusively until publication.
            let dst = unsafe {
                core::slice::from_raw_parts_mut((hhdm + frame.as_u64()).as_mut_ptr(), 4096)
            };
            dst.copy_from_slice(&self.bytes[start..start + 4096]);
            Ok(())
        };
        let mut pmm = crate::PMM.lock();
        let counts = |sched: &Scheduler, pmm: &PhysicalMemoryManager| {
            let owner = sched.current_memory_owner();
            (
                pmm.free_page_count(),
                pmm.owned_frame_count(owner.pid as u32),
                owner.address_space.region_count(),
                owner.mmap_next,
            )
        };
        let before = counts(sched, &pmm);
        let allocation_limit = match self.fault {
            2 => Some(0),
            3 | 5 => Some(1),
            6 => Some(4),
            _ => None,
        };
        if let Some(limit) = allocation_limit {
            pmm.fail_test_allocations_after(limit);
        }
        let result = map_private_kind(
            addr,
            length,
            prot as u32,
            flags,
            &mut pmm,
            sched,
            MappingKind::Anonymous,
            false,
            Some(&mut populate),
            true,
        );
        if allocation_limit.is_some() {
            pmm.clear_test_allocation_failure();
            assert_eq!(result, Err(MmapError::NoMemory));
            assert_eq!(before, counts(sched, &pmm));
        }
        let mapped = result?;
        crate::serial_println!(
            "[VFS-D2A] committed pid={} base={:#x} span={} bytes={} offset={} prot={:#x}",
            self.caller.pid,
            mapped,
            self.bytes.len(),
            self.done,
            self.args[5],
            prot
        );
        Ok(Some(mapped))
    }
}

pub(crate) fn step(args: [u64; 6]) -> Result<Option<u64>, MmapError> {
    let mut sched = crate::sched::SCHEDULER.lock();
    // A cross-core reply can mark the caller Ready before the owning CPU
    // has parked its syscall frame. Let dispatch restore Running before
    // consuming the reply or publishing another chunk.
    if sched.current_process().state == crate::process::ProcessState::Ready {
        return Ok(None);
    }
    let caps = crate::capability::CAP_BROKER.lock();
    let caller = crate::ipc::vfs_process_identity(sched.current_process());
    let epoch = crate::ipc::vfs_mapping_epoch(&sched, &caps);
    let previous = {
        let mut pending = PENDING.lock();
        match pending.as_ref() {
            Some(tx) if tx.caller != caller => return Err(MmapError::Unsupported),
            _ => pending.take(),
        }
    };
    let mut tx = match previous {
        Some(tx) => tx,
        None => Transaction::new(args, caller, epoch.ok_or(MmapError::FileIo)?, &mut sched)?,
    };
    let result = if epoch == Some(tx.file.epoch) && tx.args == args {
        tx.populate(&mut sched, &caps)
    } else {
        crate::ipc::vfs_registration::cancel_mapping_read(caller, tx.file.epoch, tx.file.token);
        Err(MmapError::FileIo)
    };
    if matches!(result, Ok(None)) {
        *PENDING.lock() = Some(tx);
        sched.account_and_apply_churn_penalty();
        let tick = sched.global_tick;
        if let Some(idx) = sched.current_process_index() {
            sched.processes[idx].block_start_tick = tick;
            sched.remove_from_ready_queues(idx);
        }
    } else {
        if tx.fault != 0 {
            assert!(result.is_err());
            crate::serial_println!(
                "[VFS-D2A] rollback pid={} fault={} staged_bytes={} error={:?}",
                caller.pid,
                tx.fault,
                tx.done,
                result
            );
        }
        if tx.file.release() {
            crate::ipc::close_vfs_file_owner(
                caller,
                tx.file.token,
                tx.file.epoch,
                &mut sched,
                &caps,
            );
        }
        #[cfg(feature = "vfs_d2b_test")]
        lifecycle::terminal(&tx, &sched, result);
    }
    result
}

/// Ring-3 fixture checkpoints. The syscall path remains the only installer.
pub(crate) fn checkpoint(mode: u64, arg: u64) -> u64 {
    let sched = crate::sched::SCHEDULER.lock();
    let process = sched.current_process();
    if !crate::ipc::vfs_registration::spawn::fixture::is_d2a_caller(process.pid) {
        return u64::MAX;
    }
    match mode {
        24 => {
            assert!((1..=6).contains(&arg));
            let caller = crate::ipc::vfs_process_identity(process);
            assert!(NEXT_FAULT.lock().replace((caller, arg)).is_none());
            1
        }
        25 => {
            let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
            let Ok(address) = VirtAddr::try_new(arg) else {
                return u64::MAX;
            };
            let page = Page::<Size4KiB>::containing_address(address);
            unsafe { process.address_space.lookup_entry(page, hhdm) }
                .map_or(0, |(_, flags)| flags.bits())
                & (PageTableFlags::PRESENT
                    | PageTableFlags::USER_ACCESSIBLE
                    | PageTableFlags::WRITABLE
                    | PageTableFlags::NO_EXECUTE)
                    .bits()
        }
        26 => {
            let pmm = crate::PMM.lock();
            let frames = pmm.owned_frame_count(process.pid as u32);
            let (rows, refs) = super::super::fd_table::FdTable::service_description_counts();
            crate::serial_println!("[VFS-D2A] checkpoint pid={} sample={} owned_frames={} regions={} service_rows={} service_refs={}",
                process.pid, arg, frames, process.address_space.region_count(), rows, refs);
            frames as u64
        }
        _ => unreachable!(),
    }
}
