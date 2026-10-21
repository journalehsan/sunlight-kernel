//! Host ring measurements only; no native IPC or notification costs.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Instant,
};
use sunlight_fastlane_ring::{RecvError, Ring, SendError};
struct CountAlloc;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static ALLOC: CountAlloc = CountAlloc;
fn run<const N: usize, const P: usize>() {
    const COUNT: usize = 100_000;
    let mut ring = Ring::<N, P>::new(1).unwrap();
    let (mut p, mut c) = ring.split(1).unwrap();
    let data = [42; P];
    let mut out = [0; P];
    for _ in 0..10_000 {
        p.try_send(&data).unwrap();
        c.try_recv(&mut out).unwrap();
    }
    let mut samples = Vec::with_capacity(COUNT);
    let before = ALLOCS.load(Ordering::Relaxed);
    let start = Instant::now();
    for _ in 0..COUNT {
        p.try_send(black_box(&data)).unwrap();
        black_box(c.try_recv(black_box(&mut out)).unwrap());
    }
    let elapsed = start.elapsed();
    let allocations = ALLOCS.load(Ordering::Relaxed) - before;
    for _ in 0..COUNT {
        let t = Instant::now();
        p.try_send(black_box(&data)).unwrap();
        c.try_recv(black_box(&mut out)).unwrap();
        samples.push(t.elapsed().as_nanos());
    }
    samples.sort_unstable();
    let start = Instant::now();
    for _ in 0..COUNT {
        assert_eq!(black_box(c.try_recv(&mut out)), Err(RecvError::Empty));
    }
    let empty = start.elapsed().as_nanos() as f64 / COUNT as f64;
    for _ in 0..N {
        p.try_send(&data).unwrap();
    }
    let start = Instant::now();
    for _ in 0..COUNT {
        assert_eq!(black_box(p.try_send(&data)), Err(SendError::Full));
    }
    let full = start.elapsed().as_nanos() as f64 / COUNT as f64;
    for _ in 0..N {
        c.try_recv(&mut out).unwrap();
    }
    let rate = COUNT as f64 / elapsed.as_secs_f64();
    println!("single capacity={N} payload={P} messages_s={rate:.0} payload_bytes_s={:.0} empty_ns={empty:.2} full_ns={full:.2} pair_p50_ns={} pair_p95_ns={} pair_p99_ns={} steady_allocations={allocations}", rate * P as f64, samples[COUNT/2], samples[COUNT*95/100], samples[COUNT*99/100]);
    assert_eq!(allocations, 0);
    // Persistent threads: warmup, then barrier excludes thread creation from timer.
    let barrier = std::sync::Barrier::new(2);
    thread::scope(|s| {
        let b = &barrier;
        s.spawn(move || {
            for round in 0..2 {
                b.wait();
                for _ in 0..COUNT {
                    loop {
                        match p.try_send(black_box(&data)) {
                            Ok(()) => break,
                            Err(SendError::Full) => std::hint::spin_loop(),
                            e => panic!("{e:?}"),
                        }
                    }
                }
                b.wait();
                if round == 0 {
                    b.wait();
                }
            }
        });
        for round in 0..2 {
            barrier.wait();
            let start = Instant::now();
            for _ in 0..COUNT {
                loop {
                    match c.try_recv(black_box(&mut out)) {
                        Ok(_) => break,
                        Err(RecvError::Empty) => std::hint::spin_loop(),
                        e => panic!("{e:?}"),
                    }
                }
            }
            barrier.wait();
            if round == 0 {
                barrier.wait();
            } else {
                let rate = COUNT as f64 / start.elapsed().as_secs_f64();
                println!("two_thread capacity={N} payload={P} messages_s={rate:.0} payload_bytes_s={:.0}", rate * P as f64);
            }
        }
    });
}
fn main() {
    run::<2, 64>();
    run::<64, 64>();
    run::<2, 256>();
    run::<64, 256>();
    run::<2, 1024>();
    run::<64, 1024>();
    run::<2, 4096>();
    run::<64, 4096>();
}
