# Adaptive document service heap

Native Hayro originally selected `dynamic-heap-128m` when it became
the default renderer in `f6975c42f`. Native std's System allocator and
the C allocation ABI share sunlight-libc's free list. The setting was
an arena capacity for the renderer and its document, raster, and IPC
buffers; it was not measured live demand. The allocator used to map
the entire arena on its first allocation. The previous compressed boot
matrix recorded an eager 32,768-page request and zero free PMM frames
in both 512 MiB guests. The mmap transaction rolled back and docserv exited.

The native renderer now selects `adaptive-document-heap`. This uses
existing native private anonymous mmap and munmap calls. Existing
allocators in other separately built binaries retain their policies.
The first extent targets 256 KiB; allocations reuse free blocks before
mapping another extent. Growth maps at least the request plus alignment
padding, rounded to a page, with a 256 KiB target for small requests.
If that target fails, the allocator retries the exact required extent.
The sum of mapped extents is bounded at the original 128 MiB. Each
extent has its own coalescing list, and the allocator returns fully free
growth extents to the kernel. The initial extent is retained for reuse.
Failed maps are not published. A failed fallible reallocation leaves
the original block intact.

The aggregate bound does not guarantee that all live allocation
patterns that fit the old single arena fit the new segmented arena.
Partially occupied extents can strand free bytes, and the allocator
does not move live blocks. Hayro and Rust still have infallible allocation
paths; this change does not make arbitrary rendering OOM recoverable.
The existing document, bitmap, and protocol limits are unchanged.
The reply serializer now sizes its buffer once and reserves it fallibly.
Hayro's bitmap copy and IPC buffer copies also reserve fallibly. Postcard
reads the length-delimited pixel bytes before allocating the destination,
eliminating repeated doubling during decode. Native Postcard bytes and
the JSON representation retain their prior layout.

The compressed seed still matches 613 entries and 154,184,731 bytes
against the audited seed. This change does not alter the shared
immutable module, ordinary packaging, 16 MiB kernel heap, PMM
accounting, ZRAM watermarks or VFS migration prerequisites.

## Native measurement

The optional `native-heap-probe` feature measures readiness, a
10-second idle interval, RPC transactions, and recovery. The probe
uses a client thread in the service process, so reported heap usage
includes the client, and reported latency covers the local RPC round
trip rather than the experience of a desktop application. The heap
capacity is mapped space; live bytes are requested payload and
backing high water includes allocator metadata and alignment.
Free PMM frames count the entire guest; the running minimum includes
transient allocations between serial checkpoints. All frame counts
refer to 4 KiB frames.

Reproduce with `DOCSERV_FEATURES=native-heap-probe bash tools/build_docserv.sh`,
`bash tools/build_boot_resource_prototype.sh`, and
`tools/qualify_boot_resource_prototype.py` with `--document-heap`.
Use fresh optical guests with two vCPUs and no network or disk.

| Guest | Free at ready | Free at idle | Minimum through workload | Free after workload | 3× render + open/count/close |
| --- | ---: | ---: | ---: | ---: | ---: |
| 512 MiB BIOS | 19,953 | 14,275 | 1,461 | 14,275 | 600 ms |
| 512 MiB UEFI | 19,991 | 13,231 | 417 | 13,231 | 680 ms |
| 768 MiB BIOS | 86,570 | 79,811 | 66,997 | 79,811 | 570 ms |
| 1 GiB BIOS | 151,576 | 145,347 | 132,533 | 145,347 | 650 ms |

All four guests passed the boot milestones and document workload on
QEMU 11.1.3. The service mapped 256 KiB at readiness and idle, with
714–722 bytes of live requests at readiness and about 2 KiB of live
allocator backing. Each guest opened a six-page PDF through native IPC,
rendered two pages at 1× (1,938,816 bytes each) and one at 3× (17,449,344
bytes), rejected closed handles, and generated a 128×128 thumbnail.
The 1× transactions took 80–790 ms in this run. The probes checked bitmap
geometry and length; the host renderer regressions cover image output.

The largest sampled heap capacity was 17,715,200 bytes with 17,468,155
live request bytes. Transient live backing peaked at 34,921,808 bytes.
After the workload the heap returned to 256 KiB of capacity and 1,062
live request bytes. The probe also grew a 17 MiB, 64 KiB aligned block,
freed it, and confirmed a rejected over-budget reserve preserved the
original buffer. The same service then handled the document RPCs.
Matching post-idle and recovery PMM counts and returned heap capacity
verify cleanup for this workload; they do not prove cleanup of all
resources for every renderer error or process exit path.

Earlier attempts are retained in the [results JSON](document-service-adaptive-heap-results.json).
An allocator-only build started docserv but exhausted both 512 MiB guests
during geometric reply growth. Sizing the reply removed that failure;
geometric client decoding then exhausted those guests. Both process
failures are separate from the final passing run. The final 512 MiB
guests remained responsive after the large render but touched low
physical headroom. These are bounded startup and workload passes.
Sustained 512 MiB operation, concurrent viewer workloads, maximum-size
documents, and arbitrary renderer OOM recovery remain unqualified.

The native service and viewer release builds, the ordinary kernel release
check with the native viewer, nine host allocator tests, and eleven host
document tests passed. The last host check also covers preserving the
existing JSON representation alongside the native Postcard optimization.
