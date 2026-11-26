# SunlightOS reclaim metadata and heap capacity audit

**Natural reclaim remains unqualified.** In the saved 2 GiB native pressure workload, the candidate queue hit a contiguous-allocation limit on one and four vCPUs. The two-vCPU guest halted on kernel heap exhaustion. Bounded debugger runs on the same image reproduced queue failure and identified a second bottleneck on all three topologies: timer telemetry builds a transient frame-owner tree on the same heap and can panic on a node allocation. The saved workload reached zero natural activations, scans, or evictions across all three guests. [Saved pressure report](zram-natural-pressure.md) · [Saved pressure results](zram-natural-pressure-results.json) · [Audit results](reclaim-metadata-capacity-audit-results.json)

## Heap and metadata accounting

The kernel maps 4,096 PMM frames into a contiguous virtual range at boot and initializes a 16,777,216-byte heap. There is no runtime growth path. The image uses `linked_list_allocator` 0.10.6 with first-fit allocation and coalescing adjacent free spans. Its 16-byte free nodes live inside free spans. Allocations are rounded up to at least 16 bytes and to a multiple of 8, subject to their requested alignment. There is no separate per-allocation header. An otherwise large enough span can be unusable when alignment or splitting would leave a fragment smaller than a free node. The allocator implements allocate and free and uses the default allocate-copy-free reallocation, so growing the queue requires the full replacement span while the old buffer is still live. An empty 16 MiB heap could hold at most 699,050 candidate entries in one buffer if it held nothing else. That arithmetic ceiling omits replacement space and other kernel allocations. [Heap setup](../../kernel/src/memory/heap.rs) · [Candidate reservation](../../kernel/src/memory/swap.rs)

| Allocation or structure | Measured size | Accounting |
| --- | ---: | --- |
| Candidate | 24 B | Full scheduler PID, virtual page and physical frame with a packed deferral bit |
| Static candidate mutex and descriptor | 40 B | Kernel image |
| Eager mmap rollback journal | 40 B per page | 327,680 B per 32 MiB batch or 10,240 B per 1 MiB batch, transient heap |
| Mapping region / address-space ledger | 40 B / 11,032 B | 256 committed records and 16 pending entries per ledger. The 64-slot static ledger array including its mutex occupies 707,592 B |
| VM region staging vector | 10,240 B per staged image | Heap when staging a region layout, outside the ordinary append path used here |
| PMM owner array / bitmap | 16,777,216 B / 524,288 B | Static image storage sized for 16 GiB of tracked address range |
| Process record / kernel stack | 4,032 B / 32,768 B | Record occupies scheduler vector capacity, stack is a separate heap allocation |
| Page table | 4,096 B | PMM page-table class |
| Telemetry frame vector / owner-tree nodes | 8 B per vector slot / 280 B leaf / 376 B internal | Transient heap, including vector spare capacity |
| ZRAM slot / free index / backing descriptor | 32 B / 4 B / 24 B | Variable heap capacity. Populated backing frames use PMM |

Sizes come from DWARF or ELF symbols in the saved ISO's kernel, or fixed allocation counts in source. A page-table frame consumes PMM headroom separately from the heap. Direct anonymous insertion preflights the static region ledger and may merge compatible adjacent ranges, so the ledger's storage does not grow with resident page count. The stable nonqueue heap aggregated 2,620,536 bytes after startup across all three topologies. It includes scheduler, stacks and other live kernel state. The samples do not provide allocation-level attribution for that entire baseline. [VM ledger](../../kernel/src/process/region.rs) · [Address-space storage](../../kernel/src/process/address_space.rs) · [Anonymous mapping](../../kernel/src/process/mmap.rs) · [PMM](../../kernel/src/memory/pmm.rs) · [Telemetry](../../kernel/src/telemetry.rs) · [ZRAM](../../kernel/src/memory/zram.rs)

## Candidate reservations and scaling

Eager mmap reserves the journal and queue capacity for the complete batch before entering its per-page watermark check. Queue capacity must cover resident candidates, live swapped slots and the incoming entries. A swap-out trades a resident entry for a slot's space and swap-in consumes that existing reservation when it re-enrolls the restored page. Demand-backed mappings defer frame and candidate provisioning until the fault path. The ZRAM free-list reservation covers every issued index so discard has space even after the pool has issued all its current slots. Natural-pressure samples had zero slot, free-index and backing-descriptor capacity, so none of those allocations contributed to these stops. [Candidate lifecycle and reservation controls](zram-repeated-reclaim.md) · [Swap path](../../kernel/src/memory/swap.rs) · [ZRAM pool](../../kernel/src/memory/zram.rs)

