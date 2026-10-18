# Scheduler correctness hardening — Phase B

Date: 2026-10-18. Baseline: `d2faa88e6aee8974e131a19db4cc808af6d8b866`, plus
the Phase A audit/probe already in the working tree. The existing
`.version_state.json` modification is excluded. This work preserves the global
scheduler mutex, per-CPU RR FIFOs, stealing architecture, IPC generation
arbitration and session membership. It adds no scheduling class or weight model.
See [Phase A](scheduler-audit.md) for the historical audit; its old source line
references describe the baseline, not the current checkout.

## Findings verified and patch boundaries

| Finding | Verification and focused change |
| --- | --- |
| Outgoing stack lifetime | Confirmed source-level lifetime violation: the old dispatcher could detach/reap a task while Rust still executed on its kernel stack; a second CPU could restore a published context before the outgoing epilogue finished accessing it. A pre-fix guest crash was **not** reproduced. Stable per-CPU handoff pins now cover the publication-to-physical-pivot interval. |
| Self-exit/borrowers | Self-exit also cleared logical ownership before its caller physically left the task's stack. This matters for a native join that frees a borrower-owned user stack after observing Reaped. Exit now retains the same physical pin until its idle-stack pivot. |
| Reschedule rearming | Confirmed by source and an executable production-helper fixture: normal local rotation called an enqueue helper that requested another dispatch. Internal requeue is now silent; external publication still requests dispatch. No end-of-dispatch clear can erase a later producer's request. |
| Quantum accounting | Confirmed by source: per-core delivered-tick state survived voluntary/idle dispatches and could be inherited by another task. Slice state now starts/stops per dispatch; calibrated elapsed time determines expiry, and policy bookkeeping retires a slice once. |
| Nice starvation | The frozen Phase A selector still reproduces +8..+10 starvation. Aging credit was canceled by debt on the same selection visit: this was queue-admission policy, not a cumulative CPU runtime arithmetic bug or an intentional documented right to starve. Aged eligible tasks now bypass the skip test. |

The changes are separable for review: (1) handoff/lifetime guards and assembly
release, (2) request publication/consumption, (3) slice accounting, (4) the small
aging admission guard. Tests and the opt-in guest fixture are separate files.
No repository commits were created or user changes discarded.

## Physical context and reclamation contract

