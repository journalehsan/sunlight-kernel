# Scheduler Phase C1.2F: tick dependency and extreme tails

This investigates the C1.2E candidate, rather than assuming that its approximately
20 ms median is imposed by a 10 ms clock. Evidence is retained under
[`target/scheduler-c12f/` evidence index](../../target/scheduler-c12f/evidence.md);
the per-trial results are in
[scheduler-c12f-trials.csv](scheduler-c12f-trials.csv).

## What the dispatch traces establish

A blocked timer can become Ready and be selected during the **same timer IRQ**.
The timer does not intrinsically require another tick after publication.
However, C1.2E's retained reschedule **bit** supplies no interrupt by itself.
After an earlier eligible service blocks, FIFO resumes the interrupted worker;
an already-Ready timer's retained bit is normally consumed on another periodic
IRQ. Multiple earlier services repeat this sequence, producing tick multiples.

For example, the retained C1.2E `after-trace/capture1`, trial 5, has these actual
CPU-0 timestamps in nanoseconds:

| Timestamp | Observation |
| ---: | --- |
| 14,074,828,087 | Timer notification publication |
| 14,074,829,592 | Timer BlockedOnIpc wake, nice −5, worker PID 42 current |
| 14,074,841,876 | Scheduler entry/decision |
| 14,074,860,200 | Select TTY PID 4, nice −5 |
| 14,074,883,262 | TTY blocked; scheduler entered through reschedule vector |
| 14,074,897,329 | Resume worker PID 42 with its residual turn |
| 14,084,884,030 | Next periodic IRQ entry |
| 14,084,913,598 | Select PID 30, nice −10 |
| 14,084,961,144 | That service blocked; resume worker again |
| 14,094,950,008 | Next periodic IRQ entry |
| 14,094,980,558 | Select timer PID 6 |
| 14,094,992,721 | Timer receive |

This is a 20.151 ms wake-to-selection interval containing actual worker returns
between services. Across the two historical candidate boots, 8,900 of 8,901
completed steady external timer queue waits contain multiple dispatch decisions.
Service execution itself takes microseconds. This is a demonstrated dispatch
delivery dependency, not an inference from a percentile histogram.

The new observer adds dispatch/accounting, context-save, selection, architecture,
reaping, and handler-tail checkpoints. Event 13 records the Rust handler tail
**before dropping SCHEDULER**, not physical IRET. Event 14 records retained
priority requests. The existing receive event remains the later service endpoint.
Interrupts remain disabled until the established naked restore/IRET completes.
Subsequent reschedule-vector selections without an intervening periodic IRQ
demonstrate that the continuation no longer depends on that IRQ.

Final trace boot 1, trial 5, shows the same pattern completed within 0.147 ms:

| Timestamp (ns) | Observation |
| ---: | --- |
| 20,481,693,610 | Actual timer wake, worker PID 42 current |
| 20,481,734,965 | Select earlier nice −10 task PID 29 |
| 20,481,788,067 | That task blocked; enter reschedule handler |
| 20,481,807,539 | Retain timer wake while resuming PID 42 |
| 20,481,812,531 | Worker selected; reschedule-handler tail, request pending |
| 20,481,820,363 | Next scheduler entry, **no intervening periodic IRQ** |
| 20,481,840,656 | Select timer through reschedule vector |
| 20,481,855,603 | Timer receive |

## Change and rejected experiment

The final change publishes the existing coalesced dispatch-notification mask
when retaining an unsatisfied priority wake. Timer and reschedule handlers flush
it after dropping SCHEDULER, with interrupts still disabled. The existing LAPIC
try-send is bounded; a busy ICR retains the mask and reschedule bit for a later
opportunity. No recursive scheduling or interrupt-enable window is introduced.

An unrestricted immediate-IPI prototype exposed an additional defect: treating
every internally requeued higher-priority CPU peer as a fresh wake can revoke
nice +8/+9/+10 admission before its first work instruction. Its mixed trial 4
completed with zero operations in all three positive-nice slots. This failed
capture is retained under `after-mixed/capture1`, and is excluded from acceptance.
Its normal traces/performance results (`after-trace`, `after-performance`) are
also prototype evidence, not the final candidate.

The final dispatcher tracks `priority_wake_pending` under the existing scheduler
lock. External Ready publications, including live-owner IPC terminal wakes, set
it; actual dispatch clears it; internal rotation/residual requeue does not mint
another wake. The priority selector still uses strict lower nice, local FIFO
order, affinity, ownership, dispatchable context, and remote stack-pin checks.
Only an **unsatisfied external wake** can preempt an admitted turn. This both
delivers an already-pending service wake promptly and preserves ordinary
nice-weighted CPU admission. It adds no task class or timer-name selection rule.

