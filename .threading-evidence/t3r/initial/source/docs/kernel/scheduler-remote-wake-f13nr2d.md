# FastLane F1.3N-R2D: native lifecycle and dispatch closure evidence

## Executive decision

**Gate A: VERIFIED NATIVE. Gate B: NOT READY. Gate C: BLOCKED.**

R2D closes the two native verification gaps left open by R2C: the initial
stack-handoff pin racing a remote task exit (R2D-A) and a delayed/deferred
reschedule dispatch overlapping a newer pin generation on the same slot
(R2D-B). Both are forced natively on 2/4/12 CPUs with bounded real scheduler
transitions, and both pass under repetition. No production scheduler algorithm
changed. The only source edits are test-only: a new debug feature
`pin_release_r2d_test`, a broadened `cfg` on the exception-exit fixture hooks, a
`cfg`-gated busy-ICR injection for the delayed-dispatch fixture, and the
fixture/parser tooling. Native Doorbell adapter, Restricted SHM, the public
notification syscall and the production FastLane transport all remain disabled.

The authoritative machine-readable index is
`target/fastlane-f13nr2d-evidence/validation-summary.json`. Raw serial, QEMU
commands, CPU identities, ELF/ISO hashes and build-source hashes are in
`native-final/results.json` and `native-final/*cpu.serial.log`. The final fixture
ELF SHA-256 is `552c9028f82bebaf231dae846eb4303253c023831a6a7700241460dc18586efa`
and the ISO SHA-256 is
`84d7f942376414b19b15f865a7baf9ec726322ef19d2ac3c318b53c3aa5cdc88`, both built
from source revision `eec49c6862c7140971da66d6de0c943ab729d814` with feature
`pin_release_r2d_test`. Failed or superseded fixture attempts are retained
separately and never substituted for the final matrix.

## Remaining gaps inherited from R2C

R2C forced release-wins and publication/release/recheck boundaries and audited
terminal stack retention, but explicitly reported two native gaps as **NOT
FORCED**:

* **D2/D3 initial-pin exit overlap.** No native case terminated a task while its
  *initial* (deschedule) pin was still held by the owning CPU; the R2C terminal
  audit ran after the task's migration had already resumed on a later pin
  lifetime.
* **Delayed dispatch across a newer pin.** No native case let a deferred
  reschedule delivery from an older pin lifetime complete after the same
  handoff slot had been legally reused with a new pin occupant.

R2C also left the mixed/audio C1.2F capture-2 failure unclassified and matched
performance unavailable. This report forces both gaps, classifies the C1.2F
failure from preserved raw evidence, and keeps performance at MATCHED BASELINE
UNAVAILABLE.

## Lifetime and ordering contract

The contract is unchanged from R2C. A handoff slot belongs to a physical CPU.
Its `pin` is a process-table index, not a task pointer, PID, generation token or
nesting count. `waiters` is an `AtomicU64` bitmask of CPUs that rejected a task
pinned on that owner. Publication happens under `SCHEDULER` before the live owner
detaches; `stack_handoff_core` preserves the physical owner across migration.
Selectors and reapers access process records under `SCHEDULER`, and both slot
reuse and reaping reject a physically pinned index, so an index cannot be
replaced while it still names an active kernel stack. The static aligned slot
itself never moves.

For one attempted rejection, the named operations are:

* **O** selector observes the index in the owner's pin (Acquire).
* **W** waiter publishes its CPU bit (`fetch_or`, SeqCst).
* **Q** waiter rechecks the pin (SeqCst load).
* **X** owner, after pivoting RSP, exchanges EMPTY into the pin (x86 locked `xchg`).
* **L** owner tests the waiter mask after X.
* **D** `take_waiters` exchanges zero into the mask (SeqCst).
* **U** waiter removes only its own bit (`fetch_and`, Relaxed) when Q differed.

O alone is not a scheduling obligation. If Q still reads this index, W precedes
X in the relevant order, and a registration not withdrawn by that waiter must be
visible to L/D. The releasing CPU then owes each claimed CPU bit a scheduling
event; D transfers the bits to `pin_release_wake`.

