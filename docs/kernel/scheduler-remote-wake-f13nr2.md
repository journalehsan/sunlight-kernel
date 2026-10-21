# Scheduler remote wake F1.3N-R2: pin-release progress

Status: **PARTIAL. Repair implemented, host and native-functional gates pass. Native forced-race, matched performance and full compatibility replay were NOT run. Adapter readiness is NOT granted.**

## 1. Original counterexample (reproduced before the change)
The "pin" is the per-CPU outgoing-stack handoff slot (`sched/handoff.rs`, `STACK_HANDOFFS`), one `AtomicUsize` per CPU holding the task index whose kernel stack the CPU still executes on. It is not a nesting counter: a second `pin` on the same CPU panics, so nesting is forbidden by construction (Case D is vacuous; inner/outer ordering cannot occur).

Observed order (source: `finish_current_process`, timer/reschedule naked epilogues, RR selection):
1. T0 CPU0 `pin_outgoing_stack(0, task)` under S.
2. T1 task becomes Ready and is queued on CPU1 (wake publishes before CPU0 pivots).
3. T2 `request_reschedule_on(1)` publishes request + IPI.
4. T3 CPU1 `take()` consumes the request; `pick_next_round_robin` sees `stack_is_pinned_remotely`, requeues the task, returns to idle.
5. T4 CPU0 stores EMPTY in the epilogue.
6. T5 nothing requests or delivers an event to CPU1. Only the periodic timer could repair it.

Original failing evidence: `target/fastlane-f13nr2-evidence/00-original-counterexample.log` (`request=false dispatch=false`, test `legacy_unregistered_rejection_strands_work_original_counterexample`, formerly `known_limit_...`). This is a host model of production request/handoff state. Native reachability was not reproduced.

## 2. Root cause
Rejecting a task for a remote pin consumed the CPU's request and recorded no obligation, and the release was a plain store with no consumer.

## 3. Invariant
When a CPU rejects a task because of a remote pin, the releasing CPU must, after the physical release, request and deliver a scheduling pass to that CPU, unless the rejecter observed the release itself.

## 4. Repair (minimal; reuses existing machinery)
- `handoff.rs`: each slot gains a `waiters: AtomicU64` bitmask (offset 8, `repr(C)`). `wait_release(owner, waiter, task)` does `fetch_or(SeqCst)` then re-reads the pin (SeqCst). True = still pinned, releaser owes a wake. False = already released, registration withdrawn, caller dispatches normally.
- Release is now `xchg` (full barrier) followed by a load of `waiters`. Dekker-style pairing: either the waiter sees EMPTY or the releaser sees the waiter bit. Host stress test `racing_register_and_release_never_loses_the_waiter` (20000 iterations).
- `sched::pin_release_wake()` runs only when `waiters != 0`: for each waiter it calls the existing `request_reschedule_on` (coalesced delivery ownership) then `flush_dispatch_notifications` (bounded Busy-ICR budget, existing). Ordinary unpin with no waiters adds one load and no IPI.
- Called from the timer epilogue, reschedule epilogue, `terminate_current_user_process` and `process_exit` after the release, on the destination stack with IF clear, no locks held, 16-byte aligned.
- Rejection sites (`pick_next_round_robin`, `steal_work`) call `wait_for_pin_release` (atomic-only, safe under S). `priority_preemption_candidate` only observes and is unchanged.

Memory ordering and lock order: registration and release use SeqCst RMW; the wake path takes no scheduler lock; the request uses the existing AcqRel `take`/`request_delivery` linearization, so a request published after a consumer's `take` survives (existing tests plus `pin_release_coalesces_with_inflight_request_and_survives_ack_r2`). No new generation counter and no new periodic wake.

## 5. Interleaving coverage
Covered by host tests: request before/while pinned (counterexample), release with no waiters (0 IPIs over 100 cycles), multiple waiters with one in-flight kick coalesced, ack then new request survives, Busy-ICR during release (4 attempts, accepted), late waiter after release (no stranded bit). Not separately modelled: migration during release and task exit before release (existing queue/eligibility checks re-run on the woken pass; unverified here).

## 6. Evidence
- Host boundary after repair: `01-host-boundary-after.log`: 29 passed, 0 failed (includes the original counterexample kept as a legacy test).
- Scheduler host gates (`tools/test_scheduler_host.py`): PASS (`03-scheduler-host.log`). Two harness stubs (`tools/scheduler_policy_probe.py`, `tools/tests/scheduler_service.rs`) were updated for the new helper; a first run failed for that reason only.
- Native QEMU TCG xAPIC `remote_wake_test` fixture: PASS at 1, 2, 4 and 12 CPUs (`02-native-matrix.log`, `native/*.serial.log`) on the repaired kernel (elf sha256 `a8800d69...f780`, iso `8eed9c7c...1c8b`). This fixture exercises the repaired request/Busy-ICR/idle paths and the changed epilogue asm (exit, timer and reschedule tails). It does not force the pinned-rejection-then-release race, so it is regression evidence for the asm change, not proof of the new wake.
- `git diff --check` clean; `cargo check --package sunlight-kernel` passes; `rustfmt --check` on `handoff.rs` clean.

## 7. Not done (blockers)
- Native forced before/after-pivot pin-release scenario (A, B, C, D, F of the task) and per-CPU counters for it.
- Matched before/after performance and C1.2F latency comparison. No numbers are claimed.
- Full compatibility replay (IPC, FastLane F1.2/F1.3 host, VM/TLB, SHM, exec, D1-D3, RecoveryBlocked, network TCP/TLS, formatting/Clippy/WAV baselines). Not run.
- Cancellation/migration interleavings are argued from source only.
- Registered waiter bits are not cleared if the waiter's later pass finds the task gone; a stale bit costs at most one spurious coalesced request.

## 8. Readiness decision
**Not READY.** Properties 1-6 are supported by host evidence and 8 by native functional runs; 7 (real forced-race delivery), 9-10 (storm bound measured, performance, compatibility) are unverified. Next step: add a native fixture that holds the pin open across the target's rejection (using `regression::handoff_delay`-style widening) and assert execution without the timer.
