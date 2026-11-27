extern crate std;

use super::*;
use std::{sync::Barrier, thread};
use sunlight_ipc::vfs_rpc::registration::EndpointOwner;

const GEOMETRY: Geometry = Geometry {
    blocks: 4,
    block_bytes: 512,
};
const RW: Rights = Rights {
    read: true,
    write: true,
    flush: false,
};
const ALL: Rights = Rights { flush: true, ..RW };
const RO: Rights = Rights { write: false, ..RW };
const FIRST: Extent = extent(0, 512);
const WHOLE: Extent = extent(0, 2048);

const fn extent(offset: u64, length: u64) -> Extent {
    Extent { offset, length }
}

fn registration(endpoint: u64, epoch: u64) -> Registration {
    Registration {
        owner: EndpointOwner {
            endpoint,
            pid: 42,
            incarnation: 2,
            address_space_generation: 3,
        },
        epoch,
    }
}

fn setup() -> (DeviceLease, Media, Session) {
    let device = DeviceLease::new();
    let media = device.register_media(GEOMETRY).unwrap();
    let session = device.bind_registration(registration(0, 1)).unwrap();
    (device, media, session)
}

fn sync(device: &DeviceLease, transition: &Transition) -> Flush {
    let flush = device.start_flush(transition).unwrap();
    device.complete_flush(&flush, Outcome::Success).unwrap();
    flush
}

#[test]
fn admission_requires_live_binding_and_exact_lease() {
    let (device, media, incumbent) = setup();
    let foreign = DeviceLease::new()
        .bind_registration(incumbent.registration)
        .unwrap();
    assert_eq!(
        device.acquire_authenticated(&media, &foreign, FIRST, RW),
        Err(Error::Unauthorized)
    );
    assert_eq!(
        device.bind_registration(registration(0, 0)),
        Err(Error::Unauthorized)
    );
    let other = device.bind_registration(registration(1, 2)).unwrap();
    let lease = device
        .acquire_authenticated(&media, &incumbent, FIRST, RO)
        .unwrap();
    assert!(matches!(
        device.read_authenticated(&other, &lease, FIRST),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        device.write_authenticated(&incumbent, &lease, FIRST, [1; BLOCK_SIZE]),
        Err(Error::Unauthorized)
    ));
    let pending = device
        .read_authenticated(&incumbent, &lease, FIRST)
        .unwrap();
    device.withdraw_registration(&incumbent).unwrap();
    let replacement = device.bind_registration(registration(0, 3)).unwrap();
    assert_eq!(
        device.withdraw_registration(&incumbent),
        Err(Error::Unauthorized)
    );
    for session in [&incumbent, &replacement] {
        assert!(matches!(
            device.read_authenticated(session, &lease, FIRST),
            Err(Error::Unauthorized)
        ));
    }
    device.complete(&pending, Outcome::Success).unwrap();
    sync(&device, &device.retire(&lease).unwrap());
    let next = device
        .acquire_authenticated(&media, &replacement, FIRST, RW)
        .unwrap();
    assert!(matches!(
        device.read_authenticated(&replacement, &lease, FIRST),
        Err(Error::Stale)
    ));
    assert!(next.generation() > lease.generation());
}

#[test]
fn extent_checks_precede_credit_and_id_allocation() {
    let (device, media, session) = setup();
    for invalid in [
        extent(0, 0),
        extent(1, 512),
        extent(0, 513),
        extent(1536, 1024),
        extent(u64::MAX, 512),
    ] {
        assert_eq!(
            device.acquire_authenticated(&media, &session, invalid, RW),
            Err(Error::OutOfRange)
        );
    }
    let lease = device
        .acquire_authenticated(&media, &session, extent(512, 1024), RW)
        .unwrap();
    for invalid in [
        FIRST,
        extent(1536, 512),
        extent(512, 1024),
        extent(510, 512),
        extent(u64::MAX, 512),
    ] {
        assert!(matches!(
            device.read_authenticated(&session, &lease, invalid),
            Err(Error::OutOfRange)
        ));
    }
    assert_eq!(device.snapshot().outstanding, 0);
    let last = extent(1024, 512);
    let write = device
        .write_authenticated(&session, &lease, last, [9; BLOCK_SIZE])
        .unwrap();
    assert_eq!(write.0.key.id, lease.generation() + 1);
    assert_eq!(
        device.complete(&write, Outcome::Success),
        Ok(Completion::Written)
    );
    let read = device.read_authenticated(&session, &lease, last).unwrap();
    let mut expected = [0; BLOCK_SIZE];
    expected[..512].fill(9);
    assert_eq!(
        device.complete(&read, Outcome::Success),
        Ok(Completion::Read(expected))
    );
    let first = device
        .read_authenticated(&session, &lease, extent(512, 512))
        .unwrap();
    assert_eq!(
        device.complete(&first, Outcome::Success),
        Ok(Completion::Read([0; BLOCK_SIZE]))
    );
}

