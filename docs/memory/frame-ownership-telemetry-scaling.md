# Frame ownership telemetry scaling

Periodic frame ownership sampling now uses fixed kernel image scratch and
zero per-frame heap allocations. Completed samples retain the existing
telemetry v5 fields, process order, and procfs RSS rules. Under the tested
pressure load, some samples exceeded the time budget and were discarded.

The [metadata audit](reclaim-metadata-capacity-audit.md) traced prior
280-byte, 8-byte-aligned allocation panics to a leaf in the timer's
`BTreeMap<u64, (usize, bool)>`. The [segmented storage run](reclaim-segmented-storage.md)
stopped after initializing 704 MiB, with 16,774,912 heap bytes used and
2,304 free. Its failed allocation matches the traced layout. That run
has no stack trace, so the tree leaf is the likely site rather than a
confirmed attribution for that exact stop.

## Design and bounds

Two passes over the page tables replace retained vectors of frame addresses
and the tree. The first pass deduplicates aliases within each address space,
marks PMM `UserPrivate` frames that have more than one owner, and counts each
resulting global bucket transfer once. A space contributes multiple owners
when it has more than one distinct TGID. The second pass counts all present
mappings for RSS, then counts each distinct frame once in the private or
shared bucket. A present huge mapping contributes its constituent 4 KiB pages.
Spaces beyond the visible row limit still participate. Frames outside PMM's
tracked range are never private; a fixed table deduplicates them per space.

Three bitmaps cover the PMM's 16 GiB tracked span. A summary bitmap clears
only words touched by the previous space. The scratch symbol occupies
1,613,840 bytes including its lock and padding, in kernel BSS. Per-task
snapshot, ordering, core and RSS storage still use the heap; the ordering
request is 24 bytes per sampled task on x86-64. The 16 MiB heap, reclaim
watermarks, candidate chunks and allocation-free restore path are unchanged.
ZRAM still has no VFS or persistent-storage dependency.

The scheduler and PMM guards stabilize page-table lifetime and frame classes
through both passes. Publication still happens outside those guards. Both
walks share a 2,000,000-unit work budget covering table slots and huge-page
expansion. They also check an 8 ms elapsed budget between batches of at most
512 entries. The elapsed budget starts after task ordering and scratch reset,
and checks occur at batch boundaries, so it is not an upper bound on lock
hold time. The work budget still applies when the clock is coarse. If either
budget or the external table's 32-probe limit is exhausted, the timer keeps
the previous complete generation and retries on the normal cadence.

Incremental counters could move work off the sampler, but exact results
would require coherent updates across aliases, shared address spaces, TGID
changes, COW, swap, and teardown. Aggregation in small physical ranges would
reduce static scratch, but repeatedly walk the same tables or require
validation across lock releases. Fixed scratch was the smaller change to
the ownership model. Freshness under sustained pressure remains its main
limitation: the old generation can age past procfs's three-second RSS limit
and reads can return the existing `EAGAIN`.

## Matched host comparison

The [host harness](../../tools/tests/frame_ownership_bench.rs) runs the old
sort/deduplicate/tree algorithm and the new bitmap aggregation on the same
mapping streams. It tracks allocation layouts and peak live scratch through
vector growth and tree insertion, and compares mapped, private and shared
counts and the global transfer. All cases match exactly. Timings are host
medians of nine runs, with allocation instrumentation, and exclude native
page-table walks and contention. The bounded algorithm was measured to
completion here without the timer deadline.

| Mapping case | Legacy peak extra heap | Bounded peak extra heap | Legacy median | Bounded median |
| --- | ---: | ---: | ---: | ---: |
| 4,096 private frames | 233,120 B | 0 B | 0.56 ms | 0.07 ms |
| 32,768 frames with aliases, COW and external mappings | 2,243,016 B | 0 B | 8.32 ms | 0.42 ms |
| 180,224 private frames | 10,919,152 B | 0 B | 33.91 ms | 1.45 ms |
| 262,144 frames with sharing and aliases | 17,944,600 B | 0 B | 77.04 ms | 4.24 ms |

The largest legacy layout rose from 32 KiB to 4 MiB. Allocation counts and
maxima are in the [machine results](frame-ownership-telemetry-scaling-results.json).
Native heap measurements below are checkpoint maxima, not whole-run
allocator high-water marks. The host layout measurements do not include
fragmentation or unrelated native allocations.

## Native validation

Production compilation passed. Host regressions passed for the kernel,
IPC, scheduler, retirement, ZRAM, swap slots, descriptors, and VFS service
and registration. The existing 100-child bounded fault workload passed on
1, 2, and 4 vCPUs under QEMU TCG. Each run recorded 3,600 successful
ordinary read faults, 2,400 deferrals, 2,400 repeat candidate evictions,
and 100 normal exits and physical reaps. These runs used controlled reclaim.
Matched old/new telemetry fields were compared on the host; a native field
comparison and consumers' UI behavior were not tested.

| vCPUs | Initialized | Activations / evictions | Max heap checkpoint | Pressure workload |
| --- | ---: | ---: | ---: | --- |
| 1 | 1,012 MiB | 21 / 62 | 8,956,688 B | Verified all bytes; exit 37 and reap |
| 2 | 1,012 MiB | 21 / 40 | 8,956,656 B | Verified all bytes; exit 37 and reap |
| 4 | 1,010 MiB | 20 / 39 | 8,944,296 B | Allocation stop before verification; exit 38 and reap |

The unchanged pressure workload crossed the production start watermark in
all three guests and evicted pages. Four vCPUs reported only nine free PMM
frames at the last successful batch; the next allocation path returned an
error. The captures do not trace its exact failing allocation. There was no
kernel heap panic. The independent timer client returned 14, 105 and 34
successful replies, with zero failed replies; maxima were 450, 1,120 and
40 ms respectively. These are workload-wide observations, not a latency
guarantee or an attribution to telemetry. On one vCPU, seven attempts
during pressure were dropped at about 8.27–8.44 ms, and a later sample
completed after reap. Fresh samples at peak pressure remain blocked.

Actual pressure-driven activation and eviction are observed on all three
topologies. Natural same-page swap-in and re-eviction remain **unqualified**:
the pressure captures recorded zero swap-in attempts, and live ZRAM slots
remained after worker reap. Worker reap alone does not establish that the
global pool drained. The four-vCPU full verification gate remains blocked.
