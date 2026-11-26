# Compressed boot module runtime memory accounting

Both 512 MiB guests reached **zero free PMM frames before bundle discovery**.
The native document service's first heap allocation requested an eager
128 MiB anonymous mapping. It ran out of frames partway through, rolled
back, and exited. Session configuration subsequently discovered eligible
bundles with free frames available. The discovery marker reports a
historical minimum, so it cannot attribute the peak to discovery itself.

## Fresh boot matrix

One instrumented opt-in ISO, QEMU 11.1.3 TCG, two vCPUs, optical boot,
no NIC or disk. Values are free 4 KiB frames; the minimum is cumulative
from PMM initialization through bundle discovery.

| MiB / firmware | Entry | Pre-scheduler | Phase 3 | Namespace | Session ready | Bundles | Minimum | Document service |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| 512 BIOS | 47701 | 41030 | 36919 | 36916 | 22306 | 18270 | 0 | ENOMEM; exited |
| 512 UEFI | 46117 | 39454 | 35345 | 35342 | 21262 | 20103 | 0 | ENOMEM; exited |
| 768 BIOS | 113237 | 106566 | 102457 | 102454 | 53252 | 52088 | 52088 | Ready |
| 1024 BIOS | 178773 | 172102 | 167993 | 167990 | 118788 | 118244 | 118244 | Ready |

All four passed the 16 MiB heap, Phase 3, VFS inventory, 2 KiB shared
memory VFS read, namespace, swap-policy, session foundation/config
readiness and eligible-bundle gates. The 768 MiB and 1 GiB controls also
logged document-service readiness; PDF rendering and sustained request
latency were not tested. A preliminary 512 MiB BIOS run of the original
unmodified ISO timed out after the same document-service allocation
failure; the fresh instrumented run passed the gates. See the
[results](compressed-boot-runtime-accounting-results.json) for serial paths
and raw totals. The retained module independently matched all 613 entries
and 154,184,731 logical bytes against the audited seed.

## Physical ownership and lifetimes

The kernel image is 32,146 frames (131,670,016 B), pinned and included in
PMM managed totals. That includes the embedded executable catalog and
17,301,504 B of static frame owner and bitmap storage (16 MiB + 512 KiB).
The compressed seed is 21,092 frames (86,392,832 B) outside PMM; the
kernel and VFS service share those same frames. Their inventory and
mount state are separate; the kernel's share is not isolated from heap
usage, and VFS metadata is part of its process footprint. The mapping
adds page tables; reading uses bounded 4 KiB decode scratch without a
decoded cache. No separately classified RAMFS data or cache frames were
observed at the sampled checkpoints.

Limine excluded 122,888,192 B (BIOS) or 123,297,792 B (UEFI) as
bootloader-reclaimable. The previous prototype's QMP capture showed the
complete original staged ELF intact after Phase 3. For this instrumented
ELF the rounded file size is 119,336,960 B; this is an estimate of the
staging component, not a new physical scan. The remaining loader pages
are not individually attributed. The configured-RAM residual beyond
PMM managed RAM, module and loader ranges includes other firmware or
memory-map exclusions; MMIO and framebuffer apertures are not added
to the configured RAM budget. No loader or module frames were released.

The PMM equation at each checkpoint is free plus the owner classes in
the results file. The 16 MiB kernel heap stays backed by 4,096 frames;
its allocator uses about 236 KB pre-scheduler and 4.5–4.6 MB by session
discovery. The initial bootstrap address spaces (including init, VFS
and TTY) and other early process allocations account for 2,486 private
frames and 72 page-table frames in aggregate before scheduling. Phase 3
increases private ownership to 6,584 frames, including 4,098 tracked
anonymous candidates. The BIOS 512 MiB guest then grows to 24,945
private and 321 table frames at discovery. Shared pages and other
classified costs are in the JSON; a zero class means no frames were
charged there and does not rule out that purpose inside another owner.
Class totals are read under the PMM lock. Heap and reclaim diagnostics
are sampled immediately afterward under their own locks.

## What consumes the last frames

The native libc allocator maps the whole configured arena on its first
allocation. Native anonymous mmap eagerly allocates and zeros frames;
the existing demand-backed path is for Linux-personality mappings.
The renderer requested 32,768 pages. At the failed allocation, the
BIOS transaction had installed 21,588 and the UEFI transaction 20,546
pages, and both had zero free. Total private ownership at that moment
was 43,210 and 41,648 frames, respectively. Rollback recovered
21,630 and 20,586 free frames; the renderer's later teardown returned
1,746 user frames and 12 page-table frames in each guest. The controls
successfully retain the full 32,768-frame heap mapping while the
renderer stays up. This is residency created by the allocator policy;
the workload's actual heap high-water use was not measured.

Bundle discovery enumerates directories, opens each manifest, reads it
into a 4 KiB stack buffer, closes the descriptor, and uses stat to
check executable availability. The catalog retains parsed metadata.
Discovery does not load each bundle executable or decode the seed in
bulk. System service launch does copy loadable ELF segments from the
embedded executable catalog into process-private frames. That runtime
copy is separate from any executable bytes present in the filesystem;
the audit did not establish a safe shared mapping for those segments.

## ZRAM and remaining work

All guests configured one pool. Neither control activated reclaim.
Each 512 MiB guest activated it once: BIOS stored/reclaimed 50 pages
(114,974 encoded bytes, 50 backing frames) and UEFI 49 (112,575 encoded
bytes, 49 backing frames). No swap-in succeeded through discovery.
These were real kernel swap-outs at the production watermark; the
physical backing offsets the frame return at these samples, and the
bounded direct-reclaim transaction could not satisfy the arena.
Candidate storage and pool metadata are heap allocations, not separate
PMM charges. This test does not establish repeated or sustained reclaim.

The code change is limited to instrumentation on the opt-in build.
The ordinary release check passed. The best follow-up is to make the
native renderer's arena commit memory as needed while preserving its
128 MiB capacity. That needs tests for native fault handling, kernel
copies into user buffers, and real document rendering. Reducing its
configured capacity would trade boot success for an unmeasured workload
limit. Loader staging is another opportunity, once the staging extent
and the last users of loader responses, stacks and page tables have
been proven independently. No change to those lifetimes, reclaim policy,
service selection, shared seed, or VFS migration prerequisites was made.

**512 MiB passes the bounded boot gate, but is not a sustainable
configuration on this evidence.** The document service fails at startup
and the tests stop at discovery, before desktop or sustained workloads.
