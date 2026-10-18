# Scheduler Phase C1.2D: timer-service latency and fairness

Follow-up: [C1.2E measurements and corrections](scheduler-phase-c12e-timer.md)
reproduce the retained raw captures, distinguish actual wakes from queue
insertion, and explain why the tick-phase metric below is not independent
hardware interrupt delivery lateness. That report also lists regenerated
values where the retained captures differ from this historical table.

Measured on October 18, 2026 on one vCPU with QEMU TCG (`thread=multi`,
`-cpu max`, 1024 MiB, serial capture, host CPU 6). The comparison uses the
validated C1.2B and C1.2C captures already retained under `target/`, plus two
candidate captures: an uninstrumented performance capture and an instrumented
stage trace. The workload, seed, creation orders, 22-run suite, and validators
are the C1.2C protocol.

The candidate preserves C1.2C CPU fairness and removes the specific timer tail
caused by consuming a higher-priority wake on the same tick that the current
slice expires. The smallest scheduler change is in
[`schedule_tick`](../../kernel/src/sched/mod.rs#L1635): a higher-priority Ready
candidate is considered even when the current slice is expired. An expired
current turn is still retired normally; only an unexpired turn receives the
existing frozen residual credit. `timer_server` is now present in the existing
protected High service policy (`nice=-5`) in
[`load_defaults`](../../services/sunlight-niced/src/config.rs#L94).

## Reproduction and stage separation

C1.2B already showed equal dispatch counts with unequal CPU service. C1.2C
fixed that by preserving residual turns, but its service trace showed timer
Ready-to-selection waits up to 1,238.77 ms. The relevant path is:

```text
BSP 100 Hz tick
  -> send_timer_tick (coalesced endpoint notification)
  -> wake_pid(timer_server)
  -> enqueue_ready / local RR queue
  -> reschedule request
  -> schedule_tick
  -> higher-priority candidate selection
  -> timer_server dispatch
```

The C1.2D observer records each stage under the existing scheduler lock. The
deadline hook is in [`timer_rust`](../../kernel/src/arch/x86_64/interrupts.rs#L1012);
readiness and dispatch accounting are in
[`scheduler_bench_diag`](../../kernel/src/scheduler_bench_diag.rs#L139).
`deadline_max_ns` measures the calibrated-clock observation against the
benchmark-relative 10 ms tick schedule. `waits_ns` measures Ready publication
to selected dispatch. `cpu_ns` is the service's scheduler-attributed runtime
delta from the same benchmark interval.

For the instrumented candidate, deadline observation lateness stayed below
8.04 ms in every long group. It is therefore not the source of the hundreds of
milliseconds of service delay. The service is runnable for almost the whole
interval but executes for only a few milliseconds per 5–10 second group, which
matches its blocking IPC loop. The latency is Ready-to-dispatch residence.

| candidate group | timer deadline samples | deadline max | timer Ready waits | wait p50 / p95 / p99 / max | timer CPU time (mean/worker group) |
| --- | ---: | ---: | ---: | --- | ---: |
| 5 s forward | 3,430 | 3.76 ms | 330 | 81.04 / 253.29 / 782.56 / 953.93 ms | 3.09 ms |
| 5 s reverse | 3,490 | 3.17 ms | 430 | 70.95 / 162.15 / 283.56 / 364.78 ms | 5.07 ms |
| 10 s forward | 3,947 | 8.04 ms | 496 | 71.00 / 131.84 / 263.47 / 364.22 ms | 9.10 ms |
| 10 s reverse | 3,985 | 6.64 ms | 496 | 71.02 / 131.75 / 243.18 / 293.85 ms | 6.38 ms |

The first three 1-second trials include service startup and niced policy
installation; their timer maxima were 731–954 ms. Steady-state 10-second runs
were below 365 ms. Those startup observations remain in the pooled values and
are not silently removed.

The deadline clock is a TSC-derived measurement relative to a coarse 100 Hz
tick. QEMU TCG host load, TSC calibration, interrupt entry, and serial/report
tails affect absolute throughput and small timestamp differences. The stage
separation is valid because the deadline and Ready timestamps are sampled in
the kernel, while service CPU time uses the existing exactly-once accounting
clock. It is not a bare-metal latency claim.

## Root cause

C1.2C correctly made an equal-or-lower-priority wake a re-evaluation rather
than an unconditional FIFO rotation. It also dispatched a strictly lower-nice
Ready task directly and preserved an interrupted worker's residual entitlement.
The missing boundary was the current-slice expiration case:

1. A timer wake made `timer_server` Ready and set the CPU reschedule bit.
2. On the same timer interrupt, the worker slice expired.
3. C1.2C only searched for a priority candidate inside the *unexpired* slice
   branch, so it consumed the request and performed ordinary FIFO rotation.
4. The timer service was already Ready, so later 100 Hz notifications did not
   publish another wake request for it.
5. It remained behind several 180–250 ms worker turns. This explains the
   repeated 0.7–1.2 s Ready residence without blaming the deadline publisher.

The candidate changes only step 3. If a strict-lower-nice Ready candidate exists,
it is selected at the expiration boundary. The current worker's expired turn
goes through normal tail requeue and FullQuantum feedback. If the turn is still
unexpired, C1.2C's `slice_credit` path remains unchanged. No MLFQ, EEVDF, lock,
queue-class, stack-handoff, or reclamation redesign was introduced.

## Queue, preemption, and feedback trace

The trace records the existing `reason` mask on every worker dispatch. Bits are
1=request pending before tick bookkeeping, 2=request pending after
`sched.tick`, 4=timer-service wake, 8=TTY wake, 32=reschedule IPI, and
64=deadline completion. Bits may coexist. Queue placement remains the local
per-CPU RR FIFO; the priority candidate is removed from that queue and selected
directly. An interrupted unexpired worker is reinserted at the front with its
bounded residual credit; an expired worker is reinserted at the tail.

Across the long candidate groups, the worker dispatch counts and trace-derived
residual-slice utilization were:

| group | worker dispatch records | timer-wake reason observations | TTY-wake observations | nominal-slice utilization |
| --- | ---: | ---: | ---: | ---: |
| 5 s forward | 1,311 | 318 | 299 | 11.0% |
| 5 s reverse | 1,662 | 424 | 389 | 9.0% |
| 10 s forward | 1,921 | 495 | 460 | 8.5% |
| 10 s reverse | 1,925 | 494 | 466 | 8.6% |

This lower utilization is expected after direct service dispatches: workers are
split into more dispatch fragments, while their residual entitlement prevents
the fragments from becoming extra turns. FullQuantum burst feedback is applied
once per completed turn. Worker burst histories, nice values, affinity masks,
and reaping remained valid in all 88 long-trial records. The guest benchmark
does not emit the core's raw context-switch counter, so the report does not
invent a context-switch total; dispatch records and reason bits are the
validated preemption evidence available from this workload.

## Three-version comparison

The C1.2B and C1.2C values below are the final paired distributions from the
C1.2C report. Candidate fairness and throughput use the uninstrumented
C1.2D performance capture so observer overhead is not mixed into throughput.
Candidate stage latency uses the separate instrumented capture described above.

| duration/order | A: C1.2B CPU Jain | B: C1.2C CPU Jain | C: candidate CPU Jain | A ops/s | B ops/s | C ops/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 5 s forward | 0.7650 | 0.9920 | 0.9928 | 1.289 M | 1.162 M | 1.259 M |
| 5 s reverse | 0.8032 | 0.9921 | 0.9924 | 1.353 M | 1.121 M | 1.187 M |
| 10 s forward | 0.8391 | 0.9973 | 0.9997 | 1.365 M | 1.085 M | 1.197 M |
| 10 s reverse | 0.7512 | 0.9978 | 0.9998 | 1.424 M | 1.084 M | 0.989 M |

The candidate's mean long-run CPU Jain is 0.9962 across these four groups,
versus 0.9948 for C1.2C and 0.7896 for C1.2B. Candidate throughput varies by
TCG run and host load; it is within −8.8% to +10.3% of the paired C1.2C values.
No throughput improvement claim is made.

Representative 10 s forward trial 9, slot order 0–3, shows the policy effect:

| version | worker CPU ms | worker dispatches | final burst |
| --- | --- | --- | --- |
| A: C1.2B | 563, 2,217, 3,796, 3,361 | 61, 61, 60, 60 | 304, 368, 528, 464 |
| B: C1.2C | 2,704, 2,438, 2,426, 2,368 | 137, 57, 49, 43 | 624, 584, 604, 564 |
| C: candidate | 2,507, 2,464, 2,405, 2,462 | 142, 117, 91, 116 | 624, 624, 584, 604 |

The candidate keeps CPU service equal while allowing service-driven dispatch
fragments. The C1.2B and C1.2C worker CSVs and the candidate serial captures are
the source records:

- [`scheduler-c12b-workers.csv`](scheduler-c12b-workers.csv)
- [`scheduler-c12c-workers.csv`](scheduler-c12c-workers.csv)
- [`candidate capture`](../../target/scheduler-c12d/candidate/capture2/1cpu-run1.serial.log)
- [`instrumented stage capture`](../../target/scheduler-c12d/candidate/capture3/1cpu-run1.serial.log)

## Acceptance criteria from verified baselines

These are distribution-relative criteria, not guessed scheduler constants:

- Every 5 s and 10 s group must have CPU Jain at least the lowest C1.2C long
  group, 0.9920. Candidate: 0.9924–0.9998, pass.
- Timer Ready-wait p95, p99, and maximum must be no worse than the corresponding
  C1.2C measured group values (p95 426.57–1,198.52 ms, p99 337.94–1,218.56
  ms, maximum 443.72–1,238.77 ms). Candidate instrumented values are
  p95 131.75–253.29 ms, p99 243.18–782.56 ms, and maximum 293.85–953.93 ms,
  pass.
- All worker traces must retain bounded residual accounting, valid nice/affinity,
  no overflow, and verified Reaped completion. The 22-run candidate suite
  passed these checks.
- The High timer-service host regression must show service progress within the
  existing 200 ms host-fixture bound while all four equal-nice workers receive
  runtime. It passed; this is a regression bound for the extracted production
  logic, not a guest SLA.

The deadline stage has no C1.2B/C1.2C baseline distribution because those
versions did not record it. The candidate trace establishes that deadline
observation lateness is single-digit milliseconds; future work should retain
that trace before tightening a separate deadline criterion.

## Regression and safety evidence

The focused host suite passed 20 extracted scheduler/accounting tests,
including the new expiration-boundary priority test, bounded timer-service
progress with sustained worker runtime, residual-credit tests, equal-nice
fairness, nice +8/+9/+10 progress, queue admission, and exactly-once CPU
accounting. Changed Rust files pass direct `rustfmt --check`; the workspace
`cargo fmt --all -- --check` remains blocked by pre-existing unrelated format
drift in `kernel/build.rs` and several service files.

The default kernel check passed. The lifecycle gate passed at 1, 2, and 4
vCPUs with deferred reaping, slot reuse, exit-race, stack-handoff, and timer
invariants. `./tools/test.sh phase_sec` passed the 2-vCPU security/IPC gate,
including capability isolation, bounded IPC, timeout/deadline cancellation,
terminal generation arbitration, borrower teardown/reuse, and SMP terminal
stress.

## Remaining risks

The candidate still has finite-window startup tails and TCG variability. The
timer service is protected High priority and can interrupt equal-nice workers;
the existing residual-credit and +8/+9/+10 progress contracts remain covered by
host tests, but sustained higher-priority demand still has no general minimum
share guarantee for lower-priority tasks. The service observer adds measurable
instrumentation overhead, so its throughput is excluded from the performance
table. Raw guest context-switch counters, KVM/real hardware behavior, mixed
audio/UI latency, and end-to-end timer reply latency remain unmeasured.

If a future workload reproduces a timer p99 above the verified C1.2C baseline
after excluding startup and clock artifacts, isolate the service policy or
revert the fairness policy while retaining the independent stack, accounting,
IPC, and security fixes. Do not optimize the global scheduler lock in this
phase.
