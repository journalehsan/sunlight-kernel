# PY-2: native libc compatibility and first static-link prerequisites

Date: 2026-11-14. **Verdict: PARTIAL. No interpreter linked or executed. No Python output.**

Evidence root (**E**): `docs/audit/probes/python/results-2026-11-14-py2/`.
Branch `main`, HEAD `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`. The incoming dirty worktree, including user kernel/runtime changes and untracked files, is preserved. No kernel edits, commit, push, reset, clean, stash, or checkout.

## Gates and evidence boundaries

| Gate | Result | Meaning |
|---|---|---|
| G1 | PASS | Locked build, exact source/build interpreter, fresh configure, original 180-object source set and 179 attempted units reproduced |
| G2 | PASS | Required memory/stream formatting foundation passes bounded native/oracle tests; unsupported formats remain explicit |
| G3 | PARTIAL | Unbuffered stream/fd contracts tested; POSIX stat metadata and buffered configuration remain missing |
| G4 | PARTIAL | Shared formatter, wide helpers, sorting, immutable environment and math declarations repaired; mandatory platform/libc gaps remain |
| G5 | PASS for complete attempt/accounting | Compilation itself FAIL: 160 passed, 19 failed, one unattempted |
| G6 | NOT_RUN | Required object compilation fails, so real static-link prerequisites are absent |
| G7 | NOT_RUN | No linked SunlightOS interpreter |
| G8 | PASS | Final-source regressions below |
| G9 | PASS | Baseline recovery hashes, source inventory and inherited qualifications recorded |

The remaining mandatory functionality makes this **PY-2 PARTIAL**, independently of the conditional NOT_RUN phases. Cross-configure, individual C objects, fixture links, interpreter link, runtime initialization, and Python execution are separate results.

## Baseline and source identity

`source-baseline.json` records revision, branch, complete tracked/untracked Git status and 77,557 incoming file hashes before production edits. `baseline.py`, `locked-build.log`, `build-python.log`, `configure-manifest.json`, `configure.log`, `core-before.log` and `core-before-serial.log` preserve commands and results. The locked build command is:

```sh
bash tools/build_native_std.sh build --manifest-path tools/native-c/Cargo.toml \
  --target-dir "$PWD/target/native-c" --release --locked
```

The exact local Python 3.13.0 archive SHA-256 is `086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`. `cpython-identity.json` confirms the retained archive and matching CPython 3.13.0 final build interpreter `/tmp/sunlight-r1-cpython/build-host/python`; it is a Linux build tool, not guest Python. `toolchain.json` records installed tool versions. This is a locked installed-toolchain rebuild with existing caches, not clean-room or bit-identical reproducibility.

CPython source is `/tmp/sunlight-py1-cpython/Python-3.13.0`; the separate build is `/tmp/sunlight-py2-cpython/cross-static`. PY-1's minimized configure/config.sub recognition patch and unchanged `Setup.local` are retained. No CPython edits or source removals were made. Dynamic loading remains upstream `dynload_stub.o`; Linux libraries, permissive undefined-symbol flags and replacement Python output are never used.

The before inventory reproduces **180 configured objects, 179 attempted, 138 PASS, 41 FAIL, one NOT_RUN**. The first serial error remains undeclared `sprintf` in `Parser/string_parser.c`. All 179 units have independent diagnostics with Clang's error limit removed; the original two multiline commands were initially missed by the parser, then recovered. Their isolated diagnostics use SHA-256-verified pre-edit headers reconstructed in `/tmp/sunlight-py2-baseline-headers`. This reconstruction is explicitly recorded, not misrepresented as an earlier invocation. `before-*.d` records transitive preprocessing dependencies. Compiler diagnostics are not unresolved-symbol/link evidence; the missing-export field before edits is NOT_RUN because a fresh pre-edit archive symbol scan was not captured.

Each inventory row records source/object, command, attempt/result, first error, full diagnostics, missing headers/declarations/types/constants, root-cause families, mandatory/optional selection, and dependency evidence. Selected `unicodedata` is optional to minimal CPython but remains in the unchanged denominator; mandatory bootstrap modules remain included.

## libc repairs

* Formatted output: `snprintf`, `vsnprintf`, `sprintf`, `vsprintf`.
* Stream adoption/pushback: `fdopen`, `ungetc`.
* Native fd/path adapters: `open`, `fcntl`, `unlink`.
* Wide strings: `wcslen`, `wcscpy`, `wcsncpy`, `wcscmp`, `wcsncmp`, `wcschr`, `wcsrchr`, `wmemchr`, `wcstok`.
* LP64 conversions: `strtoul`, `wcstol`.
* Startup environment: `getenv`.
* Sorting: `qsort`.

