//! Opt-in bounded observations. No tokens, pointers retained, or extra locks.
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
static EVENTS: AtomicU64 = AtomicU64::new(0);
static ACTIVE: AtomicBool = AtomicBool::new(false);
const LIMIT: u64 = 4096;

pub(crate) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

pub(crate) fn event(kind: &str, generation: u64, pid: usize, inc: u64, vm: u64, a: u64, b: u64) {
    ACTIVE.store(true, Ordering::Relaxed);
    let n = EVENTS.fetch_add(1, Ordering::Relaxed);
    if n < LIMIT {
        crate::serial_println!("[FL-BELL-TRACE] event={} id={} cpu={} generation={} pid={} inc={} vm={} a={:#x} b={:#x}",
            kind, n, super::current_cpu_id(), generation, pid, inc, vm, a, b);
    } else if n == LIMIT {
        crate::serial_println!("[FL-BELL-TRACE] event=overflow limit={}", LIMIT);
    }
}

impl super::Scheduler {
    pub(crate) fn trace_bell_process(&self, kind: &str, i: usize, a: u64, b: u64) {
        let p = &self.processes[i];
        if let Some(r) = p.fastlane_bell {
            event(
                kind,
                r.handle.generation,
                p.pid,
                r.who.incarnation,
                r.who.vm_generation,
                a,
                b,
            );
        }
    }
}
