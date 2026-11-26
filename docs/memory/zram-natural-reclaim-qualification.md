# Natural repeated anonymous reclaim qualification

The previous adaptive workload crossed the production low watermark but
generated only 16 reclaim activations. The FIFO already contained 4,098
candidates before the worker started. Those activations reclaimed older
pages; the worker kept all its candidates and recorded no swap-ins.
Refilling a few large mappings did not create enough independent reclaim
opportunities to reach the worker. An early run of the new turnover workload
confirmed a second limit: the guest reclaimed 3,204 older pages, then stopped
scanning because net frame recovery brought PMM free above the low watermark.

The opt-in fixture holds a 1 MiB active set that it reads and writes between
allocation batches and every 32 scratch turnovers, and a 2 MiB inactive set
split evenly between read-only and writable NX mappings. A much larger
anonymous job workspace is filled and replaced in phases. Each scratch
turnover allocates, writes and unmaps 256 KiB. The initial page budget is
`min(PMM free − low + 1024, 393216)`, including scratch. Up to 8,192
additional pages of workspace absorb recovered headroom within the absolute
393,216-page (1.5 GiB) worker payload cap. Page tables and existing process
overhead are outside that payload cap and are accounted for separately.

The first pressure phase evicts the cold set. An ordinary byte-for-byte
read restores it, and writes change the first byte of each writable page.
The next pressure phase observes the existing one-scan deferral of restored
candidates. The fixture releases that phase's workspace normally: otherwise
each deferred entry sits behind the large retained workspace in the FIFO.
The third phase supplies ordinary allocation pressure again. The final
sweep reads both permission classes, verifies the changed bytes, and leaves
64 pages of each class unread for normal process reap. The observation
syscall counts transitions; candidate selection, deferral and watermarks
remain production policy.

The final run allows 4,096 turnovers per phase, 220 seconds on the worker's
guest clock, 250 seconds on the monitor's guest clock and a 360-second host
watchdog. The monitor sends independent timer IPCs with a one-second deadline.
Time checks between operations cannot preempt a blocked guest syscall.
Telemetry freshness uses the BSP capture and observation clock; monitor
estimates from userspace are reported separately. These are instrumented
QEMU TCG results, and do not estimate the independent host deschedule or
per-task Ready-to-dispatch components.

The feature uses the existing 16 MiB heap, corrected PMM accounting,
bounded sampler, single-pass anonymous unmap, segmented candidate storage,
allocation-free re-enrollment and in-kernel ZRAM backing. No VFS or L1C-3
changes are involved.

In the fresh 2 GiB, four-vCPU, two-pool guest, PMM started with
441,505 managed frames and 333,160 free. Low and target were 6,898 and
13,797 frames. The initial budget was 327,286 pages (1,340,563,456 bytes);
the lowest sampled free count was 5,296 frames. The worker stayed within
the 16 MiB heap configuration; peak sampled kernel heap use was
10,977,080 bytes.

The primary guest directly witnessed all 512 cold addresses evicted,
restored by a user-mode non-present read fault (vector 14, error 0x4),
deferred and naturally evicted again. Restore checked present, user,
NX and the original writable bit: 256 addresses were read-only and
256 writable. The first and final sweeps verified 1,345,544,192 and
1,345,810,432 bytes, respectively. The final sweep restored 384 cold
pages, including 192 writable pages whose updated bytes survived the
second eviction. The remaining 128 were still swapped at exit.

Init collected exit 37 once, then observed physical reap. PMM free
increased from 6,253 to 336,285 frames, exactly the 329,376 resident
user frames and 656 page tables released. Reap removed all 128 remaining
worker slots and reduced the worker's owned frames, candidates and regions
to zero. Its VM root and kernel stack were reclaimed. The other
4,098 slots remained in the two pools with 6,283,264 backing bytes,
unchanged across this last reap; they explain why global slot counts
did not return to start.

The primary independent timer client got 890 replies during the worker
window with zero failures (p50/p95/p99/max: 10/40/400/530 ms).
**Telemetry freshness failed:** 15 of 443 samples completed and the
maximum observed age reached 95.413 seconds, beyond the three-second
procfs cutoff. Aborted samples preserve the last complete generation
and its original timestamp. The run measures stale publication under
sustained churn; it does not isolate how much of the scan's elapsed
budget was guest execution versus vCPU delay.

Both single-pool guests also witnessed the 512 same-address re-evictions,
verified both sweeps and exited with status 37 collected once. Each reap
released the 128 remaining worker slots and reconciled the PMM free-frame
increase exactly with the resident user and page-table frames released.

| vCPUs / pools | Resident frames / tables reaped | Fresh samples / attempts | Peak sample age | Timer replies / failures | Timer p95 / max |
| --- | ---: | ---: | ---: | ---: | ---: |
| 4 / 2 | 329,376 / 656 | 15 / 443 | 95.413 s | 890 / 0 | 40 / 530 ms |
| 2 / 1 | 329,879 / 657 | 23 / 280 | 16.780 s | 532 / 0 | 60 / 830 ms |
| 1 / 1 | 330,134 / 657 | 40 / 198 | 7.114 s | 83 / 0 | 520 / 530 ms |

The lifecycle passes on all three topologies. Telemetry freshness fails on
all three. The timer client completed every observed request, but only
sampled service responsiveness and does not establish a worst-case bound.
The test covers read-triggered restoration, not write or instruction-fetch
faults, concurrent workers, or long-run heap stabilization.

The feature-free release build and direct four-vCPU boot to Phase 3.0
passed. The kernel host suite passed 220 tests. Standard SWAP-1 and
Phase 3.0 wrappers stopped before guest launch at the four existing VFS
ownership findings. See the [machine results](zram-natural-reclaim-qualification-results.json)
for ordered page witnesses, distributions, launch arguments and evidence hashes.

To reproduce the fixture, build with
`cargo build --offline --locked -p sunlight-kernel --features zram_pressure_test`
and package `target/x86_64-unknown-none/debug/sunlight-kernel` with
`tools/make_hybrid_iso.sh`. Boot fresh guests using the launch arguments
in the results JSON. Require the status 37 marker, the ordered PID and
virtual-address witnesses for both permission classes, and the physical
reap record: a successful watchdog exit alone does not qualify reclaim.