## Initial-pin/exit state machine (R2D-A)

The fixture forces the R2C **D2** window ("exit after rejection before the
original release"), and thereby also exercises the D3 registration obligation: a
real remote `terminate_process_by_pid` commits while CPU0 still physically owns
the task's kernel stack through the **initial** (deschedule) pin, with CPU1's
waiter bit already published. It is not a later migrated pin.

| # | State | Transition owner | Invariant |
| -: | --- | --- | --- |
| S0 | Task `Ready`, admitted on CPU0, round prepared | fixture controller | distinct live generation; one queue occurrence |
| S1 | CPU0 selected the task in its real `schedule_tick` | CPU0 | `core_current_task(0) == idx` |
| S2 | CPU0 publishes the outgoing stack under `SCHEDULER` and pins slot 0 | CPU0 epilogue | `is_pinned(0, idx)` true; task set `BlockedOnIo`, affinity remote-only |
| S3 | CPUs 1..N-1 select the entry and reject the held pin | each remote CPU | `wait_release` publishes bit, Q reads held, entry requeued once |
| S4 | CPU1 reaches `idle`, waits for its own registered bit, marks `BOUNDARY=3` | CPU1 | waiter bit observed while pin still held |
| S5 | CPU0 `handoff_checkpoint` observes `BOUNDARY=3`, requires `waiter_snapshot(0)==0b10`, sets `ACCEPTED` | CPU0 | release does not yet occur |
| S6 | CPU1 takes `SCHEDULER`, asserts held pin + `Ready` + exactly one occurrence, calls real `terminate_process_by_pid` | CPU1 | `Finished`, pin **still held**, `kernel_stack` retained, generation unchanged, occurrence 0, duplicate exit rejected |
| S7 | CPU1 sets `TERMINATED`; CPU0 proceeds through its real RSP pivot and X | CPU0 | release occurs only after the whole remote terminal audit |
| S8 | CPU0 drain claims `0b10`; `pin_release_wake` offers CPU1 a pass | CPU0 | exactly the registered bit claimed |
| S9 | CPU1 reschedule pass selects nothing (task Finished), sets `EXITED` | CPU1 | `execution=none`; no revived run |
| S10 | CPU0 reaps, asserts `Reaped`, no kernel stack, empty pins/waiters | CPU0 | slot reusable; `frames_before == frames_after` |

The controlling invariant is in `reap_process_resources`
(`kernel/src/sched/mod.rs:2624`): it refuses to free anything while
`stack_is_pinned(idx)`. `terminate_process_by_pid`
(`kernel/src/sched/mod.rs:3049`) only marks `Finished`, removes the entry from
the ready queues and requests a reschedule on the owning core; the actual stack
free is deferred until the pin is gone. That is why S6 observes `Finished` with
the stack still retained and the pin still held, and why the remote CPU's later
pass is offered but finds nothing runnable.

### Actual forced native exit trace

Observed identically on 2/4/12 CPUs (2-CPU excerpt, `native-final/2cpu.serial.log`):

```text
[R2C] reuse round=7 task=0 old_generation=7 generation=8 old_wake=retired pins=0 waiters=0 dispatch=quiescent
[R2D-A] audit round=7 task=0 generation=8 owner=0 waiter=1 pin=held state=Finished stack=retained queued=0 late_wake=retired duplicate_exit=rejected
[R2V] waiter round=7 cpu=1 registered_mask=0x2 rejected=1 ack=1 accepted=1 handled=1 pass=1 sends=1 requests=1
[R2D-A] forced round=7 owner=0 rejecter=1 task=0 generation=8 rejected=0x2 released=0x2 execution=none pin_release_precedes_reap=1 remote_exit=Finished stack_retained=1 revived=0 duplicate_run=0 waiter_drained=1 pending_dispatch=0 timer_assistance=0
[R2V] cycle round=7 source=0 execution=none rejected=0x2 released=0x2 timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped
```

The plan is not a sequential exit: CPU1 terminates the task while CPU0 is paused
inside its own publish path holding the initial pin, before X. `pin_release_precedes_reap=1`
records that the terminal transition committed while the pin was still held, so
the actual stack release is guaranteed to follow the terminal audit. `revived=0`,
`duplicate_run=0` and `execution=none` record that no CPU resumed the terminal
task and nothing was double-queued. Bounded TSC watchdogs guard every wait; no
scheduler lock is held across a cross-CPU barrier, and the CPU waiting for a
lock (CPU1 in S6) is a different CPU from the one spinning (CPU0), so no
lock-order deadlock is installed.

### Required assertion mapping

| Required check | Native evidence |
| --- | --- |
| No kernel-stack use-after-free | pin held at terminal commit; `reap_process_resources` refuses while pinned; `stack_retained=1` |
| No double stack release | single owner slot, single X per round; `pins=0` at teardown; only one release-kick claimed |
| No retired-task execution | `execution=none`; CPU1 pass returns `result=0` |
| No dangling scheduler reference | occurrence 0 after removal; `queued=0` |
| No duplicate runqueue membership | `diagnostic_ready_occurrences(idx)==1` before, 0 after |
| No invalid reactivation | `revived=0`; later `wake_io_pid`/`wake_pid` attempted and did not requeue |
| Waiter obligations released | `released=0x2`; mask empty at teardown |
| Frame/metadata accounting | `frames_before==frames_after`; `metadata_slots=1`; `retired=8` |

## Delayed dispatch versus new pin (R2D-B)

R2D-B forces a sequence in which the older pin lifetime's deferred delivery
completes after the slot is legally reused:

```text
Pin A (generation 6) established on CPU0 slot 0
CPU1 rejects the pinned entry, publishes waiter bit, then ack
CPU1 publishes boundary=1 (registration-wins) while pin A is still held
CPU0 releases pin A; drain claims the full remote mask
Release-kick dispatch is forced Busy 3x (bounded deferral), then retried
Slot 0 enters quiescent reuse; pin B (generation 7) is published on the same slot
The earlier deferred delivery completes; the new runnable work still runs
```

The central distinction the fixture proves is that a deferred IPI is a **CPU
reschedule request**, not a task-specific wake. It carries no task identity.
Coalesced per-CPU intent/delivery (`RescheduleRequests`) plus the waiter bitmask
are the only carried state. A stale acknowledgement therefore cannot erase newer
work, because acknowledgement is a per-CPU swap of that CPU's coalesced
intent — it has nothing to do with any particular pin or task generation.

### Actual forced native overlap trace

Observed identically on 2/4/12 CPUs (12-CPU excerpt, `native-final/12cpu.serial.log`):

```text
[R2C] reuse round=6 task=0 old_generation=6 generation=7 old_wake=retired pins=0 waiters=0 dispatch=quiescent
[R2V] waiter round=6 cpu=1 registered_mask=0x2 rejected=1 ack=1 accepted=1 handled=1 pass=1 sends=1 requests=1
[R2D-B] forced round=6 owner=0 waiter=1 task=0 generation=7 boundary=1 registration=published release=claimed busy_deferrals=3 retried=1 request_survives_ack=1 old_dispatch_scoped=cpu cleanup=reaped
[R2V] cycle round=6 source=0 execution=11 rejected=0xffe released=0xffe timer_assistance=0 pins=0 waiters=0 queued=0 cleanup=reaped
```

`boundary=1` records that CPU1's waiter bit was published while pin A was still
held (registration-wins). `release=claimed` records that the drain claimed the
full remote mask (`0xffe` on 12 CPUs) after release. `busy_deferrals=3` records
that the first three bounded dispatch attempts for the release kick were
injected Busy and returned without sending, and `retried=1` records that the
work was still present and was delivered on the next flush. `request_survives_ack=1`
records that the coalesced per-CPU request was not lost by the intervening
acknowledgements. `old_dispatch_scoped=cpu` records the key invariant: the
deferred delivery refers only to a CPU scheduling obligation.

The slot is then reused for a new generation (round 6's `generation=7` after
`old_generation=6`), and the target of the older pin lifetime actually executes
on CPU N-1 in its own pass (`execution=11` on 12 CPUs). Because the deferred
delivery carries no task identity, it cannot clear the newer pin, cannot erase a
newer waiter bit, cannot execute two CPUs on one stack, cannot dereference a
retired task, and cannot strand eligible work.

### Required assertion mapping

| Required check | Native evidence |
| --- | --- |
| Pin A not reintroduced | new `generation=7`; pin A's slot reused quiescently |
| Pin B valid to its release | `waiter round=6` shows one real remote rejection on the reused slot |
| Waiter bits attributed per slot protocol | drain claims the full remote mask; `released=0xffe` |
| New work not cleared by old ack | `request_survives_ack=1`; coalesced per-CPU request retained across Busy deferrals |
| Dispatch reaches scheduling machinery | `while waiter … pass=1` and `execution=11` |
| No invalid stack owner | no two CPUs execute the target; `pins=0` at teardown |
| No permanently stranded runnable task | target executes; both lifetimes clean up |
| No orphaned dispatcher state | `queued=0`, `pending_dispatch=0`, `waiters=0` at teardown |

## Generation and ABA correctness

R2D did **not** add pin generation fields. The existing design already excludes
the ABA hazard the fixture probes:

* The pin is a slot index, and the slot may only be reused after the previous
  occupant is fully reaped (`stack_is_pinned` guard in `reap_process_resources`).
* The waiter mask is drained by SeqCst `swap` under the release protocol; a
  registration that outlives a release is either claimed by the drain or
  withdrawn by the registering CPU (U). It never survives into the next pin
  occupant as a stale obligation, because the mask is empty before reuse and a
  new registration targets the new pin lifetime.
* The deferred dispatch state is per-CPU and coalesced (`REQUESTED` /
  `DELIVERY_OWNED`), not per-task. It cannot name a retired task, so an old
  delivery cannot act on a new generation.
* VM identity generations are asserted strictly increasing and nonzero across
  reuse (`old_generation`/`generation` in every `[R2C] reuse` line); the fixture
  records the live generation *before* dispatch and never reads a reaped identity.

The fixture confirmed this empirically: `request_survives_ack=1` with
`busy_deferrals=3` and a passing target execution on the reused slot. No ABA
defect was demonstrated, so no production correction was required and the fast
path is preserved unchanged.

## Task-stack ownership proof

Ownership is a per-CPU single-owner slot with a CAS publication and an
`xchg`-based release in the naked epilogue. During R2D-A the task is `Finished`
yet its `kernel_stack` is retained and `is_pinned(0, idx)` remains true; the
reaper is the only path that frees the stack and it refuses while pinned. During
R2D-B the older pin is released before reuse and the newer occupant is a distinct
generation. At every teardown (`pins=0 waiters=0 queued=0 cleanup=reaped`) and
with `frames_before==frames_after`, no stack was freed while another CPU executed
it and no CPU ran a stack owned by a different CPU.

## Waiter and dispatch lifecycle accounting

After each native cycle the fixture asserts zero active pins, zero waiter bits,
zero target queue occurrences, zero remote request bits and zero deferred
dispatch, with one reusable `Reaped` scheduler record remaining. Eight test
generations retire per topology. The prepared vector retains its bounded capacity;
its empty length plus the single retained record are valid fixture metadata, not
active tasks. CPU0's ordinary local control request is excluded rather than
falsely reporting globally zero intent. Heap bytes, complete scheduler allocation
bytes and independent reference-count instrumentation are **unmeasured**; this is
not a blanket zero-metadata-drift claim.

## 1/2/4/12 CPU evidence

Build: `cargo build --package sunlight-kernel --bin sunlight-kernel --features
pin_release_r2d_test --offline`. Run: `python3 tools/test_remote_wake_smp.py
--pin-r2d --cpus 1 2 4 12 --evidence-dir
target/fastlane-f13nr2d-evidence/native-final --timeout 150` (QEMU TCG). The
extended harness projects the eight-round R2D trace onto the mandatory R2C
six-round contract and validates R2D-A/R2D-B on top.

| CPUs | Actual CPU IDs | R2D-A execution | R2D-B execution | Registered held rejections | Frames before / after | Result |
| ---: | --- | --- | --- | ---: | --- | --- |
| 1 | 0 | local control | local control | 0 | 68,313 / 68,313 | PASS local |
| 2 | 0,1 | none | 1 | 4 | 68,035 / 68,035 | PASS |
| 4 | 0..3 | none | 3 | 12 | 67,523 / 67,523 | PASS |
| 12 | 0..11 | none | 11 | 44 | 65,475 / 65,475 | PASS |

Every topology asserts its local APIC ID equals its scheduler ID. The 1-CPU run
is a local ownership/exit/release/cleanup control and makes no remote-race claim.
2 CPUs force both cross-CPU scenarios with one remote waiter. 4 CPUs exercise
competing selection and pin reuse. 12 CPUs exercise multiple remote waiters,
deferred dispatch and repeated lifecycle cycles.

### Bounded repetition

`python3 tools/fastlane_f13nr2d_repeat.py --cpus 2 4 12 --repetitions 5
--timeout 150 --image-dir target/fastlane-f13nr2d-evidence/native-final --output
target/fastlane-f13nr2d-evidence/repetitions-final` replays the exact final image
(`--skip-build` per repetition) five times per topology. Result: **15/15 pass**
against image SHA-256 `84d7f942…`. Every attempt is retained, including the
serial log and validator verdict; nothing was retried until a pass appeared.
This distinguishes stable behavior from a one-off interleaving but is not
exhaustive concurrency coverage.

## C1.2F mixed-failure investigation

The preserved R2C capture is
`target/fastlane-f13nr2c-evidence/c12f/mixed-capture2` (retained **FAIL**).

| Step | Finding |
| --- | --- |
| Failure classification | startup service-sample failure, run `1-1-1` only; `tty_server` `ready_count=0` and empty `waits_ns` |
| Assertion/checkpoint | the required startup `tty_server` service sample; the four worker operations all recorded progress (224,111 / 218,141 / 223,786 / 220,655) |
| Fixture completion | completed; trials 2..8 passed |
| Capture timeout | none; the `SERVICE` record was emitted |
| Parser failure | none; the validator parsed the trace and produced a per-trial diagnosis |
| Serial truncation | none |
| Host variability | possible; the ordering START→ready window is tight |
| Same-image retry | `c12f/mixed-isolated-retry` passed all eight trials with the same image |

Root cause from the raw serial: the measurement window began before
`tty_server` published its first `Ready`. In the failing boot the `SCHED-BENCH
START` line is 369 and `[TTY] Login screen ready` is 395; in the passing retry
the same ordering holds (START 353, ready 410). Both runs measure the same way;
the difference is that the startup sampling window in the failed run closed
before the first `Ready` was counted. This is an **evidence/timing-collection
gap in the startup service sample**, not a scheduler assertion failure: all four
workers progressed, the steady-state trials passed, and the same-image retry
passed. It is classified as an evidence-collection limitation, not a
reproducible scheduler regression.

### Same-image repetitions

`target/fastlane-f13nr2d-evidence/c12f-repeat/repeat.json` replays the exact R2C
mixed image (SHA-256 `775a0f7f…`) three times: **3/3 pass**. The original failed
capture is retained and is not erased by these passes, per the R2D rule that a
passing retry does not overwrite a failed attempt.

## Performance findings and limitations

Status: **MATCHED BASELINE UNAVAILABLE**. No destructive Git operations
(`stash`, `reset`, `checkout`, `clean`) were used, and no credible matched
pre-repair source/binary/workload baseline could be reconstructed without
altering the active working tree. Consequently every quantitative figure below
is **NOT RUN**, not estimated:

pin acquire/release cost; zero/one/multi-waiter release cost;
pin-release-to-execution latency; remote dispatch latency; reschedule
requests/sec; IPIs/sec; coalesced-dispatch rate; scheduler lock hold time; and
matched C1.2F timer p50/p95/p99/max. The functional TCG fixture establishes
coalescing behavior only, not rates. Missing matched performance is an evidence
gap, not proof of a correctness regression, and no speedup or regression
percentage is claimed.

## Full compatibility replay

### Previously repaired wakeup gaps (R2D-F)

The existing regressions were replayed unchanged and all pass:

| Mechanism | Regression | Result |
| --- | --- | --- |
| Request-only wake | remote smoke, request-only scheduling opportunity | PASS, 1/2/4/12 CPUs |
| Busy-ICR deferred delivery | remote smoke, deferred dispatch has independent progress | PASS (plus R2D-B forced Busy retry) |
| Pin-release wake | R2V native rejection fixture | PASS, 2/4 CPUs; original counterexample stays repaired |
| Registration/release ordering | R2C native ordering fixture | PASS, 2/4 CPUs |
| Reschedule acknowledgement | R2D-B `request_survives_ack=1` across Busy deferrals | PASS; older ack cannot erase newer coalesced work |
| CPU idle entry | remote smoke idle-entry fixture | PASS; targets do not depend on incidental timer interrupts |

| Gate | Outcome |
| --- | --- |
| Kernel host | PASS, 91 tests |
| Ring / Doorbell host | PASS, 13 tests (5 / 8) |
| IPC/service host | PASS, 115 tests |
| Boundary suite | PASS, 29 tests |
| Extracted scheduler host | PASS |
| Scheduler/evidence/ordering validators | PASS (21 / 22 / parser) |
| Default, pin fixture, R2D fixture and R2D release target compilation | PASS |
| R2D native matrix | PASS, 1/2/4/12 CPUs |
| R2D repetition | PASS, 15/15 |
| R2V rejection regression | PASS, 2/4 CPUs |
| R2C ordering regression | PASS, 2/4 CPUs |
| Request-only / Busy-ICR remote smoke | PASS, 1/2/4/12 CPUs |
| Exec/authority + resident boot + VM/TLB/unmap/protect | PASS, 1/2/4/12 CPUs (aggregate exit 1 only from `format-check`) |
| D1/D2/D3 lifecycle | PASS, 1/2/4 CPUs |
| Native TCP/TLS | PASS, 12 CPUs, certificate verification plus unknown-issuer/wrong-host rejection |
| Strict Ring/Doorbell Clippy | PASS |
| Strict kernel Clippy | FAIL at matched dependency/build-script diagnostics; 24 pairs, zero current-only vs R2C |
| Changed-file formatting / diff check | PASS |
| Workspace formatting | FAIL; 53 hunks, identical set to retained R2C evidence |
| Media/WAV | FAIL; 36 pass plus the matched missing WAV (`mp3_converted_repository_wav_is_valid_but_exceeds_loader_limit`) |
| Miri | BLOCKED, cargo-miri unavailable for the toolchain |

Reproduce with:

```sh
python3 tools/test_remote_wake_smp.py --pin-r2d --cpus 1 2 4 12 \
  --evidence-dir target/fastlane-f13nr2d-evidence/native-final --timeout 150
python3 tools/fastlane_f13nr2d_repeat.py --cpus 2 4 12 --repetitions 5 \
  --image-dir target/fastlane-f13nr2d-evidence/native-final \
  --output target/fastlane-f13nr2d-evidence/repetitions-final
python3 tools/fastlane_f11pr41_validate.py --native --regressions
bash tools/test_sunlightd_d3.sh --cpus 1 2 4
python3 tools/network_tls_gate.py --iso <resident.iso> --cpus 12 --timeout 240 \
  --evidence target/fastlane-f13nr2d-evidence/tcp-tls
PYTHONPATH=tools python3 tools/fastlane_f13nr2d_validate.py
```

The TCP/TLS gate first failed inside the sandbox (local socket creation denied);
that attempt is BLOCKED environment evidence, and the approved retry outside the
sandbox passed. The lifecycle/resident/VM aggregate runner exits nonzero only for
the pre-existing workspace-format check; its native assertions and its own source
hash verification passed. That aggregate exit is retained, not relabelled PASS.

Distinguishing proven pre-existing failures from new regressions: the strict
kernel Clippy diagnostic set and the workspace-format hunk set were both diffed
against the retained R2C logs and are byte-identical (zero current-only
entries). The media/WAV failure reproduces the matched R2C failure. These are
recorded, not hidden or "fixed" by rewriting unrelated code. One genuine new
regression was found and fixed during this phase: a two-site `rustfmt`
misformat in the newly edited `pin_release_test.rs` (fixed; `rustfmt --check`
now passes for all changed files).

## Resource and lifecycle accounting

| CPUs | Active pins | Waiter mask | Deferred dispatch | Outstanding requests | Queue occurrences | Frames before / after | Result |
| ---: | ---: | ---: | ---: | ---: | ---: | --- | --- |
| 1 | 0 | 0 | 0 | 0 | 0 | 68,313 / 68,313 | PASS |
| 2 | 0 | 0 | 0 | 0 | 0 | 68,035 / 68,035 | PASS |
| 4 | 0 | 0 | 0 | 0 | 0 | 67,523 / 67,523 | PASS |
| 12 | 0 | 0 | 0 | 0 | 0 | 65,475 / 65,475 | PASS |

No invalid task resurrection, no stack release while executing, no stale waiter
ownership, no orphaned dispatch work, no duplicate runnable insertion, and no
explained frame or metadata drift remain. Zero physical-frame drift is required
but is not substituted for the scheduler-state assertions above.

## Exact remaining blockers

* Gate B preconditions: an exact-identity conditional block/wake handoff,
  cancellation, registration withdrawal, and adapter lifecycle integration are
  still unverified. These are Doorbell-adapter semantics, distinct from the
  scheduler delivery proven here.
* Gate C preconditions: Restricted SHM authorization, per-recipient rights,
  active revocation, production delegation wiring and production lifecycle
  validation remain independently unverified.
* Matched pre-repair performance remains unavailable.
* Coverage limits: tested QEMU topology uses contiguous APIC IDs 0..N-1; the
  waiter mask is `AtomicU64` (indices 0..63); sparse/high APIC IDs, CPU
  hot-unplug, and other pinned-stealing/alternate-work overlaps are not covered.
* Pre-existing baseline limitations remain: strict kernel Clippy, workspace
  formatting, the media/WAV fixture, and Miri availability.

## Final Gate A/B/C classification

* **Gate A — Pin-release scheduler repair: VERIFIED NATIVE.** The original native
  rejection counterexample stays repaired; both registration/release orderings
  and both R2C boundaries are covered; the initial-pin/exit overlap is forced
  natively with a real remote `terminate_process_by_pid`; the delayed-dispatch/
  new-pin overlap is forced natively with a bounded Busy-deferred retry across
  legal slot reuse; migration and slot reuse are safe; no lost-progress
  counterexample remains; the 1/2/4/12 matrix and 15/15 repetitions pass; the
  relevant compatibility tests pass.
* **Gate B — Native Doorbell scheduler readiness: NOT READY.** Readiness to
  begin adapter engineering is not full Doorbell correctness. The adapter must
  still prove its own atomic condition-check, waiter registration, block
  transition, wakeup, closure and cancellation semantics.
* **Gate C — Production Native FastLane: BLOCKED.** Restricted SHM
  authorization, rights enforcement, active revocation, production delegation
  and lifecycle validation remain independent prerequisites and are not waived
  by successful scheduler tests.
