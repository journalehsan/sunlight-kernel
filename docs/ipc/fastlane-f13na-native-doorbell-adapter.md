# FastLane F1.3N-A: native Doorbell adapter foundation

## Executive summary

This phase implements an experimental, kernel-owned Doorbell using the actual
SunlightX task scheduler. It is not a production notification ABI or a native
cross-process FastLane transport. All adapter integration, fault injection and
fixture entry points require **both** `debug_assertions` and the opt-in
`native_doorbell_test` feature. Ordinary boot and release builds exclude them.

The final native fixture reaches its critical assertions on 1, 2, 4 and 12
QEMU CPUs. Each boot runs 42 consumer lifetimes, creates/retires 43 objects,
observes genuine scheduler blocking, checks execution after wake, and finishes
with no waiter, task-reference, queue, pin, dispatch or physical-frame drift.
The associated regression replay and final readiness decision are recorded in
the validation sections below and in the machine-readable evidence index.
Gate B is **PARTIAL**: native Doorbell assertions pass, but an unmatched D3
activation failure in a repeated compatibility run prevents final acceptance.

Gate A remains **VERIFIED NATIVE**. Gate C remains **BLOCKED**. Neither the
adapter nor these fixtures grant access to restricted FastLane pages.

## Scheduler block/wake audit

The audit starts from the R2D/R2C/R2V/R reports, F1.2/F1.3/F1.3N reports, and
the C1.2F tick-tail report. No pin-release algorithm is changed.

Relevant existing mechanisms are:

- Blocking IPC receive serializes the last condition check, receive waiter
  publication and task-state change under `SCHEDULER`. IPC queue access is
  inside that established scheduler transaction, not a separate optimistic
  check followed by an independently locked Blocked write.
- `wake_io_pid`, used here only after validating the exact task incarnation,
  changes `BlockedOnIo` to `Ready`, sets the existing priority-wake marker,
  removes stale queue membership and either requests the live owner's CPU or
  enqueues a detached task through the existing affinity-aware scheduler.
- `schedule_tick` owns dispatch/Running publication. A task made Ready while
  still executing its old context is not independently inserted twice. The
  existing live-owner and stack-handoff pin rules protect that context until
  the outgoing CPU publishes its handoff.
- `request_reschedule_on` publishes retained scheduling work. The coalesced
  dispatch mask and bounded `flush_dispatch_notifications` deliver the existing
  reschedule vector after guards are released. A busy ICR retains the request;
  this adapter does not introduce an IPI transport or a timer retry loop.
- Process termination, pending fatal signals, address-space borrower teardown,
  current-task finish and defensive reaping are the lifecycle integration
  points. They remove the exact Doorbell registration before task retirement.

### Lock ordering and ownership

`SCHEDULER` is the only adapter serialization lock. The fixed object table is
inside it; there is no object-lock-to-scheduler-lock inversion. Existing
runqueue/core-state operations retain their established scheduler ordering.
For reference, blocking IPC receive takes `SCHEDULER`, then `CAP_BROKER`,
then its endpoint shard; this adapter takes neither broker nor shard lock.
No IPC queue is acquired by the adapter, and there is no payload copying,
allocation, remote-CPU waiting or serial output in its transaction.

Fixture `HANDLE`/trace locks are short-lived; a handle is copied and its guard
released before acquiring `SCHEDULER`. Trace callbacks do not acquire the
scheduler. Test exclusion delays perform a fixed amount of arithmetic while
serialized, never wait for another CPU while holding the scheduler lock.
All barriers that wait for another CPU run without scheduler/object guards.

The consumer owns registration and Blocked publication; a producer, cancellation
or lifecycle event owns completion and Ready publication; the scheduler alone
owns context selection and Running publication. These are distinct events.

### The required handoff refinement

The existing asynchronous park-after-SYSRET self-IPI is not by itself proof
that a task has stopped executing. A 4/12-CPU development fixture exposed this:
the next user syscall could execute before that IPI, and its test barrier with
IF clear prevented the intended park. This was a fixture handoff defect, not a
counterexample to R2D pin release.

