# Boot image architecture and memory footprint audit

The current image cannot boot at 512 MiB: Limine's staged ELF and contiguous
destination alone require **568.21 MiB**. The lowest passing configuration
tested remains 768 MiB. The best first prototype is a smaller ELF with an
indexed, compressed Limine module read in bounded chunks. The model puts
that design at **460.84 MiB through the sampled pre-scheduler checkpoint**.
No repackaged image was built or booted; 512 MiB remains unqualified.

Evidence: [low-memory qualification](low-memory-boot-qualification.md),
[qualification results](low-memory-boot-qualification-results.json),
[PMM accounting](pmm-physical-accounting.md), and the
[machine-readable audit](boot-image-architecture-audit-results.json).
The measurements use the hash-matched MEM-Q1 qualified artifact and existing
probe image, not a fresh source build.

## Exact ELF and ISO sizes

The qualified ISO is 295,868,416 bytes; its executable is 291,733,920 bytes.
The ISO directory contains the kernel, Limine configuration, BIOS and UEFI
boot files (including the 2,949,120-byte UEFI boot image), and no standalone
root filesystem or ordinary app pack. ISO and firmware boot-image sizes
are media sizes; they do not represent resident kernel RAM. The root seed
is a compiled RAMFS entry table with pointers into immutable ELF bytes.

| ELF section | File bytes | Allocated bytes | PT_LOAD |
|---|---:|---:|---|
| Null | 0 | 0 | — |
| .text | 1,142,885 | 1,142,885 | RX |
| .rodata | 272,515,080 | 272,515,080 | R |
| .data | 18,044,672 | 18,044,672 | RW |
| .got | 32 | 32 | RW |
| .telemetry | 16,232 | 16,232 | RW |
| .bss | 0 | 12,340,672 | RW |
| .shstrtab | 52 | 0 | — |

| Program header | File offset | File bytes | Memory bytes | Page-rounded memory |
|---|---:|---:|---:|---:|
| LOAD RX | 4,096 | 1,142,885 | 1,142,885 | 1,146,880 |
| LOAD R | 1,150,976 | 272,515,080 | 272,515,080 | 272,519,168 |
| LOAD RW | 273,670,144 | 18,063,208 | 30,404,032 | 30,404,608 |
| Total | | 291,721,173 | 304,061,997 | 304,070,656 |

The 12,747 bytes outside the segments comprise 64 bytes of ELF header,
168 bytes of program headers, 512 bytes of section headers, 52 bytes of
section names, and 11,951 bytes of padding. The destination spans
`0xffffffff80000000..0xffffffff921fbdc0` before rounding. The full file
stages into 291,737,600 bytes, including its final partial page.

## What occupies the image

- **RAMFS:** 613 entries (472 files and 141 directories), 29,424 bytes of
  entry records, and 154,184,731 logical file bytes. Deduplicating file
  offsets gives **88,533,007 physical payload bytes** in the kernel
  ELF. User and root samples sometimes point at the same bytes. The
  logical totals include 84,109,942 bytes of images (including SIMG),
  64,137,702 of audio, 2,754,744 for Btop, 61,505 of foreign executables
  by filename, 9,849 of icons, and 3,110,989 of other files. Each of the
  three largest game music tracks is 5,760,044 bytes. The JSON retains
  file paths, offsets, sizes and namespace metadata.
- **Direct executable payloads:** Full-file comparison against build
  inputs identifies 181,397,304 unique bytes. The largest are VFS server
  (88,797,744), Yazi (26,942,728), Dictionary (14,058,032), Vortex shell
  (5,817,168), Rappid Rabbit (5,502,192), and TTY server (3,314,704).
  The nested VFS ELF has 88,588,850 bytes of read-only data and contains
  an exact match for every distinct kernel RAMFS payload range, covering
  88,533,007 bytes of the outer seed. These are separately stored bytes;
  the nested copy is already counted within the executable total. The
  combined direct and RAMFS range union accounts for 269,930,311 bytes
  of .rodata; 2,584,769 remain outside that union. The JSON distinguishes
  confirmed input matches from ELF-header-derived extents, which do not
  establish the complete original file length or prove a launch path.
- **Fonts, icons and tables:** Exact matches include a 1,520-byte kernel
  console/panic font, generated atlas data of 8,697, 19,161 and 7,749 bytes,
  and four 4,114-byte TUI icons. These exclude any opaque assets inside
  nested applications. The large writable statics include the
  16,777,216-byte initialized frame-owner array, 524,288-byte PMM bitmap,
  4,128,768 bytes of AP stacks and 2,097,152 bytes of idle stacks (sizes
  derived from source declarations). Other persistent data includes
  scheduler, IPC, address-space ledger and telemetry state. The
  production ELF is stripped; there is no claim of complete
  symbol-by-symbol attribution within these sections.

