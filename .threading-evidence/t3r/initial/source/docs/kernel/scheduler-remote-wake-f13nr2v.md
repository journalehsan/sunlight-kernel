# FastLane F1.3N-R2V: native pin-release verification

Decision: **PARTIALLY VERIFIED** pin-release repair; **NOT READY** for native
Doorbell adapter development. Native FastLane transport remains **BLOCKED pending
restricted SHM authorization and lifecycle verification**. This is a verification
fixture, not a transport implementation or a scheduler policy change.

## Repair and lifecycle audit

The outgoing-stack slot is owned by the physical CPU, not by a migrating task.
`STACK_HANDOFFS` has 64 cache-line-aligned slots, each containing a task-table
index and a CPU waiter mask. `pin_outgoing_stack` publishes the index under
`SCHEDULER` before relinquishing task ownership; a second pin on that CPU panics.
The task records `stack_handoff_core`, so pickers and reapers can locate the
physical owner even after `owning_core` becomes empty. A pin is not a counter.

`pick_next_round_robin` and `steal_work_round_robin` observe the remote slot,
then call `wait_for_pin_release`. `wait_release` registers `1u64 << waiter`
with SeqCst `fetch_or`, then rechecks the task index with a SeqCst load. A held
pin makes the selector restore the queue entry and reject it. An already released
or differently occupied slot makes the selector withdraw its bit and continue
the current selection pass. That continuing pass, rather than a future timer,
is the independent progress path for the release-wins case.

All four physical release sites pivot away from the outgoing stack, exchange
EMPTY into the pin, then load the waiter mask. A nonzero mask invokes
`pin_release_wake` on the new stack, with interrupts disabled and no scheduler
guard. Its SeqCst swap claims the bits, calls the existing coalesced
`request_reschedule_on` for each online waiter, and flushes the existing bounded
dispatcher. The pin exchange is a full x86 barrier. Together with the register
RMW/recheck, either the waiter observes release or the releaser observes its bit.
This ordering argument alone does not prove interrupt delivery or execution.

The drain precedes further pinning on the same CPU: local interrupts remain
disabled through the epilogue and there is no nested pin. A late waiter can
register after the drain; its recheck sees release and withdraws. Registration
is serialized by `SCHEDULER`; two concurrent registrations by the same CPU
cannot erase each other's bit. A different CPU's bit is untouched. Task-table
reuse and reclamation check the physical pin under the scheduler lock; exit
detaches execution before the permanent idle-stack pivot and defers reclamation
until the pin clears. A leftover notification carries only a CPU identity,
never a dereferenced task pointer. Eligibility is re-evaluated on the next pass.
These are source arguments; the native coverage limits below still apply.

`RescheduleRequests::take` consumes intent and delivery ownership in one AcqRel
exchange. A later remote request owns a new delivery; an earlier one shares an
already owed delivery. The dispatch mask is swapped in bounded flush passes;
Busy retries use the existing immediate finite budget, not a periodic retry.
The release path cannot clear a waiter without first claiming responsibility
for its CPU scheduling event. There is no CPU hot-unplug protocol in this scope.

The original unregistered-rejection model remains executable in
`tools/fastlane_f13nr_boundary.py`: consumed request, cleared pin, no dispatch.
The 20,000-iteration registration/release test and 100-cycle zero-waiter/no-IPI
control remain unchanged. No production pin-release algorithm was changed.

## Native fixture architecture and exact sequence

Enable `pin_release_test` in a debug kernel; it is separate from
`remote_wake_test` and normal resident boot. Four real ELF processes are
allocated before dispatch. The existing scheduler probe gains role 3: one native
ProcessYield followed by ProcessExit, preserving its integer/SSE cookie.

1. Every participating CPU reports its actual local APIC identity. APs mask only
   their local periodic timers in the isolated idle loop. CPU0 retains its timer.
2. CPU0 publishes one preallocated process through `wake_io_pid`. The process
   executes in ring 3 on CPU0 and yields through the existing ABI.
