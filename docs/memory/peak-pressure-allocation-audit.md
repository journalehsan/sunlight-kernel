# Peak pressure allocation audit

The four-vCPU stop reproduced in the 2 GiB QEMU TCG guest. PID 8
(`freezram`) requested a 4,096-byte anonymous user frame on the ninth
page of a 1 MiB eager mmap. PMM had zero free frames; the 16 MiB heap
had 8,958,752 bytes used and 7,818,464 bytes free. The failure was
physical capacity exhaustion, before page-table installation for that page.
The preceding nine-free-frame checkpoint alone could not identify the
failed allocation in the original capture. The instrumented reproduction
observed it before rollback, then recorded exit 38 and physical reap.

The PMM reserved the kernel by subtracting a fixed higher-half link address,
which treated the image as loaded at physical zero. The image was already
excluded from free memory by the Limine map at its actual load address.
The incorrect second reservation removed roughly 290 MiB of usable low RAM.
The fix translates the linker bounds using Limine's executable address
response. The tracked image frames remain pinned in the kernel image class;
they count in the managed total once so the image and static RAMFS still
participate in the physical accounting equation. The four-vCPU corrected
image spans `0x59fc0000..0x6c1f6db8`, and PMM total rises from 367,230 to
441,522 frames. The production watermark formula and limits are unchanged;
the resulting thresholds reflect the corrected managed total.

| Heap use at reproduced failure | Bytes |
| --- | ---: |
| Segmented candidates: 262,990 entries of capacity in 1,547 chunks | 6,324,136 |
| ZRAM vector reservations: two pools, 32 entries each for slots, free list and backing descriptors | 3,840 |
| Live eager mmap rollback journal: 256 × 40 bytes | 10,240 |
| Remaining live heap charges, by subtraction | 2,620,536 |
| Total used | 8,958,752 |
| Total free | 7,818,464 |

Candidate capacity comes from the reaped checkpoint: rollback and reap
remove entries without shrinking chunks. Pool capacities were already at
32 in both pools before failure and remained there after reap. The residual
is aggregate allocator use after removing known layouts, not a per-owner
inventory; it includes other live kernel objects and any allocator rounding.
Largest free span and whole-run heap high-water were not measured. The
failed request was for one physical frame, so heap fragmentation cannot
explain this stop. The 16 MiB heap is also backed by 4,096 PMM frames.
The last checkpoint before failure had 288,325 user-private and 616
page-table frames. At reap, ZRAM held 55 slots in 55 PMM frames
(225,280 bytes backing for 127,249 bytes payload). Both the last
successful checkpoint and reap had one backing frame per evicted page:
those evictions did not create net free physical capacity. Live slots
remaining after reap are global pool observations and are not evidence of
worker-owned slot leakage. Candidate chunks retained after reap account for
the persistent increase in settled heap use.

| vCPUs | Original initialized | Original last free frames | Original max heap checkpoint | Corrected minimum checkpoint free frames | Corrected result |
| --- | ---: | ---: | ---: | ---: | --- |
| 1 | 1,012 MiB | 266 | 8,956,688 B | 74,378 | PASS |
| 2 | 1,012 MiB | 8 | 8,956,656 B | 74,139 | PASS |
| 4 | 1,010 MiB | 9 | 8,944,296 B | 73,607 | PASS |

The original one- and two-vCPU guests started with 770 and 512 more
free frames than four vCPUs. Their final checkpoints also left little
headroom. All corrected runs used the unchanged 31 × 32 MiB plus
20 × 1 MiB workload. Each verified 1,061,158,912 bytes, exited 37,
was collected once, and physically reaped. The reap gate asserted reclaimed
address space, no kernel stack or pending cleanup, and zero worker-owned
frames, candidates and regions. The reaper returned 259,623 user frames
and 515 page-table frames per worker. The pools stayed empty because the
workload never reached the corrected physical watermark.

Frame ownership sampling still shares a 2,000,000-unit work cap and an
8 ms elapsed check across the two fixed-scratch walks. The check runs at
batch boundaries and excludes ordering and scratch reset; it is not a
lock-hold guarantee. The reproduced stop recorded incomplete samples at
8.30–8.36 ms and a later complete sample. Even with ample physical
headroom, the corrected four-vCPU guest dropped eight attempts at
8.24–8.63 ms during the mapping and verification run. Incomplete passes
retain the last complete generation; under sustained load, procfs may
return its existing freshness error. No sampling limits were raised.

The production release build and nine host regression jobs passed.
The separate four-vCPU controlled fault fixture completed 100 children
and 200 repeat rounds. It exercises the fault and reclaim lifecycle
under controlled selection; it does not qualify natural same-page behavior.
See the [machine results](peak-pressure-allocation-audit-results.json)
for commands, serial hashes, checkpoints and separate gate outcomes.
Post-fix natural activation, eviction, and telemetry freshness at the
production watermark are **BLOCKED** in this bounded fixture. Natural
same-page restoration and re-eviction remain **unqualified**.
