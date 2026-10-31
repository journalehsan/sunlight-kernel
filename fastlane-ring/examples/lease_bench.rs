//! Host threads only. These measurements do not establish native IPC speedup.
//! Both paths generate and check every payload byte; leases include clearing.
use std::{hint::black_box, sync::Barrier, thread, time::Instant};
use sunlight_fastlane_ring::{Consumer, Producer, RecvError, Ring, SendError};

fn pattern(sequence: u64, index: usize) -> u8 {
    (sequence.rotate_left((index % 64) as u32) as u8) ^ index as u8
}

fn generate(bytes: &mut [u8], sequence: u64) {
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = pattern(sequence, index);
    }
}

fn verify(bytes: &[u8], sequence: u64) {
    for (index, byte) in bytes.iter().enumerate() {
        assert_eq!(*byte, pattern(sequence, index), "payload integrity failure");
    }
    black_box(bytes);
}

fn send<const N: usize, const P: usize, const LEASES: bool>(
    producer: &mut Producer<'_, N, P>,
    sequence: u64,
    scratch: &[u8; P],
) -> Result<(), SendError> {
    if LEASES {
        let mut write = producer.try_reserve(P)?;
        generate(black_box(write.payload_mut()), sequence);
        write.commit();
        Ok(())
    } else {
        producer.try_send(black_box(scratch))
    }
}

fn recv<const N: usize, const P: usize, const LEASES: bool>(
    consumer: &mut Consumer<'_, N, P>,
    sequence: u64,
    scratch: &mut [u8; P],
) -> Result<(), RecvError> {
    if LEASES {
        let read = consumer.try_read()?;
        assert_eq!(read.payload().len(), P);
        verify(read.payload(), sequence);
        read.release();
    } else {
        assert_eq!(consumer.try_recv(black_box(scratch))?, P);
        verify(scratch, sequence);
    }
    Ok(())
}

fn run<const N: usize, const P: usize, const LEASES: bool>() {
    const WARMUP: u64 = 10_000;
    const COUNT: u64 = 100_000;
    let mode = if LEASES { "leases" } else { "copy" };
    let mut ring = Ring::<N, P>::new(1).unwrap();
    let (mut producer, mut consumer) = ring.split(1).unwrap();
    let mut input = [0; P];
    let mut output = [0; P];
    for sequence in 0..WARMUP {
        if !LEASES {
            generate(&mut input, sequence);
        }
        send::<N, P, LEASES>(&mut producer, sequence, &input).unwrap();
        recv::<N, P, LEASES>(&mut consumer, sequence, &mut output).unwrap();
    }

    // Pair latency includes generation, validation and Instant clock overhead.
    // It is a single-thread operation pair, never an inter-process RTT.
    let mut samples = Vec::with_capacity(COUNT as usize);
    for sequence in 0..COUNT {
        let start = Instant::now();
        if !LEASES {
            generate(&mut input, sequence);
        }
        send::<N, P, LEASES>(&mut producer, sequence, &input).unwrap();
        recv::<N, P, LEASES>(&mut consumer, sequence, &mut output).unwrap();
        samples.push(start.elapsed().as_nanos());
    }
    samples.sort_unstable();

    // Persistent threads and two rounds separate warmup from steady transfer.
    // The main thread times between start/completion barriers; no thread create
    // or join is in that interval. Barriers and scheduling are still included.
    let barrier = Barrier::new(3);
    let (elapsed, full_retries, empty_retries) = thread::scope(|scope| {
        let producer = scope.spawn(|| {
            let mut full_retries = 0u64;
            for count in [WARMUP, COUNT] {
                barrier.wait();
                for sequence in 0..count {
                    if !LEASES {
                        generate(&mut input, sequence);
                    }
                    loop {
                        match send::<N, P, LEASES>(&mut producer, sequence, &input) {
                            Ok(()) => break,
                            Err(SendError::Full) => {
                                full_retries += 1;
                                std::hint::spin_loop();
                            }
                            other => panic!("{other:?}"),
                        }
                    }
                }
                barrier.wait();
                if count == WARMUP {
                    full_retries = 0;
                }
            }
            full_retries
        });
        let consumer = scope.spawn(|| {
            let mut empty_retries = 0u64;
            for count in [WARMUP, COUNT] {
                barrier.wait();
                for sequence in 0..count {
                    loop {
                        match recv::<N, P, LEASES>(&mut consumer, sequence, &mut output) {
                            Ok(()) => break,
                            Err(RecvError::Empty) => {
                                empty_retries += 1;
                                std::hint::spin_loop();
                            }
                            other => panic!("{other:?}"),
                        }
                    }
                }
                barrier.wait();
                if count == WARMUP {
                    empty_retries = 0;
                }
            }
            empty_retries
        });
        barrier.wait();
        barrier.wait();
        let start = Instant::now();
        barrier.wait();
        barrier.wait();
        let elapsed = start.elapsed();
        (elapsed, producer.join().unwrap(), consumer.join().unwrap())
    });
    let rate = COUNT as f64 / elapsed.as_secs_f64();
    println!(
        "{{\"scope\":\"host_threads\",\"mode\":\"{mode}\",\"capacity\":{N},\"payload_bytes\":{P},\"messages\":{COUNT},\"elapsed_ns\":{},\"messages_per_second\":{rate:.3},\"payload_bytes_per_second\":{:.3},\"single_thread_pair_p50_ns\":{},\"single_thread_pair_p95_ns\":{},\"single_thread_pair_p99_ns\":{},\"full_retries\":{full_retries},\"empty_retries\":{empty_retries},\"integrity\":\"PASS\"}}",
        elapsed.as_nanos(), rate * P as f64,
        samples[COUNT as usize / 2], samples[COUNT as usize * 95 / 100],
        samples[COUNT as usize * 99 / 100],
    );
}

fn main() {
    run::<2, 64, false>();
    run::<2, 64, true>();
    run::<64, 64, false>();
    run::<64, 64, true>();
    run::<2, 256, false>();
    run::<2, 256, true>();
    run::<64, 256, false>();
    run::<64, 256, true>();
    run::<2, 1024, false>();
    run::<2, 1024, true>();
    run::<64, 1024, false>();
    run::<64, 1024, true>();
    run::<2, 4096, false>();
    run::<2, 4096, true>();
    run::<64, 4096, false>();
    run::<64, 4096, true>();
}
