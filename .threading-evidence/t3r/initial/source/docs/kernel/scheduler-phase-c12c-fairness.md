# Scheduler Phase C1.2C: isolated CPU-service fairness correction

Measured 2026-10-18. This phase preserves per-CPU RR FIFOs, the global SCHEDULER
mutex, work stealing, stack handoff pins, deferred reclamation and reschedule
rearming. It adds no scheduling class, MLFQ, EEVDF or lock restructuring.
The unchanged [C1.2B benchmark](scheduler-phase-c12b-fairness.md) measures CPU
service independently of dispatch opportunities.

## Root cause and correction

C1.2B trial 9 recorded dispatches `[59,59,58,58]` but CPU service
`[547.06,3985.21,3681.20,1732.09]` ms. The baseline path was
`timer_rust → wake_pid → enqueue_ready → queue_process_to_core(..., true)`.
Every Ready publication set a per-CPU reschedule request. `schedule_tick`
interpreted every request as an unconditional rotation: it closed execution and
slice clocks, moved the interrupted worker to the FIFO tail and dispatched a
fresh budget next time. Periodic background wakes therefore destroyed some
workers' unconsumed turns. Full-turn workers received burst +32 while their
interrupted peers did not. Dispatch counts concealed that feedback loop.
The dispatcher also called the voluntary short-burst churn helper on forced
preemption, which could further misclassify an involuntary short fragment.

The correction makes a request mean **re-evaluate**. For an affinity-eligible
Running RR task with an unexpired slice, equal/lower-priority Ready publications
leave its queue position, runtime interval and slice untouched. Priority here
has one explicit rule: **strictly lower nice is higher wake priority**. Burst
score continues to determine quantum length; it does not confer permission to
interrupt equal-nice peers. The first eligible local FIFO candidate with lower
nice can interrupt immediately at a scheduling opportunity. The priority
candidate is removed from its existing queue entry and selected directly,
rather than rotating unrelated peers ahead of the service that caused the wake.
This priority dispatch bypasses the ordinary nice admission-counter visit.
Existing busy-CPU notification/timer fallback timing is unchanged.

A priority-interrupted worker retains one frozen entitlement in `slice_credit`,
then resumes at the front of its local FIFO after the priority task relinquishes.
This entitlement stores the original budget and cumulative consumed ns/fallback
ticks; it does not add another quantum. Resumption uses `Option::take`, ignores
new quantum calculations and bypasses repeated nice admission/debt/aging visits.
Sub-tick consumption survives repeated interruptions and migration; time spent
waiting on another task is excluded. FullQuantum feedback, counter decay and
quantum-override retirement occur once when the whole turn ends. Expiration
returns the worker to the tail. Constructors initialize credit to None;
non-runnable state transitions and teardown clear it. Existing selector affinity,
context, ownership and remote stack-pin checks also protect priority selection.

## Preemption semantics

| Event | Slice/service budget | Queue and feedback |
| --- | --- | --- |
| Quantum expiration | Retire frozen turn once; next turn gets normal calculated budget | Requeue at tail; FullQuantum once, normal nice housekeeping |
| Equal/lower-priority background wake | Keep the running turn and open runtime interval | Enqueue awakened task normally; consume request without rotating current |
| Higher-priority Ready candidate | Close actual execution interval; save only this turn's residual | Select priority candidate; interrupted worker resumes ahead of peers; no voluntary churn or premature policy feedback |
| Voluntary yield | Discard residual and end turn | State Ready forces dispatch; tail rotation; existing syscall churn checkpoint remains |
| Block/sleep, including wake before physical park | End turn; no sleep-time credit | Non-runnable task leaves queue; raced Ready wake still forces owner handoff; later wake gets fresh budget |
| Exit/kill/owner teardown | Close runtime once, discard entitlement | Never requeue terminal task; existing pin/reaper/borrower rules retained |
| Affinity-invalid current | Force normal deschedule | Existing eligibility/steal behavior retained; no residual-position guarantee for affinity mutation |