`owning_core` still identifies logical current execution; it is not the only
lifetime guard. [handoff.rs](../../kernel/src/sched/handoff.rs#L1) holds one
cache-line-aligned atomic task-index slot per CPU in static storage. Unlike a
pointer into `Vec<Process>`, its address survives vector growth and slot reuse.
The process's `stack_handoff_core` points to the last relevant CPU; a stale CPU
value is harmless unless that CPU's live atomic slot still matches the index.
Membership/ownership mutations remain serialized under SCHEDULER.

The sequence is:

1. With IRQs disabled and SCHEDULER held, pin the outgoing task before clearing
   logical ownership or publishing its saved context
   ([pin helpers](../../kernel/src/sched/mod.rs#L805),
   [dispatch](../../kernel/src/sched/mod.rs#L1544)).
2. Remote RR selection/stealing defers a pinned context and **retains its queue
   entry**, rather than dropping its only runnable reference
   ([steal guard](../../kernel/src/sched/mod.rs#L1278),
   [local FIFO guard](../../kernel/src/sched/mod.rs#L1446)). The local CPU can
   select its own pinned frame: it alone completes that epilogue.
3. Resource reaping, Finished-to-Reaped sweeping and publication into reusable
   slots reject a matching physical pin
   ([reuse](../../kernel/src/sched/mod.rs#L526),
   [direct reap](../../kernel/src/sched/mod.rs#L2317),
   [sweep](../../kernel/src/sched/mod.rs#L2617)). Existing owner, queue, borrower,
   current-CPU and active-address-space checks remain in force.
4. The naked timer/reschedule epilogue finishes **all outgoing-stack accesses**,
   switches RSP if required, then clears the stable slot with an aligned x86
   store, before restoring incoming registers. The store is Release on x86 TSO;
   readers use Acquire
   ([timer](../../kernel/src/arch/x86_64/interrupts.rs#L767),
   [reschedule](../../kernel/src/arch/x86_64/interrupts.rs#L833)). The slot pointer
   is held in callee-saved R13, whose task value is already in the saved frame.
   A no-switch return is protected by the restored logical owner.
5. Self-exit/fatal exception callers pivot to permanent idle storage and release
   the pin in the **same asm block**, then jump to the idle loop
   ([finish](../../kernel/src/sched/mod.rs#L3735),
   [syscall exit](../../kernel/src/arch/x86_64/syscall.rs#L1953),
   [exception exit](../../kernel/src/arch/x86_64/interrupts.rs#L250)). There is no
   Rust return on a released old stack after that store.

If a remote kill already won before self-exit, the finisher preserves its exit
code/terminal state but still retires the open runtime/slice and logical current
entry before idling ([terminal exit race](../../kernel/src/sched/mod.rs#L3759)).
The previous early return could leave the interval open through the subsequent
idle halt. The guest gate injects this ordering once per boot, verifies the
winning code remains 137 and asserts both clocks are already closed; it does
not inject over the fixture's nonzero register-corruption failure result.

The pin performs one bounded CAS, not a wait for another CPU; nested publication
is an invariant failure. NMI handlers never acquire or release these pins. IRQs
remain disabled through handoff, and TSS/FS_BASE/FXSAVE/CR3 handling stays under
the existing scheduler lock. No new heap allocation, UART output or waiting
loop was introduced in the production handoff/request/slice bookkeeping.
The current inactive Bore selector was not enabled or validated; enabling it
still requires a separate review of its selection paths.

Reaping now gets a BSP elapsed-deadline opportunity every 50 global ticks even
when no task is current. This is necessary to make a final, deferred Finished
task eventually Reaped. It also avoids per-CPU repeated modulo-triggered sweeps.
It does **not** add a reclamation worker or bound the existing resource cleanup;
heavy cleanup/allocations/logging already present under S remain a Phase C risk.

## Reschedule request contract

[RescheduleRequests](../../kernel/src/sched/requests.rs#L1) preserves the original
coalesced per-CPU bit architecture. Request is a Release OR; peek is Acquire;
entry consumes only the local bit with AcqRel AND. Multiple requests already
pending coalesce into one dispatch that sees current state under S. A request
set after entry consumption remains pending for another dispatch.

[queue_process_to_core](../../kernel/src/sched/mod.rs#L633) takes an explicit
notification flag: external enqueue uses true, the current dispatch's local
rotation uses false. The latter neither sets nor clears the pending bit. A
late/duplicate IPI with no pending request returns without switching. Periodic
timers still request selection while idle so CPUs can discover stealable work;
that intentional polling is distinct from endless runnable-task self-rearming.
Remote wake/park notification flushing and its periodic-timer fallback remain.

The empty-pop branch releases its core guard before calling steal_work, avoiding
a latent thief-lock reacquisition trap if that branch becomes reachable.
The canonical order remains S -> one core; no two core locks are held together.

## Quantum and runtime contract

[Slice](../../kernel/src/sched/quantum.rs#L1) is fixed-size per-core dispatch
state. Its contract differs from cumulative `cpu_runtime_ns`:

- A fresh dispatch starts a fresh slice, including same-task renewal, wake from
  idle and migration. Idle has no open slice. Quanta remain the existing adaptive
  5..50 tick formula and existing nice promotion override; no weight or priority
  classes were introduced.
- Freeze that slice's budget at dispatch. With calibrated TSC, elapsed local
  time determines expiry: `floor((now - start)/10 ms) >= budget`. Delayed IRQs
  cannot grant a full extra quantum, duplicate timestamp samples do not double
  charge, and an IPI does not manufacture a timer tick. Without calibration,
  delivered timer callbacks are the explicit coarse fallback. A compile-time
  assertion ties the tick unit to timekeeping.
- Blocking changes logical state but does not stop actual execution. Its
  syscall/IRQ tail counts until descheduling bookkeeping. Yield, blocking,
  preemption and exit finish the old slice once, then clear per-core ticks;
  blocked/idle intervals cannot transfer to another task.
- A slice finishing at or beyond its frozen budget receives FullQuantum policy
  treatment once. Counter decay/override retirement happens at finish, including
  early deschedules, and uses the frozen budget rather than an already changed
  burst score. `timeslice_used` records completed slice usage, not aggregate time
  accumulated across tasks.
- [start_charging_runtime](../../kernel/src/sched/mod.rs#L959) begins the open
  task execution interval and slice after dispatch cleanup/architectural setup;
  [stop_execution_clock](../../kernel/src/sched/mod.rs#L989) checkpoints/closes
  runtime and retires the slice. First entry starts after queue seeding, and
  ordinary selection starts after opportunistic reaping, avoiding charging that
  setup/cleanup interval to a task that has not resumed.

The shared cumulative accounting arithmetic was already sound in the reviewed
cases and is retained. Repeated checkpoints advance an open interval; stopping
twice adds nothing. Runtime includes owned syscall/interrupt execution up to
descheduling bookkeeping; it is not pure user-instruction time. The small
restore/IRET interval remains an approximation. TSC synchronization and detailed
attribution remain measurement questions. Watchdog enforcement remains tied to
timer opportunities/quantum expiry, not a hard real-time execution guarantee.

## RR fairness contract and preserved reproduction

The supported nice range remains -10..+10. Normal ordering remains local FIFO,
with existing nice credit/debt/promotion and finite skip passes. A task whose
time since last dispatch exceeds `AGING_THRESHOLD_TICKS` (100 global ticks)
must not be skipped for nice debt on its next **eligible local FIFO visit**
([admission guard](../../kernel/src/sched/mod.rs#L1472)). This is eventual
admission, not a proportional share or hard wall-time latency guarantee.
Finite runnable populations, ongoing scheduling, valid affinity/context and
eventual physical handoff completion are required. Existing stealing can
dispatch sooner and still bypasses local nice admission; that policy asymmetry
is deferred. No IPC priority inheritance is added.

[scheduler_policy_probe.py](../../tools/scheduler_policy_probe.py) runs the
[frozen Phase A selector](../../tools/tests/scheduler_rr_phase_a.rs) and current
production functions in the same fixture: one CPU, one test task and a neutral
peer, 10,000 simulated selection opportunities, all 21 nice values and initial
burst scores 0/256/1024. It also executes the production pin-eligibility helpers
with a remote pin and verifies the deferred queue entry survives release.

| Nice, initial burst 256 | Phase A selections | Phase B selections | Phase B longest simulated wait |
| --- | ---: | ---: | ---: |
| 0 | 5000 | 5000 | 2 |
| +7 | 95 | 100 | 101 |
| +8 | 0 | 99 | 101 |
| +9 | 0 | 99 | 101 |
| +10 | 0 | 99 | 101 |

The unchanged debt accumulation/skip policy caused the reproduction. The host
fixture stubs architectural dispatch, real interrupts and stealing; these
counts and simulated waits are not guest timing or promised service shares.
Assertions cover bounded fixture service after the fix while preserving the
old counterexample. The test uses the real selector rather than a copied fix.

## Validation and reproducibility

```sh
cargo check --package sunlight-kernel --bin sunlight-kernel --offline
python3 tools/test_scheduler_host.py
python3 tools/test_scheduler_host.py --workspace-libraries --timeout 90
python3 tools/test_scheduler_smp.py --cpus 1 2 4 12 --timeout 360
SUNLIGHT_TEST_CPUS=2 SUNLIGHT_TEST_TIMEOUT=360 ./tools/test.sh phase_sec
SUNLIGHT_TEST_CPUS=2 SUNLIGHT_TEST_MEMORY_MB=4096 SUNLIGHT_TEST_TIMEOUT=360 \
  ./tools/test.sh helios-thread-probe
```

The focused host command compiles production modules directly with rustc and
runs them serially, including the timekeeper wrapper needed for its crate-level
serial macro. It also executes the production selector and queue-publication
fixtures. The optional library sweep uses separate cargo invocations to avoid
the workspace-wide host/SunlightOS feature union. `--resume` skips prior passing
library packages; timeouts are reported, not treated as passes.

| Check | Actual result |
| --- | --- |
| Default kernel check | PASS after fixes; existing dependency cfg/style warnings. |
| Feature-enabled kernel build | PASS with `scheduler_b_test`; GNU as/ld build the tiny native assembly fixture only for that feature. |
| Focused host unit tests | **24/24 PASS:** original 15 scope/runtime/timekeeper tests plus 3 handoff, 2 request and 4 slice tests. |
| Production policy fixture | PASS for 63 Phase A and 63 Phase B nice/burst cases; Phase A starvation reproduced, current eventual admission asserted. Remote pin retains a queued context until release. |
| Production queue helper fixture | PASS: external publication requests dispatch; internal requeue is silent and preserves a later external request; duplicate request consumption and owned/terminal rejection checked. |
| Entire workspace library command | Does not compile: Wise Owl's combined host/SunlightOS feature set gives missing std/String/Vec errors. No claim of a green workspace-wide run. |
| Individually isolated host libraries | **60/66 packages PASS, 1,831 tests in complete passing packages.** Six failures/timeouts listed below; partial passing tests inside failed packages are not included in that total. This covers library targets, not every repository binary/integration/standalone test. |
| Final isolated TCG lifecycle gate | **1, 2, 4, 12 CPUs PASS**, 1 GiB RAM, QEMU 11.1.1, `-cpu max`, no disk/network. This is functional SMP evidence, not scaling/performance data. |
| Normal two-CPU security/IPC boot gate | PASS on the final production code: native borrower cleanup, generation/deadline arbitration and 64 SMP terminal-state iterations. |
| Linux thread probe, two CPUs / 4 GiB | PASS: clone/thread execution, shared descriptor/eventfd paths, TLS and child-TID clearing. The initial 1 GiB attempt could not start the shell (ELF mapping ran out of frames), so it did not execute the probe and is not counted as a thread regression pass. |

The isolated [guest gate](../../kernel/src/sched/regression.rs#L1) runs real ring-3
workers, including +8..+10 nice workers, CPU-bound external/fatal termination,
native self-exit, native/Linux shared-VM borrowers and prebuilt second-wave slot
reuse, plus the terminal-kill/self-exit ordering above. Each yielding worker
checks integer/SSE cookies across 32 yields. A
bounded, test-only post-lock delay widens the physical handoff race window;
no synchronization waits or UART output are added there. Assertions check unique
queue presence, current/owner consistency, expected exit codes, absent stacks
on Reaped entries, eventual cleanup and activity on every configured CPU.
VM/replacement fixture allocations occur before scheduling; test-only cleanup
uses the existing reaper. Ordinary production builds omit the gate/delay.

Final matrix serial evidence:

| CPUs | Deferred direct-reap attempts | Reused slots | Exit races | Result |
| ---: | ---: | ---: | ---: | --- |
| 1 | 4 | 1 | 1 | PASS |
| 2 | 7 | 2 | 1 | PASS |
| 4 | 7 | 4 | 1 | PASS |
| 12 | 12 | 12 | 1 | PASS |

Counts are test observations and may vary with interleavings; positive deferred
reap count is asserted. The ELF SHA256 for that run is
`afab1f29cb88a1e9f262be3ec3b47595551d731c98e2d1b22f5fce7446895abb`.
[test_scheduler_smp.py](../../tools/test_scheduler_smp.py) retains the exact
ELF/ISO, checksums, command lines, results JSON and per-CPU-count serial/QEMU logs
under `target/scheduler-b-evidence/`. QEMU is stopped after the complete serial
PASS marker, matching existing repository gate practice. The first harness
attempt reached PASS but timed out waiting for debug-exit; that attempt is
retained under `initial-timeout/` and is not counted as a successful harness run.
Review also found a stale-log race in the initial marker-based runner: it could
read an old PASS before QEMU opened/truncated the serial file. Those potentially
stale runs were discarded. The final runner truncates each log synchronously
before Popen and requires a complete PASS line; the final matrix above was
rebuilt and rerun after that correction, with new boot/begin/PASS output for
each CPU count. Earlier evidence is retained separately for traceability.
Disassembly of both naked IRQ stubs was inspected: outgoing RFLAGS access
precedes RSP pivot, which precedes the aligned pin-release store and register
pops. No pre-fix stack corruption incident or guest latency improvement is
claimed.

The normal gate console transcripts are retained as `phase-sec-2cpu.log` and
`helios-thread-2cpu-4g.log` in that evidence directory. The incomplete 1 GiB
desktop attempt's serial log is retained separately. The focused lifecycle
matrix uses the smaller isolated fixture and did complete at 1 GiB.

Unrelated host-library outcomes (their implementation sources were unchanged):

| Package | Observed limitation |
| --- | --- |
| sunlight-libc | 90 s timeout in `secret_store::tests::default_options_are_private_and_bounded`; also stalled in the initial parallel sweep. |
| sunlight-utils | 90 s timeout while running `comm::tests::disjoint_files`. |
| sunlight-reminders | `date_and_time_validation_is_strict` failed; 11 other tests passed. |
| sunlight-networkd | Host library tests fail to compile: missing `vec!` imports. |
| rappid-rabbit | Host library tests fail to compile: missing parse_html/StyleContext/collect_embedded_stylesheets symbols in that configuration. |
| sunlight-api-lab | Three JSON pretty-printing expectations failed; 13 other tests passed. |

Raw per-package output, completion codes and commands are under
`target/scheduler-b-evidence/host-libraries/`; the failed workspace-wide build
was also captured in `/tmp/scheduler-b-workspace-host.log` during this session.
These failures were not repaired as part of scheduler hardening. Baseline-only
reruns of those unrelated crates were not performed, so the report attributes
them to the observed configuration rather than claiming a new baseline proof.

## Phase C risks and unexecuted coverage

- The global mutex, full process/FD/deadline scans and resource cleanup under
  IRQ-disabled scheduling remain. This patch adds no new fast-path allocation
  but does not remove inherited lifecycle allocation/UART or bound all cleanup.
  Pin release adds an extra CPU-ID lookup in interrupt entry. Its cost is
  unmeasured; no performance improvement is claimed.
- The guest matrix is an isolated lifecycle stress fixture. Full mixed
  audio/UI/IPC/CPU workloads, input-to-visible latency, wake p50/p95/p99,
  lock wait/hold histograms, real hardware/KVM, non-dense APIC topology and
  general affinity mutation have **not** been tested/measured here.
- Host pin tests and guest delayed epilogues exercise the guards; they are not
  exhaustive model checking, a memory sanitizer or proof against every NMI/
  exception interleaving. Preserve IRQ-off release ordering and stable atomic
  storage in future changes.
- Phase A's waitpid completion-versus-slot-reuse status hazard is not addressed
  by stack quiescence. Preserve completion records until consumed in a separate
  lifecycle change; session/incarnation/request identities must remain distinct.
- Stealing's one-queued-task refusal, order reversal, affinity edge cases,
  different nice admission on steals, invalid-context fallback handling and
  inactive Bore mode remain separate review items. RR starvation rescue is an
  eventual-admission contract, not priority inheritance or hard real-time audio.
- Fix the host-suite configuration/failures before claiming all repository host
  tests pass. None of the listed failed/unexecuted coverage is silently included
  in a green result.

Phase C should first measure a source-matched hardened baseline and bound
remaining global critical sections. Any independent per-CPU synchronization or
new scheduling policy still requires a separate design/review and measured
justification.
