# PY-1: CPython 3.13.0 minimal static bootstrap

Date: 2026-11-14. **Verdict: PY-1 PARTIAL. No native Python statement executed.**
Branch `main`, HEAD `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, with incoming uncommitted runtime work preserved. No commit, push, reset, clean, stash or checkout.

Evidence root (**E**): `docs/audit/probes/python/results-2026-11-14-py1/`.
Authoritative regression runs contain **verified** in their names. Earlier attempts remain diagnostic evidence and do not validate later source.

## Outcome and gates

| Gate | Assessment | Evidence boundary |
|---|---|---|
| G1 baseline and identity | PARTIAL sequencing | Worktree hashes, locked build and matching build Python captured before edits. Independent archive verification and pristine upstream manifest recorded after initial edits; final source identity is confirmed. |
| G2 condition variables | PARTIAL | Public process-private API implemented and functional native tests pass; exact registration/park, signal/timeout and competing destroy schedules remain UNQUALIFIED |
| G3 libc bootstrap | PARTIAL | strncpy, immediate group exit and additional string conversions pass native tests; further mandatory libc dependencies remain absent |
| G4 actual core compilation | PASS for attempt; compilation FAIL | Final static configuration attempts 179 translation units: 138 success, 41 failure; one configured object not attempted |
| G5 interpreter static link | NOT_RUN | Compilation prerequisite fails; no interpreter link command or unresolved-symbol inventory exists |
| G6 interpreter execution | NOT_RUN | No interpreter executable; loader/startup, first statement, imports and repeated interpreter startup unexecuted |
| G7 legacy regressions | PASS | See final-source matrix below and E/validation-summary.json |
| G8 integrity and assessment | PASS | E/source-integrity.json, E/incoming-content-preservation.json and E/cpython-source-integrity.json |

Cross-configure success, core compilation, interpreter link, loader/startup, Python statement execution, imports/runtime, threaded Python and production readiness remain separate. Only cross-configure succeeds here. New native libc/pthread fixtures are not Python execution. **First Python output: none. Threaded Python and production readiness: NOT_READY.**

## Baseline and reproducibility

Read the R2A, R2B.1, R2B.4 and R2B.5 reports and their existing probe configuration. The incoming worktree contains substantial user-owned libc, pthread, kernel/group-lifecycle and evidence changes. `source-baseline.json`, `worktree.log`, `revision.log` and `branch.log` bind that dirty source; HEAD alone does not identify it.

`baseline.py` inventories 71,182 original file/symlink entries, builds the native archive with the unchanged locked script, records the matching build interpreter and repeats `make -j1 python` in the retained R2A acceptance directory. All occur before production edits. `locked-build.log` exits 0. The first real failure is missing `pthread_condattr_t` in `Include/internal/pycore_pythread.h:66`, followed by `pthread_cond_t` and `strncpy` diagnostics in the genuine parser compilation. Programs/python.c and Parser/token.c compile in that attempt; this is not a claim that the interpreter compiles.

The locked runtime command remains:

```sh
bash tools/build_native_std.sh build --manifest-path tools/native-c/Cargo.toml \
    --target-dir "$PWD/target/native-c" --release --locked
