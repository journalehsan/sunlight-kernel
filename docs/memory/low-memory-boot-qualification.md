# MEM-Q1: Low-memory boot qualification

**768 MiB is the lowest passing configuration tested.** The ordinary image
still cannot enter the kernel at 512 MiB. Limine panics while allocating the
contiguous ELF destination, after staging the executable file and before
copying its load segments. The direct run has an empty kernel serial log;
the guarded run records `High memory allocator: Out of memory` with
`elf64_load → ext_mem_alloc_type_aligned` in the trace.

The prior PMM qualification ISO reproduced the 512 MiB stop and passed at
768 MiB, 1 GiB and 2 GiB in fresh guests. A fresh pre-change production ISO
showed the same result. All tests used SeaBIOS, QEMU 11.1.3 TCG, two vCPUs,
and no network or writable guest disks. The direct runs use ordinary
`limine.conf`; the guarded runs use the same ELF and a bounded serial
capture with Limine `serial: yes` and `verbose: yes`. Both use the hybrid
ISO path. The configuration loads the executable from the boot volume,
with KASLR disabled and no external modules.

| RAM | Direct, repaired image | Guarded, repaired image |
| --- | --- | --- |
| 512 MiB | No kernel serial within 100 s | Limine OOM in `elf64_load`; no kernel entry |
| 768 MiB | Kernel, ordinary boot and service gates pass | Kernel, ordinary boot and service gates pass |
| 1 GiB | Kernel, ordinary boot and service gates pass | Kernel, ordinary boot and service gates pass |
| 2 GiB | Kernel, ordinary boot and service gates pass | Kernel, ordinary boot and service gates pass |

Ordinary boot means the existing `[SunlightOS] Phase 3.0 OK` marker.
The service check also observed init's VFS namespace prerequisite, swap
policy configuration, and the session foundation and configuration
readiness markers. The latter cover specific services; desktop login and
interaction, network or server workload readiness, and sustained service
operation were not qualified.

## Loader footprint and repair

In the local Limine checkout at `5ed5674868183107ea4f0a42b34799af6fe8efee`,
`limine_load` calls `freadall` with the bootloader-reclaimable map type,
then calls `elf64_load`. The latter takes a separate allocation for the
span of all loadable segments while the staging buffer is still live.
The table counts both allocations rounded to pages. It is a lower bound
on bootloader memory demand; other allocations, firmware holes and the
contiguous range requirement also matter.

| ELF | File bytes | Page-rounded staging | Page-rounded destination | Simultaneous lower bound |
| --- | ---: | ---: | ---: | ---: |
| Before repair | 304,074,776 | 304,078,848 B | 304,070,656 B | 608,149,504 B (579.98 MiB) |
| After repair | 291,733,920 | 291,737,600 B | 304,070,656 B | 595,808,256 B (568.21 MiB) |

The orphan `.got` landed after `.bss` in the writable load segment.
That forced the ELF to carry file padding across the zero-filled region.
The linker script now explicitly places the GOT before `.bss` and Cargo
tracks the script as a build input. The ELF shrank by 12,340,856 bytes
without changing the page-rounded destination or removing resources.
Static RAMFS payload accounts for 154,184,731 bytes inside the pinned
kernel image. No separate RAMFS copy is allocated at mount.

Even after the repair, staging and destination alone exceed 512 MiB.
The 1 GiB guest loads the same image and has room for both. The true
minimum between 512 and 768 MiB was not swept. Supporting 512 MiB with
this payload needs a boot path that avoids or substantially reduces the
overlapping staging allocation; changing heap or reclaim policy cannot
resolve a failure that precedes kernel entry.

## Kernel memory and startup

Temporary serial probes measured the following checkpoints in a separate
release image. The probe image reserved the same number of image frames
and had the same page-rounded ELF file size as the repaired production
image. The probe was removed after capture.

| Guest RAM | Initial managed frames | Initial free frames | Free frames after heap smoke | Free frames before scheduler |
| --- | ---: | ---: | ---: | ---: |
| 768 MiB | 124,311 | 50,075 | 45,971 | 21,786 |
| 1 GiB | 189,847 | 115,611 | 111,507 | 87,322 |
| 2 GiB | 451,991 | 377,755 | 373,651 | 349,466 |

In all three probes, initial free equals usable map pages and managed
equals initial free plus 74,236 pinned image frames. The map reports
295,591,936 bytes as bootloader reclaimable, including the staged ELF and
other bootloader allocations. This is distinct from the pinned image.
The existing PMM leaves those reclaimable pages outside its managed pool,
so their physical lifetime is not treated as available kernel capacity.
The 4,096,000-byte framebuffer is also outside the pool.

The heap has 4,096 physical backing frames. After the smoke allocation
was dropped, allocator use was zero of 16,777,216 bytes. At scheduler
handoff it was 181,896 bytes used at each tested size. Both checkpoints
had zero ZRAM pools, stored pages and backing bytes. In the ordinary
direct guests, the swap service subsequently configured one pool with
physical budgets of 124,308, 189,844 and 451,988 KiB, respectively.
These are limits, not resident backing allocations. No heap peak, steady
state allocator use, or natural ZRAM reclaim is inferred from these samples.

The production release build and 220 kernel host library tests passed.
The repair leaves the heap, PMM accounting, reclaim watermarks, ZRAM
policy, bounded telemetry, VFS migration prerequisites and ordinary
service set intact. See the [JSON results](low-memory-boot-qualification-results.json)
for the maps, per-run gates, commands and artifact hashes. Prior evidence:
[PMM qualification](pmm-physical-accounting.md),
[PMM machine results](pmm-physical-accounting-results.json), and the
[peak pressure audit](peak-pressure-allocation-audit.md).
