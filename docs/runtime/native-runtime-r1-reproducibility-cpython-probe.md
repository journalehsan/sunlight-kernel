# Native runtime R1: reproducibility and CPython 3.13.0 probe

Date: 2026-11-14. Revision: `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, branch `main`.

## 1. Executive summary and independent gates

**R1 COMPLETE for its controlled repair/validation scope. CPython is not operational.** The canonical locked native C build passes repeatedly after a three-line local-package lockfile repair. All 94 existing native suite cases passed in a fresh 2-vCPU SunlightOS guest: 73 C cases and 21 C++ cases. Supplemental heap/ABI and existing TCP loopback/poll guest tests passed. Static C++ ABI validation also passed.

A genuine pinned CPython 3.13.0 cross-configure used the actual native driver and a locally built matching 3.13.0 build interpreter. Unmodified upstream rejects the platform; disposable platform-recognition experiments reached real compiler/linker feature checks and stopped at C99 math. No target interpreter was built, linked or executed. Compiler/linker-observed gaps and source-inferred requirements are recorded separately.

| Gate | Status | Acceptance evidence |
|---|---|---|
| G1 locked native C build | PASS | Initial failure reproduced; reviewed offline graph repair; canonical command passed in suite, C++ preparation and two final repetitions. |
| G2 native C suite | PASS | All 73 existing C cells build/link/guest/acceptance PASS on 2 CPUs; extra heap/ABI and TCP loopback PASS. |
| G3 native C++ ABI regression | PASS | All 21 C++ guest cells PASS on 2 CPUs; target ABI/archive/unwind/vendor checks PASS. |
| G4 pthread/condvar analysis | PASS | Implementation, public API, Rust PAL and native futex/scheduler reviewed; design and race tests below. No implementation changes. |
| G5 pinned valid cross-configure attempted | PASS | Exact 3.13.0 host interpreter; SunlightOS host triplet and native static toolchain; original and isolated experimental attempts retained. Configure itself FAIL. |
| G6 actual compatibility inventory | PASS | JSON separates configure compiler/linker diagnostics, standalone native fragments, source analysis, stubs and UNKNOWN semantics. Bounded discovery, not exhaustive port requirements. |
| G7 final source integrity | PASS | Final hash comparison permits only native-C lock repair and new R1 report/evidence; initial audit unchanged; repeated final locked command succeeds. |

Evidence root, abbreviated **E** below:
`docs/audit/probes/native-foundation/results-2026-11-14-runtime-r1/`.
Machine summaries: `E/test-result-summary.json`, `E/source-integrity-manifest.json`, `E/cpython-compatibility-inventory.json`.

## 2. Initial source state and historical evidence

Initial tracked tree was clean; the only existing untracked path reported by Git was `docs/runtime/`, containing the previous readiness audit. Its contents were hashed before modification and preserved. No relevant source/dependency edits were in progress in the baseline. `E/initial-source.json` records revision, branch, status and SHA-256 of tracked and nonignored untracked files before the repair. The final comparison checks these original files, not just Git status; this also protects the untracked audit.

Read `docs/runtime/native-compatibility-cpython-readiness-audit.md`, its runner failure evidence under `/tmp/sunlight-runtime-audit-20261114`, and referenced native-foundation/AF_UNIX suite CSVs and audit documentation. Prior results remain historical. The present suite reruns the historical fixture sources; `historical=True` in the runner CSV describes fixture provenance, **not historical execution**. Every R1 PASS here is a fresh execution against the repaired source state.

The previous failing command was reproduced literally, with a new output directory:

```sh
python3 tools/native-c/guest.py --suite --cpus 2 --timeout 180 \
  --out docs/audit/probes/native-foundation/results-2026-11-14-runtime-r1/initial-suite
```

The runner invokes `bash tools/build_native_c.sh`, then `bash tools/build_native_cxx.sh`, builds C/C++ fixtures with `tools/native-c/{cc,cxx}.py`, builds isolated support/kernel images, creates a disposable ISO and runs QEMU TCG. The native C command is:

```sh
bash tools/build_native_std.sh build --manifest-path tools/native-c/Cargo.toml \
  --target-dir "$PWD/target/native-c" --release --locked