## Loader and memory lifetimes

At Limine revision `5ed5674868183107ea4f0a42b34799af6fe8efee`,
`common/protos/limine.c` loads the executable with `freadall` as
bootloader reclaimable. `common/fs/file.s2.c` requests a page-aligned full
file allocation for the optical path. `common/lib/elf.c::elf64_load`
then requests one contiguous span from the lowest to highest loadable
virtual address, aligned to the largest segment alignment (4 KiB here).
It copies the segments, using the staged ELF for headers and relocations.
Separating payloads into modules reduces both ELF allocations; modules are
loaded subsequently, occupy their own page-rounded executable/module
ranges, and do not make the executable staging vanish. The current config
has no modules. A module provider requires an explicit module request,
uses its reported addresses and sizes, and must account for module pages
separately from the translated kernel executable span.

The two ELF allocations overlap for at least the destination allocation
and copy. The current loader leaves the file staged at handoff, even
without an executable file request. MEM-Q1 recorded the 512 MiB OOM at
that destination allocation. In a new 768 MiB guarded run, the loading
message appeared at 0.708 seconds after QEMU launch, first kernel serial
at 15.259 seconds and Phase 3 at 15.598 seconds. This measures the loader
interval visible on serial (14.551 seconds to kernel serial); it does
not isolate time inside the allocator or copying loop.

In a separate 768 MiB rerun using the existing probe ISO, first kernel
serial appeared at 19.485 seconds, PMM initial at 19.506, post-heap at
19.557, pre-scheduler at 19.783, and Phase 3 at 19.876. After observing
Phase 3, the audit stopped QEMU via QMP and read the high reclaimable
range. The full 291,733,920-byte probe ELF matched at physical
`0x1e8db000`, immediately after the loaded image. This confirms intact
staging through at least the first **0.391 seconds of kernel serial
progress** in that run, in addition to its loader lifetime. The times
are host observations sampled at roughly 10 ms intervals; do not subtract
timestamps across guests. No staging release was observed.

| Class | Attributable bytes | Lifetime and interpretation |
|---|---:|---|
| Executable staging | 291,737,600 | Loader allocation, still intact after Phase 3; type 5 excluded from PMM |
| Other loader-reclaimable | 3,854,336 | Includes loader structures, mapping and stack costs; exact constituent retirement was not measured |
| Loaded kernel | 304,070,656 | Pinned in the physical accounting, including static RAMFS; mount creates no second payload there |
| Heap backing | 16,777,216 | 4,096 PMM frames remain backed after smoke drop; allocator use was 0 B then and 181,896 B pre-scheduler |
| Additional net PMM use after heap checkpoint | 99,061,760 | Net used-frame increase to pre-scheduler; includes startup work and is not an allocation-class peak |
| Framebuffer | 4,096,000 | Separate map range outside managed RAM and outside the configured RAM budget in these captures |

Initial free was 50,075 frames, post-heap 45,971, pre-scheduler 21,786.
No ZRAM store backing existed at those checkpoints. Later policy setup
is not proof of populated stores. The PMM probe previously verified
return of 64 ordinary frames and of heap allocator use after the smoke
test, not return of loader memory. No per-object boot allocation trace
or post-service high-water mark exists. The PMM does not currently admit
type 5, so even logically dead loader bytes remain unavailable. Limine's
protocol places response objects, boot stacks and page tables there.
Safe reclamation requires copying or retiring every retained response,
checking active page tables, HHDM mappings, descriptor tables and BSP/AP
stacks, and ensuring all APs have left any loader-owned handoff state.
Freeing type 5 wholesale at kernel entry is unsound. Modules use a
different map type and require separate ownership and release rules.

## Packaging alternatives

All rows below are estimates except the baseline observations. The broad
candidate extracts 176,455,834 verified unique bytes while retaining the
provisional bootstrap allowlist, including the current VFS server blob,
configuration, command files and app manifests. This measures extractable
content, not a validated dependency closure. The narrow selection under
`/Applications` and `/usr/share` alone moved only 27,125,291 bytes after
shared ranges were held back; even deferring all of it leaves the two
ELF allocations above 512 MiB. Host Zstd level 3 measured 101,586,295 bytes
for one stream or 107,477,400 bytes with independent streams of up to
1 MiB, excluding package index and namespace metadata.

