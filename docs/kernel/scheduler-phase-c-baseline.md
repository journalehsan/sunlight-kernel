# Scheduler Phase C1: instrumentation implementation and performance baseline

Date: 2026-10-18. Source state: Phase B described in
[scheduler-hardening.md](scheduler-hardening.md). This report records the
available evidence and defines the next measurable baseline. It does not claim
performance improvement or scheduler regression from package tests.

## 1. Regression triage

The six outcomes below are from isolated host library tests on
`x86_64-unknown-linux-gnu`, run serially with a 90-second per-package limit.
Raw logs and `summary.json` are in the ignored
`target/scheduler-b-evidence/host-libraries/` directory. All six packages are
outside the kernel scheduling implementation; none executes the guest scheduler.

| Package | Observed failure | Classification | Evidence boundary |
| --- | --- | --- | --- |
| `sunlight-libc` | 90-second timeout in `secret_store::tests::default_options_are_private_and_bounded`; earlier parallel sweep also stalled. | Environment/resource limitation (probable), scheduler regression unsubstantiated. | Log stops at the test name; no hang trace or isolated reproduction identifies the cause. Unknown remains possible. |
| `sunlight-utils` | 90-second timeout in `comm::tests::disjoint_files`. | Environment/resource limitation (probable), scheduler regression unsubstantiated. | Log stops during this test. Host process scheduling is unrelated to the guest scheduler; test-level deadlock/resource cause not established. |
| `sunlight-reminders` | `date_and_time_validation_is_strict` failed; 11 other tests passed. | Unrelated failure. | Package assertion concerns date/time validation; no scheduler code path is involved. It is not established whether this was pre-existing because no prior package baseline was recorded. |
| `sunlight-networkd` | Host library test compilation failed on missing `vec!` imports. | Unrelated failure. | Compile-time configuration/import error, before test execution; no scheduler code path. |
| `rappid-rabbit` | Host library test compilation failed on missing `parse_html`, `StyleContext`, and `collect_embedded_stylesheets` symbols in this configuration. | Unrelated failure. | Compile-time symbol/configuration mismatch, before test execution. |
| `sunlight-api-lab` | Three JSON pretty-printing expectation failures; 13 tests passed. | Unrelated failure. | Assertion output concerns JSON formatting. Not established as pre-existing absent a prior result. |

No item is classified as a confirmed scheduler regression. The timeout causes
and historical status of assertion failures remain unproven. Reproduction
commands are given below; the original run's full logs are preserved. Do not
change unrelated packages to improve the sweep score.

## 2. Instrumentation design and current status

Instrumentation is opt-in through `scheduler_perf_diag` and is disabled by
default. `kernel/src/sched/perf.rs` holds fixed-size 64-byte-aligned per-CPU
records, saturating counters, fixed logarithmic histograms, and field-wise
snapshot/reset APIs that never acquire scheduler/core locks. Histogram bounds
in nanoseconds are 1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096,
8192, 16384 and 32768; the final bucket represents larger values too. Report
percentiles as bucket ranges, not exact points. A multi-field snapshot is not
an instantaneous global cut.

Implemented coverage: schedule_tick invocations and selected dispatches,
context switches, reschedule requests and consumed bits, RR enqueue/dequeue/
steal counts, sampled queue depth/high-water, same-CPU runnable-wait histogram,
nice +8/+9/+10 dispatch counts and max runnable wait, plus timer Phase 1 global
scheduler-lock wait/hold histograms. Lock timing and runnable latency are
recorded only with calibrated TSC, and only same-CPU intervals are used. Hot
path updates allocate nothing, log nothing, block nowhere, and perform no
cross-CPU read-modify-write. `perf::observe_clock` checks repeated local
samples; latency and lock histograms remain disabled until the CPU has observed
two nondecreasing timestamps, and are permanently disabled for that CPU after
a regression.