The adapter therefore has a small, debug-only kernel-stack continuation. It
copies the saved syscall frame to the current task's owned kernel stack using
the existing exec-style stack-trampoline convention, runs the conditional
operation, drops all guards, and synchronously enters the **existing**
reschedule interrupt vector if registration succeeded. The task cannot execute
the next user instruction until that handler has performed its scheduling pass.
If actually blocked, it resumes this continuation only when selected again.
If notification or exit already won, the handler sees Ready or Finished and
does not publish a stale Blocked state. The asynchronous retained request may
coalesce or cause an extra harmless scheduling entry; it cannot erase the
condition or receipt.

No ordinary spinlock crosses the context switch. IF is clear for the normal
syscall/interrupt handoff, not a spin wait covering the consumer's blocked
residence. Other tasks and idle execute with the usual interrupt policy while
the consumer is descheduled.

## Object, task and generation ownership

`doorbell_state::Table` contains four inline slots; creation and all operations
perform no heap allocation. A private `Handle(Channel)` qualifies the slot by
a checked, monotonically increasing object generation. Retirement never resets
that generation. A second checked global epoch qualifies each registration,
so a delayed cancellation cannot withdraw a later wait on the same object.

The single consumer is `{pid, foreground.incarnation}`, authenticated from the
actual `Process` record. PID reuse alone is never sufficient. The native
completion path checks that identity, its exact registration token, non-exiting
state and missing prior receipt before calling the PID-based scheduler helper.
No raw task pointer or object pointer is retained by deferred dispatch work.

The producer/owner is a reserved kernel identity, not a userspace task chosen
by a caller. Only trusted kernel code possessing the private handle can invoke
producer operations. The exclusive test boot loads its own probe; the debug
dispatch number is not added to the public syscall enum or a userspace SDK.

## State representation

The existing serialized `sunlight-fastlane-doorbell` model stores sequence,
consumer ownership, optional waiter and first terminal reason. The table adds
the exact registration token. The process stores `{token, optional outcome}`.
The actual scheduler state and live-owner/pin metadata describe physical
execution; a second parallel scheduler state machine is unnecessary.

| Conceptual state | Concrete representation |
| --- | --- |
| OPEN / NO_WAITER | Open model, no table token or process receipt |
| WAIT_REGISTERING | Inside the scheduler transaction, not externally observable |
| WAITER_BLOCKED | Table token + process registration + `BlockedOnIo`; live ownership may still exist before handoff |
| WAKE_PENDING | Model waiter withdrawn, process receipt set, task Ready or already selected |
| CLOSING / CLOSED | First terminal reason committed; any live waiter completed synchronously; explicit retirement follows receipt/lifecycle cleanup |

No asynchronous object operation requires a persistent CLOSING enum variant.
Terminal publication and waiter extraction occur in one transaction. Physical
storage retirement is a separate checked operation.

## Conditional-wait linearization and lost-wakeup argument

The consumer snapshots under scheduler serialization, Acquire-checks its owned
work condition, then calls `doorbell_wait_if_unchanged`. The latter validates
the current Running task and live CPU owner, terminal state, generation,
sequence and exclusive registration, and publishes the token and Blocked state
before releasing `SCHEDULER`. This transaction is the successful-wait
linearization point. A stale snapshot linearizes at the changed-sequence check
and returns without registering or blocking.

Notify linearizes at the checked sequence increment (or Exhausted terminal
publication) under the same lock. It extracts the model waiter, commits the
exact process receipt, and calls the existing wake helper before releasing
serialization. Ready publication is later than sequence publication; actual
consumer execution is later still. Neither an IPI send nor a queue insertion
is reported as execution.

There are only two relevant serial orders:

1. Notify wins before the transaction: the sequence/lifecycle check rejects
   sleeping, regardless of whether the earlier work check saw the payload.
2. Wait wins: notify sees the committed token and Blocked state. It either
   changes the still-live task to Ready before handoff, or makes the detached
   blocked task Ready and schedules its target CPU. There is no subsequent
   consumer write of Blocked outside this transaction.

Thus neither forbidden check/notify/block ordering exists. A wake before
physical switching persists as a receipt and Ready state; a wake after
switching persists as scheduler work until selection. Spurious wakes withdraw
the registration and return `Spurious`; the consumer must recheck and may wait
again with a new epoch. No timer is needed to repair a missed event.

## Local and cross-CPU delivery

