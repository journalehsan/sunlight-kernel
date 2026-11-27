# Adaptive TLS service heap

The native TLS service now uses the same libc extent allocator as the
document and TTY services. Its feature selects an 8 MiB aggregate cap.
The first allocation normally maps 256 KiB; later allocations grow the
heap in 256 KiB extents or larger page-rounded extents when required.
The allocator retains the first extent and unmaps empty growth extents.
It uses the existing synchronized free lists and ordinary native anonymous
mappings. The TLS IPC, certificate verification, crypto provider, and four
session slots are unchanged.

The initial setting was qualified against 4,002 bytes of ready payload
(5,096 bytes including allocator overhead) and a 5,168-byte startup
backing high water. The service starts with zero heap capacity before its
first allocation. Ordinary sampled requests fit within the 256 KiB
extent; the measured idle capacity reduction is 7.75 MiB (96.875%).
There is still fragmentation risk: multiple extents cannot satisfy
every allocation pattern that fits in a single contiguous 8 MiB arena.

The optional `heap-probe` samples request duration and post-request
payload, backing, session count, and allocator failures. It also makes a
fallible reservation at the budget boundary, verifies rejection, then
reserves 1 MiB and drops it. In each primary guest the probe grew mapped
capacity to 1,314,816 bytes and returned to 262,144 bytes. This synthetic
allocation dominates the running high-water counter; the workload results
therefore report the largest **reply-boundary** samples, which exclude
temporary allocations already freed by the time a reply is sent.
The rejection probe covers the allocator's fallible interface and later
service availability. Rustls and other infallible allocation paths can
still abort on exhaustion; arbitrary in-flight OOM recovery is unqualified.

The BIOS 512 MiB comparison with adaptive TTY in both guests had 17,264
free frames at document idle with eager TLS and 19,256 with adaptive TLS:
1,992 frames more (7.78125 MiB). The user-private PMM class fell by 1,986
frames. These are whole-guest observations against a prior image, so the
differences also reflect normal image and page-table changes. The
nominal TLS heap backing reduction is 1,984 pages. The [results JSON](tls-service-adaptive-heap-results.json)
records other guest checkpoints and per-request measurements.

To reproduce, build TLS with `sunlightos,heap-probe`, default features
disabled, and the userspace TLS flags from `tools/build.sh`. Build TTY
with its default adaptive profile, build docserv with
`DOCSERV_FEATURES=native-heap-probe bash tools/build_docserv.sh`, and
run `bash tools/build_boot_resource_prototype.sh`. Use
`tools/network_tls_gate.py --compressed-boot --ram 512 --cpus 2 --tcg`
with that ISO; use `--uefi` for UEFI and repeat for 768 and 1024 MiB BIOS.
Ensure the prebuilt service ELFs are current: the kernel build can reuse
existing release artifacts. Raw serial logs, command timings, and packet
captures are under `target/tls-heap/`.

The full fixture passed on 512 MiB BIOS, 512 MiB UEFI, and 768 MiB BIOS.
Each exercised the TTY foreground command path, issuer and hostname
rejection, verified HTTPS and SOCKS, 132 redirected TLS connections,
and eight 192 KiB transfers. The largest sampled reply held 57,879 bytes
of payload and 59,776 bytes of backing. Median connect request time was
50 ms in all three guests; maxima were 190, 130, and 230 ms respectively.
Each recorded 266 close replies, with zero sessions after the last close.
Both 1 GiB BIOS attempts reached TLS readiness and passed the document
probe, but timed out at bundle discovery before the fixture ran.

A separate 768 MiB run reached two live TLS sessions with 14,425 bytes
of payload and 16,736 bytes of backing, and returned to zero sessions.
The second fetch exited successfully; the first tab's exit status was
not observed, so the end-to-end overlap gate did not pass. Exact network
resource ownership after close remains unqualified. Neither bounded
workloads nor boot-time PMM measurements establish sustained 512 MiB
operation.
