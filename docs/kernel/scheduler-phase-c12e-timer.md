# Scheduler Phase C1.2E: timer responsiveness and mixed workloads

## Scope and measurement protocol

This extends [C1.2D](scheduler-phase-c12d-latency.md) with actual guest
observations of interrupt delivery, timer readiness, service execution, timer
RPC completion, lower-priority progress, and audio DMA under CPU contention.
Measurements use QEMU TCG, `thread=multi`, `-cpu max`, one vCPU, 1024 MiB,
and host affinity `{6}`. They do not establish bare-metal or KVM behavior.

The ordinary contention workload remains four synchronized native borrower
workers, the deterministic integer mix, one monotonic syscall per iteration,
seed 1, forward/reverse creation order, two 1-second warmups, and five
repetitions per order/duration at 5 and 10 seconds. CPU fairness is Jain's index
of **attributed CPU nanoseconds per trial**, averaged across trials. Throughput
is actual arithmetic-loop operations divided by reported elapsed time,
including completion tails. Dispatch counts are not a CPU-service proxy.

The new trace is bounded at 16,384 records per trial. Worker/service wait arrays
remain bounded at 2,048 samples. All trace access uses the existing global
scheduler lock. No trace allocation or serial emission occurs while collecting;
after measurement and verified borrower reaping, records are copied to heap
storage and printed. An overflow rejects the capture. Existing background
service logs, including boot and audio diagnostics, are retained.

Every trace timestamp is a calibrated same-CPU kernel clock observation.
Validation rejects uncalibrated clocks, nonzero observer CPU IDs, backwards
timestamps within or between trials, incomplete frames, missing IPC stages,
duplicate call identities, inconsistent client counts, wait overflow, invalid
worker state, and missing Reaped completion. This verifies the observed clock
domain and ordering; it does not establish the absolute accuracy of the TSC/PIT
calibration or synchronize clocks across CPUs.

Normal-suite startup is trials 1–4: two 1-second and the first two 5-second
trials. Steady state is trials 5–22. The mixed-suite startup is trials 1–3;
steady state is trials 4–8. Per-wake nice values remain recorded, so delayed
installation of the High timer policy is visible rather than inferred from
elapsed time. Startup samples remain in the raw evidence and per-trial CSV.

### Stages and endpoint semantics

The observer in [scheduler_timer_trace.rs](../../kernel/src/scheduler_timer_trace.rs)
records these separate points:

| Stage | Observation and limits |
| --- | --- |
| Interrupt observation | First Rust timer-handler timestamp, after the naked entry saved GPRs; successive entry gaps measure delivery cadence. |
| Deadline/tick publication | Immediately after the BSP queues/coalesces the 100 Hz timer notification. This is the kernel's timer notification, not a new userspace alarm API. |
| Timer readiness | Actual BlockedOnIpc-to-Ready transition; separate queue-publication records distinguish external admission from internal requeue. |
| Dispatch | Selected context after `schedule_tick` returns in Rust; the naked restore/IRET tail follows. |
| Service execution | Timer receive-to-reply/wait entry, with wall time and the service's attributed CPU delta recorded separately. |
| Reply completion | Generation-matched request enqueue, service receive, successful reply commit, and caller terminal-result consumption. |

The timer server exposes `GET_TICKS`, not an arbitrary deadline callback API.
RPC tests call that real endpoint using ordinary capability lookup and
`ipc_call_timeout`; no synthetic service or privileged bypass is substituted.
The client checks the returned `TimerMsg::REPLY` and records its actual
userspace round-trip maximum with the coarse monotonic clock. Fine-grained RPC
quantiles end at kernel consumption of the reply, just before the final syscall
return. Neither timestamp is described as a measured user instruction after
SYSRET. Tick-to-service and request-to-reply are separate observed chains; their
pooled percentiles must not be added together.

Notifications coalesce. The first pending publication measures accumulated
notification residence; the latest publication measures freshness at receive.
The endpoint payload is `latest_global_tick - base_tick + 1`, not a global tick.
The decoder verifies its constant offset from the latest published global tick
within and across trials. It does not join the endpoint-relative sequence to an
unrelated global tick. Pending notifications already present at a window edge
are censored where the complete chain cannot be observed.

### Correction to the C1.2D interpretation

`timekeeping::advance_global_tick` derives the global tick from elapsed TSC time
and catches up after delayed interrupts. Consequently C1.2D's
`deadline_max_ns < 8.04 ms` is a benchmark-relative **tick phase remainder**;
it is not independent proof that the hardware interrupt arrived within 8.04 ms
of its intended deadline. C1.2E retains that historical field but also records
handler-entry interarrival gaps and phase against delivered IRQ ordinal at
nominal 100 Hz. Nominal phase includes calibration/frequency drift and host
descheduling, so it is not a pure hardware deadline-miss measurement either.