#[test]
fn overlapping_and_adjacent_ranges_share_one_exclusive_reservation() {
    let (device, media, session) = setup();
    let other = device.bind_registration(registration(1, 2)).unwrap();
    let lease = device
        .acquire_authenticated(&media, &session, extent(0, 1024), RW)
        .unwrap();
    for requested in [extent(512, 1024), extent(1024, 512)] {
        assert_eq!(
            device.acquire_authenticated(&media, &other, requested, RW),
            Err(Error::Occupied)
        );
    }
    assert!(matches!(
        device.request_flush(&session, &lease),
        Err(Error::Unauthorized)
    ));
    assert_eq!(
        device.acquire_authenticated(&media, &other, FIRST, ALL),
        Err(Error::Unauthorized)
    );
    let transfer = device
        .begin_authenticated_transfer(&session, &lease, &other, extent(1024, 512), RW)
        .unwrap();
    sync(&device, &transfer);
    let next = device.finish(&transfer).unwrap().unwrap();
    assert!(matches!(
        device.read_authenticated(&other, &next, FIRST),
        Err(Error::OutOfRange)
    ));
    let read = device
        .read_authenticated(&other, &next, extent(1024, 512))
        .unwrap();
    device.complete(&read, Outcome::Success).unwrap();
}

#[test]
fn planned_replacement_changes_geometry_and_rejects_old_completions() {
    let (device, media, session) = setup();
    assert_eq!(device.register_media(GEOMETRY), Err(Error::Occupied));
    let lease = device
        .acquire_authenticated(&media, &session, WHOLE, ALL)
        .unwrap();
    let old_write = device
        .write_authenticated(&session, &lease, FIRST, [8; BLOCK_SIZE])
        .unwrap();
    let release = device.begin_release(&lease).unwrap();
    assert!(matches!(device.start_flush(&release), Err(Error::Busy)));
    device.complete(&old_write, Outcome::Success).unwrap();
    let old_flush = sync(&device, &release);
    device.finish(&release).unwrap();
    device.remove_media(&media).unwrap();
    assert_eq!(
        device.acquire_authenticated(&media, &session, FIRST, RW),
        Err(Error::NoMedia)
    );
    for invalid in [
        Geometry {
            blocks: 0,
            ..GEOMETRY
        },
        Geometry {
            blocks: u64::MAX,
            ..GEOMETRY
        },
        Geometry {
            block_bytes: 513,
            ..GEOMETRY
        },
    ] {
        assert_eq!(device.register_media(invalid), Err(Error::InvalidGeometry));
    }
    let replacement = device
        .register_media(Geometry {
            blocks: 1,
            block_bytes: 256,
        })
        .unwrap();
    assert_eq!(device.remove_media(&media), Err(Error::Stale));
    assert_eq!(
        device.acquire_authenticated(&media, &session, FIRST, RW),
        Err(Error::Stale)
    );
    assert_eq!(
        device.acquire_authenticated(&replacement, &session, FIRST, RW),
        Err(Error::OutOfRange)
    );
    assert_eq!(
        device.acquire_authenticated(&replacement, &session, WHOLE, RW),
        Err(Error::OutOfRange)
    );
    let whole = extent(0, 256);
    let next = device
        .acquire_authenticated(&replacement, &session, whole, ALL)
        .unwrap();
    let current = device.read_authenticated(&session, &next, whole).unwrap();
    assert_eq!(
        device.complete(&old_write, Outcome::Ambiguous { applied: true }),
        Err(Error::Stale)
    );
    assert_eq!(
        device.complete(&current, Outcome::Success),
        Ok(Completion::Read([0; BLOCK_SIZE]))
    );
    let flush = device
        .start_flush(&device.request_flush(&session, &next).unwrap())
        .unwrap();
    assert_eq!(
        device.complete_flush(&old_flush, Outcome::Failed),
        Err(Error::Stale)
    );
    assert!(device.snapshot().flush_pending);
    device.complete_flush(&flush, Outcome::Success).unwrap();
}

#[test]
fn removed_media_settles_old_work_without_unlocking_replacement() {
    let (device, media, session) = setup();
    let lease = device
        .acquire_authenticated(&media, &session, WHOLE, ALL)
        .unwrap();
    let old_write = device
        .write_authenticated(&session, &lease, FIRST, [5; BLOCK_SIZE])
        .unwrap();
    device.remove_media(&media).unwrap();
    assert!(matches!(
        device.read_authenticated(&session, &lease, FIRST),
        Err(Error::AdmissionClosed)
    ));
    assert_eq!(device.snapshot().outstanding, 1);
    assert_eq!(device.register_media(GEOMETRY), Err(Error::Fenced));
    assert_eq!(
        device.complete(&old_write, Outcome::Ambiguous { applied: true }),
        Ok(Completion::Ambiguous)
    );
    assert_eq!(
        device.complete(&old_write, Outcome::Success),
        Err(Error::Stale)
    );
    assert_eq!(device.snapshot().outstanding, 0);
    assert_eq!(
        device.snapshot().status,
        Status::Fenced(Fence::MediaRemoved)
    );
    assert_eq!(device.register_media(GEOMETRY), Err(Error::Fenced));
}

