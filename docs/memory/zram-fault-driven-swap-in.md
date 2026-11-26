# Native fault-driven ZRAM swap-in qualification

The opt-in `zram_fault_test` image runs 100 native `/bin/freezram` children serially after the ordinary `sunlight-swapd` service has configured the pools and reaped. Each child uses ordinary spawn and private anonymous `mmap`. It fills 16 pages with byte `j` of page `i` equal to `(37*i + 11*floor(j/128)) mod 256`, then uses native `mprotect` to make the first six pages read-only. The existing authenticated diagnostic fill evicts pages 0–11. A read-only checkpoint confirms those 12 PTEs reference live ZRAM slots and the other four pages are resident.

The child then reads every byte through its userspace mapping. All first reads of the evicted pages execute the x86 `movzbl` at RIP `0x40b735` in the tested image. The trace is emitted from the successful return branch of the normal x86 page-fault handler. It records vector 14, PID, CR2, RIP and error code `0x4` (user-mode read of a non-present page). Each child's records identify exactly the 12 evicted addresses, between the populated and restored checkpoints. The restored checkpoint follows the comparison of all 65,536 bytes and a store and read-back on each of the six restored writable pages. It checks present/user/NX and writable bits against the expected permissions and the VM ledger. The restored set contains six read-only NX pages and six read-write NX pages.

Init collects exit status 37 once and then waits for the scheduler's physical reap. The workload leaves the mapping live until ordinary process exit. The settled checkpoint checks that the VM root has been reclaimed, the kernel stack and cleanup flag have cleared, and the child has no resident frames, VM regions, open descriptors, reclaim candidates, owned endpoints or buffered IPC entries. Every cycle is checked against the corresponding guest baseline.

## Four vCPUs and two pools

The fresh 2 GiB QEMU TCG guest completed all 100 cycles, with 1,200 successful vector 14 restores. The accounting below held across all cycles. Compressed bytes are payload bytes. Backing bytes are whole physical frames.

| Resource | Before workload | Resident | Populated | Restored | After reap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Free PMM frames | 261,805 | 261,228 | 261,238 | 261,228 | 261,805 |
| User-private PMM frames | 28,201 | 28,768 | 28,756 | 28,768 | 28,201 |
| Page-table PMM frames | 84 | 94 | 94 | 94 | 84 |
| Child-owned PMM frames | 0 | 567 | 555 | 567 | 0 |
| Live swap slots | 0 | 0 | 12 | 0 | 0 |
| Compressed payload bytes | 0 | 0 | 2,004 | 0 | 0 |
| Backing bytes | 0 | 0 | 8,192 | 0 | 0 |
| Global reclaim candidates | 4,098 | 4,114 | 4,102 | 4,102 | 4,098 |
| Child reclaim candidates | 0 | 16 | 4 | 4 | 0 |
| Child VM regions / descriptors | 0 / 0 | 6 / 3 | 6 / 3 | 6 / 3 | 0 / 0 |

Each pool held six live slots, 1,002 payload bytes and one backing frame at the populated checkpoint. Every slot was released during fault-in. Each reap released 567 resident user frames and 10 page tables, with zero swapped leaves to discard. The scheduler reused child row 3 throughout. The pool-owned slot, free-list and backing-descriptor capacities stabilized in the first cycle and stayed fixed through restore and reap. Each pool retained six vacant slots and one vacant backing-frame descriptor for reuse.

The whole-kernel heap settled at 2,598,664 B in 93 of 100 four-vCPU samples, including the first and last. The other samples reached 4,365,056 B and returned to the warmed level. The physical counts and pool capacities held through those excursions. The heap counter does not identify allocation sites.

## Reproduction and limits

Build `cargo build --offline --locked -p sunlight-kernel --features zram_fault_test`. Package the debug ELF with `tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel <output.iso>`. Boot fresh QEMU guests with 2 GiB RAM and 4, 2 and 1 vCPUs. The expected completion marker is `[ZRAM-LIFE] fault workload completed cycles=100 status=37 collected_once=1`. The machine results contain exact QEMU arguments, image and serial hashes, and first and last cycle witnesses. [Machine results](zram-fault-driven-swap-in-results.json)

This workload qualifies serial bounded anonymous-page restoration after targeted eviction. It verifies denied write and execute permissions through PTE and ledger inspection. It exercises successful read-triggered faults and stores to already-restored writable pages. Write-triggered and instruction-fetch-triggered restore, concurrent children, natural pressure during fault-in, slot-generation exhaustion, exec and remote termination remain outside the workload. The restored pages are left out of the reclaim candidate queue by the existing policy.

The standard SWAP-1 gate separately exercises pressure reclaim, direct internal swap-in, corrupted-payload rejection, stale-slot rejection and swapped-page munmap. Its direct swap-in assertions are regression controls. The vector 14 records and successful userspace reads supply the native fault evidence. No production memory-management defect was demonstrated by this workload. The existing free-list reservation and ring-0 RAM-only storage are used as before. [SWAP-1 audit](../storage/SWAP1_AUDIT.md) · [Populated-exit qualification](zram-process-lifecycle.md)