The adapter uses `wake_io_pid` only after exact identity/token validation.
For a live context the existing owner CPU is requested. For a detached task,
existing enqueue/affinity rules select the CPU. The fixture pins its consumer
to CPU 0 and rotates real producer execution across the other CPUs. N8 has
all remote CPUs notify concurrently, serialized only for the short transaction.

Notify without a waiter increments sequence but does not request an IPI.
Subsequent burst notifications after extracting the one waiter do not enqueue
again. The existing request/mask coalescing and R2D pin release remain the sole
remote dispatch machinery. Evidence records real APIC/CPU identities, source
masks, accepted sends, handler entries, scheduler selections and user ACKs.

## Closure, cancellation and task exit

Close is idempotent and first terminal reason wins. New waits/snapshots/notify
return that terminal result. A pending waiter receives a terminal receipt and
is made Ready. A notification that completed before close may leave its
Notified receipt; the consumer must then recheck lifecycle and sees closure.
Close does not overwrite a completed receipt or insert the task again.

Cancellation is scoped to the exact object/task/registration epoch. Cancel
first withdraws the waiter, writes Cancelled and wakes once; later notify only
increments sequence. Notify first writes Notified; later cancellation returns
no completion. An old epoch cannot cancel a new wait. N12 forces both orders.

Consumer termination clears its process receipt, removes the exact model
waiter and commits ConsumerExit without waking the dead task. The existing
exit machinery owns stack/context/address-space reclamation. Reaping checks
the same cleanup defensively. N13 forces an exit attempt while registration
is serialized; it commits immediately afterward and before physical park.
N14 terminates an already physically blocked consumer. Reused PIDs acquire a
new incarnation and cannot receive the old object's wake.

The fixture's producer is kernel-owned and cannot disappear silently. Its
explicit shutdown operation commits ProducerExit and wakes a blocked consumer;
N10's additional producer-exit round tests that contract. This is not a
general process-supervision or survivor-adoption mechanism.

Retirement requires a terminal model, no table waiter, and no process receipt
for that handle. In-flight operations cannot outlive the scheduler guard;
deferred scheduler work contains CPU/task scheduling state, not a Doorbell
reference. Slots can then be reused exactly once with a new generation.

## Exhaustion

Sequence increments are checked. A notification at `u64::MAX` commits
Exhausted, extracts and completes any waiter, and never wraps. A snapshot at
MAX may register, but the next notify terminates it safely. Subsequent waits
and notify return Exhausted. N15 forces this exact native boundary.
Object-generation and registration-epoch exhaustion reject new reservations
without mutating an entry or stranding an existing waiter; host tests cover
those unreachable-in-practice allocation boundaries.

## Native race evidence

Every round has buffered race, timing and cleanup records. Successful boot
alone is insufficient: the parser requires all 42 complete transition rows,
actual CPU/APIC identities, scheduler selections, user receipts or explicit
exit, Reaped cleanup, resource equality and the expected critical order.

| Case | Forced evidence and outcome |
| --- | --- |
| N1 | Notify before snapshot; updated sequence and visible work, no registration |
| N2 | Notify between snapshot and Acquire-work check; visible work, no block |
| N3 | Notify after empty work check; changed sequence, no registration |
| N4 | Remote attempt during registration; failed scheduler try-lock proves exclusion, notification commits after transaction and before physical park |
| N5 | Remote attempt after token publication but before Blocked write; same exclusion, retained Ready/receipt prevents stale block; UP tests local pre-park completion |
| N6 | Independent CPU-0 execution proves previous consumer context detached and Blocked; local notify wakes it |
| N7 | Physically blocked CPU-0 consumer; actual other CPU publishes, dispatch/selection/user ACK observed |
| N8 | Eight notifications per remote producer; exactly one completion, final sequence contains all publications |
| N9 | Spurious scheduler wake, receipt consumed, condition rechecked, second actual block and notification |
| N10 | Close blocked waiter; additional producer-exit round returns terminal receipt |
| N11 | Notify then close and separate close then notify object; first-terminal semantics, no new block |
| N12 | Cancel/notify and notify/cancel orders; one Ready publication and one receipt |
| N13 | Remote exit attempt during registration; exclusion observed; exact task withdrawn before park, no user resume |
| N14 | Exact task terminated after independent physical-block observation; no stale receipt or wake |
| N15 | MAX snapshot, real block, Exhausted completion and execution |
| N16 | Same storage/PID reused with new object and task generations; old handle/incarnation rejected |
| N17 | 24 further create/block/notify/execute/close/reap/retire cycles with unrelated ring-3 work on every CPU |