#[test]
fn whole_device_flush_drains_and_ambiguity_fences() {
    let (device, media, session) = setup();
    let lease = device
        .acquire_authenticated(&media, &session, WHOLE, ALL)
        .unwrap();
    let write = device
        .write_authenticated(&session, &lease, FIRST, [5; BLOCK_SIZE])
        .unwrap();
    let flush = device.request_flush(&session, &lease).unwrap();
    assert!(matches!(device.start_flush(&flush), Err(Error::Busy)));
    assert!(matches!(
        device.read_authenticated(&session, &lease, FIRST),
        Err(Error::AdmissionClosed)
    ));
    device.complete(&write, Outcome::Success).unwrap();
    assert_eq!(device.state.lock().transport.durable[0][0], 0);
    sync(&device, &flush);
    assert_eq!(device.state.lock().transport.durable[0][0], 5);
    assert_eq!(device.snapshot().status, Status::Active);
    let flush = device
        .start_flush(&device.request_flush(&session, &lease).unwrap())
        .unwrap();
    device
        .complete_flush(&flush, Outcome::Ambiguous { applied: false })
        .unwrap();
    assert_eq!(
        device.snapshot().status,
        Status::Fenced(Fence::AmbiguousFlush)
    );
    assert!(!device.snapshot().flush_pending);
}

#[test]
fn concurrent_admission_withdrawal_and_transfer_linearize() {
    let (device, media, incumbent) = setup();
    let successor = device.bind_registration(registration(1, 2)).unwrap();
    let lease = device
        .acquire_authenticated(&media, &incumbent, WHOLE, ALL)
        .unwrap();
    let start = Barrier::new(4);
    let (admitted, transfer) = thread::scope(|scope| {
        let admission = scope.spawn(|| {
            start.wait();
            device.read_authenticated(&incumbent, &lease, FIRST)
        });
        let withdrawal = scope.spawn(|| {
            start.wait();
            device.withdraw_registration(&incumbent)
        });
        let transfer = scope.spawn(|| {
            start.wait();
            device.begin_authenticated_transfer(&incumbent, &lease, &successor, FIRST, RW)
        });
        start.wait();
        withdrawal.join().unwrap().unwrap();
        (admission.join().unwrap(), transfer.join().unwrap())
    });
    if let Ok(command) = admitted {
        device.complete(&command, Outcome::Success).unwrap();
    } else {
        assert!(matches!(
            admitted,
            Err(Error::Unauthorized | Error::AdmissionClosed)
        ));
    }
    if let Ok(change) = transfer {
        sync(&device, &change);
        assert_eq!(device.finish(&change), Err(Error::Unauthorized));
    } else {
        assert!(matches!(transfer, Err(Error::Unauthorized)));
    }
    let retire = device.retire(&lease).unwrap();
    if device.snapshot().status != Status::Vacant {
        sync(&device, &retire);
    }
    assert!(device
        .acquire_authenticated(&media, &successor, FIRST, RW)
        .is_ok());
}

#[test]
fn withdrawn_successor_cannot_commit_and_lifecycle_reclaims_resources() {
    let (device, mut media, incumbent) = setup();
    let successor = device.bind_registration(registration(1, 2)).unwrap();
    let lease = device
        .acquire_authenticated(&media, &incumbent, WHOLE, ALL)
        .unwrap();
    let change = device
        .begin_authenticated_transfer(&incumbent, &lease, &successor, FIRST, RW)
        .unwrap();
    sync(&device, &change);
    device.withdraw_registration(&successor).unwrap();
    assert_eq!(device.finish(&change), Err(Error::Unauthorized));
    device.cancel(&change).unwrap();
    device.withdraw_registration(&incumbent).unwrap();
    sync(&device, &device.retire(&lease).unwrap());
    for epoch in 3..67 {
        let session = device.bind_registration(registration(0, epoch)).unwrap();
        let lease = device
            .acquire_authenticated(&media, &session, WHOLE, ALL)
            .unwrap();
        let pending = core::array::from_fn::<_, DATA_CREDITS, _>(|_| {
            device.read_authenticated(&session, &lease, FIRST).unwrap()
        });
        assert!(matches!(
            device.read_authenticated(&session, &lease, FIRST),
            Err(Error::NoCredit)
        ));
        device.withdraw_registration(&session).unwrap();
        let retire = device.retire(&lease).unwrap();
        for command in pending {
            device.complete(&command, Outcome::Failed).unwrap();
        }
        sync(&device, &retire);
        device.remove_media(&media).unwrap();
        assert_eq!(
            device.snapshot(),
            Snapshot {
                lease: None,
                status: Status::Vacant,
                outstanding: 0,
                retained_data: 0,
                flush_pending: false
            }
        );
        assert!(device.state.lock().admission.sessions.is_empty());
        media = device.register_media(GEOMETRY).unwrap();
    }
}
