# PY-3: mandatory libc contracts and complete CPython core compilation

2026-11-14. **Overall: PARTIAL. No interpreter linked or executed; no actual Python output.**

Evidence root (**E**): `docs/audit/probes/python/results-2026-11-14-py3/`.

Exactly the retained CPython **3.13.0 final** source and matching build interpreter were used. All **180 configured objects were attempted: 163 PASS, 17 FAIL, zero unattempted**. PY-2 had 160 PASS, 19 FAIL and one unattempted. The two repaired failed units are `Python/initconfig.o` and `Python/formatter_unicode.o`; `Modules/getbuildinfo.o` is newly attempted and passes. Mandatory failures remain, so the genuine static interpreter link and native interpreter smoke are **NOT_RUN**. This does not establish interpreter readiness.

| Gate | Result | Evidence boundary |
|---|---|---|
| G1 | PASS | Locked baseline, exact archive/source/build interpreter and all 19 PY-2 failures reproduced; unattempted getbuildinfo identified without inferred diagnosis. |
| G2 | PASS | Owning-layer limitations established in source; existing native metadata subset tested. No filesystem production adapter invented. |
| G3 | PARTIAL | Implemented native-tested bulk conversions, locale conventions, unbuffered selection, clock resolution, and _exit. Mutable environment/calendar/langinfo and full buffering remain unresolved. |
| G4 | PASS | Required semantics evaluated against existing native services; no unsafe stubs or fake constants. |
| G5 | PASS | Genuine configure plus all 180 configured object attempts; 163 PASS, 17 FAIL, zero unattempted. |
| G6 | PASS | Conditional blocked-compilation acceptance: all mandatory blockers and exact diagnostics preserved. No complete interpreter link claimed. |
| G7 | PASS | Final-source native regressions and supported CPU matrix; see regression-results.json. |
| G8 | PASS | Standalone reversible PY-3 patch, incoming regular-file hashes, CPython/kernel integrity and security review; inherited qualifications preserved. |

G2 and G4 evaluate the expressly allowed blocked capabilities; they do not label those capabilities operational. G5 covers complete configure/attempt accounting and compilation itself fails. G6 uses the mission's conditional acceptance for demonstrably blocked mandatory compilation. G3 remains PARTIAL independently of those conditional gates; PY-3 COMPLETE is not claimed.