3. At reschedule entry, a test-only checkpoint changes the yielded test process
   to BlockedOnIo and permits remote CPUs. Actual `schedule_tick` saves the
   context, pins CPU0's outgoing stack and selects the existing idle context.
4. After that selection, while still under S, the fixture wakes the blocked
   process and places its sole queue entry on CPU1 using existing methods.
5. After dropping S and flushing, CPU0 pauses before its normal epilogue. It
   retains its valid physical stack pin. This bounded checkpoint takes no lock,
   allocates nothing, prints nothing, and waits at most five calibrated TSC
   seconds. It does not enable interrupts or release the stack itself.
6. CPU1 enters the real reschedule handler and real RR selector, rejects the
   remotely pinned entry and registers its actual waiter bit. The callback
   asserts the recheck returned held and snapshots the real waiter mask.
7. The fixture verifies the request was consumed, no task selected, and exactly
   one runnable queue occurrence remains. Under S it moves that entry to the
   next remote CPU. This repeats through CPU N-1, giving multiple real waiters
   on the same pin. After dropping S each handler acknowledges completion.
8. CPU0 resumes only after every remote rejection pass has returned, verifies
   the complete mask, and returns normally to the **production reschedule
   assembly epilogue**. Its real RSP pivot and exchange release the pin.
9. The production wake helper claims exactly the expected mask. Test hooks record
   release publication, request calls, accepted ICR sends, reschedule receptions
   and subsequent scheduler passes. The final remote CPU actually resumes the
   saved user context, validates its cookie and calls ProcessExit with code 0.
10. A separate exit-commit checkpoint publishes completion after
    `finish_current_process`. CPU0 waits for all physical pins and remote
    requests to clear, reaps the task, checks all waiter masks and queue
    membership, then begins the next cycle.

The 1-CPU configuration is a local yield/exit/reap control; it claims zero remote
rejections. Forced native passes at other CPU counts require all rejection bits,
all masks, delivery/handler/pass observations, actual execution and teardown.
A boot that misses those assertions is NOT RUN, not PASS.

## Progress independent of the timer

Only AP timers are masked; CPU0's periodic timer remains the controller and
cleanup mechanism. During the critical phase, any AP timer entry panics the
fixture. The rejecting CPU consumes its initial request and returns no selected
context before release. After all acknowledgements, the only new remote task
progress publication is the production release wake. Execution asserts a
release-owned event, reschedule-handler reception and a scheduler pass; UART
reporting happens after teardown, outside the critical path. The host timeout
only terminates a failed guest and cannot count as progress.

This establishes timer independence for the forced remote resume, not a
timer-free boot or timer-free cleanup. Counters are bounded per-CPU atomics;
there is no synchronous logging in the release or rejection hot paths and no
new production polling. The pre-pivot spin is test-only and bounded.

CPU0's continuing timer cannot repair the remote wake in this fixture:
`Scheduler::tick` requests its own CPU, the isolated processes have no IPC,
poll or futex deadlines, and the cycle controller publishes no new work before
the exit-commit checkpoint. The sole ready entry is on the final remote queue,
whose affinity excludes CPU0; RR stealing also skips a queue of length one.
There are no resident service tasks in this image to publish another wake.
At final debug exit CPU0 may still have the ordinary local timer scheduling
request that entered the controller; this is an expected control request, not
an outstanding remote release notification. It is not reported as global
zero-request teardown.

## Ordering, migration, exit and reuse coverage

Registration-wins is forced across four cycles: the physical release waits for
the selector rejection acknowledgements. Release-wins and the concurrent
registration boundary remain host-tested, not deterministically native forced.
Waiting inside `wait_release` for another CPU while holding S would violate
this fixture's lock-safety requirement. No such rendezvous was added.

The native migration is real CPU0 execution followed by final remote execution,
with a saved context and exactly one queue entry throughout rejection. CPU masks
are fixture-owned process inputs; this does not assert production affinity
policy or exhaustive migration interleavings. Every intermediate CPU executes
its own rejection handler. Handoff slot 0 is reused for different task indices
over four cycles, with an empty waiter mask between cycles. The process table
has four preallocated slots: **process-table index/PID reuse is not covered**.

