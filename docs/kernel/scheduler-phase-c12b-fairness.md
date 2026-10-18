# Scheduler Phase C1.2B: single-vCPU fairness and repeatability

Measured on 2026-10-18. The synchronized benchmark still exposes a CPU-service
fairness problem. Nice-0 RR admission/rotation works, but equal dispatch counts
do not imply equal CPU time: periodic service wakeups repeatedly truncate some
workers' turns, discard their remaining budgets, and move them to the tail.
Burst feedback then gives longer quanta to workers that complete full turns.
Creation order changes which slot suffers. No scheduler implementation, quantum,
aging, or lock policy was changed in this phase.

## Experiment and contracts

The original 560 ms pilot's counts `[12540, 31845, 315164, 383319]` have Jain
fairness 0.55758 and a largest/smallest ratio of 30.57. Its framing validator
proved completion, not fairness or equal runnable exposure. The old benchmark
already waited for four READY arrivals; it did **not** simply start measuring
while workers were still being created. However, it released START before
publishing STOP, allowing a preemption between the two stores to expose a zero
deadline. That is a real publication hazard, not an observed explanation for
this pilot. The original slot-dependent seed and rotate count also prevented a
strictly identical computation comparison.

The revised benchmark:

- Registers each worker with an observer, then publishes its READY arrival.
  The coordinator requires all four registrations/arrivals before measuring.
  STOP is stored first, then START with release ordering; workers acquire START
  before reading STOP. This is a one-shot barrier per cohort. Single-CPU waits
  yield so all participants can arrive. Waits have a 30-second guest timeout.
- Uses nice 0 for the coordinator, inherited nice 0 for every worker, and checks
  every worker's nice and affinity. All four masks are `u64::MAX`; with one
  online vCPU their identical effective affinity is `{CPU 0}`. No affinity or
  scheduler policy override is necessary. Every worker uses the same seed,
  rotate-by-7 instruction mix, clock check per operation, and progress-store
  cadence. Work counts are arithmetic-loop iterations, including one monotonic
  syscall per iteration, not retired instructions or pure userspace CPU time.
- Records creation, READY, first workload entry, deadline observation, observer
  completion, and verified kernel Reaped completion. The coordinator waits for
  syscall 156 to confirm all four Reaped before reusing shared measurement state.
- Runs 1 second once per order, then five repetitions each of 5 and 10 seconds
  per order, alternating forward `[0,1,2,3]` and reverse `[3,2,1,0]` within each
  repetition: 22 trials per guest boot. The seed remains 1. TIDs verify creation
  rank independently of slot labels. These are fresh worker cohorts within a
  boot, not five independent cold boots. The suite intentionally replaces the
  legacy short duration passed by the feature-gated boot launcher.

Production RR in `kernel/src/sched/mod.rs` is FIFO `pop_front` / `push_back`.
Candidates must be Ready, affinity-eligible, dispatchable, and not executing on
another CPU. Nice 0 bypasses debt/credit skips; it does not fix quantum length.
For nice 0, quantum = `5 + floor(burst_score * 45 / 1024)` ticks, with 10 ms per
tick: 50–500 ms. Normal threads start at burst 256 (160 ms). Barrier yields can
alter burst history, so initial and final burst values are recorded rather than
silently reset. The report checks actual initial histories in the long trials.
The contract guarantees rotation/admission, not equal accumulated CPU service.

## Observation and validation

`kernel/src/scheduler_bench_diag.rs` is compiled only with
`scheduler_guest_bench`. Its private syscall registers/baselines/completes
workers; post-dispatch hooks in the existing interrupt paths observe selections
without changing decisions. `kernel/src/sched/` is unchanged. Observation uses
the existing SCHEDULER guard and scheduler read accessors, retaining the existing
SCHEDULER-to-core lock order. No lock class or lock policy was added.

Per worker, the observer retains existing scheduler CPU-runtime deltas, selected
dispatch counts, every ready-to-selected wait, nominal quantum/burst at selection,
and observed selected-to-descheduled duration. Dispatch counts include a final
selection that may only notice the expired deadline and exit. CPU time includes
kernel execution attributed to the worker. Waits include the initial wait from
barrier release and all subsequent Ready waits, up to dispatch. No worker blocks
or yields in its measured loop. Eligibility checks at scheduling observations
require Ready/Running, valid context, CPU-0 affinity, nice 0, and no pending exit.
These checks are scheduling-point observations, not an instruction-by-instruction
state trace. All checked workers remained eligible and were subsequently Reaped.

