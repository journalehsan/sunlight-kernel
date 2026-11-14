# Native runtime R2A: headers, libm and CPython compile probe

Date: 2026-11-14. Revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, branch `main`.

**R2A COMPLETE for the authorized header/libm milestone. CPython is not built or operational.** All seven acceptance gates pass. CPython 3.13.0 now completes cross-configure with the actual native driver and matching build interpreter. Actual core compilation stops at unsupported pthread TLS/condition-variable types and APIs. No threading implementation, kernel or scheduler production code changed.

Evidence root (**E**):
`docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2a/`.
Authoritative final runs have `acceptance` in their names. Earlier iterations remain diagnostic evidence.

| Gate | Status | Evidence and boundary |
|---|---|---|
| G1 baseline and R1 locked build | PASS | `initial-source.json`, `baseline-locked-build.log`; R1 source/evidence inspected and existing changes preserved. |
| G2 standard headers | PASS | `acceptance-focused/focused-results.json`; C11/C++17 parsing, type sizes/alignments/signedness, all PRI/SCN macro types, native conversions, assertion success/disabled/failure and ctype behavior. |
| G3 targeted libm | PASS | 17,342 native reference vectors, errno/signgam and SSE domain/pole flag checks; double and float variants. Long-double ABI inspected and deliberately unsupported. |
| G4 existing regressions | PASS | `acceptance-suite/`: 73 C + 21 C++; `acceptance-cxx-abi/`, `acceptance-heap-abi-guest/`, `acceptance-tcp/`. |
| G5 CPython configure | PASS | `cross-acceptance-result.json`: exit 0, fresh directory, real toolchain, isolated R1 recognition patch and three evidenced negative cache answers. |
| G6 real compilation | PASS | `acceptance-compile-results.json`: five explicit object targets and `make -j1 python` attempted; all exit 2 with actual compiler diagnostics. Successful compilation is not the gate requirement. |
| G7 inventory and integrity | PASS | `cpython-compatibility-inventory.json`, `source-integrity-manifest.json`, `validation-summary.json`; no unexpected original-file changes or deletion. |

## Baseline, ownership and reproducibility

Read both previous runtime reports and inspected R1's machine summaries, source manifest, toolchain, CPython origin/environment, symbol probes and platform patches. Initial state already included the R1 three-version native-C lock repair plus untracked R1 evidence and runtime reports. These are user-owned baseline content, not R2A repairs. `initial-source.json` hashes tracked and nonignored untracked files before changes. No reset, clean, stash, checkout, commit or push was performed.

The literal existing build, `bash tools/build_native_c.sh`, passed before implementation, through acceptance-suite preparation, and after all validation (`final-locked-repeat.log`). It invokes:

```sh
bash tools/build_native_std.sh build --manifest-path tools/native-c/Cargo.toml \
  --target-dir "$PWD/target/native-c" --release --locked
```

This demonstrates repeated locked builds with the retained installed toolchain and existing build cache. It does not claim a clean-room, bit-reproducible artifact or a different nightly's compatibility.

`toolchain.json` records rustc 1.98.0-nightly revision `8954863c8`, Cargo 1.98 nightly, Clang/LLD 23.1.4, QEMU 11.1.2, full versions and target/driver/linker-script contents and hashes. C/C++ use static `x86_64-unknown-none-elf`, SysV LP64, SSE2, no red zone/PIC/PIE, freestanding Clang resource headers plus `sunlight-libc/include`, native startup/archive and linker script. No host libc or Linux personality is linked. The Rust JSON target remains unchanged. Guest runs use QEMU TCG, two CPUs, 2048 MiB, isolated images and the existing runner's output/exit/reaping checks.

The authoritative public headers belong to `sunlight-libc/include`; `tools/build_native_c.sh` symlinks them into its sysroot. Native exports come from `sunlight-libc` in `libsunlight_native_c.a`. R1 established genuine absent exports for atanh/frexp/lgamma/modf, rather than merely a missing separate `-lm`. Its provenance and this run's `archive-symbols.log` distinguish strong C wrappers from incidental weak compiler-builtins exports. CPython continues to use `--with-libm=` because math resides in the monolithic native archive.

