# Native std and Hayro

The published Hayro 0.8.0 renderer builds and renders real PDF pages with the
native SunlightOS target and a pinned overlay on Rust's actual `std` sources.
`hayro-native` enables it in `sunlight-docserv`; no renderer dependency is
patched, and the ELF has the native personality. Document Viewer also builds
against native std and uses its capability IPC client to call this service.
The existing Helios/musl viewer can call the same native service.
The kernel retains its freestanding target and does not use std.

## Native std platform

`tools/native-std/prepare.py` checks compiler revision `8954863c8` (2026-06-05),
stages Rust sources and host libraries under `target/native-std/sysroot`, and
applies `tools/native-std/overlay`. It never edits the installed rustup sysroot.
The wrapper supplies that sysroot to Cargo's real `build-std`. Source selection
covers PAL, System allocation, args/environment, clocks, secure entropy,
stdio, files/paths, TLS, threads and synchronization; Rust's collections,
memory I/O, channels and futex lock algorithms remain the upstream code.
Unknown-platform no-thread synchronization is not selected.
Native paths use slash separators and root-based absolute paths, including
absolute-path replacement in `PathBuf::join`; the platform does not opt into
the wider Unix libc implementation.
std owns its initial environment snapshot. `set_var`/`remove_var` update that
snapshot; they do not update the kernel EnvMap inherited by native spawn.

System and the default allocator share the bounded, reclaiming native heap.
The renderer and native viewer each opt into 128 MiB; other applications retain
their existing heap budgets. Native hash seeds use the conditioned kernel entropy directly without
allocation or IPC under runtime locks and fail closed when unavailable.
Clocks retain the actual realtime and monotonic precision of the native ABI.
The runtime reads argc/argv/envp from the common exec stack (legacy service
spawns repurpose the entry registers), initializes TLS and environment before
calling Rust's generated main entry, and reports panic/abort through native
stderr and group exit.

Native TLS has 255 usable process-wide keys and per-thread pointer slots in the TCB
page, following the self pointer and errno. Destructors run before completion.
Compare/wait/wake uses native syscall 155, aligned atomic word reads, VM identity
and a scheduler-serialized compare/enqueue. Absolute monotonic deadlines wake
through the BSP timekeeper; unmap cancels wait registrations. Rust supplies the
mutex, rwlock, once, condvar and parking algorithms above that primitive.
Sleep retries spurious wakes until the full monotonic interval has elapsed.

Native std threads use syscall 158 to opt into shared descriptors and cwd.
Existing syscall 22 retains its previous descriptor behavior. Joining waits
for syscall 156 to confirm that the task is reaped and off every CPU, then
releases stack/TLS mappings. Detached handles are bounded and swept on the
next spawn/join/detach; complete VM teardown also releases them. A finished
detached thread can retain its mappings until that sweep. Group exit (157)
terminates all tasks sharing the caller's VM. No new syscall grants authority
outside the current VM or changes capability IPC semantics.

Syscall entry now establishes System V stack alignment before calling Rust,
while preserving the original saved register frame. The native std worker gate
exposed a misaligned FXSAVE local in the old syscall entry path.

The anonymous-page swap candidate list reserves the exact known mapping size
instead of doubling its capacity. This reduces the kernel heap peak while
mapping the renderer and viewer heaps alongside the desktop. Mapping rollback,
ownership and swap policy are unchanged.
The fixed kernel heap is 16 MiB (previously 8 MiB) to hold desktop service state,
anonymous-page tracking, a replacement tracking buffer and mmap rollback
journals together while both app heaps and rendering workers are live. The
8 MiB configuration still failed worker-stack mapping despite available RAM.

This is a supported renderer/runtime subset, not a complete implementation of
every std OS API. Networking, process Command/fork, directory iteration,
filesystem links/locks/timestamps and several file mutation APIs explicitly
remain unsupported. General compiled TLS/AVX, unwinding and immediate detached
mapping recovery remain separate work. Native panic/OOM terminates the service
process; catch_unwind does not recover an aborting native renderer. The service
unit requests restart on failure; an induced supervisor restart is not part of
these gates. Clients receive transport failure and their handles must be
reopened after restart. The 128 MiB heap bounds memory, so very complex PDFs or
large renders can exhaust it even below the protocol's 16-megapixel limit.
Text extraction and PS/EPS/XPS/CBZ/EPUB remain outside the Hayro PDF backend.

## Implemented foundation

`tools/targets/x86_64-sunlightos.json` is an experimental, separate userspace
target with the System V hard-float ABI and SSE2. The kernel keeps its existing
soft-float target. Userspace binaries use the native syscall ABI and are not
stamped as Linux/Helios programs. Native libc exports and IPC account/logging
paths recognize both the existing `none` target and the new `sunlightos` target.
The target has no precompiled standard library; the build wrapper compiles the
pinned std sources with the overlay.

