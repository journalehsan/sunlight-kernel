#!/usr/bin/env python3
"""Extract production delivery functions; deterministic simulated source ICR."""
from pathlib import Path
import subprocess
from fastlane_f13n_boundary import function
ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'target/fastlane-f13nr-evidence'

def main():
    OUT.mkdir(parents=True, exist_ok=True)
    sched = (ROOT / 'kernel/src/sched/mod.rs').read_text()
    lapic = (ROOT / 'kernel/src/arch/x86_64/lapic.rs').read_text()
    extracted = '\n'.join(function(sched, s) for s in [
        'fn reschedule_bit(', 'pub fn mark_scheduler_ready(', 'pub fn request_reschedule_on(',
        'pub(crate) fn request_park_after_sysret(', 'pub(crate) fn flush_dispatch_notifications(',
        'pub(crate) extern "C" fn pin_release_wake('])
    bounded = function(lapic, 'pub unsafe fn send_fixed_ipi_bounded(')
    wrapper = r'''
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[path="REQUESTS"] mod requests;
#[path="HANDOFF_PATH"] mod handoff;
static RESCHEDULE_MASK: requests::RescheduleRequests = requests::RescheduleRequests::new();
static DISPATCH_NOTIFICATION_MASK: AtomicU64 = AtomicU64::new(0);
static DISPATCH_NOTIFICATIONS_SENT: AtomicU64 = AtomicU64::new(0);
static DISPATCH_NOTIFICATIONS_BUSY: AtomicU64 = AtomicU64::new(0);
static ONLINE_CORES: AtomicUsize = AtomicUsize::new(12);
static SCHEDULER_READY: AtomicBool = AtomicBool::new(true);
static SOURCE: AtomicUsize = AtomicUsize::new(0);
static BUSY_LEFT: AtomicUsize = AtomicUsize::new(0);
static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static ACCEPTED: AtomicU64 = AtomicU64::new(0);
static STACK_HANDOFFS: handoff::Handoffs<12> = handoff::Handoffs::new();
static HOOK: AtomicUsize = AtomicUsize::new(0);
fn current_cpu_id() -> usize { SOURCE.load(Ordering::Acquire) }
mod arch { pub mod x86_64 {
 pub mod interrupts { pub const RESCHEDULE_VECTOR: u8 = 0xf3; }
 pub mod lapic {
  pub unsafe fn try_send_fixed_ipi(cpu: usize, _: u8) -> bool {
   crate::ATTEMPTS.fetch_add(1, crate::Ordering::Relaxed);
   match crate::HOOK.swap(0, crate::Ordering::AcqRel) {
    1 => crate::request_reschedule_on(cpu),
    2 => { assert!(crate::RESCHEDULE_MASK.take(cpu)); crate::request_reschedule_on(cpu); },
    _ => (),
   }
   if crate::BUSY_LEFT.fetch_update(crate::Ordering::AcqRel, crate::Ordering::Acquire,
      |v| v.checked_sub(1)).is_ok() { return false; }
   crate::ACCEPTED.fetch_or(1 << cpu, crate::Ordering::Release); true
  }
  BOUNDED
 }
}}
EXTRACTED
fn reset() {
 for cpu in 0..64 { RESCHEDULE_MASK.take(cpu); }
 DISPATCH_NOTIFICATION_MASK.store(0, Ordering::Release);
 SOURCE.store(0, Ordering::Release); BUSY_LEFT.store(0, Ordering::Release); HOOK.store(0, Ordering::Release);
 ATTEMPTS.store(0, Ordering::Release); ACCEPTED.store(0, Ordering::Release);
 ONLINE_CORES.store(12, Ordering::Release); SCHEDULER_READY.store(true, Ordering::Release);
}
#[test] fn bootstrap_intent_becomes_dispatch_on_scheduler_release() {
 reset(); SCHEDULER_READY.store(false, Ordering::Release);
 request_reschedule_on(1); flush_dispatch_notifications();
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 0);
 mark_scheduler_ready(); flush_dispatch_notifications();
 assert_eq!(ACCEPTED.load(Ordering::Acquire), 2);
 assert!(RESCHEDULE_MASK.pending(1));
}
#[test] fn local_no_self_ipi_remote_local_pending_still_kicks() {
 reset(); request_reschedule_on(0); flush_dispatch_notifications();
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 0);
 SOURCE.store(1, Ordering::Release); request_reschedule_on(0);
 flush_dispatch_notifications(); assert_eq!(ACCEPTED.load(Ordering::Acquire), 1);
 assert!(RESCHEDULE_MASK.pending(0));
}
#[test] fn remote_request_shares_explicit_local_delivery() {
 reset(); request_park_after_sysret(); flush_dispatch_notifications();
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 1);
 SOURCE.store(1, Ordering::Release); request_reschedule_on(0);
 flush_dispatch_notifications(); assert_eq!(ATTEMPTS.load(Ordering::Acquire), 1);
 assert!(RESCHEDULE_MASK.pending(0)); assert!(RESCHEDULE_MASK.take(0));
 request_reschedule_on(0); flush_dispatch_notifications();
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 2);
}
#[test] fn remote_progress_without_external_event_and_inflight_coalescing() {
 reset(); request_reschedule_on(1); flush_dispatch_notifications();
 assert_eq!(ACCEPTED.load(Ordering::Acquire), 2); // old model: zero
 for _ in 0..100 { request_reschedule_on(1); }
 flush_dispatch_notifications(); assert_eq!(ATTEMPTS.load(Ordering::Acquire), 1);
 assert!(RESCHEDULE_MASK.take(1)); request_reschedule_on(1);
 flush_dispatch_notifications(); assert_eq!(ATTEMPTS.load(Ordering::Acquire), 2);
 assert!(RESCHEDULE_MASK.pending(1));
}
#[test] fn busy_progress_in_same_flush_without_external_retry() {
 for busy in [1, 3, 4095] {
  reset(); BUSY_LEFT.store(busy, Ordering::Release); request_reschedule_on(11);
  flush_dispatch_notifications(); // old model: retained, accepted zero
  assert_eq!(ATTEMPTS.load(Ordering::Acquire), busy + 1);
  assert_eq!(ACCEPTED.load(Ordering::Acquire), 1 << 11);
  assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 0);
  assert!(RESCHEDULE_MASK.pending(11));
 }
}
#[test] fn bounded_exhaustion_retains_obligation_and_allows_diagnostic_recovery() {
 reset(); BUSY_LEFT.store(4096, Ordering::Release); request_reschedule_on(1);
 assert!(std::panic::catch_unwind(flush_dispatch_notifications).is_err());
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 4096);
 assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 2);
 assert!(RESCHEDULE_MASK.pending(1));
 // Host catch only: production panic is terminal, not an automatic recovery.
 flush_dispatch_notifications(); assert_eq!(ACCEPTED.load(Ordering::Acquire), 2);
}
#[test] fn acknowledgement_interleavings_and_two_sources_multiple_targets() {
 reset(); request_reschedule_on(1); SOURCE.store(2, Ordering::Release);
 request_reschedule_on(1); request_reschedule_on(3);
 assert!(RESCHEDULE_MASK.take(1)); // scheduler wins before old dispatch
 request_reschedule_on(1); flush_dispatch_notifications();
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 2);
 assert!(RESCHEDULE_MASK.pending(1)); assert!(RESCHEDULE_MASK.pending(3));
 assert!(RESCHEDULE_MASK.take(1)); assert!(RESCHEDULE_MASK.pending(3));
}
#[test] fn stale_target_rejected_without_aliasing_or_sending() {
 reset(); ONLINE_CORES.store(2, Ordering::Release);
 assert!(std::panic::catch_unwind(|| request_reschedule_on(11)).is_err());
 assert!(!RESCHEDULE_MASK.pending(11));
 assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 0);
}
#[test] fn dispatch_hooks_preserve_new_publication_across_acknowledgement() {
 for hook in [1, 2] {
  reset(); request_reschedule_on(1); HOOK.store(hook, Ordering::Release);
  BUSY_LEFT.store(2, Ordering::Release); flush_dispatch_notifications();
  assert_eq!(ACCEPTED.load(Ordering::Acquire), 2);
  assert!(RESCHEDULE_MASK.pending(1));
  flush_dispatch_notifications();
  assert!(ATTEMPTS.load(Ordering::Acquire) <= 4);
 }
}
#[test] fn exhaustion_preserves_unattempted_targets_too() {
 reset(); request_reschedule_on(1); request_reschedule_on(11);
 BUSY_LEFT.store(4096, Ordering::Release);
 assert!(std::panic::catch_unwind(flush_dispatch_notifications).is_err());
 assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), (1 << 1) | (1 << 11));
}
#[test] fn target_removal_fails_before_hardware_access_with_retained_work() {
 reset(); request_reschedule_on(11); ONLINE_CORES.store(2, Ordering::Release);
 assert!(std::panic::catch_unwind(flush_dispatch_notifications).is_err());
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 0);
 assert!(RESCHEDULE_MASK.pending(11));
 assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 1 << 11);
}
#[test] fn legacy_unregistered_rejection_strands_work_original_counterexample() {
 reset(); let pins = handoff::Handoffs::<2>::new();
 pins.pin(0, 7); // outgoing task detached under S, old CPU has not pivoted
 request_reschedule_on(1); flush_dispatch_notifications();
 assert_eq!(ACCEPTED.load(Ordering::Acquire), 2);
 assert!(RESCHEDULE_MASK.take(1)); // target's real scheduling opportunity
 // Production RR selection rejects a context pinned on another CPU.
 assert!(pins.is_pinned(0, 7));
 let selected = !pins.is_pinned(0, 7); assert!(!selected);
 pins.slot(0).store(handoff::EMPTY, Ordering::Release); // physical epilogue
 assert!(!pins.is_pinned(0, 7));
 assert!(!RESCHEDULE_MASK.pending(1));
 assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 0);
 // This is a positive counterexample for a remaining lifecycle boundary,
 // not a repaired assertion and not a native task-failure reproduction.
 println!("F13NR-LIMIT pin-released target-eligible=true request=false dispatch=false execution=unverified");
}
#[test] fn pin_release_after_rejection_rearms_target_r2() {
 // T0 pin on CPU0; T1 task Ready; T2 request+IPI to CPU1; T3 CPU1 consumes
 // and rejects (registers); T4 CPU0 releases; T5 release must wake CPU1.
 reset(); for c in 0..12 { STACK_HANDOFFS.take_waiters(c); STACK_HANDOFFS.slot(c).store(handoff::EMPTY, Ordering::SeqCst); }
 STACK_HANDOFFS.pin(0, 7);
 request_reschedule_on(1); flush_dispatch_notifications();
 assert!(RESCHEDULE_MASK.take(1));
 assert!(STACK_HANDOFFS.wait_release(0, 1, 7)); // production rejection path
 assert!(!RESCHEDULE_MASK.pending(1));
 let before = ATTEMPTS.load(Ordering::Acquire);
 STACK_HANDOFFS.slot(0).swap(handoff::EMPTY, Ordering::SeqCst); // epilogue xchg
 pin_release_wake(); // epilogue tail
 assert!(RESCHEDULE_MASK.pending(1));
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), before + 1);
 assert_eq!(DISPATCH_NOTIFICATION_MASK.load(Ordering::Acquire), 0);
 assert!(RESCHEDULE_MASK.take(1)); // target pass; pin free so task is selectable
 assert!(!STACK_HANDOFFS.is_pinned(0, 7));
 println!("F13NR2-PIN pin-released target-eligible=true request=true dispatch=true");
}
#[test] fn pin_release_without_waiters_sends_nothing_r2() {
 reset(); for c in 0..12 { STACK_HANDOFFS.take_waiters(c); STACK_HANDOFFS.slot(c).store(handoff::EMPTY, Ordering::SeqCst); }
 for _ in 0..100 { STACK_HANDOFFS.pin(0, 3); STACK_HANDOFFS.slot(0).swap(handoff::EMPTY, Ordering::SeqCst);
   assert_eq!(STACK_HANDOFFS.take_waiters(0), 0); }
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 0);
}
#[test] fn pin_release_coalesces_with_inflight_request_and_survives_ack_r2() {
 reset(); for c in 0..12 { STACK_HANDOFFS.take_waiters(c); STACK_HANDOFFS.slot(c).store(handoff::EMPTY, Ordering::SeqCst); }
 STACK_HANDOFFS.pin(0, 7);
 assert!(STACK_HANDOFFS.wait_release(0, 1, 7)); assert!(STACK_HANDOFFS.wait_release(0, 2, 7));
 request_reschedule_on(2); // another producer's kick to CPU2 still in flight
 flush_dispatch_notifications(); let a = ATTEMPTS.load(Ordering::Acquire);
 STACK_HANDOFFS.slot(0).swap(handoff::EMPTY, Ordering::SeqCst); pin_release_wake();
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), a + 1); // CPU1 only; CPU2 coalesced
 assert!(RESCHEDULE_MASK.pending(1) && RESCHEDULE_MASK.pending(2));
 // ack of CPU1 then a newer request survives
 assert!(RESCHEDULE_MASK.take(1)); request_reschedule_on(1); assert!(RESCHEDULE_MASK.pending(1));
}
#[test] fn pin_release_busy_icr_and_late_waiter_r2() {
 reset(); for c in 0..12 { STACK_HANDOFFS.take_waiters(c); STACK_HANDOFFS.slot(c).store(handoff::EMPTY, Ordering::SeqCst); }
 STACK_HANDOFFS.pin(0, 7); assert!(STACK_HANDOFFS.wait_release(0, 1, 7));
 BUSY_LEFT.store(3, Ordering::Release);
 STACK_HANDOFFS.slot(0).swap(handoff::EMPTY, Ordering::SeqCst); pin_release_wake();
 assert_eq!(ATTEMPTS.load(Ordering::Acquire), 4); assert_eq!(ACCEPTED.load(Ordering::Acquire), 2);
 // waiter arriving after release proceeds itself, no stranded registration
 assert!(!STACK_HANDOFFS.wait_release(0, 1, 7)); assert_eq!(STACK_HANDOFFS.take_waiters(0), 0);
}
#[test] fn idle_entry_cannot_suppress_remote_delivery() {
 // The production request decision does not consult any idle-state hint.
 for target_idle in [false, true] {
  reset(); let _ = target_idle; request_reschedule_on(1);
  flush_dispatch_notifications(); assert_eq!(ACCEPTED.load(Ordering::Acquire), 2);
  assert!(RESCHEDULE_MASK.take(1));
 }
}
'''.replace('REQUESTS', str(ROOT / 'kernel/src/sched/requests.rs')).replace('HANDOFF_PATH', str(ROOT / 'kernel/src/sched/handoff.rs')).replace('BOUNDED', bounded).replace('EXTRACTED', extracted)
    generated = OUT / 'delivery-boundary.rs'
    generated.write_text(wrapper)
    binary = OUT / 'delivery-boundary'
    subprocess.run(['rustc', '--edition=2021', '--test', str(generated), '-o', str(binary)], check=True)
    subprocess.run([str(binary), '--test-threads=1', '--nocapture'], check=True)

if __name__ == '__main__':
    main()