`cpu_runtime_ns` accounting is unchanged: checkpoints advance the open execution
interval and closing it charges only its remaining tail. Preserved entitlement
is policy state, never extra CPU runtime. The Running forced-dispatch path now
calls `account_current_runtime`, not the voluntary churn helper. Ready/blocked
states retain dispatch-side voluntary feedback for I/O/futex paths that do not
checkpoint themselves. Callers that already checkpointed have `last_start_ns`
cleared, so the dispatcher cannot apply that feedback twice. The TSC clock is used when calibrated;
without calibration only delivered timer ticks can measure service. That fallback
cannot resolve sub-tick execution and is tested as such.

## Measurement protocol

The final comparison uses 1 vCPU, QEMU TCG `thread=multi`, CPU model `max`, 1024
MiB, no NIC, serial capture, seed 1 and host CPU-6 affinity. Each image runs the
same synchronized cohort suite: one 1-second trial per creation order, then five
repetitions of 5 and 10 seconds in both forward/reverse creation order, 22 trials
and 88 workers per image. Worker code, barrier, nice, affinity, instruction mix,
clock calls, lifecycle observation and strict validators are unchanged.

The original C1.2B evidence remains in `target/scheduler-c12c/historical-before/`.
A source-isolated baseline build restores HEAD scheduler, slice and Process
sources and adds only the same feature-gated service-ready observation hook as
C1.2C. The observer records timer/TTY Ready-publication-to-selection waits, with
bounded storage and overflow reporting, under the existing scheduler guard;
no hot-path allocation or UART output is introduced. It does not measure timer
deadline-to-IPC-reply or input-to-visible latency. Wake observations near a trial
boundary may complete during its worker completion tail; still-pending service
waits at report time are censored. Distributions describe completed wakes.

The first corrected suite is retained as `pilot-after/`; it overlapped builds
and lifecycle checks and is excluded from the final throughput comparison.
The final corrected suite ran after all final builds/gates; the unchanged
baseline image was then rerun immediately, without another build. The labels
"before" and "after" identify scheduler policies, not chronological capture order.
This final control followed a large throughput fall between corrected captures;
earlier baseline/corrected captures are retained as additional variability evidence.
The five repetitions are fresh cohorts within each boot, not independent cold
boots. TCG service accounting, host load, coarse 10 ms workload deadlines,
initial burst history and completion tails remain limitations of comparison.

## Final CPU-service and throughput measurements

Both final captures passed all 22 trials, with 88 eligible nice-0 workers and
complete wait/dispatch traces per image. All workers progressed and were Reaped;
no trace overflow or invalid scheduling-point observation occurred. All 20 long
corrected trials started with burst `[304,304,304,304]`; the baseline had 16 such
trials. Its other initial histories were `[304,304,304,352]`, `[304,304,352,352]`,
`[352,304,304,304]` and `[352,352,304,304]`. The overall long-run CPU Jain mean rose
from **0.7896 to 0.9948**; individual trial ranges were **0.6400–0.8836** before
and **0.9908–0.9996** after. Jain is computed per trial, then averaged.

| Seconds | Order | CPU Jain before → after (after min–max) | Work Jain before → after | ops/s before → after |
| ---: | --- | --- | --- | --- |
| 1 | forward | 0.7315 → 0.9022 (0.9022–0.9022) | 0.7392 → 0.8984 | 1264599 → 1218558 |
| 1 | reverse | 0.8995 → 0.9009 (0.9009–0.9009) | 0.9152 → 0.8793 | 984272 → 1344809 |
| 5 | forward | 0.7650 → 0.9920 (0.9908–0.9944) | 0.7671 → 0.9916 | 1288997 → 1162323 |
| 5 | reverse | 0.8032 → 0.9921 (0.9916–0.9929) | 0.8014 → 0.9734 | 1352962 → 1120739 |
| 10 | forward | 0.8391 → 0.9973 (0.9954–0.9994) | 0.8402 → 0.9985 | 1364823 → 1084731 |
| 10 | reverse | 0.7512 → 0.9978 (0.9956–0.9996) | 0.7493 → 0.9974 | 1423939 → 1083912 |

The 1-second results are single observations; the other rows each contain five
repetitions. Variability below is population SD, not a confidence interval.
Largest/smallest is the per-trial CPU-service ratio averaged across repetitions.