Not implemented or not validated: exhaustive TSC monotonicity validation and
cross-vCPU offset/drift calibration, wake-to-dispatch latency, queue depth
after remote queue mutations, direct syscall/IPC/lifecycle lock timing, other
global scheduler-lock acquisition sites, and an in-guest report command. Idle
entry/exit and tasks moved by RR
stealing have counters, but elapsed idle distributions and non-steal migrations
are not measured. The `rearms` counter represents consumed requests and does not
separate coalescing causes. Nice wait is omitted when the runnable timestamp
originated on another CPU. Cross-CPU metrics are unavailable. Instrumentation
enabled/disabled overhead is not measured.

The implemented subset is not a complete scheduler-wide metric set. Complete
the uncovered metrics above before treating it as such; do not alter lock order
or add scheduler locks in C1.

## 3. Reproducible benchmark plan and results

There are **no scheduler throughput or latency measurements** for this source
state. `tools/scheduler_bench.py` is a QEMU capture harness: it records VM
metadata, raw serial logs and `[SCHED-PERF]` lines for repeated 1/2/4/8/12 CPU
runs. It does not create a workload guest; no deterministic benchmark guest
suite is implemented. All requested workload results, CPU-count performance
results, distributions, and enabled/disabled overhead are **not measured**.
The lifecycle gates are correctness evidence only.

Functional evidence collected 2026-10-18: standard Phase B fixture passed at
1, 2, 4, 8 and 12 vCPUs; the same fixture with `scheduler_perf_diag` enabled
passed at 1, 2 and 4 vCPUs. QEMU 11.1.1 used TCG (`thread=multi`), `-cpu max`,
1024 MiB, no NIC and serial-file output. The feature-enabled runner built
revision `49c2875f11591deadfc313514e5c1dc095e8beac` with a dirty worktree.
The command also used `-device isa-debug-exit,iobase=0xf4,iosize=0x04`,
`-no-reboot` and `-no-shutdown`. The fixture creates 2 tasks per CPU plus 6
owner/borrower fixtures and a replacement wave; this short lifecycle workload
is not a throughput configuration. Raw serial/QEMU logs are under
`target/scheduler-c1-functional/` (default feature) and
`target/scheduler-c1-perf-functional/` (diagnostics enabled), each with its own
`results.json`. The fixture is deterministic and uses no seed. These results do
not measure throughput or latency.

| Run | CPUs | Result | Evidence boundary |
| --- | --- | --- | --- |
| `python3 tools/test_scheduler_smp.py --cpus 1 2 4 8 12` | 1, 2, 4, 8, 12 | PASS | Lifecycle/correctness only; TCG |
| `python3 tools/test_scheduler_smp.py --perf-diag --cpus 1 2 4` | 1, 2, 4 | PASS | Instrumented lifecycle/correctness only; TCG |
| `python3 tools/test_scheduler_host.py` | host | PASS | Production state/lifecycle/accounting/policy tests |
| `cargo check --package sunlight-kernel --offline` | host build | PASS | Default feature set |
| `cargo check --package sunlight-kernel --features scheduler_perf_diag --offline` | host build | PASS | Diagnostic feature compiles |
| `cargo fmt --all -- --check`, `git diff --check` | source | PASS | Formatting/whitespace |

Run the capture harness with a benchmark ISO that emits meaningful guest
measurements:

```sh
python3 tools/scheduler_bench.py --iso target/scheduler-bench.iso \
  --accel tcg --cpus 1 2 4 8 12 --warmup 30 --duration 60 \
  --repetitions 5 --seed 1 --memory 1536M
```

It retains raw per-run serial data and `results.json`; it does not derive
statistics from missing guest metrics. Pin host affinity where supported and
record host load separately. Never compare TCG results to bare-metal
throughput.

Suggested matrix: (A) pinned CPU-bound workers; (B) CPU-bound plus timed
sleepers; (C) IPC ping-pong/fan-out; (D) bounded create/join churn; (E) timer
wake storms; (F) deliberately skewed preferred-CPU placement; (G) nice 0 and
+8/+9/+10 competing workers; (H) request/response latency during saturation;
(I) audio loopback/playback with underrun counters, only if capture/output
evidence is available. Keep task counts, payloads, timer intervals, affinities,
and random seeds identical across runs. Report operations/s, completed context
switches/s, wake and ready-wait p50/p95/p99/max, lock wait/hold distributions,
queue depth/imbalance, migrations, idle transitions, and nice-class maximum
wait/fairness. Audio success requires actual underrun/capture evidence, not a
boot marker.

