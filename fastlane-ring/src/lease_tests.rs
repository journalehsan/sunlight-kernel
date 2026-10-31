use super::*;
use std::{panic::AssertUnwindSafe, sync::mpsc, thread, time::Duration};

#[test]
fn leases_borrow_the_same_slot_and_preserve_layout() {
    let mut ring = Ring::<2, 17>::new(7).unwrap();
    let header = ring.header();
    let slot = ring.payload.slots[0].get();
    // SAFETY: form only an address, with no dereference/read of the payload.
    let expected = unsafe { core::ptr::addr_of!((*slot).bytes).cast::<u8>() };
    let (mut producer, mut consumer) = ring.split(7).unwrap();
    let mut write = producer.try_reserve(17).unwrap();
    assert_eq!(write.payload_mut().as_ptr(), expected);
    assert_eq!(write.payload_mut(), &[0; 17]);
    write.payload_mut().copy_from_slice(b"direct ring bytes");
    assert!(matches!(consumer.try_read(), Err(RecvError::Empty)));
    write.commit();
    let read = consumer.try_read().unwrap();
    assert_eq!(read.payload().as_ptr(), expected);
    assert_eq!(read.payload(), b"direct ring bytes");
    read.release();
    assert!(matches!(consumer.try_read(), Err(RecvError::Empty)));
    assert_eq!(producer.ring.header(), header);
    assert_eq!(size_of::<Ring<2, 17>>() as u64, header.total_size);
}

#[test]
fn cancelled_and_reused_storage_never_exposes_old_bytes_through_leases() {
    let mut ring = Ring::<1, 64>::new(1).unwrap();
    let (mut producer, mut consumer) = ring.split(1).unwrap();
    for len in (0..=64).rev() {
        producer.try_send(&[0xdd; 64]).unwrap();
        consumer.try_read().unwrap().release();
        {
            let mut write = producer.try_reserve(len).unwrap();
            assert_eq!(write.payload_mut(), &[0; 64][..len]);
            write.payload_mut().fill(0xee);
        } // cancelled, including when there were zero bytes
        assert!(matches!(consumer.try_read(), Err(RecvError::Empty)));
        let mut write = producer.try_reserve(len).unwrap();
        assert_eq!(write.payload_mut(), &[0; 64][..len]);
        if len > 0 {
            write.payload_mut()[0] = 0x42; // leave the rest deliberately unwritten
        }
        write.commit();
        let read = consumer.try_read().unwrap();
        assert_eq!(read.payload().len(), len);
        if len > 0 {
            assert_eq!(read.payload()[0], 0x42);
            assert_eq!(&read.payload()[1..], &[0; 64][..len - 1]);
        }
        read.release();
    }
}

#[test]
#[expect(
    clippy::forget_non_drop,
    reason = "Intentionally test the safe mem::forget contract of borrowed role leases"
)]
fn panic_and_forget_do_not_publish_or_reclaim() {
    let mut ring = Ring::<1, 8>::new(1).unwrap();
    let (mut producer, mut consumer) = ring.split(1).unwrap();
    assert!(std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut write = producer.try_reserve(8).unwrap();
        write.payload_mut().fill(0xff);
        panic!("cancel reservation");
    }))
    .is_err());
    assert!(matches!(consumer.try_read(), Err(RecvError::Empty)));
    core::mem::forget(producer.try_reserve(8).unwrap());
    assert!(matches!(consumer.try_read(), Err(RecvError::Empty)));
    producer.try_reserve(8).unwrap().commit();
    assert!(std::panic::catch_unwind(AssertUnwindSafe(|| {
        let read = consumer.try_read().unwrap();
        assert_eq!(read.payload(), &[0; 8]);
        panic!("retain message");
    }))
    .is_err());
    core::mem::forget(consumer.try_read().unwrap());
    assert!(matches!(producer.try_reserve(1), Err(SendError::Full)));
    let read = consumer.try_read().unwrap();
    assert_eq!(read.payload(), &[0; 8]);
    read.release();
    producer.try_send(b"next").unwrap();
    let mut out = [0xaa; 8];
    assert_eq!(consumer.try_recv(&mut out), Ok(4));
    assert_eq!(&out, b"next\xaa\xaa\xaa\xaa");
}

#[test]
fn copying_and_leases_interoperate_without_consuming_on_drop_or_short_output() {
    let mut ring = Ring::<2, 8>::new(1).unwrap();
    let (mut producer, mut consumer) = ring.split(1).unwrap();
    assert!(matches!(
        producer.try_reserve(9),
        Err(SendError::PayloadTooLarge)
    ));
    producer.try_send(b"copy").unwrap();
    let mut write = producer.try_reserve(5).unwrap();
    write.payload_mut().copy_from_slice(b"lease");
    write.commit();
    assert!(matches!(producer.try_reserve(0), Err(SendError::Full)));
    {
        let read = consumer.try_read().unwrap();
        assert_eq!(read.payload(), b"copy");
    }
    let mut short = [0x55; 3];
    assert_eq!(
        consumer.try_recv(&mut short),
        Err(RecvError::OutputBufferTooSmall)
    );
    assert_eq!(short, [0x55; 3]);
    let mut out = [0xaa; 8];
    assert_eq!(consumer.try_recv(&mut out), Ok(4));
    assert_eq!(&out, b"copy\xaa\xaa\xaa\xaa");
    let read = consumer.try_read().unwrap();
    assert_eq!(read.payload(), b"lease");
    read.release();
}