The hot observation hook allocates and logs nothing. Fixed global storage holds
2048 wait/dispatch records per worker; reset initializes it in place, and final
reporting copies directly to bounded heap storage after measurement. This avoids
large kernel-stack temporaries. Instrumentation overhead was not independently
calibrated; these are instrumented TCG measurements, not a bare-metal throughput
claim.

WAIT `reason` is a bitmask: 1 = request pending before timer bookkeeping,
2 = request pending after `sched.tick` (this includes internal wakes, **not just
quantum expiration**), 4 = timer_server was blocked and is being woken,
8 = tty_server was blocked and is being woken, 32 = reschedule IPI path,
64 = worker reached its wall-clock deadline. Bits can coexist. A trace with
reason 4 or 8 and neither bit 1 nor bit 2 directly identifies an external wake
that caused the rotation. Recorded durations determine whether the nominal
quantum had expired.

The existing `tools/scheduler_bench.py::validate_guest_report` validates every
START/SAMPLE/END frame, run identity and sample indices. `--suite` reuses that
validator for all 22 frames. `tools/scheduler_fairness.py` additionally rejects
missing worker/wait/lifecycle data, overflow, unequal affinity, wrong creation
order, early completion, non-runnable observations, count inconsistencies, or
unreaped workers. It computes Jain fairness, population CV, max/min ratios, exact
nearest-rank wait percentiles, and per-slot share mean/SD across repetitions.
A value of 1 for Jain fairness means equal shares.

The workload deadline uses the existing coarse 10 ms monotonic clock; scheduler
CPU/wait intervals use the calibrated TSC-based ns clock. Their endpoints differ
by coarse-clock sampling, scheduling and completion tails. The reported elapsed
ms includes the coordinator observing all logical completions, but excludes
subsequent reap polling and report emission. Per-worker entry/exit lags are in
the CSV; do not interpret counts as though all workers entered at the same
instant or use completion-tail wall time as CPU time.


## Measured results

Primary run: QEMU 11.1.1, TCG `thread=multi`, `-cpu max`, **1 vCPU**, 1024 MiB,
no NIC, display disabled, full serial-file capture. QEMU inherited host CPU-6
pinning. The 16-logical-CPU host's 1/5/15-minute loads were
`[1.96, 2.04, 1.80]` at start and `[1.44, 1.79, 1.75]` at end. Host load and TCG
wall-clock accounting can change work/CPU-second rates; compare CPU service and
counts separately. No throughput speedup claim is made. The original first
complete suite overlapped the correctness VM sweep; the final suite did not.
The two kernel check commands finished during the final VM's boot, before its
first measurement.

All 22 final trials passed the capture validator, with 88 worker records and
complete individual wait/dispatch traces. All 20 long trials showed strong
CPU-service skew. Eighteen of those trials began with burst `[304,304,304,304]`;
trial 3 began `[352,304,304,304]` and trial 4 `[304,352,352,352]`. Thus unequal
barrier history is visible but is not required for the measured effect.
All measured worker masks were identical and all nice values remained zero.
No checked worker became ineligible or failed to reap. Maximum initial workload
entry lag was 380 ms, and maximum deadline-observation lag was 70 ms.

Repetition summary (population SD; percentages below are shares of total work).
Jain statistics here are calculated per trial and then averaged, not calculated
from averaged worker counts:

| Duration | Order | Trials | Work Jain mean (min–max) | CPU Jain mean | Mean largest/smallest | First-created work share mean ± SD |
| --- | --- | ---: | --- | ---: | ---: | ---: |
| 1 s | forward | 1 | 0.7380 (0.7380–0.7380) | 0.7378 | 31.10 | 1.22% ± 0.00 pp |
| 1 s | reverse | 1 | 0.4371 (0.4371–0.4371) | 0.4397 | 28.18 | 2.55% ± 0.00 pp |
| 5 s | forward | 5 | 0.7973 (0.7447–0.8337) | 0.8015 | 6.10 | 7.27% ± 2.91 pp |
| 5 s | reverse | 5 | 0.7699 (0.7270–0.8275) | 0.7705 | 6.37 | 6.77% ± 1.52 pp |
| 10 s | forward | 5 | 0.8043 (0.7528–0.8951) | 0.8043 | 5.97 | 7.16% ± 1.89 pp |
| 10 s | reverse | 5 | 0.8031 (0.7705–0.8261) | 0.8078 | 6.52 | 5.96% ± 0.34 pp |