N4/N5/N13 do **not** claim a notification or termination committed inside a
locked transaction: that is intentionally impossible. They force the attempt
and prove exclusion, then force completion between transaction and physical
handoff. A fixed bounded delay makes the attempt observable; failure to reach
the ordering fails the fixture rather than becoming a boot pass.

The UP boot labels the remote/exclusion N4/N7/N13 cases **NOT_FORCED**, not
remote successes. Their 2/4/12-CPU counterparts are **FORCED_AND_PASSED**.
All other intended cases are FORCED_AND_PASSED. Development failures and
superseded successful images remain in separate evidence directories.

## Native CPU matrix and accounting

Final source identity and ELF/ISO SHA-256 values are in
`target/fastlane-f13na-evidence/native-final/results.json`. The source revision
is R2D commit `e088ba276` plus this uncommitted adapter patch; hashes identify
the actual tested content, not just HEAD.

| CPUs | Critical matrix | Created / retired | Physical frames before / after |
| ---: | --- | ---: | ---: |
| 1 | FORCED_AND_PASSED, UP exclusions above | 43 / 43 | 68151 / 68151 |
| 2 | FORCED_AND_PASSED | 43 / 43 | 67893 / 67893 |
| 4 | FORCED_AND_PASSED | 43 / 43 | 67381 / 67381 |
| 12 | FORCED_AND_PASSED | 43 / 43 | 65333 / 65333 |

Each final row additionally reports active objects=0, waiters=0, process
registration references=0, outstanding completion obligations=0, deferred
notifications=0, pending requests=0, queued entries=0 and handoff pins=0.
There are 37 consumed/withdrawn completion events and zero adapter metadata
heap allocations per boot. The four-slot table and fixed trace buffer remain
static; their bytes are not represented as allocated/freed heap objects.
All task stacks are retired; every per-round cleanup explicitly observes
Reaped, not merely Finished. Worker user-execution counters are nonzero on
every CPU. Zero frame drift is therefore accompanied by direct waiter and
task-metadata checks, not used as a substitute for them.
The fixture preallocates its finite task population. Preparation-vector and
scheduler task/runqueue capacities are retained kernel/test scaffolding, not
live Doorbell references; global heap-capacity reclamation is not measured.
The zero-allocation claim concerns the inline Doorbell table and registrations,
not all fixture preparation or existing scheduler allocation.

QEMU uses TCG, `-cpu max`, 1024 MiB, no NIC and the trusted fixture ISO.
Each CPU's periodic LAPIC timer is masked. Blocking rounds have zero timer
advance; a possible initial N1 global-tick mirror catch-up is recorded
separately and cannot repair any block because N1 does not wait.

## Idle and latency measurements

N7 holds a directly observed detached Blocked consumer for at least 100 ms.
Final durations are 100.211/100.241/100.264/100.183 ms on 1/2/4/12 CPUs,
respectively. Target reschedule-handler entries during each interval are zero:
observed Doorbell-induced polling wakeups are **0/s for that interval**. This
does not claim the whole kernel has no timer activity in normal production.

Measurements use the fixed native trace buffer, not serial prints in measured
paths. Same-source notify-commit to Ready publication is comparable; cross-CPU
clock offset/drift was not established, so cross-CPU notify-to-execution
latency is **unavailable**. Same-CPU execution samples end at the actual user
ACK syscall, not IPI delivery or Runnable publication.
The commit checkpoint follows the serialized model update; the Runnable
checkpoint follows the task-state write. These are instrumented observation
points, not instruction-exact hardware timestamps. Each round retains its
last wait/notify duration; earlier calls in the coalescing burst are not a
separate latency population.

| CPUs | Registration median (ns), 35 samples | Notify median (ns), 38 samples | Commit-to-Ready median (ns), 33 samples |
| ---: | ---: | ---: | ---: |
| 1 | 4313 | 6866 | 1196 |
| 2 | 4995 | 8118 | 1496 |
| 4 | 6647 | 19141 | 4320 |
| 12 | 8666 | 18525 | 4268 |