The existing `printf/fprintf/vprintf/vfprintf` engine now shares one sink-aware renderer with memory output. `snprintf` stores at most capacity minus one bytes, terminates nonzero-capacity output, and counts the full required length. Size zero permits a NULL destination. Padding beyond memory capacity is counted without a proportional loop. `sprintf` writes directly under the standard caller-capacity precondition; no guessed whole-output temporary or memory-safety claim is made.

The shared engine retains integer flags, signed/unsigned conversions and native LP64 length modifiers; it adds binary64 f/F/e/E/g/G and wide lc/ls. Decimal conversion uses allocation-free Rust core formatting and a fixed conversion workspace bounded from binary64's decimal expansion (309 integral digits and at most 1074 fractional digits), with requested trailing precision streamed separately. This workspace is not a whole-output sprintf buffer. Width, signs, alternate form, precision, exponent normalization and nonfinite values are handled explicitly. The existing C numeric locale is used; LC_CTYPE provides ASCII or strict UTF-8 for wide output. Byte precision never splits a wide character. Unsupported hexadecimal float, long double, positional formats and `%n` return ENOSYS; count overflow returns EOVERFLOW; invalid wide scalars/encoding return EILSEQ. No fenv, alternate rounding or signaling-NaN qualification is claimed.

`fdopen` queries native syscall 49 flags, validates access compatibility, allocates before append mutation, enables real native append and transfers ownership only on success. It never opens/truncates the adopted descriptor. Failure leaves the descriptor owned by the caller. A single pushback slot is protected by the existing stream lock; reads consume it, seek accounts for its logical position and successful seek discards it. All streams remain unbuffered; fflush is not fsync. Concurrent fclose/use remains an invalid caller lifetime.

`open/fcntl/unlink` wrap existing native operations and preserve errors; unsupported commands/flags are rejected. No kernel interface changes. `BUFSIZ=8192` is an application block-size recommendation, not a buffering claim. errno additions include EOVERFLOW, the EWOULDBLOCK/EAGAIN alias and reserved standard error names; these do not advertise signal/device semantics.

Wide-string helpers operate on native signed 32-bit wchar_t and preserve code units without claiming locale collation or complete Unicode locale support. Numeric conversions reuse the existing intmax parser. `getenv` exposes the existing immutable startup environment without reallocating or changing its Rust static-reference lifetime. Mutable environment operations remain absent. qsort is allocation-free heapsort with bounded stack use; it does not promise stable ordering.

Eleven declarations expose real pre-existing native compiler-builtins symbols: atan2, ceil, copysign, cos, exp, floor, fmod, hypot, pow, round and sin. `archive-symbols.log` and the guest ABI/value fixture distinguish declarations from actual symbols. Their older errno/precision qualifications remain; no long-double narrowing or new full-libm claim is introduced.

## Validation and preserved failures

`formatted-io-validation.json` records **1,780 independent host-libc binary64 oracle cases per CPU** plus integer boundaries/modifiers, empty output, zero capacity, truncation/canaries, va_list wrappers, 100,000-byte required output, overflow, C++ consumption and explicit unsupported errors. Wide ASCII/UTF-8 tests cover error handling, multibyte byte precision and both alignments. `make-format-tests.py` records the deterministic seed, format list and host libc identity.

`file-stream-validation.json` covers empty/regular files, EOF versus errors, partial fread elements, 2048-byte transfers, mixed read/write, ungetc/seek/tell, failed/successful fd adoption, nontruncating w adoption, real append, descriptor ownership and four concurrent writers producing 200 intact records. The retained runner enforces real guest file/page/handle/name cleanup. Buffered seek interactions and injected mid-transfer short-I/O failures remain NOT_RUN; no buffered API was added.

Early attempts are retained: a fixture had an incorrect expected integer-output length (21 versus 18); a named test file omitted unlink and tripped the resource gate; a debug invocation used unsupported --elf syntax; the later wide formatter really emitted padding on both sides, demonstrated by `wide-debug-v2/` returning 13 bytes for an 8-byte field. The padding condition was corrected and left/right regression controls added. These observations are not replaced by later successes.

Only **validated-*** runs are final acceptance. Earlier final-/verified-/acceptance-/qualified- series overlap later source/header fixes, include the wide failure, or were cancelled as redundant driver series; their logs and source-match receipts remain diagnostic evidence. `intermediate-cancellations.json` records stopping only owned obsolete orchestration drivers through verified process handles; active child guest attempts were allowed to finish. There are no inflated timeouts or weakened runner resource checks.

## Final-source native regressions

| CPUs | C | C++ | Lifecycle | Functional cases | TCP |
|---|---|---|---|---|---|
| 1 | 73/73 | 21/21 | 17/17 | 37/37 | NOT_RUN |
| 2 | 73/73 | 21/21 | 17/17 | 37/37 | PASS |
| 4 | 73/73 | 21/21 | 17/17 | 37/37 | PASS |