| Alternative | Loader checkpoint MiB | Pre-scheduler proxy MiB | Headroom at 512 MiB |
|---|---:|---:|---:|
| Current | 572.39 | 682.90 | −170.90 |
| Smaller ELF + raw borrowed module | 404.12 | 518.62 | −6.62 |
| Smaller ELF + compressed module + bounded reads | 338.33 | 460.84 | 51.16 |
| Compressed module with eager expansion | 338.33 | 621.12 | −109.12 |
| Deferred from runtime boot filesystem | 235.83 | 358.34 | 153.66 |
| Embedded bootstrap + ordinary compressed module | 338.33 | 460.84 | 51.16 |
| Compressed pack embedded in ELF | 440.83 | 563.34 | −51.34 |

The baseline loader column is the sum of observed image, type 5 and
536,576 bytes of other excluded guest RAM; the measured occupied-or-excluded
pre-scheduler checkpoint is 716,070,912 bytes. In the model, subtract the
selected union from both file and image before rounding, then charge
retained staging, the rounded destination and any module, observed loader
overhead and map exclusions. Charge another 115,871,744 bytes of net
startup PMM use, preserving the 16 MiB heap. The bounded cases budget
an additional 4 MiB for decoder/workspace and metadata and 8 MiB for
cache; these are design budgets, not measurements. The raw case budgets
4 MiB for metadata. The eager case holds raw output and compressed input
concurrently. The compressed-inside-ELF option charges the pack twice at
handoff. The two bounded module rows share a numerical model; the explicit
bootstrap split adds a dependency and failure-policy boundary. The table
does not capture every transient peak, pack growth, fragmentation,
device/topology variance, or service and app allocations after scheduler
handoff. A 51 MiB modeled margin is a reason to prototype, not a guarantee.

- **Raw borrowed module:** Simple random access; no full-output copy.
  Requires immutable page ownership and validated index ranges. Every
  launch or open reference must retain valid backing. Avoiding the
  raw model's small deficit would require additional savings, such as
  verified staging release, before treating it as a 512 MiB candidate.
- **Compressed module:** Store opaque compressed bytes as standard Limine
  modules; decode within the OS. Do not depend on deprecated automatic
  compressed-module handling or change Limine's ELF format. Use indexed
  reads with hard limits and existing authorization, VM permissions and
  error semantics. A whole-app decode into a temporary buffer would
  invalidate the model unless independently budgeted. Validate random
  reads, executable loading, file mapping, and cache and reference cleanup.
- **Deferred filesystem:** Can avoid the resident pack, but Limine's
  ability to read the boot ISO does not provide a kernel runtime reader.
  The qualified guest has only the optical boot image; the kernel's
  optional FAT mounts depend on its own device and volume selection.
  Review signed/trusted volume identity, runtime driver and filesystem
  support, missing-media policy, dependency cycles with deviced/init/VFS,
  and preserved paths, metadata and capability checks before choosing
  this option. The estimate assumes the reader can reach the pack before
  the first dependent service, with bounded memory. Swap-out and
  fault-driven swap-in must remain VFS independent.

## Recommended implementation and qualification

1. Trace the actual dependency closure from kernel entry through init,
   device discovery, namespace publication, swap policy and login. Include
   startup assets, bundled interpreters and late exec consumers. Keep
   bootstrap inputs available before VFS and do not move mount or trust
   authority. Verify every path, mode, owner, capability and ordinary
   service/app against the existing image.
2. Build an opt-in image with the existing Limine loader and one bounded
   compressed module. Keep the production layout as a control. Use
   page-backed workspace sized independently of the 16 MiB kernel heap;
   validate index and payload integrity before use. Track module residency,
   scratch, cache, process copying and any loader pages admitted later as
   distinct physical classes without changing PMM accounting or reclaim
   policy. Measure fragmentation and the largest contiguous loader request.
3. Treat VFS RAMFS duplication as a separate opportunity after owner
   review. Replace duplicate backing only with an explicit shared immutable
   provider, preserving both namespace contracts and retirement rules.
   Do not promote VFS-L1C-3 or VFS-D1 on this evidence. Preserve the
   [integration prerequisites](../filesystems/vfs-l1c-integration-readiness.md),
   including trusted registration reuse, transaction/resource closure,
   interactive Btop and Qalculate, and independent SWAP-1 qualification.
   Loader page reclamation should also be an independent change with
   verified retirement points and exact frame admission checks.
4. Require fresh 512 MiB BIOS and UEFI target boots and larger-memory
   controls with read-only media. Capture loader allocation high water,
   PMM/map reconciliation, heap and module/cache peaks, Phase 3 and
   namespace readiness, scheduler and session readiness, desktop
   interaction, ordinary app launches, and service/reclaim regressions.
   Exercise missing/corrupt pack handling and resource lifetime through
   launch, exit and reload. Report boot, desktop and sustained workload
   gates separately. Current evidence is BIOS guest execution on an
   unchanged artifact; no 512 MiB candidate, UEFI candidate, or physical
   machine was tested in this audit.