Existing quantum credit remains frozen while interrupted. Resume consumes that
credit once; it does not refill it or accumulate admission debt again. Full-turn
feedback happens once; block, yield, and exit discard residual credit. The host
regression executes immediate notifications without advancing the clock,
checks exactly 180 ms of original entitlement, and runs nice 0/+8/+9/+10 CPU
peers with a repeatedly waking service and a bounded notification chain. The
unrestricted prototype fails that regression.

## Extreme-tail decomposition

The historical 346.46 ms outlier includes 274.77 ms inside one dispatch call.
That capture lacks the new checkpoints, so it cannot retrospectively identify
the operation or divide it into host and guest execution.

The new baseline reproduces much larger intervals and identifies an unnecessary
operation: `reap_process_resources` unconditionally calls
`PMM.diagnostic_report_pid`, which scans all 4,194,304 frame-owner entries. This
is a 16 MiB table covering 16 GiB of physical frames, including unused entries
and borrower threads that own no user address space. It executes with guest
interrupts disabled on a dispatch/cleanup path. The scan is now opt-in through
`verbose_diag` or `scheduler_b_test`; resource reclamation and its safety checks
are unchanged.

Baseline trace boot 1, trial 5, PID 42 brackets that scan at
24,790,747,548–25,798,334,825 ns: **1,007.587 ms**. Trial 17 also contains
805.893, 484.691, and 479.119 ms scan intervals. The stage-21→22 observations
separately delimit user-address-space reclamation; these scan intervals are
stage 24→25, after reclamation and cleanup messages.

Read-only Linux `/proc/PID/task/TID/schedstat` samples provide independent host
runtime and runnable-wait counters. Byte offsets of the serial messages directly
surrounding the scan associate its host interval without fitting or subtracting
guest/host clocks. Both boundary sample uncertainty and counter-read skew are
retained as inner/outer bounds. For PID 42's roughly one-second guest interval,
the host counters show over 900 ms of vCPU runnable wait: host scheduling extends
an already expensive guest IRQ-off diagnostic. Neither all of the second nor
all remaining IRQ jitter can be described as guest instruction execution.

| Independent host bound for that scan | Lower / upper (ms) |
| --- | ---: |
| Wall interval enclosing serial boundaries | 985.269 / 1,034.736 |
| TCG vCPU host execution | 83.219 / 93.927 |
| TCG vCPU host runnable wait | 912.380 / 943.979 |

`samples.csv` retains **every** observed timer wake and queue-publication sample,
including right-censored lower bounds; `notifications.csv` additionally retains
left/right notification edges. Completed-sample quantiles never turn a censored
lower bound into a completed wait or quietly discard startup. Baseline boot 1,
trial 5, has a still-pending wake with a **1,799.720 ms lower bound** at the trace
edge, larger than its completed waits. Complete raw events, individual IRQ gaps,
checkpoint intervals, and >300 ms completed wake windows remain in each capture's
`analysis/` directory.

Long **RPC completion** waits remain and have a different cause. Final mixed
boot 1, trial 7, caller PID 5/generation 192 is saturated throughout:

| Stage | Timestamp (ns) / duration |
| --- | ---: |
| Request | 127,544,492,441 |
| Timer receive | 127,544,799,112 / 0.307 ms from request |
| Reply committed | 127,544,848,735 / 0.050 ms service interval |
| Resume worker PID 59 | 127,544,947,853 |
| Next worker PID 60 | 127,898,716,862 |
| Next worker PID 61 | 128,264,246,169 |
| Next worker PID 62 | 128,610,330,561 |
| Caller selected | 128,945,274,178 |
| Caller consumes reply | 128,945,297,519 / 1,400.449 ms after commit |

All five client/worker tasks have nice 0. Each worker retains its bounded turn
through the intervening short service interruptions; the replied-to caller
waits in FIFO behind them. Its own Ready publication cannot preempt equal-nice
peers. This is preserved CPU-service fairness, rather than delayed timer service
or a missing reschedule request. Across the two final mixed boots, 33 steady
completed RPCs exceed 300 ms. All individual RPCs and their full >300 ms event
windows are retained in `rpc-samples.csv`, `rpc-tails.json`, and `timer.json`;
accepted mixed captures have zero censored requests. Wake-to-dispatch results
must not be presented as RPC latency bounds.

## Startup policy

The old timer starts at nice 0 and waits for sunlight-niced to install its
protected High policy. Equal-nice CPU peers therefore correctly retain their
turns during this period; the ordinary priority-wake path cannot compensate.

