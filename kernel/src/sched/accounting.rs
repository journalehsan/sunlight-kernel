//! Independent halt-time accounting. Writers are the local CPU with IRQs off;
//! readers use a seqlock and never acquire the scheduler lock. No NMI writers.
use super::{current_cpu_id, MAX_CORES};
use crate::arch::x86_64::interrupts::now_ns;
use core::sync::atomic::{AtomicU64, Ordering};
use sunlight_ipc::cpu_accounting::CpuCounters;

#[repr(align(64))]
struct IdleClock {
    sequence: AtomicU64,
    total: AtomicU64,
    start: AtomicU64,
    entries: AtomicU64,
    exits: AtomicU64,
}
impl IdleClock {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            total: AtomicU64::new(0),
            start: AtomicU64::new(0),
            entries: AtomicU64::new(0),
            exits: AtomicU64::new(0),
        }
    }
}
static IDLE: [IdleClock; MAX_CORES] = [const { IdleClock::new() }; MAX_CORES];

/// Called with maskable interrupts disabled immediately before STI;HLT.
pub fn enter_idle(cpu: usize) {
    let c = &IDLE[cpu];
    c.sequence.fetch_add(1, Ordering::AcqRel);
    c.start.store(now_ns().max(1), Ordering::Relaxed);
    c.entries.fetch_add(1, Ordering::Relaxed);
    c.sequence.fetch_add(1, Ordering::Release);
}

/// Interrupt entry closes the halt interval BEFORE locks or scheduler work.
/// A context switch need not ever return to the interrupted idle frame.
pub fn interrupt_entry(cpu: usize) {
    let c = &IDLE[cpu];
    let start = c.start.load(Ordering::Relaxed);
    if start == 0 {
        return;
    }
    c.sequence.fetch_add(1, Ordering::AcqRel);
    let total = c.total.load(Ordering::Relaxed);
    c.total.store(
        total.saturating_add(now_ns().saturating_sub(start)),
        Ordering::Relaxed,
    );
    c.start.store(0, Ordering::Relaxed);
    c.exits.fetch_add(1, Ordering::Relaxed);
    c.sequence.fetch_add(1, Ordering::Release);
}

pub fn interrupt_entry_local() {
    interrupt_entry(current_cpu_id());
}

pub fn snapshot(cpu: usize) -> Option<CpuCounters> {
    let c = &IDLE[cpu];
    for _ in 0..8 {
        let seq = c.sequence.load(Ordering::Acquire);
        if seq & 1 != 0 {
            core::hint::spin_loop();
            continue;
        }
        let total = c.total.load(Ordering::Relaxed);
        let start = c.start.load(Ordering::Relaxed);
        let entries = c.entries.load(Ordering::Relaxed);
        let exits = c.exits.load(Ordering::Relaxed);
        let now = now_ns();
        core::sync::atomic::fence(Ordering::Acquire);
        if seq != c.sequence.load(Ordering::Relaxed) {
            continue;
        }
        let idle = total
            .saturating_add(if start != 0 {
                now.saturating_sub(start)
            } else {
                0
            })
            .min(now);
        return Some(CpuCounters {
            elapsed_ns: now,
            busy_ns: now - idle,
            idle_ns: idle,
            idle_entries: entries,
            idle_exits: exits,
            ..CpuCounters::ZERO
        });
    }
    None
}

/// Common BSP/AP idle loop. Interrupt handlers close each interval; the
/// fallback after HLT covers vectors which do not have a dedicated hook.
pub fn idle_loop() -> ! {
    let cpu = current_cpu_id();
    loop {
        x86_64::instructions::interrupts::disable();
        enter_idle(cpu);
        x86_64::instructions::interrupts::enable_and_hlt();
        x86_64::instructions::interrupts::disable();
        interrupt_entry(cpu);
    }
}
