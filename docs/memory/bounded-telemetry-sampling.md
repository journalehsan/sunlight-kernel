# Bounded telemetry sampling under natural memory pressure

The 2 GiB, four-vCPU, two-pool QEMU TCG guest kept complete telemetry
samples within 2.005 seconds at the observed BSP checkpoints. The timer
client received 207 replies during the worker window with no failures;
the slowest took 210 ms. The worker verified both data sweeps, exited
normally and was physically reaped. The one- and two-vCPU single-pool
guests also completed the workload with fresh samples.

The [previous service responsiveness run](pressure-service-responsiveness.md)
removed repeated queue scans from anonymous unmap but still retained a
sample estimated to be 23 seconds old. A fresh baseline with that unmap fix
confirmed the sampling cause: 19 of 20 worker-window attempts hit the
8 ms elapsed limit, 16 of them before classification finished, and the sole
completion came after physical reap. No attempt reached the 2 million work
limit or exhausted external scratch. Median setup and classification times
were 0.240 and 8.024 ms; maximum PMM acquisition time was 0.045 ms. The
client saw a maximum estimated age of 21.139 seconds. These measurements
identify the abort path; they do not separate guest execution from time a
QEMU vCPU could not run.

## Implementation and consistency

The walker now coalesces consecutive physical frames within charged
batches and classifies and counts them by bitmap word. Word masks and
population counts handle aliases, while the mapped-page tally still counts
every present mapping, including base pages within huge mappings.
Classification now also counts the current address space's unique private
and shared frames. If a later space makes a previously private frame
shared, only rows before the last such transition can need correction.
Those earlier spaces are revisited under the same guards; rows with no
private frames need no correction. Multiple TGIDs sharing one address
space still count as multiple owners. An external frame remains shared and
uses the same bounded deduplication table. The global private-to-shared
transfer happens once per frame.

Batching alone yielded 4 complete worker-window samples out of 21.
Combining the walks and limiting correction yielded 15 of 22 at the old
cadence. The final sampler retries 25 BSP ticks after an incomplete
attempt and keeps its 100-tick interval after success. Each attempt retains
the existing work and elapsed limits. Missing a sample discards the
partial rows and leaves both the published generation and its original
timestamp in place. The elapsed limit is polled at batch boundaries and
excludes setup, so it does not cap total lock hold at exactly 8 ms.

An incremental cursor across timer interrupts could shorten individual
holds, but the fixed bitmaps do not record the versions of page tables,
PMM frame classes or task membership used to populate them. A resumed walk
could combine states that never coexisted, or follow a retired table. A
global invalidation epoch would need to cover all those mutations and
could restart repeatedly during fill, unmap and reclaim. This option was
evaluated from the locking and lifetime contracts; no cursor prototype was
run. The implemented approach keeps scalar capture, classification and
any correction under the same scheduler and PMM guards, then publishes
outside them. The fixed scratch size, 16 MiB heap, watermark policy,
single-pass candidate withdrawal, segmented reclaim and allocation-free
re-enrollment are preserved. Ring-0 ZRAM remains independent of VFS.

## Native results

| vCPUs / pools | Complete / attempts | Peak sample age (s) | Sampler p50 / p95 / max (ms) | Timer p50 / p95 / p99 / max (ms) | Failed replies |
| --- | ---: | ---: | ---: | ---: | ---: |
| 4 / 2 | 23 / 34 | 2.005 | 7.427 / 8.528 / 8.533 | 0 / 20 / 170 / 210 | 0 |
| 1 / 1 | 17 / 29 | 2.063 | 7.156 / 8.473 / 8.529 | 460 / 520 / 1030 / 1030 | 0 |
| 2 / 1 | 18 / 28 | 2.262 | 7.433 / 8.648 / 8.684 | 0 / 50 / 500 / 720 | 0 |

The worker window runs from start through init's physical reap
checkpoint. Age uses the same BSP clock as capture and measures the oldest
published sample observed through each attempt, before publication. The
three-second procfs RSS cutoff uses BSP ticks; this run observed sample
age and generations, but did not probe each procfs read. The existing
client also watched published generations throughout the run; its
cross-clock age estimates are reported separately. At four vCPUs, all 20
client replies before the worker and all 20 after reap returned within one
10 ms tick. The one-vCPU 1,030 ms reply succeeded with the unchanged
1,000 ms IPC timeout; caller elapsed time includes rescheduling, so it
does not establish a subsecond end-to-end guarantee.

At four vCPUs, median setup/classify/correction times were
0.389/7.121/0.001 ms. Correction peaked at 0.037 ms and all 11 misses
were time aborts. The maximum selected BSP entry gap was 244.397 ms;
maximum scheduler guard wait/hold was 0.179/8.706 ms and PMM guard
wait/hold on sampling ticks was 0.049/8.569 ms. The selected ticks include
sample ticks and outliers. BSP gaps and caller elapsed time measure
service opportunity and client responsiveness; per-task Ready-to-dispatch
delay and host deschedule time were not isolated. These are instrumented
QEMU observations, not worst-case bounds. External table exhaustion,
heavily shared mappings that require more correction work, and sustained
vCPU delay can still leave an old sample.

| vCPUs | Verified bytes per sweep | Minimum free / low watermark (frames) | Natural evictions | Peak sampled heap (bytes) |
| --- | ---: | ---: | ---: | ---: |
| 4 | 1,367,826,432 | 5,326 / 7,007 | 41 | 10,759,496 |
| 1 | 1,370,935,296 | 5,337 / 7,019 | 40 | 10,779,936 |
| 2 | 1,369,890,816 | 5,333 / 7,015 | 48 | 10,771,760 |

Each guest verified both sweeps including the rewritten cohort and
collected exit 37 once. Reap checks passed for zero owned frames,
candidates and VM regions, reclaimed VM root and absent kernel stack.
The primary reap released 334,750 user frames and 665 page tables.
The global pool retained slots after reap; worker reap released zero
swap slots, and the runs recorded no swap-in attempts. Natural same-page
restoration, restored attributes, re-eviction and reap with live worker
swap slots remain **unqualified**.

The matched host benchmark checked mapped/private/shared counts and
global bucket transfers against the reference for private, alias,
cross-owner, multiple-TGID and external-frame workloads. It reported zero
extra heap allocations for scratch aggregation. A native field-by-field
comparison was not run. The feature-free release build, 220 kernel host
tests and direct feature-free boot to Phase 3.0 passed. The standard
SWAP-1 and Phase 3.0 wrappers remain **blocked before QEMU** by the four
existing VFS ownership guard findings. No L1C-3 or VFS-D1 work was started.
The [machine results](bounded-telemetry-sampling-results.json) include
phase distributions, checkpoints, launch arguments and artifact hashes.