```

The wrapper prepares pinned Rust std and appends the SunlightOS JSON target, `-Z build-std=std,panic_abort`, `-Z build-std-features=` and `-Z json-target-spec`. Exact per-case commands and exit codes are retained in the runner build/ELF/kernel/ISO/QEMU logs.

## 3. Locked build root cause

Initial runner exit: **1**; nested Cargo exit: **101**, before fixture linking or guest execution. `E/initial-suite/build-runtime.log` contains complete merged stdout/stderr and exit status. Diagnostic: Cargo cannot update `tools/native-c/Cargo.lock` under `--locked`.

`tools/native-c/Cargo.toml` owns an independent `[workspace]`; it is not resolved against root `Cargo.lock`. Its only dependency is path `sunlight-libc` with feature `native-c`, which depends on path `sunlight-ipc`. All three manifests already have version `0.2.7279-alpha.2`. The native C lock retained `0.2.7254-alpha.2` for all three. The mismatch is present in committed sources, not caused by an existing user edit.

An isolated disposable directory `/tmp/sunlight-r1-lock-1qqs0g3l` copied `tools/native-c`, `sunlight-libc`, and `ipc`. `cargo generate-lockfile --offline --manifest-path .../tools/native-c/Cargo.toml` resolved that graph without touching the checkout. Its entire diff is `E/lock-proposed.diff`.

Investigated alternatives: no added/removed workspace package records, no dependency-edge changes, no registry/Git dependencies or version upgrades, no target-specific dependency sections in this graph, unchanged lock format 4 accepted by installed Cargo 1.98 nightly. Root workspace membership is immaterial to the isolated native manifest. Target build-std dependencies are separately supplied by the pinned overlay; they did not require additions to this lock. Necessity and sufficiency are demonstrated by the exact locked failure before and success after.

## 4. Applied repair and reproducibility limits

Only production build-input change: three version records in `tools/native-c/Cargo.lock`, `0.2.7254-alpha.2` → `0.2.7279-alpha.2`. Package names and edges remain identical; no external packages changed. `E/Cargo.lock.proposed` and `E/source-diff.patch` preserve proposed/applied forms. Runtime behavior, headers, ABI, target specification, manifests and toolchain configuration were not edited.

The suite's original locked build passed. The C++ static checker repeated the same runtime build. After supplemental tests, `bash tools/build_native_c.sh` passed twice (`E/final-locked-build{,-repeat}.log`). This establishes repeated locked command success for this source/toolchain. It does not claim a hermetic toolchain download, bit-identical clean-room ISO rebuild or another Cargo version's behavior. Rust overlay preparation requires rustc revision `8954863c8`; root rust-toolchain says nightly, so retain the recorded toolchain rather than treating any future nightly as interchangeable.

## 5. Native C validation matrix

Common state: baseline revision plus the reviewed lock repair; exact hashes in source-integrity manifest. Clang 23.1.4, LLD 23.1.4; native archive built with rustc `1.98.0-nightly (8954863c8 2026-06-05)` / Cargo `1.98.0-nightly (0b1123a48)`, Rust LLVM 22.1.6. Driver selects `x86_64-unknown-none-elf`, static SysV x86-64/SSE2, no red zone/PIC/PIE/stack protector, freestanding, compiler builtin headers plus native headers only, native crt0/archive and `tools/native-c/user-space.ld`. No Linux personality or host libc. QEMU 11.1.2 TCG, `-cpu max`, 2048 MiB, **2 CPUs**. No 4-CPU or hardware claim.

`E/final-suite/results.csv` and enriched `E/test-result-summary.json` retain all individual cells, source/compiler commands, build/link/guest results, expected/observed exits and assertions. All 73 C cells: **PASS**. Assertions returning nonzero fail the runner; cases with output contracts additionally compare stdout/stderr. Reaping/resource counters are checked. A boot-only observation is never used as PASS.

| Area | Fresh native result | Observed fixture scope |
|---|---|---|
| Entry, exit, startup and callbacks | PASS | `entry`, `exit`, `atexit`, callback order/register/capacity/raw-exit and exit-cleanup. |
| stdin/stdout/stderr | PASS | `write`, `stdout`, `stdio-full`, write short/zero/error cases; exact output and fixture input. Tmpfs descriptors, not interactive tty acceptance. |
| Heap | PASS, bounded | Existing exit-cleanup/dlfcn and C++ allocation fixtures; supplemental `heap-abi.c` checks malloc alignment, calloc zeroing, realloc growth/shrink preservation and free. No exhaustion/conformance claim. |
| Files and errno | PASS | `stdio`, `stdio-full`, `open-unlink`, write errors, directory contracts; open/read/write/fseek/close and selected errno paths. |
| Time and sleep | PASS | clock, time clocks/errors/zero/sleep; no wall-clock adjustment or realtime condvar claim. |
| pthread lifecycle/mutex/TLS | PASS | pthread and basic/exit/detached/attr; worker counter synchronization, returned values, private TLS, declared pthread ABI checks. |
| TCP/socket/poll | PASS, bounded | Suite checks descriptor/error/lifecycle semantics; additional existing TCP loopback/poll runner exercises real guest service transport and reports `tcp-poll: PASS socket_live=0 listener_live=0 stream_live=0`. External TCP NOT_RUN. |
| Math | PASS, bounded | math and math-contract exercise advertised sqrt/fabs/SSE subset; no full C99 libm claim. |
| Static linking and C ABI | PASS, bounded | Suite ELF checks plus pthread ABI fixture. Supplemental `-O0` probe crosses register/stack arguments (8 integers, 9 doubles) and aggregate return; guest prints `R1 heap and SysV ABI PASS`. No universal ABI conformance claim. |
| Remaining suite contracts | PASS | wchar, locale/langinfo, signal, mmap, entropy, dirent, termios, dlfcn, epoll/poll; each limited to fixture assertions. |

Supplemental commands/logs: `E/heap-abi-build.log`, `E/heap-abi-guest/`, `E/tcp-loopback/{poll-summary.json,guest/}`. TCP invocation was `python3 tools/native-c/run_poll_tcp.py --mode loopback --cpus 2 --timeout 180 --skip-build --out E/tcp-loopback`. It builds existing service/client fixtures and uses the normal native guest runner. NIC capture is not evidence of external TCP for guest-internal loopback.

Initial suite: BLOCKED at build; build/link/guest after that point NOT_RUN. No failed current-source guest assertions and no UNQUALIFIED PASS results were found. Unsupported condvars/key/cancellation were not implemented or called as a purported passing test.

## 6. Native C++ ABI regression matrix

`python3 tools/native-c/tests/run_cxx_runtime.py --out E/cxx-abi` passed target-only archive/symbol ownership, vendored libc++abi/libunwind integrity, C++14/17/20 header probes, final ELF/frame inspection, required exception/guard exports and absence of unresolved symbols, dynamic interpreter or NEEDED entries. The preserved bare historical archive-free command intentionally LINK_FAILs as its negative control; the configured driver links the same source. It is not a current runtime regression.

Architecture: static x86-64 SysV/Itanium C++ ABI, target-built LLVM vendor libc++abi/libunwind, native C archive, custom `sunlight-cxx` headers/runtime, `.init_array`, unwind sections and TLS. Compiler version does not change the vendored LLVM provenance; those hashes are checked separately.

| ABI area | Fresh 2-CPU guest evidence | Result |
|---|---|---|
| Global/static constructors and destruction | cpp-runtime, cpp-runtime-native, cxx-runtime-execution; local-static basic/order/nested output and destructor order | PASS |
| Thread-safe local initialization | concurrent/retry plus basic/nested/order; recursive initialization expected termination 134 | PASS |
| C++ allocation/deallocation | wrapped malloc/free counters, string and exception storage; expected allocation/overflow/null failure termination | PASS |
| Exceptions/stack unwinding | typed exception classes, unwind cleanup, uncaught/destructor/worker/storage termination paths | PASS |
| RTTI/dynamic_cast | exception-class and runtime-execution assertions | PASS |
| C/C++ interactions | extern-C native allocator/pthread/write and runtime wrappers | PASS |
| Threads/lifecycle/TLS | cpp-runtime and runtime-execution lifecycle/TLS destructor assertions in supported std::thread wrapper | PASS |
| Bounded string/output | cxx-string/backend and cxx-output/backend exact output | PASS |

All **21** C++ guest cells PASS. The suite's supported lifecycle paths do not establish arbitrary raw pthread C++ TLS destruction. Tests are not full libc++/libstdc++ coverage: custom string/output/nullary thread support and ABI archives do not provide general containers, filesystem or standard synchronization headers. No C++ production semantics changed.

## 7. pthread and condition-variable assessment

Sources: `sunlight-libc/include/pthread.h`, `src/{pthread,thread,tls}.rs`; `tools/native-std/prepare.py`, native PAL futex/TLS overlays; `sunlight-cxx/src/lifecycle.cpp`; `kernel/src/arch/x86_64/syscall.rs:3043`, `kernel/src/sched/mod.rs:2104`, `kernel/src/process/mod.rs:410`.

* create allocates a control block, stack and TLS, spawns the native borrower thread and publishes its opaque handle. join waits on a state futex, claims the join, waits for native thread reaping (syscall 156), then releases mappings. self uses TLS control identity; the initial thread has a stable sentinel identity. equal compares handles.
* detach is supported; reclamation is lazy on later create/join, with a bounded 64-slot deferred queue and a documented capacity-exhaustion leak until process exit. General resource exhaustion and all concurrent misuse are not qualified by the fixtures.
* mutex uses an atomic u32, acquire CAS, futex wait/retry and release store/wake-one. Only basic normal mutex semantics; attributes ignored by init, destroy returns success, no ownership/robust/recursive/errorcheck or timed mutex contract.
* **C pthread condition variables are fully absent** in the inspected header/exports, not just unexported or declared stubs. Compiler fragments confirm missing condvar declarations. Rust std selects its own native-futex condvar backend; that is another API, not an available pthread compatibility layer. C++ guards similarly do not export pthread condvars.
* pthread keys/get/set/destructor API and pthread_once are absent from public C declarations and native export inventory. Existing Rust PAL keys and C++ thread-local destructor lists have distinct contracts. CPython `Python/thread_pthread.h` uses pthread key APIs; its `Python/condvar.h` and GIL backend use condvars. These are source-inferred dependency edges backed by direct declaration/export probes, not a completed CPython thread-object compile.
* pthread_exit publishes the return value and invokes native thread exit. There is no POSIX cancellation/cleanup handler contract, general pthread key destructor iteration or timed condition wait. Do not infer these from Rust/C++ cleanup.

### Smallest correct approach, analysis only

For process-private condvars, a userspace waiter queue plus existing native futexes can support a correct ordinary wait/signal/broadcast without a new syscall. Use an internal condvar lock and a persistent per-waiter atomic notification word; register the waiter while the caller still owns the external mutex. Only after publishing registration release the external mutex, then compare-and-wait on the notification word. A signal serialized by the internal lock chooses and marks one eligible waiter before wake; broadcast marks/wakes all currently eligible waiters. A wake before actual futex registration is retained in the word, so compare-before-sleep rejects sleeping. This avoids the lost notification window without requiring a kernel atomic-unlock-and-wait syscall. A bare unlock-then-register implementation is incorrect.

The kernel's syscall 155 validates/alignment-checks the u32 and checks its expected value while holding the scheduler lock, registers address-space identity/address/deadline and blocks under the same lock. Wake is serialized by that lock, supports bounded counts, and uses the same identity. Deadline is absolute monotonic ns; finite-expiry completion returns 2, value mismatch 1, wake 0; the PAL consumes the completion receipt. Timeout, unmap cancellation and exit detach registrations through scheduler/process lifecycle machinery. These are source findings, not new SMP stress results.

Waiters must tolerate spurious wakes and always reacquire the external mutex before returning, including timeout/error. Under the internal lock arbitrate signal versus timeout, detach once, and maintain object/node lifetime through the last kernel wake. Use owned heap waiter nodes/reference discipline if asynchronous termination can outlive a waiter stack; do not leave stack pointers in a queue that survives unmapping. Process teardown and external thread death need explicit lifetime/abandonment design, not an assumption that pthread_exit runs from inside a blocked wait.

Lock order: caller mutex → short internal queue lock for registration/signaling; release internal lock before blocking; completion removes under internal lock and releases it before reacquiring caller mutex. Never block holding internal queue lock, and avoid holding it over allocation or unrelated runtime locks. Wake while lifetime remains protected, with release/acquire notification ordering. Define destroy-with-waiters EBUSY and safe object lifetime; fairness is not promised. Sequence-counter-only broadcast designs need wraparound and signal-selection analysis; a per-waiter queue is easier to specify correctly for signal-one versus broadcast.

Monotonic condattr waits can use the native absolute deadline directly. Default POSIX realtime absolute waits require an evidenced realtime-to-monotonic policy, overflow checking and rechecking clock changes; a one-time conversion is not sufficient to claim full realtime adjustment semantics. Existing clock_gettime is a foundation, not proof. Cancellation is absent; a first bounded backend can explicitly exclude cancellation, but must not advertise full POSIX cancellation semantics. Kernel process cleanup alone does not reclaim a userspace condvar queue correctly.

Required R2 native race tests on 2 and 4 CPUs: forced interleavings before/after queue insertion, mutex unlock and futex entry; signal-before-sleep; one signal with many waiters; broadcast; repeated predicate loops/spurious wakes; timeout-vs-signal winners; mutex reacquisition on timeout; already-expired/overflow deadlines and supported clock changes; object reuse/destroy-busy; repeated create/join/detach; blocked waiter termination/unmap and memory reclamation; TLS key destructor iteration/repopulation and once concurrent/reentrant policy. Record bounded completion and waiter/heap/reap counters, not just final prints.

## 8. CPython 3.13.0 origin, interpreter and cross-configure

Pinned archive: [official CPython 3.13.0 release](https://www.python.org/downloads/release/python-3130/) and `https://www.python.org/ftp/python/3.13.0/Python-3.13.0.tar.xz`. Downloaded only after no matching local source/interpreter was found in the inspected locations. SHA-256 `086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`; MD5 `726e5b829fcf352326874c1ae599abaa` matches the official release page. SHA-256 matches the downloaded official Sigstore bundle message digest. The bundle is retained, but certificate/signature trust was not independently verified; do not interpret this as a full Sigstore authentication ceremony. Source patchlevel and matching build interpreter report exactly **3.13.0 final**.

