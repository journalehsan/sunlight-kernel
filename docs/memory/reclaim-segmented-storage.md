# Segmented reclaim candidate storage

The candidate queue now grows in 4,088-byte kernel heap allocations. It no
longer needs a contiguous span for the entire resident and swapped population.
The kernel heap is still 16 MiB. Natural reclaim remains unqualified.

## Design and contracts

[The queue](../../kernel/src/memory/candidate_queue.rs) stores 170 entries per
chunk in a singly linked ring. A head and tail cursor delimit the live FIFO.
Append and pop are constant time, including across chunk boundaries. Removing
candidates for a frame, range or exiting process compacts survivors in one
stable pass. There is no separately growing pointer directory. A directory
would simplify indexing, which this queue does not need, but would still need
a contiguous replacement during growth. PMM-backed chunks would avoid the
heap budget but would add frame ownership and accounting to a queue already
managed by the kernel heap.

Growth preallocates a detached chain of chunks and links it into the ring only
after the entire request succeeds. If the active interval wraps into the
prefix of the head chunk, that prefix moves to the first new chunk to keep FIFO
order. Growth copies at most 169 entries and never replaces an existing chunk.
Allocation failure frees the detached chain, leaving the original order,
capacity and claims intact. The maximum individual queue request is 4,088
bytes, aligned to eight bytes; growth can still fail if too few usable spans of
that size remain.

[Swap integration](../../kernel/src/memory/swap.rs) still reserves room for
resident candidates, all live ZRAM slots and the incoming batch. Existing
scheduler and PMM serialization protects the transition between a resident
entry and a swapped slot. A slot gives its capacity back on successful restore;
enqueue never allocates. Candidate identity, eligibility, FIFO ordering, and
the one-scan deferral of restored pages are unchanged. Pool free-list
reservations are unchanged. Ordinary process cleanup removes entries and live
slots; empty chunks stay available at the queue's high-water mark, as vacant
capacity did before. The kernel keeps the existing reclaim policy and
watermarks, uses ordinary spawn paths, and owns ZRAM entirely in ring 0
without VFS involvement.

## Heap cost

The entry stays at 24 bytes. At full occupancy, the eight-byte link raises
heap cost from 24 to 24.0471 bytes per candidate slot (0.196% overhead).
Rounding adds at most 169 vacant slots on successful growth beyond the old
capacity. Vacant capacity can grow after cleanup, as before. The x86-64 static
queue descriptor is 56 bytes, plus the lock (64 bytes together versus 40
previously). There are no separate PMM frame charges for the chunks.

The matched host test uses the production first-fit allocator in a 16 MiB
arena, a fixed 2,620,536-byte allocation for the measured nonqueue baseline,
and the 327,680-byte eager mmap journal for each growth. It measures allocator
usage on every successful allocation while growing from 4,098 to 176,130
entries by 21 batches of 8,192. It does not replay the guest's allocation
interleaving or include timer telemetry scratch.

| Measured host allocator use | Contiguous FIFO | Segmented FIFO |
| --- | ---: | ---: |
| Capacity at 176,130 entries | 176,130 | 176,290 |
| Queue heap bytes at that point | 4,227,120 | 4,239,256 |
| Effective queue bytes per live candidate | 24.0000 | 24.0689 |
| Total settled heap bytes | 6,847,656 | 6,859,792 |
| Peak heap bytes | 11,205,848 | 7,187,472 |

Peak use fell by 4,018,376 bytes (35.9%) in that matched test. The lower
peak comes from eliminating overlap between the old and replacement buffers.
The native logs provide checkpoints and failure snapshots; they cannot measure
whole-run allocator high-water marks. The
[prior audit](reclaim-metadata-capacity-audit.md) recorded both queue growth
failures and independent timer telemetry heap exhaustion.

## Validation

The host FIFO regression compares wrapped and unwrapped growth, rotation and
stable removal against `VecDeque`. The fragmented-heap test uses four 4,088-byte
holes separated by live allocations: an 8,176-byte request fails, two chunk
allocations succeed, and a subsequent three-chunk request fails after obtaining
two. Rollback restores the heap and keeps a simulated live slot's capacity
available for allocation-free re-enrollment. Dropping the queue and the
separators returns the arena to zero used bytes. The full host kernel suite
passed 220 tests. The scheduler, retirement, ZRAM, swap-slot, IPC, descriptor,
and VFS host regressions passed, as did the production release kernel build.

The [repeated reclaim fixture](zram-repeated-reclaim.md) passed in 2 GiB QEMU
guests with one, two and four vCPUs. Each guest completed 100 ordinary child
exits, 2,400 repeat evictions and 3,600 user read faults that restored swapped
pages. Every repeat deferred all 12 candidates on its first scan and evicted
them on its second. Candidate capacity stayed at 4,250 through restoration;
the census checked enrollment, free-list reservations and post-reap resource
counts. A separate four-vCPU guest passed 100 ordinary exits with 12 swapped
leaves still live per child, releasing the slots and backing without swap-in.

The one-vCPU production-policy pressure workload passed the old 672 MiB
queue growth stop and initialized 704 MiB. At 672 MiB, native settled heap
use was 6,847,656 bytes before and 6,859,792 bytes after. At 704 MiB the new
queue held 184,322 candidates with room for 184,450, using 4,435,480 bytes
of the 7,056,016-byte settled heap. The guest then halted on a 280-byte
allocation with 16,774,912 bytes used and 2,304 bytes free. That request and
pressure profile match the timer telemetry failure confirmed in the prior
audit; this run did not capture the caller. No candidate reserve failure was
logged. The last complete checkpoint was 74,573 free frames above the start
watermark, with zero activations, scans and evictions.

Natural activation, same-page restore and re-eviction, timer responsiveness
during natural reclaim, and pressure worker reap remain unqualified. The
two- and four-vCPU natural-pressure reruns were not run this stage. Timer
telemetry was left unchanged. [Machine results](reclaim-segmented-storage-results.json)
record the commands, serial evidence and separate gate outcomes.