Reproduce the package sweep and individual failures (the first command writes
new evidence under the ignored target directory):

```sh
python3 tools/test_scheduler_host.py --workspace-libraries --timeout 90
cargo test --package sunlight-libc --lib --target x86_64-unknown-linux-gnu --offline -- --test-threads=1
cargo test --package sunlight-utils --lib --target x86_64-unknown-linux-gnu --offline -- --test-threads=1
cargo test --package sunlight-reminders --lib --target x86_64-unknown-linux-gnu --offline -- --test-threads=1
cargo test --package sunlight-networkd --lib --target x86_64-unknown-linux-gnu --offline -- --test-threads=1
cargo test --package rappid-rabbit --lib --target x86_64-unknown-linux-gnu --offline -- --test-threads=1
cargo test --package sunlight-api-lab --lib --target x86_64-unknown-linux-gnu --offline -- --test-threads=1
```

These reproduce host package outcomes, not scheduler performance. The host
production scheduler policy/lifecycle/accounting test command
`python3 tools/test_scheduler_host.py` passed in this phase.

## 4. Global lock analysis

The active scheduler owns the process vector, online count, tick mirror, and
legacy state through one global `spin::Mutex<Scheduler>`. Per-CPU RR queues have
separate locks, but current scheduling acquires the global lock first, so the
queues do not provide parallel scheduling. Canonical order is global scheduler
lock then at most one core lock; two core locks are not held together.

Operations under the global lock include current-task/state transitions,
enqueue/dequeue and stealing bookkeeping, ownership/affinity validation,
quantum and nice aging updates, wake/block publication, timer/deadline scans,
process lookup and lifecycle/reaping decisions. Architecture state (CR3,
FS_BASE, FPU state and saved-frame ownership) is coordinated in dispatch while
the lock is held. Some callers additionally perform IPC shard operations,
allocator work, resource cleanup, page-table walks or logging while holding it;
these are candidates for critical-section shortening, but moving them requires
explicit lifetime snapshots and lock-order proofs.

Likely CPU-local data: active core's current index, queue head/tail, slice
accounting and reschedule bit. Cross-CPU shared data: process membership and
slot reuse, task lifecycle/ownership, affinity, shared resource borrower
counts, global timer deadlines, and publication of a runnable task to another
CPU. They cannot simply become local without a protocol for migration, wakeup,
exit/reap and stale indices.

Observed structural bottlenecks, not measured contention results:

1. Every CPU serializes on the same scheduler mutex; this is the primary
   scalability ceiling by design.
2. Placement scans online cores and locks their queue records sequentially.
3. BSP tick processing scans the process vector for timer/TTY services and
   performs wakeup work under the lock.
4. Reaping/resource cleanup and some syscall helpers can make lock hold time
   dependent on process count, mappings, or unrelated subsystem work.
5. Cross-CPU wakeups mutate shared task state and request masks while still
   depending on the global serialization point. Their actual cost is unknown.
6. Shared diagnostic atomics/counters and request masks may create cache-line
   traffic; no hardware counter or cache-coherence profile is available to
   confirm magnitude. Per-core alignment already exists for core state and
   handoff pins.

Proposed ownership contract for C2 design review (not implementation):

* Lock order: lifecycle/membership lock (if retained) -> one run-queue lock;
  never acquire two run-queue locks simultaneously. Define and audit ordering
  with IPC, VFS, PMM and address-space locks before introducing any second lock.
* Membership table owns generation-qualified task identity and slot reuse;
  run queues own only queue membership and local ordering. Queue entries must
  not become stale raw vector indices across removal/reuse.
* A CPU owns its current execution context and slice. Remote CPUs may request a
  reschedule or publish a generation-qualified wakeup, but cannot reclaim an
  executing stack or mutate local-only slice state.