#[test]
fn leases_fail_closed_on_invalid_lengths_and_cursor_distances() {
    let mut ring = Ring::<2, 8>::new(1).unwrap();
    // SAFETY: exclusive fixture ownership, before any role is borrowed.
    unsafe { (*ring.payload.slots[0].get()).len = 9 };
    ring.producer.value.store(1, Ordering::Relaxed);
    {
        let (_, mut consumer) = ring.split(1).unwrap();
        assert!(matches!(consumer.try_read(), Err(RecvError::InvalidFrame)));
        assert!(matches!(consumer.try_read(), Err(RecvError::InvalidFrame)));
    }
    assert_eq!(ring.consumer.value.load(Ordering::Relaxed), 0);
    ring.producer.value.store(3, Ordering::Relaxed);
    let (mut producer, mut consumer) = ring.split(1).unwrap();
    assert!(matches!(
        producer.try_reserve(1),
        Err(SendError::InvalidChannel)
    ));
    assert!(matches!(
        consumer.try_read(),
        Err(RecvError::InvalidChannel)
    ));
}

#[test]
fn held_read_lease_blocks_reuse_while_other_thread_makes_progress() {
    let mut ring = Ring::<1, 8>::new(1).unwrap();
    let (mut producer, mut consumer) = ring.split(1).unwrap();
    producer.try_send(b"original").unwrap();
    let read = consumer.try_read().unwrap();
    let (progress_tx, progress_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    thread::scope(|scope| {
        scope.spawn(move || {
            for _ in 0..10_000 {
                assert!(matches!(producer.try_reserve(8), Err(SendError::Full)));
            }
            progress_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let mut write = producer.try_reserve(8).unwrap();
            write.payload_mut().copy_from_slice(b"replaced");
            write.commit();
        });
        progress_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(read.payload(), b"original");
        read.release();
        release_tx.send(()).unwrap();
    });
    let read = consumer.try_read().unwrap();
    assert_eq!(read.payload(), b"replaced");
    read.release();
}

fn pattern(seq: u64, i: usize) -> u8 {
    (seq.rotate_left((i % 64) as u32) as u8) ^ i as u8
}

fn stress<const N: usize, const P: usize>(count: u64, start: u64) {
    let mut ring = Ring::<N, P>::new(1).unwrap();
    ring.producer.value.store(start, Ordering::Relaxed);
    ring.consumer.value.store(start, Ordering::Relaxed);
    let (mut producer, mut consumer) = ring.split(1).unwrap();
    // A panic aborts the opposite spin loop, so a data failure cannot hang it.
    let stopped = core::sync::atomic::AtomicBool::new(false);
    struct StopOnDrop<'a>(&'a core::sync::atomic::AtomicBool);
    impl Drop for StopOnDrop<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    thread::scope(|scope| {
        let stopped_ref = &stopped;
        scope.spawn(move || {
            let _stop = StopOnDrop(stopped_ref);
            for seq in 0..count {
                // Includes empty, short, full and varying lengths on slot reuse.
                let len = seq as usize % (P + 1);
                loop {
                    match producer.try_reserve(len) {
                        Ok(mut write) => {
                            for (i, byte) in write.payload_mut().iter_mut().enumerate() {
                                *byte = pattern(seq, i);
                            }
                            if seq % 257 == 0 {
                                thread::yield_now(); // consumer must not see this slot yet
                            }
                            write.commit();
                            break;
                        }
                        Err(SendError::Full) => {
                            assert!(!stopped_ref.load(Ordering::Acquire));
                            thread::yield_now();
                        }
                        error => panic!("unexpected reservation result: {:?}", error.err()),
                    }
                }
            }
        });
        let _stop = StopOnDrop(&stopped);
        for seq in 0..count {
            loop {
                match consumer.try_read() {
                    Ok(read) => {
                        assert_eq!(read.payload().len(), seq as usize % (P + 1));
                        for (i, byte) in read.payload().iter().enumerate() {
                            assert_eq!(*byte, pattern(seq, i), "seq={seq}, byte={i}");
                        }
                        if seq % 263 == 0 {
                            thread::yield_now(); // writer must not reuse while borrowed
                        }
                        read.release();
                        break;
                    }
                    Err(RecvError::Empty) => {
                        // The producer may finish after our first Empty read.
                        // Acquire normal completion before rechecking publication.
                        if stopped.load(Ordering::Acquire) {
                            let read = consumer.try_read().expect("producer ended too early");
                            assert_eq!(read.payload().len(), seq as usize % (P + 1));
                            for (i, byte) in read.payload().iter().enumerate() {
                                assert_eq!(*byte, pattern(seq, i));
                            }
                            read.release();
                            break;
                        }
                        thread::yield_now();
                    }
                    error => panic!("unexpected read result: {:?}", error.err()),
                }
            }
        }
        assert!(matches!(consumer.try_read(), Err(RecvError::Empty)));
    });
}

#[test]
fn concurrent_zero_copy_stress_and_rollover() {
    stress::<1, 64>(10_000, u64::MAX - 8);
    stress::<2, 17>(100_000, u64::MAX - 32);
    stress::<64, 64>(1_000_000, 0);
    stress::<8, 1024>(10_000, 0);
    stress::<2, 4096>(10_000, 0);
}