## Header implementation and ABI

* `assert.h`: single evaluation, no evaluation under NDEBUG, reinclusion after NDEBUG changes, C11 static_assert alias, C++ inclusion, expression/file/line/function diagnostic, and callback-free abnormal process termination with status 134. Native SIGABRT delivery is unsupported; no abort API is fabricated. Failure never resumes execution. C and C++ failure fixtures verify stderr, no post-assert output and no atexit callback.
* `inttypes.h`: every signed/unsigned PRI and SCN family for fixed, least, fast, max and pointer integer types; intmax integration, imaxdiv_t, imaxabs, imaxdiv, strtoimax/strtoumax and wide equivalents. Conversion implements C11 base 2–36/autodetection, whitespace/signs, hex prefixes, end pointers, saturation/ERANGE, invalid-base EINVAL, and success preserving errno. INTMAX_MIN absolute value, invalid division and invalid pointers retain C's undefined-input boundary. SCN macros are usable format strings; this does not provide a scanf implementation.
* `sys/types.h`: one owner for ssize_t/off_t/time_t/clock_t (signed long, size/alignment 8), pid_t (signed int, 4/4), mode_t/uid_t/gid_t/nlink_t (unsigned int, 4/4), dev_t/ino_t (unsigned long, 8/8). These define a public native LP64 ABI, not a reinterpretation of the kernel's private Stat wire structure. time.h, unistd.h and sys/mman.h now consume those definitions. No pthread placeholder types or option/version claims were added.
* Transitive `ctype.h`: 12 byte-classification and two case-conversion exports. ASCII classes match the supported C and UTF-8 locales; multibyte characters are handled by wchar decoding, not byte ctype. EOF is handled correctly.
* Supporting headers: limits.h gains native SSIZE_MAX; errno.h gains EDOM, ERANGE and EBUSY; math.h gains tested additions, classification macros, and correctly typed existing log/ldexp exports used by CPython longobject.c. The older math subset has no general errno guarantee; a global math_errhandling contract is deliberately not advertised.

Compile-time ABI assertions cover sizes, alignment, signedness and intmax compatibility. Clang format checking validates all 154 PRI/SCN/type combinations separately in C11 and C++17, including native int_fast16_t == short. The runtime fixture validates integer boundaries, prefixes, no-conversion/end-pointer behavior, overflow, wide conversions, division and actual formatted output. `acceptance-focused` retains every command, source, object/link log and native result. No Linux host headers are used by these probes.

## libm reuse and numerical validation

Pinned Rust libm **0.2.16** algorithms were already available under the R1 native-std compiler-builtins source tree. They are now vendored under `sunlight-libc/vendor/libm`, with upstream license notices and per-file provenance retained. Numerical sources, build.rs and configure.rs are unchanged. Cargo.toml removes only the test-only registry no-panic dependency; the provenance note is new. `libm-vendor-integrity.json` and the final manifest verify the upstream file hashes. libm is an optional local dependency enabled only by native-c. Lock changes add that dependency and disambiguate existing registry libm users; they do not upgrade those users.

New strong C exports adapt the Rust tuple results to C output pointers and supply errno for atanh domain/poles and lgamma poles/finite overflow:
`atanh`, `atanhf`, `frexp`, `frexpf`, `modf`, `modff`, `lgamma`, `lgammaf`, `lgamma_r`, `lgammaf_r`, plus signgam. The reentrant variants do not update shared signgam; ordinary lgamma variants do. Shared signgam requires caller synchronization. Direct float reentrant C ABI execution is not separately qualified; its algorithm path is exercised through lgammaf and the inventory marks that distinction PARTIAL.

long double is **x87 extended, 64-bit mantissa, 16-byte storage/alignment**, not double. atanhl/frexpl/lgammal/modfl remain absent; no lossy wrappers or misleading declarations were introduced. Actual isolated native link failures are in `missing-symbol-links.json`. No observed core translation unit selects these variants.

