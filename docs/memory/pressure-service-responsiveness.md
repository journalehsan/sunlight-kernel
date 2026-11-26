# Service responsiveness under natural memory pressure

Removing repeated candidate scans from anonymous unmap restored timer
responsiveness in the fresh 2 GiB, four-vCPU, two-pool QEMU TCG run.
The timer client saw no failed replies and its slowest reply was 260 ms.
Telemetry still went stale, so the primary scenario is only partially
qualified.

The [previous run](zram-adaptive-natural-pressure.md) reported two timer
failures (4,980 and 6,150 ms elapsed) and a sample age of about 8.81 s.
Its logs could not attribute individual failures. A new instrumented
baseline with the old unmap logic reproduced both symptoms: two timer
failures (3,030 and 1,400 ms), and 44 incomplete ownership samples in 44
attempts while the worker was active. The client saw no committed
generation in that window.

| Image and phase | Successful replies | Reply p50 / p95 / p99 / max (ms) | Failures | Complete ownership samples |
| --- | ---: | --- | ---: | ---: |
| Baseline, before worker | 20 | 0 / 10 / 10 / 10 | 0 | 0 / 2 |
| Baseline, worker through reap | 179 | 0 / 60 / 1,330 / 1,940 | 2 | 0 / 44 |
| Baseline, after reap | 20 | 0 / 0 / 0 / 0 | 0 | 1 / 3 |
| Fixed, before worker | 20 | 0 / 0 / 0 / 0 | 0 | 1 / 2 |
| Fixed, worker through reap | 183 | 0 / 50 / 110 / 260 | 0 | 1 / 22 |
| Fixed, after reap | 20 | 0 / 0 / 0 / 0 | 0 | 2 / 2 |

Replies use the public guest monotonic clock with 10 ms granularity; zero
means the call returned within one observed tick interval. The request
timeout remained 1,000 ms. Caller elapsed time includes delivery,
execution, and time before the caller could run again. Records are grouped
by serial order at worker start and physical reap checkpoints; the sole
completion in the fixed work interval was after the kernel logged physical
reap and just before init recorded its checkpoint.

| Worker-window measurement | Baseline p50 / p95 / max (ms) | Fixed p50 / p95 / max (ms) |
| --- | --- | --- |
| BSP timer entry gaps, selected ticks | 219 / 1,526 / 1,830 | 53.3 / 103 / 222 |
| Scheduler guard wait on those ticks | 0.012 / 0.093 / 0.160 | 0.003 / 0.061 / 0.172 |
| Scheduler guard held on those ticks | 8.327 / 8.681 / 10.377 | 0.063 / 8.500 / 9.755 |
| PMM guard held on sampling ticks | 8.398 / 8.599 / 10.126 | 8.320 / 8.588 / 9.529 |
| Ownership sampling attempt | 8.361 / 8.556 / 8.770 | 8.285 / 8.514 / 8.540 |
| Direct reclaim execution | 13.055 / 14.716 / 14.716 | 12.910 / 15.768 / 15.768 |
| Scheduler guard held by 32 MiB unmap | Not measured directly | 66.434 / 101.438 / 102.645 |

The timer probe logs sample ticks and outliers (entry gap of at least
30 ms or guard wait/hold of at least 10 ms). These are distributions of
selected events, not all timer interrupts. Entry gaps measure lost BSP
timer opportunities; per-task Ready-to-dispatch latency and host deschedule
time were not measured. Kernel durations use the local calibrated clock
within each operation and include any time the guest vCPU could not run.
The opt-in fixture writes serial timing records, so these runs are not
production latency guarantees.

Between the first verification and the second fill, the workload unmaps
its ballast. A 32 MiB request ran 256 retirement chunks of 32 pages,
each retaining over the whole segmented candidate queue under scheduler
and PMM guards. The baseline's longest BSP gaps and both missed replies
occurred in that interval. The candidate queue held up to roughly 338,000
entries, so even a single early unmap could visit tens of millions of
entries. Reclaim and telemetry samples in this run were each on the
millisecond scale; neither had measured durations approaching the
second-scale gaps. The candidate-scan change and the reduction in gaps
and reply tails together implicate unmap as the dominant reproduced
service stall. The historical calls lack this timing evidence.

Anonymous unmap now withdraws the entire preflighted range from the
candidate queue once, after reserving retirement capacity and while both
guards are held. Candidate withdrawal removes a reclaim option and frees
no backing. Each retirement chunk still stages and withdraws its leaves,
completes the TLB shootdown, and only then releases its frames or swap
slots. The change preserves the segmented queue and allocation-free
re-enrollment. The heap, production watermarks, ZRAM ownership and
independence from VFS, and the telemetry algorithm and budgets are
unchanged.

Telemetry retains the last complete generation when a bounded walk cannot
finish. Most fixed-run attempts took about 8 ms and returned incomplete
even though PMM acquisition took at most 0.050 ms during the worker
window. The unchanged generation persisted throughout the worker's
active window and was about 23 s old just before the reap checkpoint.
That age is an estimate from the public BSP clock and the internal
sample clock; the unchanged sequence is direct evidence. The timings
are consistent with the elapsed budget firing during traversal, but
the probe does not log the exact abort reason. Fixing unmap did not
make the exact two-pass scan fit its budget. A future solution needs
less work per sample or a way to validate an exact snapshot across
bounded slices; increasing the budget would extend lock holds.

The fixed worker crossed the production low watermark of 7,007 frames
and reached 5,326 free. Both sweeps verified 1,367,826,432 bytes each,
including the rewritten cohort on pass two. Init collected exit 37
once. Reap released 334,750 user frames and 665 page tables, leaving no
worker frames, candidates, regions, VM root, or kernel stack. The 40
global ZRAM slots still present at reap were not worker slots. The
sampled heap peak was 10,759,496 bytes. Natural global eviction was
observed; natural same-page restoration and re-eviction, restored
attributes, and reap with live worker swap slots remain **unqualified**.

The feature-free release build, 220 kernel host tests and a direct
feature-free boot to Phase 3.0 passed. The standard SWAP-1 and Phase 3.0
wrappers were blocked before QEMU by the existing VFS ownership guard.
The one- and two-vCPU single-pool runs were not started because primary
telemetry freshness failed. The
[machine results](pressure-service-responsiveness-results.json) include
the timing distributions, launch arguments, and image and log hashes.
