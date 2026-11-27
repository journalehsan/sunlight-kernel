extern crate std;

use super::*;
use sunlight_ipc::vfs_rpc::registration::EndpointOwner;

pub(super) fn owner(pid: u64) -> Registration {
    Registration {
        owner: EndpointOwner {
            endpoint: pid,
            pid,
            incarnation: 1,
            address_space_generation: 1,
        },
        epoch: pid,
    }
}

fn synchronize(device: &DeviceLease, change: &Transition) -> Flush {
    let flush = device.start_flush(change).unwrap();
    device.complete_flush(&flush, Outcome::Success).unwrap();
    flush
}

#[test]
fn acquire_and_release() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    assert_eq!(lease.owner(), owner(1));
    let change = device.begin_release(&lease).unwrap();
    assert_eq!(device.snapshot().status, Status::Quiescing);
    assert_eq!(device.acquire(owner(2)), Err(Error::Occupied));
    assert_eq!(device.finish(&change), Err(Error::Busy));
    synchronize(&device, &change);
    assert_eq!(device.snapshot().status, Status::Synchronized);
    assert_eq!(device.finish(&change), Ok(None));
    assert_eq!(device.snapshot().status, Status::Vacant);
    assert!(matches!(device.read(&lease, 0), Err(Error::Stale)));
}

#[test]
fn duplicate_acquisition_and_cross_device_keys_are_rejected() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    assert_eq!(device.acquire(owner(1)), Err(Error::Occupied));
    assert_eq!(device.acquire(owner(2)), Err(Error::Occupied));

    let other = DeviceLease::new();
    let other_lease = other.acquire(owner(1)).unwrap();
    assert_eq!(lease.generation(), other_lease.generation());
    assert!(matches!(
        other.write(&lease, 0, [1; BLOCK_SIZE]),
        Err(Error::Stale)
    ));
    let command = device.read(&lease, 0).unwrap();
    let other_command = other.read(&other_lease, 0).unwrap();
    assert_eq!(
        other.complete(&command, Outcome::Success),
        Err(Error::Stale)
    );
    assert_eq!(other.snapshot().outstanding, 1);
    assert_eq!(
        other.complete(&other_command, Outcome::Success),
        Ok(Completion::Read([0; BLOCK_SIZE]))
    );
}

#[test]
fn outstanding_operations_use_bounded_credits_and_complete_once() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    assert!(matches!(
        device.read(&lease, BLOCKS),
        Err(Error::OutOfRange)
    ));
    let operations =
        core::array::from_fn::<_, DATA_CREDITS, _>(|_| device.read(&lease, 0).unwrap());
    assert_eq!(device.snapshot().outstanding, DATA_CREDITS);
    assert!(matches!(device.read(&lease, 0), Err(Error::NoCredit)));
    assert_eq!(
        device.complete(&operations[0], Outcome::Failed),
        Ok(Completion::Failed)
    );
    let replacement = device.read(&lease, 1).unwrap();
    assert_eq!(
        device.complete(&operations[0], Outcome::Ambiguous { applied: true }),
        Err(Error::Stale)
    );
    assert_eq!(device.snapshot().outstanding, DATA_CREDITS);
    let expected = [0; BLOCK_SIZE];
    for operation in operations.iter().skip(1).chain([&replacement]) {
        assert_eq!(
            device.complete(operation, Outcome::Success),
            Ok(Completion::Read(expected))
        );
    }
    assert_eq!(device.snapshot().outstanding, 0);
    assert_eq!(device.snapshot().status, Status::Active);
}

#[test]
fn transfer_requires_drain_then_confirmed_durability() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    let bytes = [17; BLOCK_SIZE];
    let write = device.write(&lease, 0, bytes).unwrap();
    let read = device.read(&lease, 0).unwrap();
    let change = device.begin_transfer(&lease, owner(2)).unwrap();
    assert!(matches!(
        device.read(&lease, 0),
        Err(Error::AdmissionClosed)
    ));
    assert!(matches!(
        device.write(&lease, 0, bytes),
        Err(Error::AdmissionClosed)
    ));
    assert!(matches!(device.start_flush(&change), Err(Error::Busy)));
    assert_eq!(
        device.complete(&write, Outcome::Success),
        Ok(Completion::Written)
    );
    assert!(matches!(device.start_flush(&change), Err(Error::Busy)));
    assert_eq!(
        device.complete(&read, Outcome::Success),
        Ok(Completion::Read(bytes))
    );
    assert_eq!(device.state.lock().transport.durable[0], [0; BLOCK_SIZE]);
    let flush = device.start_flush(&change).unwrap();
    assert!(device.snapshot().flush_pending);
    assert_eq!(device.finish(&change), Err(Error::Busy));
    assert_eq!(device.acquire(owner(2)), Err(Error::Occupied));
    device.complete_flush(&flush, Outcome::Success).unwrap();
    assert_eq!(device.state.lock().transport.durable[0], bytes);
    assert_eq!(device.snapshot().lease, Some(lease.clone()));
    let successor = device.finish(&change).unwrap().unwrap();
    assert_eq!(successor.owner(), owner(2));
    assert!(successor.generation() > lease.generation());
    let read = device.read(&successor, 0).unwrap();
    assert_eq!(
        device.complete(&read, Outcome::Success),
        Ok(Completion::Read(bytes))
    );
    assert!(matches!(device.write(&lease, 0, bytes), Err(Error::Stale)));
}

