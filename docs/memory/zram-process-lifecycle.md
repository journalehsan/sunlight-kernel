# ZRAM process lifecycle qualification

## Workload and scope

The opt-in `zram_lifecycle_test` kernel build embeds fixture variants of init and the existing `freezram` applet. Init launches and waits for the ordinary `sunlight-swapd` service. The kernel computes the same pool configuration used at ordinary boot. Init waits for the service to reap before taking the pre-workload baseline. Init runs one child at a time before launching the rest of the boot services.

Each child uses ordinary native spawn, private anonymous `mmap` and deterministic writes to all 16 pages. Byte `j` in page `i` is `(37*i + 11*floor(j/128)) mod 256`. The child submits 12 pages to the existing authenticated diagnostic fill operation. A read-only checkpoint validates their swapped PTEs and live slot IDs and checks that the other four pages are resident. The child returns 37 through its normal `SYS_EXIT` path with the mapping intact. Init consumes the status once and observes the existing scheduler reaper reach `Reaped` before spawning again. An early run showed that a successful `waitpid` can precede physical reap, so the fixture polls for that state after collecting the status.

The scheduler untracks the exiting process, reclaims its address space, and discards swapped leaves directly. ZRAM empties the payload chunks, returns an empty backing frame to the PMM and retains the vacant slot index for reuse. The fixture samples PMM classes, live ZRAM storage, pool metadata, reclaim candidates and per-process metadata before and after. The serial log also records each normal exit and the reaper's exact user-frame, page-table and swap-block counts.

The L1C accounting reports establish a reference for ordinary scheduler record reuse and reaps with empty swap backing. The ZRAM fixture needs no VFS publication transaction. It uses the normal init namespace handshake and the embedded executable resolver. All tested swap storage and teardown operations execute in ring 0 with RAM backing. [L1C resource accounting](../filesystems/vfs-l1c-resource-closure.md) · [L1C integration readiness](../filesystems/vfs-l1c-integration-readiness.md)

## Resource-management repair

A populated two-pool sample exposed six live slots per pool and room for four indices on each free list. The old allocation path called `try_reserve(1)` when the free list was empty, even while multiple slot indices were outstanding. The unrepaired guest recorded both free-list capacities rising from four to eight during the first reap. The repaired guest reserved eight per pool before the first discard, and none of its 100 reaps grew either list. The fix reserves space for every issued slot index before publishing a new one. The free list stays pool-owned and reusable, and teardown has space to return indices without heap allocation. The slot limit and generation rules are unchanged. [ZRAM pool implementation](../../kernel/src/memory/zram.rs)

## Results

The primary fresh **2 GiB, four-vCPU, two-pool guest passed 100 ordinary lifecycles**. Each populated sample held 12 swapped pages, representing 49,152 logical bytes. The payload compressed to 2,004 bytes and occupied two physical backing frames. Both pools held six slots, 1,002 payload bytes and one backing frame. The following counts held across the full run.

| Resource | Before workload | Child resident | Child populated | After each reap |
| --- | ---: | ---: | ---: | ---: |
| PMM free frames | 261,814 | 261,238 | 261,248 | 261,814 |
| User-private PMM class | 28,201 | 28,767 | 28,755 | 28,201 |
| Page-table PMM class | 84 | 94 | 94 | 84 |
| Child-owned PMM frames | 0 | 566 | 554 | 0 |
| Compressed PMM frames | 0 | 0 | 2 | 0 |
| Compressed payload bytes | 0 | 0 | 2,004 | 0 |
| Backing allocation bytes | 0 | 0 | 8,192 | 0 |
| Live swap slots | 0 | 0 | 12 | 0 |
| Global reclaim candidates | 4,098 | 4,114 | 4,102 | 4,098 |
| Child reclaim candidates | 0 | 16 | 4 | 0 |
| Child VM regions / descriptor entries | 0 / 0 | 5 / 3 | 5 / 3 | 0 / 0 |

Every reap released **554 resident user frames, 10 page tables and 12 swapped leaves**. The ZRAM pool counters independently recorded 1,200 slot releases and zero swap-in attempts across the 100 cycles. At settlement, the child's VM root was reclaimed, its kernel stack and cleanup flag were clear, and its local IPC queues, deferred-reply buffers and owned endpoints were empty. The scheduler retained four rows at capacity 512 and reused the reaped record with an empty descriptor table.

Each pool retained six vacant slots and one vacant backing-frame descriptor after warmup. The pool's slot and free-list capacities stayed at eight each, and its backing descriptor capacity stayed at four. These pool-owned allocations remained available for reuse throughout the run. The four-vCPU whole-kernel heap measured 2,597,512 B before the first child and returned to 2,598,664 B after cycles 1 and 100 and at 86 of 100 settled samples. Other settled samples reached up to 4,365,072 B and returned to the warmed level. The pool capacities and physical counts held across those excursions. The global heap counter has no allocation-site attribution in this run.

### One-pool lifecycle guests

Both lower CPU counts ran the same fixture in independent fresh 2 GiB guests. The existing policy admits one pool at one or two vCPUs. All 12 slots shared one 4,096-byte backing frame, so eviction increased the free count by 11 frames. Both guests returned to their own physical and ZRAM baselines after every reap, with the same per-process reap counts and zero swap-in attempts.

| vCPUs | Pools | Completed cycles | Free-frame baseline | User-private / page-table baseline | Settled heap on all cycles |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1 | 100 | 262,326 | 28,201 / 84 | 2,598,568 B |
| 1 | 1 | 100 | 262,602 | 28,201 / 86 | 2,598,472 B |

The single pool retained 12 vacant slots and one vacant backing-frame descriptor. Standard SWAP-1 requires at least two pools, and its harness selects four vCPUs. The one-pool results qualify the lifecycle on those topologies under their supported policy.

### Regression gates

Production kernel and userspace release compilation and the host kernel, ZRAM codec, slot, scheduler, retirement, IPC, descriptor and VFS suites passed. The isolated candidate passed the ownership guard, ordinary Phase 3.0 boot at 4, 2 and 1 vCPUs, and standard SWAP-1 at four vCPUs. The primary workspace retains the same four pre-existing ownership findings, and its ordinary gate stopped at the guard. The native lifecycle images came from the primary workspace. Raw logs and the census checker are under `target/zram-process-lifecycle/`.

## Reproduction and limits

Build with `cargo build --offline --locked -p sunlight-kernel --features zram_lifecycle_test`. Run `tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel <output.iso>` and boot the ISO in a fresh 2 GiB QEMU TCG guest with four vCPUs and serial output. Look for `[ZRAM-LIFE] completed cycles=100 status=37 collected_once=1`, the per-PID census and the matching scheduler reap records. The machine results include exact guest arguments, image hashes and checkpoints. [Machine results](zram-process-lifecycle-results.json)

The workload qualifies ordinary exit with populated backing for a serial, bounded native workload. It uses targeted diagnostic eviction. Standard SWAP-1 supplies the independent pressure, direct swap-in, exact-data restore and munmap controls. The lifecycle run deliberately produces no accesses to the swapped range after fill and checks the swap-in attempt counter. Fault-driven swap-in, concurrent or large children, slot-generation exhaustion, memory-pressure races, exec and remote termination remain outside this qualification. [SWAP-1 audit](../storage/SWAP1_AUDIT.md)

Detached publication remains disabled. L1C-3 and VFS-D1 remain deferred. Populated ZRAM combined with a VFS transaction, its file and IPC census, and fault-driven restore remain open integration work.
