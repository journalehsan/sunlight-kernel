//! Opt-in, bounded scheduler performance counters. Writers update only the
//! record for the CPU executing the scheduler path. Snapshots use atomic loads;
//! reset uses swaps and never takes a scheduler/core lock.
use core::sync::atomic::{AtomicU64, Ordering};

pub const HIST_BUCKETS: usize = 16;
/// Upper bounds in nanoseconds: 1,2,4,...,32768; final bucket is >32768.
pub const HIST_UPPER_NS: [u64; HIST_BUCKETS] = [
    1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1_024, 2_048, 4_096, 8_192, 16_384, 32_768,
];

#[repr(align(64))]
pub struct CpuRecord {
    pub invocations: AtomicU64,
    pub dispatches: AtomicU64,
    pub switches: AtomicU64,
    pub requests: AtomicU64,
    pub rearms: AtomicU64,
    pub enqueues: AtomicU64,
    pub dequeues: AtomicU64,
    pub steals: AtomicU64,
    pub migrations: AtomicU64,
    pub idle_enter: AtomicU64,
    pub idle_exit: AtomicU64,
    pub queue_depth: AtomicU64,
    pub queue_high_water: AtomicU64,
    pub nice_dispatches: [AtomicU64; 3],
    pub nice_wait_max_ns: [AtomicU64; 3],
    pub runnable_wait: [AtomicU64; HIST_BUCKETS],
    pub lock_wait: [AtomicU64; HIST_BUCKETS],
    pub lock_hold: [AtomicU64; HIST_BUCKETS],
}

impl CpuRecord {
    pub const fn new() -> Self {
        Self {
            invocations: AtomicU64::new(0),
            dispatches: AtomicU64::new(0),
            switches: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            rearms: AtomicU64::new(0),
            enqueues: AtomicU64::new(0),
            dequeues: AtomicU64::new(0),
            steals: AtomicU64::new(0),
            migrations: AtomicU64::new(0),
            idle_enter: AtomicU64::new(0),
            idle_exit: AtomicU64::new(0),
            queue_depth: AtomicU64::new(0),
            queue_high_water: AtomicU64::new(0),
            nice_dispatches: [const { AtomicU64::new(0) }; 3],
            nice_wait_max_ns: [const { AtomicU64::new(0) }; 3],
            runnable_wait: [const { AtomicU64::new(0) }; HIST_BUCKETS],
            lock_wait: [const { AtomicU64::new(0) }; HIST_BUCKETS],
            lock_hold: [const { AtomicU64::new(0) }; HIST_BUCKETS],
        }
    }
}

pub static CPU: [CpuRecord; super::MAX_CORES] = [const { CpuRecord::new() }; super::MAX_CORES];
static CLOCK_LAST_NS: [AtomicU64; super::MAX_CORES] =
    [const { AtomicU64::new(0) }; super::MAX_CORES];
static CLOCK_STATE: [core::sync::atomic::AtomicU8; super::MAX_CORES] =
    [const { core::sync::atomic::AtomicU8::new(0) }; super::MAX_CORES];

/// Observe one local calibrated-clock sample. Returns true only after two
/// nondecreasing samples and remains false permanently if a regression occurs.
pub fn observe_clock(cpu: usize, ns: u64) -> bool {
    if cpu >= super::MAX_CORES || crate::arch::x86_64::interrupts::tsc_hz() == 0 {
        return false;
    }
    let state = &CLOCK_STATE[cpu];
    if state.load(Ordering::Relaxed) == 2 {
        return false;
    }
    let previous = CLOCK_LAST_NS[cpu].load(Ordering::Relaxed);
    if previous != 0 && ns < previous {
        state.store(2, Ordering::Release);
        return false;
    }
    CLOCK_LAST_NS[cpu].store(ns, Ordering::Relaxed);
    if previous != 0 {
        state.store(1, Ordering::Release);
    }
    state.load(Ordering::Acquire) == 1
}

#[derive(Clone)]
pub struct Snapshot {
    pub invocations: u64,
    pub dispatches: u64,
    pub switches: u64,
    pub requests: u64,
    pub rearms: u64,
    pub enqueues: u64,
    pub dequeues: u64,
    pub steals: u64,
    pub migrations: u64,
    pub idle_enter: u64,
    pub idle_exit: u64,
    pub queue_depth: u64,
    pub queue_high_water: u64,
    pub nice_dispatches: [u64; 3],
    pub nice_wait_max_ns: [u64; 3],
    pub runnable_wait: [u64; HIST_BUCKETS],
    pub lock_wait: [u64; HIST_BUCKETS],
    pub lock_hold: [u64; HIST_BUCKETS],
}

/// Read or reset one CPU's record without acquiring any scheduler locks. A
/// reset is field-wise and concurrent updates may land on either side of it.
pub fn snapshot(cpu: usize, reset: bool) -> Option<Snapshot> {
    let r = CPU.get(cpu)?;
    let arr = |src: &[AtomicU64; HIST_BUCKETS]| core::array::from_fn(|i| read(&src[i], reset));
    Some(Snapshot {
        invocations: read(&r.invocations, reset),
        dispatches: read(&r.dispatches, reset),
        switches: read(&r.switches, reset),
        requests: read(&r.requests, reset),
        rearms: read(&r.rearms, reset),
        enqueues: read(&r.enqueues, reset),
        dequeues: read(&r.dequeues, reset),
        steals: read(&r.steals, reset),
        migrations: read(&r.migrations, reset),
        idle_enter: read(&r.idle_enter, reset),
        idle_exit: read(&r.idle_exit, reset),
        queue_depth: read(&r.queue_depth, reset),
        queue_high_water: read(&r.queue_high_water, reset),
        nice_dispatches: core::array::from_fn(|i| read(&r.nice_dispatches[i], reset)),
        nice_wait_max_ns: core::array::from_fn(|i| read(&r.nice_wait_max_ns[i], reset)),
        runnable_wait: arr(&r.runnable_wait),
        lock_wait: arr(&r.lock_wait),
        lock_hold: arr(&r.lock_hold),
    })
}

#[inline]
fn bucket(ns: u64) -> usize {
    let mut i = 0;
    while i + 1 < HIST_BUCKETS && ns > HIST_UPPER_NS[i] {
        i += 1;
    }
    i
}

#[inline]
pub fn add(counter: &AtomicU64, amount: u64) {
    let _ = counter.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
        Some(v.saturating_add(amount))
    });
}

#[inline]
pub fn sample(hist: &[AtomicU64; HIST_BUCKETS], ns: u64) {
    add(&hist[bucket(ns)], 1);
}

/// Snapshot/reset primitive. Reset returns a consistent value per field, not
/// an instantaneous cross-CPU snapshot; it never acquires scheduler locks.
pub fn read(counter: &AtomicU64, reset: bool) -> u64 {
    if reset {
        counter.swap(0, Ordering::AcqRel)
    } else {
        counter.load(Ordering::Relaxed)
    }
}