The resumed process self-exits and its user frames, page tables and kernel stack
are reclaimed after release. Exit concurrent with registration, remote kill,
exception termination and outstanding dispatch is **unverified**. The first
12-CPU attempt exposed an instrumentation bug: announcing execution before
`finish_current_process` let CPU0 inspect a still-Running process. Its raw
failure is retained separately; completion now follows the real exit commit.
This was a fixture defect, not a production lost-wakeup counterexample.

## Four physical release paths

| Path | Forced native waiter evidence | Scope |
| --- | --- | --- |
| Reschedule epilogue | Native matrix below | Real pin, rejection, exchange, mask drain, remote resume |
| Timer epilogue | Unverified with waiters | Smoke coverage does not prove rejection on this path |
| `terminate_current_user_process` | Unverified | No forced exception/termination fixture |
| `process_exit` | No waiter forced | Actual migrated task exit, physical release and subsequent reap |

Source inspection confirms all four use exchange, post-release mask load and
the shared helper on a safe destination stack. This is not four-path native
validation. Exit paths cannot resume a terminal process merely to manufacture
an execution assertion.

## CPU masks and dispatch

The design supports 64 scheduler indices with `AtomicU64` masks and bit N for
CPU N; index 63 fits, index 64 cannot. The native fixture restricts itself to
1..12 and asserts raw local APIC ID equals the scheduler index on every CPU.
There is no mask expansion or silent fixture truncation.

Production has pre-existing contiguous-APIC assumptions: `current_cpu_id` clamps
the CPUID APIC ID to 63, `reschedule_bit` clamps to 63, and AP startup caps the
number of APs, not sparse physical IDs. Therefore arbitrary sparse/high APIC
IDs and future CPU-count expansion are **not validated** and are not safely
generalized by this report. The tested QEMU topology has contiguous IDs.
Online CPU identities remain stable throughout each fixture; no hotplug is
performed. The earlier request-only/Busy/coalescing fixture is replayed
separately and must not be counted as forced pin-rejection evidence.

## Native matrix and accounting

Final results are authoritative in
`target/fastlane-f13nr2v-evidence/validation-summary.json` and
`native/results.json`; raw serial/QEMU streams, ELF/ISO hashes, build logs,
commands and source hashes are retained there. Only builds matching the final
fixture source hashes qualify. The final forced-fixture ELF SHA-256 is
`1bba1006bf35160de86b8128f07983348166d6758f858e50519e02b050c23a11`.

| CPUs | Actual source → execution | Cycles | Rejections / release notification bits | Free frames before / after | Result |
| ---: | --- | ---: | ---: | --- | --- |
| 1 | 0 → 0 | 4 | 0 / 0 | 68,235 / 68,235 | PASS local control |
| 2 | 0 → 1 | 4 | 4 / 4 | 67,977 / 67,977 | PASS forced remote |
| 4 | 0 → 3 | 4 | 12 / 12 | 67,465 / 67,465 | PASS forced multi-waiter |
| 12 | 0 → 11 | 4 | 44 / 44 | 65,417 / 65,417 | PASS forced multi-waiter |

All 60 intended remote rejection/registration obligations were observed and
fulfilled. Every CPU 1..N-1 handled a real rejection and a later release event;
the resumed task ran on N-1. Across the remote configurations there were 72
accepted sends in the tracked release epochs: 60 waiter notifications plus 12
normal target-exit self-IPIs. The 1-CPU row tracks zero release-waiter IPIs;
this does **not** mean its yield/exit control path generated no self-IPIs.
Counters are scoped to registered waiter CPUs after release, not all machine
interrupts. The native matrix was rebuilt after the final Rust formatting edits;
the collector verifies its recorded source hashes against the current files.

