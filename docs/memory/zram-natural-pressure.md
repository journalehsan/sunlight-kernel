# ZRAM natural memory pressure qualification

**Blocked before the reclaim watermark on all three guest topologies.** Native userspace ran in fresh 2 GiB QEMU TCG guests. The four- and one-vCPU guests failed a reclaim candidate capacity reservation while more than 87,000 physical frames were free. The two-vCPU guest halted on kernel heap exhaustion. No run activated natural reclaim.

## Workload

The opt-in `zram_pressure_test` image uses ordinary spawn, private anonymous mmap, userspace loads and stores, and normal exit. It initializes every byte to a deterministic, compressible pattern. A 1 MiB hot set has its first byte per page checked and updated after every allocation; the remainder is idle after initialization until a full verification sweep. The cap is 1,012 MiB of anonymous mappings: 31 batches of 32 MiB followed by 20 batches of 1 MiB. Userspace checks a 90-second guest clock limit between allocation batches and every 1 MiB during verification. An independent timer client has a 120-second guest limit; the host runner has a 180-second watchdog. The guest limits cannot interrupt a blocked kernel call.

The timer client issues IPC to the ordinary timer service, targeting a 100 ms interval with a 1,000 ms IPC deadline. Init continues processing registrations and lookups. The checkpoints sample PMM, ZRAM, candidates and kernel heap. There is no diagnostic fill, forced watermark, or scoped selection in this workload.

## Native findings

| vCPUs / pools | Last initialized MiB | Free frames at last sample / watermark | Candidates | Heap at last sample | Timer replies / max guest latency | Outcome |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 4 / 2 | 672 | 87,765 / 5,748 | 176,130 | 6,847,656 B | 15 / 20 ms | ENOMEM, exit 38, reaped |
| 2 / 1 | 544 | 121,109 / 5,756 | 143,362 | 6,061,224 B | 11 / 0 ms | Kernel heap panic |
| 1 / 1 | 672 | 88,535 / 5,760 | 176,130 | 6,847,656 B | 6 / 360 ms | ENOMEM, exit 38, reaped |

The timer client logged no failed replies before the stop points. Latencies use the guest's 10 ms clock, so 0 ms means no tick elapsed. These samples cover bring-up and allocation growth only. They do not bound wall-clock pauses or establish service responsiveness under reclaim.

The four- and one-vCPU runs both failed while trying to add 8,192 entries to a full 176,130-entry candidate queue. The checkpoint inside the failed reservation recorded 7,175,336 heap bytes in use. In each run init collected status 38 once. Reaping released 172,583 user frames and 345 page tables, with zero swapped leaves; the child's owned frames, candidates and VM regions reached zero, and its stack and VM root were reclaimed. Global candidate count returned to 4,098. Queue capacity remained 176,130 and the whole kernel heap remained at 6,814,600 bytes at the reap checkpoint. That is retained capacity after one failure cycle, not evidence of stabilization.

On two vCPUs the allocator reported 16,775,464 heap bytes used and 1,752 free when a 280-byte allocation failed. The logs do not identify which allocation site exhausted the heap. The panic prevented cleanup verification. An earlier experiment with chunked candidate storage progressed farther on four vCPUs but still encountered small-allocation heap panics. That experiment was removed; the final source preserves the existing candidate queue, reservations, deferral and allocation-free re-enrollment.

All recorded natural checkpoints show zero activations, scans, reclaims, live slots, backing bytes and swap-in attempts. The completed batches passed their hot-set byte checks. The full data sweep, swap restoration, re-eviction of the same page, resource stabilization during reclaim, and cleanup after a successful full workload remain unqualified.

## Controls and reproduction

The [repeated reclaim](zram-repeated-reclaim.md) and [populated exit](zram-process-lifecycle.md) results are historical controlled diagnostics. They cover restored-page re-enrollment, byte and permission checks, and populated exit in their respective fixtures. Standard SWAP-1 is a separate forced-pressure control. None qualifies repeated selection at the production watermark.

The release kernel build and nine host regression jobs passed on this checkout. The standard SWAP-1 and Phase 3.0 scripts stopped before QEMU at the existing VFS ownership guard, which reported the same four findings as the prior report. Historical isolated-candidate passes are recorded in the previous report. The controlled repeated and populated native suites were not rerun for this report.

Build with `cargo build --offline --locked -p sunlight-kernel --features zram_pressure_test` and package the debug kernel with `tools/make_hybrid_iso.sh`. Boot fresh guests in the order 4, 2, 1 vCPUs. The [machine results](zram-natural-pressure-results.json) record exact QEMU arguments, artifact hashes, checkpoints and earlier attempts. The host runner's zero exit for four and one vCPUs means it saw the expected stop marker; the guest workload itself failed.

The next step is to attribute the transient kernel heap allocations and independently confirm the candidate growth bottleneck before choosing a production repair. Repeat the same guest gates after that repair. A resumed run must witness the same anonymous page compressed, restored by an ordinary access, and selected again to qualify repeated reclaim.