After successful embedded ELF loading and before Ready publication, both native
spawn paths now initialize UID-0 `/sbin/timer_server` to nice −5. This matches
the existing niced High default. A basename, a different path, or a non-root
launch gets no boost; arbitrary VFS binaries and exec retain their existing
authority rules. Root already has authority to set this nice. Niced continues
to maintain the protected policy. The host tests check path/UID rejection, and
the final trace's first startup trial already records nice −5 wakes. Other
startup services and IRQ-off work can still delay execution.

## Measurement protocol and results

The baseline has two cold-boot 22-trial trace suites; the final candidate has
three, including an additional cadence control. Each policy also has two
cold-boot 22-trial trace-disabled performance suites. Each mixed policy has two
cold-boot eight-trial suites. Configuration is one vCPU, QEMU TCG multi-thread,
`-cpu max`, 1024 MiB, no NIC. Trace policy pairs use host CPUs 6 and 10;
performance pairs use 12 and 14; mixed pairs use 8 and 9 with policies swapped.
These are distinct physical host cores, including different host core types;
the same placements appear on both sides. Independent captures run concurrently,
so shared host resources and runnable-wait noise remain limitations.

Normal before/after images use the identical frozen C1.2E normal workload ELF.
Mixed before/after images use the same newly frozen mixed ELF. It guarantees at
least one real timer RPC per trial even if a startup audio RPC outlasts the
one-second warmup; such tail requests remain outside the saturation subset.
The initial mixed baseline with zero warmup replies is retained as rejected
evidence in `before-mixed/capture1`. Validators were not relaxed.

Normal startup is trials 1–4; mixed startup is 1–3. All are retained in the CSV
and raw samples. Steady quantiles use nearest rank over completed actual wakes.
CPU Jain uses attributed CPU nanoseconds per trial, not dispatches or operation
counts. Throughput includes the reported worker completion tail. New `COST`
records measure actual context-switch counter deltas over the diagnostic window,
including cleanup/IPC tails, rather than estimating them from worker dispatches.

Actual wake-to-selected-dispatch, milliseconds; all accepted trace boots are pooled:

| Duration/order | Completed wakes before / final | p50 / p95 / p99 / max before | p50 / p95 / p99 / max final |
| --- | ---: | ---: | ---: |
| 5s forward | 1,291 / 5,224 | 20.401 / 30.598 / 40.265 / 170.674 | 0.197 / 1.132 / 2.239 / 4.534 |
| 5s reverse | 1,299 / 5,226 | 20.386 / 30.582 / 32.259 / 195.184 | 0.184 / 1.095 / 2.077 / 4.793 |
| 10s forward | 3,152 / 13,001 | 20.399 / 30.606 / 41.425 / 1568.365 | 0.192 / 1.149 / 2.268 / 5.831 |
| 10s reverse | 3,050 / 12,979 | 20.393 / 30.601 / 83.073 / 997.356 | 0.215 / 1.182 / 2.753 / 54.861 |

Cold-boot detail, all steady trials; this retains the slow-cadence final boot:

| Policy / boot | Wake p50 / p95 / p99 / max (ms) | IRQ p50 / p95 / p99 / max (ms) | Completed wakes / right-censored wakes |
| --- | ---: | ---: | ---: |
| before-trace / capture1 | 20.154 / 30.229 / 48.670 / 1568.365 | 10.062 / 10.092 / 21.434 / 1247.118 | 4,441 / 15 |
| before-trace / capture2 | 20.432 / 30.621 / 52.234 / 638.972 | 10.176 / 10.585 / 25.054 / 406.858 | 4,351 / 11 |
| final-trace / capture1 | 0.177 / 1.565 / 2.106 / 7.392 | 16.284 / 16.342 / 16.917 / 18.917 | 8,665 / 0 |
| final-trace / capture2 | 0.283 / 0.943 / 2.994 / 54.861 | 10.171 / 10.302 / 11.030 / 47.071 | 13,820 / 0 |
| final-trace / capture3 | 0.126 / 0.370 / 1.982 / 4.678 | 10.093 / 10.106 / 10.792 / 12.825 | 13,945 / 0 |

Completed and unfinished actual wakes, including startup. Unfinished maximum is a lower bound, **not** a completed maximum:

| Policy / phase | Completed / right-censored wakes | Completed p50 / p95 / p99 / max (ms) | Largest unfinished lower bound (ms) |
| --- | ---: | ---: | ---: |
| before-trace / startup | 210 / 4 | 20.468 / 779.503 / 899.286 / 937.537 | 72.378 |
| before-trace / steady | 8,792 / 26 | 20.396 / 30.598 / 51.052 / 1568.365 | 1799.720 |
| final-trace / startup | 3,164 / 0 | 0.081 / 0.480 / 2.083 / 11.147 | — |
| final-trace / steady | 36,430 / 0 | 0.198 / 1.156 / 2.404 / 54.861 | — |

Trace-disabled performance, 36 steady trials per policy:

| Metric | Before | Final |
| --- | ---: | ---: |
| CPU-service Jain mean | 0.994806 | 0.993927 |
| Throughput, operations/s mean | 556,720 | 553,827 |
| Actual context switches/s mean | 200.21 | 508.11 |

The pooled mean throughput change is −0.52%; the context-switch rate is 2.54×.
The pooled throughput number masks substantial baseline variability:

| Policy / performance boot | Host CPU | CPU Jain mean | Operations/s mean | Context switches/s mean |
| --- | ---: | ---: | ---: | ---: |
| Before / capture1 | 12 | 0.994397 | 394,498 | 197.32 |
| Before / capture2 | 14 | 0.995216 | 718,941 | 203.10 |
| Final / capture1 | 12 | 0.993734 | 552,928 | 507.11 |
| Final / capture2 | 14 | 0.994121 | 554,726 | 509.12 |

Same-core observed throughput changes are +40.16% and −22.84%, with differing
concurrent host activity. Thus −0.52% is a descriptive pooled result, **not a
bound on regression or proof of equal throughput**. These captures establish
the extra context-switch activity and near-equal CPU service within each
equal-nice cohort. They do not isolate per-IPI CPU cost or establish a precise
throughput effect. Creation-order throughput changes also vary in sign.

Mixed positive-nice progress: minimum **per-trial** operations / attributed CPU ms across six steady nice tests per policy:

| Nice | Before operations / CPU ms | Final operations / CPU ms |
| --- | ---: | ---: |
| +8 | 592,625 / 1230.669 | 897,920 / 1697.497 |
| +9 | 184,551 / 268.824 | 936,506 / 1791.184 |
| +10 | 16,089 / 29.117 | 974,591 / 1842.201 |

Timer RPC completion is a separate endpoint. Full steady distributions (including completion tails):

| Policy / CPU cohort | Completed RPCs | p50 / p95 / p99 / max (ms) |
| --- | ---: | ---: |
| before-mixed-retry / [0, 0, 0, 0] | 16 | 1333.562 / 1408.171 / 1408.171 / 1408.171 |
| before-mixed-retry / [0, 8, 9, 10] | 61 | 508.917 / 648.431 / 661.511 / 661.511 |
| final-mixed / [0, 0, 0, 0] | 16 | 1299.521 / 1400.805 / 1400.805 / 1400.805 |
| final-mixed / [0, 8, 9, 10] | 441 | 1.002 / 274.007 / 346.360 / 375.599 |

Notification coalescing, normal steady traces; publications include unfinished window edges:

| Policy | Publications / tick receives | Publications per receive | First pending publication → receive p50 / p99 (ms) | Latest publication → receive p50 / p99 (ms) |
| --- | ---: | ---: | ---: | ---: |
| before-trace | 27,362 / 8,800 | 3.109 | 20.435 / 52.286 | 0.079 / 0.959 |
| final-trace | 36,435 / 36,430 | 1.000 | 0.220 / 2.441 | 0.220 / 2.439 |

The decoder verifies the endpoint-relative sequence offset against published global ticks. More prompt receives reduce coalescing; they do not imply more accurate hardware interrupt delivery.

One final trace boot programs LAPIC initial count **1,024,606** and observes
16.284 ms median IRQ spacing, despite a `tick_hz=100` target. Its TSC calibration
is approximately 2.510 GHz. The other accepted trace boots program approximately
627k–634k counts. The current `calibrate_lapic_timer` waits until a nominal 10 ms
TSC threshold, then multiplies the observed countdown by 100 without measuring
the actual elapsed interval. A host pause can extend that interval and inflate
the programmed period. That mechanism is a source-based explanation consistent
with this capture; the calibration interval itself was not stage-traced. The
16.284 ms boot remains included in all-capture results and is reported separately.
Clock-driver calibration changes are deferred; no improved LAPIC accuracy is
attributed to the scheduler change.