Each successful cycle verifies zero active physical pins, zero waiter masks,
no pending remote reschedule or dispatch, exactly one queue occurrence while
rejected and zero after reap, and no kernel-stack reference on the retired
task. Final physical free pages must equal the pre-allocation count. Metadata
accounting reports the fixed table length and four retired tasks, not allocator
byte telemetry. Heap allocation drift, full reference-count audits and pending
notification latency distributions are not inferred from frame equality.

## Performance and C1.2F

**MATCHED BASELINE UNAVAILABLE.** No equivalent frozen pre-repair source/build
and matching performance protocol has been established. Historical C1.2F
captures predate this scheduler state and cannot supply a matched comparison.
No stash, reset, history change or source restoration was used. QEMU TCG
functional counters are not throughput, cycle-cost, fairness or latency bounds.
There is no claim of speedup, preserved p99, or measured uncontended overhead.

The existing zero-waiter host control still requires zero IPIs over 100 cycles.
Forced multi-waiter cycles bound accepted sends and request calls per CPU;
these counts include the final target's normal exit self-IPI. They are not
IPIs/sec. The C1.2F host policy/queue/service gates are replayed; current native
latencies are recorded below. Matched overhead and regression measurements
remain an acceptance blocker.

Two current-build normal C1.2F trace boots completed 22 trials each. The
table pools completed wakes within the 18 steady trials of each boot, using
the existing nearest-rank analyzer. These are **service wake-to-selected-dispatch**
intervals on CPU0, not pin-release-to-execution latency:

| Boot | Completed / censored steady wakes | p50 / p95 / p99 / max (ms) | Mean per-trial CPU Jain |
| --- | --- | --- | --- |
| 1 | 13,928 / 0 | 0.089953 / 0.208271 / 1.452002 / 2.474539 | 0.996477 |
| 2 | 13,891 / 0 | 0.090552 / 0.214733 / 1.003172 / 1.914696 | 0.996274 |

Configuration: QEMU TCG multi-thread, `-cpu max`, one vCPU, 1024 MiB,
no NIC, identical frozen trace ISO for both boots, no fixed host CPU affinity.
The original suite retains startup trials, raw bounded scheduler trace events,
worker operation/runtime results and 10 ms host scheduler samples. Exact
commands, manifests and per-stage quantiles are under `c12f/`. Host scheduling,
clock calibration and trace overhead remain limitations. This is descriptive
current-state evidence, not a matched regression bound or a future maximum.
Pin acquire/release cost, remote latency, lock hold cost, IPIs/sec and
trace-disabled throughput were not measured.

Two additional mixed trace boots completed eight trials each with emulated
Intel HDA and the silent host backend. All timer RPCs completed. Minimum
per-trial worker operations at nice +8/+9/+10 were respectively
2,523,754 / 2,198,767 / 2,576,516 in boot 1 and
2,660,837 / 2,190,720 / 2,550,180 in boot 2. This proves finite-workload
positive-nice progress, not a general minimum CPU share or human-audible output.

| Mixed boot / steady cohort | Service wake p99 / max (ms) | RPC completion p99 / max (ms) |
| --- | --- | --- |
| 1 / equal nice | 0.925170 / 1.123563 | 1376.815663 / 1376.815663 |
| 1 / positive nice | 0.972447 / 3.248790 | 371.447851 / 371.794336 |
| 2 / equal nice | 0.949953 / 3.306043 | 1387.380559 / 1387.380559 |
| 2 / positive nice | 1.003141 / 2.525582 | 371.907209 / 371.962826 |

Long equal-nice RPC completion tails therefore remain present despite prompt
timer service. Service-dispatch quantiles are not RPC bounds. Full raw trace,
startup/steady partitions, operation/runtime counts, audio fixture records and
per-stage distributions are preserved in `c12f/mixed-capture{1,2}/timer.json`.
These unmatched single-vCPU results do not establish SMP fairness or the
performance effect of the pin-release repair.

## Compatibility replay and remaining gates

