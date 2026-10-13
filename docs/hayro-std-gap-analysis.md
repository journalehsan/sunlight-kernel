# Native Rust std gaps for Hayro

## Finding (2026-10-12)

Running the published Hayro 0.8.0 renderer unchanged as a native SunlightOS
program requires a real Rust standard-library platform port. SunlightOS
currently provides `core`, `alloc`, native syscalls and userspace libraries,
but no native Rust `std` target or platform implementation. This is a separate
project from adding alloc-only support upstream. The latter remains the
smaller rendering-specific route described in [Phase 2](hayro-native-phase2.md).

Improving `sunlight-libc` alone will not make Cargo's missing `std` crate
available. Conversely, making `std` compile will not fix Vello's separate LLVM
code-generation failure or establish safe native SIMD execution. Both routes
need supported userspace code generation and verified floating-point/SIMD
state preservation before a native renderer can be accepted.

This investigation adds a source-backed implementation plan. It does not add
a platform port, dependency forks, new syscalls or a rendering workaround.
Full native rendering and in-system PDF display remain unverified.

## Evidence and scope

Sources inspected are the live worktree, published dependency sources in the
local Cargo registry, the installed Rust sources, and the existing Phase 2
compiler logs. Versions are Hayro 0.8.0, Vello 0.3.0 and
`rustc 1.98.0-nightly (8954863c8 2026-06-05)`, LLVM 22.1.6. This is an audit of
these versions, not a claim about future releases or upstream main.

Rust paths below are relative to
`/home/ehsantor/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/lib/rustlib/src/rust/library/`.
Dependency paths are relative to each versioned registry crate. The
[dependency matrix](hayro-phase2-matrix.md) records the selected target graph;
host proc macros and generators do not require a SunlightOS runtime port.

The live target specification, obtained with
`rustc -Z unstable-options --print target-spec-json --target x86_64-unknown-none`,
declares `std: false`, `rustc-abi: softfloat`, disabled SSE/AVX features and
`+soft-float`. It is the freestanding kernel target, not a SunlightOS userspace
platform target. `target-cpu=x86-64-v2` does not replace that ABI definition.

The recorded Vello release builds fail in both `vello_common` and `vello_cpu`
with `rustc-LLVM ERROR: Do not know how to split the result of this operator!`.
The existing diagnostic SSE override builds rlibs with target ABI warnings;
it is not a validated solution. No new Vello build or native execution was
performed in this audit, and the precise LLVM root cause is still unresolved.

## Which std APIs actually need OS support?

Many imports named `std` are already implemented by `core` or `alloc`:
`Vec`, `String`, `Box`, `Rc`, `Arc`, `RefCell`, iterators, formatting, ordinary
errors, traits and atomics. They are not separate missing kernel facilities.
An upstream alloc-only port would adjust imports, ownership and features.
A native std port would supply the actual Rust crate and its platform hooks.

Memory-backed `std::io::Read`, `Write`, `Seek` and `Cursor` also do not require
filesystem syscalls. Moxcms parses ICC profiles with `Cursor`/`Read`;
embedded CMaps decompress Brotli between in-memory buffers. Their compilation
requires the std APIs selected by these releases, but this does not justify
adding a new kernel file API for every I/O trait.

The concrete platform needs are:

| Area | Consumer / evidence | Existing SunlightOS facility | Missing work |
| --- | --- | --- | --- |
| Target and sysroot | Native target says `std: false`; Rust `std/build.rs` selects `restricted_std`; `std/src/sys/pal/mod.rs` selects `unsupported` | Kernel/native userspace builds and linker script | Separate userspace target contract and maintained Rust platform implementation; rebuild compatible core/alloc/std and distribute the matching sysroot |
| Allocation | Every parser/render container; `std::alloc::System`; Phase 1 counting allocator uses `System` directly | Reclaiming `SunlightAlloc`, anonymous mappings, heap statistics (`sunlight-libc/src/alloc.rs:15`, `:733`; `mman.rs:65`) | Wire both the default allocator and `System` correctly; define bounded capacity, alignment, realloc/OOM and bootstrap behavior without recursive allocation/IPC |
| Blocking synchronization | Interpreter `cache.rs:7` uses `Arc<Mutex<_>>`; syntax std mode uses `Mutex`/`RwLock`; colors/functions and embedded CMaps use `OnceLock`/`LazyLock` | Atomics and internal spinlocks; Linux compatibility has a limited futex path | Native shared-address-space wait/wake contract; Rust mutex/rwlock/once/condvar/parking backends; correct publication and lost-wakeup avoidance |
| Thread lifetime | Production viewer `sunlight-docview/src/ui.rs:115` spawns a worker and `:272` closes/joins it | `sunlight-libc/src/thread.rs:58` starts a native thread | Join completion and return ownership, detach lifecycle, safe stack/TLS reclamation and shared process resources; thread-local destructors |
| TLS | Rust runtime/thread machinery; `std/src/hash/random.rs:55` caches HashMap seeds per thread | FS_BASE per task, a one-page TCB containing self pointer and errno | Rust TLS keys or compiler TLS image layout, initialization/destruction and main-thread bootstrap; errno TCB is not general Rust TLS |
| Files, paths and stdio | Host driver and production backend use `File`, `Path`, metadata, readers/writers; service diagnostics use stderr | Native open/read/write/seek/stat/close, stdout/stderr descriptors and VFS capability boundary | std file/descriptor/stdio adapters, error translation and cleanup; explicit supported path/encoding contracts and bounded snapshot reads |
| Time and sleep | Moxcms `dat.rs:102` provides `ColorDateTime::now`; viewer/driver diagnostics and timed waits use clocks | `sunlight-libc/src/time.rs:1` exposes realtime and monotonic clocks | std `SystemTime`/`Instant` backend; native blocking sleep/deadlines when needed by thread/parking APIs; preserve actual clock precision |
| Entropy | Default std `HashMap`/`HashSet` seeds, including interpreter optional-content state; general std random hooks | Conditioned kernel entropy and capability-scoped rand service (`sunlight-libc/src/rand.rs:1`) | std random adapter, including failure/bootstrap policy; avoid entropy IPC while holding runtime locks; preserve fail-closed behavior |
| Math and SIMD | Vello/fearless_simd, pic-scale, moxcms, pxfm and JPEG dispatch | CPU initialization enables x87/SSE; existing core atomics and libm-capable dependencies | Supported userspace floating-point ABI, math/builtin linkage, CPU/OS feature detection and per-task register state preservation |
| Startup and failure | Native `_start` versus Rust std runtime; prototype `catch_unwind` | Native argv/envp, exit, CRT helpers and panic-abort programs | Rust runtime init/cleanup, args/env, process-wide exit versus thread exit, panic reporting and abort; unwinding is a separate optional project |

An API present in a dependency source is not proof that the sample calls it.
For example, pic-scale compiles `thread::available_parallelism`, but its
selected non-threading implementation creates no worker pool. Moxcms compiles
`SystemTime::now` for profile dates; no native trace proves that method is
called by this PDF. These still matter when compiling unchanged std-based
libraries, but should not drive speculative kernel additions.

## Concrete SunlightOS gaps to improve

### Native blocking wait and wake

`kernel/src/arch/x86_64/syscall.rs:2162` implements Linux futex wait/wake. It
requires `is_linux_compat()`, records waiters in Linux process state and rejects
timed waits. It cannot currently back a native std process. Reuse its generic
scheduler/address-space reasoning when designing native synchronization,
without treating the Linux-only ABI as native support.

One atomic compare-and-enqueue wait plus wake primitive can support several
Rust synchronization APIs. Deadlines are needed for timed condvars, parking
and channel waits; separate kernel syscalls for every Rust lock type are not
necessary. Specify address validation, address-space identity, lifetime across
unmap, completion/timeout races and acquire/release publication. Keep existing
capability, syscall and IPC contracts intact.