#[test]
fn failed_synchronization_resumes_same_owner_and_allows_retry() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    let write = device.write(&lease, 0, [5; BLOCK_SIZE]).unwrap();
    device.complete(&write, Outcome::Success).unwrap();
    let failed = device.begin_transfer(&lease, owner(2)).unwrap();
    let flush = device.start_flush(&failed).unwrap();
    device.complete_flush(&flush, Outcome::Failed).unwrap();
    assert_eq!(device.snapshot().status, Status::Active);
    assert_eq!(device.snapshot().lease, Some(lease.clone()));
    assert_eq!(device.finish(&failed), Err(Error::Stale));
    assert_eq!(device.state.lock().transport.durable[0], [0; BLOCK_SIZE]);
    let retry = device.begin_transfer(&lease, owner(2)).unwrap();
    let retry_flush = device.start_flush(&retry).unwrap();
    assert_eq!(
        device.complete_flush(&flush, Outcome::Success),
        Err(Error::Stale)
    );
    assert!(device.snapshot().flush_pending);
    device
        .complete_flush(&retry_flush, Outcome::Success)
        .unwrap();
    assert_eq!(device.finish(&retry).unwrap().unwrap().owner(), owner(2));
    assert_eq!(device.state.lock().transport.durable[0], [5; BLOCK_SIZE]);
}

#[test]
fn cancellation_before_flush_keeps_accepted_work_and_invalidates_ticket() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    let first = device.read(&lease, 0).unwrap();
    let cancelled = device.begin_transfer(&lease, owner(2)).unwrap();
    device.cancel(&cancelled).unwrap();
    assert_eq!(device.snapshot().status, Status::Active);
    assert_eq!(device.snapshot().outstanding, 1);
    let retry = device.begin_transfer(&lease, owner(2)).unwrap();
    assert!(matches!(device.start_flush(&cancelled), Err(Error::Stale)));
    assert_eq!(device.cancel(&cancelled), Err(Error::Stale));
    device.complete(&first, Outcome::Success).unwrap();
    synchronize(&device, &retry);
    device.cancel(&retry).unwrap();
    assert_eq!(device.finish(&retry), Err(Error::Stale));
    assert_eq!(device.snapshot().lease, Some(lease));
    assert_eq!(device.snapshot().status, Status::Active);
}

#[test]
fn cancellation_during_flush_waits_for_definite_completion() {
    for outcome in [
        Outcome::Success,
        Outcome::Failed,
        Outcome::Ambiguous { applied: false },
        Outcome::Ambiguous { applied: true },
    ] {
        let device = DeviceLease::new();
        let lease = device.acquire(owner(1)).unwrap();
        let write = device.write(&lease, 0, [4; BLOCK_SIZE]).unwrap();
        device.complete(&write, Outcome::Success).unwrap();
        let change = device.begin_transfer(&lease, owner(2)).unwrap();
        let flush = device.start_flush(&change).unwrap();
        device.cancel(&change).unwrap();
        assert_eq!(device.snapshot().status, Status::Cancelling);
        assert!(matches!(
            device.read(&lease, 0),
            Err(Error::AdmissionClosed)
        ));
        assert_eq!(device.finish(&change), Err(Error::Busy));
        device.complete_flush(&flush, outcome).unwrap();
        assert_eq!(device.snapshot().lease, Some(lease));
        assert!(!device.snapshot().flush_pending);
        let durable = if outcome.applied() { 4 } else { 0 };
        assert_eq!(
            device.state.lock().transport.durable[0],
            [durable; BLOCK_SIZE]
        );
        if outcome.ambiguous() {
            assert_eq!(
                device.snapshot().status,
                Status::Fenced(Fence::AmbiguousFlush)
            );
            assert_eq!(device.finish(&change), Err(Error::Fenced));
            assert_eq!(device.acquire(owner(2)), Err(Error::Fenced));
        } else {
            assert_eq!(device.snapshot().status, Status::Active);
            assert_eq!(device.finish(&change), Err(Error::Stale));
        }
    }
}