The independent oracle is host Python math/glibc libm, not the vendored Rust library. Deterministic seed 20261114 generates hex-encoded expected results retained in `math-vectors.h`; the native target computes and compares results. `numerical-reference.json` records generation, counts and tolerances:

| Function | Native vectors | Acceptance |
|---|---:|---|
| atanh / atanhf | 1,820 each | at most 4 double/float ULP; double tiny/subnormal and zero exact; NaN/infinity classification separate |
| lgamma / lgammaf | 2,021 each | absolute error ≤ 8e-14 / 4e-6 times max(1, abs(reference)); zeros/classification exact; double reentrant signs checked where defined |
| frexp / frexpf | 2,415 each | exact fractions and finite exponents, including normalization of subnormals and signed zero |
| modf / modff | 2,415 each | exact fractional/integer output, signed fractional zero, infinity and NaN classification |

**All 17,342 vectors pass.** Inputs cover both signs, zero, smallest subnormal/normal values, finite magnitudes, infinities, quiet NaN, atanh boundaries/adjacent values/out-of-domain, lgamma negative intervals/poles/large values and modf large integral values. Additional checks demonstrate domain/pole/overflow errno, errno preservation on representative successes, signgam changes and SSE invalid/divide-by-zero flags for atanh. No public fenv API, alternate-rounding guarantee, underflow-errno guarantee, signaling-NaN qualification or full libm conformance is claimed. Existing log/ldexp declarations get bounded native value/ABI checks; their general semantics remain PARTIAL.

## Regression results and iterations

Final legacy results: **73/73 C and 21/21 C++ PASS**, individually retained in `acceptance-suite/results.csv` and JSON. No legacy case was skipped or excluded. Historical fixture provenance labels do not mean historical execution: these are fresh final-source executions. The existing C++ target checker passes exception/unwind, RTTI, static guard/init, archive ownership, required exports, ELF/frame and vendor-integrity checks. Its historical archive-free LINK_FAIL is an intentional negative control.

Supplemental heap/ABI prints `R1 heap and SysV ABI PASS`. TCP loopback prints `tcp-poll: PASS socket_live=0 listener_live=0 stream_live=0`; all poll-summary checks pass. This is native loopback, not external network validation.

Keep new cases separate: six focused guest scenarios (math, C headers, C++ headers, ctype, C assertion failure, C++ assertion failure) all PASS. Math contains the vectors above; ctype compares 257 valid inputs × 14 functions against host byte-classification results. Two format-type compile checks and their builds/links are additional checks, not legacy guest cases. The negative assertion cases correctly exit 134. An additional device probe demonstrated ENOENT for /dev/ptmx and /dev/ptc.

Retained early failures: an unnecessary nested Cargo workspace in the first vendor manifest blocked guest preparation and was removed; a C++ command incorrectly left `-x c++` active for archive inputs and was corrected to object compilation followed by linking; the initial math oracle incorrectly demanded a particular gamma sign at negative integer poles, where the sign is undefined. Its expected/result infinities already agreed. Corrected tests omit that undefined sign requirement and pass. These are not silently excluded legacy failures. Final acceptance runs include all corrections.

## CPython configure and actual compilation

Archive identity is unchanged from R1: upstream CPython **3.13.0**, SHA-256
`086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`.
A fresh extraction at `/tmp/sunlight-r2a-cpython/Python-3.13.0` was compared against the actual R1 source: only configure and config.sub differed from pristine, exactly matching R1's minimized recognition patch after application. `cpython-source.json` records this comparison. The patch is retained in E and touches only disposable CPython sources; no upstream changes are hidden. The matching R1 host build interpreter still reports 3.13.0 final.

Final build directory: `/tmp/sunlight-r2a-cpython/cross-acceptance`. Exact command/environment, Makefile, pyconfig.h, config.status, full stdout/stderr and config.log are retained with `cross-acceptance` prefixes. The command uses `--build=x86_64-pc-linux-gnu --host=x86_64-unknown-sunlightos --with-build-python=/tmp/sunlight-r1-cpython/build-host/python --disable-shared --without-ensurepip --with-pkg-config=no --with-libm=`, CC set to the native driver, CFLAGS=-O2, LDFLAGS=-static, native llvm-ar/ranlib and CONFIG_SITE=/dev/null. A separate --target is not needed for this Autoconf program build; the compiler's program ABI is the native SunlightOS target.

