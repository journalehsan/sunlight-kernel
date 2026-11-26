# Repeated anonymous-page reclaim

## Candidate lifecycle

Eager anonymous mappings enroll each resident frame after publication. Demand-backed mappings enroll the frame when the first access installs a PTE. Reclaim removes the candidate and replaces the present PTE with a generation-checked ZRAM slot. The old frame is released after the shootdown. Previously, swap-in installed a new owned frame and discarded the slot without enrolling the new frame. After the first restore, ordinary pressure reclaim therefore had no queue entry for that virtual page.

Swap-in now appends one candidate after PTE publication and slot discard. The entry identifies the full scheduler PID of the address-space owner, the virtual page and the new PMM frame. Eligibility still requires an anonymous, unmap-capable region, matching owner and a present user mapping. The trusted swap administrator and display service remain excluded from pressure reclaim. The scanner rechecks the region, frame ownership and PTE before eviction. The whole transition is serialized by the scheduler and PMM locks. Swap-out removes the resident entry and a successful restore consumes its swapped marker exactly once. Failed restore leaves the slot live. Existing unmap, replacement and process-reap paths retire their candidates and slots.

Candidate reservations account for live ZRAM slots alongside resident entries. Pressure reclaim pops a candidate before publishing a slot so that the slot takes over the same capacity. Targeted eviction can encounter a page outside the queue and reserves room before publishing a slot. Demand allocation also reserves room before publishing its PTE. Restore uses the capacity vacated by slot discard and performs no candidate heap allocation. Per-pool free-list reservation continues to cover every issued slot index.

A restored candidate enters at the tail and rotates once at its first eligible scan. Fault-in itself provides the signal that the page was recently accessed. The pass's initial queue length bounds scanning, so the same pass cannot revisit the deferred candidate. The one-shot flag fits in the low alignment bit of the stored physical frame address and preserves the candidate entry size. This uses the existing bounded queue, watermark, deadline and scan limits. Native spawn uses its ordinary entry points. ZRAM runs entirely in ring 0 with RAM backing and has no VFS or persistent-storage dependency.

## Native qualification

The opt-in `zram_fault_test` build uses ordinary spawn, anonymous `mmap`, `mprotect`, userspace loads and stores, and normal exit. Each guest runs 100 serial children. Each child initializes 16 pages and marks six read-only. Authenticated diagnostic fill evicts the first 12 pages once. After the first restore, two further rounds select those same 12 candidates and pass them through the production candidate eligibility, deferral and eviction code. Each round checks that the first scan defers all 12 and the second compresses all 12. The scoped fixture chooses the addresses so the result does not depend on unrelated pages or guest-wide memory pressure.

Each round checks every byte of the 16-page mapping, then writes and reads back a new value on each restored writable page. Those values must survive the next compression and native x86 vector 14 read fault. Checkpoints match the PTE permissions to the VM ledger, including six restored read-only NX pages and six restored read-write NX pages. They also check PMM ownership, the live slot behind each swapped leaf and the exact candidate matching every resident leaf. Init collects exit status 37 once and waits for the normal scheduler reap with the mapping still present.

Each topology completed 300 compression and restore rounds, including 2,400 evictions through the pressure-reclaim candidate path and 3,600 successful vector 14 user reads of non-present pages. Each child verified 196,608 bytes across the three rounds. The settled resource census returned to the boot baseline for free frames, user and page-table classes, and global candidate count. On each reap the child shed 567 user frames and 10 page tables. Slots, descriptors, VM regions, kernel stack and child candidates reached zero.

### Four vCPUs and two pools

| Resource | Guest baseline | Child resident | Compressed | Restored | After reap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Free PMM frames | 261,764 | 261,187 | 261,197 | 261,187 | 261,764 |
| User-private PMM frames | 28,201 | 28,768 | 28,756 | 28,768 | 28,201 |
| Page-table PMM frames | 84 | 94 | 94 | 94 | 84 |
| Child-owned PMM frames | 0 | 567 | 555 | 567 | 0 |
| Global candidates | 4,098 | 4,114 | 4,102 | 4,114 | 4,098 |
| Child candidates | 0 | 16 | 4 | 16 | 0 |
| Live slots | 0 | 0 | 12 | 0 | 0 |
| Compressed payload bytes | 0 | 0 | 2,004–2,010 | 0 | 0 |
| Physical backing bytes | 0 | 0 | 8,192 | 0 | 0 |

