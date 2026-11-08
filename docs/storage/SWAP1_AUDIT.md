# SWAP-1 audit and implementation record

This audit distinguishes the pre-SWAP-1 `HEAD` snapshot from the implementation
in the working tree. Pre-change line numbers are from `git show HEAD:<path> | nl
-ba`; current line numbers refer to the files in this tree.

## Pre-SWAP-1 verdict

Classification: **initialized but unused**, **test/manual-only**, and **unsafe
under real pressure**. The page lifecycle was substantially real when invoked,
but no production path called reclaim. A boot smoke block and `freezram` could
change usage telemetry, so nonzero swap usage did not prove eviction.

1. **Store and initialization.** One lazy `Mutex<ZramState>` held a sparse
   `Vec<(BlockId, Option<Vec<u8>>)>` (`kernel/src/memory/zram.rs` HEAD:18-22,
   61-62). Boot called `zram::init` before the heap and a synthetic smoke fill
   after heap initialization (`kernel/src/main.rs` HEAD:291-293, 337-361,
   2837-2853).
2. **256 MiB meaning.** `256 * MiB / 4096` was only the logical ID/slot-count
   ceiling (`zram.rs` HEAD:4-6, 32-38). It was not reserved RAM, compressed-byte
   capacity, or a physical-memory budget.
3. **Allocation behavior.** Slot metadata and per-page payload vectors grew on
   demand (`zram.rs` HEAD:27-28, 46-48, 68-80, 134-141). No 256 MiB reservation
   occurred, but there was no compressed-byte or heap hard limit.
4. **LZ4/API/format.** Kernel used `lz4_flex 0.11`, block `compress` and
   `decompress_into`; a one-byte private tag selected raw (`0`) or LZ4 (`1`),
   with no prepended original-size field (`zram.rs` HEAD:64-100;
   `kernel/Cargo.toml` HEAD dependency `lz4_flex`).
5. **Codec failures.** Compression allocation was infallible at the API level
   and could hit the kernel allocation handler. Decompression errors, bad tags,
   raw length mismatch, and output lengths other than 4096 became generic
   `InvalidData` (`zram.rs` HEAD:68-100, 144-152).
6. **Incompressible pages.** They were stored raw as 4097 bytes including tag
   (`zram.rs` HEAD:70-80), potentially consuming more heap than the freed frame.
7. **Slot representation.** `BlockId = usize` (`zram.rs` HEAD:8), allocated by
   linear search through a sparse vector (`zram.rs` HEAD:36-58).
8. **Slot fields.** Identity contained only the slot index. There was no pool
   ID, generation, compressed length, checksum, or permission metadata
   (`zram.rs` HEAD:18-22).
9. **PTE marker.** A swapped leaf was non-present with empty flags and physical
   address field `(block_id + 1) << 12`; decoding reversed that transform
   (`kernel/src/process/address_space.rs` HEAD:718-725, 747-753, 931-944).
10. **Candidate lifecycle.** Successful anonymous `mmap` registered every
    installed page (`kernel/src/process/mmap.rs` HEAD:350-358, 424-435).
    `munmap`/rollback removed frames and process reap removed a PID's candidates
    (`kernel/src/memory/swap.rs` HEAD:28-49;
    `kernel/src/sched/mod.rs` HEAD:1843).
11. **Selection.** Reclaim used FIFO `Vec::remove(0)`, an O(n) operation, and
    permanently removed candidates even on transient/full/incompressible
    failure (`swap.rs` HEAD:159-186).
12. **Swap-out flow.** It copied through HHDM into a compressed slot, validated
    the current `(frame, flags)`, rolled the slot back on mismatch, atomically
    replaced the PTE, synchronously invalidated through MM-2B, and only then
    freed the frame (`swap.rs` HEAD:58-94;
    `address_space.rs` HEAD:699-762). The release ordering was correct.
13. **Swap-in flow.** Fault handling identified the marker and ledger
    protection (`kernel/src/arch/x86_64/interrupts.rs` HEAD:520-563). It
    allocated an owned frame, decompressed into it, replaced the exact marker,
    then discarded the slot (`swap.rs` HEAD:103-151). The frame was not
    explicitly sanitized first, though successful decompression overwrote 4096
    bytes.