Per-trial counts and selected dispatches, in **slot order 0,1,2,3**. Reversing
creation order changes rank, not the slot labels used in this table:

| Trial | Seconds | Rep | Order | Worker operations | Worker dispatches | Work Jain |
| ---: | ---: | ---: | --- | --- | --- | ---: |
| 1 | 1 | 1 | forward | 22810, 703438, 709447, 440974 | 3, 3, 3, 2 | 0.7380 |
| 2 | 1 | 1 | reverse | 1318851, 418926, 47305, 46797 | 4, 6, 5, 5 | 0.4371 |
| 3 | 5 | 1 | forward | 1173128, 1140921, 3249760, 3495675 | 65, 65, 65, 65 | 0.8060 |
| 4 | 5 | 1 | reverse | 2591455, 1201729, 2929811, 393726 | 31, 32, 32, 32 | 0.7493 |
| 5 | 10 | 1 | forward | 1486144, 3959186, 4158078, 4607956 | 109, 110, 109, 109 | 0.8951 |
| 6 | 10 | 1 | reverse | 5560660, 4204750, 4378072, 912276 | 65, 65, 65, 65 | 0.8261 |
| 7 | 5 | 2 | forward | 381671, 2984418, 1710378, 2067667 | 30, 29, 29, 29 | 0.7851 |
| 8 | 5 | 2 | reverse | 1788456, 2030355, 3099933, 442862 | 32, 32, 33, 33 | 0.7911 |
| 9 | 10 | 2 | forward | 837724, 6097263, 5702160, 2611192 | 59, 59, 58, 58 | 0.7528 |
| 10 | 10 | 2 | reverse | 6277410, 2932795, 5024554, 868177 | 65, 65, 65, 66 | 0.7705 |
| 11 | 5 | 3 | forward | 439858, 2200536, 2388868, 2513365 | 32, 32, 33, 32 | 0.8337 |
| 12 | 5 | 3 | reverse | 2886460, 3143391, 727290, 729427 | 50, 50, 50, 50 | 0.7270 |
| 13 | 10 | 3 | forward | 888201, 4259297, 3606681, 6613865 | 62, 62, 61, 61 | 0.7802 |
| 14 | 10 | 3 | reverse | 5213048, 3437213, 4512200, 754538 | 59, 59, 60, 60 | 0.8081 |
| 15 | 5 | 4 | forward | 487123, 2400699, 1503165, 2527188 | 38, 38, 38, 38 | 0.8169 |
| 16 | 5 | 4 | reverse | 2153146, 1450632, 3331513, 443924 | 33, 32, 32, 33 | 0.7548 |
| 17 | 10 | 4 | forward | 877804, 3923170, 6073786, 3857462 | 64, 63, 63, 63 | 0.7987 |
| 18 | 10 | 4 | reverse | 5125814, 6059942, 3227599, 971326 | 70, 71, 71, 71 | 0.7958 |
| 19 | 5 | 5 | forward | 395798, 3440353, 1484121, 2284333 | 29, 29, 28, 28 | 0.7447 |
| 20 | 5 | 5 | reverse | 2376239, 1871919, 2732368, 488775 | 36, 37, 37, 37 | 0.8275 |
| 21 | 10 | 5 | forward | 1245233, 2823386, 4981820, 6330838 | 83, 82, 82, 82 | 0.7948 |
| 22 | 10 | 5 | reverse | 4641587, 3636969, 5761874, 940781 | 67, 67, 67, 68 | 0.8149 |

The [88-row worker CSV](scheduler-c12b-workers.csv) records counts, CPU ns,
dispatches, total waiting time, p50/p95/p99/max waiting, entry/exit lag, initial
and final burst, nice, affinity, and lifecycle outcome for **every worker in
every final trial**. [Parsed measurements](../../target/scheduler-c12b/final-capture/1cpu-run1.fairness.json)
retain every wait and selected run duration as well as per-trial Jain/CV/max:min
for work, CPU time and dispatches. Across the long trials, dispatch counts
differed by at most one between workers, despite much larger CPU-time gaps.

Pooled individual Ready-wait distributions below are in ms; these include each
worker's initial wait and use nearest-rank percentiles, with no histogram
estimation. Per-worker percentiles and waiting totals are in the CSV:

