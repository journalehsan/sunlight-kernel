#!/usr/bin/env python3
"""F1.3N scheduler delivery audit. Host counterexamples, NOT native wake tests.

Compile the current production request/flush bodies with a controlled LAPIC
boundary. Hardware completion and native scheduling are deliberately unclaimed.
Run serially because the extracted production functions use global state.
"""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/fastlane-f13n-evidence'


def function(source, signature):
    start = source.index(signature)
    opening = source.index('{', start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    source = (ROOT / 'kernel/src/sched/mod.rs').read_text()
    extracted = '\n'.join(function(source, signature) for signature in [
        'fn reschedule_bit(', 'pub fn request_reschedule_on(',
        'pub(crate) fn flush_dispatch_notifications(',
    ])
    wrapper = r'''
#![deny(warnings)]
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[path = "REQUESTS_PATH"] mod requests;
static RESCHEDULE_MASK: requests::RescheduleRequests = requests::RescheduleRequests::new();
static DISPATCH_NOTIFICATION_MASK: AtomicU64 = AtomicU64::new(0);
static DISPATCH_NOTIFICATIONS_SENT: AtomicU64 = AtomicU64::new(0);
static DISPATCH_NOTIFICATIONS_BUSY: AtomicU64 = AtomicU64::new(0);
static BUSY: AtomicBool = AtomicBool::new(false);
static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static SENT: AtomicU64 = AtomicU64::new(0);
mod arch { pub mod x86_64 {
    pub mod interrupts { pub const RESCHEDULE_VECTOR: u8 = 0xf3; }
    pub mod lapic {
        pub unsafe fn try_send_fixed_ipi(cpu: usize, vector: u8) -> bool {
            assert_eq!(vector, 0xf3);
            crate::ATTEMPTS.fetch_add(1, crate::Ordering::Relaxed);
            if crate::BUSY.load(crate::Ordering::Acquire) { return false; }
            crate::SENT.fetch_or(1u64 << cpu, crate::Ordering::Release);
            true
        }
    }
}}
EXTRACTED
fn reset() {
    for cpu in 0..64 { RESCHEDULE_MASK.take(cpu); }
    DISPATCH_NOTIFICATION_MASK.store(0, Ordering::Release);
    BUSY.store(false, Ordering::Release);
    ATTEMPTS.store(0, Ordering::Release);
    SENT.store(0, Ordering::Release);
}
#[test]
fn boundary_request_only_does_not_deliver_interrupt() {
    reset();
    request_reschedule_on(1);
    flush_dispatch_notifications();
    assert!(RESCHEDULE_MASK.pending(1));
    assert_eq!(ATTEMPTS.load(Ordering::Acquire), 0);
    assert_eq!(SENT.load(Ordering::Acquire), 0);
    println!("F13N-BOUNDARY request-only cpu=1 request=retained attempts=0 delivery=absent");
}
#[test]
fn boundary_busy_icr_retains_work_without_autonomous_retry() {
    reset();
    request_reschedule_on(11);
    DISPATCH_NOTIFICATION_MASK.store(1 << 11, Ordering::Release);
    BUSY.store(true, Ordering::Release);
    flush_dispatch_notifications();
    assert!(RESCHEDULE_MASK.pending(11));
    assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 1 << 11);
    assert_eq!(ATTEMPTS.load(Ordering::Acquire), 1);
    assert_eq!(SENT.load(Ordering::Acquire), 0);
    // Hardware becomes available. No timer/IRQ/syscall tail is invoked.
    BUSY.store(false, Ordering::Release);
    assert_eq!(ATTEMPTS.load(Ordering::Acquire), 1);
    assert_eq!(SENT.load(Ordering::Acquire), 0);
    println!("F13N-BOUNDARY busy-icr cpu=11 request=retained deferred=retained delivery=absent");
    // A subsequent *external* flush succeeds. This is retention, not liveness.
    flush_dispatch_notifications();
    assert_eq!(SENT.load(Ordering::Acquire), 1 << 11);
    assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 0);
    assert!(RESCHEDULE_MASK.pending(11));
    println!("F13N-BOUNDARY later-external-flush cpu=11 send=accepted execution=unobserved");
}
#[test]
fn boundary_coalesced_delivery_preserves_remote_request() {
    reset();
    for _ in 0..8 {
        request_reschedule_on(1);
        request_reschedule_on(11);
        DISPATCH_NOTIFICATION_MASK.fetch_or((1 << 1) | (1 << 11), Ordering::Release);
    }
    flush_dispatch_notifications();
    assert_eq!(ATTEMPTS.load(Ordering::Acquire), 2);
    assert_eq!(SENT.load(Ordering::Acquire), (1 << 1) | (1 << 11));
    assert!(RESCHEDULE_MASK.take(1));
    assert!(RESCHEDULE_MASK.pending(11));
    assert!(!RESCHEDULE_MASK.take(1));
    println!("F13N-BOUNDARY coalesced targets=1,11 attempts=2 remote-request=retained");
}
'''.replace('REQUESTS_PATH', str(ROOT / 'kernel/src/sched/requests.rs')).replace('EXTRACTED', extracted)
    generated = OUT / 'delivery-boundary.rs'
    generated.write_text(wrapper)
    binary = OUT / 'delivery-boundary'
    subprocess.run(['rustc', '--edition=2021', '--test', str(generated), '-o', str(binary)],
                   cwd=ROOT, check=True, timeout=60)
    subprocess.run([str(binary), '--test-threads=1', '--nocapture'],
                   cwd=ROOT, check=True, timeout=60)


if __name__ == '__main__':
    main()
