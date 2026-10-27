//! Bounded native-session test observations. No bearer values are stored.
//! Slots are reserved once and published with release ordering; the sequence is
//! a trace reservation order, not an independent operation linearization order.
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

const CAPACITY: usize = 256;
struct Slot {
    event: AtomicU64,
    cpu: AtomicU64,
    pid: AtomicU64,
    incarnation: AtomicU64,
    a: AtomicU64,
    b: AtomicU64,
    c: AtomicU64,
}
impl Slot {
    const fn new() -> Self {
        Self {
            event: AtomicU64::new(0),
            cpu: AtomicU64::new(0),
            pid: AtomicU64::new(0),
            incarnation: AtomicU64::new(0),
            a: AtomicU64::new(0),
            b: AtomicU64::new(0),
            c: AtomicU64::new(0),
        }
    }
}
static SLOTS: [Slot; CAPACITY] = [const { Slot::new() }; CAPACITY];
static NEXT: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static DUMPED: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "native_session_h4_test")]
pub(crate) fn enable() {
    ENABLED.store(true, Ordering::Release);
}

pub(crate) fn arm(case: u64, session: u64, epoch: u64, p: &crate::process::Process) {
    if !matches!(case, 4..=6 | 12..=15) || ARMED.swap(true, Ordering::AcqRel) {
        return;
    }
    ENABLED.store(true, Ordering::Release);
    record(
        1,
        p.pid as u64,
        p.foreground.incarnation,
        case,
        session,
        epoch,
    );
}

/// Called at a real boundary. Each slot has a unique writer and no lock, heap,
/// serial I/O, waiting, or mutation of production authorization state.
pub(crate) fn record(event: u64, pid: u64, incarnation: u64, a: u64, b: u64, c: u64) {
    if !ENABLED.load(Ordering::Acquire) {
        return;
    }
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    if n >= CAPACITY {
        return;
    }
    let s = &SLOTS[n];
    s.cpu
        .store(crate::sched::current_cpu_id() as u64, Ordering::Relaxed);
    s.pid.store(pid, Ordering::Relaxed);
    s.incarnation.store(incarnation, Ordering::Relaxed);
    s.a.store(a, Ordering::Relaxed);
    s.b.store(b, Ordering::Relaxed);
    s.c.store(c, Ordering::Relaxed);
    s.event.store(event, Ordering::Release);
}

/// Called by the existing unprivileged fixture's completion log, outside any
/// scheduler/broker/IPC guard. Holes and overflow remain visible to parsers.
pub(crate) fn dump() {
    if !ENABLED.load(Ordering::Acquire) || DUMPED.swap(true, Ordering::AcqRel) {
        return;
    }
    let count = NEXT.load(Ordering::Acquire);
    crate::serial_println!(
        "[H3-TRACE] schema=1 count={} capacity={} overflow={}",
        count,
        CAPACITY,
        u8::from(count > CAPACITY)
    );
    for (i, s) in SLOTS.iter().enumerate().take(count.min(CAPACITY)) {
        let event = s.event.load(Ordering::Acquire);
        crate::serial_println!(
            "[H3-TRACE] schema=1 seq={} event={} cpu={} pid={} inc={} a={} b={} c={}",
            i + 1,
            event,
            s.cpu.load(Ordering::Relaxed),
            s.pid.load(Ordering::Relaxed),
            s.incarnation.load(Ordering::Relaxed),
            s.a.load(Ordering::Relaxed),
            s.b.load(Ordering::Relaxed),
            s.c.load(Ordering::Relaxed)
        );
    }
}