| Seconds | Order | CPU Jain SD before → after | Mean CPU max:min before → after | First-created CPU share before → after (mean ± SD, %) |
| ---: | --- | ---: | ---: | --- |
| 5 | forward | 0.0696 → 0.0013 | 7.14 → 1.26 | 6.08 ± 0.37 → 26.70 ± 0.18 |
| 5 | reverse | 0.0431 → 0.0005 | 6.52 → 1.26 | 6.16 ± 1.24 → 26.75 ± 0.09 |
| 10 | forward | 0.0328 → 0.0015 | 5.08 → 1.13 | 7.33 ± 1.64 → 26.97 ± 0.99 |
| 10 | reverse | 0.0299 → 0.0014 | 5.31 → 1.11 | 8.38 ± 2.03 → 26.84 ± 0.76 |

CPU shares still reflect finite-window RR ordering: the first-created worker
gets about 26.7–27.0%, rather than exactly 25%. Maximum initial entry lag rose
from 630 to 580 ms; maximum deadline-observation lag was 90 ms before and 250 ms
after. Identical CPU service is not promised for arbitrary initial burst history.

The representative **refreshed trial 9**, in slot order `[0,1,2,3]`:

| Image | Operations | CPU ms | Dispatches | Final burst |
| --- | --- | --- | --- | --- |
| Baseline | 829618, 3324302, 5520285, 5072738 | 563.01, 2216.57, 3796.30, 3360.54 | 61, 61, 60, 60 | 304, 368, 528, 464 |
| Corrected | 3159461, 2874934, 2853381, 2789147 | 2704.35, 2438.33, 2426.16, 2367.64 | 137, 57, 49, 43 | 624, 584, 604, 564 |

The corrected dispatch counts are deliberately **unequal**: priority service
interruptions split some workers' turns into many small dispatches. They now
resume with the same residual entitlement ahead of peers. Service, rather than
dispatch count, is much closer to equal. Full-turn burst feedback is no longer
systematically withheld from the repeatedly interrupted worker.

All per-trial work counts, CPU ns, dispatches, runnable wait totals and exact
percentiles, burst histories and lifecycle outcomes are in the
[refreshed baseline CSV](scheduler-c12c-before-workers.csv) and
[corrected CSV](scheduler-c12c-workers.csv), 88 rows each. The original
[C1.2B CSV](scheduler-c12b-workers.csv) remains unchanged. The comparison JSON
retains per-group CPU-share variability and per-trial fairness data remains in
the validated capture JSONs.

Throughput below uses actual reported elapsed time, including logical completion
tails; normalized throughput divides by the measured workers' summed CPU time.
These are observed changes between sequential boots, not hardware speedup claims.

| Seconds | Order | M operations/s before ± SD | After ± SD | Change | M operations / worker CPU-second before → after |
| ---: | --- | ---: | ---: | ---: | ---: |
| 5 | forward | 1.289 ± 0.120 | 1.162 ± 0.246 | -9.83% | 1.325 → 1.197 |
| 5 | reverse | 1.353 ± 0.086 | 1.121 ± 0.332 | -17.16% | 1.388 → 1.148 |
| 10 | forward | 1.365 ± 0.074 | 1.085 ± 0.368 | -20.52% | 1.388 → 1.103 |
| 10 | reverse | 1.424 ± 0.043 | 1.084 ± 0.263 | -23.88% | 1.447 → 1.101 |

The 5-second reverse-order work Jain mean (0.9734) remains below its CPU Jain
mean (0.9921). Corrected long-trial per-worker work/CPU-second rates ranged
0.366–1.439 million, versus 1.084–1.553 million before. CPU runtime includes
attributed kernel/interrupt work and TCG/host timing, so equal CPU time need not
produce identical loop counts. The long-group throughput changes ranged from
-23.88% to -9.83%. The 5-second reverse group changed by -17.16%, alongside a
normalized-rate change of -17.28%.
There is no supported claim that reduced switch overhead improves throughput.

The host's 1/5/15-minute loads were [3.22, 3.33, 2.77] → [2.59, 3.15, 2.81] for baseline and [3.60, 2.59, 2.40] → [6.65, 3.75, 2.84] for corrected. Builds and test VMs finished before these captures, but host load/frequency and TCG timing were not controlled.

Additional capture variability (20 long trials per wave, arithmetic mean CPU
Jain; throughput is total operations divided by total reported elapsed time):