#[test]
fn delayed_completions_and_notifications_cannot_affect_a_new_generation() {
    let device = DeviceLease::new();
    let old = device.acquire(owner(1)).unwrap();
    let old_write = device.write(&old, 0, [11; BLOCK_SIZE]).unwrap();
    let retirement = device.retire(&old).unwrap();
    device.complete(&old_write, Outcome::Success).unwrap();
    let old_flush = synchronize(&device, &retirement);
    let current = device.acquire(owner(1)).unwrap();
    let write = device.write(&current, 0, [12; BLOCK_SIZE]).unwrap();
    assert_ne!(current.generation(), old.generation());
    let snapshot = device.snapshot();
    assert_eq!(
        device.complete(&old_write, Outcome::Ambiguous { applied: true }),
        Err(Error::Stale)
    );
    assert_eq!(
        device.complete_flush(&old_flush, Outcome::Ambiguous { applied: true }),
        Err(Error::Stale)
    );
    assert_eq!(device.transport_lost(&old), Err(Error::Stale));
    assert!(matches!(device.retire(&old), Err(Error::Stale)));
    assert!(matches!(device.begin_release(&old), Err(Error::Stale)));
    assert_eq!(device.snapshot(), snapshot);
    device.complete(&write, Outcome::Success).unwrap();
    assert_eq!(device.state.lock().transport.visible[0], [12; BLOCK_SIZE]);
}

#[test]
fn retirement_drains_active_work_and_invalidates_pending_transfer() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    let write = device.write(&lease, 1, [9; BLOCK_SIZE]).unwrap();
    let transfer = device.begin_transfer(&lease, owner(2)).unwrap();
    let retirement = device.retire(&lease).unwrap();
    assert_eq!(device.snapshot().status, Status::Retiring);
    assert!(matches!(
        device.read(&lease, 0),
        Err(Error::AdmissionClosed)
    ));
    assert_eq!(device.cancel(&transfer), Err(Error::Stale));
    assert!(matches!(device.start_flush(&retirement), Err(Error::Busy)));
    device.complete(&write, Outcome::Success).unwrap();
    let failed = device.start_flush(&retirement).unwrap();
    device.complete_flush(&failed, Outcome::Failed).unwrap();
    assert_eq!(device.snapshot().status, Status::Retiring);
    assert_eq!(device.acquire(owner(2)), Err(Error::Occupied));
    synchronize(&device, &retirement);
    assert_eq!(device.snapshot().status, Status::Vacant);
    assert_eq!(device.finish(&transfer), Err(Error::Stale));
    assert_eq!(device.state.lock().transport.durable[1], [9; BLOCK_SIZE]);
}

#[test]
fn retirement_supersedes_an_in_flight_or_completed_transfer_barrier() {
    for complete_first in [true, false] {
        let device = DeviceLease::new();
        let lease = device.acquire(owner(1)).unwrap();
        let transfer = device.begin_transfer(&lease, owner(2)).unwrap();
        let flush = device.start_flush(&transfer).unwrap();
        if complete_first {
            device.complete_flush(&flush, Outcome::Success).unwrap();
        }
        device.retire(&lease).unwrap();
        assert_eq!(device.finish(&transfer), Err(Error::Stale));
        if !complete_first {
            assert_eq!(device.snapshot().status, Status::Retiring);
            device.complete_flush(&flush, Outcome::Success).unwrap();
        }
        assert_eq!(device.snapshot().status, Status::Vacant);
    }
}

#[test]
fn ambiguous_write_fences_transfer_and_preserves_other_credits() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    let first = device.write(&lease, 0, [8; BLOCK_SIZE]).unwrap();
    let second = device.write(&lease, 1, [9; BLOCK_SIZE]).unwrap();
    let change = device.begin_transfer(&lease, owner(2)).unwrap();
    assert_eq!(
        device.complete(&first, Outcome::Ambiguous { applied: true }),
        Ok(Completion::Ambiguous)
    );
    assert_eq!(device.snapshot().outstanding, 1);
    assert_eq!(device.state.lock().transport.visible[0], [8; BLOCK_SIZE]);
    assert_eq!(device.snapshot().status, Status::Fenced(Fence::AmbiguousIo));
    assert_eq!(
        device.complete(&second, Outcome::Success),
        Ok(Completion::Written)
    );
    assert_eq!(device.snapshot().outstanding, 0);
    assert_eq!(device.snapshot().lease, Some(lease.clone()));
    assert_eq!(device.finish(&change), Err(Error::Fenced));
    assert_eq!(device.acquire(owner(2)), Err(Error::Fenced));
    assert!(matches!(
        device.write(&lease, 0, [1; BLOCK_SIZE]),
        Err(Error::AdmissionClosed)
    ));
}