In addition, regenerating the C1.2D service table from its retained `capture3`
gives 608/601 ten-second forward/reverse completed waits, rather than 496/496.
The corresponding p99/max are 263.47/364.78 ms and 273.59/324.28 ms. Comparisons
in this report use validated raw captures; the original table is retained as
historical documentation. The supplied fairness mean, 0.9962, is reproducible
from the C1.2D performance capture, while some throughput rows also differ from
the original prose table and are regenerated below.

## Reproduced cause and minimal change

The trace reproduces a remaining lost re-evaluation across *two* services:

1. A worker runs while the timer becomes Ready, publishing a reschedule request.
2. The existing first-eligible priority selector dispatches an earlier eligible
   high-priority service, such as TTY, mouse, audiod, or PTY. The timer remains
   Ready. Worker residual credit is correctly frozen.
3. That service blocks. Dispatch consumes its request and FIFO resumes the
   worker at the front with the correct residual entitlement.
4. The timer is already Ready, so subsequent coalesced ticks do not publish a
   new wake request. It can wait until the residual worker turn expires.

For example, baseline trace trial 5 contains a 301.59 ms timer wake wait.
At 19,589,188,554 ns, worker PID 43/nice 0 is interrupted; mouse PID 12/nice
−10 is selected first. After that service blocks and the worker resumes, the
next recorded dispatch opportunity is approximately 211 ms later. Other
earlier eligible services repeat the pattern. Timer execution itself is only
microseconds. Queue placement and the preserved worker turn are functioning;
the already-Ready priority task has lost its future re-evaluation request.