The C/C++ split uses the retained suite's original 73/21 classification, including both historical and native cpp-runtime controls. Functional cases include four TLS/once cases, heap/ABI, condvar functional/timeout tests, string/guard and immediate group-exit controls, the 17 lifecycle cases and five new PY-2 cases. The legacy suite retains libm numerical tests. Host/public ABI, C++ exceptions/RTTI/static initialization and vendor checks have separate command results in `native-regressions.json`. Each accepted command is bound to `validated-source.json`; locked build/repeat also pass on this source. TCP 1 CPU is unsupported and NOT_RUN.

Exact condvar registration/park, winning signal/timeout and competing destroy schedules remain **UNQUALIFIED**. Functional/stress passes do not establish those schedules. Initial-thread group authority, retirement proof obligations and autonomous detached reclamation remain incomplete; this milestone neither changes nor closes them. Stream locking and fd ownership changes introduce no kernel, IPC, address-space or capability changes.

## Final configured compilation and remaining causes

| State | Configured | Attempted | PASS | FAIL | Unattempted |
|---|---|---|---|---|---|
| Before | 180 | 179 | 138 | 41 | Modules/getbuildinfo.o |
| Final | 180 | 179 | 160 | 19 | Modules/getbuildinfo.o |

The source denominator remains unchanged. Final configure and full `make -k -j4 -W .../Include/Python.h python`, serial first-error reproduction, and independent unlimited-diagnostic compiles are retained under `configure-qualified-*`, `core-qualified-*` and `qualified-*.o.log`. `cpython-compilation-matrix.json` is the authoritative per-unit inventory. Source hashes and explicit dependency manifests prevent treating failed objects as independent API defects.

First remaining serial error:

```text
/tmp/sunlight-py1-cpython/Python-3.13.0/Objects/mimalloc/alloc.c:880:12: error: call to undeclared function 'realpath'; ISO C99 and later do not support implicit function declarations [-Wimplicit-function-declaration]
```

Root-cause family counts below overlap; they are blocked units per family, not independent defect counts:

| Family | Failed units affected |
|---|---|
| Formatted I/O | 0 |
| File streams | 1 |
| Filesystem/stat | 11 |
| String and memory | 0 |
| Wide characters and Unicode support | 1 |
| Environment | 3 |
| Time and locale | 5 |
| Signals | 5 |
| Math | 0 |
| Platform and ABI | 3 |
| Compiler/toolchain | 0 |
| Other or UNKNOWN | 9 |

`remaining-blockers.json` separates remaining libc work from owning-layer blockers. Calendar/timeval/tm/timezone conversion, localeconv/lconv/langinfo data, bulk multibyte conversions, mutable environment, buffering, canonical paths and other observed adapters remain missing. They are not represented as implemented or kernel-blocked merely because this milestone leaves them open.

Essential native blockers include POSIX stat metadata (native wire exposes only size/uid/gid/mode/file_type/nlinks), broader signal semantics, nonblocking entropy (native getrandom intentionally accepts flags zero only), and native group process identity (SYS_GETPID returns native worker process.pid, with a tgid special case only for Linux). Supplying guessed stat fields, asynchronous-signal constants, GRND_NONBLOCK mapped to private noncrypto behavior, or a getpid wrapper returning a worker identity would conceal real incompatibility. These portions stop at the owning-layer boundary; no unauthorized kernel repair is attempted.

## Link, execution, integrity and next milestone

**Actual static interpreter link: NOT_RUN.** Required core objects are absent. `static-link-results.json` leaves link command/order/startup objects, unresolved/duplicate symbols, relocations, TLS/ABI, format and entry metadata unavailable. The native archive symbol scan is not a substitute link attempt.

**Native CPython execution: NOT_RUN. Actual Python stdout: none.** There is no interpreter binary to validate, initialize, evaluate, print or cleanly exit. `native-execution-results.json` keeps these prerequisites explicit. No launcher output or libc fixture is counted as Python.

`source-integrity.json` verifies all incoming original content except exactly **12 authorized libc originals**, with all 12 recoverable to the exact baseline hashes by reversing `py2-production.patch`. New production files are `sunlight-libc/include/fcntl.h`, `sunlight-libc/src/c_file.rs`, `sunlight-libc/src/wide_string.rs` and `sunlight-libc/src/sort.rs`. No incoming file is missing; no kernel file or CPython source changed. Existing user modifications and previous evidence are preserved. Generated binaries stay ignored. Diff checks pass; focused rustfmt passes with module reordering disabled to preserve the incoming module order (the default check exposes pre-existing ordering differences).

Recommended **PY-3: native platform contracts and remaining mandatory libc closure**. Resolve stat metadata, process identity, signals and nonblocking entropy through separately authorized owning-layer work; implement the remaining observed libc contracts without altering CPython, repeat the same complete source set, then attempt the genuine static link and first Python statement only when prerequisites pass. Preserve inherited concurrency/security qualifications.