The baseline is branch `main`, revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`. `source-baseline.json` records Git status, toolchain versions and 77,830 tracked/unignored regular-file hashes before production changes. Incoming modifications and untracked evidence remain in place. Symlinks and directory entries are outside that hash inventory. No reset, stash, clean, commit or push was performed. Existing user kernel edits were neither reverted nor extended.

The archive `/tmp/sunlight-r1-Python-3.13.0.tar.xz` has SHA-256 `086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`. Every regular archive member was compared against `/tmp/sunlight-py1-cpython/Python-3.13.0`: the only incoming differences are PY-1's `configure`/`config.sub` SunlightOS recognition changes, retained separately as `inherited-platform-recognition.patch`. No new CPython edits were made. The matching Linux build tool is `/tmp/sunlight-r1-cpython/build-host/python`, version 3.13.0 final; its version and executable hash are recorded. It is not a native interpreter result.

`locked-baseline.log` and `locked-final.log` retain successful `bash tools/build_native_c.sh` executions, which invoke the existing installed native toolchain with `--release --locked` and package its native archive/header tree. This is an installed-toolchain rebuild with retained caches, not clean-room or bit-for-bit reproducibility. `py2-failures-reproduced.json`, `py2-failure-inventory.json` and `baseline-*.log` reproduce all 19 failed PY-2 units before production edits. The unattempted `Modules/getbuildinfo.o` had no inferred diagnosis.

`mbstowcs` and `wcstombs` now provide bulk conversion with private per-call state over the existing C ASCII/strict UTF-8 baseline. They count without storing for NULL output, respect element/byte capacity, exclude the terminator from their result and never emit a partial UTF-8 character. Invalid, overlong, surrogate and truncated inputs fail with EILSEQ. Counts are checked for overflow. Conversions allocate nothing and share no conversion state; callers must serialize locale changes against conversions. Native cases cover exact bounds, zero bounds, counting, canaries, guard pages, malformed input and concurrent callers.

`localeconv` and native LP64 `struct lconv` expose consistent C conventions: decimal dot, empty grouping/thousands and monetary strings, CHAR_MAX for unspecified monetary numeric values. Returned storage has process lifetime and is read-only to callers. All currently installed locale names retain this numeric/monetary baseline, including `en_US.UTF-8`; full US locale semantics are not advertised. Calendar `nl_langinfo` constants/data remain absent, so `_localemodule.o` now advances from missing lconv to missing DAY_1.

`setvbuf` explicitly supports **only _IONBF**, the actual existing stream policy. Buffered/line-buffered requests fail ENOSYS; invalid modes fail EINVAL. Caller buffer storage is never retained or dereferenced, and failure leaves the stream policy unchanged. `setbuf(NULL buffer)` selects the same unbuffered policy; non-NULL requests fail through errno. This is a supported subset, not working buffered stdio. Configure detects the real setvbuf implementation, and `initconfig.o` compiles. CPython's interactive buffered-policy requests still cannot be honored and remain a runtime limitation.

`clock_getres` reports the existing sources' real quantization: one second for RTC realtime and 10 ms for the canonical 100 Hz monotonic timekeeper (`kernel/src/timekeeping.rs:10`, syscall implementation at `kernel/src/arch/x86_64/syscall.rs:12018`). NULL output is permitted, invalid IDs preserve output and fail EINVAL, success preserves errno. No synthetic current time or timezone was introduced. Calendar/tm/timeval conversions, strftime, timezone data and CPU accounting remain separate unfinished work.

`_exit` uses the same existing immediate process-group termination primitive as inherited `_Exit`: `sl_std_exit` -> syscall 157 -> `process_exit`. Native worker execution exits the whole group with status 38, without the registered atexit callback or owner-survival message. Existing `_Exit(37)`, main-return/normal-exit, pthread-exit and lifecycle controls remain separate tests. `_Exit` and `strncpy` were already present in the incoming tree and are not claimed as PY-3 implementations. The original string guard-page fixture and new byte-padding/exact-bound/unaligned/return-value cases pass; overlap remains undefined by C, with no memmove guarantee.

**Filesystem capability: BLOCKED for realpath and POSIX stat, with the existing metadata subset validated.** Native VFS `FileType` has only File/Directory (`sunlight-fs/src/vfs.rs:51`), and its path validator rejects relative paths and dot/dot-dot (`sunlight-fs/src/path.rs:3`). There is no native symlink/readlink/canonical-resolution primitive. Lexically removing components could skip nonexistent components and cannot prove traversal permissions, symlink resolution or races. No realpath export or successful lexical substitute was added. Absolute/relative canonicalization, cwd expansion, multiple separators, dangling links/loops, allocation/buffer policy and permission/race semantics remain BLOCKED, not untested implementation claims.

The existing 24-byte metadata wire carries size, uid, gid, permission mode, File/Directory type and nlinks (`sunlight-libc/src/lib.rs:384`, `kernel/src/arch/x86_64/syscall.rs:9268` and `:10548`). It lacks device/inode identity, timestamps, special-file/symlink metadata and block accounting. Existing C `fstat` writes that native record; silently replacing its destination layout with a larger POSIX structure would also break its ABI. No fields were invented, no sys/stat.h compatibility layout was added, and no full stat/lstat adapter was claimed. `filesystem-test.c` validates real tmpfs files, empty/post-write size, mode 0600, link count, path/fd agreement, directory type, missing components, close/unlink and error paths. `guards-errors-test.c` verifies raw native ENOENT=-3 and EINVAL=-6; the existing libc mapper translates these to standard errno. fstat negative/null/closed cases verify EBADF/EFAULT. Permission-denial injection and filesystem races are NOT_RUN; there is no new permission bypass.

**Entropy: required nonblocking semantics BLOCKED.** The existing secure C flags-zero path checks entropy readiness and uses capability-IPC `rand` service chunks with bounded timeouts; errors wipe partial destination data and map to generic EIO. CPython bootstrap explicitly requires GRND_NONBLOCK. Existing IPC may wait and cannot supply its nonblocking/EAGAIN contract. Rust private bit 1 selects a noncryptographic PRNG and must not be aliased to C GRND_NONBLOCK. No clock/PID fallback, fake data, readiness shortcut, /dev/urandom device or new random adapter was added. Functional random-service tests do not qualify cryptographic quality or nonblocking behavior.

**Signals: required semantics BLOCKED.** Existing syscall 174 permits initial-thread synchronous SIGUSR1/2 registration/raise only, rejects workers/groups and nonzero masks/SA flags, and lacks required SIGINT/fatal delivery semantics. kill/sigprocmask remain explicit ENOSYS. CPython's SIGINT, SIGABRT/SIGSEGV/SIGFPE, SA_RESTART/SA_ONSTACK/SA_NODEFER and broader dispositions cannot be supplied by declarations alone. No fake handlers/constants were added. Consequently abort was not silently replaced with _Exit. Native getpid also still returns a worker identity unless the Linux-specific tgid override applies; a misleading C process-identity wrapper was not exported. These require separately authorized owning-layer contracts.

Mutable environment is also unresolved: existing Rust getenv returns process-lifetime immutable borrows into envp. setenv/unsetenv cannot mutate/free those values without an explicit shared ownership and synchronization design. Remaining calendar and C-locale langinfo data are feasible libc work; they are not all mislabeled as kernel blockers.

The fresh isolated build is `/tmp/sunlight-py3-cpython/cross-static`. `configure-manifest.json` preserves the genuine command and environment: native `tools/native-c/cc.py`, static LDFLAGS, llvm-ar/ranlib, matching build Python, `--disable-shared --without-ensurepip --with-pkg-config=no --with-libm= --disable-test-modules`. PY-2's Setup.local is unchanged, as are justified no-semaphore/no-ptmx/no-ptc caches and upstream dynload_stub selection. No unsupported feature was forced to yes. Configured Makefile, pyconfig.h and config.log are retained. The object set remains exactly 180; optional unicodedata remains selected and is not reclassified to change the denominator.

`core-make.log` preserves genuine make -k compilation. `compile-commands.json`, `object-manifest.json` and `object-inventory.json` preserve every unit, command, result, first diagnostic, required APIs/headers, classification, ownership and repair. Independent unlimited-error compiles avoid attributing interleaved make diagnostics to the wrong unit. Fatal missing includes can still mask later dependencies; these are not a complete unresolved-symbol inventory. `Modules/getbuildinfo.o` is accepted only from executing its exact upstream recipe with /bin/sh so Git substitutions expand normally. Its initial extracted-argv diagnostic and blocked make-target retry are retained separately; the latter's prerequisite error is not attributed to getbuildinfo.

| Failed object | Classification | First actual final diagnostic |
|---|---|---|
| `Modules/_io/fileio.o` | Filesystem/platform | `Modules/_io/fileio.c:254:28: error: variable has incomplete type 'struct stat'` |
| `Modules/_localemodule.o` | libc/header | `Modules/_localemodule.c:488:14: error: use of undeclared identifier 'DAY_1'` |
| `Modules/_stat.o` | Filesystem/platform | `Modules/_stat.c:322:15: error: use of undeclared identifier 'S_IFDIR'` |
| `Modules/_threadmodule.o` | Signals | `Modules/_threadmodule.c:2052:18: error: use of undeclared identifier 'SIGINT'` |
| `Modules/faulthandler.o` | Signals | `Modules/faulthandler.c:78:6: error: use of undeclared identifier 'SIGFPE'` |
| `Modules/getpath.o` | Filesystem/platform | `Modules/getpath.c:186:21: error: variable has incomplete type 'struct stat'` |
| `Modules/main.o` | Filesystem/platform | `Modules/main.c:393:28: error: variable has incomplete type 'struct stat'` |
| `Modules/posixmodule.o` | Filesystem/platform | `Modules/posixmodule.c:2608:41: error: incomplete definition of type 'struct stat'` |
| `Modules/signalmodule.o` | Signals | `Modules/signalmodule.c:614:14: error: use of undeclared identifier 'SIGHUP'` |
| `Modules/timemodule.o` | Time/environment | `Modules/timemodule.c:478:13: error: incomplete definition of type 'struct tm'` |
| `Objects/obmalloc.o` | Filesystem/platform | `Objects/mimalloc/alloc.c:880:12: error: call to undeclared function 'realpath'; ISO C99 and later do not support implicit function declarations [-Wimplicit-function-declaration]` |
| `Python/bootstrap_hash.o` | Entropy | `Python/bootstrap_hash.c:105:28: error: use of undeclared identifier 'GRND_NONBLOCK'` |
| `Python/fileutils.o` | Filesystem/platform | `Python/fileutils.c:1236:1: error: conflicting types for '_Py_fstat_noraise'` |
| `Python/marshal.o` | Filesystem/platform | `Python/marshal.c:1606:28: error: variable has incomplete type 'struct stat'` |
| `Python/pylifecycle.o` | Time/environment | `Python/pylifecycle.c:266:9: error: call to undeclared function 'setenv'; ISO C99 and later do not support implicit function declarations [-Wimplicit-function-declaration]` |
| `Python/pytime.o` | Time/environment | `Python/pytime.c:534:28: error: incomplete definition of type 'struct timeval'` |
| `Python/sysmodule.o` | Unsupported kernel contract | `Python/sysmodule.c:2455:17: error: call to undeclared function 'getpid'; ISO C99 and later do not support implicit function declarations [-Wimplicit-function-declaration]` |

`core-serial.log` independently reproduces make -j1 python failure (exit 2). First serial make failure remains `Objects/mimalloc/alloc.c:880:12`: undeclared realpath. That object also diagnoses abort and missing sys/syscall.h; its full log is retained. No empty syscall header was created to suppress mimalloc's platform integration requirement. New API availability advances other diagnostics without resolving independent stat/time/signal contracts.

**Genuine interpreter static link: NOT_RUN.** Mandatory objects are absent. The prospective linker line in make's dry-run is not executed and is not a link attempt. `link-results.json` explicitly leaves linker command/map, unresolved/duplicate symbols, relocations, TLS model and ABI outcomes NOT_RUN. `archive-symbols.log` confirms exactly one definition of each added API and inherited _Exit/strncpy; this is archive evidence only. No diagnostic interpreter link, dummy symbols or object omissions were used.

**Native CPython startup, -V, -c and print: NOT_RUN.** No CPython executable exists. `native-smoke-results.json` records no stdout/stderr/exit observation and no actual Python output. Guest libc fixture strings are not interpreter output.

| CPUs | C | C++ | Lifecycle | Existing functional | All functional | TCP |
|---|---|---|---|---|---|---|
| 1 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 37/37 PASS | 40/40 PASS | NOT_RUN |
| 2 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 37/37 PASS | 40/40 PASS | PASS |
| 4 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 37/37 PASS | 40/40 PASS | PASS |

Native regression status: **PASS**. The original 73/21 C/C++ classification is retained. The existing 37 functional cases include 17 lifecycle cases, heap/ABI, TLS/once, immediate exit, condvar functional controls and PY-2's five fixtures. Three PY-3 cases make 40 total. The unchanged formatted-I/O fixture checks 1,780 host-oracle vectors in the guest per CPU; host oracle generation is not native execution. Additional guard/error probing runs on 2 CPUs. pthread/C++ header ABI, archive ownership/duplicate-symbol, vendor and historical ELF checks are separate target/static evidence. `validated-executions.json` binds commands to `validated-source.json`; final-source matching is checked after every command. TCP at 1 CPU is unsupported by the retained runner and is NOT_RUN.

The security review preserves process-group versus pthread exit, native VFS authority, descriptor ownership, immutable environment lifetime, errno, per-stream locking and C ABI boundaries. New conversions own only stack-local state; locale metadata is immutable; stream policy never adopts caller memory. No kernel, scheduler, IPC, capability or existing structure layout changed. No unbounded retry/poll loop was introduced. Conversion time remains proportional to caller input, and existing locale/stream lock progress limitations are unchanged. Condvar exact interleavings remain UNQUALIFIED; detached autonomous reclamation, retirement fault coverage and group-integrated security witnesses remain incomplete. Functional passes do not close these inherited findings.

`source-integrity.json` verifies incoming regular files except the ten authorized libc originals below, with no missing incoming regular files, kernel changes, CPython changes or PY-1/PY-2 evidence changes. `py3-production.patch` contains only PY-3 production deltas, including changes to already-dirty files, and reverses byte-for-byte in a temporary copy. An initial integrity parser wrongly counted baseline-excluded symlinks/directories as new files; its result is retained and the corrected check uses the baseline's regular-file coverage. The original broad rglob baseline traversal was stopped before production edits and rerun using the Git file list; the accepted baseline precedes every libc edit. Diff whitespace and focused rustfmt checks pass; inherited formatting in termination.rs was preserved instead of reformatted.

Production files:

* `sunlight-libc/include/locale.h`
* `sunlight-libc/include/stdio.h`
* `sunlight-libc/include/stdlib.h`
* `sunlight-libc/include/time.h`
* `sunlight-libc/include/unistd.h`
* `sunlight-libc/src/locale.rs`
* `sunlight-libc/src/stdio.rs`
* `sunlight-libc/src/termination.rs`
* `sunlight-libc/src/time.rs`
* `sunlight-libc/src/wchar.rs`

New report/evidence files are confined to this report and E; generated binaries remain under ignored target or /tmp paths. Main report: `docs/runtime/python-py3-libc-core-static-link.md`. Standalone patch: `docs/audit/probes/python/results-2026-11-14-py3/py3-production.patch`. Required JSON deliverables, raw compiler/configure/guest logs and validation drivers are in E.

Recommended PY-4 scope: separately authorize authoritative native metadata/canonical-path, process identity, required signal and nonblocking secure-entropy contracts; preserve existing permission/capability boundaries. In parallel milestone sequencing, close libc calendar/langinfo, mutable-environment ownership and genuine buffering contracts with focused native negative tests. Reattempt the identical complete object set, then execute a real static link only after required compilation succeeds, classify actual linker failures and finally supervise the actual native interpreter. No interpreter-readiness claim is made by this report.