Each scheduled task now owns a 16-byte-aligned, 512-byte FXSAVE state. Timer
rescheduling saves the outgoing state and restores the next task, including
idle-to-task dispatch and initial process entry. New processes start with
reset x87/MXCSR state and zeroed XMM registers. Native and Linux threads inherit
the creator's live FP state. Reaped slots clear their saved state. The kernel
continues to avoid FP/SIMD instructions; OSXSAVE remains off because AVX/YMM
preservation is not implemented.

The native FP probe runs eight borrower threads plus the address-space owner.
Each keeps different x87, XMM0/XMM15, x87 control-word and MXCSR values across
64 busy intervals and native yield syscalls. Aligned SIMD stores also verify
the corrected native thread trampoline stack alignment. The isolated QEMU gate runs with two
and four CPUs. It neither attaches a user's disk nor replaces the normal ISO.
The same native program parses the bundled four-page PDF and checks white/red
pixels from the alloc-only Vello vector fixture. It does not rasterize a PDF
page. This is a preemption/isolation test, not a forced-affinity migration test.

## Build and guest gates

```sh
# Full unchanged Hayro graph against native std.
bash tools/check_native_std.sh

# Native Hayro service and native std UI/client executable.
bash tools/build_docserv.sh

# Package the native renderer for the desktop.
SUNLIGHT_DOCUMENT_VIEWER=1 bash tools/build.sh

# std, actual PDF raster, lifecycle and capability IPC gate.
DOCSERV_FEATURES=native-hayro-smoke bash tools/build_docserv.sh
bash tools/test_docserv.sh
DOCSERV_TEST_CPUS=4 python3 tools/docserv_boot_gate.py

# Native std viewer worker -> native service -> canvas and Files thumbnail.
bash tools/build_docserv.sh
DOCVIEW_BACKEND=native DOCVIEW_FEATURES=native-ui-smoke bash tools/build_docview.sh
DOCVIEW_BACKEND=native DOCSERV_TEST_UI=1 bash tools/test_docserv.sh

# Optional verification of the existing Helios viewer against native Hayro.
DOCVIEW_FEATURES=native-ui-smoke bash tools/build_docview.sh
DOCSERV_TEST_UI=1 bash tools/test_docserv.sh

# Restore ordinary binaries after smoke builds.
bash tools/build_docserv.sh

# Original FP foundation and boot regression.
bash tools/test_native_std_foundation.sh
bash tools/test.sh

# Native desktop Run -> sun-exec -> viewer -> Open Document dialog (UEFI).
bash tools/build_docserv.sh
DOCVIEW_BACKEND=native bash tools/test_docview_launch.sh
```

Each document gate boots its own VM without attaching a user's disk. Set
`DOCSERV_TEST_LOG` to preserve a separate serial log and `DOCSERV_SKIP_BOOT=1`
to build the isolated image before running the Python gate independently.
The UI gate captures `target/docview-native.ppm`. MuPDF/Ghostscript rollback is
still available with `DOCSERV_BACKEND=mupdf-legacy` and its musl C toolchain.

## Validation

Native service gates pass with two and four CPUs: collections, clocks, mutex,
rwlock, once, condvar and timeout, TLS isolation/destructors, repeated thread
join, shared File offsets, inherited/mutable environment and absolute/relative
paths. Separate child processes verify OOM and worker panic exit 101 and group
exit 43. Five six-page PDF rendering cycles recover their heap after warmup;
a four-page text/vector/image fixture passes dimensions and colored-pixel
checks. Capability IPC open/count/render/thumbnail/close and stale handles pass.
Logs: `target/native-hayro-runtime-{2,4}cpu.log`.

The Helios viewer passes six-page forward/back navigation, zoom, resize, cache
recovery, Files thumbnail and clean close against the native service.
Evidence: `target/native-hayro-helios-viewer.log` and
`target/docview-hayro-helios.png`. The same window gate also passes with the
fully native std viewer on four CPUs, including its channels and rendering
worker. Evidence: `target/native-hayro-native-viewer.log` and
`target/docview-hayro-native.png`.

The production native viewer passes the 12-CPU UEFI desktop Run/sun-exec launch
and Open Document dialog gate. This verifies chooser creation and directory
listing, separately from the fixture-driven render/navigation gate; it does
not claim a file selection click through the chooser. Evidence:
`target/native-docview-launch-final.log` and
`target/docview-launch-fix/native-viewer-open-dialog.png`.

The 14 host service/viewer tests pass. The 81 serial libc library tests pass
with two existing secret-store tests excluded: those execute native syscall 35
on the GNU host and hang there. Host checks alone do not establish guest behavior.

The original x87/XMM/MXCSR isolation and alloc-only parser/vector gate passes
with two and four CPUs. The ordinary Phase 3.0 boot regression also passes.
Logs: `target/native-std-foundation-final.log` and
`target/native-std-phase3-final.log`.

The normal `target/sunlightos.iso` is rebuilt with `native_document_viewer` and
the production document-service supervisor unit. Renderer/viewer/Files smoke
features and login injection are disabled. An isolated four-CPU boot of that
exact ISO reaches Foundation Complete and registers `docserv` with the 16 MiB
kernel heap: `target/native-document-production-boot.log`.