The final 54.861 ms wake maximum (trace boot 2, trial 14) first waits while PID 29
at nice −10 remains current; that service blocks about 36.47 ms after the timer
wake. The worker-to-timer continuation then uses the reschedule vector without
another tick. An additional approximately 18.07 ms lies between the post-reap
checkpoint and the retained-priority observation, encompassing runtime start
and the priority recheck. These checkpoints exclude the removed owner-table
scan but do not divide that interval into guest execution, TCG work, and host
descheduling. This tail is retained and remains an attribution limitation.

## Verification and reproduction

The final default kernel check, **51 focused Rust host tests** (including the 25-test extracted production service/accounting binary), independent policy/queue probes, and **21 Python validator tests** pass. The missing-notification regression fails before the change; the immediate-notification nice-progress regression fails with the unrestricted prototype and passes with the final pending-wake rule.

| Lifecycle vCPUs | Deferred reaps | Reused slots | Exit races | Result |
| ---: | ---: | ---: | ---: | --- |
| 1 | 5 | 1 | 1 | PASS |
| 2 | 6 | 2 | 1 | PASS |
| 4 | 8 | 4 | 1 | PASS |
| 8 | 10 | 8 | 1 | PASS |
| 12 | 12 | 12 | 1 | PASS |

The 2-vCPU `phase_sec` gate passes with its complete 53,661-byte serial stream retained. Its expected markers cover capability isolation/forgery/revocation, message and user-pointer validation, bounded IPC/backpressure and endpoint reuse, notification coalescing, timeout cancellation, terminal generations/arbitration, nameserver identity, deferred replies, deadline wake/race/late-reply behavior, and SMP terminal-state stress. Lifecycle gates exercise stack handoff, deferred reclamation, borrower ownership, reuse and exit races. Rust formatting and `git diff --check` also pass. These are functional SMP gates, not SMP timing measurements.

The per-trial CSV contains 230 accepted comparison trials plus 22 clearly labelled `notify-trace` prototype trials. Prototype and invalid captures remain separate from final-candidate acceptance.

The final image manifests preserve kernel, ISO, workload and input-source hashes,
commands, revision, and worktree patch. Capture `results.json` files preserve
full QEMU commands, host placement/load, ISO and serial hashes, and validation
status. `build-final.py`, `build-matrix.py`, `run-final.py`, and `run-matrix.py`
under the evidence root preserve build/run ordering and the rejected experiments.
Each accepted capture retains its uncropped serial stream and host samples.

```sh
python3 tools/scheduler_timer_build.py --mode trace --output target/c12f-repro/trace
python3 tools/scheduler_bench.py --suite --iso target/c12f-repro/trace/sunlightos.iso \
  --cpus 1 --warmup 0 --duration 900 --repetitions 1 --memory 1024M \
  --qemu-cpu 6 --host-sched --output target/c12f-repro/trace/capture1
python3 tools/scheduler_tick_investigation.py \
  target/c12f-repro/trace/capture1/1cpu-run1.serial.log \
  --output target/c12f-repro/trace/capture1/analysis
python3 tools/scheduler_c12f_summary.py --csv docs/kernel/scheduler-c12f-trials.csv
python3 tools/test_scheduler_host.py
python3 -m unittest discover -s tools -p 'test_scheduler*.py'
```

For exact comparison, use each retained manifest's frozen ELF, feature set, and
source snapshot; current-source builds can otherwise include later changes.
The normal and mixed binaries are deliberately distinct workloads.

## Limits

Two boots establish repeatability, not confidence intervals or a maximum-latency
guarantee. TSC/PIT calibration is checked for validity and monotonicity, not
absolute frequency accuracy. Guest attributed runtime includes host stalls while
the guest task is current. Host runnable wait excludes sleeping, TCG translation,
and other emulation costs; samples do not timestamp actual LAPIC expiration.
Uninstrumented guest IRQ-off work and host interrupt-delivery jitter remain
confounded outside the bracketed operations.

The final change removes a demonstrated tick-delivery dependency and an unconditional
diagnostic scan. Timer IRQ delivery itself remains periodic, may coalesce, and
can be late even when wake-to-dispatch is short. There is no KVM device here and
no bare-metal measurement. Selected-dispatch and Rust handler-tail events precede
physical IRET; receive is a later kernel-observed service stage. No direct
per-switch CPU-cycle cost or hardware audio quality is claimed.

All timing traces are single-vCPU. SMP lifecycle gates do not establish remote
busy-CPU wake latency or SMP performance; other request-only remote wake paths
still rely on their existing delivery opportunities.

There is no general minimum share under continuously renewing higher-priority
external demand. Positive-nice progress is validated for these finite workloads.
No MLFQ, EEVDF, new scheduling class, global lock restructuring, reclaim redesign,
or IPC/capability authority expansion is introduced.