The installed Rust fallback `std/src/sys/sync/mutex/no_threads.rs` uses
`Cell<bool>` with `unsafe impl Sync` under the assumption that no threads
exist. `once/mod.rs` likewise selects a no-threads implementation for unknown
platforms. Those assumptions are unsuitable for SunlightOS native threads or
the viewer worker. Public std locks must retain Rust's synchronization semantics.

### Join, TLS and shared process state

The native threading module explicitly lacks join. Its trampoline discards the
entry result, and dropping the current `JoinHandle` does not reclaim stack/TLS
mappings. The kernel currently clones the native thread's descriptor table;
descriptor-table mutations are not shared as a std process would expect.
These are visible implementation gaps, rather than deductions from missing
crate errors (`sunlight-libc/src/thread.rs:6`, `:32`, `:47`;
`kernel/src/arch/x86_64/syscall.rs:1969`).

`munmap` now has a real native wrapper and kernel path, despite the older
thread-module comment calling reclamation deferred with real munmap. The
remaining threading work is to coordinate cleanup after the thread has left
its stack, implement completion/return ownership and test repeated lifetimes.
Availability of munmap alone does not make automatic cleanup safe.

FS_BASE switching already exists (`kernel/src/sched/mod.rs:1469`, `:1509`).
The existing TCB only initializes self pointer and errno. Decide between a
native Rust TLS key backend and compiler TLS support; the latter needs an
explicit `.tdata`/`.tbss`/TLS-template contract with the linker and ELF startup.
The current userspace linker script does not specify those sections, and no
general TLS-template initialization was found in the native loader. Rust TLS
destructors must run on thread completion before TLS storage is freed.

### Floating-point and SIMD state

`kernel/src/arch/x86_64/cpu.rs:24` enables x87/SSE and clears CR0.TS. The examined
context switch saves general registers and FS_BASE
(`kernel/src/sched/context.rs:7`; `kernel/src/sched/mod.rs:1467`). Source search
found no FXSAVE/FXRSTOR or XSAVE/XRSTOR implementation in `kernel/src`.
Enabling instructions does not establish preservation of x87, XMM or MXCSR
across tasks, interrupts and CPU migration.

This is a generic correctness gap to resolve and test before claiming safe
native rendering. Start with an explicitly supported x87/SSE state contract.
AVX dispatch additionally needs OSXSAVE/XCR0 and full YMM state management.
Compiler CPU features, runtime dispatch and the kernel's actual saved state
must agree. A single-thread renderer still gets preempted by other processes.
The LLVM failure should be reduced independently against the exact target and
toolchain; do not equate the source-level state gap with a proven LLVM cause.

### Allocation and renderer containment

The native heap defaults to 256 KiB static or 1/8/16 MiB dynamic presets.
The prior full-renderer host workload peaked around 10.36 MB of requested
allocations, excluding allocator overhead, stacks and mappings. Those are
recorded host results, not a new native memory measurement. Five page caches
and large outputs increase the requirements beyond a parser smoke test.

Wire the allocator into a native std port, then measure capacity and reuse.
`try_reserve` at the snapshot boundary does not make Hayro's infallible parser,
decompression, color-table and pixmap allocations recoverable. Enlarging the
heap does not create a working-set bound. Require either upstream fallible
budgets/cancellation or tested disposable renderer-process containment, with
owner-scoped IPC and mappings released after faults/OOM. A std port alone
does not satisfy this requirement.

## A real std port has more hooks than PAL

In this Rust revision, platform selection is distributed across
`std/src/sys/{alloc,args,env,exit,fd,fs,io,net,os_str,path,pipe,random,stdio,sync,thread,thread_local,time}`
as well as `sys/pal`. Adding a PAL directory alone is insufficient. In
particular, the unknown-none selection has no `GlobalAlloc for System`, no
selected random `fill_bytes`, and no general TLS key implementation. Time and
thread fallbacks are unsupported/panic paths.