| Seconds | Order | Wait observations | p50 ms | p95 ms | p99 ms | Max ms |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | forward | 11 | 372.46 | 549.64 | 549.64 | 549.64 |
| 1 | reverse | 20 | 181.86 | 373.42 | 374.57 | 374.57 |
| 5 | forward | 772 | 70.51 | 252.66 | 372.60 | 474.20 |
| 5 | reverse | 734 | 70.55 | 262.58 | 294.79 | 434.08 |
| 10 | forward | 1499 | 70.55 | 281.96 | 342.22 | 524.38 |
| 10 | reverse | 1311 | 80.54 | 282.00 | 323.12 | 453.96 |

Trial 9 is a direct control for initial burst history: every worker began at
burst 304 with a nominal 180 ms budget. CPU times in slot order were
`[547.06, 3985.21, 3681.20, 1732.09]` ms, against dispatch counts
`[59,59,58,58]`. Slot 0 ended at burst 304; peers ended at `[496,560,400]`.
Its first five selected intervals lasted `[4.13, 9.02, 9.87, 9.12, 9.03]` ms,
all with reason 6 (tick-stage request plus timer-server wake), rather than
consuming the nominal 180 ms. Slot 1's fourth interval lasted 181.14 ms;
its next nominal budget rose to 190 ms.

Across all 20 long trials, every nonterminal selected interval of the
first-created worker ended before its nominal budget. Its pooled selected
interval median is computed below; this includes the terminal deadline check.
Intervals with pure timer-server or TTY wake reasons (4/8/12) confirm the
wake-to-rotation path without a request already pending at the tick stages:

- First-created intervals: 1086, including 1066 nonterminal intervals; median selected duration 9.92 ms.

- Trial 3, slot 1, dispatch index 1: reason 4, selected duration 10.15 ms versus nominal 180 ms.

- Trial 3, slot 2, dispatch index 1: reason 8, selected duration 9.78 ms versus nominal 190 ms.


Bit-2-only short intervals are also present: `sched.tick` contains internal
IPC/timer/futex/poll timeout wake paths as well as expiration. Their exact
woken PID is not retained by this observer. They must not all be labeled
quantum expirations; the duration/budget evidence rules that out for short
intervals. The confirmed external timer/TTY path is sufficient to establish
that unrelated readiness can truncate a CPU-bound worker's unexpired turn.

## Root cause and subsequent-phase proposal

The evidence separates three mechanisms:

1. **Benchmark hazards:** the old release ordering was unsafe, computations
   depended on slot, the 500 ms window covered very few nominal quanta, and the
   validator only checked a complete frame. These are corrected or made visible.
   The old benchmark did already have an arrival barrier. There is no evidence
   that its deadline race actually fired in the original successful pilot.
2. **RR dispatch fairness:** all four workers stay runnable and get closely
   matched dispatch opportunities. There is no measured queue loss, nice skip,
   affinity exclusion, worker creation failure, or lifecycle failure. Short
   windows and differing first-entry times matter, but cannot explain persistent
   multi-second CPU-runtime differences after identical initial burst scores.
3. **CPU-service fairness:** `timer_rust` wakes blocked timer_server/TTY tasks.
   `wake_pid` → `enqueue_ready` → `enqueue_process_to_core` →
   `queue_process_to_core(..., true)` sets the CPU's reschedule bit.
   `schedule_tick` consumes that request, stops the current execution clock and
   slice, changes Running to Ready, puts the current worker at the FIFO tail,
   and picks the next task. The new dispatch always gets a fresh slice from
   `start_charging_runtime`. There is no retained service entitlement or
   compensation for the unused portion of an unrelated wake-preempted turn.
   The awakened service is often still behind other workers, so it cannot block
   and wake again until later in that same rotation. This creates a repeating
   queue phase: workers near the service's next wake lose turns, while peers
   consume full quanta. Reversing creation order moves that phase to another
   worker slot. Full-quantum consumers gain burst +32; repeatedly interrupted
   workers generally do not. The quantum formula amplifies the service gap.

This is a reproducible scheduler CPU-service fairness problem under the normal
background service workload, rather than a failure of FIFO admission itself.
Equal dispatch counts satisfy the narrower RR rotation contract, but do not
satisfy the desired equal-workload CPU fairness. The exact original pilot has
no per-dispatch trace, so its individual preemptions cannot be reconstructed;
the corrected guest reproduces its class of skew and provides the causal trace.