The production change is
[`retain_priority_request`](../../kernel/src/sched/mod.rs#L1646), called after
ordinary and idle dispatch. If the **existing** eligibility/priority selector
finds a strictly lower-nice Ready task than the newly selected task, dispatch
retains a coalesced reschedule bit for the next safe opportunity. It adds no
IPI or immediate recursive dispatch. Equal/lower-priority peers do not rearm
ordinary rotations. Selection order, admission, worker residual credit, quantum
feedback, stack handoff and reclamation paths retain their existing rules.

The extracted production regression fails without this change and passes with
it. It reproduces two waiting services, the first service's IPC block, a FIFO
worker resume, and then a scheduling opportunity **without a synthetic new
request**. It verifies exactly 180 ms of original worker entitlement and one
full-turn burst update. A second regression covers the idle-dispatch path.

No MLFQ, EEVDF, lock splitting, new scheduling class, quantum-size change,
special timer queue, or security/IPC authority change is introduced. The extra
eligibility scan is O(local queue length) under the existing scheduler/core
lock order; its cost must be assessed through the matched throughput captures.

## Results

### Repeatable timer latency

Two cold boots per policy passed the full 22-run suite and stage validator.
All steady-state timer wakes in these traces have nice −5. The following pools
completed **actual wake-to-selected-dispatch** samples across the two boots;
nearest-rank percentiles are in milliseconds. Five-second rows contain eight
steady trials per order; ten-second rows contain ten. Censored edge wakes are
reported in each `timer.json` and excluded from completed-sample percentiles.

| Seconds/order | Wakes before / after | p50 before → after | p95 before → after | p99 before → after | Maximum before → after |
| --- | ---: | ---: | ---: | ---: | ---: |
| 5 forward | 522 / 1,274 | 70.41 → 20.16 | 130.72 → 30.27 | 191.09 → 49.74 | 241.32 → 172.30 |
| 5 reverse | 515 / 1,268 | 70.41 → 20.16 | 110.66 → 30.24 | 211.16 → 43.33 | 291.58 → 115.98 |
| 10 forward | 1,210 / 3,182 | 77.27 → 20.16 | 133.05 → 30.23 | 281.50 → 40.28 | 372.05 → 137.83 |
| 10 reverse | 1,235 / 3,178 | 70.43 → 20.16 | 140.77 → 30.24 | 261.48 → 40.30 | 349.00 → 346.46 |

The separate queue-publication-to-selection results match closely: ten-second
p50/p99 before are 77.27/281.50 ms forward and 70.43/261.47 ms reverse; after
they are 20.16/40.27 and 20.16/40.29 ms. Thus the improvement is also present at
the actual readiness transition, not created by internal requeue accounting.

Cold-boot repeatability, ten-second actual wake waits:

| Boot/order | Baseline p50 / p99 / maximum | Candidate p50 / p99 / maximum |
| --- | --- | --- |
| 1 forward | 73.05 / 281.50 / 341.88 | 20.16 / 40.28 / 137.83 |
| 1 reverse | 73.90 / 273.63 / 349.00 | 20.16 / 40.32 / 346.46 |
| 2 forward | 80.37 / 271.54 / 372.05 | 20.16 / 40.29 / 70.56 |
| 2 reverse | 70.42 / 261.47 / 341.89 | 20.16 / 40.29 / 90.72 |

The evidence supports lower steady median/p95/p99 dispatch latency. It does
**not** support a hard maximum or uniform improvement of every extreme tail.
Startup queue-wait maxima are 931.64 ms before and 935.11 ms after; the timer
still initially runs at nice 0 until niced installs its High policy.

### Interrupt, publication, and execution separation

Each end of a range below summarizes one creation order pooled across both
boots; maxima span both orders. All values are in milliseconds.

| Stage | Baseline p50 / p99 | Candidate p50 / p99 | Largest baseline / candidate observation |
| --- | --- | --- | --- |
| IRQ entry interarrival | 10.054 / 15.39–17.79 | 10.067 / 21.76–21.85 | 84.84 / 275.21 |
| Entry → notification publication | 0.0106–0.0112 / 0.052–0.062 | 0.0126–0.0129 / 0.057–0.086 | 16.65 / 20.23 |
| Publication → actual Ready | 0.0014 / 0.0035–0.0048 | 0.0012 / 0.0035–0.0056 | 0.0179 / 0.0205 |
| First pending publication → tick receive | 70.46–77.29 / 261.51–281.53 | 20.173–20.174 / 40.29–40.32 | 372.08 / 350.18 |
| Latest coalesced publication → tick receive | 0.046–0.047 / 0.786–0.829 | 0.039 / 0.158–0.879 | 2.76 / 3.51 |
| Selected dispatch → tick receive | 0.018 / 0.035–0.045 | 0.0145 / 0.034–0.058 | 1.65 / 2.76 |
| Receive → reply/wait entry, wall | 0.0072–0.0073 / 0.014–0.017 | 0.0052 / 0.012–0.020 | 0.599 / 0.032 |
| Same service interval, attributed CPU | 0.0074–0.0075 / 0.014–0.017 | 0.0053 / 0.012–0.020 | 0.0367 / 0.0316 |

Service execution is not the hundreds-of-milliseconds steady bottleneck. More
frequent service opportunities increase completed timer wakes by about 2.6×;
the unchanged service body executes in small fragments. The small execution
differences are observations, not a claim of faster service code. IRQ cadence
p99 is worse in the candidate traces, and a long dispatch interval remains.
Neither the historical phase metric nor these observations justify claiming
improved hardware interrupt delivery.

## Mixed workload protocol

Each mixed boot runs eight fresh four-worker cohorts. Trials 1–2 last one second;
trials 3–8 last ten seconds. Odd trials use four nice-0 workers. Even steady
trials use nice `[0,+8,+9,+10]` in slot order and reverse creation order. The
coordinator issues real timer RPCs, then blocks on a private endpoint's 100 ms
receive timeout before the next request. This simultaneously exercises IPC
deadline expiry under saturation and bounds offered request/trace demand.

The unpaced pilot is retained as invalid evidence: its nice-worker trials
produced thousands of RPCs and overflowed the bounded trace. The validator
rejected it; its truncated distributions are excluded from acceptance. Both
final policies use the same paced workload ELF.

Where available, the coordinator requests a 440 Hz, at most five-second tone
after releasing the CPU workers, then reads audiod status after the trial.
QEMU has Intel HDA, `hda-micro`, and the silent `none` audio backend. Counters
demonstrate DMA progress and reported underruns; they do not demonstrate
audibility or perceptual quality. Startup endpoint/device unavailability is
recorded, not treated as playback success.

RPCs fully inside the four-worker workload are additionally classified using
the guest monotonic tick domain and lifecycle records, with a full tick margin
at both edges. Fine-grained durations still use the same-CPU trace clock.
Requests or completions in startup/worker completion tails remain in the full
distribution and are separated from that saturation subset.

## Functional and safety regressions

The candidate's `scheduler_b_test,scheduler_perf_diag` lifecycle image passed:

| vCPUs | Deferred reaps | Reused slots | Exit races | Result |
| ---: | ---: | ---: | ---: | --- |
| 1 | 5 | 1 | 1 | PASS |
| 2 | 6 | 2 | 1 | PASS |
| 4 | 7 | 4 | 1 | PASS |
| 8 | 9 | 8 | 1 | PASS |
| 12 | 12 | 12 | 1 | PASS |

These are functional lifecycle results, not SMP performance measurements.
The 2-vCPU `phase_sec` gate passed with its full 54,664-byte serial stream
retained. Markers cover capability forgery/isolation/revocation, message and
pointer validation, bounded IPC/backpressure/reuse, notification coalescing,
timeout cancellation, terminal generations/arbitration, nameserver identity,
deferred replies, deadline wakes/races/late replies, and 64 iterations of SMP
terminal-state stress. Existing memory/security expectations also passed.

The focused host runner passed 48 Rust tests plus the independent policy/queue
probes. Its extracted production service/accounting binary passed 22 tests,
including the two new request-retention regressions, expiration-boundary
priority, frozen/fractional residual credit, exactly-once CPU charging, full-turn
feedback, equal-nice service fairness, lower-priority progress, and I/O/yield
handling. Architecture instructions and physical reclamation remain stubbed in
that extraction; the guest lifecycle gate supplies their functional coverage.
The default kernel check and 19 Python validator tests passed. Changed Rust
files pass direct `rustfmt --check`, and `git diff --check` passes.

## Evidence and reproduction

All generated evidence remains under ignored `target/scheduler-c12e/`: complete
serial streams, QEMU commands, accelerator/CPU/memory configuration, host
affinity/load, ISO hashes, exact image inputs, build logs, source snapshots,
validation output, regression logs, and the rejected pilot. Individual service
and worker records are retained, including censored counts and overflow flags.

Each image's `manifest.json` identifies its exact workload, kernel, ISO and
source hashes. Matched normal before/after images use the same frozen workload
ELF; matched mixed images use the same paced workload ELF. The normal frozen
ELF differs from the original C1.2D embedded workload, so historical work rates
are context, while the new matched-policy captures isolate this scheduler
change with an identical workload. Throughput acceptance uses images with the
new stage trace disabled; the established bounded worker/service CPU-and-Ready
observer remains enabled, just as in C1.2D's performance capture.

The build helper captures exact inputs and can build a current-source image:

```sh
python3 tools/scheduler_timer_build.py --mode trace --output target/c12e-repro/trace
python3 tools/scheduler_timer_build.py --mode mixed --output target/c12e-repro/mixed
python3 tools/scheduler_timer_build.py --mode performance --output target/c12e-repro/performance
# Use the same frozen workload on both policy builds with --bench-elf PATH.
taskset -c 6 python3 tools/scheduler_bench.py --suite \
  --iso target/c12e-repro/trace/sunlightos.iso --cpus 1 --warmup 0 \
  --duration 900 --repetitions 1 --memory 1024M --output target/c12e-repro/trace/capture1
python3 tools/scheduler_timer.py target/c12e-repro/trace/capture1/1cpu-run1.serial.log \
  --output target/c12e-repro/trace/capture1/timer.json
taskset -c 6 python3 tools/scheduler_bench.py --mixed --audio \
  --iso target/c12e-repro/mixed/sunlightos.iso --cpus 1 --warmup 0 \
  --duration 900 --repetitions 1 --memory 1024M --output target/c12e-repro/mixed/capture1
python3 tools/test_scheduler_smp.py --cpus 1 2 4 8 12 --perf-diag \
  --evidence-dir target/c12e-repro/correctness
SUNLIGHT_TEST_SERIAL_LOG=target/c12e-repro/security.serial.log bash tools/test.sh phase_sec
python3 tools/test_scheduler_host.py
python3 -m unittest discover -s tools -p 'test_scheduler*.py'
```

For a source-matched baseline, apply only the inverse of the production
`retain_priority_request` change to an isolated source tree, retain the same
observer, and supply the same frozen workload. The measured before/after source
trees are preserved beneath each image's `source/`; `build-wave.py` and
`run-wave.py` preserve this session's exact build and run ordering.

## Remaining limitations

Candidate trace boot 1, trial 18 retains a 346.46 ms timer wake wait. A
274.77 ms portion lies between a dispatch-decision observation for blocked
PID 1 and the selected-context observation inside the reschedule handler.
No new hardware IRQ entry is observed during that interval. The dispatch path
includes architecture work and opportunistic reclamation under disabled
interrupts, and host descheduling can extend any interval. These observations
do not identify which operation caused the stall. It is not silently excluded
as startup or blamed solely on hardware. This phase does not establish a hard
maximum-latency guarantee or justify a lock/reclamation redesign.

Two cold boots per final policy/workload provide repeatability evidence, not
statistical confidence or exhaustive interleaving proof. Host CPU frequency,
other host load and TCG translation affect work rates. The workload includes
kernel/interrupt service in attributed CPU time and performs a syscall per
arithmetic operation. Instrumentation changes execution cost and code layout.
The selected-dispatch observation precedes physical IRET; service receive and
caller result consumption delimit later stages separately.

There is no general minimum CPU share under continuously runnable
higher-priority demand. Nice +8/+9/+10 progress is tested for this finite mixed
workload. General live nice/affinity changes, cross-CPU timing/SMP performance
fairness, hardware audio, input-to-visible latency, and an arbitrary timer alarm
API are outside these measurements. The global scheduler lock remains intact
and has no new worst-case hold-time guarantee.