| Capture | Long-run CPU Jain mean | Aggregate M operations/s |
| --- | ---: | ---: |
| Historical C1.2B | 0.7960 | 1.480 |
| Earlier refreshed baseline | 0.7906 | 1.456 |
| Final baseline control | 0.7896 | 1.370 |
| First corrected pilot | 0.9950 | 1.559 |
| Intermediate corrected | 0.9950 | 1.434 |
| Final corrected | 0.9948 | 1.101 |

The pilot overlapped builds/VMs; the intermediate image preceded the direct-I/O
feedback refinement. They are excluded from the final paired table. Both new
baseline controls reproduce poor CPU-service fairness, while every corrected
capture has mean CPU Jain above 0.99. Work-rate variability is much larger than
CPU-share variability. The baseline rerun reduced the apparent throughput gap
but did not remove it: the final corrected image still measured 9.83–23.88%
lower in the four long groups. Host/TCG conditions and code effects were not
separated by a controlled frequency/load experiment; a scheduler-induced
throughput cost remains an unresolved risk, not a demonstrated optimization.

## Latency tradeoffs

Pooled individual worker Ready-wait percentiles, in ms (nearest rank):

| Seconds | Order | p95 before → after | p99 before → after | Max before → after |
| ---: | --- | ---: | ---: | ---: |
| 5 | forward | 303.05 → 684.76 | 466.84 → 745.46 | 573.28 → 775.57 |
| 5 | reverse | 272.68 → 674.72 | 337.94 → 745.26 | 443.77 → 765.37 |
| 10 | forward | 261.81 → 785.43 | 312.95 → 896.16 | 443.83 → 957.61 |
| 10 | reverse | 261.72 → 785.57 | 322.13 → 896.11 | 443.89 → 946.65 |

Corrected pooled worker p50 waits are only 0.055–0.065 ms, dominated by immediate
resumes after short higher-priority service dispatches. They must not be read
as shorter waits between whole RR turns; p95/p99 waits increased substantially.

Observed service Ready-publication-to-selection waits, pooled over five trials
per group, in ms:

| Seconds | Order | Service | Samples before / after | p50 before → after | p95 before → after | p99 before → after | Max before → after |
| ---: | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 5 | forward | timer_server | 148 / 32 | 100.75 → 809.03 | 426.57 → 933.33 | 505.31 → 977.70 | 748.13 → 977.70 |
| 5 | forward | tty_server | 144 / 188 | 91.14 → 60.44 | 422.84 → 483.35 | 503.08 → 869.52 | 748.18 → 913.25 |
| 5 | reverse | timer_server | 166 / 33 | 100.73 → 797.98 | 293.11 → 946.55 | 433.78 → 946.72 | 443.72 → 946.72 |
| 5 | reverse | tty_server | 158 / 218 | 90.59 → 60.44 | 292.02 → 402.78 | 412.77 → 717.04 | 443.75 → 857.19 |
| 10 | forward | timer_server | 392 / 54 | 80.76 → 936.83 | 292.77 → 1198.52 | 413.62 → 1228.63 | 443.77 → 1228.63 |
| 10 | forward | tty_server | 379 / 401 | 80.52 → 60.45 | 281.84 → 463.21 | 392.62 → 735.03 | 443.81 → 839.04 |
| 10 | reverse | timer_server | 429 / 55 | 80.54 → 936.61 | 292.88 → 1218.56 | 353.11 → 1238.77 | 443.86 → 1238.77 |
| 10 | reverse | tty_server | 403 / 417 | 70.51 → 60.45 | 281.84 → 473.33 | 323.15 → 665.77 | 442.95 → 907.06 |

