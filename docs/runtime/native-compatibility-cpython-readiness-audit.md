# Native runtime, C/C++ and CPython readiness audit

Date: 2026-11-14. Inspected revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`; initial tracked worktree clean. Audit only. Status applies to this checkout and the evidence identified below, not to every possible configuration or hardware. **No CPython was configured, linked, or executed.** This is a bounded source and evidence review, not an exhaustive symbol-by-symbol POSIX conformance certification.

## Executive summary

Native applications have meaningful, independently implemented Rust and C runtime foundations and a bounded C++ implementation. They are not a complete general-purpose Unix or ISO C++ platform. The custom Rust std has a native PAL and thread/futex support but explicitly rejects many filesystem operations. Native C has headers, startup, static linking and a substantial tested subset; its pthread interface lacks condition variables and cancellation, its signals are bounded synchronous signals, and its loader does not load shared objects. Native C++ is a static library and ABI adapter, not a daemon or a full standard library. Existing 2/4-vCPU guest reports show useful narrow execution, including C++ exceptions and TCP, but do not establish broad compatibility.

**Minimal native CPython is not yet demonstrated or ready to claim.** A more decisive immediate blocker is that the supported clean runtime build failed under `--locked`; a CPython cross-configuration and actual compile/link inventory have not been attempted. pthread condition variables, headers/types/functions, process/terminal semantics, UTF-8/locale and platform configuration need evidence against the chosen CPython build. Dynamic loading, C++ completeness, subprocess, TLS, and pip are not prerequisites for the first static interpreter, though each blocks later goals. The absence of a CPython build means its complete hard-blocker set is unknown.

## Architecture and dependency map

```text
Native Rust app -> patched Rust std (target_os=sunlightos) -> sl_std_* C ABI
                                                    -> sunlight-libc -> native syscalls -> kernel
Native C app -> native crt0 + static libsunlight_native_c.a -> libc -> native syscalls -> kernel
Native C++ app -> native crt0 + libsunlight_cxx.a + libsunlight_eh.a
               -> libc/pthread/TLS and target-built libc++abi/libunwind -> native syscalls -> kernel
                                    |-> network fd bridge -> capability IPC -> net_server -> network device
                                    |-> secure getrandom -> nameserver/capability IPC -> rand_service -> kernel entropy
