use super::*;
use std::{thread, vec};

#[test]
fn layout_and_initialization() {
    assert!(Ring::<0, 8>::new(1).is_err());
    assert!(Ring::<3, 8>::new(1).is_err());
    assert!(Header::for_config::<{ usize::MAX }, 8>(1).is_err());
    assert!(Header::for_config::<131072, 8>(1).is_err());
    assert!(Ring::<1, 0>::new(1).is_err());
    assert!(Ring::<1, 4097>::new(1).is_err());
    assert!(Ring::<1, 8>::new(0).is_err());
    let h = Header::for_config::<2, 7>(1).unwrap();
    for mutate in 0..11 {
        let mut bad = h;
        match mutate {
            0 => bad.magic = 0,
            1 => bad.version += 1,
            2 => bad.capacity = 3,
            3 => bad.payload_size += 1,
            4 => bad.slot_stride += 1,
            5 => bad.generation += 1,
            6 => bad.producer_offset += 1,
            7 => bad.consumer_offset += 1,
            8 => bad.payload_offset += 1,
            9 => bad.total_size += 1,
            _ => bad.capacity = 0,
        }
        assert!(bad.validate::<2, 7>(1).is_err());
    }
    assert_eq!(size_of::<Header>(), 64);
    assert_eq!(core::mem::offset_of!(Metadata, ready), 64);
    assert_eq!(core::mem::offset_of!(Ring<2, 7>, producer), 4096);
    assert_eq!(core::mem::offset_of!(Ring<2, 7>, consumer), 8192);
    assert_eq!(core::mem::offset_of!(Ring<2, 7>, payload), 12288);
    assert_eq!(size_of::<Slot<7>>(), 12);
    for _ in 0..128 {
        let mut ring = Ring::<2, 7>::new(1).unwrap();
        assert!(ring.split(2).is_err());
        ring.metadata.ready.store(0, Ordering::Relaxed); // cancelled trusted setup
        assert!(ring.split(1).is_err());
        ring.retire();
        assert!(ring.split(1).is_err());
    }
    assert!(Header::for_config::<65536, 4096>(1).is_ok());
}

fn exercise<const N: usize>() {
    let mut ring = Ring::<N, 64>::new(1).unwrap();
    let (mut p, mut c) = ring.split(1).unwrap();
    let mut out = [0xaa; 80];
    assert_eq!(c.try_recv(&mut out), Err(RecvError::Empty));
    assert_eq!(p.try_send(&[0; 65]), Err(SendError::PayloadTooLarge));
    for cycle in 0..100 {
        for i in 0..N {
            p.try_send(&[(i + cycle) as u8; 64]).unwrap();
        }
        assert_eq!(p.try_send(&[]), Err(SendError::Full));
        assert_eq!(
            c.try_recv(&mut out[..63]),
            Err(RecvError::OutputBufferTooSmall)
        );
        for i in 0..N {
            assert_eq!(c.try_recv(&mut out), Ok(64));
            assert_eq!(&out[..64], &[(i + cycle) as u8; 64]);
            assert_eq!(&out[64..], &[0xaa; 16]);
        }
        for len in [0, 1, 7, 64, 2] {
            p.try_send(&[0x42; 64][..len]).unwrap();
            assert_eq!(c.try_recv(&mut out), Ok(len));
            assert_eq!(&out[..len], &[0x42; 64][..len]);
        }
    }
}
#[test]
fn sizes_fifo_reuse_backpressure() {
    exercise::<1>();
    exercise::<2>();
    exercise::<8>();
    exercise::<256>();
}

#[test]
fn rollover() {
    let mut ring = Ring::<2, 8>::new(1).unwrap();
    let start = u64::MAX - 1;
    ring.producer.value.store(start, Ordering::Relaxed);
    ring.consumer.value.store(start, Ordering::Relaxed);
    assert_eq!(start.wrapping_sub(start), 0);
    let (mut p, mut c) = ring.split(1).unwrap();
    p.try_send(&[1]).unwrap();
    p.try_send(&[2]).unwrap();
    assert_eq!(p.try_send(&[]), Err(SendError::Full));
    let mut out = [0; 8];
    assert_eq!(c.try_recv(&mut out), Ok(1));
    assert_eq!(out[0], 1);
    p.try_send(&[3]).unwrap();
    for expected in [2, 3] {
        c.try_recv(&mut out).unwrap();
        assert_eq!(out[0], expected);
    }
    assert_eq!(c.try_recv(&mut out), Err(RecvError::Empty));
}
#[test]
fn corrupt_frame_and_cursor_fail_closed() {
    let mut ring = Ring::<2, 8>::new(1).unwrap();
    // Exclusive fixture access; no peer handles exist.
    unsafe {
        (*ring.payload.slots[0].get()).len = 9;
    }
    ring.producer.value.store(1, Ordering::Relaxed);
    {
        let (_, mut c) = ring.split(1).unwrap();
        for _ in 0..2 {
            assert_eq!(c.try_recv(&mut [0; 16]), Err(RecvError::InvalidFrame));
        }
    }
    assert_eq!(ring.consumer.value.load(Ordering::Relaxed), 0);
    ring.producer.value.store(3, Ordering::Relaxed);
    let (mut p, mut c) = ring.split(1).unwrap();
    assert_eq!(p.try_send(&[]), Err(SendError::InvalidChannel));
    assert_eq!(c.try_recv(&mut [0; 8]), Err(RecvError::InvalidChannel));
}

fn stress<const N: usize, const P: usize>(count: u64, slow_producer: bool) {
    let mut ring = Ring::<N, P>::new(1).unwrap();
    let (mut p, mut c) = ring.split(1).unwrap();
    thread::scope(|s| {
        s.spawn(move || {
            let mut bytes = vec![0; P];
            for seq in 0..count {
                for (i, b) in bytes.iter_mut().enumerate() {
                    *b = (seq.rotate_left((i % 64) as u32) as u8) ^ i as u8;
                }
                loop {
                    match p.try_send(&bytes) {
                        Ok(()) => break,
                        Err(SendError::Full) => thread::yield_now(),
                        err => panic!("{err:?}"),
                    }
                }
                if slow_producer && seq % 17 == 0 {
                    thread::yield_now();
                }
            }
        });
        let mut out = vec![0; P];
        for seq in 0..count {
            loop {
                match c.try_recv(&mut out) {
                    Ok(len) => {
                        assert_eq!(len, P);
                        break;
                    }
                    Err(RecvError::Empty) => thread::yield_now(),
                    err => panic!("{err:?}"),
                }
            }
            for (i, b) in out.iter().enumerate() {
                assert_eq!(*b, (seq.rotate_left((i % 64) as u32) as u8) ^ i as u8);
            }
            if !slow_producer && seq % 17 == 0 {
                thread::yield_now();
            }
        }
    });
}
#[test]
fn concurrent_stress() {
    stress::<1, 64>(10_000, true);
    stress::<2, 256>(100_000, false);
    stress::<64, 64>(1_000_000, true);
    stress::<8, 1024>(10_000, false);
    stress::<2, 4096>(10_000, true);
}