* Migration is a transaction: remove source membership, preserve runnable
  ownership, publish destination membership, then notify destination. Define
  rollback and exit/reap races before enabling parallel queue mutation.
* Preserve Phase B handoff pin lifetime through physical stack pivot, the
  coalesced request consume/rearm contract, and exactly-once slice retirement.

## 5. Prioritized candidates

1. Implement deterministic guest workloads and validate TSC monotonicity plus
   cross-vCPU offset/drift; then measure the diagnostic-on/off overhead and the
   repeated raw performance matrix.
2. Add wake-to-dispatch, elapsed idle-duration and non-steal migration metrics
   with explicit clock-domain validity; expand lock timing beyond timer Phase 1.
3. Bound and move demonstrably expensive timer scans/reaper cleanup out of the
   global critical section using immutable snapshots and lifecycle references.
4. Remove avoidable repeated global lock acquisitions in callers that can safely
   share one locked transaction; measure lock wait/hold before and after.
5. Cache or incrementally maintain CPU queue load rather than locking every CPU
   for each placement, with invariant checks against actual queue lengths.
6. Design per-CPU run-queue ownership and narrow shared lifecycle locking only
   after the C0 workload exposes a measurable bottleneck and C2 race model is
   reviewed. No lock replacement is part of C1.

## 6. Correctness invariants to preserve

* A task is present at most once in a run queue; Ready/current/owner/queue
  metadata agree, and terminal or blocked tasks are not dispatched.
* Queue references remain valid across process-vector growth, slot reuse,
  teardown and stealing; affinity/context validation remains enforced.
* The outgoing stack stays pinned until execution has physically pivoted away;
  reaping and slot reuse reject any live handoff pin.
* Only the local CPU consumes its reschedule bit. Internal dispatch requeue
  does not create another request; a request published after consume remains
  pending. Wakeups are not lost between readiness check and wait registration.
* Each dispatch starts a fresh quantum; execution time is charged once, and
  blocked/idle time is never charged to a task or inherited by its successor.
* Nice +8 through +10 retain eventual queue admission under sustained
  competition; no CPU-count change may reintroduce starvation.
* Shared address-space/FD/resource borrowers are not reclaimed until all
  borrowers and stack handoffs have ended. IPC generation/deadline arbitration
  and authorization checks remain unchanged.
* Lock ordering has no cycles; IRQ/NMI restrictions and the no-two-run-queue
  locks rule remain explicit.

## 7. Proposed Phase C2 and rollback criteria

C2 should be a separately reviewed lock-ownership prototype, selected only
after C0/C1 measurements identify the dominant wait/hold path. Start with one
operation family (prefer enqueue/wakeup or placement), retain the global lock as
the lifecycle authority, and add invariant assertions plus targeted SMP stress
before expanding scope. Do not combine this with a policy, queue-class, or
quantum redesign.

Rollback the C2 change if any lifecycle/security/IPC gate regresses, queue and
owner invariants fail, nice +8..+10 maximum wait worsens beyond the agreed
baseline envelope, handoff/reaping failures appear, or repeated benchmark runs
show no improvement beyond observed run-to-run variability. Establish numeric
acceptance thresholds from the completed C1 distributions before C2 starts;
there are no valid numbers to set those thresholds today.

## 8. Unexecuted work and limitations

The guest workload suite (CPU-bound contention, sleepers, IPC, creation/teardown,
timer stress, placement skew, nice fairness, interactive saturation and audio
underrun), performance runs at every CPU count, cross-vCPU clock validation,
lock distributions, latency percentiles, medians/min/max, and diagnostic
overhead remain unmeasured. `scheduler_bench.py` is only the VM capture harness.
The SMP lifecycle gate is functional evidence and must not be called a
performance benchmark. Host scheduler policy/lifecycle/accounting tests passed;
default and feature-enabled kernel `cargo check` passed. No scheduler policy,
lock order, queue ownership, quantum, lifecycle, IPC or security behavior was
changed. Phase B stack handoff/deferred reclamation and reschedule contracts
remain intact by source review.