The first compressed round contains 2,004 payload bytes. Each repeat contains 2,010 bytes after userspace changes six bytes. Each pool holds six slots and one physical backing frame while populated, then retains six vacant slot indices and one vacant backing descriptor. Slot and free-list capacities remain at eight per pool and backing-descriptor capacity remains at four. The candidate queue's capacity warms from 4,098 to 4,115 and stays there through all restores, repeat evictions and reaps.

The settled whole-kernel heap measured 2,598,688 bytes in 92 samples and reached 4,365,080 bytes in the others before returning to the warmed level. Physical counts and pool capacities held across these excursions. The heap counter has no allocation-site attribution.

### One-pool guests

| vCPUs | Pools | Children | Free-frame baseline | User-private / page-table baseline | Physical backing at eviction |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1 | 100 | 262,276 | 28,201 / 84 | 4,096 B |
| 1 | 1 | 100 | 262,532 | 28,201 / 86 | 4,096 B |

All 12 slots share one backing frame. Eviction increases each guest's free count by 11 frames, and restore returns it to the child's resident count. Each pool retains 12 vacant slot indices and one vacant backing descriptor. The slot and free-list capacities stay at 16 and backing-descriptor capacity stays at four. Candidate counts and queue capacity follow the two-pool run. The one-vCPU heap settled at 2,598,496 bytes in every sample. The two-vCPU heap settled at 2,598,592 bytes in 99 samples and 4,364,760 bytes in one.

## Regression coverage

A separate `zram_lifecycle_test` image repeated ordinary exit with 12 live swapped leaves in a fresh 2 GiB, four-vCPU, two-pool guest. All 100 reaps released 555 resident frames, 10 page tables and 12 swapped leaves per child, with zero swap-in attempts. Settled PMM, live storage and candidate counts matched the guest baseline. Production kernel and userspace release builds and the host kernel, scheduler, retirement, ZRAM, slot, IPC, descriptor and VFS regression suites passed.

The existing isolated candidate passed ordinary Phase 3.0 boot at four, two and one vCPU, and standard SWAP-1 at four vCPUs, with the production reclaim changes applied. The primary checkout still reports the same four pre-existing VFS ownership findings. The isolated candidate passes the ownership gate. The native ZRAM images came from the primary checkout.

## Reproduction and limits

Build the repeated workload with `cargo build --offline --locked -p sunlight-kernel --features zram_fault_test`. Use `tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel <output.iso>`. Boot independent 2 GiB QEMU TCG guests with four, two and one vCPU. Wait for `[ZRAM-LIFE] fault workload completed cycles=100 status=37 collected_once=1`. Expect 200 `[ZRAM-REPEAT]` lines per guest, each reporting zero evictions on the first scan and 12 on the second. Exact invocations, build and serial hashes, checkpoint ranges and first and last child witnesses are in the machine results. [Machine results](zram-repeated-reclaim-results.json)

The repeated workload covers serial children, selected candidate reclaim, successful read-triggered page faults and stores to resident writable pages. Standard SWAP-1 independently covers pressure-driven selection, direct swap-in, corrupt payload and stale slot rejection, and swapped-page munmap. Repeated reclaim under natural pressure, concurrent children, write- or instruction-fetch-triggered restoration, exec, remote termination and generation exhaustion remain unqualified. The one-scan deferral can be consumed quickly under sustained pressure. Accesses after restore do not refresh it.

The prior fault-driven report's referenced results JSON was absent from the checkout. Its archived native summaries confirm that the older workload restored 12 pages per child and left the child with four candidates. [Prior fault-driven report](zram-fault-driven-swap-in.md) · [Populated lifecycle report](zram-process-lifecycle.md)