Timer-service median latency rose from 81–101 ms to 798–937 ms, with maximum
1.239 s. These are material responsiveness costs. TTY median latency was about
60 ms after correction, while its tail also increased in some groups. The
normal service boot includes `niced`: its
[defaults](../../services/sunlight-niced/src/config.rs#L101) configure TTY High
(nice -5), and its [monitor](../../services/sunlight-niced/src/monitor.rs#L182)
applies configured nice values after daemon startup. The serial stream places
that startup within corrected trial 3. Thus the TTY pool includes boot-time and
later priority-service observations; **per-wake service nice values were not
traced**, so it is not an all-equal-nice latency experiment. This source-backed
interpretation also explains many short worker fragments with preserved turns.
The timer is not a configured High service in those defaults.

The isolated fairness acceptance is demonstrated by CPU-time Jain improvement.
No general desktop/audio latency acceptance is inferred from that result. The
unchanged benchmark retains normal service startup, dynamic priority policies
and background activity; neither image suppresses them or resets worker bursts.

## Safety validation

Focused production host tests, all 13 Python validator tests, default kernel
check, combined `scheduler_guest_bench,scheduler_perf_diag` check, workspace Rust
formatting and whitespace checks passed. Existing dependency warnings remain;
no workspace-wide host-library pass is claimed.

| Guest CPUs | Deferred direct-reap attempts | Reused slots | Exit races | Lifecycle result |
| ---: | ---: | ---: | ---: | --- |
| 1 | 5 | 1 | 1 | PASS |
| 2 | 6 | 2 | 1 | PASS |
| 4 | 8 | 4 | 1 | PASS |
| 8 | 8 | 8 | 1 | PASS |
| 12 | 12 | 12 | 1 | PASS |

The ordinary `./tools/test.sh phase_sec` 2-CPU gate also passed. Its markers
cover memory W^X/NX and rollback, borrower teardown/reuse, capability isolation,
IPC queue/backpressure/coalescing, timeout cancellation, terminal arbitration,
generation identity, late replies and 64 iterations of SMP terminal-state stress.
No pin, naked IRQ handoff, deferred reclamation, reschedule-bit protocol or
security implementation was relaxed. The inactive BORE mode was not enabled or
measured.

## Source and regression coverage

- [Queue publication](../../kernel/src/sched/mod.rs#L642),
  [runtime and turn retirement](../../kernel/src/sched/mod.rs#L1004),
  [RR admission/resume](../../kernel/src/sched/mod.rs#L1481),
  [priority eligibility and dispatch](../../kernel/src/sched/mod.rs#L1615),
  [frozen entitlement arithmetic](../../kernel/src/sched/quantum.rs), and
  [Process storage/initialization](../../kernel/src/process/mod.rs#L263)
  contain the correction. [ProcessYield](../../kernel/src/arch/x86_64/syscall.rs#L1981)
  still marks Ready and performs its voluntary checkpoint before requesting
  the SYSRET reschedule IPI. Linux sched_yield translates to this same syscall.
- [scheduler_service_probe.py](../../tools/scheduler_service_probe.py) extracts
  the **production** dispatcher, selector, queue publication and runtime/slice
  helpers into a [host fixture](../../tools/tests/scheduler_service.rs).
  Eight dispatch tests cover equal-nice CPU workers under 100 Hz background
  readiness, fractional interruption/resume, full expiration/tail rotation,
  voluntary yield/churn, block/sleep/wake/termination, duplicate enqueue,
  eligibility and nice-only priority, burst saturation with existing aging,
  +8..+10 progress under higher-priority wakes, frozen overrides and avoiding
  repeated debt/admission on a resumed turn, and direct versus already-checkpointed
  I/O blocking feedback. Architecture instructions, physical
  stack switching, stealing and reclamation are stubbed in this host fixture;
  those results are not presented as guest or SMP performance measurements.
- Six production Slice unit tests include sub-tick interruptions, CPU migration
  excluding wait time, frozen budget despite changed quantum, duplicate close,
  stale/idle ticks, expiration and the uncalibrated fallback. Four existing
  cumulative CPU-accounting tests also run in the extracted binary (18 tests).
- The focused host runner retains the pin/request/timekeeping/foreground scope,
  runtime, production queue and all-nice policy probes. Its frozen Phase A
  reproduction still demonstrates +8..+10 starvation; current policy passes
  all supported nice values at burst 0/256/1024. Nice-0 FIFO selection remains
  balanced in that independent admission test.
- The unchanged framing/suite validators pass their 6 and 4 tests. Three added
  service-evidence validator tests accept complete or historical traces and
  reject missing/duplicate/overflowed/invalid latency records.

## Reproduction and evidence

The corrected image can be built and captured with the C1.2B commands, substituting
`target/scheduler-c12c-repro` for the evidence directory. The userspace `schedbench`
ELF remains byte-identical to C1.2B; its SHA256 is
`d1a5fef8be88053863be42dbbf8de95988412512ea1fc2a5a3f8a6da3a7821e0`.
For a baseline, use revision `bfb23e322afb3b4042a9604ec4b63f3705f678c7` in an
isolated source tree, apply the retained `baseline-observer.patch`, and build
with the same userspace artifacts and `scheduler_guest_bench` feature.
The source-isolated baseline used here is retained as `baseline-source/kernel/`;
other workspace sources were symlinked to the unchanged workspace, not copied.

```sh
python3 tools/test_scheduler_host.py
python3 -m unittest discover -s tools -p 'test_scheduler_bench.py'
python3 -m unittest discover -s tools -p 'test_scheduler_fairness.py'
python3 -m unittest discover -s tools -p 'test_scheduler_service_compare.py'
python3 tools/test_scheduler_smp.py --cpus 1 2 4 8 12 \
  --evidence-dir target/scheduler-c12c-repro/correctness
./tools/test.sh phase_sec
# Build both images before running either performance capture; use an available host CPU.
taskset -c 6 python3 tools/scheduler_bench.py --suite \
  --iso target/scheduler-c12c-repro/sunlightos.iso --accel tcg --cpus 1 \
  --warmup 0 --duration 900 --repetitions 1 --seed 1 --memory 1024M \
  --output target/scheduler-c12c-repro/after
python3 tools/scheduler_fairness.py target/scheduler-c12c-repro/after/1cpu-run1.serial.log
python3 tools/scheduler_service_compare.py \
  target/scheduler-c12c-repro/before/1cpu-run1.serial.log \
  target/scheduler-c12c-repro/after/1cpu-run1.serial.log \
  --output target/scheduler-c12c-repro/comparison.json
```

`target/scheduler-c12c/` retains both complete final serial streams, exact QEMU
commands and host loads in `before/results.json` and `after/results.json`,
validated worker/wait/lifecycle JSON and CSV, service-latency comparison JSON,
both measured ELFs/ISOs, build/check logs, baseline observer patch, source
snapshots/manifest and SHA256 evidence checksums. `correctness/` retains its
separate guest images/hashes and full serial logs. `security-2cpu.log` is the
security gate's expected-marker console transcript; that existing script removes
its temporary full serial log. The pilot, intermediate corrected, earlier refreshed baseline and historical
captures are retained separately and excluded from final comparison. The intermediate corrected image
preceded the final direct-I/O blocking-feedback regression; its images and source
snapshot are retained separately as well. Generated evidence stays under
ignored `target/`; the documentation and worker CSVs are the reviewable results.

## Remaining risks and limits

- This corrects one isolated CPU-service failure. Adaptive 50–500 ms quanta and
  existing aging still permit finite-window imbalance and unequal initial burst
  histories. Equal nice is not a guarantee of identical instantaneous shares.
  The unchanged benchmark is syscall-heavy and instrumented TCG work, not a
  hardware or pure userspace instruction-throughput result.
- Equal-priority service wakes now wait for admitted RR turns. With four CPU
  workers, four maximum quanta alone can contribute roughly 2 seconds of Ready
  wait, plus other queued tasks, scheduler overhead and delivery delays. This is
  not a hard latency bound. Lower-nice readiness still interrupts at the next
  scheduling opportunity, but sustained higher-priority demand has no hard
  minimum-share guarantee for lower-priority tasks. No interactive/audio SLA or
  input-to-visible acceptance is claimed. Kernel deadline expiration and fatal
  signal checks still run in `tick` before dispatch gating.
- Priority re-evaluation scans the local FIFO under the existing scheduler/core
  locks. The extra scan and resume-front maintenance are O(local queue length).
  Critical sections, resource cleanup and the global lock remain unbounded by
  this phase; no lock-contention improvement is claimed.
- Credit follows a task if stolen/migrated, but preserving its position relative
  to peers on a different CPU is not promised. The 1/2/4/8/12-CPU gate is lifecycle
  safety evidence, not an SMP CPU-service fairness experiment. General live
  affinity/nice mutation, non-dense APIC topology, real hardware/KVM, mixed
  UI/audio/IPC workloads and end-to-end IPC latency are unmeasured.
- The uncalibrated clock fallback counts delivered ticks and cannot retain
  sub-tick service accurately. TSC synchronization, restore/IRET attribution,
  observer overhead and host/TCG variability retain the prior limitations.
  Host extraction tests stub architecture and reclamation; guest stress is not
  exhaustive interleaving proof. Phase A's waitpid completion/slot-reuse hazard
  and the previously documented unrelated workspace test issues remain separate.
