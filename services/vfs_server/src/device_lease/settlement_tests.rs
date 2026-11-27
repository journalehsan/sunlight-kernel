use super::{tests::owner, *};

#[test]
fn settlement_tests_removed_media_retains_authenticated_buffer() {
    let device = DeviceLease::new();
    let session = device.bind_registration(owner(1)).unwrap();
    let geometry = Geometry {
        blocks: 1,
        block_bytes: BLOCK_SIZE as u64,
    };
    let media = device.register_media(geometry).unwrap();
    let lease = device
        .acquire_authenticated(
            &media,
            &session,
            Extent {
                offset: 0,
                length: geometry.block_bytes,
            },
            Rights {
                read: true,
                write: false,
                flush: false,
            },
        )
        .unwrap();
    let op = device
        .read_authenticated(
            &session,
            &lease,
            Extent {
                offset: 0,
                length: geometry.block_bytes,
            },
        )
        .unwrap();
    device.remove_media(&media).unwrap();
    device.abandon(&op).unwrap();
    assert_eq!(device.snapshot().outstanding, 0);
    assert_eq!(device.snapshot().retained_data, 1);
    assert_eq!(device.register_media(geometry), Err(Error::Fenced));
    device.complete(&op, Outcome::Failed).unwrap();
    assert_eq!(device.snapshot().retained_data, 0);
    assert_eq!(
        device.snapshot().status,
        Status::Fenced(Fence::MediaRemoved)
    );
    assert_eq!(device.register_media(geometry), Err(Error::Fenced));
}

#[test]
fn settlement_tests_ordered_drain_and_transfer() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    let write = device.write(&lease, 0, [7; BLOCK_SIZE]).unwrap();
    let read = device.read(&lease, 1).unwrap();
    let change = device.begin_transfer(&lease, owner(2)).unwrap();
    assert!(matches!(
        device.read(&lease, 0),
        Err(Error::AdmissionClosed)
    ));
    assert_eq!(
        device.complete(&read, Outcome::Success),
        Ok(Completion::Read([0; BLOCK_SIZE]))
    );
    assert_eq!(device.snapshot().retained_data, 1);
    device.abandon(&write).unwrap();
    assert_eq!(device.abandon(&write), Err(Error::Stale));
    assert_eq!(device.snapshot().outstanding, 0);
    assert!(matches!(device.start_flush(&change), Err(Error::Busy)));
    assert_eq!(device.finish(&change), Err(Error::Busy));
    assert_eq!(
        device.complete(&write, Outcome::Success),
        Ok(Completion::Written)
    );
    assert_eq!(device.snapshot().retained_data, 0);
    assert_eq!(device.complete(&write, Outcome::Success), Err(Error::Stale));
    let flush = device.start_flush(&change).unwrap();
    assert_eq!(device.finish(&change), Err(Error::Busy));
    device.complete_flush(&flush, Outcome::Success).unwrap();
    let next = device.finish(&change).unwrap().unwrap();
    assert_eq!(next.owner(), owner(2));
    assert_eq!(device.state.lock().transport.durable[0], [7; BLOCK_SIZE]);
    assert_eq!(device.abandon(&write), Err(Error::Stale));
}

#[test]
fn settlement_tests_loss_keeps_fence_after_late_ack() {
    let device = DeviceLease::new();
    let lease = device.acquire(owner(1)).unwrap();
    let op = device.read(&lease, 0).unwrap();
    let change = device.begin_release(&lease).unwrap();
    device.transport_lost(&lease).unwrap();
    device.abandon(&op).unwrap();
    assert_eq!(device.snapshot().retained_data, 1);
    assert_eq!(device.finish(&change), Err(Error::Fenced));
    assert_eq!(
        device.complete(&op, Outcome::Failed),
        Ok(Completion::Failed)
    );
    assert_eq!(device.snapshot().retained_data, 0);
    assert_eq!(
        device.snapshot().status,
        Status::Fenced(Fence::TransportLost)
    );
    assert_eq!(device.acquire(owner(2)), Err(Error::Fenced));
}

#[test]
fn settlement_tests_bounded_retention_across_lifecycles() {
    let device = DeviceLease::new();
    for _ in 0..64 {
        let lease = device.acquire(owner(1)).unwrap();
        let requests = core::array::from_fn::<_, TRANSPORT_SLOTS, _>(|block| {
            let op = device.read(&lease, block).unwrap();
            device.abandon(&op).unwrap();
            op
        });
        assert_eq!(device.snapshot().outstanding, 0);
        assert_eq!(device.snapshot().retained_data, TRANSPORT_SLOTS);
        assert!(matches!(device.read(&lease, 0), Err(Error::NoCredit)));
        for op in requests.iter().rev() {
            device.complete(op, Outcome::Success).unwrap();
        }
        let change = device.begin_release(&lease).unwrap();
        let flush = device.start_flush(&change).unwrap();
        device.complete_flush(&flush, Outcome::Success).unwrap();
        assert_eq!(device.finish(&change), Ok(None));
        assert_eq!(device.snapshot().retained_data, 0);
        assert!(!device.snapshot().flush_pending);
    }
}
