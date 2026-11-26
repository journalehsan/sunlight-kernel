# PMM physical memory accounting qualification

The Limine-based image reservation and the initial PMM frame equation
passed at 1, 2, and 4 GiB in fresh two-vCPU QEMU TCG guests. The 512 MiB
guest did not reach PMM. This qualification follows the
[peak pressure audit](peak-pressure-allocation-audit.md); exact maps,
addresses, and artifact references are in the
[machine results](pmm-physical-accounting-results.json).

| Guest RAM | RAM identified by map (bytes) | Initial free frames | Occupied image frames | Managed frames | Ordinary boot |
| --- | ---: | ---: | ---: | ---: | --- |
| 512 MiB | — | — | — | — | BLOCKED before kernel entry |
| 1 GiB | 1,073,205,248 | 112,600 | 74,235 | 186,835 | PASS |
| 2 GiB | 2,146,947,072 | 374,744 | 74,235 | 448,979 | PASS |
| 4 GiB | 4,294,430,720 | 899,030 | 74,235 | 973,265 | PASS |

The exact counts come from a release image with temporary serial probes.
The ordinary release image uses 74,234 image frames and reports free/managed
MiB rounded down: 439/729, 1463/1753, and 3511/3801, respectively. Ordinary
boot here means reaching the existing serial gate, `[SunlightOS] Phase 3.0 OK`.
Desktop readiness and prolonged service operation were outside that gate.

Each probe reservation starts at Limine's executable physical base. The
three ELF load segments are covered by the translated virtual interval
`0xffffffff80000000..0xffffffff921fadc8`; rounding up the exclusive end
matches the executable map entry exactly in all three maps (304,066,560
bytes). There were no external modules or usable/image overlaps. Initial
free equals the map's usable pages, and initial managed equals free plus
pinned image pages. The embedded RAMFS is part of those pinned pages.
The 4 GiB guest includes a usable 1 GiB range above 4 GiB; it contributes
to the PMM total. This probe did not specifically allocate from that range.

The installed-memory estimate previously counted all Limine reserved
address space as installed RAM. Every captured map includes a 12 GiB
reserved aperture at `0xfd00000000`, far outside guest RAM. The correction
excludes ambiguous reserved entries from the installed estimate while
retaining the raw reserved/bad map total in the existing reserved field.
The reserved field is therefore not a component to subtract from installed
RAM. The corrected estimate is RAM explicitly identified by the map, not
a DIMM inventory; it is 536,576 bytes below configured RAM in these guests.
Bootloader-reclaimable ranges account for 307,929,088 bytes at 1 and 2 GiB
and 307,937,280 bytes at 4 GiB. PMM leaves them outside its managed
pool. The framebuffer (4,096,000 bytes) is also outside that pool. No
bootloader memory reclamation was added.

Before other boot allocations, the probe allocated 64 distinct 4 KiB
frames from usable ranges outside the image, checked the free count at
every allocation and return, and restored the exact baseline. The
temporary allocation class returned to zero. This proves ordinary frame
return under the sampled conditions; the prior audit separately records
user and page-table frame return at worker reap. All 220 existing kernel
host library tests passed.

Immediately after heap initialization and the temporary heap smoke
allocation's drop, the 16 MiB heap had 0 bytes allocated and 16,777,216
bytes free. Its 4,096 physical backing frames are separate from heap
allocator use. At that checkpoint ZRAM had no active pools, no heap
reservations for slot/free/backing vectors, and no payload or backing
frames. Static pool and policy objects occupy kernel image data and are
not an additional PMM charge. These measurements precede swap service
configuration.

The 512 MiB BIOS boot produced no kernel serial output in the bounded
window; a VGA capture showed Limine loading the executable. The alternate
512 MiB OVMF probe reached firmware's DVD handoff without kernel entry
within the same bound. No PMM, heap, or ZRAM measurements exist for that
size. The captures identify a pre-kernel boot limitation for this image;
they do not prove a particular loader allocation failure.

The 16 MiB heap, reclaim watermarks, segmented candidates, fixed-scratch
telemetry, ZRAM policy/accounting and VFS independence, and spawn paths
were preserved. Natural activation, eviction, and telemetry freshness at
the production watermark remain blocked in the prior bounded pressure
fixture. Natural repeated same-page reclaim remains **unqualified**.