14. **Rollback.** Swap-out discarded a committed slot when PTE validation or
    replacement failed (`swap.rs` HEAD:68-90). Swap-in freed the new frame and
    retained marker/slot on allocation/read/PTE failure (`swap.rs` HEAD:111-147),
    but ignored discard failure after publication (`swap.rs` HEAD:148).
15. **Permissions/state.** The PTE marker preserved none. Fault-in reconstructed
    W/X/NX from the MM-2C ledger (`interrupts.rs` HEAD:542-563). Ledger region
    identity survived because residency did not alter the ledger. Readability
    had no independent x86 leaf bit; CoW/software state and checksum were not in
    the slot. Tracked candidates originated in eager anonymous mappings, not a
    general CoW reclaim registration path.
16. **MM-2C/MM-2D.** Ledger validation treated residency separately. MM-2D
    preflight recognized a live swapped marker and commit released its ZRAM
    block without fault-in after synchronous range invalidation
    (`kernel/src/process/mmap.rs` HEAD:550-614, 680-703). Process teardown walked
    non-present leaves and discarded blocks (`address_space.rs` HEAD:1288-1321).
17. **Telemetry.** Only logical total slots, used slots, and sum of payload
    lengths were available (`zram.rs` HEAD:212-222); sysinfo and telemetry
    exported logical used and compressed KiB (`syscall.rs` HEAD:4371-4399;
    `kernel/src/telemetry.rs` HEAD:202-204, 281-287).
18. **Pressure trigger.** There was no caller of `memory::swap::reclaim`; PMM
    allocation returned `None` directly and anonymous `mmap` failed
    (`mmap.rs` HEAD:359-377).
19. **Daemon/background/direct reclaim.** None existed. Only the boot smoke,
    `freezram`, focused MM tests, and the otherwise-unreferenced reclaim routine
    could exercise storage.
20. **Normal workload evidence.** None. Because no production reclaim caller
    existed, no ordinary workload could swap out a mapped page; only synthetic
    or explicit test/manual paths could increase usage.
21. **Lock order.** Live VM paths used Scheduler -> PMM, documented at
    `kernel/src/main.rs` HEAD:522-529. Swap then briefly acquired candidate or
    global ZRAM mutexes; address-space ledger access and MM-2B shootdown occurred
    under Scheduler/PMM. No explicit complete PMM/address-space/swap/ZRAM/ledger/
    scheduler ordering contract existed, but the inspected swap calls did not
    hold ZRAM across shootdown.
22. **SMP safety.** Candidate and ZRAM structures were mutex-protected and live
    page-table changes were serialized by Scheduler, but one global ZRAM lock,
    linear slot lookup, FIFO shifting, and no generation made the design hot and
    stale-reference unsafe.
23. **Process exit.** Reap removed candidates (`sched/mod.rs` HEAD:1843), then
    address-space teardown discarded every swapped leaf (`address_space.rs`
    HEAD:1288-1321).
24. **munmap.** Swapped anonymous leaves were removed and released exactly once
    without swap-in (`mmap.rs` HEAD:550-614).
25. **exec.** `exec_into_process` reclaimed the old address space through the
    same teardown walker (`kernel/src/process/spawn.rs` HEAD:35-109), including
    swapped leaves.
26. **ZRAM full.** Slot allocation returned `OutOfSpace`, swap-out left the
    mapping/frame intact, but reclaim dropped that candidate and continued only
    while other candidates remained (`zram.rs` HEAD:36-53;
    `swap.rs` HEAD:165-186).
27. **PMM exhausted on swap-in.** Allocation returned `OutOfSpace` before
    touching the slot/PTE (`swap.rs` HEAD:111-125). The page remained
    recoverable, but the fault handler returned failure and the user fault was
    subsequently fatal; no emergency reserve existed.