#[test]
fn transport_loss_keeps_pending_data_or_flush_accounted_until_settlement() {
    for during_flush in [false, true] {
        let device = DeviceLease::new();
        let lease = device.acquire(owner(1)).unwrap();
        let write = device.write(&lease, 0, [7; BLOCK_SIZE]).unwrap();
        let change = device.begin_transfer(&lease, owner(2)).unwrap();
        let flush = during_flush.then(|| {
            device.complete(&write, Outcome::Success).unwrap();
            device.start_flush(&change).unwrap()
        });
        device.transport_lost(&lease).unwrap();
        assert_eq!(device.snapshot().flush_pending, during_flush);
        assert_eq!(device.snapshot().outstanding, usize::from(!during_flush));
        if let Some(flush) = flush {
            device.complete_flush(&flush, Outcome::Success).unwrap();
        } else {
            device.complete(&write, Outcome::Failed).unwrap();
        }
        assert_eq!(
            device.snapshot().status,
            Status::Fenced(Fence::TransportLost)
        );
        assert_eq!(device.snapshot().outstanding, 0);
        assert!(!device.snapshot().flush_pending);
        assert_eq!(device.finish(&change), Err(Error::Fenced));
        assert_eq!(device.acquire(owner(2)), Err(Error::Fenced));
    }
}

#[test]
fn concurrent_acquire_and_transfer_attempts_have_one_winner() {
    use std::{sync::Barrier, thread};

    let device = DeviceLease::new();
    let start = Barrier::new(2);
    let lease = thread::scope(|scope| {
        let first = scope.spawn(|| {
            start.wait();
            device.acquire(owner(1))
        });
        start.wait();
        let second = device.acquire(owner(2));
        match (first.join().unwrap(), second) {
            (Ok(lease), Err(Error::Occupied)) | (Err(Error::Occupied), Ok(lease)) => lease,
            other => panic!("expected one acquisition: {other:?}"),
        }
    });
    let start = Barrier::new(3);
    let change = thread::scope(|scope| {
        let first = scope.spawn(|| {
            start.wait();
            device.begin_transfer(&lease, owner(3))
        });
        let acquire = scope.spawn(|| {
            start.wait();
            device.acquire(owner(5))
        });
        start.wait();
        let second = device.begin_transfer(&lease, owner(4));
        assert_eq!(acquire.join().unwrap(), Err(Error::Occupied));
        match (first.join().unwrap(), second) {
            (Ok(change), Err(Error::AdmissionClosed))
            | (Err(Error::AdmissionClosed), Ok(change)) => change,
            other => panic!("expected one transfer: {other:?}"),
        }
    });
    synchronize(&device, &change);
    let start = Barrier::new(2);
    let next = thread::scope(|scope| {
        let first = scope.spawn(|| {
            start.wait();
            device.finish(&change)
        });
        start.wait();
        let second = device.finish(&change);
        match (first.join().unwrap(), second) {
            (Ok(Some(lease)), Err(Error::Stale)) | (Err(Error::Stale), Ok(Some(lease))) => lease,
            other => panic!("expected one commit: {other:?}"),
        }
    });
    assert!([owner(3), owner(4)].contains(&next.owner()));
    assert_eq!(device.finish(&change), Err(Error::Stale));
}

#[test]
fn repeated_lifecycles_reclaim_all_operation_and_flush_credits() {
    let device = DeviceLease::new();
    let mut last_generation = 0;
    for value in 0..64 {
        let lease = device.acquire(owner(1)).unwrap();
        assert!(lease.generation() > last_generation);
        let operations = core::array::from_fn::<_, DATA_CREDITS, _>(|block| {
            device.write(&lease, block, [value; BLOCK_SIZE]).unwrap()
        });
        let transfer = device.begin_transfer(&lease, owner(2)).unwrap();
        for operation in operations.iter().rev() {
            device.complete(operation, Outcome::Success).unwrap();
        }
        synchronize(&device, &transfer);
        let lease = device.finish(&transfer).unwrap().unwrap();
        let release = device.begin_release(&lease).unwrap();
        synchronize(&device, &release);
        assert_eq!(device.finish(&release), Ok(None));
        assert_eq!(
            device.snapshot(),
            Snapshot {
                lease: None,
                status: Status::Vacant,
                outstanding: 0,
                retained_data: 0,
                flush_pending: false,
            }
        );
        last_generation = lease.generation();
    }
}