A maintainable port needs a pinned compiler/Rust-source revision and a small,
reviewable native backend patch set, followed by an upstream target strategy.
Its low-level bindings should remain below std and avoid a dependency cycle
through a std-using application library. Existing no_std native ABI wrappers
and capability IPC can be reused where their contracts match. Any unsupported
public OS operation must fail explicitly rather than manufacture success.

Rust's reusable higher-level I/O, paths, collections, Arc, channels and lock
algorithms should be preserved. There is no need to write a substitute crate
named `std` or duplicate those APIs throughout the workspace. A native std
userspace port also does not require changing the kernel to use std.

## Recommended implementation order and acceptance gates

| Stage | Deliverable | Gate before moving on |
| --- | --- | --- |
| 1. Target / CPU foundation | Document a native userspace ABI; reduce Vello LLVM failure; preserve supported FP/SIMD state across all dispatch paths | Consistent core/alloc build and userspace executable link; QEMU register-sentinel/preemption tests with 2 and 4 CPUs, including migration; kernel boot regression |
| 2. Minimal real std runtime | Pinned native Rust platform backend: startup/exit, allocator/System, stderr, args/env, entropy, TLS and clocks; memory-backed I/O uses Rust's implementation | Native std executable runs Vec/Arc, Cursor, HashMap, time and init/drop probes; heap recovery and OOM process-exit behavior measured |
| 3. Synchronization / thread lifetime | Native compare/wait/wake, deadlines, Rust sync/parking backend; join/detach, TLS destructors and shared descriptors | Contention and lost-wakeup tests, timeouts racing wake, once initialization, TLS isolation/drop, join publication and repeated spawn/join reclamation in QEMU |
| 4. VFS and viewer runtime | std file/stdio adapters; bounded snapshots; production worker channels and process-wide cleanup | Read/seek/EOF/errors/close probes, repeated viewer worker shutdown and owner/capability isolation; no descriptor or mapping growth |
| 5. Full unchanged renderer | Compile the pinned Hayro graph with supported native std; link existing canvas/service integration | Actual sample raster displayed in SunlightOS, repeated open/render/drop and resource recovery; fault/OOM containment; no host-render substitutions |

Stage 3 is required before accepting the current viewer worker, while the core
memory renderer does not need worker threads. Thread-safe synchronization and
TLS selection still need correct semantics in a native std target that allows
native thread creation.

Networking, GPU support, fontconfig, arbitrary Linux syscall coverage, a
PostScript document interpreter and Rayon are not demonstrated prerequisites
of this pinned CPU PDF graph. The existing generic document/canvas protocol
can be retained. Text extraction and non-PDF backend parity are separate
integration requirements, not std primitives.

## Validation boundary for this investigation

Two isolated `build-std` checks were attempted with an empty diagnostic crate
under `/tmp/sunlight-std-gap-probe`, outside production resolution:

```sh
cargo +nightly check --manifest-path /tmp/sunlight-std-gap-probe/Cargo.toml \
  --target x86_64-unknown-none -Z build-std=std,panic_abort --offline
cargo +nightly check --manifest-path /tmp/sunlight-std-gap-probe/Cargo.toml \
  --target x86_64-unknown-none -Z build-std=std,panic_abort \
  -Z build-std-features= --offline
```

The first stopped at uncached `addr2line 0.25.1`; the second, with optional std
features disabled, stopped at uncached `libc 0.2.185`. Neither reached std
compilation. These are dependency-cache limitations, not measured std compiler
or linker failures. Logs are `target/hayro-phase2/hayro-std-build-audit.log`
and `hayro-std-minimal-build-audit.log` in that directory. No downloads, installed
sysroot changes or runtime tests were needed to establish the source gaps.

The smallest immediate generic implementation to prioritize is the target /
FP-state foundation, followed by native synchronization and thread lifetime.
For Hayro alone, upstream alloc-only interpreter/raster support continues to
avoid the larger std platform project, but does not remove the target/SIMD or
resource-containment gates. Neither path is currently a working native PDF
renderer.