These are instrumented native TCG diagnostic costs, including trace overhead,
not hardware benchmarks or end-to-end FastLane performance. The single stale
wait sample per boot is 467803/614837/692279/452578 ns and is dominated by
cold/instrumented execution; it is not a robust immediate-return benchmark.
Raw samples and descriptive p50/p95/p99/max are retained in the JSON, but
reliable tail percentiles and matched speedup measurements are unavailable.

Phase context-switch/IPI counts are retained, but include controller, park,
yield and exit scheduling, so **per-notification context-switch/IPI cost is
unavailable**, not obtained by dividing unrelated phase totals. The N8 burst
coalesces 7/7/23/87 notifications beyond the first wake on 1/2/4/12 CPUs.

## Ring ordering and isolation

The native fixture uses a kernel-owned atomic work counter (Release producer,
Acquire consumer), not an SHM mapping or native Ring transport. F1.2 Ring and
F1.3 host Ring/Doorbell integration tests are retained and rerun. The Ring's
cursor Acquire/Release implementation is unchanged.

Future trusted Ring integration must keep: payload write, Release producer
cursor, notify; consumer sequence snapshot, Acquire Ring check, conditional
wait, then Acquire Ring/lifecycle recheck. A changed sequence closes the
empty-check-to-registration window; it never substitutes for cursor memory
ordering. No independent untrusted processes share legacy unrestricted SHM
for this fixture.

## Regression replay

The exact commands, exit codes, source/image identities and raw logs are
indexed by `target/fastlane-f13na-evidence/validation-summary.json`. Reproduce
with:

```sh
python3 tools/test_native_doorbell.py --cpus 1 2 4 12 --evidence-dir target/fastlane-f13na-evidence/new-attempt
python3 tools/fastlane_f13na_validate.py --checks
python3 tools/fastlane_f13na_lint.py
python3 tools/fastlane_f13na_validate.py --compatibility --c12f
python3 tools/fastlane_f13na_validate.py
```

The native runner refuses to overwrite a completed attempt. Keep prior
aggregate/check logs separately when deliberately repeating those commands.

Final-source host kernel/adapter, Ring/Doorbell, IPC/service, scheduler host,
boundary and evidence-parser tests pass. Default, fixture and release target
checks pass. The R2D native fixture passes again on 1/2/4/12 CPUs. Foundation
and isolated adapter strict Clippy, clean changed-file formatting, and
`git diff --check` pass.

| Final-source host suite | Result |
| --- | --- |
| Kernel library | 91 passed |
| Kernel library with native adapter state | 97 passed, including six new table/epoch/lifecycle tests |
| F1.3 Doorbell / F1.2 Ring | 8 / 5 passed |
| IPC / audio daemon / sunlightd libraries | 61 / 25 / 29 passed |
| Existing scheduler evidence parsers | 21 passed |
| F1.3N-A evidence parser | 5 passed; rejects boot-only, missing transitions, false remote/receipt/cleanup claims |

Full strict Cargo kernel Clippy remains blocked by the baseline's 24
message/path pairs, with zero current-only. Fixture Cargo Clippy and its
no-dependencies variant are compared against their appropriate retained
baseline logs. A supplemental actual-primary-crate Clippy invocation, using
the real compiler arguments/dependencies and a read-only HEAD archive,
reaches 132 baseline/current diagnostic fingerprints with zero current-only.
Four fixture-specific findings from the first pass were fixed before the
final native image; their raw failed attempt is preserved.

Workspace formatting retains the same 53 hunks (48 distinct normalized
hunks), with no new hunk. The media test retains the matching repository WAV
failure (36 passed, one failed). Miri is unavailable. These remain explicit
failures/tool limitations, not passing suites.

| Native compatibility replay | Result |
| --- | --- |
| R2D pin release | PASS, 1/2/4/12 CPUs; Gate A retained |
| Scheduler state/lifecycle native regression | PASS, 1/2/4/12 CPUs |
| C1.2F timer trace | PASS, complete 22-run CPU-0 guest suite and trace analysis |
| C1.2F mixed timer/IPC/audio | PASS, complete eight-run CPU-0 guest suite and analysis |
| Capability issuance, process exit/exec, incarnation and SHM retirement | PASS, 1/2/4/12 CPUs, explicit authority/lifecycle markers |
| Resident boot and remote SHM release | PASS, 1/2/4/12 CPUs; restricted FastLane still blocked |
| VM/TLB shootdown | PASS, 12 CPUs |
| VM unmap / protection | PASS, four CPUs each |
| RecoveryBlocked / bounded manager restart | PASS, 1/2/4 CPUs |
| D1/D2/D3 lifecycle | First 1/2/4 matrix PASS; repeated frozen-harness matrix: one CPU PASS, two CPUs FAILED, four CPUs NOT_RUN |
| Isolated D1/D2/D3 frozen-ISO replay | PASS, 1/2/4 CPUs, unchanged sources and 18 complete graph-accounting cycles; earlier unmatched failure remains unresolved |
| Native TCP/TLS | PASS, 12 CPUs with certificate installation/verification, unknown-CA and wrong-host rejection, direct and SOCKS paths |

