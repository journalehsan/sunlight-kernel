# Physical Memory Accounting (Phase 1)

## Previous metric semantics

### Global used memory

Before Phase 1, Tasks Monitor and telemetry reported:

```text
total_ram_kb = PMM.total_frames * 4
used_ram_kb  = (PMM.total_frames - PMM.free_frames) * 4
```

So **used = managed − free**. That figure includes **every** non-free managed
frame: kernel image (including embedded INITRAMFS assets), heap, page tables,
user pages, SHM, ZRAM storage, driver buffers, and anything else allocated from
the PMM. It is exact for managed usable RAM and is **not** “sum of task RSS”.

### Per-task memory

Per-task `mem_pages` / UI “Mapped” is produced by walking the process page
tables and counting **present user-half pages** (`count_user_pages`). That is:

* present mapped physical pages in the user half of the address space
* includes executable and read-only pages
* includes SHM and other shared mappings **once per process mapping**
* does **not** include page-table frames themselves
* does **not** include kernel stacks
* is **not** standard Linux RSS with full sharing semantics

Summing per-task mapped pages therefore **over-counts** shared physical frames.

## Installed / usable / managed / reserved

| Term | Meaning |
|------|---------|
| **Installed** | Sum of conventional RAM ranges from the Limine memmap (usable + reclaimable + reserved RAM types). Framebuffer / pure MMIO is excluded. |
| **Usable / managed** | Frames tracked by the PMM bitmap (`TOTAL_FRAMES * 4096`). This is the denominator for the main used percentage. |
| **Free** | PMM free frame count × 4096. |
| **Reserved** | Installed − managed (firmware / non-usable), or explicit firmware-reserved sum when known. Not charged as task or cache memory. |

## Page classification

Each allocated managed frame has **one primary class** packed into the high
byte of `FRAME_OWNER` (low 24 bits remain owner PID). Classes:

```text
Free, ReservedFirmware, ReservedKernelImage, KernelCore, KernelHeap,
KernelStack, PageTable, UserPrivate, SharedMemory, RamFsFileData,
RamFsMetadata, FileSystemCache, GraphicsBuffer, DeviceDma,
CompressedMemory, OtherAccounted
```

Counters are O(1) on alloc / free / reclass. Snapshots never scan all frames.

## Conservation equation

```text
managed_bytes
  ≈ free_bytes
  + task_private_unique
  + shared_memory_unique
  + kernel_core + kernel_heap + kernel_stack
  + page_tables
  + ramfs_file_data (+ ramfs_metadata if measured)
  + filesystem_cache + other_reclaimable_cache
  + graphics + device_dma
  + zram_physical
  + other_accounted
  + unclassified
```

Tolerance: up to 16 pages of alignment residual (`CONSERVATION_TOLERANCE_BYTES`).
Any unexplained residual is **Unclassified**, never Cache or RAMFS.

## Task-private and shared memory

* **Tasks & services** (unique) = frames classed `UserPrivate`.
* **Shared memory** (unique) = frames classed `SharedMemory` once per object,
  regardless of mapping count.
* Per-task **Mapped** may include shared pages; row sums may exceed unique
  private + shared global.

## RAMFS

Static INITRAMFS file payloads are `include_bytes!` in the kernel image.
Phase 1:

* measures exact sum of static file payload bytes
* page-rounds and **carves** that size out of `ReservedKernelImage` for display
  under **RAMFS data** so conservation does not double-count
* **RAMFS metadata: unavailable** (lives on general kernel heap; not estimated)
* **retained boot image = 0** (measured): there is no second copy after mount

```text
RAMFS file contents consume physical RAM (embedded in the kernel image).
```

Dynamic `Vec` buffers for written RAMFS files also live on the kernel heap and
are not reclassified in Phase 1.

## Cache

Cache is **only** real ownership (`FileSystemCache` / other reclaimable classes).

```text
The difference between global used memory and summed task memory must not
automatically be labeled as cache.
```

Phase 1 typically reports Cache = 0 until a real page cache exists.

## Kernel and page tables

* Kernel image → `ReservedKernelImage` (minus RAMFS carve for display)
* Linked-list heap pages → `KernelHeap` (8 MiB at boot)
* Page-table frames → `PageTable`
* Kernel stacks currently allocate from the heap → under Kernel Heap (not a
  separate `KernelStack` charge yet)

## Graphics and devices

* Framebuffer / MMIO ranges are **not** counted as ordinary managed RAM.
* Driver DMA from PMM should use `DeviceDma` when wired; Phase 1 may leave
  many driver frames under `OtherAccounted` → visible as Other / Unclassified
  until sites are annotated.

## ZRAM

* **Physical**: compressed storage frames (`CompressedMemory` / allocator
  consumed bytes).
* **Logical**: stored page count × page size (telemetry only).
* Logical bytes are **never** added to physical used RAM.

## Unclassified

```text
unclassified = used_managed − sum(exact non-overlapping classes)
```

Checked subtraction; large residuals set `FLAG_LARGE_UNCLASSIFIED` and appear
in serial/CLI/UI.

## Snapshot consistency

`PhysicalMemoryAccountingSnapshotV1` is captured under the same telemetry
generation as process enumeration when the scheduler lock is held for scalar
capture. Class counters are atomic; generation increments per snapshot.

## Capability model

Aggregate counters are exposed read-only via the existing telemetry mapping
(`SYS_MAP_TELEMETRY`). No new privileged memory-inspection endpoint is added.
Physical addresses and raw page-table dumps are not exposed to ordinary apps.

## Tasks Monitor UI

* Header: used of usable, task count
* Breakdown lines: Tasks&svc, Shared, Kernel, PT, RAMFS, Cache, Gfx/dev,
  Other/Unclass, Free
* Table column **Mapped** = present user pages (not unique private)
* Existing process list and controls preserved

## CLI

```text
memoryctl accounting
memoryctl accounting --details
memoryctl accounting --tasks
memoryctl accounting --verify
```

## Controlled experiments / QEMU gate

```text
./tools/test.sh memory-accounting
```

Markers include SNAPSHOT, SHARED_UNIQUE, RAMFS_DELTA/RELEASE, PAGE_TABLES,
WALLPAPER/static measurement, BOOT_IMAGE_RESIDENCY, CONSERVATION,
UNCLASSIFIED_BOUNDED, RESOURCE_BASELINE, IDLE_CPU, UI_RENDER, FINAL.

Host unit tests live in `kernel/src/memory/accounting.rs` (`cargo test -p sunlight-kernel --lib` where host tests are enabled for that module; the accounting tests are `cfg(test)` and run on the host when the crate is tested with std helpers — primary gate is the ISO test).

## Known limitations (Phase 1)

* Large residual **Unclassified** is expected until more alloc sites set
  explicit classes (many early boot/device paths still use `OtherAccounted`).
* Kernel stacks not separately classed.
* RAMFS metadata not separated from heap.
* No filesystem page cache yet.
* Graphics SHM surfaces remain under SharedMemory (not double-counted as Gfx).
* Per-task private vs shared split in the row is not yet walked; only global
  unique private/shared classes are exact.
* Process generation is address-space generation low 32 bits.

## Performance

* Snapshot: O(number of classes)
* No full physical-frame scan on UI refresh
* Task mapped pages still use existing page-walk cadence (unchanged)