Disposable source/build roots: `/tmp/sunlight-r1-cpython/`. Host build used upstream `configure --without-ensurepip`, `make -j8 python`, both successful. This host executable is build tooling only. [Upstream cross-build documentation](https://docs.python.org/3.13/using/configure.html#cross-compiling-options) requires matching build/host Python versions; the extracted fixed release's configure logic and version output are retained in evidence. No newer CPython was substituted.

Cross command (full absolute command in `E/configure-environment.json`):

```sh
CC="python3 $PWD/tools/native-c/cc.py" CFLAGS=-O2 LDFLAGS=-static \
AR=llvm-ar RANLIB=llvm-ranlib CONFIG_SITE=/dev/null PKG_CONFIG=/bin/false \
/tmp/sunlight-r1-cpython/Python-3.13.0/configure \
 --build=x86_64-pc-linux-gnu --host=x86_64-unknown-sunlightos \
 --with-build-python=/tmp/sunlight-r1-cpython/build-host/python \
 --disable-shared --without-ensurepip --with-pkg-config=no
```

The build triplet is Linux because build tools execute on the host. The **host/target program triplet is SunlightOS**, with the driver's LLVM freestanding ELF target and native sysroot. Autoconf does not require a separate `--target` for this program build. No host headers/libraries or Linux compatibility personality are substituted. No cross cache values or config.site answers were fabricated. No forced HAVE macros, platform cache or ABI-size assumptions were supplied.

| Attempt | Result and interpretation |
|---|---|
| Unmodified upstream | FAIL exit 1: config.sub does not recognize sunlightos. Upstream platform integration issue, not missing libc function evidence. |
| config.sub recognition only | FAIL exit 1: cross build not supported for x86_64-unknown-sunlightos. |
| Disposable platform recognition | FAIL exit 1 after extensive compiler/linker checks: C99-compatible libm required; immediate acosh check cannot find separate `-lm`. This alone does not prove acosh missing. |
| Minimized recognition patch + `--with-libm=` | FAIL exit 1: acosh/asinh link successfully from native archive, **atanh undefined**; same C99 libm fatal gate. `--with-libm=` changes library naming, not a cache assertion or added math implementation. |

`E/cpython-platform-experimental.patch` records the first executed recognition experiment; it also inserted inactive host-recognition arms into compiler-selection branches. `E/cpython-platform-proposed.patch` minimizes the proposal to config.sub and the two cross-host platform-recognition cases and was used in the final no-libm attempt. Neither patch touched repository production sources. A future upstream-quality patch should update configure.ac and regenerate configure consistently; these diagnostic generated-script patches are not a port.

## 9. Compiler/linker compatibility and actual gaps

Full diagnostics: `E/cpython-configure-{original,config-sub,platform,no-libm}{,-config}.log` where applicable. `E/configure-observed-gaps.json` includes exact diagnostic line numbers from the platform attempt; `E/configure-observed-gaps.txt` is the complete explicit header/symbol/library list from that attempt. The final no-libm log additionally demonstrates atanh. `E/api-probes.json` and per-fragment logs distinguish native header parsing, declarations, archive exports and static address linking. `E/math-probes.json` has separate no-`-lm` address links. These are not falsely labeled CPython object compilation or guest semantics.

Observed core/public-header gaps include **assert.h, inttypes.h** (Clang's forwarding header cannot find a target inttypes.h), **sys/types.h, ctype.h, sys/stat.h, fcntl.h, sys/time.h, sched.h, semaphore.h, setjmp.h**. Some are unconditional public API headers; others are conditional backend/module requirements. Configure also probes numerous other-platform/optional headers (Linux, Windows, Bluetooth, database, networking, process, terminal, resource headers). Missing optional headers are not all first-interpreter blockers. `ac_nonexistent.h` is an intentional compiler negative control and excluded from requirements.

Actual missing C declarations/exports observed in bounded fragments include pthread_cond_*, pthread_key_create/delete/get/setspecific, pthread_once, pthread_attr_setstacksize, getenv/setenv/unsetenv, abort, setvbuf, snprintf, getpid, fork, execv, pipe, dup and selected wchar conversion APIs. `getenv_bytes`/Rust getenv helpers do not export the required C getenv ABI. **lseek is exported but undeclared in the tested unistd.h inclusion**, so link availability through that header is UNKNOWN in the fragment, not a missing implementation claim. Header declarations and archive/link availability are separate inventory fields.

Configure's actual undefined-symbol list includes, among many conditional features, access/chdir/chmod, stat/fstat/lstat families, exec/fork/wait families, readv/writev, clock_getres/settime/nanosleep variants, pthread_sigmask and TLS-related candidates, and socket resolver/optional process/terminal APIs. Consult the exact JSON/text inventory rather than treating this paragraph as an exhaustive minimum. `main` from a compiler identification check is excluded from requirement gaps. A failed feature test may also fail on a prerequisite header: those failures alone do not prove every tested symbol absent.

The final no-libm attempt establishes atanh as a real undefined symbol. Independent address links also fail for **frexp, lgamma, modf**. Most other probed math names resolve as weak compiler_builtins exports (for example acosh/asinh/cos/tgamma), even though native math.h advertises only a narrow sqrt/fabs subset. `E/archive-symbol-provenance.log` identifies archive object ownership. Address resolution does not validate function prototypes, numeric accuracy, errno/fenv behavior or a supported public libm ABI. Do not silently expose or certify those incidental exports.

Separate-library checks for `-lpthread`, `-lm`, optional libraries and several other-OS libraries must be distinguished from unavailable functions in the monolithic native archive. In particular pthread_detach resolves in configure despite lack of a separate pthread archive. Rust build-std and the native C archive satisfy some symbols that are absent from public C headers. This is why documentation-only missing-API inference would be misleading.

`E/cpython-compatibility-inventory.json` classifies each discovered requirement as VERIFIED/PARTIAL/STUB/MISSING/UNKNOWN with subsystem, source references, native locations, header/link availability, runtime evidence, confidence, severity and next action. VERIFIED header entries mean successful header parsing only, with explicit evidence; they never imply runtime verification. Source behavior entries preserve ENOSYS stubs (kill/sigprocmask), Rust-versus-pthread distinctions and unknown bootstrap imports. Critical interpreter threading/public-header requirements are separated from optional extension and other-platform feature candidates. There is no readiness percentage.

## 10. Minimal static interpreter feasibility

Target configure did not finish and produced no final target Makefile/pyconfig.h. Minimal target interpreter build/link: **BLOCKED**; target interpreter guest execution: **NOT_RUN**. No Linux-host pyconfig.h was reused as a target config, and no unsupported HAVE values were forced to reach a pretend link. Host Python's successful execution proves only build-tool availability.

A static native interpreter remains **unproven**. Immediate dependency classes: toolchain/platform recognition and archive naming; libc headers/C99 math/public symbols; threads/condvars/keys; filesystem metadata and import layout; time semantics; signals (linked ENOSYS stubs); memory allocation/mapping semantics; optional shared-module loading. Encoding/bootstrap packaging and platform-selected missing requirements remain UNKNOWN until configure and real object compilation advance. Dynamic loading and broad optional modules need not block a deliberately static/frozen first interpreter.

## 11. Final source integrity and evidence inventory

`E/source-integrity-manifest.json` compares every original source hash with final state, records final revision/branch/status, authorized differences, added R1 deliverables, generated-output areas and test-source fingerprint. `E/test-result-summary.json` attaches that fingerprint, revision, compiler/linker/target/CPU and evidence to each test. Original files other than the three-line lock repair are unchanged, including the prior untracked audit. No source drift was detected between the repaired tests and final build. Documentation/evidence additions are not executable build inputs; no PASS was transferred from a different runtime source state.

| Category | Inventory |
|---|---|
| Existing modifications | No tracked changes; existing untracked docs/runtime audit preserved byte-for-byte. |
| R1-authorized modifications | tools/native-c/Cargo.lock three local package versions; this new report; new isolated E tree containing logs, JSON, diagnostic scripts/fixture, proposed patches and input copies. |
| Build-generated artifacts | Existing build infrastructure writes target/native-c, target/native-cxx, target/native-std, guest/support/kernel outputs and disposable guest ISOs; evidence-local ELF/object/map/frame outputs; /tmp disposable lock and CPython source/build trees. These are not production-source repairs. Preexisting ignored target contents were not a clean-build baseline. |
| Unrelated source drift | None detected by baseline hash/Git comparison. |

Runner-created .gitignore files keep guest binaries/images out of Git; E also ignores standalone ELF/object/archive products. They remain available locally. No earlier evidence directory was overwritten. No reset, clean, stash, checkout, commit, push, privileged disk/mount operation or production runtime change was performed.

Relevant negative evidence remains intact: original locked rejection; expected historical bare C++ LINK_FAIL; upstream triplet rejection; platform rejection; failed configure checks; absent library diagnostics; final atanh failure; standalone failed headers/declarations/links.

## 12. Remaining blockers and R2 recommendation

Recommend **R2: minimal CPython 3.13 platform/header and threading contract**, driven by this inventory rather than general POSIX expansion:

1. Define minimal static interpreter configuration and accepted platform patch; rerun configure without separate-lib assumptions and without invented cache answers. Resolve the demonstrated C99 math/header/public C ABI blockers only as selected by that build. Preserve incidental compiler-builtins versus supported ABI distinctions.
2. Implement process-private pthread condvar/timed wait and key/TLS destructor requirements with explicit lifecycle, clock and cancellation limits. Run the native race tests above on 2/4 CPUs before GIL/bootstrap claims. pthread_once only if selected source requires it; absence is inventoried, not automatically a CPython hard blocker.
3. Advance real target object compilation and static link; update inventory from actual undefined symbols. Package/freeze bootstrap encodings and minimal import path. Only after link success attempt guest `-c 'print(1)'`, error/exit/repeated startup and thread tests.

External networking, full C++ standard libraries, general process APIs, dynamic extension loading and pip remain separate goals unless the chosen minimal build proves a dependency. R1 restores a reproducible native runtime validation baseline; it does not complete a CPython port.