A focused next-phase experiment should distinguish an **unrelated Ready
publication** from **expiration, explicit yield, block, exit, and required
priority preemption**. In nice-0 RR, an unrelated equal-priority wake should
leave an eligible current task's unexpired turn intact; idle CPUs and tasks
that must stop should still dispatch immediately. Limit the change to the local
wake/request decision and prove it against this unchanged benchmark. Define the
priority rule explicitly before retaining immediate preemption for higher
priority wakes. Measure timer/TTY/IPC wake latency alongside CPU fairness: an
unexpired RR quantum can be as long as 500 ms, so that latency tradeoff needs an
explicit acceptance bound. Keep quantum calculation, aging and locking outside
that experiment. Merely retaining the unused budget while still moving the
same repeatedly interrupted task to the tail would not by itself compensate
its lost CPU service. No proposed fix is implemented here.

## Evidence and correctness

All complete raw serial streams, VM commands, ISO/serial hashes, parsed records,
source snapshots, and build/test output remain under
`target/scheduler-c12b/`. In particular:

- `original-pilot/`: unchanged copy of the original successful C1.2A capture.
- `capture/`: first complete, valid 22-run synchronized suite, with a smaller
  observer that did not yet retain slice durations and wake-stage reasons.
- `trace-capture/`: rejected diagnostic attempt, including its double fault
  inside the new observer syscall. Large fixed-array stack temporaries were
  replaced with in-place reset and a heap snapshot after measurement.
- `trace2-capture/`: all 22 trials finished, but trial 21 had 137–138 dispatches
  per worker and exceeded the 128-record trace limit. The strict validator
  rejected it. Its raw/aggregate evidence is preserved, not relabeled as a
  complete trace. The final observer's limit is 2048 records per worker.
- `final-capture/`: primary complete trace and validated 22-run suite used in
  the tables and CSV. No trace overflow or eligibility errors occurred.
- `correctness/`: existing Phase B guest lifecycle gate, PASS at 1, 2, 4, 8,
  and 12 vCPUs. This is functional SMP coverage, not an SMP fairness result.
- `host.log`, `validator-tests.log`: focused production scheduler
  lifecycle/accounting/policy/queue tests passed; 6 framing and 4 suite test
  methods passed, including subcases that reject corrupted/incomplete evidence.
- `check-default.log`, `check-diagnostics.log`, `fmt.log`: default kernel and
  combined `scheduler_guest_bench,scheduler_perf_diag` checks, Rust formatting,
  and whitespace checks passed. No workspace-wide host-library pass is claimed.
- `source-manifest.json`, `SHA256SUMS`, `source/`: the measured source snapshot,
  kernel/userspace binary hashes, and evidence hashes. Scheduler source hashes
  match HEAD; policy, quantum, aging and lock implementation files are unchanged.

Reproduce from the repository root (no privileged disk image is needed):

```sh
mkdir -p target/scheduler-c12b-repro
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
  cargo build --package sunlight-bench --bin schedbench --release \
  --target x86_64-unknown-none --offline
cargo build --package sunlight-kernel --features scheduler_guest_bench --offline
bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel \
  target/scheduler-c12b-repro/sunlightos.iso target/limine "$PWD"
taskset -c 6 python3 tools/scheduler_bench.py --suite \
  --iso target/scheduler-c12b-repro/sunlightos.iso --accel tcg --cpus 1 \
  --warmup 0 --duration 900 --repetitions 1 --seed 1 --memory 1024M \
  --output target/scheduler-c12b-repro/capture
python3 tools/scheduler_fairness.py \
  target/scheduler-c12b-repro/capture/1cpu-run1.serial.log
python3 tools/test_scheduler_host.py
python3 -m unittest discover -s tools -p 'test_scheduler_bench.py'
python3 -m unittest discover -s tools -p 'test_scheduler_fairness.py'
python3 tools/test_scheduler_smp.py --cpus 1 2 4 8 12 \
  --evidence-dir target/scheduler-c12b-repro/correctness
```

Choose an available host CPU in place of 6 where necessary. `--duration 900`
is the host capture timeout; guest intervals are the fixed 1/5/10-second suite.
The harness stops when the suite completes, fails, or faults, and fails closed
on invalid evidence. Host affinity is inherited by QEMU. Repetitions and seed
are provided by the suite itself; suite mode rejects other host repetition,
seed or CPU settings. The ordinary capture mode retains its single-frame API.