28. **Heap in fault path.** LZ4 decompression itself used `decompress_into`, but
    successful swap-in called `track_anon` with `Vec::push` (`swap.rs` HEAD:126,
    148-150). Capacity was normally retained from prior tracking, but the API did
    not prove allocation-free behavior. Swap-out always allocated compressed
    vectors.
29. **Boot ordering.** Kernel initialized PMM/VMM/heap and later spawned init;
    init launched base services through the kernel spawn endpoint
    (`main.rs` HEAD:275-361, 504-538;
    `services/init/src/main.rs` HEAD:82-99). A policy service could start from
    init after syscall/IPC/SMP system information was ready.
30. **Privilege/IPC options.** The kernel already had embedded-path resolution,
    per-process identity/address-space generation, capability-gated spawn, and
    kernel-authenticated syscall caller identity. Nameserver names alone were
    replaceable registry metadata and were not suitable authorization.
31. **System information.** PMM `stats()` exposed tracked usable total/free
    frames (`kernel/src/memory/pmm.rs` HEAD:251-258); Scheduler exposed
    `online_cores`; sysinfo exported PMM total (`syscall.rs` HEAD:4380-4384).
32. **Disk swap/suspend/hibernate.** No disk swap, swapfile, S3, S4, resume, or
    hibernation-image code existed. ACPI implemented only S5 power-off
    (`kernel/src/arch/x86_64/acpi.rs`:608-675).

## SWAP-1 result

- Policy is pure/shared/tested at `ipc/src/swap_policy.rs`:39-109. It uses PMM
  usable bytes, online CPUs, explicit 2/8 GiB boundaries, 4 KiB rounding, a
  32-pool maximum, 256 MiB minimum logical pool size, checked representability,
  and deterministic first-pool remainder distribution.
- The physical payload budget is `min(RAM/4, logical/2, 2 MiB)`, page-rounded
  (`ipc/src/swap_policy.rs`:9-12, 77-84). The 2 MiB ceiling follows the fixed
  8 MiB kernel heap (`kernel/src/memory/heap.rs`:8-13); it is not reserved.
- Pool state, hard metadata limit (16,384 slots), fallible slot/free metadata,
  per-pool byte limits, per-pool locks, stats, and immutable one-shot config are
  in `kernel/src/memory/zram.rs`:11-12, 32-175, 178-367.
- Slot identity is pool:5 / index:22 / generation:13 in the PTE-safe 40-bit
  field (`kernel/src/memory/swap_slot.rs`:3-60). Generations never wrap: an
  exhausted slot is retired (`zram.rs`:151-175, 270-281).
- LZ4 compression now uses allocation-free `compress_into`; compressed data
  must be smaller than a raw page. Incompressible data is rejected. Every slot
  stores original-page FNV-1a, and swap-in validates length, exact 4096-byte
  output, and checksum before publication (`kernel/src/memory/zram_codec.rs`:3-57).
- Pool selection hashes address-space generation, virtual page and CPU; it
  starts fairly and tries each pool at most once (`kernel/src/memory/swap.rs`:
  109-114; `kernel/src/memory/zram.rs`:406-429).
- Swap-out and swap-in preserve full PTE flag bits in slot metadata, validate
  them against MM-2C ledger protection, sanitize the allocated frame, zero
  failed output, keep failed slots recoverable, and do not hold a pool lock
  across MM-2B (`kernel/src/memory/swap.rs`:94-215).
- Direct reclaim starts below bounded `max(total/64, 256)` and targets bounded
  `max(total/32, 512)` pages, capped at 64/128 MiB respectively. Each activation
  is capped at 256 pages and a bounded candidate scan; failures rotate rather
  than busy-loop (`kernel/src/memory/swap.rs`:18-23, 80-92, 261-339). It is
  invoked before anonymous `mmap` frame allocation
  (`kernel/src/process/mmap.rs`:367-373).
- Eligibility revalidates anonymous ledger kind/policy/owner/PTE, excluding
  display and swap-policy services; SHM, framebuffer, telemetry, boot-shared,
  kernel, DMA, MMIO and page-table frames never enter this candidate path
  (`kernel/src/memory/swap.rs`:218-259).
