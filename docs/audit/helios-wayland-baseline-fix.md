# Helios Wayland baseline stabilization — November 24, 2026

Evidence: `target/helios-wayland-baseline-fix/run.CpbKUb/` (relative paths below
refer to that directory). The earlier infrastructure baseline and its
archived ELF/ISO files were retained.

## Artifact identity

The historical runner checked a hard-coded SHA-256 before it staged the
manifest or ELF. The historical expected and archived digest was
`577ac203166af6171f5ae2789a4a9da9d39ae40dc6d801188288703a8995ba9e`;
the retained rebuild was
`5c7b5b51a6281b96506f252fb97054334a7f01bc40a4f3db10ee7a2f2420b314`.
These reproduce the failed comparison. The historical override explicitly
selected the archived ELF; the old guest had ELF validation but no SHA-256
comparison of the installed file.

This is a build artifact mismatch, before any bundle installation or ISO
generation. Both ELFs are static x86-64 executables and their compiler
comments agree. Section dumps and symbol inventories in the evidence root
show different `.text`, `.rodata`, `.data` and DWARF bytes; stripping DWARF
does not reconcile them. For example, `sunlight_wayland_paint` is
`0x2106` bytes in the archived executable and `0x2025` in the rebuild.
The rebuilt Rust static archive exports the latter size before the C link.
The mismatch cannot be explained solely by a path, timestamp, build ID,
the Linux ELF header stamp, or container metadata. The original Rust build
inputs that produced the archived machine code were not preserved, so the
specific flag or revision responsible for that code generation delta
remains unknown. This is not a demonstration of nondeterminism with
identical inputs.

The canonical test build links the Canvas C fixture and Rust static library,
stamps the ELF, and emits the expected SHA-256. The SunApp runner builds in
a new client directory, consumes that pin, checks the source and staged
ELFs, and embeds the expected digest in the launcher. The RAMFS has the
source manifest embedded separately and takes its program bytes from the
staged ELF. In QEMU the launcher opens the installed program through VFS,
hashes its bytes, and rejects mismatches before spawn. It does not fall
back to the archived artifact.

Two fresh client output directories with the same dependency build produced
the same digest at link
(`4818cb14bbcbdf4cf161eeaec7bdc3b53b19c797e8117ef8db970d5c2aee2c5a`)
and after stamping
(`d54133b0d828fb0d97204dfa3057ab2376e959ea382456d9a8fbf076664b9506`).
The latter is also the staged digest and the digest reported by the guest.
This does not claim byte identity across independent toolchain or
dependency builds with different paths.

The source base is `089850a5e05adadc97101d50a855fe1c3fd3d178`, plus the
local test changes. The Rust build uses
`1.98.0-nightly (8954863c8 2026-06-05)`, LLVM 22.1.6, the
`x86_64-unknown-linux-musl` release profile with panic abort, LTO and one
codegen unit. The C link uses GCC 16.2.1, GNU ld 2.47, musl 1.2.6,
`-static -no-pie -O2 -pthread`, libwayland 1.26.0, libffi 3.4.8 and
wayland-protocols 1.50. Build logs retain the full commands and SHA-256
inventory. `embedded-kernel.elf` is extracted from each ISO; its digest
identifies the kernel containing the RAMFS rather than a separate archive.

## Registry and lifecycle

The fixture originally accepted a socket and then waited for a desktop
window before reading even the registry and sync requests. The retained
direct Canvas TCG replay logged `desktop unavailable` followed by a
registry dispatch error. Other retained TCG failures exhausted registry
dispatch before configure. KVM and the successful replays had time to
serve the registry and render. The revised fixture handles registry and
configure independently, deferring the desktop check until it actually
needs to create the SGP window. No evidence implies stale RAMFS,
fragmented registry events, or a need for longer client dispatch timeouts.
This identifies the blocking dependency; the older `errno=0` samples
did not have enough trace events to reconstruct every scheduling decision.

The first new wire boot reached configure and ack, but login preceded
registration of `sessiond`. Its first failure is **boot ordering**; it is
retained under `wire/tcg-initial/`. The gate now waits for the existing
session service marker before typing the login. A new Canvas SunApp control
still exhausted frame dispatch while waiting for the desktop after
registry completed; another missed the host SGP redraw deadline. These are
separate first-frame readiness/redraw results in `canvas-sunapp/`.
The revised test launcher waits for a Desktop window in SGP before spawn.

The guest records listener registration, accept, global advertisement,
registry sync, configure/ack, each commit and SGP redraw, and peer EOF.
The removal marker is emitted by the compositor after deleting the
matching window from its state, regardless of explicit close or an owner
cleanup that uses the same deletion routine. The gate correlates it to the
two redraws by window ID. It independently checks two QMP scanout captures
against the expected frame patterns and captures the changed scanout after
EOF and removal, verifying both patterns are absent. The old client
capture holds still provide a finite window for the host; they do not
authorize a capture or a pass. The host does not acknowledge a capture to
the client, so a long host stall can still miss a frame and fail the gate.

Transport and historical static ELF probes were not rerun. No transport,
input, dynamic ELF, or Qalculate! implementation was changed. The
historical KVM waitpid no-child failure remains unresolved; a passing
Canvas waitpid does not exercise or resolve the original interleaving.

## Two-vCPU matrix

| Test | TCG initial | TCG same-image replay | KVM | Evidence directory |
| --- | --- | --- | --- | --- |
| Original wire fixture | Pass | Pass | Pass | `wire/session-ready-*` |
| Standalone libwayland | Pass | Pass | Pass | `libwayland/session-ready-*` |
| Direct Canvas | Pass | Pass | Pass | `canvas/canvas/session-ready-*` |
| Canvas SunApp with desktop-ready launcher | Pass | Pass | Pass | `spawn-ready/{tcg-initial,tcg-replay,kvm}` |

Each pass has two matching redraws, peer EOF, removal of that window ID,
and three verified captures. Each client exited zero in the scheduler
trace. The three SunApp guests verified the fresh pin and returned
`waitpid Ok(0)`. The first three rows use images from before extracting
the same fixture desktop predicate into the shared test module; the
launcher row uses the rebuilt fixture and launcher.

The observed matrix supports using these tests as static controls for
loader planning. For a strict baseline acceptance gate, keep loader
implementation on hold until capture advances the client explicitly (or
the remaining finite capture-window dependency is accepted). The earlier
failures and the historical waitpid issue remain separate evidence.
