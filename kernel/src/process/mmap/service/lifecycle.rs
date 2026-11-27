//! Rendezvous and accounting for the isolated native lifecycle fixture.
use super::*;
use crate::process::{fd_table::FdTable, Process, ProcessState};

struct Probe {
    caller: Caller,
    action: u64,
    baseline: Sample,
    held: Option<(Caller, u64)>,
    retired: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct Sample {
    frames: usize,
    regions: usize,
    cursor: u64,
}

impl Sample {
    fn read(process: &Process) -> Self {
        Self {
            frames: crate::PMM.lock().owned_frame_count(process.pid as u32),
            regions: process.address_space.region_count(),
            cursor: process.mmap_next,
        }
    }
}

static PROBE: spin::Mutex<Option<Probe>> = spin::Mutex::new(None);

fn assert_uninstalled(process: &Process, tx: &Transaction, probe: &Probe) {
    assert_eq!(Sample::read(process), probe.baseline);
    let hhdm = VirtAddr::new(crate::HHDM_REQ.response().unwrap().offset);
    for address in (tx.base..tx.base + tx.bytes.len() as u64).step_by(4096) {
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(address));
        assert!(!unsafe { process.address_space.is_occupied(page, hhdm) });
    }
}

pub(super) fn retired(caller: Caller, bytes: usize) {
    let mut probe = PROBE.lock();
    if let Some(probe) = probe.as_mut().filter(|probe| probe.caller == caller) {
        let (_, id) = probe.held.expect("service holding the reply");
        assert_eq!((probe.action, bytes), (2, 4096));
        assert_eq!(FdTable::service_description_counts(), (1, 1));
        probe.retired = true;
        crate::serial_println!(
            "[VFS-D2B] terminal pid={} incarnation={} image={} id={} disposition=caller-retired staged={} service_rows=1 service_refs=1",
            caller.pid, caller.incarnation, caller.address_space_generation, id, bytes,
        );
    }
}

pub(super) fn terminal(
    tx: &Transaction,
    sched: &Scheduler,
    result: Result<Option<u64>, MmapError>,
) {
    let mut probe = PROBE.lock();
    let Some(active) = probe.as_ref().filter(|probe| probe.caller == tx.caller) else {
        return;
    };
    let disposition = match active.action {
        1 => {
            assert_eq!(result, Ok(Some(tx.base)));
            "committed"
        }
        3 => {
            assert_eq!(result, Err(MmapError::FileIo));
            assert_uninstalled(sched.current_process(), tx, active);
            "service-withdrawn"
        }
        _ => panic!("exit fixture returned from mmap"),
    };
    let (_, id) = active.held.expect("held PREAD preceded terminal result");
    assert_eq!(FdTable::service_description_counts(), (1, 1));
    let after = Sample::read(sched.current_process());
    crate::serial_println!(
        "[VFS-D2B] terminal pid={} incarnation={} image={} id={} disposition={} staged={} frames={} regions={} cursor={} service_rows=1 service_refs=1",
        tx.caller.pid, tx.caller.incarnation, tx.caller.address_space_generation,
        id, disposition, tx.done, after.frames, after.regions, after.cursor,
    );
    probe.take();
}

pub(crate) fn checkpoint(mode: u64, arg: u64) -> u64 {
    let sched = crate::sched::SCHEDULER.lock();
    let process = sched.current_process();
    let caller = crate::ipc::vfs_process_identity(process);
    let mut state = PROBE.lock();
    if mode == 27 && crate::ipc::vfs_registration::spawn::fixture::is_d2a_caller(process.pid) {
        assert!((1..=3).contains(&arg));
        assert_eq!(FdTable::service_description_counts(), (1, 1));
        assert!(state
            .replace(Probe {
                caller,
                action: arg,
                baseline: Sample::read(process),
                held: None,
                retired: false,
            })
            .is_none());
        return 1;
    }
    if mode == 29 && process.pid == 1 {
        return u64::from(state.as_ref().is_some_and(|probe| {
            probe.action == 2 && probe.caller.pid == arg && probe.held.is_some()
        }));
    }
    if mode == 30 && process.trusted_vfs_service {
        return u64::from(state.is_none());
    }
    if mode != 28 || !process.trusted_vfs_service {
        return u64::MAX;
    }
    let Some(probe) = state.as_mut() else {
        return 1;
    };
    if probe.held == Some((caller, arg)) {
        return if probe.retired {
            state.take();
            1
        } else {
            0
        };
    }
    let Some((epoch, token)) =
        crate::ipc::vfs_registration::fixture_mapping_read(probe.caller, caller, arg)
    else {
        return 1;
    };
    let pending = PENDING.lock();
    let tx = pending.as_ref().expect("published mmap continuation");
    assert_eq!(
        (tx.caller, tx.file.epoch, tx.file.token, tx.done),
        (probe.caller, epoch, token, 4096)
    );
    let index = sched.process_index_by_pid(tx.caller.pid as usize).unwrap();
    let owner = &sched.processes[index];
    if owner.state != ProcessState::BlockedOnIpc || sched.live_owner_core(index).is_some() {
        return 0;
    }
    assert_uninstalled(owner, tx, probe);
    assert_eq!(FdTable::service_description_counts(), (1, 2));
    crate::serial_println!(
        "[VFS-D2B] parked pid={} incarnation={} image={} epoch={} id={} token={} action={} staged={} frames={} regions={} cursor={} service_rows=1 service_refs=2",
        tx.caller.pid, tx.caller.incarnation, tx.caller.address_space_generation, epoch, arg, token,
        probe.action, tx.done, probe.baseline.frames, probe.baseline.regions, probe.baseline.cursor,
    );
    probe.held = Some((caller, arg));
    match probe.action {
        1 => 1,
        2 => 0,
        3 => 2,
        _ => unreachable!(),
    }
}