- `sunlight-swapd` reads system info, computes/submits policy once, checks pool
  health, and exits (`services/sunlight-swapd/src/main.rs`:13-37). Init starts it
  after timer and before the general supervisor (`services/init/src/main.rs`:
  22-35).
- SwapAdmin is granted only when the kernel resolves the embedded swapd path,
  not by caller-supplied process/name-server name
  (`kernel/src/process/spawn.rs`:23-25, 522-524, 581-585). The syscall binds PID
  plus address-space generation, recomputes policy from kernel PMM/Scheduler
  state, rejects ordinary clients and reconfiguration
  (`kernel/src/arch/x86_64/syscall.rs`:4407-4485). Reap revokes owner authority
  without disabling configured pools (`kernel/src/sched/mod.rs`:1843-1845;
  `kernel/src/memory/zram.rs`:369-375).
- Bounded aggregate and per-pool statistics are available at
  `kernel/src/memory/zram.rs`:32-75, 465-509; reclaim diagnostics are at
  `kernel/src/memory/swap.rs`:37-78, and read-only userspace snapshots are
  provided by swapctl operations 5/6 in
  `kernel/src/arch/x86_64/syscall.rs`:4487-4588.

## Hibernation and future disk-swap readiness (audit only)

1. A 512-byte `BlockDevice` supports read/write/count and a small write-back
   cache can flush dirty slots (`sunlight-block/src/lib.rs`:24-40, 79-123).
   This is enough for raw block I/O, not a safe preallocated swapfile.
2. The FAT32 layer is explicitly read-only and only exposes first cluster/size;
   it follows mutable cluster chains during reads and offers no allocation,
   extent enumeration, or pinning (`sunlight-fat/src/fat.rs`:14-31, 79-103,
   198-245). Physical extents therefore cannot yet be pinned/discovered safely.
3. Earliest current block access is after driver/FAT bootstrap, well after PMM,
   VMM, heap, ACPI and IDT. Resume-before-normal-kernel restore support does not
   exist; a HIBERNATE-1 design must add a minimal early reader or bootloader path.
4. There is no driver/device quiesce protocol, so storage, network, display,
   timers, DMA and interrupt state cannot yet be frozen consistently.
5. SMP has bring-up and MM-2B IPIs, but no stop-all-secondary-CPUs snapshot/
   resume rendezvous or saved AP architectural-state format.
6. A future image must exclude kernel/boot trampoline, page tables used by the
   resume reader, MMIO, framebuffer, DMA rings/buffers, device-owned/pinned
   frames, and the image I/O workspace.
7. The header must include magic, format version, kernel/build identity, usable
   RAM size, page/chunk counts, compression algorithm, per-chunk/image checksum,
   and an atomic completion marker.
8. Use independently verifiable chunked LZ4, never one unbounded stream.
9. Write payload and checksums first, flush, then publish the completion marker;
   boot must reject absent/incomplete/mismatched images.
10. Hibernation contains credentials, keys and private process memory. It needs
    confidentiality and authenticity with keys unavailable to an offline disk
    attacker; checksum alone is insufficient.
11. Validate header, build/RAM/topology constraints, all chunk bounds/checksums,
    excluded ranges and destination overlap before restoring any page.
12. Disk-swap extents and hibernation-image extents must be separately reserved,
    pinned, versioned and made mutually exclusive during image creation.
13. Future runtime disk swap must have lower selection priority than all ZRAM
    pools; SWAP-1 defines no disk device identity.
14. Reserve worst-case on-disk logical capacity up to usable RAM plus header,
    chunk tables, checksums and alignment. Fitting must never depend on LZ4.
15. Actual images will often be smaller after LZ4, but no fixed 1-2 GiB claim is
    safe for every 4 GiB memory image.
16. Persist distinct states for normal active swap, image-writing, valid
    resumable image, and consumed/invalidated image.

Recommendation: run a separate **HIBERNATE-1 audit/design phase** before any
S3/S4, disk-swap, swapfile, or resume implementation.
