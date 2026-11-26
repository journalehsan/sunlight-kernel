# Adaptive natural ZRAM pressure

The fresh 2 GiB, four-vCPU, two-pool QEMU TCG guest crossed the
production reclaim watermark and finished both workload passes. Natural
reclaim evicted 16 pages, all outside the worker. The worker recorded zero
swap-ins, so natural eviction, restoration and re-eviction of the same
workload page remain **unqualified**. The one- and two-vCPU single-pool
qualification runs were not started because the primary gate did not pass.

The opt-in pressure fixture uses the corrected PMM counts, the production
watermarks and ordinary anonymous mappings. At workload start PMM reported
448,465 managed and 340,185 free 4 KiB frames. The low and target marks
were 7,007 and 14,014 frames. The workload cap was
`min(free - low + 1024, 393216)` pages: **334,202 pages (1,368,891,392
bytes)** in this run. The fixture adjusts each ballast batch to live free
headroom, with a nominal 1–32 MiB range and a shorter final batch. It
retains a 2 MiB inactive cohort (half read-only) and reads and rewrites the first byte
of each page in a 1 MiB active set between batches. It verifies the cohort
and ballast byte for byte, writes a new value to the writable cohort, then
unmaps and replaces the ballast for a second pass. Each sweep checked
1,367,842,816 bytes, excluding the separately checked active set.

The worker had a 90-second guest clock limit checked between allocations
and every MiB during verification. The monitor used a 120-second guest
limit and a 1-second IPC deadline. The host watchdog was 180 seconds.
Guest checks cannot interrupt a blocked syscall. Init collected status 37
once, waited for physical reap and stopped the monitor normally. The first
attempt stopped at a fixture dispatch error before pressure; its log is
retained separately.

| Checkpoint | Free PMM frames | Worker candidates | Reclaims (cumulative) | Live ZRAM slots | Payload / backing bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| Start | 340,185 | 0 | 0 | 0 | 0 / 0 |
| First pass filled | 5,326 | 334,202 | 8 | 8 | 17,660 / 32,768 |
| Second pass filled | 5,324 | 334,202 | 16 | 16 | 35,840 / 65,536 |
| Worker reaped | 340,743 | 0 | 16 | 16 | 35,840 / 65,536 |

There were 16 activations and 16 candidate scans, with one successful
eviction per activation and zero no-progress events. The global candidate
count fell from 4,098 at start to 4,082 at reap, while the worker never
lost a candidate at the filled checkpoints. The 16 evictions each used
one backing frame, so they yielded no net PMM frames. Both data sweeps
passed, including the changed writable cohort bytes on the second pass.
The restore flag witness never ran: restored read-only, writable and NX
attributes remain unqualified under natural pressure.

The independent timer client logged 204 replies and two failures between
worker start and fixture completion, including the brief post-reap
interval. The longest successful reply took 3,000 ms; failures returned
after 4,980 and 6,150 ms on the guest clock. This does not establish the
requested service responsiveness. Of 50 frame-ownership sample attempts in
that interval, 48 were incomplete. The monitor first observed a nonzero
committed sequence after the first sweep. The same sequence persisted for
about 9.09 seconds between published timestamps, spanning the second
pressure phase, and was up to 8.81 seconds old when read. Telemetry
freshness failed in this run. These samples do not isolate the causes of
the IPC delays or telemetry misses.

Reap released 334,754 worker user frames and 665 page tables, with no
swapped leaves. The worker had zero owned frames, candidates and regions
after reap; its VM root and kernel stack were reclaimed. The 16 global
slots remained and must not be counted as worker leakage or as restored
guest-wide accounting. Sampled heap allocation peaked at 10,756,616 bytes
and measured 10,723,560 bytes at reap on the unchanged 16 MiB heap.
Candidate capacity remained at 338,300 entries. Long-run heap stabilization
and termination with live worker slots were not qualified here.

The feature-free release build, 220 kernel host tests, and a direct
feature-free four-vCPU boot to Phase 3.0 passed. The standard SWAP-1 and
Phase 3.0 wrappers both stopped at the existing VFS ownership guard before
launching QEMU. See the [machine results](zram-adaptive-natural-pressure-results.json)
for raw evidence paths and hashes. No production PMM, heap, watermark,
candidate storage, telemetry scratch, ZRAM ownership or VFS behavior was
changed.