```

`toolchain.json` preserves installed compiler/linker/QEMU versions and exact native driver, linker script, target, toolchain and Cargo configuration contents/hashes. This is installed-toolchain rebuild reproducibility with existing caches, not a clean-room or bit-identical build claim. Native C uses freestanding static x86-64/SysV LP64, the native CRT/archive and public headers, without host libc or Linux runtime libraries.

Pre-edit source identification used the retained R2A/R1 provenance and actual probe paths; the independent archive download and fresh pristine-source manifest were recorded subsequently. The retained upstream archive and a fresh download of Python.org's exact 3.13.0 XZ archive both hash to:

`086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`.

See `upstream-download.log`, `upstream-archive-sha256.log` and `cpython-identity.json`. The build interpreter `/tmp/sunlight-r1-cpython/build-host/python` reports CPython 3.13.0 final (`build-python.log`). Its Linux host execution is a cross-build tool, never the guest runtime.

## Condition-variable implementation

Production implementation: `sunlight-libc/src/condvar.rs`; ABI: `sunlight-libc/include/pthread.h`.

Implemented `pthread_cond_init/destroy/wait/timedwait/signal/broadcast`, `pthread_condattr_init/destroy/setclock/getclock`, the two public types and `PTHREAD_COND_INITIALIZER`. CPython's actual `pycore_condvar.h`, `condvar.h` and `thread_pthread.h` select these operations. pthread.h and time.h consume the same dependency-free bits/time.h, which supplies clockid_t, the complete native timespec and clock IDs. The tv_sec long is exactly compatible with the existing time_t typedef. This preserves both complete timespec visibility for CPython and standalone use by bare C++ probes. No process-shared API or claim is added. No new kernel syscall is introduced.

The object contains a futex mutex gate, destroyed flag, waiter-list pointer and clock ID: **size 24, alignment 8** on native LP64; attributes are **4 bytes**. C static assertions and C++17 compilation/linking and native execution check the public layout and linkage. The original C++ header fixture omitted native main's C linkage; its link failure is retained separately, and `header-abi-v3.cpp` and `verified-header-abi-build.log` are the corrected compile/link control.

Each wait owns a stack-local record containing a one-shot atomic futex word and list link. Under the gate, it registers the record before releasing the associated mutex. A notifier using that mutex cannot pass registration before the release. Notification stores one under the gate and wakes that record's futex; a notification before kernel park is caught by the futex value comparison. Per-waiter one-shot state avoids sequence-counter wrap/ABA. Signal chooses one unnotified record; broadcast marks all. There is no allocation, wrapping sequence counter or accumulated pre-wait signal credit.

The gate serializes notification, removal and destroy, and protects record lifetime through wake syscalls. It is released before blocking on the wait record or reacquiring the associated mutex. Records remain linked through reacquisition, so destroy returns EBUSY during that interval. Destroy marks an empty object dead; later operations on retained destroyed storage return EINVAL. Concurrent reinitialization and freeing storage before all concurrent callers return are invalid caller behavior, not supported lifetime operations.

Both CLOCK_REALTIME (default) and CLOCK_MONOTONIC are supported. Native absolute monotonic futex deadlines drive blocking. Realtime waits re-read the selected clock after bounded sleeps of at most 100 ms rather than freezing a realtime deadline into an indefinite monotonic wait. Native clock resolution remains coarse: monotonic ticks are 10 ms and realtime reports whole seconds. Spurious kernel wakes retry; notification, errors and timeout return only after mutex reacquisition on supported paths. A notification racing timeout/removal may win and return success; callers must recheck their predicate. A valid negative absolute seconds value is treated as an already elapsed deadline; invalid nanoseconds and unrepresentable positive deadlines return EINVAL.

Null/misaligned objects, invalid attributes/clocks, destroyed objects and malformed deadlines are checked. Arbitrary invalid mappings and mutex nonownership retain the existing C/pthread preconditions. Cancellation, robust mutexes, process sharing, clock adjustment mechanisms and automatic detached reclamation remain outside this implementation.

Native tests cover signal before wait (no saved credit), single-waiter signal, four-waiter broadcast, predicate-false notifications, mutex reacquisition, invalid deadline, past deadline, both clocks, busy destroy, destroyed-state rejection, reinitialization after quiescence and 40 contention rounds with four waiters per CPU. `condvar-timeout.c` adds 60 signal/timeout attempts per CPU and records both outcomes. These are real guest tests, not host simulations.

**Qualification limits:** the associated-mutex handshake deterministically establishes registration before the busy-destroy check, and timeout/reacquisition controls execute directly. The tests do not force notification into the exact userspace-registration/kernel-park interval or the kernel's winning timeout boundary. Predicate-false notifications test caller rechecking; they are not an independently forced raw futex spurious-wake witness. Competing empty-object destroy/signal entrances, arbitrary destruction/free overlap and exhaustive futex scheduling remain UNQUALIFIED. Stress is explicitly labeled stress. The implementation is experimental, not production-qualified.

## libc changes and bounded validation

* `strncpy`: copies at most n bytes, pads after the first NUL, leaves a fully consumed n-byte destination unterminated and returns the original destination. It assumes disjoint valid regions. Native tests cover 81 source-length/count combinations with canaries; a separate mapping test places the source/destination against an unmapped page to detect excess reads/writes and verifies no source read after the first NUL.
* `_Exit`: calls the existing callback-free `sl_std_exit`, which uses native process-group exit syscall 157. It never calls pthread_exit, normal C exit, TLS destructors or the atexit registry, and adds no stdio flush step. Worker-triggered `_Exit(37)` terminates the live owner and worker; owner-triggered exits 0, 37 and 255 terminate a live peer. Exact output absence/presence, status, physical reaping and the runner's resource controls are checked. Streams are unbuffered, so a separate buffered-flush distinction is not executable; the no-flush claim is source-bound.
* Additional demonstrated parser/core string gaps: `strcpy`, `strcspn`, `strpbrk`, `strstr` and LP64 `strtol`. The conversion reuses the existing intmax parser without changing it. Native tests cover search/miss/empty boundaries, copy return/content, signed minimum, overflow/ERANGE, prefixes and end pointers. These are bounded tests, not an exhaustive standards claim.
* `unistd.h` now declares the existing native lseek ABI using off_t. No new seek implementation or offset behavior is fabricated.

`_Exit` inherits the existing native group retirement path. Runner resource balances and reaping are demonstrated; broader retirement, authenticated IPC and detached-reclamation proof obligations from R2B.4/R2B.5 remain PARTIAL/UNQUALIFIED. This work neither repairs nor upgrades those properties. No kernel, capability, ASLR, memory-protection, retirement barrier or production FastLane change is made.

## CPython configuration and source patches

Fresh source: `/tmp/sunlight-py1-cpython/Python-3.13.0`.
Authoritative build: `/tmp/sunlight-py1-cpython/cross-static`.

`configure-verified-manifest.json` records the full command/environment. It uses the native cc.py, llvm-ar/ranlib, CFLAGS=-O2, LDFLAGS=-static, explicit build/host triples, matching build Python, `--disable-shared`, `--without-ensurepip`, `--with-pkg-config=no`, `--with-libm=` and `--disable-test-modules`. All target math is in the native archive. Explicit negative cache answers remain semaphores and the two absent PTY devices, as evidenced by prior native probes. Other upstream cross-run assumptions remain assumptions, not guest-qualified features.

`DYNLOADFILE=dynload_stub.o` selects CPython's existing supported adapter for a platform without dynamic extension loading. Native dlopen exposes only the fixed main-program namespace, so symbol presence alone must not select shared-object loading. This is an explicit target configuration override; no fake successful loader implementation is added.

`Setup.local` retains all mandatory upstream bootstrap modules and adds static unicodedata, while disabling optional extensions and third-party dependencies including SSL, SQLite, ctypes, multiprocessing, GUI, and optional test modules. `MODSHARED_NAMES` is empty. Core Unicode, UTF-8, builtins and import machinery are preserved; their compilation does not prove runtime readiness.

The final `cpython-platform.patch` is the inherited minimized recognition patch: config.sub accepts sunlightos and two configure cases identify SunlightOS. Generic CPython behavior is unchanged. The initial extraction used the broader inherited experimental recognition patch; that attempt and patch are retained as `cpython-platform-initial.patch`. Final source differs from pristine only in configure and config.sub, exactly accounted for in `cpython-source-integrity.json`. Setup.local and every override are separate from upstream source edits.

## Actual compilation, linking and execution

The initial broad `make -k -j4 python` attempt is retained in `core-compile-01.log`: **177 units attempted, 130 succeeded, 47 failed**. It used the initial recognition/configuration and predates the final string fixes; it is diagnostic evidence only.

The final minimal static build repeats the full genuine target with the completed configure definitions and the repaired header. Make -W treats the upstream Python.h prerequisite as newer for this invocation, forcing real translation-unit recompilation without modifying the source: **180 configured object targets; 179 translation units attempted, 138 succeeded, 41 failed**. `Modules/getbuildinfo.o` is not attempted because its prerequisites are incomplete. `compile-summary.json` lists every attempted source, source hash, object, exact compiler command and outcome; `core-static-object-manifest.txt` lists the configured target set. These counts include mandatory built-in module units and the program entry, and are not a claim that all Python standard-library extensions were built.

`core-compile-verified.log` is the complete parallel log. `core-compile-verified-serial.log` repeats the first remaining serial failure:

```text
Parser/string_parser.c:134:17: error: call to undeclared function 'sprintf'
```

`libc-gap-inventory.json` retains the actual missing header/declaration/identifier and incomplete-type inventory. Important remaining families are sprintf/snprintf/vsnprintf; fdopen/ungetc/BUFSIZ; fcntl.h/open/access and descriptor flags; stat layout/macros; time/tm/timeval and calendar conversion; localeconv/lconv; environment mutation and path resolution; wide-string and UTF-8 conversion helpers; additional math declarations; and signal numbers/flags beyond the existing narrow synchronous subset. Missing declarations are not automatically missing implementation symbols. Conflicting prototypes arise with incomplete public types; no wire ABI mismatch or unsupported compiler feature is established. Clang's default diagnostic limit truncates some units after 20 errors, so the inventory is bounded.

The current printf family supports only an explicit integer/string subset and writes to streams. Supplying a declaration without a real memory-output formatter would not repair the first blocker. Likewise, incomplete stat/time/locale structures and unsupported signal operations need real owning-layer contracts; arbitrary layouts and fake success would conceal the remaining work. These dependencies remain open.

The compiler exits 2 before archive completion and interpreter link. **Static interpreter link: NOT_RUN.** Link command, archive order, unresolved symbols, duplicate symbols and relocation diagnostics are unavailable, rather than invented. Native fixture links exercise the actual CRT/archive only. **Native interpreter execution: NOT_RUN**, including initialization/finalization, sys.version, arithmetic, Unicode, containers, exceptions, imports and repeated startup/shutdown. No Hello-from-SunlightOS output exists.

## Final-source regression matrix

| CPUs | C | C++ | Lifecycle | TLS/once | Heap/ABI | Condvar + timeout | Strings + guard | _Exit | TCP |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 73/73 | 21/21 | 17/17 | 4/4 | PASS | PASS, bounded | PASS | PASS | NOT_RUN (unsupported) |
| 2 | 73/73 | 21/21 | 17/17 | 4/4 | PASS | PASS, bounded | PASS | PASS | PASS |
| 4 | 73/73 | 21/21 | 17/17 | 4/4 | PASS | PASS, bounded | PASS | PASS | PASS |


Authoritative rows come from `verified-*-cpu-*/results.json` and `verified-executions.json`, bound to `verified-source.json`. Supplemental timeout/guard/owner-exit cases are in `verified-functional-cpu-*`. TCP is native loopback on the runner's supported 2/4 CPU configurations; 1 CPU TCP and all 12 CPU testing are NOT_RUN. C++ includes expected fatal exits, unwinding, RTTI and static initialization. Heap/ABI, retained-owner admission, TLS/once, C/C++ public ABI and C++ runtime/vendor checks are separate evidence.

An initial C++ ABI control found that including time.h from pthread.h pulled stddef.h into the retained bare-compiler negative probe. A forward-declaration intermediate repair passed the bare C++ control but the repeated real CPython build exposed incomplete timespec in its thread/GIL/parking units. That negative build is retained in core-compile-acceptance.log. The final repair moves the unchanged native clock ABI into the dependency-free bits/time.h shared by pthread.h and time.h; no standard type layout is fabricated and neither consumer loses the complete structure. The initial successful C++ control is retained in cxx-abi-header-repair.log; the full final unchanged ABI checker is repeated in verified-cxx-abi. The original final-run series overlaps a formatting-only source change; the formatted series predates that header repair. Its per-command `source_matches=false` receipts are preserved and excluded from final-source acceptance; the final verified matrix and configure/core compilation are rerun after formatting and the complete shared-header repair. The first focused runner used the wrong JSON key (`expect_exit` instead of `expected_exit`) and failed before guest execution; corrected attempts have distinct v2/final/formatted evidence. A manifest helper initially used the wrong working directory and triggered an unnecessary disposable configure rerun; compile accounting now parses the preserved generated Makefile and compiler commands directly. Neither harness mistake is counted as a production behavior failure.

## Source integrity, review and next milestone

Final integrity checks pass for all 71,182 original entries: exactly the nine authorized original production files differ, no original file is missing, and no symlink target changed. All final regression commands report matching production hashes. No kernel or prior evidence change is attributed to PY-1.

`py1-production.patch` isolates this milestone from the incoming dirty tree. The nine changed original files can each be reversed by reversing only the isolated PY-1 edits to recover the exact incoming SHA-256; `incoming-content-preservation.json` records all nine matches. The new implementation files are `sunlight-libc/src/condvar.rs` and `sunlight-libc/include/bits/time.h`. Existing user kernel, scheduler, IPC, pthread/TLS, locks, vendored math and prior reports/evidence remain unchanged. `git diff` against HEAD also includes user-owned baseline changes and must not be mistaken for the PY-1 patch.

Recommended **PY-2: mandatory libc and static core closure**. First implement and qualify real memory-output formatting for the observed formats, then close the captured stdio/file/stat, wide-character, environment, time and locale requirements using the native owning layers. Resolve the mandatory modules' signal contract explicitly against supported kernel behavior; do not advertise absent delivery semantics. Repeat the entire core manifest, link the real native archive if compilation succeeds, preserve all linker diagnostics and attempt the first Python statement only after a genuine executable exists. Keep exact condvar/futex race qualification and inherited retirement limitations explicit; no unrelated security expansion is authorized by this recommendation.