Helios app -> Linux/musl ABI -> compat-linux syscall translation -> kernel (separate personality)
```

`tools/targets/x86_64-sunlightos.json` selects static x86-64 SysV, SSE2, no red zone, aborting Rust panic and `os=sunlightos`. `tools/native-std/prepare.py` patches a pinned Rust source tree and routes specific PAL modules; `tools/native-std/overlay/pal/sunlightos/mod.rs` declares the native `sl_std_*` boundary, implemented in `sunlight-libc/src/std_runtime.rs`. Selected Linux sync implementation is adapted through the native futex PAL; that reuse does not turn the target into Linux. `sunlight-libc/src/crt0.rs`, `tools/native-c/user-space.ld`, `tools/native-c/cc.py` and `tools/build_native_c.sh` define C startup and static linking. `kernel/src/process/spawn.rs` constructs process entry/stack and `kernel/src/arch/x86_64/syscall.rs` dispatches native versus translated Linux personality. `compat-linux/src/lib.rs` is the separate Helios ABI; a musl link or Linux host execution alone does not prove guest behavior.

`sunlight-cxx/README.md`, `tools/build_native_cxx.sh` and `tools/native-c/cxx.py` establish a statically linked library/runtime adapter and custom headers. No `sunlight-cxx` service registration was found in the inspected service manifests; this scoped search is not proof about every external deployment. It is neither a general DSO nor a full libc++ implementation. The target-built LLVM libc++abi/libunwind sources and provenance are under `sunlight-cxx/vendor/`; `sunlight-cxx/src/` implements lifecycle, guards, allocation, output and string portions. Network calls use `sunlight-libc/src/net_bridge.rs` and `ipc/src/netop.rs`; C `getrandom` calls the secure IPC route in `sunlight-libc/src/random.rs` and `rand.rs`, not the optional local non-cryptographic generator.

## Previous audit comparison

Historical claims are compared with present source and retained evidence, not treated as fresh test results.

| Prior finding | Current classification | Evidence and scope |
|---|---|---|
| 2026-11-10 Python audit: native C headers and driver absent in tested setup | **Resolved for bounded driver** | `sunlight-libc/include/`, `tools/native-c/cc.py`, `tools/build_native_c.sh`; present locked build fails, so reproducibility remains open. |
| No guest C pthread evidence | **Partially resolved** | `docs/audit/probes/native-foundation/results-2026-11-14-af-unix-state-validation/regression/dup-runtime/suite-2/results.csv` and `suite-4/results.csv` contain lifecycle cases; cancellation and condvars remain absent. |
| Native signal delivery entirely missing | **Partially resolved** | `sunlight-libc/src/signal.rs` syscall 174 and `docs/audit/std-progress.md` Phase 16 show synchronous bounded handling; `kill` and `sigprocmask` return ENOSYS, async interruption remains unverified. |
| Native C++ probe cannot link | **Resolved for configured driver; old bare command remains applicable** | Phase 23 follow-up in `docs/audit/std-progress.md`; later suite CSV reports `cpp-runtime` PASS. Bare `-nostdlib` invocation without archives/startup remains a separate failure. |
| Native C Internet TCP unavailable | **Partially resolved** | `docs/networking/2026-11-12_NATIVE_C_TCP_BRIDGE.md`, `2026-11-12_NATIVE_POLL_TCP_READINESS.md`; later AF_UNIX validation in `2026-11-14_AF_UNIX_STATE_VALIDATION.md`. No general UDP/DNS or full POSIX inference. |
| Native Rust std filesystem has many unsupported methods | **Still applicable** | `tools/native-std/overlay/fs/sunlightos.rs:237` and following; `readdir`, mutation and canonicalize explicitly unsupported. C dirent support is separate. |
| Shared library loading and CPython execution absent | **Still applicable / unable to verify** | `sunlight-libc/src/dlfcn.rs` supports only a fixed main-image namespace; no CPython execution was found or attempted. |
| Rust std runtime/FP foundation | **Partially resolved** | `docs/native-std-post-implementation-audit.md` and prior guest logs describe tested slices; they are not conformance evidence for all std APIs. |

No regression is asserted merely from the fresh build failure: the recorded failure establishes a current build blocker, without a controlled earlier baseline proving when or why it changed.

## Rust std assessment

Classification refers to the native std interface, not underlying kernel capability. Native evidence listed is historical; tests for this audit are listed separately below. `docs/native-std-post-implementation-audit.md` and `docs/audit/std-progress.md` contain prior host and QEMU details. No fresh Rust smoke was executed.

| Area | Current source/dependency and evidence | Status and unresolved semantics |
|---|---|---|
| Process / args / env | `overlay/args`, `overlay/env`, `overlay/pal/sunlightos/mod.rs`; libc startup and kernel process loader | **PARTIAL**: args, initial environment and exit paths; standard spawn/exec/wait backend not demonstrated. Environment mutation and process-state semantics need probes. |
| Threads / TLS | `overlay/thread/sunlightos.rs`, `overlay/thread_local`, `sunlight-libc/src/thread.rs`, `tls.rs`; kernel native thread | **PARTIAL**: spawn/join/sleep and TLS implementations; prior native runtime gates. Lifecycle, resource exhaustion and TLS destructors across all exits require explicit tests. |
| Synchronization | `prepare.py`, `overlay/pal/sunlightos/futex.rs`; kernel futex | **PARTIAL**: std sync implementation selected with native futex; no fresh mutex/condvar stress or edge-case validation. Atomic hardware support does not prove synchronization semantics. |
| Files / directories | `overlay/fs/sunlightos.rs`, `std_runtime.rs`, VFS | **PARTIAL**: open/read/write/seek/stat paths; sync, truncate, directory enumeration/mutation, links, canonicalization explicitly unsupported. Host/native targeted coverage must be read from prior audit; no fresh file smoke. |
| TCP / UDP / DNS / polling | `prepare.py` has no native net overlay; kernel and libc networking are separate paths | **UNKNOWN/MISSING for std API**: libc TCP evidence cannot be attributed to `std::net`; test actual backend selection and runtime. Native Rust TCP loopback **NOT_RUN**. |
| Clocks / sleep | `overlay/time/sunlightos.rs`, `overlay/thread/sunlightos.rs`, native clocks | **PARTIAL**: monotonic/realtime calls and sleep loops; precision, clock jumps and errors not fully qualified. |
| Heap / mapping | `overlay/alloc`, `sunlight-libc/src/alloc.rs`, `mman.rs`; kernel VM | **PARTIAL**: allocator and anonymous mapping exist; allocation failure, alignment, protections and mapping contract need wider native tests. |
| stdio / errors | `overlay/stdio`, `overlay/io/error`, `std_runtime.rs` | **PARTIAL**: descriptors and error conversion; fixture uses tmpfs files, not an interactive tty. |
| Panic | target JSON `panic-strategy=abort` | **PARTIAL**: abort configured; unwind/cleanup/backtraces are not demonstrated. C++ exception support is independent. |
| paths / cwd | `overlay/paths/sunlightos.rs`, `overlay/fs` | **PARTIAL**: getcwd/chdir and path splitting; current_exe and canonicalize unsupported; relative and traversal edge cases require testing. |
| signals / loading / platform | native PAL and `sunlight-libc/src/signal.rs`, `dlfcn.rs` | **PARTIAL**: narrow signal and fixed-image symbol namespace; not general POSIX signals or shared-object loading. |

## C and POSIX assessment

The table separates declarations (H), implementation (S), observed native static link (L), historical guest execution (G), and verified error semantics (E). `Y*` means only the named subset; `?` means not independently inventoried or tested. All historical G evidence is bounded to listed fixtures and revisions, not a fresh checkout run. The 2026-11-14 `suite-2/results.csv` and `suite-4/results.csv` identify build, guest and acceptance per case; `docs/audit/std-progress.md` provides detailed fixture contracts and host ABI runs. Host tests do not establish guest semantics.

| Capability | H / S / L / G / E | Evidence and semantic boundary |
|---|---|---|
| Startup, argc/env, atexit | Y / Y / Y* / Y* / partial | `crt0.rs`, `termination.rs`; `entry`, `atexit`, `exit` cases. General ABI/init ordering and terminal startup require more coverage. |
| ABI, errno / TLS | Y / Y / Y* / Y* / partial | `include/errno.h`, `src/errno.rs`, `tls.rs`; LP64 SysV target. Compare structures and errno isolation against CPython-generated calls, not only header presence. |
| malloc family / alignment | Y / Y / Y* / Y* / partial | `alloc.rs`, `include/stdlib.h`; allocator fixtures; overflow, exhaustion and all alignment APIs need independent coverage. |
| string/memory/conversion/ctype | Y / Y / Y* / Y* / partial | `string.rs`, `mem.rs`, headers; historical probes are narrow. Full locale-sensitive and conversion behavior unverified. |
| fd and stdio | Y / Y / Y* / Y* / partial | `unistd.rs`, `fd.rs`, `stdio.rs`; `stdio-full`, write cases. Guest runner uses ordinary tmpfs descriptors, not terminal/pipe acceptance. |
| directories / metadata | Y / Y / Y* / Y* / partial | `dirent.rs`, `include/dirent.h`; root/entries/errors cases. Relative and noncanonical forms explicitly return ENOSYS (`dirent.rs:83`). Struct layout and broad POSIX behavior remain limited. |
| process / wait | partial / partial / ? / ? / ? | native spawn and loader exist (`kernel/src/process/spawn.rs`); POSIX fork/exec/wait family and wait status for CPython not established. |
| signals / masks | Y / partial / Y* / Y* / partial | `signal.rs`: synchronous selected signals and return trampoline; `kill` and `sigprocmask` ENOSYS. Async delivery, signal masks, EINTR/SA_RESTART and terminal-generated SIGINT not proven. |
| pthread lifecycle / mutex / TLS | Y / partial / Y* / Y* / partial | `pthread.h`, `pthread.rs`: create/join/detach, basic mutex; limited attrs and deferred detached cleanup. Cancellation, robust mutexes, condition variables and general pthread key API absent from inspected header. |
| clocks | Y / Y / Y* / Y* / partial | `time.rs`, `include/time.h`; clock fixture. Timed condition waits cannot be inferred from clock support. |
| sockets / resolver / poll | Y* / partial / Y* / Y* / partial | `socket.rs`, `net_bridge.rs`, `multiplex.rs`; historical creation/loopback and separate external TCP reports. DNS, UDP, socket options and full readiness semantics need separate tests. |
| mmap / mprotect | Y / Y / Y* / Y* / partial | `mman.rs`, `kernel/src/process/mmap.rs`; anonymous mapping fixtures. File mappings, shared memory and complete POSIX flags not established. |
| locale / env / wchar | Y / partial / Y* / Y* / partial | `locale.rs`, `langinfo.rs`, `wchar.rs`, `env.rs`; bounded locale implementations and narrow probes; Unicode and mutable environment requirements need CPython-specific validation. |
| math / floating point | Y / partial / Y* / Y* / partial | `include/math.h`, math fixture and kernel FP state audits; not a full libm accuracy, exceptions, rounding or complete symbol proof. |
| dynamic loading | Y / restricted / Y* / Y* / partial | `dlfcn.rs`: `dlopen(NULL, 2)` and bounded main-image export table; no arbitrary shared-object load/relocate/unload. |
| headers, constants, feature macros | partial / n/a / partial / n/a / ? | `sunlight-libc/include/` is a finite native header tree. No CPython configure/header ABI inventory; declarations must not be interpreted as full POSIX. |

## C++ assessment

`sunlight-cxx/README.md` has historical sections with superseded claims; its final Phase 22 section describes the accepted subset. Native driver uses static target archives, LLVM 21.1.0 ABI/unwind, `.init_array`, `__cxa_atexit`, per-thread lifecycle and futex-backed guards. `sunlight-cxx/src/new.cpp`, `lifecycle.cpp`, `guard_support.cpp`, `entry.cpp` and vendored sources identify actual ownership. Historical 2/4 CPU suite and guard acceptance under `docs/audit/probes/native-foundation/results-2026-11-11-cxx-runtime/guard-acceptance-2026-11-12/` cover selected constructors/destructors, typed exceptions, RTTI, TLS, static initialization and retry/concurrency. `cpp-runtime` output and exit were tested with configured driver; the old bare archive-free link is a separate diagnostic failure.

`std::string`, limited output streams and nullary `std::thread` are custom bounded implementations, not standard-library ABI compatible; `string` allocation failure has fail-fast behavior. `vector`, `map`, `shared_ptr`, `unique_ptr`, `mutex`, `condition_variable`, `filesystem`, `chrono`, `fstream` and full input streams have no corresponding headers in inspected `sunlight-cxx/include` and are **MISSING/UNKNOWN**, not tested. Aligned/nothrow new, DSO/JIT unwinding, arbitrary raw pthread TLS cleanup and unwinding through Rust/C frames are outside the documented contract. ABI versioning should freeze target triple, Itanium ABI, compiler/LLVM and archive/header revisions, object layout, exception ownership, allocation ownership and visibility together; cross-language interfaces should be `extern "C"`, explicitly paired alloc/free and prohibit unwinding across Rust boundaries. Static linking is supported; shared dynamic linking is not established. No full upstream C++ std library has been supplied.

## CPython reference and dependencies

Reference: upstream **CPython 3.13.0** (fixed release baseline, selected as a stable C11 series with documented cross-build support, not a claim that it is the newest release). The upstream 3.13 configure documentation is https://docs.python.org/3.13/using/configure.html and the fixed source tag is https://github.com/python/cpython/tree/v3.13.0. Upstream specifies C11, IEEE-754 and threads; cross compilation requires a build Python matching the target version. Static libpython and statically selected built-in extensions are supported upstream. This audit did not fetch/configure this source, establish an exact feature set, or produce `config.log`; all dependency mapping below is an engineering assessment, not a successful build.

| Dependency | Assessment / first-interpreter relevance |
|---|---|
| Compiler/sysroot/config | Native freestanding clang driver and target archives exist but current `--locked` build fails. Need matching 3.13 build Python, target configure cache, no host headers/libs and actual symbol inventory. **Blocker to demonstrated bootstrap.** |
| Startup, args, stdio, file access | Narrow native probes exist; CPython initialization, stdlib import paths, real terminal interaction and script files untested. Python's encodings/bootstrap files must be packaged or frozen. |
| Allocation, VM, stack, floating point | Basic guest probes exist; CPython allocator alignment/failure, recursion stack bounds and C floating semantics unverified. Anonymous mapping constraints may be configurable, but must be measured rather than assumed. |
| Threads, TLS, GIL | Native pthread lifecycle/mutex exists; public pthread condvars absent. CPython 3.13 configured threading and GIL synchronization requirements must be tested against actual source/configuration. No GIL stress evidence. |
| Clock and entropy | clock and secure random paths exist; service availability and failure propagation during interpreter initialization need guest tests. No deterministic fallback for secure entropy should be introduced. |
| Locale, Unicode and errors | Native bounded locale/wchar and errno; CPython UTF-8 startup, codecs, filesystem encoding, error translation and non-ASCII paths untested. |
| Signals and REPL | Basic synchronous signals exist, but asynchronous delivery, masks and terminal SIGINT are not established. A basic command loop may be prototyped without claiming interactive Ctrl-C correctness; real tty test required. |
| Filesystem / import | C dirent is narrow, Rust std directory limitations are independent; static/frozen bootstrap imports and installed library layout require a real interpreter. |
| Network, select, subprocess | TCP service bridge and limited poll exist; DNS/UDP and complete select semantics unverified. `subprocess`/multiprocessing need process semantics and possibly shared mapping; optional for `print(1)`. |
| zlib / TLS / SQLite / libffi | No target C libraries or CPython integrations established in this audit; optional for first interpreter, required respectively for common compression, HTTPS, sqlite3 and ctypes use. Rust TLS is not C OpenSSL. |
| Extensions / packaging | Shared objects unavailable; statically built modules are a possible early path. `pip` additionally requires packaging layout, Python modules, compression, certificates, resolver/TLS and installation semantics; none demonstrated. |

A static minimal interpreter is architecturally plausible before shared libraries, **not verified feasible** until an actual upstream configure/build and native execution establish the missing contracts. Do not misclassify an optional extension as an unconditional first-bootstrap requirement.

## Validation and limitations

Fresh command: `python3 tools/native-c/guest.py --suite --cpus 2 --timeout 180 --out /tmp/sunlight-runtime-audit-20261114`. QEMU version was inspected, but the runner stopped at `bash tools/build_native_c.sh`: Cargo refused to update `tools/native-c/Cargo.lock` with `--locked` (exit 101). Evidence: `/tmp/sunlight-runtime-audit-20261114/{invocation.log,build-runtime.log,revision.log,worktree.log}`. **BUILD_FAIL; link NOT_RUN; guest NOT_RUN**. No lockfile was modified to make the test pass. The failure does not establish the underlying dependency change or a regression.

Requested fresh Rust file/thread/mutex/condvar/time/TCP, C hello/heap/file/pthread/mutex/condvar/socket/math, and C++ hello/lifecycle/containers/exceptions/thread/filesystem: **NOT_RUN**, because the canonical native runtime build stopped before suite construction. Rust and C++ full containers/filesystem have additional unsupported/unknown surfaces as above. No new host test substitutes for these. Historical guest results are retained in the repo: `docs/audit/probes/native-foundation/results-2026-11-14-af-unix-state-validation/regression/dup-runtime/suite-{2,4}/results.csv` report build, run and acceptance for 94 named cases per CPU configuration, including entry, stdio, pthread, signal, math and C++ runtime. Those rows are separate earlier snapshots; source/revision applicability must be verified before treating them as validation of this final tree. Dedicated TCP results are in `docs/networking/2026-11-12_NATIVE_POLL_TCP_READINESS.md`; do not equate its internal loopback pcap absence with external NIC evidence. Prior host ABI/log evidence is catalogued in `docs/audit/std-progress.md`. No fresh hardware, interactive terminal, full Rust std or upstream CPython test ran.

## Readiness matrix

`VERIFIED` means the stated narrow historical case passed, not platform conformance. `PARTIAL` means limited implementation/evidence; `STUB` explicit failing implementation; `MISSING` scoped absence; `UNKNOWN` insufficient evidence; `REGRESSED` requires an established comparison. All native-test entries below are historical unless explicitly fresh.

| Component | Implemented | Native tested | Semantic confidence | CPython impact | Priority |
|---|---|---|---|---|---|
| Native C toolchain/startup | PARTIAL | historical VERIFIED; fresh BUILD_FAIL | partial; lockfile issue | blocks reproducible port | P0 |
| libc core/stdio/heap | PARTIAL | historical VERIFIED slices | partial | bootstrap | P0 |
| C pthread/TLS/mutex | PARTIAL | historical VERIFIED slices | partial | GIL/bootstrap | P0 |
| pthread condvars/cancellation | MISSING | NOT_RUN | none | configured thread backend/advanced modules | P0 |
| filesystem and paths | PARTIAL | historical VERIFIED slices | partial | imports/scripts | P1 |
| time/entropy | PARTIAL | historical VERIFIED slices | partial | startup/runtime | P1 |
| native Rust std | PARTIAL | historical slices; fresh NOT_RUN | partial | indirect only | P1 |
| signals/masks | PARTIAL / STUB | synchronous historical cases | low for async | Ctrl-C and signals | P1 |
| native C TCP/poll | PARTIAL | historical loopback/external | bounded | socket/network | P3 |
| dynamic loading | PARTIAL / MISSING DSO | main namespace only | bounded | extensions/ctypes | P4 |
| native C++ ABI/runtime | PARTIAL | historical VERIFIED slices | bounded | not required by C core | independent |
| C++ standard library | PARTIAL | string/output/thread slices | low beyond supported subset | C++ extensions | independent |
| CPython cross-config/interpreter | UNKNOWN | NOT_RUN | none | all Python | P2 |

Independent verdicts: (1) C toolchain/runtime **PARTIAL**, fresh reproducible build blocked; (2) minimal bootstrap **UNKNOWN / NOT_RUN**, not ready to claim; (3) interactive REPL **UNKNOWN / NOT_RUN**, terminal and signal behavior open; (4) script execution **UNKNOWN / NOT_RUN**, import/layout and files open; (5) core stdlib **UNKNOWN / NOT_RUN**; (6) networking/TLS **PARTIAL foundations / NOT_RUN in Python**; (7) native extensions **PARTIAL static possibility, dynamic MISSING**; (8) pip/packaging **UNKNOWN / NOT_RUN** with known prerequisite gaps; (9) production runtime **not ready** due to absent interpreter and conformance/security acceptance.

## Dependency-aware roadmap

| Milestone | Blockers and implementation | Dependencies | Acceptance / native tests | Risks |
|---|---|---|---|---|
| P0: reproducible C baseline and ABI discovery | Diagnose locked Cargo mismatch, publish deterministic target sysroot and compare real CPython 3.13 configure/compiler undefined symbols and structures; implement missing demonstrated C/POSIX requirements | none | clean isolated locked rebuild; no host dependencies; header/layout/export checks; 2/4-CPU C stdio, allocation, file, errno/TLS/error-path probes | ABI drift, host contamination, stale lock, silent errno mismatch |
| P1: threading and bootstrap services | Implement needed pthread condvar/timed-wait/TLS semantics, validate clock, secure entropy, paths and locale against selected build | P0 inventory | 2/4 CPU create/join, mutex/condvar wake, timeout, lost-wake stress, TLS cleanup, entropy service failures, UTF-8 and filesystem errors | deadlocks, thread cleanup, insecure entropy fallback, locale corruption |
| P2: minimal static CPython | Cross-configure exact 3.13.0 with matching build Python, target configuration and built-in/frozen bootstrap; resolve actual remaining missing APIs | P0/P1 as actual source requires | link without host libraries; guest `-c 'print(1)'`, errors, imports, repeated initialization and exit on 2/4 CPUs; retain config.log and symbol maps | falsely configured HAVE macros, allocator mismatch, stack exhaustion |
| P3: REPL, scripts and core library | Package library/encodings, establish tty input, file paths, clock and signal behavior; build optional core modules incrementally | P2 | interactive terminal input/output, Ctrl-C and EOF; file scripts, Unicode imports and curated upstream tests with explicit skips | incorrect async signals, path escapes, filesystem durability |
| P4: networking and broader stdlib | Extend only observed socket/DNS/poll gaps; target zlib, OpenSSL, SQLite with provenance and configuration | P2/P3 plus service contracts | guest loopback/external DNS/TLS with certificate validation, compression and database transactions; separate peer/network evidence | certificate validation, fd lifetime, network privileges |
| P5: dynamic extensions and packaging | loader/relocations, TLS and ABI policy, libffi, installation layout, subprocess/shared resources as required | P2-P4 and loader security design | load/unload real extension, callbacks and failure paths; isolated offline package install, then authenticated HTTPS; multiprocessing tests | untrusted code, relocation validation, ABI/version breaks |
| Independent C++ expansion | choose full target C++ library strategy, allocator/exception ABI and build reproducibility | P0 ABI; not prerequisite for C Python core | multi-TU init, exceptions, RTTI, STL containers, threading/filesystem; static/DSO tests only when supported | mixing custom std ABI with upstream libraries, cross-language unwinding |

## Final verdict

The next engineering milestone is **restore and verify a reproducible locked native C runtime build, then run a pinned CPython 3.13.0 cross-configuration and symbol/header inventory**. Use that evidence to implement the minimum real threading and libc gaps before attempting a static bootstrap. This preserves the distinction between a buildable runtime, narrow guest tests, a working interpreter and production-quality Python.