Once startup settled, every 32 MiB batch added 8,192 candidates and 196,608 bytes of resident heap queue storage. PMM user frames grew by 8,192 per batch and page tables grew by 16 frames per batch, with one additional table during the first batch. That is 24 bytes of resident heap and about 8 bytes of PMM page-table storage per anonymous page at the observed density. The 40-byte-per-page journal is temporary. A successful growth also briefly overlaps the old and new queue buffers. The samples record completed batches, so they omit both temporary costs.

| Saved guest | Initialized MiB | PMM free / start watermark | Candidate length and capacity | Queue / total heap bytes | Added PMM page tables | Outcome |
| --- | ---: | ---: | ---: | ---: | ---: | --- |
| 1 vCPU, 1 pool | 672 | 88,535 / 5,760 | 176,130 | 4,227,120 / 6,847,656 | 337 | ENOMEM, reaped |
| 2 vCPUs, 1 pool | 544 | 121,109 / 5,756 | 143,362 | 3,440,688 / 6,061,224 | 273 | Heap panic after next mmap returned |
| 4 vCPUs, 2 pools | 672 | 87,765 / 5,748 | 176,130 | 4,227,120 / 6,847,656 | 337 | ENOMEM, reaped |

The saved one- and four-vCPU failures requested 184,322 entries, or 4,423,728 contiguous bytes. Heap use at the failure was 7,175,336 bytes, exactly the prior sample plus the journal. The remaining 9,601,880 bytes would have covered the request in aggregate, so the allocator and request layout imply there was no admissible contiguous span. The original serial capture did not measure the largest hole. The subsequent one-vCPU debugger run directly confirmed this failure mechanism at a different batch. The saved reaps dropped candidate length to 4,098 and reclaimed the worker's 345 table frames, including its pre-workload tables. They retained 172,032 vacant queue entries (4,128,768 bytes). The post-reap heap was 6,814,600 bytes, including 2,587,480 bytes outside that queue. This is one cycle of retained high-water capacity. [Saved pressure results](zram-natural-pressure-results.json) · [Audit results](reclaim-metadata-capacity-audit-results.json)

## Failure attribution

Six bounded debugger runs reused the saved ISO. GDB stopped on the failed reserve branch or on allocation-error entry and read the allocator free list directly. The sum of the span sizes matched the allocator's aggregate free counter at every stop. The timing and allocation interleaving varied across boots. To reproduce the probes, use the saved QEMU arguments with `-S -gdb unix:<socket>,server=on,wait=off`, attach GDB with the ELF extracted from the ISO, and set hardware breakpoints at `0xffffffff80014d68` and `0xffffffff80001ef0`. These addresses are specific to the recorded ELF hash. The audit results give the timeout, stop logs and free-span sizes.

| Debugger run | Failed allocation | Request B | Free B | Largest free span B |
| --- | --- | ---: | ---: | ---: |
| 1 vCPU, initial | Telemetry tree leaf | 280 | 2,120 | 264 |
| 1 vCPU, follow-up | Candidate replacement | 4,030,512 | 9,995,096 | 3,801,200 |
| 2 vCPUs, initial | Telemetry tree internal node | 376 | 2,552 | 384 |
| 2 vCPUs, follow-up | Candidate replacement | 3,244,080 | 2,232,384 | 2,230,096 |
| 4 vCPUs, initial | Telemetry tree leaf | 280 | 1,248 | 264 |
| 4 vCPUs, follow-up | Telemetry tree leaf | 280 | 2,584 | 288 |

Each request was 8-byte aligned. The 384-byte and 288-byte spans would each have left 8 bytes, too small for a free node. Neither run had another suitable span. The two-vCPU replacement also failed the aggregate budget. That stop had 14,544,832 bytes in use, of which 3,047,472 were the old queue and 327,680 were the journal. The remaining 11,169,680 bytes include the approximately 2.62 MB steady baseline and other live allocations. This stop did not census those allocations by site.

The telemetry panic traces lead from the timer interrupt through `commit_telemetry_snapshot_inner` to an infallible insertion into `BTreeMap<u64, (usize, bool)>`. The snapshot walks user page tables, collects and deduplicates frame addresses per address space, and retains those vectors while the owner tree counts distinct frames across address spaces. Unique anonymous pages grow both structures. Their scratch allocations are freed after a successful snapshot, which explains why the settled queue slope misses this pressure. On the four-vCPU follow-up, the debugger measured a 156,199-element worker frame vector with 262,144 slots (2,097,152 bytes) and 160,210 live keys in the owner tree. At most 11 keys fit in a node, so that tree occupied **at least 4,078,200 bytes**, using the smaller node size for every node. The actual tree includes partly filled and larger internal nodes. Nonqueue heap occupancy was 10,320,192 bytes above the matched steady baseline at that stop. That delta includes telemetry scratch and any other live changes, so it is not an exact tree size.