Three explicit cache answers are negative and evidenced: POSIX semaphores disabled because the header/implementation are absent (overriding upstream's optimistic cross default); /dev/ptmx and /dev/ptc absent, demonstrated in the native device probe. No unsupported pthread/shared-loading/fork/signal feature was forced to yes. `configure-cache.json` records all cache values and separates upstream cross-run assumptions, whose native semantics remain UNKNOWN, from explicit answers. A successful configure does not validate those runtime assumptions or promise optional module builds.

Initial R2A configure passed mandatory C99 math, then asked for /dev/ptmx cross evidence. After obtaining native evidence, fresh configure completed; it completed again on final source. Header checks now find inttypes.h and sys/types.h; the initial isolated real CPython unit compile found missing ctype.h, subsequently implemented and tested. Remaining unsupported optional libraries/modules are visible in config.log and remain separate from the first core blocker.

Actual `make -j1` attempts targeted Programs/python.o, Python/pyctype.o, Python/pymath.o, Python/thread.o, Objects/longobject.o and python. All failed reproducibly at real compilation. First serial target: **Programs/python.c**, including `Include/cpython/pythread.h:37`, reports **unknown type pthread_key_t**. thread.c additionally demonstrates missing pthread_condattr_t/pthread_cond_t and condition-variable declarations. No fake type or function was added. Earlier isolated compile probes used only the actual incomplete configure definitions, clearly labeled; final diagnostics use the genuine completed target pyconfig.h and generated Makefile.

Other observed findings: missing strerror declaration and actual link symbol (**libc API gap**); incomplete/conflicting struct timespec visibility in the pthread-selected backend (**CPython platform integration / header integration**, not a demonstrated wire ABI mismatch); longobject's static keyword initializer diagnostics (**UNKNOWN**, possibly secondary to earlier missing pthread types). Missing SSIZE_MAX, EBUSY and log/ldexp declarations found in earlier real unit attempts were repaired within header scope and disappear from final diagnostics. No interpreter link was reached, so there is no interpreter unresolved-symbol list to invent. Isolated missing-symbol links are labeled separately. Target interpreter execution is **NOT_RUN**.

## Inventory, integrity and next milestone

`cpython-compatibility-inventory.json` carries forward all R1 requirements and adds the relevant new headers/types/math and actual unit diagnostics (505 entries, including historical duplicates). Each entry records compile, link, native execution, implementation, confidence, remaining blocker and evidence. Historical R1 evidence is retained separately. Header-only R1 VERIFIED classifications are conservatively PARTIAL unless native R2A behavior now supports the stated contract. VERIFIED means bounded demonstrated native behavior, not exhaustive standards coverage. Optional dependencies and source-inferred requirements are not represented as actual first-core failures.

Original-source differences are limited to the libc manifest/module registration, the scoped header changes and two dependency locks. New production files are four public headers, three Rust support modules and the pinned libm vendor tree. The existing R1 lock repair is preserved. All prior R1 reports/evidence and kernel/scheduler/pthread production files remain byte-identical. Two historical `.threading-evidence/.../Cargo.lock` symlinks resolve to the root Cargo.lock: their followed-content hashes change with the authorized dependency lock addition, but the symlink targets and link objects are unchanged. This alias effect is explicit in the integrity manifest, not an unrelated evidence edit. Generated binaries/images stay in ignored build/evidence areas. No commit or push.

Recommended **R2B**: implement and qualify actual pthread key/TLS destructor and process-private condvar/timed-wait contracts, with clock/lifetime/race tests and an explicit once policy if selected; then repeat the same core compilation. Resolve the independently observed strerror and time-header/platform integration gaps without hiding unsupported APIs. Reassess secondary compiler diagnostics after threading types are real. Continue toward a minimal static core link with preserved unresolved-symbol diagnostics, then attempt native interpreter execution only when linking succeeds. Shared modules, optional third-party libraries and pip remain later scope.