The validation collector records each exact command and outcome, including
host boundary/race, kernel, Ring, Doorbell, IPC/service, scheduler validators,
target checks, Clippy, formatting and audio/media. Additional replay uses the
existing remote dispatch, lifecycle/exec/resident/VM, RecoveryBlocked, D1/D2/D3
and local certificate-verifying TCP/TLS gates, with per-build manifests. A
failed prerequisite is reported; no previous build's success is substituted.

Known earlier Clippy and missing-WAV logs are comparison evidence only. Current
failures must be retained and compared by diagnostic/file, not labelled baseline
merely because older reports mentioned Clippy. Global formatting failures are
kept separate from changed-file checks. Full compatibility success and complete
applicable strict Clippy remain required for readiness.

Observed replay results:

| Gate | Result |
| --- | --- |
| Kernel host | 91 PASS |
| Extracted scheduler host | 63 PASS, including unchanged 20,000-iteration handoff race |
| Delivery/pin boundary | 29 PASS, including legacy counterexample and 100 no-waiter cycles |
| Ring / Doorbell host | 5 / 8 PASS |
| IPC / service host | 115 PASS |
| Scheduler Python validators | 21 PASS |
| Native evidence parser negative controls | 5 PASS |
| Default and fixture bare-metal target checks | PASS |
| Remote wake/Busy/idle smoke | 1/2/4/12 CPUs PASS; separate from forced evidence |
| Existing scheduler lifecycle/reuse/exit race | 1/2/4/12 CPUs PASS |
| Native exec/authority and resident/SHM retirement | 1/2/4/12 CPUs PASS |
| RecoveryBlocked and D1/D2/D3 | 1/2/4 CPUs PASS |
| VM/TLB shootdown / unmap / protect | 12 / 4 / 4 CPUs PASS |
| Native TCP/TLS | 12 CPUs PASS, including unknown-issuer and wrong-host rejection |
| Strict Ring/Doorbell Clippy | PASS |
| Strict kernel Clippy | FAIL; same 24 diagnostic/path pairs as retained F1.3N log |
| Fixture/handoff formatting | PASS |
| Changed Rust aggregate formatting | FAIL; all 13 hunks also reproduced from HEAD copies |
| Workspace formatting | FAIL |
| Media/WAV | 36 PASS, 1 FAIL: missing repository WAV fixture |

`clippy-comparison.json` records no current-only diagnostic/path pair, but
dependency/build-script failures prevent a claim that the complete kernel and
new fixture passed Clippy. A separate strict `pin_release_test --no-deps` run
also stops at the existing kernel build-script `manual_range_contains` lint;
it does not establish complete fixture Clippy coverage. `format-comparison.json` compares exact hunk content
against read-only `git show HEAD:path` copies, ignoring only ANSI and line/file
headers. No unrelated formatting was rewritten. The lifecycle/resident/VM
runner's only failure was formatting; its source hashes were unchanged through
validation. The first TCP/TLS attempt could not create sockets in the sandbox;
the approved retry passed and both logs are retained. Its success includes a
real local ephemeral CA and verified native HTTPS, not a disabled-verification
fetch. Mixed/audio timing results are recorded separately from media decoding.

The public collector can replay host and native gates with:

```sh
python3 tools/fastlane_f13nr2v_validate.py --checks --native --compatibility
```

Current C1.2F commands and the retained reproduction driver are in
`c12f/summary.json` and `c12f/reproduce.py`. Re-running them does not produce a
matched baseline. Scheduler-pass and handler bitmaps establish per-CPU lower
bounds; they are not advertised as complete machine-wide event counts.
`native-fixture-early-exit-failure/`, `native-pre-identity/`,
`native-before-format/` and `remote-smoke-before-format/` retain superseded
attempts and must not be substituted for the final indexed builds.

The strongest possible decision for this fixture remains **PARTIALLY VERIFIED**:
release-wins native ordering, terminal races, forced timer/termination/exit
waiters, process-table reuse, matched performance and full acceptance replay
are not all proven. Remote wake scheduler: **NOT READY FOR NATIVE DOORBELL
ADAPTER DEVELOPMENT**. Native FastLane transport: **BLOCKED pending restricted
SHM authorization and lifecycle verification**.