The original two-vCPU log records a 280-byte failure with 16,775,464 bytes used and 1,752 bytes free. The new traces confirm the telemetry panic mechanism and reproduce the 280-byte layout on the same image. There is no historical stack trace from the exact original stop. The earlier discarded chunked-queue experiments also hit small-allocation panics. Chunking addresses contiguous growth and by itself leaves the telemetry heap exposure. [Timer call path](../../kernel/src/arch/x86_64/interrupts.rs) · [Telemetry allocation site](../../kernel/src/telemetry.rs) · [Audit results](reclaim-metadata-capacity-audit-results.json)

## Watermark reachability

The start watermark is total tracked PMM frames divided by 64, clamped by policy, and checked at or below the resulting free-frame count. The target uses division by 32 and its own clamps. The eager path checks before allocating each frame, after both heap reservations. The saved guest runs stopped 82,775, 115,353 and 82,017 free frames above start for one, two and four vCPUs, respectively. [Watermark implementation](../../kernel/src/memory/swap.rs) · [Saved pressure results](zram-natural-pressure-results.json)

The following are **estimates** using start-of-workload free frames, the observed page-table density, no further unrelated PMM changes and zero ZRAM slots. The queue-growth peak includes the old queue, the new queue, the journal and steady nonqueue heap. The full-telemetry column is a separate scenario using minimum queue capacity, the worker's frame vector and a densely packed owner tree for only the new anonymous pages. It excludes spare keys in nodes, other address spaces and larger internal node sizes.

| vCPUs | Anonymous MiB at start watermark | Quiet queue-growth peak | Heap with full telemetry, conservative estimate |
| --- | ---: | ---: | ---: |
| 1 | 994.71 | 15,047,896 B | ≥17,409,520 B |
| 2 | 993.72 | 15,035,608 B | ≥17,397,008 B |
| 4 | 991.75 | 15,138,008 B | ≥17,372,056 B |

The 1,012 MiB workload cap permits crossing the modeled watermark, with a subsequent allocation available to observe it. The quiet byte budget could fit on an ideally laid-out heap, but it would still need a contiguous replacement of roughly 6.2 MB. The full telemetry estimates already exceed the fixed heap before charging realistic tree occupancy. Under the observed metadata paths, the workload cannot reliably reach and operate at the production watermark. This is specific to this workload and capacity envelope. A different source of PMM pressure can have a different metadata cost. The historical repeated-reclaim tests establish controlled same-page restoration and re-eviction. Actual native pressure-driven reclaim, continued timer responsiveness during reclaim, and successful cleanup after that workload remain unqualified. [Historical controls](zram-repeated-reclaim.md)

## Architectural options

1. **Bound telemetry scratch and make allocation failure recoverable.** Evaluate reusable accounting storage keyed by PMM frame identifiers, with touched-frame tracking or epochs to avoid clearing the whole range every sample. Preserve alias and cross-address-space share accounting, and publish a complete snapshot or retain the prior complete sample. A dense table has a predictable fixed cost that scales with the tracked frame span, including holes. Chunked or sparse scratch saves space in small guests but needs explicit budgeting and cleanup. Moving the walk outside the interrupt path reduces interrupt latency exposure and still requires valid address-space lifetimes and bounded scratch.
2. **Segment candidate backing and account for it explicitly.** PMM-backed segments would replace the multi-megabyte contiguous heap request with bounded allocations. Preflight a whole batch and retain sufficient resident-plus-live-slot capacity for allocation-free re-enrollment. Segment retirement must account for vacant slots' claims and synchronize with scans and teardown. This adds chunk management and PMM charges, and the same 24 bytes per candidate still scale linearly unless the representation changes.
3. **Shrink transactional mmap metadata.** A journal of compact page indices or range-based rollback could reduce the 40-byte-per-page temporary cost where mappings are newly installed into empty PTEs. Fixed replacement and rollback over swapped or shared leaves need their existing ownership and failure semantics. Prove rollback, publication and shootdown ordering before applying the same approach to those paths.
4. **Evaluate denser candidate metadata after isolating the two bottlenecks.** A per-address-space index or compact frame identifier can reduce the resident slope, at the cost of identity lifetime and address reconstruction complexity. Preserve full PID generation, PTE and frame-owner validation, queue ordering and one-scan deferral. Reusing the PMM owner field alone would lose identity because that field holds only 24 owner bits.

The candidate and telemetry bottlenecks need independent capacity checks. Keep the existing watermark, eligibility and ownership rules, ordinary spawn, allocation-free reservations, and ring-0 ZRAM independent of VFS. The audit used the existing image without production changes. An unchanged-workload native run must reach the watermark and witness the same anonymous page compressed, restored by ordinary access, and selected again before natural repeated reclaim is qualified.