The lifecycle aggregate exits 1 because its workspace-format check fails;
all its individual native assertions pass and its source hashes match. Its
media failure remains recorded separately as the known WAV failure. Neither
aggregate exit 1 is silently presented as an entirely passing command.

TCP/TLS initially hit sandbox denial when creating local fixture sockets.
The denied log is retained as BLOCKED_SANDBOX; the separately approved local
loopback retry passes and retains serial output and packet capture. No
certificate-verification check was disabled.

### Unmatched D3 replay failure

The first full D3 matrix passes its actual native assertions, but its broad
source manifest differs because this phase's non-executed evidence-controller
Python was refined during that run. Therefore a frozen-harness replay was
required. That replay passes one CPU, then fails on two CPUs in concurrent
graph activation after four successfully reclaimed graph cycles:

```text
services/sunlight-startup-fixture/src/d3_driver.rs:50
ACTIVATE reply label: 53759 (0xD1FF, ERROR)
expected:            53632 (0xD180, OK)
```

The failing run's source manifest has no differences. Its raw serial and
controller log are under
`target/fastlane-f13na-evidence/d1-d2-d3-failed-final-harness/`; the retained
legacy `validation-summary.json` in that directory is stale, and the explicit
`classification.json` supersedes it. Four-CPU artifacts inherited from the
previous invocation are **not** evidence that the failed replay reached four
CPUs. The isolated replay uses a fresh directory and the frozen ISO without
rebuilding or modifying service logic, timeouts or the scheduler.

The `dependency_test` image does not enable `native_doorbell_test`, so the
adapter is absent from that boot. This establishes scope, not causality or a
baseline exemption. The failure has **MATCHED BASELINE UNAVAILABLE** status;
it is not folded into the historical Clippy/format/WAV exceptions. Its root
cause and compatibility stability remain unresolved. The isolated frozen-ISO
replay passes 1/2/4 CPUs with unchanged source hashes and all 18 graph samples;
its complete results are retained in `d3-isolated`. This demonstrates an
intermittent failure, not its root cause or a baseline match. It does not erase
the failed attempt, which continues to hold Gate B at PARTIAL.

## Remaining limitations and readiness

Gate A is **VERIFIED NATIVE**: its fresh critical R2D regressions pass and no
new pin-release counterexample is established.

Gate B is **PARTIAL**, not VERIFIED NATIVE. Native object state, exclusive
generation-qualified ownership, conditional wait, real block/wake, before/
during/after-handoff orders, SMP notification and execution, close/cancel,
exit/reuse, polling absence and tested resource cleanup have native evidence.
However, relevant compatibility acceptance is not established while the
unmatched D3 activation failure remains unresolved. The evidence collector
keeps this finding explicit even if a later isolated attempt passes.

Resolve or independently baseline-qualify that D3 compatibility failure and
replay the affected matrix before promoting Gate B. Do not redesign pin
release, weaken activation checks, increase timeouts or declare the failure
historical without evidence. The JSON remains the final run index.

This foundation is deliberately opt-in and experimental. It does not establish
production syscall authority, general notification fanout, survivor adoption,
hardware latency, matched throughput, cross-CPU timestamp comparability or
unbounded-lifetime accounting. RecoveryBlocked semantics are unchanged.

Before production integration, complete restricted-SHM per-recipient grants,
VFS/UAC delegation, RO/RW/NX enforcement, active revocation, process-incarnation
authorization, secure channel setup/teardown and native transport lifecycle.
Only afterward can a separately reviewed userspace ABI and cross-process data
path be considered. Gate C therefore remains **BLOCKED** regardless of Gate B.
