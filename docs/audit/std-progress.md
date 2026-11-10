> **Phase 6 update (2026-11-10):** the original API suite is now **4 build PASS /
> 14 BUILD_FAIL**. The 21 historical executable cells have **7 guest PASS /
> 0 guest FAIL / 14 guest NOT_RUN**. Unchanged `getrandom.c` passes on 2 and
> 4 CPUs, backed by the real OS random service: independent service counters
> increase by exactly one request and 16 bytes around that probe. Four new
> random contracts and two counter observers pass separately on each CPU count.
> Earlier sections and raw results retain their original snapshots.
> BUILD_FAIL means no executable, not a guest failure.

# Native std improvement — 2026-11-10

## Audit interpretation

`probes/results/results.csv` contains 18 probes per configuration. Native C
builds fail 18/18. Helios builds pass 17/18 (C++ fails); 16 host executions
pass and static `dlopen` fails. All 54 guest cells remain UNKNOWN. A Helios
binary running on Linux verifies its musl build, not SunlightOS syscall support.
No CPython interpreter has been built or run by this change.

Rust std and the C runtime are separate acceptance targets. Improving the Rust
overlay does not provide CPython's headers, startup objects, libm or pthread ABI.
The next Python milestone remains a reproducible native C sysroot with small
compile/link probes, followed by isolated guest execution and a pinned CPython
configure/link report. Preserve actual failed checks rather than forcing them
to success in a configure cache.

## Implemented: futex wake error handling

The source plan's item 3 is implemented directly in the canonical overlay:
`tools/native-std/overlay/pal/sunlightos/futex.rs`. Both wake operations now
panic on negative ABI results, matching wait's existing failure policy.
Previously wake returned false on errors and wake_all discarded errors.
Zero and positive counts retain their existing behavior. The runtime adapter
and syscall numbering are unchanged.

ABI review: `sl_std_futex` in `sunlight-libc/src/std_runtime.rs` returns the
native syscall result as i32. `sys_native_futex` in
`kernel/src/arch/x86_64/syscall.rs` returns a wake count for operation 1 and
negative raw failure codes for invalid memory/operations or an exiting process.
The std wake interfaces cannot return an I/O error; fail-fast prevents an error
from being silently interpreted as an ordinary no-waiter result. This is not a
recovery mechanism and does not prove synchronization correctness in the guest.

The standalone host harness compiles the actual overlay with injected clock
and futex functions. It tests zero/positive counts, negative results including
i32::MIN, wake-one/wake-all arguments, and wait completion/timeout behavior.
It issues no native syscall and uses thread-local injection for parallel tests.

```sh
rustc --edition=2021 --test tools/native-std/tests/futex.rs -o /tmp/sunlight-native-futex-tests
/tmp/sunlight-native-futex-tests
bash tools/build_native_std.sh check --manifest-path tools/native-std-probe/Cargo.toml --release
```

Host result: **3 passed, 0 failed**. Native target check: **PASS**, including
the rebuilt pinned std and native probe (47.59 seconds). It emitted an existing
unused-field warning for `ChronosManifest.terminal` in `sunlight-libc/src/sun_exec.rs`.
Guest wake fault injection, multi-waiter behavior, pthread
and Python GIL readiness remain **UNKNOWN**.

## Remaining priorities

* `isatty` still guesses from descriptor numbers. Native `fstat` rejects
  non-VFS descriptors and cannot distinguish a valid console from a pipe.
  Add an authoritative descriptor-kind query before claiming correct tty
  detection; do not replace one guess with another.
* Unify Rust/C environment storage for PYTHONHOME/PYTHONPATH.
* Establish native C headers, exported symbols and startup/TLS contracts;
  test actual C linking before extending Python module claims.
* Validate threads/TLS and filesystem import fixtures in an isolated guest.

`std-plan.md` and `std-readiness.md` describe the earlier source snapshot;
this progress record supersedes their futex wake-error status only.

## Native C foundation, Phase 1 — 2026-11-10

This follow-up verifies the earlier observations against the current worktree.
It does not change the futex policy or attempt a CPython configure/build.
The original `probes/results/results.csv` and all its raw logs are preserved.
There are **17 C sources and one C++ source**, not 18 C sources.

Evidence for the final implementation is under
[`probes/native-foundation/results-2026-11-10-final/`](probes/native-foundation/results-2026-11-10-final/).
The earlier `results-2026-11-10/` is a separate initial attempt, before the
driver's diagnostic handling and ELF checks were tightened; it is retained.
Both directories ignore generated objects/ELFs and retain command logs/CSVs.

### Verified first errors and their stages

The original invocation, retained in the old logs and reproduced by the new
foundation runner, used clang/clang++ targeting
`x86_64-unknown-none-elf`, `-nostdinc`, clang resource headers plus
`sunlight-libc/include`, and `-nostdlib -fuse-ld=lld`. It supplies neither a
native runtime archive/startup object nor `services/user-space.ld`.
`NATIVE_CC`/`NATIVE_CXX` were optional overrides, not an installed sysroot.
The previous Rust sysroot at `target/native-std/sysroot` stages Rust sources
and libraries; it is not a C include/lib sysroot.

Fresh reproduction in `before.csv` and `before-<probe>.log` matches all 18
historical failures. The first meaningful errors are:

| Probe | Stage | First error |
| --- | --- | --- |
| atexit.c | preprocessing | `stdlib.h` not found |
| clock.c | preprocessing | `time.h` not found |
| dirent.c | preprocessing | `dirent.h` not found |
| dlfcn.c | preprocessing | `dlfcn.h` not found |
| getrandom.c | preprocessing | `sys/random.h` not found |
| langinfo.c | preprocessing | `langinfo.h` not found |
| locale.c | preprocessing | `locale.h` not found |
| math.c | preprocessing | `math.h` not found |
| mmap.c | preprocessing | `sys/mman.h` not found |
| poll_select.c | preprocessing | `poll.h` not found |
| pthread.c | preprocessing | `pthread.h` not found |
| signal.c | preprocessing | `signal.h` not found |
| socket.c | preprocessing | `sys/socket.h` not found |
| stdio.c | preprocessing | `stdio.h` not found |
| termios.c | preprocessing | `termios.h` not found |
| wchar.c | preprocessing | `wchar.h` not found |
| cpp-runtime.cpp | preprocessing | `iostream` not found |
| string.c | linking | `ld.lld: error: undefined symbol: strcmp` |

Thus a single compiler failure does **not** explain the 18 results. There were
16 missing C headers, one missing C++ header, and one unresolved-symbol link.
No observed baseline error was in C compilation or assembly. Stages after
failed preprocessing were not reached. No baseline guest execution occurred.

`strcmp` already has a strong native C export in
`sunlight-libc/src/string.rs:175` and a declaration in
`sunlight-libc/include/string.h`. It was absent from the **link inputs**, not
absent from libc source. An additional header-free `main` with the old flags
returns linker status 0 but warns `cannot find entry symbol _start` and has
entry address **0** (`before-entry-link.log`, `before-entry-elf.log`). Such a
link is not accepted as a usable native executable by the new ELF checks.

### Canonical ABI and focused fix

Before editing, the following existing contracts were inspected:

* `sunlight-libc/src/crt0.rs` documents startup but supplies argv helpers,
  not a standalone C startup object. The actual canonical `_start` is in
  `sunlight-libc/src/std_runtime.rs:21`: it reads argc/argv/envp from the stack,
  aligns RSP, initializes the existing TLS/environment, calls main, and exits.
  The common stack is authoritative because legacy service spawn repurposes
  registers. `kernel/src/process/spawn.rs:533` constructs that aligned stack.
* `std_runtime.rs:145` exports `sl_std_write(u32, pointer, usize) -> isize`;
  negative results are encoded errors. `std_runtime.rs:306` exports group exit
  through native syscall 157. Ordinary Rust `write`/`exit` names in `lib.rs`
  are not unmangled POSIX C exports. `runtime-symbols.log` records the selected
  archive symbols, including absent POSIX write/exit and stdio exports.
* `services/user-space.ld` specifies `ENTRY(_start)` and the 0x400000 image
  base. `tools/targets/x86_64-sunlightos.json` specifies static System V x86-64,
  x87/SSE2, no red zone and aborting panic. The kernel target remains separate.

The source-level fix supplies the missing C build integration, rather than
another implementation of strcmp or startup:

* `tools/native-c/{Cargo.toml,Cargo.lock,src/lib.rs}` packages the canonical
  `sunlight-libc` with `native-std` as a static archive. The only local Rust
  behavior is an aborting panic handler using existing native group exit.
* `tools/build_native_c.sh` reuses `tools/build_native_std.sh` and its pinned
  overlay/target. It publishes `target/native-c/sysroot/lib/libsunlight_native_c.a`
  and an `include` symlink to the authoritative `sunlight-libc/include` tree.
  Rust build outputs remain under `target/native-c/x86_64-sunlightos/`.
* `tools/native-c/cc.py` uses clang builtins and canonical headers, static bare
  ELF compilation with the target's CPU/red-zone conventions, and the existing
  linker script/archive. Executable links force extraction of `_start` and
  reject undefined symbols and linker warnings. No host CRT/libc is selected.
  `native-driver-invocation.log` contains clang's full cc1 and lld commands.
* `sunlight-libc/include/sunlight_runtime.h` declares the two existing native
  write/exit exports, without POSIX adapters or replacement standard headers.
  The three `probes/native-foundation/*.c` programs isolate argument entry,
  stderr write, and explicit exit. `compiler.c` remains the freestanding probe.
* `probes/native-foundation/run.py` records each stage independently and
  validates ET_EXEC/EM_X86_64, native branding, entry equal to `_start`, no
  PT_INTERP and no undefined symbols. Its original API cases disable builtins
  so the string probe must actually resolve the canonical libc calls.
* `probes/run.py` now defaults to the prepared driver for native C and adds
  `--targets` / `--out` so the original suite can be rerun without overwriting
  the historical results. Its native C++ configuration remains separate.

Toolchain: clang **23.1.4**, LLD **23.1.4**; Rust
**1.98.0-nightly (8954863c8 2026-06-05)**, LLVM **22.1.6**, required by
`tools/native-std/prepare.py`. C LLVM triple is `x86_64-unknown-none-elf`;
Rust target is the repository JSON with `os=sunlightos`. These are native
ELFs with OSABI 0 and no HLNX01 stamp; their Linux-host executability is not
tested. The driver does not provide a C++ runtime.

### Reproduction, results and limits

Run from the repository root; use a new output path for each retained attempt:

```sh
bash tools/build_native_c.sh
python3 tools/native-c/cc.py -std=c11 -c docs/audit/probes/compiler.c -o /tmp/native-c-compiler.o
python3 tools/native-c/cc.py -std=c11 docs/audit/probes/native-foundation/entry.c -o /tmp/native-c-entry
python3 docs/audit/probes/native-foundation/run.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-final
python3 docs/audit/probes/run.py --targets native \
  --out docs/audit/probes/native-foundation/results-2026-11-10-integration
bash tools/test-mem-string-proof.sh
rustc --edition=2021 --test tools/native-std/tests/futex.rs -o /tmp/sunlight-native-futex-tests
/tmp/sunlight-native-futex-tests
bash tools/build_native_std.sh check --manifest-path tools/native-std-probe/Cargo.toml --release
```

All exact stage argv, outputs and exit codes are retained in the final
evidence directory. Both direct commands above return 0, recorded in
`direct-compile-link.log`. `after-compiler-{preprocess,compile,assemble}.log` records
the freestanding translation unit; `after-entry-*`, `after-write-*`,
`after-exit-*` record the three executable builds; `after-string-*` records
the previously failing string probe. The entry executable is ET_EXEC with
`e_entry = _start = 0x400080`, using the canonical stack reader. Disassembly
shows the call into C main and native syscall 157 after its return; this is
static inspection, not execution evidence.

The original runner's separate `results-2026-11-10-integration/results.csv`
also reports 1 build PASS / 17 FAIL, host_run NOT_RUN and guest_run UNKNOWN
for all 18 rows, using its new default native C driver.

| Probe set | Before build PASS / FAIL | After build PASS / FAIL | Guest |
| --- | --- | --- | --- |
| Original 17 C API probes | 0 / 17 | **1 / 16** | 17 UNKNOWN |
| Original C++ probe | 0 / 1 | **0 / 1** | 1 UNKNOWN |
| Foundation: compiler-only + entry/write/exit | compiler-only already PASS; executable contract unavailable | **4 / 0** | 3 UNKNOWN; compile-only N/A |

The four foundation rows pass preprocessing, compilation and assembly; all
three executable rows also pass native linking and ELF checks. The original
string row now passes every build stage. All other original rows still fail
preprocessing, with later stages NOT_RUN. `after.csv` keeps these failures.
There are **four built ELFs** with execution UNKNOWN; across all executable
probe cells there are **21 UNKNOWN**, including 17 cases without a built ELF.
There are **zero guest PASS and zero guest FAIL**.

No guest command was issued. `NATIVE_RUNNER` and `HELIOS_RUNNER` were unset.
The documented `tools/test_native_std_foundation.sh` /
`tools/native_std_boot_gate.py` boot an isolated fixed Rust FP/parser image;
`kernel/src/main.rs:283` embeds that Rust payload, and the gate accepts no
arbitrary ELF path. It is not a usable loader for these C probes. No existing
VM/disk was contacted. `guest.log` records this limitation. A future guest
run must observe entry/write exit 0, the exact stderr marker, and explicit
exit status **23**, rather than treating status 23 as a generic test failure.

Regressions: `test-mem-string-proof.sh` **PASS** (24 host tests, native
code-generation checks, C relocatable link and C++ header compilation);
futex harness **3 passed, 0 failed**; pinned native Rust std probe check
**PASS**. Their raw logs are `regression-*.log`. The existing unused
`ChronosManifest.terminal` warning remains. Cargo refreshed the previously
clean native-std probe lockfile during its check; that incidental change was
restored. No kernel boot gate was run for this build-tooling-only patch.
The header/startup link checks make no pthread, compiled C TLS, libm, dlopen,
isatty or Python-readiness claim.

**Single Phase 2 blocker:** establish the missing native C standard-header
and matching export contract, beginning with **`stdio.h` / the stdio probe**.
The first failure is still `stdio.h` not found; the archive inventory also
shows its requested `tmpfile/fputs/rewind/fgetc/fclose` symbols absent. Adding
declarations alone cannot make that probe link. Guest execution of Phase 1
ELFs remains a separate UNKNOWN acceptance gate.

## Native C foundation, Phase 2: syscall-backed stdio — 2026-11-10

The Phase 1 blocker above is resolved for basic standard streams and named
files: `stdio.h` exists and the canonical native archive exports its matching
operations. This is a minimal stdio implementation, not a complete C library.
The historical `tmpfile` probe now **links**, but its execution is still
**UNSUPPORTED**, as detailed below. No CPython configure was attempted.

Final-source evidence:
[`results-2026-11-10-stdio-complete/`](probes/native-foundation/results-2026-11-10-stdio-complete/).
The original API suite's final-source results are in
[`results-2026-11-10-stdio-complete-integration/`](probes/native-foundation/results-2026-11-10-stdio-complete-integration/).
All generated archives, objects, host executables and native ELFs in these
folders are ignored; command logs and CSVs remain reviewable.

### Implementation and changed files

| Files | Implementation |
| --- | --- |
| `sunlight-libc/src/lib.rs` | Enable the pinned compiler's `c_variadic` feature and canonical stdio module only for `native-std` on `target_os=sunlightos`. Existing Rust startup and syscall adapters are unchanged. |
| `sunlight-libc/src/stdio.rs` | Opaque, per-stream locked FILE objects; exported `stdin`, `stdout`, `stderr` map to descriptors **0, 1, 2**. Implements `fopen/fclose/fread/fwrite/fflush/fileno`, `fgetc/getc/getchar`, `fputc/putc/putchar`, `fputs/puts/fgets`, `feof/ferror/clearerr`, `fseek/ftell/rewind`, and formatted-output entry points. |
| `sunlight-libc/src/stdio/format.rs` | Allocation-free `printf/fprintf/vprintf/vfprintf` formatter using the actual SysV C varargs ABI. Supports `c/s/d/i/u/o/x/X/p/%%`, flags, field width, precision (including `*`), and `hh/h/l/ll/j/z/t`. |
| `sunlight-libc/include/stdio.h`, `sunlight-libc/include/errno.h` | C/C++ declarations, opaque FILE, EOF/seek constants, minimal errno interface matching the existing TLS-backed `__errno_location`. Headers document unbuffered operation and unsupported functions/conversions. |
| `tools/native-c/tests/stdio.c` | Dedicated C integration probe with exact output/count checks, stdin, varargs forwarding, named-file round trip, append/seek/EOF/error behavior and explicit unsupported-tmpfile check. |
| `tools/native-c/tests/stdio-host.rs`, `stdio-tests.rs` | Compile the actual canonical stdio source with host descriptor/allocator/errno adapters; fault injection is confined to tests. Includes concurrent formatted calls on one stream. |
| `tools/native-c/tests/run_stdio.py` | Build/archive, Rust host tests, C host-adapter execution, native C links, ELF/symbol/disassembly checks and C++ header compilation. Each run requires a new evidence directory. |
| `docs/audit/std-progress.md`, new evidence folders | This implementation and validation record. |

Backing uses the existing checked libc wrappers, **not Linux syscall numbers**:
`fopen` calls `open_with_flags_mode` (native **40**); `fclose` calls `close`
(**41**); input calls `read` (**42**); output calls `write` (**43**);
seek/tell call `lseek` (**44**). Allocated FILE objects use the existing native
malloc/free. The stream lock yields through the existing native yield wrapper
(**21**) under contention. Standard descriptors are inherited from the loader;
stdio creates no replacement console and performs no tty guess.

All streams are deliberately **unbuffered**, including stdout and stderr.
`fflush(stream)` checks that the stream is open, then returns 0 because no
userspace data is pending; `fflush(NULL)` has the same no-pending-data meaning.
It neither discards buffered output nor claims durable filesystem sync.
No exit flush hook is required by this policy. There is no `setvbuf` API yet.

Reads/writes loop over short transfers and return completed **items**, preserving
partial-byte progress on error. Zero-progress writes fail with EIO. EAGAIN,
EPIPE, ENOSPC and other native errors set errno and the sticky stream error
indicator; they are not hidden by retry loops or fake success. EOF has a
separate sticky indicator, cleared by successful positioning or `clearerr`.
Zero-sized transfers make no pointer access; size multiplication/length overflow
fails before constructing a slice. Open modes support `r/w/a`, `+`, `b` and
exclusive `wx`; allocation happens before open so ENOMEM cannot truncate a file.
Close consumes/frees the stream even when native backend cleanup reports an error,
matching the existing consuming-close contract in the kernel.

Float, wide, positional and `%n` formatting is explicitly unsupported and fails
with `-1`, errno=ENOSYS and the stream error indicator. Earlier output may have
already reached the descriptor. A malformed trailing `%` fails with EINVAL;
format width/count overflow fails with E2BIG. No substitute output is invented.

`tmpfile()` is explicitly **NULL / ENOSYS**, not fake success. Inspection found
`sunlight-fs/src/tmpfs.rs::unlink` invalidates open handles to the removed entry.
Opening and immediately unlinking therefore cannot implement an anonymous
stream. A named file deleted only by fclose would also leak on process exit;
that workaround was not used. The original probe source is unchanged and its
host-adapter execution is retained as **exit 1**, not a passing stdio execution.

### Commands and exact observed outputs

From the repository root:

```sh
python3 tools/native-c/tests/run_stdio.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-stdio-complete
```

Exact runner stdout (exit **0**):

```text
stdio host tests: PASS (11 tests)
stdio C host adapter: PASS (exit 0; exact stdout/stderr)
historical tmpfile probe host adapter: UNSUPPORTED (exit 1; ENOSYS)
stdio native builds and ELF checks: PASS (2 probes)
C++ headers: PASS
guest execution: UNKNOWN (no arbitrary-ELF guest runner)
```

The runner's logs retain **every exact command**, stdout, stderr and exit code,
including:

```sh
bash tools/build_native_c.sh
rustc --edition=2021 --test tools/native-c/tests/stdio-host.rs -o <out>/stdio-tests
<out>/stdio-tests
rustc --edition=2021 --crate-type=staticlib tools/native-c/tests/stdio-host.rs -o <out>/libstdio-host.a
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror -fno-builtin \
  tools/native-c/tests/stdio.c -o <out>/stdio-native
```

Here `<out>` is the absolute final evidence directory, expanded in each raw log.
The native C command emits **no stdout or stderr**, exit **0**. The native
runtime archive build returns **0**, ending exactly:
``Finished `release` profile [optimized] target(s) in 0.76s``.
It emits the existing unused `ChronosManifest.terminal` warning at
`sunlight-libc/src/sun_exec.rs:767`; no new stdio warning is emitted.
Native ELF checks verify ET_EXEC, EM_X86_64, native branding, nonzero
`e_entry == _start`, no PT_INTERP and no undefined symbols. Archive symbol
inventory and both native disassemblies are retained. These are build/static
inspection results, not guest execution.

Host tests end exactly:

```text
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

The separately linked C host-adapter executable receives stdin `Q\n`, exits
**0**, and emits exact stdout:

```text
stdio: native -42 42 0x2a -1234567890 7 %
stdio: puts
!
stdio: vprintf V
stdio: file PASS; tmpfile ENOSYS
```

Exact stderr:

```text
stdio: stderr 0007
```

These host executions use the actual canonical stream/formatter code with
**Linux descriptor, allocator and errno adapters**. They validate stdio logic,
locking, file behavior and the C varargs ABI on x86-64, not SunlightOS kernel I/O,
startup/TLS or guest filesystem behavior.

Other executed regression commands:

```sh
env -u NATIVE_RUNNER -u HELIOS_RUNNER python3 docs/audit/probes/run.py \
  --targets native \
  --out docs/audit/probes/native-foundation/results-2026-11-10-stdio-complete-integration
bash tools/test-mem-string-proof.sh
rustc --edition=2021 --test tools/native-std/tests/futex.rs -o /tmp/sunlight-native-futex-tests
/tmp/sunlight-native-futex-tests
bash tools/build_native_std.sh check --manifest-path tools/native-std-probe/Cargo.toml --release
```

All return **0**. The API runner reports **2 build PASS / 16 FAIL**, with
`stdio.c` and `string.c` passing. All 18 host_run cells remain NOT_RUN and
guest_run cells UNKNOWN. The remaining 15 C and one C++ failures are the
existing missing-header failures. Phase 1 had **1 PASS / 17 FAIL**; its raw
failure records are preserved. Native build success of `stdio.c` does not
remove its explicit unsupported `tmpfile` runtime result.

Memory/string proof ends exactly:
`test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s`;
it also passes native code-generation, relocatable C ABI and C++ header checks.
Futex proof ends exactly:
`test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s`.
Their full logs are retained in `results-2026-11-10-stdio-final/regression-*.log`;
these two standalone harnesses do not compile stdio and their source is unchanged.
The final native Rust std check ends exactly
``Finished `release` profile [optimized] target(s) in 0.15s``, with the same
existing warning. Its incidental probe lockfile refresh was restored to the
pre-check bytes. The three Phase 1 entry/write/exit C probes were also rebuilt
with `cc.py -std=c11 -Wall -Wextra -Werror`; all link with exit **0**, pass ELF
inspection and have empty undefined-symbol inventories. Logs are in the final
`regression-foundation-*.log` files. No full workspace or kernel boot check was
run for this localized userspace change.

Retained earlier attempts: `results-2026-11-10-stdio-initial/host-c-run.log`
records C exit **13** because the fixture expected printf count 47 instead of
the actual 42; the fixture expectation was corrected. The first host harness
compile also exposed relative module resolution, corrected by explicitly
naming `stdio/format.rs`. During review,
`results-2026-11-10-stdio-final/host-tests-stale-error-before.log` records a real
`fgets` failure (**0 passed / 1 failed**, exit **101**): a pre-existing stream
error incorrectly turned a later partial line at EOF into NULL. The corrected
implementation distinguishes the current read's EOF from an older error flag;
the final 11-test run includes this regression. Intermediate passing attempts
remain in the `stdio-final` and `stdio-verified` folders and do not substitute
for the final `stdio-complete` results.

### Guest limitation and single Phase 3 blocker

No guest command was issued. The existing `tools/native_std_boot_gate.py`
boots a fixed Rust FP/parser payload embedded by `kernel/src/main.rs:283` and
accepts no arbitrary C ELF path, stdin fixture or argv fixture. This is a
**test-loader integration limitation**, not evidence that the kernel rejected
these ELFs. All native C guest execution remains **UNKNOWN**. A future isolated
runner must supply a disposable writable path, stdin `Q\n`, observe the exact
stdout/stderr above and exit **0**. The earlier explicit-exit probe still
requires status **23**. No existing VM or disk was contacted.

**Single next Phase 3 blocker: real `tmpfile` backing with an anonymous-file /
open-unlink lifetime contract.** The earliest remaining failure in the original
stdio probe is now runtime NULL/ENOSYS, rather than preprocessing or linking.
Resolve that lifetime contract before claiming that probe passes. Guest C
execution remains a separate unverified acceptance gate; no claim is made for
full buffering, floating-point formatting, pthreads or CPython readiness.

## Native C foundation, Phase 3: real tmpfile backing — 2026-11-10

`tmpfile()` now returns the canonical unbuffered read/write FILE after
exclusive mode-0600 creation and immediate unlink on the native `/tmp` tmpfs.
The actual tmpfs/VFS and kernel descriptor code support the necessary lifetime
in **host execution**: the name disappears, existing references keep working,
and the last backend close releases the inode, its quota and its owned pages.
**Native guest execution remains UNKNOWN.** This is implementation and host
contract evidence, not a guest runtime PASS or a CPython readiness claim.
No CPython configure/build or additional formatting/buffering work was attempted.

Final evidence is in
[`results-2026-11-10-tmpfile-final/`](probes/native-foundation/results-2026-11-10-tmpfile-final/),
with the original API suite in
[`results-2026-11-10-tmpfile-integration/`](probes/native-foundation/results-2026-11-10-tmpfile-integration/).
Each command log retains exact argv, stdout, stderr and exit status. Generated
binaries, archives and objects remain ignored. Earlier Phase 1/2 results and
the initial Phase 3 attempts are retained separately.

### Source trace and the underlying blocker

The Phase 2 NULL/ENOSYS originated directly in `sunlight-libc/src/stdio.rs`.
Its comment identified a real backend defect: `Tmpfs::unlink` invalidated all
handles pointing at the entry and removed its inode immediately. `RamFs` is
a compatibility alias for this page-backed `Tmpfs`, not a separate backend.

The new `open_unlink_retains_inode_until_last_close` regression was executed
before fixing that backend. In `results-2026-11-10-tmpfile/vfs-before.log`,
write after unlink returns **Err(BadHandle)** instead of **Ok(8)**:
**0 passed / 1 failed**, exit **101**. The preceding test-fixture compilation
failure (missing `ToOwned` import, subsequently avoided) is separately retained
as `vfs-initial-compile.log`, also exit **101**. Neither is relabeled a pass.

The syscall path was inspected through the canonical wrappers in
`sunlight-libc/src/lib.rs`, error decoding in `sys.rs`,
`kernel/src/arch/x86_64/syscall.rs`, `kernel/src/process/fd_table.rs`, the
reaper in `kernel/src/sched/mod.rs`, and `sunlight-fs/src/{vfs,tmpfs}.rs`:

| Operation | Native contract used here |
| --- | --- |
| open | **40**, NUL-terminated path, flags, mode. `O_RDWR=2`, `O_CREAT=0x40`, `O_EXCL=0x80`; no `O_TRUNC` or `O_APPEND`. The kernel holds `KERNEL_VFS` across exclusive creation, so an existing file is never opened or truncated. |
| unlink | **65**, NUL-terminated path. Zero means success; current policy/backend failures collapse to raw `u64::MAX`, decoded as generic Failed/EIO. No new POSIX unlink export or Linux fallback was added. |
| close | **41**, consuming descriptor removal. Dup/inherited descriptors share an open description; only its final reference requests VFS close. |
| read / write | **42 / 43**, descriptor, pointer, length; counts represent progress, zero read is EOF. Kernel VFS transfers are at most 4096 bytes; canonical stdio already loops over short transfers. |
| seek | **44**, descriptor, signed offset, whence **0/1/2** for SET/CUR/END. Offset lives in the shared kernel open description; END uses `fstat_handle`, without reopening the name. |

These native values were checked in source, not inferred from Linux. Native
errors are encoded negative results, **not negative POSIX errno numbers**:
generic=-1, Again=-2, NoEntry=-3, Access=-4, BadFd=-5, Inval=-6,
IsDir=-7, NotDir=-8, I/O=-9, range=-10, pipe=-11, space=-12,
memory=-13. The checked wrappers return typed `Errno`; stdio maps it to
the TLS-backed C errno. AlreadyExists currently collapses to Failed/EIO;
the reused temporary-file creator retries that generic creation failure and
directory collisions at most **128** times, then returns EIO. More precise
collision/unlink error reporting is not claimed.

The intended guest path is supported by source: `init_kernel_vfs` in
`kernel/src/main.rs` mounts the authoritative root tmpfs using `INITRAMFS`;
`sunlight-fs/src/ramfs.rs` supplies `/tmp` with mode **1777**. User filesystem
policy permits `/tmp` creation/deletion. Disk mounts are `/boot`, `/state`
or `/mnt`; runtime mount validation permits only `/mnt` and its descendants,
so it cannot replace `/tmp` with FAT. `tmpfile` deliberately uses `/tmp`
instead of TMPDIR/TEMP or a Linux path fallback. This verifies provision and
routing in source; writable-directory and entropy-service execution in a
guest remain UNKNOWN.

### Implementation and files changed in this phase

| Files | Change |
| --- | --- |
| `sunlight-fs/src/tmpfs.rs` | Separate dynamic entry visibility from open lifetime with a linked bit. Unlink hides/clears the name; lookup/stat/open/read_dir no longer expose it. Open handles retain the inode. `fstat_handle` reports nlinks=0. Last close removes the inode and adjusts other entry indexes without changing handle generations. Implicit-root handles retain their sentinel index. |
| `sunlight-fs/src/tmpfs/quota_tests.rs` | Reproduce open-unlink failure; verify two independent opens, absent name/listing, writing after unlink, readback, recreation independence, exact page/quota retention and release, quota exhaustion, compaction and stale-handle rejection. Update the earlier unlink quota test to close its still-open handle before expecting reclamation. |
| `sunlight-libc/src/stdio.rs`, `sunlight-libc/include/stdio.h` | Implement/document tmpfile using existing `temp::mkstemp`, existing unlink/close wrappers, allocation and `SunlightFile::new` machinery. No alternate memory-only FILE representation. |
| `tools/native-c/tests/stdio-host.rs`, `stdio-tests.rs` | Compile the actual stdio **and existing temp.rs** with explicit Linux descriptor/entropy/allocator adapters. Add allocation, entropy, open, collision, unlink and consuming-close fault checks, including errno clobbering during close/free; track descriptor and FILE cleanup. Verify anonymous round trips, independent/concurrent streams, positioning, EOF/error indicators and bounded collision retries without truncation. |
| `tools/native-c/tests/stdio.c`, `docs/audit/probes/stdio.c` | Replace the dedicated ENOSYS assertion with a temporary-stream round trip. Adapt the historical tmpfile/fputs/rewind/fgetc probe itself, retaining its original contract and extending positioning, EOF/error, independent-stream and close checks. Its prior one-line source is retained in `results-2026-11-10-tmpfile/historical-probe-before.log`. |
| `tools/native-c/tests/open-unlink.c` | Native ABI probe: exclusive create, dup, unlink, failed reopen with native NoEntry=-3, write, seek, readback, close one descriptor and read through the survivor, final close. Requires an absent disposable argv path on tmpfs. Built/inspected only; guest UNKNOWN. |
| `tools/tests/fd-description/{Cargo.toml,Cargo.lock,src/lib.rs}` | Connect the existing real kernel descriptor harness to the actual VFS/tmpfs and host page provider. Verify shared offsets, seek-to-zero/readback, dup/inherited references, parent descriptor draining, surviving child I/O, final child drain and owned-frame release. This composes production structures; it does not execute kernel syscalls or the guest scheduler/reaper. Lock changes record the local dependencies and existing manifest version. |
| `tools/native-c/tests/run_stdio.py` | Run these host suites, the adapted historical C probe and all three native C builds/ELF checks; report guest UNKNOWN separately. |
| `docs/temporary-files.md`, this progress record and new evidence directories | Replace the stale unsupported status with the implementation, exact evidence and remaining runtime limits. |

FILE allocation precedes any file creation. On creation failure it is freed.
On unlink failure the descriptor is consumed/closed and the allocation freed;
the original operation's error is installed **after** cleanup, even if cleanup
changes errno or reports another error. No successful named stream is returned.
A failed unlink can leave its created name; this is an explicit failure and
the implementation cannot promise pathname cleanup when unlink itself fails.

Creation uses `/tmp/.sunlight-stdio.XXXXXX`, mode 0600, and the existing native
entropy path with flags=0. Entropy failure returns NULL/EAGAIN and creates
nothing. No unpredictable-name, name-secrecy or broader temporary-directory
security guarantee is made. Existing-file and directory collisions are bounded;
concurrent callers cannot open/truncate each other's existing named files.
Successful streams have both read/write permissions and use the existing lock,
unbuffered I/O, positioning, EOF/error and consuming fclose paths.

Unlinked inodes keep their page and inode quota charges. The last close drops
the existing `TmpfsInode`, which returns owned frames through `sunlight-mm`
and releases the charges. Tests observe both usage and the host frame-provider
live count, rather than inferring page release from metadata alone. Existing
kernel `take_all_handles`/reaper cleanup already closes final VFS references;
no process-lifecycle implementation change was needed. Guest exit/crash timing
is unverified. FAT and rename-over-existing-file lifetime are outside this
phase; neither is used to implement tmpfile.

### Exact commands and observed results

From the repository root (each runner requires a new output directory):

```sh
python3 tools/native-c/tests/run_stdio.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-tmpfile-final
env -u NATIVE_RUNNER -u HELIOS_RUNNER python3 docs/audit/probes/run.py \
  --targets native \
  --out docs/audit/probes/native-foundation/results-2026-11-10-tmpfile-integration
bash tools/test-mem-string-proof.sh
rustc --edition=2021 --test tools/native-std/tests/futex.rs -o target/tmpfile-futex-tests
target/tmpfile-futex-tests
bash tools/build_native_std.sh check --manifest-path tools/native-std-probe/Cargo.toml --release
cargo check -p sunlight-kernel
```

The stdio runner returns **0**, with exact stdout:

```text
stdio host tests: PASS (see host-tests.log for count)
actual tmpfs/VFS and kernel descriptor host tests: PASS (see host-vfs.log, host-fd-vfs.log)
stdio C host adapter: PASS (exit 0; exact stdout/stderr)
historical tmpfile probe host adapter: PASS (exit 0)
stdio/open-unlink native builds and ELF checks: PASS (3 probes)
C++ headers: PASS
guest execution: UNKNOWN (no arbitrary-ELF guest runner)
```

The runner's exact child commands include:

```sh
bash tools/build_native_c.sh
cargo test -p sunlight-fs --lib --target x86_64-unknown-linux-gnu -- --test-threads=1
cargo test --manifest-path tools/tests/fd-description/Cargo.toml --target x86_64-unknown-linux-gnu -- --test-threads=1
rustc --edition=2021 --test tools/native-c/tests/stdio-host.rs -o <out>/stdio-tests
<out>/stdio-tests
rustc --edition=2021 --crate-type=staticlib tools/native-c/tests/stdio-host.rs -o <out>/libstdio-host.a
```

`<out>` is expanded to the absolute final evidence directory in every raw log.
Host C links use `clang -std=c11 -Wall -Wextra -Werror -fno-builtin`, the
canonical include tree and the explicitly separate host archive. Native C
links use `python3 tools/native-c/cc.py` with those same C flags for
`tools/native-c/tests/stdio.c`, `docs/audit/probes/stdio.c` and
`tools/native-c/tests/open-unlink.c`. All host adapter and native C
compile/link commands return **0**; native C compiler stdout/stderr are empty.
Exact argv are in their logs.

| Layer / command log | Final observed result | Exit |
| --- | --- | --- |
| actual filesystem library, `host-vfs.log` | **90 passed, 0 failed** | **0** |
| actual kernel descriptors + VFS, `host-fd-vfs.log` | **17 passed, 0 failed**; 0 doctests | **0** |
| canonical stdio/temp with Linux adapters, `host-tests.log` | **19 passed, 0 failed** (includes 3 existing temp helper tests) | **0** |
| dedicated C, `host-c-run.log` | exact output below, stdin `Q\n` | **0** |
| adapted historical stdio C, `host-original-run.log` | empty stdout/stderr | **0** |
| native archive, `build-runtime.log` | release build finished in **0.03s**; existing ChronosManifest.terminal warning | **0** |
| three native C ELFs + C++ header check | all build/static inspection PASS; **no guest execution** | **0** each |
| memory/string, `regression-mem-string.log` | **24 passed, 0 failed**, native codegen/C relocatable ABI/C++ checks pass | **0** |
| futex, `regression-futex.log` | **3 passed, 0 failed** | **0** |
| native Rust std, `regression-native-std.log` | pinned native probe check PASS, finished in **0.38s** | **0** |
| kernel, `regression-kernel-check.log` | native kernel check PASS, finished in **1m 55s** (includes embedded service builds) | **0** |
| original API suite, `regression-api.log` | **2 build PASS / 16 FAIL**, all 18 host NOT_RUN / guest UNKNOWN | **0** |

The native Rust check's incidental probe lock refresh was restored to its
pre-check bytes (original SHA-256 is in its log). Existing warnings include the
unused ChronosManifest field, duplicate startup-fixture bin source, and the
standalone kernel-descriptor harness's cfg/dead-code warnings; none is hidden.
The kernel check also emits the VMXNET3 build marker and existing
NetOp/NetStatus/NetReady/NetDiagnostic naming warnings from sunlight-net.

Exact dedicated C stdout:

```text
stdio: native -42 42 0x2a -1234567890 7 %
stdio: puts
!
stdio: vprintf V
stdio: file PASS; tmpfile PASS
```

Exact stderr is `stdio: stderr 0007` followed by a newline. Native ELF
inspection verifies ET_EXEC/EM_X86_64, native branding, nonzero entry equal to
`_start`, no PT_INTERP and no undefined symbols. Symbol inventories and native
disassemblies are retained. The new raw open-unlink probe's host execution is
NOT_RUN because it issues native syscalls; its build is not a runtime pass.

### Before/after evidence, limitations and the single Phase 4 blocker

| Contract/probe | Before | Final |
| --- | --- | --- |
| actual tmpfs write after unlink | **Err(BadHandle)**, test exit **101** | host contract PASS, inode/pages retained until final close |
| historical stdio C host adapter | Phase 2 **exit 1 / ENOSYS**, retained in `stdio-complete/host-original-run.log` | adapted historical probe **exit 0**, empty output |
| original API suite native builds | Phase 2 **2 PASS / 16 FAIL** | **2 PASS / 16 FAIL**, unchanged |
| native C guest runtime | **UNKNOWN** | **UNKNOWN**, zero guest PASS / FAIL |

The original suite still has 17 C probes (2 build PASS / 15 FAIL) and one
C++ failure. Its additional missing-header failures remain recorded; host
adapter results do not increase native API counts. This phase's three ELF
builds are focused fixtures, not three newly supported API rows.

The initial Phase 3 runner (`results-2026-11-10-tmpfile-initial/`) passed
89 filesystem, 17 descriptor and 19 stdio/temp tests. Review then added an
implicit-root/compaction test, which exposed a new removal-helper bug:
decrementing the implicit-root sentinel caused an index-out-of-bounds panic,
**0 passed / 1 failed**, exit **101**, retained as
`results-2026-11-10-tmpfile/vfs-compaction-before.log`. The fix excludes that
sentinel from index adjustment; the final **90-test** filesystem run includes
this regression. The earlier pass does not validate the final source.

`runner-environment.log` observes NATIVE_RUNNER=None and HELIOS_RUNNER=None.
The documented `tools/test_native_std_foundation.sh` and
`tools/native_std_boot_gate.py` still embed/boot a fixed Rust FP/parser ELF
(kernel `native_std_probe` feature); they accept no arbitrary C ELF, argv path
or stdin fixture. The README's NATIVE_RUNNER example is a placeholder, not an
implementation. No guest command was issued, and no existing VM/disk was
contacted. Linux stdio adapters prove stream/C ABI logic; the actual VFS and
kernel descriptor host tests separately prove the implemented lifetime in
those production structures. Neither establishes guest startup, entropy IPC,
native syscall execution or real process-death reclamation. Those remain
**UNKNOWN**, along with general FAT/rename lifetime and full libc/Python support.

**Single recommended Phase 4 blocker: a documented isolated native C ELF
runner, followed by guest acceptance of these built probes.** It must run the
raw open-unlink ELF with a fresh tmpfs path, the adapted historical tmpfile
probe, and the dedicated stdio fixture with its argv/stdin/output contract;
capture actual exit codes and observe final-reference/process-exit cleanup.
Until then, native guest tmpfile runtime is UNKNOWN even though the underlying
source blocker and host contract tests are resolved.


## Native C foundation, Phase 4: isolated guest ELF acceptance — 2026-11-10

The native C runner is now implemented in
[`tools/native-c/guest.py`](../../tools/native-c/guest.py), with usage in
[`tools/native-c/README.md`](../../tools/native-c/README.md). All **nine built
ELFs pass** exit, exact output and physical reclamation checks in the actual
SunlightOS kernel on **2 and 4 CPUs**. The 21 historical native executable
acceptance cells are **5 PASS / 16 FAIL / 0 UNKNOWN**. The sixteen failures
cannot produce ELFs because their headers/API surface are still missing;
**their guest execution is NOT_RUN, not sixteen observed kernel failures**.
No CPython configuration or build was attempted.

### Root cause, isolated packaging and loader path

The earlier gate was not an arbitrary ELF runner: `native_std_boot_gate.py`
booted the fixed Rust FP/parser image, and the `native_std_probe` feature
embedded that one binary and injected a fixed shell command. The normal
service spawn registry in `kernel/src/process/spawn.rs` also maps fixed paths
to embedded service images. Neither provided an ELF path/argv/stdin input or
an exit-code/output protocol to the audit runner. This was missing harness
integration, not a demonstrated C ELF validation defect.

The existing `exec_into_process` already accepts native ELF bytes. Its ELF
header/segment validator, page-aligned PT_LOAD mapping, entry point, BSS
zeroing, native personality, W^X checks and bounds remain unchanged. The
common exec path supplies the stack's argc/argv/NULL/envp/NULL/auxv (including
AT_RANDOM and AT_NULL), then the canonical `_start` initializes TLS and calls
C `main`. Guest serial records show actual entries around 0x400000, valid
16-byte-aligned stack pointers, argc 1/2, native mmap and normal exit/reaping.
No Linux personality stamp, alternate CRT or relaxed loader checks were added.

The opt-in `native_c_guest` kernel feature:

* Includes a generated manifest of the supplied ELFs. Build dependencies track
  both the manifest and the `include_bytes!` inputs. Each run retains SHA-256
  hashes for the C ELFs and its private kernel ELF snapshot.
* Excludes the desktop service payloads and default boot self-tests from this
  isolated fixture. Ordinary boots still use the original payloads and full
  INITRAMFS. The fixture uses the actual Tmpfs implementation with `/` and a
  mode-1777 `/tmp`, and packages each ELF into `/tmp/native-c.elf`, reads it back
  through VFS, then loads those bytes with the ordinary `exec_into_process`.
* Runs the actual init nameserver with **spawn token zero** (registry only),
  plus the actual `rand_service`. It waits until the latter has registered and
  is blocked receiving on its own authenticated endpoint before launching C.
  Secure `getrandom(flags=0)` retains its real capability IPC route and its
  fail-closed behavior; kernel entropy readiness alone is not sufficient.
* Runs C cases sequentially as **uid/gid 1000**, without privileged service
  roles. Supplies argv and stdin `Q\n` where required, and separate ordinary
  tmpfs-backed descriptors 0/1/2. It collects stdout/stderr after actual reaping,
  encodes bounded hexadecimal serial records, and reconstructs exact host
  bytes, so guest output cannot forge a result record. Capture collection
  rejects streams larger than 1 MiB. This is file-backed standard I/O, not a
  terminal or pipe acceptance claim.
* Observes the real process exit code and `ProcessState::Reaped`, checks the
  normal reaper's reclaimed-frame counts, then verifies zero descriptors,
  zero PID-owned PMM frames and a released kernel stack. After deleting the
  fixture's captures and named work file, file-data/private/page-table frame
  counts, VFS open handles and visible `/tmp` entry counts must return to their
  pre-launch snapshots. `Vfs::open_handle_count` exposes the existing open
  accounting; no descriptor or lifecycle behavior was changed.

`kernel/src/native_c_guest.rs` contains the fixture. Its observer hooks in
scheduler idle and reaping compile out of ordinary kernels. Support and kernel builds use dedicated caches under `target/native-c-guest/`,
avoiding ordinary workspace/editor build locks and artifact replacement.
Host manifest builds are serialized until a private kernel ELF copy is made; each ISO uses
its own staging root and that private copy. QEMU attaches **only a read-only
CD**, no disk/rootfs image, host directory or network. TCG, serial capture,
virtio-rng, a 90-second wall deadline and debug-exit make runs automated and
headless. No existing VM, persistent disk or privileged tooling was used.

### Exact guest invocations and retained evidence

From the repository root, with new output directories:

```sh
python3 tools/native-c/guest.py --suite --cpus 2 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-guest-isolated-final-2cpu
python3 tools/native-c/guest.py --suite --cpus 4 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-guest-isolated-final-4cpu
```

The suite builds the runtime with `bash tools/build_native_c.sh`; C ELFs use
`python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror SOURCE -o ELF`.
Each `build-*.log` retains the exact source and absolute output path, including
all failed API builds. `elf-*.log` retains native ELF headers/program headers.
The runner separately builds the two actual supporting services:

```sh
env RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static' \
  cargo build -p sunlight-init -p rand_service --release --target x86_64-unknown-none \
  --target-dir target/native-c-guest/support
env SUNLIGHT_NATIVE_C_MANIFEST=<out>/manifest.rs \
  cargo build -p sunlight-kernel --bin sunlight-kernel \
  --target x86_64-unknown-none --target-dir target/native-c-guest/kernel \
  --no-default-features --features native_c_guest
bash tools/make_hybrid_iso.sh <out>/kernel.elf <out>/guest.iso \
  <repo>/target/limine <out>/media
qemu-system-x86_64 -accel tcg -cpu max -m 2048M -smp 2 \
  -cdrom <out>/guest.iso -boot d -display none -monitor none \
  -serial file:<out>/serial.log -nic none \
  -device virtio-rng-pci,disable-modern=on \
  -device isa-debug-exit,iobase=0xf4,iosize=0x04 -no-reboot
```

For the second run `-smp` is **4**. `<out>` and `<repo>` above are explanatory
placeholders; each actual `kernel-build.log`, `iso-build.log` and `qemu.log`
contains the fully expanded command. Successful QEMU completion is **33**
(debug-exit transport), separate from each guest process status. Successful
suite invocation returns **0**. These directories retain the full serial
transcripts, CSV/JSON results, exact decoded stdout/stderr and build logs:

* [`results-2026-11-10-guest-isolated-final-2cpu/`](probes/native-foundation/results-2026-11-10-guest-isolated-final-2cpu/)
* [`results-2026-11-10-guest-isolated-final-4cpu/`](probes/native-foundation/results-2026-11-10-guest-isolated-final-4cpu/)

Single-ELF mode takes an arbitrary `cc.py` output. Without `--expect-exit`, its
host status is the observed guest status; with it, a matching status is a
successful assertion. Actual controls used:

```sh
python3 tools/native-c/guest.py \
  docs/audit/probes/native-foundation/results-2026-11-10-guest-services/exit.elf \
  --out docs/audit/probes/native-foundation/results-2026-11-10-guest-raw-exit
# Observed guest 23, host 23.
python3 tools/native-c/guest.py \
  docs/audit/probes/native-foundation/results-2026-11-10-guest-services/exit.elf \
  --expect-exit 0 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-guest-wrong-exit
# Observed guest 23, host 1: wrong expectation is rejected.
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror \
  tools/native-c/tests/fault.c \
  -o docs/audit/probes/native-foundation/results-2026-11-10-guest-regressions/fault.elf
python3 tools/native-c/guest.py \
  docs/audit/probes/native-foundation/results-2026-11-10-guest-regressions/fault.elf \
  --expect-exit 132 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-guest-fault
# Intentional user #UD: guest 132, host 1 even though the status matches.
```

The negative fault control uses a real `ud2`, not software fault injection.
Fatal `[FAULT]`/exception/panic/unknown-syscall records fail the host gate.
The final positive transcripts have empty `faults.log` files under this same
fatal-fault policy. The earlier four-CPU transcript also has a retained
`faults-final-assessor.log` check against that policy.
Negative controls are separate from the nine positive probes and the 21
historical cells. Wrapper logs with observed host statuses are in
`results-2026-11-10-guest-regressions/{raw-exit,wrong-exit,fault-run}.log`.
The API audit's `NATIVE_RUNNER` can now use `python3 tools/native-c/guest.py`,
which automatically creates a unique evidence directory when `--out` is absent.

### Guest results and captured output

| Built probe | Actual guest exit | stdout / stderr | 2 CPU / 4 CPU |
| --- | --- | --- | --- |
| foundation `entry.c` | **0** | both empty; argc/argv and canonical TLS startup work | PASS / PASS |
| foundation `write.c` | **0** | stderr `[NATIVE-C] write OK\n` (20 bytes) | PASS / PASS |
| foundation `exit.c` | **23** | both empty; deliberately nonzero success contract | PASS / PASS |
| historical `stdio.c` | **0** | both empty; two actual anonymous tmpfile streams | PASS / PASS |
| historical `string.c` | **0** | both empty | PASS / PASS |
| focused `stdout.c` | **0** | stdout `[NATIVE-C] stdout OK\n` (21 bytes) | PASS / PASS |
| full `tests/stdio.c` | **0** | stdout 104 bytes, stderr 19 bytes, stdin `Q\n`, argc 2 | PASS / PASS |
| native `open-unlink.c` | **0** | both empty; exclusive creation, dup, unlink, NoEntry=-3, surviving-descriptor readback, final close | PASS / PASS |
| `exit-cleanup.c` | **0** | both empty; two unclosed anonymous files and leaked allocation | PASS / PASS |

Exact full-stdio stdout:

```text
stdio: native -42 42 0x2a -1234567890 7 %
stdio: puts
!
stdio: vprintf V
stdio: file PASS; tmpfile PASS
```

Exact stderr is `stdio: stderr 0007` followed by a newline. The historical
stdio probe exercises positioning, EOF/error indicators, independent streams
and fclose, through the actual native syscall and VFS path. Positive runs have
**9 PASS / 0 FAIL** each, **18 successful guest executions** across 2/4 CPUs.
There are no unexpected page faults, GPFs, invalid opcodes, kernel panics or
unknown/unhandled syscall numbers in these positive serial transcripts.

Representative actual kernel serial evidence (two-CPU service-backed run;
full records remain in `serial.log`):

```text
[ENTROPY] secure source=virtio-rng conditioner=ChaCha20 readiness=ready
[RAND] Registered as 'rand'
[NATIVE-C-SUPPORT] nameserver=production rand=production receive_wait=ready
[NATIVE-C-REAP-END] case=2 exit=23 user_frames=515 page_tables=10 owned_frames=0 fds=0 stack=0
[NATIVE-C-REAP-BEGIN] case=8 fds=5 file_frames=3
[NATIVE-C-REAP-END] case=8 exit=0 user_frames=774 page_tables=10 owned_frames=0 fds=0 stack=0
[NATIVE-C-RESULT] case=8 exit=0 stdout=0 stderr=0 clean=1 file_before=0 file_after=0 private_before=1036 private_after=1036 pt_before=16 pt_after=16 handles_before=0 handles_after=0
[NATIVE-C-DONE] cases=9
```

The cleanup control enters actual reaping with **5 descriptors**, including
**two unclosed anonymous files holding 3 PMM file-data pages**. Those pages
return to the PMM, all descriptors close, **774 user frames and 10 page-table
frames** are reclaimed, and the PID's owned-frame inventory reaches **0**.
The support processes remain resident; their **1036 private frames and 16
page-table frames** are the stable baseline. This combines physical PMM class
counters with real reaper statistics and the PID ownership scan, rather than
inferring reclamation merely from zero metadata. The raw open/unlink probe
separately proves read/write after unlink and survival after closing one dup.

### The 21 historical UNKNOWN cells: explicit dispositions

| Historical cells | Prior guest acceptance | Current acceptance | Guest execution |
| --- | --- | --- | --- |
| entry, write, exit, original stdio, original string | 5 UNKNOWN | **5 PASS** | all executed and verified |
| atexit, clock, dirent, dlfcn, getrandom, langinfo, locale, math, mmap, poll_select, pthread, signal, socket, termios, wchar, cpp-runtime | 16 UNKNOWN | **16 FAIL: BUILD_FAILED** | **NOT_RUN**, no executable exists |
| **Total** | **21 UNKNOWN** | **5 PASS / 16 FAIL / 0 UNKNOWN** | **5 observed PASS / 0 observed runtime FAIL / 16 NOT_RUN** |

The original API suite therefore remains **2 build PASS / 16 build FAIL**,
now with actual guest PASS for both supported APIs. The three foundation
ELFs add three runtime PASS, including the required exit 23. The four focused
fixtures add four further guest PASS and do not inflate historical API counts.
`compiler.c` is compile-only and is outside these 21 cells. Earlier result
files and the unexecuted Helios/host guest columns are preserved.

### Host regressions, retained failures and remaining limits

Executed host regressions:

```sh
python3 tools/native-c/tests/run_stdio.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-guest-host-stdio
bash tools/test-mem-string-proof.sh
rustc --edition=2021 --test tools/native-std/tests/futex.rs \
  -o target/native-c-guest-futex-tests
target/native-c-guest-futex-tests
bash tools/build_native_std.sh check \
  --manifest-path tools/native-std-probe/Cargo.toml --release
cargo check -p sunlight-kernel
```

The host stdio runner passes **90 filesystem tests, 17 actual kernel-descriptor
+ VFS tests, 19 stdio/temp tests**, both C host adapter executions, three native
C builds/ELF inspections and the C++ header check. Memory/string passes **24**
tests plus its native code-generation/C ABI/header checks; futex passes **3**.
That is **153 host unit tests**, all successful. Native Rust std and the final
ordinary-feature kernel check return **0**. Raw logs are in
`results-2026-11-10-guest-host-stdio/` and
`results-2026-11-10-guest-regressions/` (final kernel check:
`kernel-check-final.log`). The std check's incidental standalone probe lock
refresh was restored; before/after hashes are retained in `probe-lock.log`.
Existing ChronosManifest, net module naming, duplicate startup-fixture target
and standalone host-harness warnings remain visible. These checks are host
regressions, not a full ordinary desktop QEMU or interactive UI acceptance run.

Earlier attempts remain separate and are not relabeled:

* `results-2026-11-10-guest-initial/`: the first runner retained the entire
  desktop image (200 MiB). A diagnostic headless screen capture showed Limine
  loading it; the 90-second attempt reached kernel boot self-tests but no C
  launch. The isolated image excludes the unused desktop payloads and uses a
  minimal root, preserving the same timeout.
* `results-2026-11-10-guest-isolated/`: genuine guest entry/write/exit/string/
  stdout/open-unlink results were observed, but historical stdio returned **1**,
  full stdio **36**, and the cleanup fixture **51**, because libc's secure
  random path lacked the real nameserver/rand service. Kernel entropy was
  ready. This exposed a harness dependency, resolved by launching the actual
  services; libc was not changed or given a fallback. `-no-shutdown` also left
  this QEMU build stopped after its debug-exit record, so the host timed out
  despite `[NATIVE-C-DONE]`. The final runner omits that option and requires
  real QEMU transport exit **33** as well as the completion marker.
* `results-2026-11-10-guest-services/` and
  `results-2026-11-10-guest-final-2cpu/`,
  `results-2026-11-10-guest-final-4cpu/` and
  `results-2026-11-10-guest-acceptance/` retain additional passing guest attempts.
  Later editor Cargo lock contention motivated dedicated build caches in the
  final runner; the final 2/4-CPU runs validate that packaging change.
  The final single-ELF fault detector recognizes the kernel's actual `[FAULT]`
  records as well as legacy exception/panic markers; its deliberate #UD
  control must fail even with matching exit 132.

No existing native C probe needed a libc, syscall or loader fix after the
harness supplied its production dependencies. The native unlink/collision
error encoding still collapses some backend errors to generic EIO, as recorded
in Phase 3. This fixture accepts static native ET_EXEC images supported by the
existing loader; it does not add dynamic linking, general C headers/symbols,
terminal/pipe behavior, service restarts, or concurrent thread/crash matrices.
Ordinary exit and the separate negative user #UD control exercise the actual
reaper; they do not prove every process-lifecycle interleaving.

**Single next Phase 5 blocker: native POSIX libc API coverage, beginning with
`unistd.h` and matching exported functions.** The remaining 15 C probes and
one C++ probe still fail at missing headers/API surface (for example
`unistd.h`, `sys/random.h`, `sys/mman.h`, `time.h`, `pthread.h` and `iostream`).
Extend and verify that surface toward CPython prerequisites next; no CPython
configure/build or readiness claim is part of Phase 4.

## Native C foundation, Phase 5: first unistd slice — 2026-11-10

Original [`atexit.c`](probes/atexit.c) now builds unchanged and passes in actual
isolated SunlightOS guests on **2 and 4 CPUs**: observed exit **0**, exact stdout
`atexit OK\n` (10 bytes), empty stderr, and successful reaping/physical-frame
checks. This proves one original API case and the bounded contracts below;
it does not establish broad POSIX, terminal, signal, threading or Python support.
No CPython configure/build was attempted. No ELF loader or kernel implementation
was changed in this phase.

### Inventory before implementation: all sixteen build failures

The retained Phase 4 inputs are
[`results-2026-11-10-guest-isolated-final-2cpu/`](probes/native-foundation/results-2026-11-10-guest-isolated-final-2cpu/).
Each corresponding `build-PROBE.log` was inspected. Before editing headers or
exports, all sixteen failed builds were reproduced in
[`results-2026-11-10-unistd-before/`](probes/native-foundation/results-2026-11-10-unistd-before/),
with exact commands, exits and first diagnostics in `build-PROBE.log` and
`inventory.csv`. These are **BUILD_FAIL / guest NOT_RUN**, not sixteen actual
guest failures. Every C source below defines `_GNU_SOURCE` before its includes;
the C++ source has no such definition.

| Original probe | First concrete pre-change build error | unistd requirements visible in actual source |
| --- | --- | --- |
| atexit | `atexit.c:3:10: fatal error: 'unistd.h' file not found` | `write(int, const void *, size_t)` returning `ssize_t`; separate stdlib `atexit(void (*)(void))` |
| clock | `clock.c:2:10: fatal error: 'time.h' file not found` | none |
| dirent | `dirent.c:2:10: fatal error: 'dirent.h' file not found` | none |
| dlfcn | `dlfcn.c:2:10: fatal error: 'dlfcn.h' file not found` | none |
| getrandom | `getrandom.c:2:10: fatal error: 'sys/random.h' file not found` | none; its own header must supply its count types |
| langinfo | `langinfo.c:2:10: fatal error: 'langinfo.h' file not found` | none |
| locale | `locale.c:2:10: fatal error: 'locale.h' file not found` | none |
| math | `math.c:2:10: fatal error: 'math.h' file not found` | none |
| mmap | `mmap.c:2:10: fatal error: 'sys/mman.h' file not found` | none; its own mapping/offset contract remains blocked |
| poll_select | `poll_select.c:2:10: fatal error: 'poll.h' file not found` | none; `sys/select.h` is also needed later |
| pthread | `pthread.c:2:10: fatal error: 'pthread.h' file not found` | none |
| signal | `signal.c:2:10: fatal error: 'signal.h' file not found` | none |
| socket | `socket.c:2:10: fatal error: 'sys/socket.h' file not found` | includes unistd; calls `int close(int)` after socket creation |
| termios | `termios.c:2:10: fatal error: 'termios.h' file not found` | includes unistd but uses no function, type or constant from it; needs `tcgetattr` and `ENOTTY` elsewhere |
| wchar | `wchar.c:2:10: fatal error: 'wchar.h' file not found` | none; also includes locale |
| cpp-runtime | `cpp-runtime.cpp:1:10: fatal error: 'iostream' file not found` | none; C++ runtime is independent |

`atexit.c` uses literal fd **1** and count **10**; neither it nor socket/termios
requires any unistd constant, `off_t`, `pid_t`, `read`, `lseek`, feature-option
macro or POSIX version macro. The header additionally names the existing
standard descriptor slots as `STDIN_FILENO=0`, `STDOUT_FILENO=1`,
`STDERR_FILENO=2`. There is no `_POSIX_VERSION` claim. The selected base
signature is exposed with no feature macro, `_POSIX_C_SOURCE=200809L`, or
`_GNU_SOURCE`; this is declaration availability, not conformance to those
standards. `atexit`/`exit` are stdlib declarations, not unistd declarations.

The missing header was **not the only root cause**. A temporary declaration-only
unistd header exposed `atexit.c:5:23: error: call to undeclared function 'atexit'`.
Supplying just its correct declaration then exposed **undefined symbols
`atexit` and `write` at link**. See `atexit-declaration-only.log`,
`atexit-link-only.log`, and `diagnostic-headers.log` in the before directory.
These temporary inputs were diagnostic only; original probe source and expected
behavior were not edited.

| Needed function / related ABI | Pre-change C export | Canonical runtime / syscall and decision |
| --- | --- | --- |
| `ssize_t write(int, const void *, size_t)` | absent; Rust `crate::write(Fd, &[u8])` exists | `lib.rs::write` -> syscall **43** -> `sys_write_with_nonblocking`; factor shared `write_raw`, retain `sys::check_io_count`. Add only the C error/result adapter. |
| `int close(int)` (socket source) | absent; `sl_std_close(u32)` returns **void** and discards errors | Rust `crate::close` -> syscall **41** -> `sys_close`; do not alias the void Rust ABI to C. Not selected; socket is still blocked earlier. |
| `int atexit(void (*)(void))` | absent, no registry | Add a bounded C registry and declaration; no syscall corresponds to registration. |
| `void exit(int)` / return from C main | absent C export; startup called callback-free `sl_std_exit` | Add C termination before existing native group-exit syscall **157** and ordinary reaper. Rust-only main return and `sl_std_exit` retain their raw contracts. |
| offsets / EOF / tty | existing C `fd::lseek(i32,i64,i32)->i64`, Rust read and guessed `isatty`; not needed by original atexit | No new offset typedef, seek/read/tty declaration or export. EOF is a read/stdio condition, not a write result. Focused fixture verifies EOF via canonical stdio after a short C write. The existing fd-number-based isatty remains unsuitable as authoritative tty evidence. |

### Implemented ABI, error and lifetime decisions

Changed files:

* `sunlight-libc/include/unistd.h`: only `write`, `size_t` from compiler
  `<stddef.h>`, signed `long ssize_t`, and three standard fd constants. Native
  x86-64 is LP64: fd/C int is **32 bits**, size/count are **64 bits**.
* `sunlight-libc/src/unistd.rs` and `src/lib.rs`: C `write` returns Rust
  `isize`, calls the same single-transfer raw boundary as Rust `write`, and
  converts structured errors through existing TLS errno mapping to **-1 plus
  errno**. It never calls the stderr-specific `sl_std_write`/`write_all` path,
  loops away short progress, or constructs a Rust reference from a C pointer.
* `sunlight-libc/include/stdlib.h`, `src/termination.rs`, `src/std_runtime.rs`,
  `sunlight-libc/Cargo.toml`, `tools/native-c/Cargo.toml`: the C package enables
  `native-c` (which includes `native-std`). C main return and exported `exit`
  drain a locked, allocation-free registry of **32** simultaneous `atexit`
  registrations. Slots pop in reverse order; callbacks execute after unlocking
  and may register new callbacks. Successful registration returns 0, preserves
  errno; full registry returns -1/ENOMEM, NULL callback returns -1/EINVAL.
* `tools/native-c/tests/unistd.c`, `unistd-abi.c`, `run_unistd.py`: nine guest
  contracts, six production-module host unit tests, C11/C++17 signature/type
  checks across three feature settings and both include orders, and exactly-one
  definition checks for new and existing startup/stdio/temp/runtime symbols.
* `tools/native-c/guest.py`, `README.md`: add nine separate focused contracts,
  assert original atexit's actual expected output, and label unbuilt acceptance
  rows **BUILD_FAIL**, keeping `guest_run=NOT_RUN`. Other original expectations
  remain unchanged.
* `docs/audit/std-progress.md`: this appended evidence and current banner.

Negative C fds fail EBADF before conversion to native `Fd(u32)`. Nonzero NULL
writes fail EFAULT, counts above `isize::MAX` fail EINVAL before any transfer.
Successful zero/short/full counts preserve errno. Other native errors retain
the existing mapping (including EACCES, EAGAIN, EPIPE, ENOSPC and generic EIO).
The shared decoder rejects impossible counts above the request. Host injection
covers EAGAIN/EPIPE/ENOSPC without claiming actual pipe or disk-full guest tests.

The kernel's actual zero-count write returns 0 before validating nonnegative
fd existence/rights; the C slice preserves that behavior and documents it.
Arbitrary bad **non-NULL** memory remains native generic **EIO**, as
`user_memory_failure_for(false, ...)` returns generic failure. The dedicated
guest error fixture checks this exact result, rather than claiming EFAULT for
all bad addresses or altering the kernel encoding. This is an explicit limit
on POSIX behavior. No unsupported operation has a success-shaped stub.

Native regular-file writes transfer at most **4096 bytes** in one syscall. The
8192-byte fixture requires result **4096**, offset **4096**, the exact first
4096 bytes through `fread`, then EOF without ferror. It would fail if the new
export silently retried and returned 8192. Ordinary fd offset ownership stays
in the kernel. No offset conversion or alternate descriptor table was added.

C11 N1570 sections **7.22.4.2/7.22.4.4** informed callback signatures, minimum
32 registrations and reverse order, including registrations during callbacks.
This does not establish complete ISO C conformance. Existing FILE streams are
unbuffered, and tmpfile names are removed before return. After callbacks the
canonical group exit/reaper closes descriptors, including open anonymous
streams; no alternate stream/allocator/TLS implementation was introduced.
`atexit-exit` deliberately leaves an anonymous FILE open to verify that path.
Recursive/concurrent C exit, C++ destructors and callback crash matrices are
not established by these tests.

### Exact verification commands and evidence

Run from the repository root. Every guest directory is fresh; full expanded
compiler/support/kernel/ISO/QEMU arguments remain in its logs. The two CPU runs
have **identical per-case ELF SHA-256 hashes** for all 19 built cases, checked
in `final-matrix-check.log`. Runtime source/probe hashes are in
`final-source-hashes.log`.

```sh
python3 tools/native-c/guest.py --suite --cpus 2 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-unistd-final-2cpu
python3 tools/native-c/guest.py --suite --cpus 4 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-unistd-final-v2-4cpu
python3 tools/native-c/tests/run_unistd.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-unistd-host-final
python3 tools/native-c/tests/run_stdio.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-unistd-stdio
bash tools/test-mem-string-proof.sh
rustc --edition=2021 --test tools/native-std/tests/futex.rs \
  -o docs/audit/probes/native-foundation/results-2026-11-10-unistd-regressions/futex-tests
docs/audit/probes/native-foundation/results-2026-11-10-unistd-regressions/futex-tests
bash tools/build_native_std.sh check \
  --manifest-path tools/native-std-probe/Cargo.toml --release
cargo check -p sunlight-kernel
```

`--suite` runs `bash tools/build_native_c.sh` and compiles each C source with:

```sh
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror SOURCE -o ELF
```

For the unlocked original case SOURCE is `docs/audit/probes/atexit.c`, and ELF
is the corresponding guest evidence directory's `atexit.elf`; those full paths
are in `build-atexit.log`. Each built ELF has an `elf-NAME.log` inspection;
`results.csv/json`, `NAME.stdout.txt`, `NAME.stderr.txt`, `serial.log`,
`faults.log`, `kernel-sha256.log` and `qemu.log` retain runtime evidence. Each
suite exits **0**, QEMU's debug-exit transport is **33**, and both `faults.log`
files are empty. The earlier successful `unistd-initial-2cpu` and
`unistd-final-4cpu` runs remain separate; the final commands above rerun both
CPU counts after the C++ parameter-name repair described below.

Raw regression logs are in
[`results-2026-11-10-unistd-regressions/`](probes/native-foundation/results-2026-11-10-unistd-regressions/);
production module/header/symbol logs are in
[`results-2026-11-10-unistd-host-final/`](probes/native-foundation/results-2026-11-10-unistd-host-final/);
filesystem/descriptor/stdio/temp logs are in
[`results-2026-11-10-unistd-stdio/`](probes/native-foundation/results-2026-11-10-unistd-stdio/).
Guest evidence is in
[`2-CPU results`](probes/native-foundation/results-2026-11-10-unistd-final-2cpu/) and
[`4-CPU results`](probes/native-foundation/results-2026-11-10-unistd-final-v2-4cpu/).

| Verification | Host logic / adapters | Native build / check | 2-CPU guest | 4-CPU guest |
| --- | --- | --- | --- | --- |
| original atexit | NOT_RUN on Linux | PASS | PASS: exit 0, exact `atexit OK\n`, empty stderr | same PASS |
| historical stdio/string + entry/write/exit | NOT_RUN by guest runner | 5 PASS | 5 PASS (exit case 23) | same 5 PASS |
| other original APIs | NOT_RUN | 15 BUILD_FAIL | 15 NOT_RUN | 15 NOT_RUN |
| previous four focused stdio/temp/cleanup cases | two stdio C adapters PASS; raw native cases NOT_RUN on host | 4 PASS | 4 PASS | 4 PASS |
| nine new Phase 5 contracts | six focused unit tests included below | all nine build PASS | 9 PASS | 9 PASS |
| libc production modules | **94 PASS**, two credential tests NOT_RUN | canonical archive PASS; **12** C/C++ ABI checks PASS; unique exports PASS | covered subset above | covered subset above |
| actual filesystem + descriptor/VFS | 90 + 17 PASS | exercised by native build above | covered subset above | covered subset above |
| stdio/temp | 19 PASS; dedicated and historical C adapters PASS | 3 ELF checks PASS; C++ header PASS | full stdio/original/tmpfile fixtures PASS | same PASS |
| memory/string | 24 PASS | codegen/C ABI/C++ checks PASS | historical string PASS | same PASS |
| futex | 3 PASS | native Rust std check PASS | NOT_RUN as a new Rust guest regression | NOT_RUN as a new Rust guest regression |
| native Rust std | NOT_RUN as a host runtime | pinned probe check PASS, exit 0 | NOT_RUN as a new Rust guest regression | NOT_RUN as a new Rust guest regression |
| ordinary-feature kernel | NOT_RUN as a host runtime | **PASS**, exit 0, 2m 04s (`kernel-check.log`) | isolated native_c_guest feature only | isolated native_c_guest feature only |

### Nine new guest contracts, separate from historical counts

These are **nine named executions of the same focused ELF with different argv**,
not nine newly supported original API rows. Each is independently reaped and
checked for exact streams and physical cleanup. The prior Phase 4 nine built
cases (five historical and four focused) remain a distinct group: all nine pass
again on both CPU counts. Original atexit is the tenth supported case before
these nine new focused executions; total positive executions are **19 per CPU**.

| New contract | Expected exit | Exact stdout | Exact stderr | 2 CPUs | 4 CPUs |
| --- | --- | --- | --- | --- | --- |
| write-basic | 0 | `write OK\n` | `stderr OK\n` | PASS | PASS |
| write-zero | 0 | empty | empty | PASS | PASS |
| write-short | 0 | empty | empty | PASS | PASS |
| write-errors | 0 | empty | empty | PASS | PASS |
| atexit-order | 0 | `CBA` | empty | PASS | PASS |
| atexit-register | 0 | `BCA` | empty | PASS | PASS |
| atexit-capacity | 0 | exactly 32 `x` bytes | empty | PASS | PASS |
| atexit-exit | 27 | `A` | empty | PASS | PASS |
| atexit-raw-exit | 29 | empty | empty | PASS | PASS |

### Historical counts, retained failures and Phase 6

| Original population | Phase 4 | Phase 5 native build | Phase 5 guest (each CPU count) |
| --- | --- | --- | --- |
| 18 original API probes (17 C + one C++) | 2 build PASS / 16 BUILD_FAIL | **3 PASS / 15 BUILD_FAIL** (C: 3/14; C++: 0/1) | **3 PASS / 0 FAIL / 15 NOT_RUN** |
| 21 historical executables including three foundation probes | 5 guest PASS / 16 build-blocked NOT_RUN | **6 PASS / 15 BUILD_FAIL** | **6 PASS / 0 FAIL / 15 NOT_RUN**, zero unknown dispositions |
| 16 formerly blocked originals | 16 BUILD_FAIL, no guest executions | **1 PASS / 15 BUILD_FAIL** | **1 PASS / 0 FAIL / 15 NOT_RUN** |

All historical logs and Phase 1–4 prose remain preserved. The original
`atexit.c`, `socket.c` and `termios.c` were not changed. Adding write/unistd does
not clear socket's absent socket API or termios's absent header/tty behavior;
their current build logs still record those actual first errors.

The first broad host command, `cargo test -p sunlight-libc --lib --target
x86_64-unknown-linux-gnu -- --test-threads=1`, stalled at existing
`secret_store::tests::default_options_are_private_and_bounded`. Its production
`validate_options` calls native getuid/getgid syscalls **35/36**, which cannot be
used as Linux syscall adapters. The identified test process was stopped via
pidfd SIGTERM; Cargo returned **101**. This incomplete run remains in
`libc-initial.log`; no complete libc-host PASS is inferred from its partial
output. A subsequent bounded source-selected run passed **86** tests with ten
filtered; it is retained separately as `libc-new.log`. The final reusable
`run_unistd.py` narrows the exclusion to the two credential-calling secret-store
tests and passes **94 tests / 0 failures / 2 filtered**. Excluded tests have
**NOT_RUN** host coverage in the final run, not PASS.
No unrelated production code was changed to make those host tests safe.

The first expanded C++ header check failed at existing
`stdlib.h:9:19: error: invalid parameter name: template is a keyword`. The
parameter of `mkstemp` was renamed to `path_template`; its type, symbol and
behavior are unchanged. The failed run remains in
`results-2026-11-10-unistd-host/c++-base.log`. The final host run is in
`results-2026-11-10-unistd-host-final/`: **12/12** C11/C++17 checks pass across
feature macros and include orders, with exactly one canonical definition of
each selected export. Both final guest runs above were performed after this
header repair. This adds no C++ runtime implementation or original C++ pass.

Existing warnings remain visible: custom sunlightos host cfg checks, the
ChronosManifest unused terminal field, duplicate startup-fixture source targets,
standalone descriptor-harness cfg/dead-code warnings and sunlight-net module
naming. The native std check's incidental standalone probe lock refresh was
restored to its pre-check bytes; before/post/final hashes are in `probe-lock.log`.
Ordinary desktop QEMU, new native Rust runtime regression guests, and general
POSIX signal/pipe/terminal behavior are **NOT_RUN** in this phase.

**Single Phase 6 blocker/target: original `getrandom.c` still fails first at
`sys/random.h`. Add that narrow header and the matching native C `getrandom`
export around the existing `rand.rs::getrandom` production service path, then
verify its count/flags/errno and failure behavior in 2/4-CPU guests.** That Rust
function already exists but is not a C export; prior tmpfile entropy success is
not direct acceptance of this original C API probe. The other fourteen blocked
original APIs remain independently blocked. No configure-cache answer, CPython
build or broad compatibility assertion is authorized by the passing slice.

## Native C foundation, Phase 6: service-backed getrandom — 2026-11-10

Unchanged [`getrandom.c`](probes/getrandom.c) now compiles, links and passes in
isolated native SunlightOS guests on **2 and 4 CPUs**. Each actual process exits
**0**, with **empty stdout and stderr**, exactly as its original source requires.
Both suites pass all **26 built executions**, including the previous write,
termination, stdio, string and cleanup cases. Fourteen unsupported historical
APIs remain **BUILD_FAIL / guest NOT_RUN**, not observed runtime failures.

### Root cause and authoritative random path

The original source includes only `<sys/random.h>` and declares a 16-byte stack
buffer; its sole API call is `getrandom(b, sizeof b, 0)`. It needs compiler
`size_t`, signed LP64 `ssize_t`, and the canonical three-argument declaration.
It uses no flag macro or other header. Before implementation, the unchanged
probe's first failure was reproduced as `fatal error: 'sys/random.h' file not
found`. A temporary declaration-only header then exposed the separate link
failure, **undefined symbol `getrandom`**. These retained diagnostics and the
original source SHA-256 are in
[`results-2026-11-10-random-before/`](probes/native-foundation/results-2026-11-10-random-before/).

The existing Rust function was already secure by default; it lacked the C
header/export, not a randomness implementation. The implemented path is:

```text
C getrandom(void *, size_t, unsigned int)
  sunlight-libc/src/random.rs::getrandom (only flags == 0 accepted)
  sunlight-libc/src/rand.rs::getrandom(bytes, 0) -> fill_secure
    SecureEntropyReady syscall 89
    nameserver_lookup_timeout("rand", 2000 ms)
      EndpointBind 11 -> init endpoint 0, LOOKUP through capability IPC
    ipc_call_timeout: IpcSetDeadline 8 + IpcCall 1
      RandMsg::GET (0x7201), at most 32 bytes in each request/reply
  services/rand_service/src/main.rs::handle -> engine::ChaCha20::fill
    service seeding/reseeding: KernelEntropy -> readiness 89 + GetEntropy 87
  RandMsg::REPLY (0x72ff), words[0..4) -> caller buffer
```

This uses the production init nameserver and ring-3 `rand_service` already
spawned by `guest.py`/`kernel/src/native_c_guest.rs`. The service uses fresh
conditioned kernel entropy for its ChaCha20 state. The native C export never
uses `sl_std_entropy` (Rust std bootstrap), the Linux compatibility getrandom
handler, or the local xoroshiro PRNG. Its backend call hardcodes flags zero.
Rust's existing **`GRND_NONCRYPTO == 1`** is particularly important: forwarding
a C `GRND_NONBLOCK` bit of 1 would silently select the wrong generator.
All nonzero C flags are rejected before any backend call.

The historical source hash is unchanged:
`00f337b1a796f90005aa8d65569deef53d4bc8c722322a639a17637aa7b0c5f1`.
The host ABI build and both guest suites have identical historical ELF hash
`1cb1c6e1d39e8dd2adb3fb0c28e64bc4050d420690030cac2a22acb81a7ac628`.
All **26/26** per-case ELF hashes match between CPU runs; see
[`final-matrix-check.log`](probes/native-foundation/results-2026-11-10-random-host-final/final-matrix-check.log)
and `final-source-hashes.log` in that directory.

### Deliberate C ABI and changed files

The declaration is `ssize_t getrandom(void *buf, size_t buflen, unsigned int flags);`.
On native x86-64 LP64, `size_t` is **unsigned long (64 bits)**, `ssize_t` is
**signed long (64 bits)**, and flags is **unsigned int (32 bits)**. Rust's export
uses `*mut c_void`, `usize`, `c_uint` and `isize`. C11/C++17 type assertions verify
these types and the full function pointer signature across base,
`_POSIX_C_SOURCE=200809L`, and `_GNU_SOURCE`, in both header include orders.
These macro checks establish declaration availability, not POSIX conformance.

| Input / result | C behavior |
| --- | --- |
| flags zero, valid nonempty writable destination | return filled byte count from the existing secure service path; preserve errno |
| flags nonzero, including 1, 2, 3, 4, high bit and all bits | **-1/EINVAL**, even for zero length; no backend access |
| count above `isize::MAX` | **-1/EINVAL**, no buffer or backend access |
| nonzero count with NULL | **-1/EFAULT**, no backend access |
| zero count, flags zero, including NULL or pointer 1 | **0**, preserve errno, no dereference/slice construction or backend access |
| secure Rust backend returns its current undifferentiated -1 | **-1/EIO**, through the existing TLS errno mechanism; never misread -1 as -EPERM |
| impossible successful backend count above request | **-1/EIO**, never publish an invalid count |

No `GRND_NONBLOCK`, `GRND_RANDOM` or Rust-only noncrypto macro is advertised in
the C header. The current service protocol has no nonblocking or alternate-pool
request contract. Service readiness/lookup/timeout/protocol errors remain
undifferentiated by the existing secure Rust API; no invented detailed errno
or predictable fallback is supplied. Existing failures wipe the destination.

Files changed in this phase:

* `sunlight-libc/include/sys/random.h`: new narrow ABI and explicit limits.
* `sunlight-libc/include/unistd.h`: share a guarded `ssize_t` typedef with the
  new header; existing write declaration and behavior are unchanged.
* `sunlight-libc/src/random.rs`, `src/lib.rs`: one canonical C export, enabled
  for native C builds, argument/result adapter and six focused production-module
  unit tests. The Rust random implementation and private PRNG flag are unchanged.
* `tools/native-c/tests/random.c`: four focused guest modes and an independent
  STATS observer using the actual native register-IPC ABI and nameserver hash.
* `tools/native-c/tests/random-abi.c`, `tests/run_random.py`: twelve C/C++
  type/header checks, production libc host tests, canonical-symbol uniqueness,
  original-probe link/ELF inspection and disassembly evidence.
* `tools/native-c/guest.py`: bracket unchanged historical getrandom with two
  STATS processes, add the four separate contracts, and require the historical
  service-counter delta for suite acceptance. Historical source/expected streams
  and the guest deadline remain unchanged.
* `tools/native-c/README.md`, `docs/audit/std-progress.md`: contracts and evidence.

No kernel, syscall, loader, IPC protocol, random-service implementation,
allocator or TLS changes were needed. Pre-existing worktree changes are outside
this phase's edit list.

### Guest verification and direct service evidence

Exact top-level commands, from the repository root:

```sh
python3 tools/native-c/tests/run_random.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-random-host-final
python3 tools/native-c/guest.py --suite --cpus 2 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-random-final-2cpu
python3 tools/native-c/guest.py --suite --cpus 4 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-random-final-4cpu
```

Each suite explicitly runs `tools/build_native_c.sh`, then `tools/native-c/cc.py
-std=c11 -Wall -Wextra -Werror` on the **original** `docs/audit/probes/getrandom.c`.
Exact expanded compiler/link, support-service, kernel, ISO and QEMU commands
are retained in each run's logs, especially `build-getrandom.log`,
`support-build.log`, `kernel-build.log`, `iso-build.log` and `qemu.log`.
The ABI runner retains an additional direct original build in
`historical-build.log`, its ELF inspection, undefined-symbol check and disassembly.

Raw guest evidence:
[`2-CPU serial.log`](probes/native-foundation/results-2026-11-10-random-final-2cpu/serial.log),
[`4-CPU serial.log`](probes/native-foundation/results-2026-11-10-random-final-4cpu/serial.log).
The following records are exact; the first runner line is from each CPU run,
and the remaining records appear in both:

```text
[NATIVE-C-RUNNER] cases=26 cpus=2 entropy=true
[NATIVE-C-RUNNER] cases=26 cpus=4 entropy=true
[NATIVE-C-SUPPORT] nameserver=production rand=production receive_wait=ready
[NATIVE-C-START] case=5 name=getrandom pid=105 bytes=20760 owned_frames=516 uid=1000
[NATIVE-C-REAP-END] case=5 exit=0 user_frames=517 page_tables=10 owned_frames=0 fds=0 stack=0
[NATIVE-C-RESULT] case=5 exit=0 stdout=0 stderr=0 clean=1 file_before=0 file_after=0 private_before=1036 private_after=1036 pt_before=16 pt_after=16 handles_before=0 handles_after=0
[NATIVE-C-DONE] cases=26
```

Independent observer processes query existing `RandMsg::STATS` immediately
before and after the unchanged historical ELF, with no intervening workload.
Their decoded stdout is exactly:

```text
rand service: requests=0 bytes=0
rand service: requests=1 bytes=16
```

Both runs' `random-service.log` records:

```text
snapshots=[(0, 0), (1, 16)]
expected_delta=requests:1,bytes:16
unchanged_getrandom_service_backing=PASS
```

This verifies actual service consumption by that historical process, beyond
observing successful output or merely booting the service. STATS reads expose
only counters/readiness, never keys, seeds, capability tokens or random bytes.
Only words 0 through 3 cross the existing register transport; the observer does
not use the protocol's untransmitted last-reseed/policy fields.

| Additional guest contract | Verified behavior | 2 CPUs | 4 CPUs |
| --- | --- | --- | --- |
| random-basic | 16 bytes, errno preserved, guard bytes unchanged, service delta 1 request / 16 bytes | PASS | PASS |
| random-zero | NULL and pointer 1 with zero count; errno preserved, service delta 0 / 0 | PASS | PASS |
| random-errors | NULL/nonzero EFAULT, huge count and six unsupported flag combinations EINVAL, unchanged canaries, service delta 0 / 0 | PASS | PASS |
| random-guards | lengths 1, 16, 31, 32, 33, 65, 257; prefix/tail guards, errno, service delta 18 requests / 435 bytes | PASS | PASS |
| two service observers | exact original-process counter delta above | 2 PASS | 2 PASS |

Every built process passes ordinary reaping and physical-frame/descriptor/stack
cleanup checks. Each QEMU process exits **33** through the debug-exit transport;
each host suite command exits **0**. There are no fault/panic/unhandled-syscall
records. This is native uid/gid-1000 execution, not Linux execution of a native
ELF or a Linux compatibility personality.

### Host regressions and retained attempts

```sh
python3 tools/native-c/tests/run_stdio.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-random-stdio
bash tools/test-mem-string-proof.sh
rustc --edition=2021 --test tools/native-std/tests/futex.rs \
  -o docs/audit/probes/native-foundation/results-2026-11-10-random-regressions/futex-tests
docs/audit/probes/native-foundation/results-2026-11-10-random-regressions/futex-tests
bash tools/build_native_std.sh check \
  --manifest-path tools/native-std-probe/Cargo.toml --release
cargo check -p sunlight-kernel
```

| Regression | Result and evidence |
| --- | --- |
| libc production modules | **100 PASS / 0 FAIL / 2 filtered**, including six new random tests; `random-host-final/libc.log` |
| native C/C++ ABI | **12 PASS**, base/POSIX/GNU and both header orders; exactly one definition of getrandom, write, atexit, exit, startup, std entropy and temp exports |
| actual filesystem + kernel descriptor/VFS | **90 + 17 PASS**, `random-stdio/host-vfs.log`, `host-fd-vfs.log` |
| stdio/temp | **19 PASS**, both existing C host adapters PASS; three native ELF checks and C++ headers PASS, `random-stdio/` |
| memory/string | **24 PASS**, native codegen/C ABI/C++ checks PASS, `random-regressions/memory-string.log` |
| futex | **3 PASS**, `random-regressions/futex.log` |
| native Rust std | pinned probe check **PASS**, exit 0, `random-regressions/native-std.log` |
| ordinary-feature kernel | **PASS**, exit 0, 125.69 seconds; `random-regressions/kernel-check.log` |

The complete unit-test total above is **253 PASS**. The two existing secret-store
tests that issue native getuid/getgid remain filtered out on Linux, with
**NOT_RUN** host coverage. Unit failure injection covers EIO and the shared
structured EAGAIN mapping without claiming a guest service timeout test.
The native std check's incidental probe lockfile refresh was restored to its
pre-check bytes; `random-regressions/probe-lock.log` records all three hashes.
Existing custom-target cfg, ChronosManifest, duplicate-target, standalone-harness
and net-module naming warnings remain visible.

Retained earlier attempts are separate evidence:

* `results-2026-11-10-random-host/`: initial host compilation failed because the
  new test's `byte = 0` inferred i32 before a pointer cast. It was corrected to
  `0u8`; all final host tests were rerun. This was a test compilation failure,
  not a production getrandom failure.
* `results-2026-11-10-random-2cpu/`: the original probe actually exited 0 with
  correct empty streams, but the new counter observers exited 61 because their
  test-only lookup supplied packed ASCII instead of `ipc::name_to_u64`'s FNV-1a
  hash. The suite therefore failed service-backed acceptance. Correcting that
  observer, without changing libc or the service, yielded the separate final
  2/4-CPU runs above. The failed attempt's results and logs remain intact.

### Historical counts, remaining limits and the single Phase 7 blocker

| Original population | Phase 5 | Phase 6 native build | Phase 6 guest, each CPU count |
| --- | --- | --- | --- |
| 18 original API probes (17 C + one C++) | 3 PASS / 15 BUILD_FAIL | **4 PASS / 14 BUILD_FAIL** (C: 4/13; C++: 0/1) | **4 PASS / 0 FAIL / 14 NOT_RUN** |
| 21 historical executables, including three foundation probes | 6 guest PASS / 15 NOT_RUN | **7 PASS / 14 BUILD_FAIL** | **7 PASS / 0 FAIL / 14 NOT_RUN** |
| 16 originally blocked APIs | 1 PASS / 15 BUILD_FAIL | **2 PASS / 14 BUILD_FAIL** | **2 PASS / 0 FAIL / 14 NOT_RUN** |

The four new contracts and two observers are separate from historical counts.
The existing thirteen focused Phase 4/5 cases also pass again. All historical
sources, earlier counts and failure logs are preserved.

Only flags zero is supported. Arbitrary bad **non-NULL** or read-only mappings
are not recoverable as EFAULT by this userspace IPC adapter: nonempty callers
must supply valid writable storage. NULL/nonzero memory is explicitly tested
as EFAULT; pointer 1 is exercised only with zero length or invalid flags, never
passed as writable storage. Full invalid-mapping recovery requires a deliberate
kernel copy-to-user facility; no such syscall was added or simulated here.
No global request deadline beyond the existing per-chunk deadline, new service
restart/concurrent-client matrix, ordinary desktop guest, or new native Rust
runtime regression guest is established. There is no broad POSIX or CPython
readiness claim and no CPython configure/build was attempted.

**Single Phase 7 blocker/target: original `clock.c` still fails first at missing
`<time.h>`. Define and verify its narrow clock/sleep C surface (`struct timespec`,
`CLOCK_REALTIME`, `CLOCK_MONOTONIC`, `clock_gettime`, `nanosleep`) using the native
clock and scheduler paths, then run that unchanged probe on 2 and 4 CPUs.**
The retained final `build-clock.log` records the concrete missing-header error;
the other thirteen unsupported original APIs remain independently build-blocked.

## Native C foundation, Phase 7: native clocks and relative sleep — 2026-11-10

Unchanged [`clock.c`](probes/clock.c) now compiles, links and passes in isolated
native guests on **2 and 4 CPUs**. Each actual process exits **0** with **empty
stdout and stderr**, clean reaping and no unexpected faults/unknown syscalls.
Each suite passes **31/31 built executions**: the previous 26, the original
clock probe, and four separate time contracts. The thirteen remaining blocked
historical APIs are **BUILD_FAIL / guest NOT_RUN**, not guest failures.

### Original failure and exact requirements

The original source is exactly:

```c
#define _GNU_SOURCE
#include <time.h>
int main(void){struct timespec a,b,d={0,1000000}; return clock_gettime(CLOCK_REALTIME,&a)||clock_gettime(CLOCK_MONOTONIC,&b)||nanosleep(&d,0);}
```

It requires `<time.h>`, a complete `struct timespec` with seconds followed by
nanoseconds, `CLOCK_REALTIME`, `CLOCK_MONOTONIC`, the two-argument `clock_gettime`
declaration and the two-pointer `nanosleep` declaration. The positional request
initializer means **0 seconds + 1,000,000 ns (1 ms)**. It supplies NULL remainder
storage, short-circuits on any failure, and expects no output. `_GNU_SOURCE` is
defined before inclusion; the implementation also verifies base and
`_POSIX_C_SOURCE=200809L` inclusion. It does not call `clock`, `time`,
`clock_getres`, `clock_nanosleep`, `sleep`, timezone/calendar or CPU-time APIs.
`clock_t` and its units are not exercised by this probe.

The original missing-header diagnostic is retained in
`probes/results/native-clock-build.log` and the Phase 6 `build-clock.log`.
This phase reproduced **`fatal error: 'time.h' file not found`** before edits.
A temporary declaration-only header then exposed **undefined symbol `nanosleep`**;
the existing `clock_gettime` export already linked. Both new diagnostics are in
[`time-before/`](probes/native-foundation/results-2026-11-10-time-before/),
without modifying the historical source. Its unchanged SHA-256 is
`eee7fb2d2111f4799e69eedb698e836cf6c5546539843f1b92f5f88ebb7c0a2a`.

### Verified production paths and ABI decisions

```text
C clock_gettime(clockid_t, struct timespec *)
  sunlight-libc/src/time.rs::clock_gettime -> private KernelTimespec
  native ClockGetTime syscall 88 (clock ID in rdi, private output in rsi)
    0: kernel/src/arch/x86_64/rtc.rs::unix_time
       validated boot CMOS UTC epoch + canonical tick advancement
    1: kernel/src/timekeeping.rs::monotonic_ns
       BSP-owned global ticks * NS_PER_TICK (100 Hz; 10,000,000 ns/tick)
  validate signed seconds/nanoseconds -> publish C Timespec only on success

C nanosleep(const struct timespec *, struct timespec *)
  copy/validate request; checked relative duration -> checked monotonic deadline
  existing native clock path above (ID 1); zero duration skips clock and wait
  sunlight-libc/src/sys.rs::native_futex, shared with std_runtime::sl_std_futex
    native syscall 155 WAIT: private AtomicU32=0, absolute deadline in r10
    consume RESULT after registered completion (0=wake, 2=timeout)
  kernel sys_native_futex -> BlockedOnIo + native_futex_deadline
  scheduler expire_native_futex_timeouts -> complete_futex_wait(2) -> Ready
  re-read monotonic time after each wake; retry until original deadline
```

Rust `Instant`/`SystemTime` already use `sl_std_clock` and the same libc reader.
Rust `thread::sleep` already loops over that monotonic clock and native futex;
this phase factors its existing WAIT/RESULT syscall transport into one shared
helper, preserving Rust's return values. C performs checked conversions rather
than Rust std's saturating timeout conversions, because `u64::MAX` means an
indefinite native wait. There is no new clock state, busy-wait approximation,
Linux-personality sleep call or Linux host time source in this implementation.

`services/timer_server` accumulates kernel tick events and answers GET_TICKS;
it offers no sleep operation. `services/timezone_service` applies local civil
offsets to kernel UTC; it is not the authoritative epoch clock. Neither service
is needed by syscall 88 or native futex deadlines, and neither was changed or
added to the isolated fixture. Realtime has **one-second granularity**, with
`tv_nsec == 0`; monotonic has **10 ms granularity**. The guests verify native
source agreement/normalization, not external UTC accuracy. The existing RTC
path reports unavailable time rather than inventing an epoch; a raw generic
syscall error maps to EIO through libc.

| C surface | Native x86-64 LP64 decision |
| --- | --- |
| `time_t`, `clock_t` | signed `long`, 8 bytes; `clock_t` is type-only, with no `clock()`/`CLOCKS_PER_SEC` claim |
| `clockid_t` | signed `int`, 4 bytes; only IDs 0 and 1 supported |
| `struct timespec` | `time_t tv_sec` at offset 0; signed `long tv_nsec` at offset 8; size 16, alignment 8 |
| Rust boundary | existing `#[repr(C)] Timespec { i64, i64 }`; kernel wire `{ u64, u64 }` stays private |
| declarations | `int clock_gettime(clockid_t, struct timespec *); int nanosleep(const struct timespec *, struct timespec *);` |
| feature macros | declarations available in base/POSIX/GNU configurations; no POSIX version or option macros advertised |
| success | return 0, preserve errno; clock output published only after full validation |
| clock errors | unsupported ID or unrepresentable/unnormalized data: -1/EINVAL; NULL output: -1/EFAULT |
| request errors | NULL: -1/EFAULT; negative seconds, nanoseconds outside `[0, 1000000000)`, duration/deadline overflow or indefinite sentinel: -1/EINVAL |
| sleep completion | relative monotonic deadline, retry early wakes, zero request does no wait; request and remainder unchanged |
| backend errors | existing structured errno mapping, including generic EIO and EAGAIN; no fabricated EINTR |

Finite nanosecond conversion accepts at most **18,446,744,073 s + 709,551,614 ns**
(`u64::MAX - 1`); a nonzero current time further reduces the largest acceptable
duration. Checked addition rejects both wraparound and the `u64::MAX` sentinel.
Host tests cover both endpoints, second/nanosecond boundaries, malformed kernel
data, syscall errors and output canaries. Guest error tests independently check
five invalid IDs, both NULL clock outputs, NULL sleep input, negative/oversized
nanoseconds and conversion/deadline overflow, including errno and unchanged
remainder. Successful sleep tests include req/rem aliasing.

Non-NULL clock outputs and sleep requests must designate valid writable/readable
storage, respectively. These userspace adapters cannot recover arbitrary mapping
faults as EFAULT; no kernel copy syscall was added. Such invalid-mapping tests
are **NOT_RUN**. Native futex offers wake/timeout, not a signal-interruption
result. Completed sleeps leave `rem` untouched, as verified in both guests;
**interrupted sleep / nonzero remainder is unsupported and NOT_RUN**. Successful
completion must not be interpreted as proof of POSIX signal semantics.

### Commands, build identity and guest evidence

Exact top-level build/ABI and guest commands (repository root):

```sh
python3 tools/native-c/tests/run_time.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-time-host-v2
python3 tools/native-c/guest.py --suite --cpus 2 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-time-final-v2-2cpu
python3 tools/native-c/guest.py --suite --cpus 4 \
  --out docs/audit/probes/native-foundation/results-2026-11-10-time-final-v2-4cpu
```

The host runner and each suite explicitly run `bash tools/build_native_c.sh`,
then build the original source with:

```sh
python3 tools/native-c/cc.py -std=c11 -Wall -Wextra -Werror \
  docs/audit/probes/clock.c -o OUTPUT/clock.elf
llvm-readelf -h -l OUTPUT/clock.elf
llvm-nm --undefined-only OUTPUT/clock.elf
llvm-objdump -d OUTPUT/clock.elf
```

Here `OUTPUT` is the exact evidence directory from the command above; expanded
absolute compiler/ELF commands are in each `historical-*.log` / `build-clock.log`
and `elf-clock.log`. Host validation asserts ELF64 little-endian native x86-64
ET_EXEC, canonical `_start`, executable entry, valid LOAD bounds/alignment,
W^X, non-executable stack, no interpreter/dynamic segment or unresolved symbol.
The archive has exactly one canonical `clock_gettime`, `nanosleep`,
`sl_std_clock`, `sl_std_futex` and each checked existing runtime export.

The historical ELF SHA-256 is
`b406cad38b9bece84a8acfa660052064bc067e380cc6d7834d66d2c62802180b`.
Host-build, 2-CPU and 4-CPU copies are byte-identical; all **31/31** per-case
ELF hashes match across the CPU runs. Identity and matrix checks are recorded
in `time-regressions/matrix-check-final.log` (directory prefix
`results-2026-11-10-` throughout this record). Final source hashes are retained
in `time-regressions/final-source-hashes-v2.log`.

Raw evidence:
[`2-CPU serial.log`](probes/native-foundation/results-2026-11-10-time-final-v2-2cpu/serial.log),
[`4-CPU serial.log`](probes/native-foundation/results-2026-11-10-time-final-v2-4cpu/serial.log).
The first two records below identify each run; the remaining records occur in
both:

```text
[NATIVE-C-RUNNER] cases=31 cpus=2 entropy=true
[NATIVE-C-RUNNER] cases=31 cpus=4 entropy=true
[NATIVE-C-START] case=4 name=clock pid=104 bytes=20552 owned_frames=516 uid=1000
[NATIVE-C-REAP-END] case=4 exit=0 user_frames=517 page_tables=10 owned_frames=0 fds=0 stack=0
[NATIVE-C-RESULT] case=4 exit=0 stdout=0 stderr=0 clean=1 file_before=0 file_after=0 private_before=1036 private_after=1036 pt_before=16 pt_after=16 handles_before=0 handles_after=0
[NATIVE-C-DONE] cases=31
```

| Guest contract / native-clock measurement | 2 CPUs | 4 CPUs |
| --- | --- | --- |
| unchanged `clock.c`, exit 0 / empty streams / clean reap | PASS | PASS |
| time-clocks: normalized IDs, raw native 81/86 source agreement, 256 ordered reads | PASS | PASS |
| time-errors: invalid IDs/pointers/ranges/overflow, errno/canaries | PASS | PASS |
| time-zero: success/errno preservation, untouched rem and alias | PASS | PASS |
| sleep 1 ns | 10 ms | 10 ms |
| sleep 1 ms | 10 ms | 10 ms |
| sleep 25 ms | 30 ms | 30 ms |
| sleep 1 s + 1 ns | 1010 ms | 1010 ms |

All sleep cases pass `requested <= native elapsed <= requested + 5 s` and leave
request/remainder unchanged. These measurements use the same coarse native
clock domain as the deadline; they establish neither physical sub-tick sleep
accuracy nor a universal scheduling-latency bound. The 1 ns request is an
accepted argument, not evidence of 1 ns resolution. Exact captured values are
in each `time-sleep.stdout.txt`; `time-clocks.stdout.txt` retains the native
epoch and monotonic readings. Each suite exits **0**, QEMU exits **33** through
debug-exit, and `faults.log` is empty. The previous getrandom service counter
delta remains **1 request / 16 bytes** in each `random-service.log`; Phase 6
still uses the production rand service.

### Host and build regressions

```sh
python3 tools/native-c/tests/run_stdio.py \
  --out docs/audit/probes/native-foundation/results-2026-11-10-time-stdio
bash tools/test-mem-string-proof.sh
rustc --edition=2021 --test tools/native-std/tests/futex.rs \
  -o docs/audit/probes/native-foundation/results-2026-11-10-time-regressions/futex-tests
docs/audit/probes/native-foundation/results-2026-11-10-time-regressions/futex-tests
bash tools/build_native_std.sh check \
  --manifest-path tools/native-std-probe/Cargo.toml --release
cargo check -p sunlight-kernel
```

| Verification domain | Final result | Evidence under `probes/native-foundation/results-2026-11-10-` |
| --- | --- | --- |
| host production libc | **107 PASS / 0 FAIL / 2 filtered**; includes twelve time tests, seven new | `time-host-v2/libc.log` |
| native C11/C++17 ABI/build | **12 PASS** across base/POSIX/GNU and both include orders; unique exports; unchanged clock ELF validation PASS | `time-host-v2/` |
| host actual filesystem / kernel descriptor-VFS logic | **90 + 17 PASS** | `time-stdio/host-vfs.log`, `host-fd-vfs.log` |
| host stdio/temp and C adapters | **19 PASS**; both existing C adapters PASS; three native builds/ELF checks and C++ headers PASS | `time-stdio/` |
| host memory/string | **24 PASS**, codegen/C ABI/C++ checks PASS | `time-regressions/memory-string.log` |
| host Rust std futex adapter logic | **3 PASS** | `time-regressions/futex.log` |
| native Rust std probe build check | **PASS**, exit 0 | `time-regressions/native-std-final.log` |
| ordinary-feature kernel check | **PASS**, exit 0, 148.64 s | `time-regressions/kernel-check-final.log` |
| native 2-CPU suite | **31 PASS / 0 guest FAIL**, historical **8 PASS / 13 BUILD_FAIL / 13 guest NOT_RUN** | `time-final-v2-2cpu/results.csv`, `serial.log` |
| native 4-CPU suite | **31 PASS / 0 guest FAIL**, historical **8 PASS / 13 BUILD_FAIL / 13 guest NOT_RUN** | `time-final-v2-4cpu/results.csv`, `serial.log` |

The host unit-test total is **260 PASS**. The two pre-existing secret-store
tests that issue native credential syscalls are explicitly filtered on Linux
and remain **host NOT_RUN**. No test was stopped or filter-rejected in this
phase. Backend EAGAIN/EIO and early-wake behavior are verified by host injection;
actual guest backend failure injection and deliberately forced early wakes are
**NOT_RUN**. `time-host/` and `time-host-final/` retain earlier passing host runs; the
final runner adds explicit ELF assertions and symbol/identity logs.
The first format check required one line wrap in the shared syscall helper.
After formatting, the ELF hash comparison against the earlier artifact failed:
`.text`, `.rodata` and `.eh_frame` were identical, while `.symtab`/`.strtab`
changed (`time-regressions/format-attempt.log`, `format-artifact-diff.log`).
No guest failed. The final **v2 host and both full guest suites were rerun**
against the formatted source; the final artifact identity above is verified
independently of those earlier runs. The final format check passes.
The native Rust std check's incidental probe lockfile refresh was restored to
the original bytes; before/after/restored hashes are in
`time-regressions/probe-lock-final.log`. Existing custom-target cfg, ChronosManifest,
duplicate startup-target, standalone-harness and sunlight-net naming warnings
remain visible. These are build/host checks, separate from the native C guests;
no native Rust runtime guest success is inferred.

### Changed files and remaining blocker

* `sunlight-libc/include/time.h`: narrow types/constants/declarations and explicit limits.
* `sunlight-libc/src/time.rs`: reuse clock reader, checked relative sleep, seven new production-module host tests.
* `sunlight-libc/src/sys.rs`, `src/std_runtime.rs`: one shared existing native futex WAIT/RESULT transport.
* `sunlight-libc/src/lib.rs`: time module documentation.
* `tools/native-c/tests/time.c`, `time-abi.c`, `run_time.py`: four native guest modes, twelve C/C++ ABI checks, production host tests and strict original-ELF validation.
* `tools/native-c/guest.py`: add four separate focused cases; historical source/status/streams and the 90-second deadline are unchanged.
* `tools/native-c/README.md`, this audit, and retained evidence directories.

No kernel, syscall numbering, service implementation, Rust std overlay,
allocator, TLS, loader or historical probe changes were required. No CPython
configure/build, broad POSIX time implementation, ordinary desktop guest,
native Rust runtime guest, realtime stepping/accuracy test, or signal/EINTR
test was attempted; these are **NOT_RUN**.

| Original population | Phase 6 native build | Phase 7 native build | Phase 7 guest, each CPU count |
| --- | --- | --- | --- |
| 18 original APIs (17 C + one C++) | 4 PASS / 14 BUILD_FAIL | **5 PASS / 13 BUILD_FAIL** (C: 5/12; C++: 0/1) | **5 PASS / 0 FAIL / 13 NOT_RUN** |
| 21 historical executables including three foundation probes | 7 PASS / 14 BUILD_FAIL | **8 PASS / 13 BUILD_FAIL** | **8 PASS / 0 FAIL / 13 NOT_RUN** |
| 16 originally blocked APIs | 2 PASS / 14 BUILD_FAIL | **3 PASS / 13 BUILD_FAIL** | **3 PASS / 0 FAIL / 13 NOT_RUN** |

The four new time contracts do not inflate historical counts. Earlier failures
and Phase 6 counts remain intact.

**Single Phase 8 blocker/target: unchanged `mmap.c` still fails at missing
`<sys/mman.h>`**, independently reproduced in both final `build-mmap.log` files.
Its actual requirements are a 1 MiB private anonymous RW mapping, writing its
last byte, `mprotect(..., PROT_READ)`, then `munmap`, plus the corresponding
constants, signatures and `MAP_FAILED`. The existing Rust mmap/munmap path and
native protection syscall are implementation leads, not evidence that the C
ABI already works. Define/verify that narrow C surface, retaining protection
and overflow rules, then run that unchanged ELF on 2 and 4 CPUs. This phase
does not implement it or claim acceptance from a missing-header failure.


## Phase 8 — existing mmap ABI gap audit (2026-11-10)

The unchanged historical `docs/audit/probes/mmap.c` was inspected first with
its Phase 7 `build-mmap.log`: compilation stopped at missing `<sys/mman.h>`.
Its exact request is `mmap(NULL, 1048576, PROT_READ|PROT_WRITE,
MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)`, a last-byte write,
`mprotect(p, 1048576, PROT_READ)`, then `munmap(p, 1048576)`.
No historical probe was edited.

Repository-wide search found genuine native protection support: dispatch 52
in `kernel/src/arch/x86_64/syscall.rs`, `process::mmap::sys_mprotect`, expected
PTE permission updates in `process/address_space.rs`, and TLB invalidation.
There is **no missing VM operation blocker** for this probe. The existing
`SYS_MPROTECT` constant also already existed. No kernel code was changed.

The additions are the narrow C header and a target-gated `mprotect` C adapter
calling syscall 52. Existing Rust mmap/munmap wrappers, request validation and
target-gated exports are preserved. Static-archive symbol inspection confirms
exactly one `mmap`, `munmap` and `mprotect`; no additional export registry is
needed. The header declares signed LP64 `off_t`, `size_t` lengths and `int`
flags/protections. `MAP_NATIVE_THREAD` remains absent from the public header.
C/C++ header checks assert signatures, constant values, types, repeat inclusion
and include order under base, POSIX and GNU feature settings.

The C subset retains private anonymous mappings only, fd=-1, offset=0,
no nonfixed address hints, readable protection requests and W^X. PROT_NONE,
write-only, execute-only, RWX, unknown bits, shared/file mappings and nonzero
offsets are not advertised as supported. New mprotect validation follows the
same narrow protection subset, alignment and overflow rules. Success preserves
errno; local invalid requests return EINVAL; native VM errors remain generic
EIO. The adapter never reports a synthetic protection-change success.

Bounds ordering was reviewed before guest validation. In
`kernel/src/process/mmap.rs`, `checked_page_layout` and `UserRange` precede
`preflight_replace`/`map_fixed_transaction`; munmap checks the full range before
ledger preflight, retirement staging and leaf removal; mprotect checks the
full range, ledger coverage/policy, present PTE ownership and complete ledger
plan before its first permission update. The bounds fixture bypasses libc
using raw native syscalls 50/51/52 and submits both size overflow and a span
crossing the user-address limit from an existing valid page. Every request
fails while the original sentinel and writability remain intact.

Fresh evidence lives under `docs/audit/probes/native-foundation/`:

| Check | Result | Evidence directory/file |
|---|---|---|
| production libc host unit tests | **107 PASS**, 2 existing native-credential tests filtered on Linux | `results-2026-11-10-mman-host/libc.log` |
| twelve C11/C++17 ABI/header combinations | **PASS** | `results-2026-11-10-mman-host/c*.log` |
| unique canonical exports, unchanged-source native ELF and disassembly | **PASS**, no undefined symbols or host personality | `results-2026-11-10-mman-host/historical-*.log`, `native-symbols.log` |
| full 2-CPU guest suite | **35 PASS / 0 guest FAIL** | `results-2026-11-10-mman-2cpu/results.csv`, `serial.log` |
| full 4-CPU guest suite | **35 PASS / 0 guest FAIL** | `results-2026-11-10-mman-4cpu/results.csv`, `serial.log` |

Both guests execute the unchanged historical mmap ELF with SHA-256
`c23f3a1c6707e51adff60725357631640512cf3b6ca84fde8d6da8a33c5899ef` and exit 0.
The three additional fixtures are counted separately: `mman-subset`,
`mman-fixed`, `mman-bounds`, all exit 0 on both CPU counts with clean reap,
physical-frame and descriptor checks. The subset checks all 1 MiB for zero fill,
last-byte writes, and a real RW-to-R-to-RW transition: independent syscall 88
cannot copy its timespec to the read-only first page, then succeeds after RW
restoration. This observes kernel writable-PTE enforcement; hardware user-write
faults and forced stale-TLB/cross-core interleavings are **NOT_RUN** here.
The fixed fixture replaces the middle of three anonymous pages at the exact
address, observes zeroed replacement contents and intact neighbors. This is
fresh kernel execution of that MAP_FIXED case, not a claim of exhaustive fixed
replacement transaction or failure coverage.

Counts below were updated only after both guest runs passed:

| Historical scope | Phase 7 | Phase 8 build and guest acceptance |
|---|---|---|
| 18 API probes (17 C + 1 C++) | 5 PASS / 13 BUILD_FAIL | **6 PASS / 12 BUILD_FAIL / 12 guest NOT_RUN** |
| 21 executables including foundation | 8 PASS / 13 BUILD_FAIL | **9 PASS / 12 BUILD_FAIL / 12 guest NOT_RUN** |
| 16 originally blocked APIs | 3 PASS / 13 BUILD_FAIL | **4 PASS / 12 BUILD_FAIL / 12 guest NOT_RUN** |

The additional fixtures do not inflate historical counts. Earlier results are
retained. `mman-host/historical-before.log` preserves the original diagnostic;
`historical-unchanged.log`, `source-hashes.log` and `vm-audit.log` record source
identity and reviewed VM paths. Formatting, Python compilation and source
whitespace checks pass. Captured tool logs/CSV retain their original whitespace.


## Phase 9 — native C directory enumeration bridge (2026-11-10)

This entry was appended only after both final guest suites passed. Phase 8
does not explicitly name a Phase 9 target. The target was selected from its
retained current matrix, `results-2026-11-10-mman-2cpu/results.csv`: the first
remaining historical BUILD_FAIL is **`docs/audit/probes/dirent.c`**. Its exact
first actionable diagnostic in `build-dirent.log` is
`dirent.c:2:10: fatal error: 'dirent.h' file not found`, reproduced in the
4-CPU build log. The diagnostic is copied to `dirent-host/historical-before.log`
below. Target and starting counts were reported before implementation: 18 APIs
**6 PASS / 12 BUILD_FAIL**, 21 historical executables **9 guest PASS / 0 FAIL /
12 NOT_RUN**, 16 originally blocked APIs **4 PASS / 12 BUILD_FAIL**; both
complete Phase 8 suites **35 guest PASS**.

The unchanged source defines `_GNU_SOURCE`, includes `<dirent.h>`, declares
`DIR *d`, calls `opendir(".")`, requires a non-NULL result, tests
`readdir(d) == 0`, calls `closedir(d)` and returns that boolean. It requires
one genuine entry. It uses no entry fields, flags, directory descriptor,
seek/rewind, inode, type constants, or mutation behavior. No historical source
or expected status/output was changed. Source SHA-256 remains
`b77440b252b906b6190de25a6ac0ffb686f88eaa292a146cd529783c5a376bb8`.

Source tracing found an existing native capability, not a missing enumeration
operation: `sunlight-libc/src/lib.rs` already provides the 80-byte `DirEntry`,
`read_dir` (60), `read_dir_from` (148), `stat` (61) and `getcwd` (64).
`kernel/src/arch/x86_64/syscall.rs::sys_readdir` checks real VFS read permission,
enumerates `KERNEL_VFS`, skips the requested entry offset and copies actual
records to userspace; `sys_stat_path` supplies directory type. The production
native path is direct kernel VFS, not the userspace `vfs_server` service or a
Helios/Linux adapter. VFS requires absolute paths without dot components;
the existing getcwd wrapper supplies the actual process CWD for `"."`.
Rust std's `tools/native-std/overlay/fs/sunlightos.rs::readdir` still returns
unsupported; this phase does not claim to implement that separate interface.

The missing C header/export slice is now implemented. `DIR` is opaque;
`struct dirent` exposes only `char d_name[65]`, enough for the native 64-byte
name plus NUL, and no invented inode/type metadata. `opendir` validates the
bounded pathname, obtains CWD for `"."`, checks directory type and prefetches
one actual record through syscall 148. Its heap allocation contains its own
absolute path, offset and returned entry. `readdir` advances by native record
offset, including beyond 64 entries, and distinguishes native errors from EOF.
`closedir` frees that state; no descriptor was acquired. No synthetic dot
entries or success results are inserted. Existing Rust helpers, production
syscalls, VFS, loader and permission policy remain unchanged.

The supported path subset is `"."` and canonical absolute paths. Other relative
paths, dot components, repeated/trailing separators fail **ENOSYS**, rather than
being misresolved. NULL path is EFAULT, empty path ENOENT, a path without NUL
within 256 bytes E2BIG, a nondirectory ENOTDIR, failed allocation ENOMEM, and
NULL stream EBADF. Native errors use the existing errno mapping; enumeration
can return generic EIO where the kernel lacks a structured error. Malformed
native records/counts and offset overflow fail EIO without advancing state.
Success and EOF preserve errno. Callers must provide valid C storage and live
streams and serialize operations on each stream. Directory descriptors,
inode/type fields, seek, stable snapshots and concurrent directory mutation
semantics are not supplied by this header. Each read rescans by entry offset;
no large-directory performance claim is made.

All evidence paths below are under `docs/audit/probes/native-foundation/`, with
the common `results-2026-11-10-` prefix.

| Check | Result | Evidence suffix |
|---|---|---|
| production libc host unit tests | **114 PASS / 0 FAIL / 2 filtered**; seven new directory tests | `dirent-host-final/libc.log` |
| native C11/C++17 types, sizes, signatures, inclusion orders and base/POSIX/GNU macros | **12 PASS** | `dirent-host-final/c*.log` |
| runtime, unique exports and unchanged-source native ELF | **PASS**, canonical startup, no unresolved symbols, interpreter, dynamic segment or W+X LOAD | `dirent-host-final/build-runtime.log`, `native-symbols.log`, `historical-*.log` |
| exact validated ELF in single-case 2-CPU guest | **1 PASS**, exit 0, empty stdout/stderr, clean reap, zero faults/unknown syscalls | `dirent-original-2cpu/` |
| same exact ELF in single-case 4-CPU guest | **1 PASS**, same status/streams/cleanup checks | `dirent-original-4cpu/` |
| final complete 2-CPU suite | build **39 PASS / 11 BUILD_FAIL**; guest **39 PASS / 0 FAIL / 11 NOT_RUN** | `dirent-final-2cpu/results.csv`, `serial.log`, `faults.log` |
| final complete 4-CPU suite | build **39 PASS / 11 BUILD_FAIL**; guest **39 PASS / 0 FAIL / 11 NOT_RUN** | `dirent-final-4cpu/results.csv`, `serial.log`, `faults.log` |
| mmap, time, random, write/termination host/ABI regressions | **PASS**, twelve header checks for each; repeated libc runs remain 114 PASS / 2 filtered, not additional unique tests | `dirent-regressions/{mman,time,random,unistd}/` |
| VFS, descriptor, stdio host regressions | **90 / 17 / 19 PASS** respectively, zero filtered; both stdio C adapters exit 0 with expected streams, three native builds/ELF checks and C++ header PASS | `dirent-stdio-regressions/` |

The two filtered libc tests are the same pre-existing native credential tests
excluded from Linux execution, not directory tests. Host execution of native
ELFs is **NOT_RUN**; they run only under SunlightOS. Host/backend tests cover
CWD and path bounds, native failures, real-name conversion/termination,
EOF/errno, malformed records/counts, allocation failure, close and offset
overflow. Native fixtures independently cover real names/EOF, two independent
streams and 256 open/read/close cycles; missing/file/unsupported paths and real
kernel EACCES; an empty directory, 71 actual entries including a 64-byte name,
and opening `"."` followed by a CWD change before enumeration.

The historical ELF SHA-256 is identical in the host validation, both single
runs and both final suites:
`ceea06c856e0735ce241dbfccd9e0f9dd25dca69fab45e38b3f921f24acd4eec`.
Each final historical directory process and each focused directory fixture
exits 0 with empty stdout/stderr. Every final built case passes the ordinary
reaper, zero PID-owned frames/fds/stack, frame/handle/namespace restoration and
fault/unknown-syscall gates. `dirent-host-final/final-matrix-check.log` records
the separate counts, source identity, ELF identity and stream checks.

Retained failed observations are not overwritten by final passes. An initial
exploratory host compile returned **101** because a new test assertion required
an explicit reference (`dangerous_implicit_autorefs`); corrected test expression
and observation are recorded in `dirent-host/initial-host-observation.log`.
The first complete guest attempts, `dirent-2cpu/` and `dirent-4cpu/`, each
retained **39 guest FAIL / 11 build-blocked NOT_RUN** under the runner's global
fault gate. There were 37 completed result records before `dirent-errors`
reaped with exit **31**, then cleanup panicked with `Err(IsDir)`; `dirent-entries`
was never reached. Historical exit 0 in those failed combined runs does not
constitute acceptance. The actual failure was the new fixture attempting to
remove a directory through native unlink, which deliberately rejects
directories (`sunlight-fs/src/tmpfs.rs::unlink`). No rmdir capability was added
or fabricated. Instead the opt-in guest INITRAMFS now contains two fixed test
directories before all baseline snapshots: an initially empty writable
directory and a mode-000 directory. Focused fixtures create and remove only
ordinary files within those existing directories. This preserves every
original probe, its assertions and the runner's fault/cleanup requirements.
The final suites independently validate the corrected fixtures and final
kernel test image; the earlier single runs used the preceding test image.

Exact commands, run from the repository root (each output directory was new):

```sh
python3 tools/native-c/tests/run_dirent.py --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-host-final
python3 tools/native-c/tests/run_stdio.py --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-stdio-regressions
python3 tools/native-c/tests/run_mman.py --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-regressions/mman
python3 tools/native-c/tests/run_time.py --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-regressions/time
python3 tools/native-c/tests/run_random.py --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-regressions/random
python3 tools/native-c/tests/run_unistd.py --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-regressions/unistd
python3 tools/native-c/guest.py docs/audit/probes/native-foundation/results-2026-11-10-dirent-host-final/dirent.elf --expect-exit 0 --cpus 2 --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-original-2cpu
python3 tools/native-c/guest.py docs/audit/probes/native-foundation/results-2026-11-10-dirent-host-final/dirent.elf --expect-exit 0 --cpus 4 --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-original-4cpu
python3 tools/native-c/guest.py --suite --cpus 2 --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-2cpu
python3 tools/native-c/guest.py --suite --cpus 4 --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-4cpu
python3 tools/native-c/guest.py --suite --cpus 2 --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-final-2cpu
python3 tools/native-c/guest.py --suite --cpus 4 --out docs/audit/probes/native-foundation/results-2026-11-10-dirent-final-4cpu
rustfmt --edition 2021 --check kernel/src/native_c_guest.rs sunlight-libc/src/dirent.rs
python3 -m py_compile tools/native-c/guest.py tools/native-c/tests/run_dirent.py
git diff --check
```

The host/ABI runner records exact compiler, linker, runtime build, readelf, nm
and disassembly commands in its individual logs. Every guest retains its
invocation, runtime/support/kernel/ISO build commands, QEMU argv, immutable
kernel snapshot hash, manifest, raw serial, decoded streams and results.
The exploratory command preceding the recorded host runner was
`cargo test -p sunlight-libc --lib --target x86_64-unknown-linux-gnu dirent:: -- --test-threads=1`.

| Phase 9 changed file | Purpose |
|---|---|
| `sunlight-libc/include/dirent.h` | Narrow types, three declarations and explicit subset/lifetime contract |
| `sunlight-libc/src/dirent.rs`, `sunlight-libc/src/lib.rs` | Native C bridge, seven injected-backend host tests, native-C/test-only registration |
| `tools/native-c/tests/dirent-abi.c`, `dirent.c`, `run_dirent.py` | Header/ABI checks, three separate real guest contracts and host/native ELF validation |
| `tools/native-c/guest.py` | Register the three directory fixtures after existing cases |
| `kernel/src/native_c_guest.rs` | Two fixed directory fixtures, only in the opt-in guest gate |
| `tools/native-c/README.md`, `docs/audit/std-progress.md` | Contract, evidence, counts and limitations |
| `docs/audit/probes/native-foundation/results-2026-11-10-dirent*/` | Retained diagnostics, failures, host/build/ABI and native execution evidence |

All 371 initial modified/untracked Phase 8 files were compared with starting
hashes before documentation updates; existing source/evidence bytes were
preserved, with only the directory registration added to guest.py. The Phase 8
audit and README text remain intact before the appended Phase 9 sections.
`dirent-host-final/phase8-preservation-before-docs.log` records the initial
comparison. Changes remain uncommitted.

| Historical scope | Phase 8 | Final Phase 9, on both CPU counts |
|---|---|---|
| 18 API probes (17 C + 1 C++) | 6 PASS / 12 BUILD_FAIL | **7 PASS / 11 BUILD_FAIL / 0 guest FAIL / 11 guest NOT_RUN** |
| 21 executables including foundation | 9 PASS / 12 BUILD_FAIL | **10 PASS / 11 BUILD_FAIL / 0 guest FAIL / 11 guest NOT_RUN** |
| 16 originally blocked APIs | 4 PASS / 12 BUILD_FAIL | **5 PASS / 11 BUILD_FAIL / 0 guest FAIL / 11 guest NOT_RUN** |

The three new focused cases do not inflate historical API counts. Final
complete-suite built executions rise from 35 to **39**: one newly buildable
historical API and three focused cases. Unchanged historical mmap and all
three mapping fixtures pass again on 2 and 4 CPUs. The original mmap/munmap
paths, sys/mman.h, syscall-52 mprotect adapter and internal MAP_NATIVE_THREAD
remain intact. Phase 8 hardware user-write faults and forced stale-TLB /
cross-core interleavings remain **NOT_RUN**; this phase neither depends on
those tests nor claims their verification. No CPython or full POSIX acceptance
is implied.

**Single next build blocker in retained matrix order: unchanged
`docs/audit/probes/dlfcn.c`, line 2, missing `<dlfcn.h>`.** Both final suite
`build-dlfcn.log` files retain that exact first diagnostic. Later declarations,
exports and native loader capability must be traced before selecting an
implementation; a missing-header error alone does not establish native support.

## Phase 10 — native dynamic-symbol-loader blocker (2026-11-10)

**Decision C: stop at the genuine native loader capability blocker.** This
probe requires successful main-program symbol resolution, although it does
not request loading a shared-object file. A failure-only implementation would
fail its assertions. No public header, dl* exports, fake handles, symbol-name
special cases, host loader calls, or production loader changes were added.

The unchanged `docs/audit/probes/dlfcn.c` has SHA-256
`ff5e3d6930cf1a10bf72af7946e3d396eb56dc556291f5328f316113328598e6`.
Its complete contract is:

| Operation | Required observation |
|---|---|
| Feature selection | Defines `_GNU_SOURCE` before including `<dlfcn.h>` |
| Flags | Uses only `RTLD_NOW`; no LAZY, GLOBAL, LOCAL, DEFAULT, NEXT or other flags appear |
| Open | Exactly one `dlopen(0, RTLD_NOW)`: NULL pathname denotes the main program; NULL result immediately returns 1 |
| Lookup | Exactly one `dlsym(h, "malloc")`; non-NULL result required; the function pointer is neither cast nor called |
| Close | Exactly one `dlclose(h)` after lookup, including lookup failure; its return value is ignored |
| Error lifecycle | No `dlerror()` calls, no errno access, no error-message or clearing assertion |
| Final status | Returns the lookup-is-NULL boolean: 0 for success, 1 for lookup failure |
| Streams | No output operations; expected stdout and stderr are both empty |

There are no non-NULL pathname cases, alternate handles, repeated open/close,
invalid-handle tests, or shared-object fixtures in this source. Those behaviors
cannot be inferred from it. No handle lifetime, lookup scope, flag support or
dlerror clearing behavior has been implemented or verified in this phase.

The native runtime was rebuilt successfully with `bash tools/build_native_c.sh`.
An ordinary unchanged-source build reproduces missing `<dlfcn.h>` at line 2.
An **isolated diagnostic include directory**, never installed in the sysroot,
then supplies C declarations and a diagnostic `RTLD_NOW` value of 2. This value
is not a newly supported native ABI. The unchanged source compiles to an object;
its only undefined dl* references are `dlopen`, `dlsym` and `dlclose`. Linking
that same source with the normal native driver fails on **all three symbols**.
`dlerror` is declared in the diagnostic header but unreferenced by the probe.
The rebuilt canonical archive defines `malloc` and none of the four dl* names.
No executable is produced, so there is no historical dlfcn ELF to validate or
run identically on two CPU counts.

Repository evidence explains why this is more than a C bridge:

- `tools/native-c/cc.py` selects the freestanding native target, disables
  PIC/PIE, uses `-nostdlib -static`, the canonical static archive and
  `services/user-space.ld`, with garbage collection and no undefined symbols.
  `tools/native-c/Cargo.toml` builds a `staticlib`. There is no native shared
  library packaging path in these tools.
- The linker script places the executable at 0x400000. Inspection of the
  retained Phase 9 `dirent.elf` shows ET_EXEC, no INTERP or DYNAMIC program
  header, no `.dynsym`, and no relocations. Its ordinary `.symtab` has address
  zero and is not an allocated runtime symbol table. Archive symbol presence
  alone supplies no way to find a symbol in a running process; section garbage
  collection also gives no guarantee that an unreferenced `malloc` is retained.
- `sunlight-elf/src/lib.rs::parse_elf_header` accepts ET_EXEC (2) and rejects
  ET_DYN with `NotStaticExecutable`. The segment walker rejects PT_INTERP.
  `has_dynamic_section` exists but has no callers: this audit does **not** claim
  the production loader rejects every PT_DYNAMIC-bearing ET_EXEC.
- `kernel/src/process/elf_loader.rs::load_elf` validates fixed-address PT_LOAD
  plans, preflights PTE collisions, reserves regions, copies bytes, zeroes BSS
  and commits final protections. It returns the entry address for process
  startup. It neither publishes a runtime symbol namespace nor parses dynamic
  symbols or applies ELF relocations. Spawn/startup callers do not provide a
  running-process dlopen interface. No Rust/C native resolver was found.
- Phase 8's public C mmap bridge supports private anonymous mappings and
  rejects file descriptors/backing and W+X. Native executable VM mapping exists;
  it is not a runtime shared-object loader. A future loader would need address
  placement, symbol scope/retention, relocations and protection/lifetime handling;
  this probe's immediate blocker is **main-program symbol resolution**, not a
  missing on-disk `.so`. Legacy DOS relocation support and Linux/Helios host
  observations do not establish this native capability.

Reproduce from the repository root (the output directory must be new):

```sh
python3 tools/native-c/tests/audit_dlfcn.py --out docs/audit/probes/native-foundation/results-2026-11-10-dlfcn-audit
python3 -m py_compile tools/native-c/tests/audit_dlfcn.py
git diff --check
```

The runner retains exact commands and exit codes for runtime build, original
header failure, diagnostic-header object compilation/link failure, nm, readelf
and ELF host regressions. Evidence is under the output directory above.
`source.log` retains the original source and hash; `preservation.log` verifies
**21,119 pre-existing tracked files unchanged during the audit**, before this
documentation append. The initial worktree reported no modified/untracked
files; all existing Phase 8/9 implementation and retained evidence bytes were
preserved. No reset, cleanup or historical-source edit was performed.
Python compilation and diff whitespace checks pass. `matrix-check.log` records
an independent CSV count and retained line-2 diagnostic check for both CPU
counts, confirming the baseline below.

| Test domain | Phase 10 result |
|---|---|
| Canonical native runtime rebuild | PASS |
| Unchanged source with original includes | Expected BUILD_FAIL: missing header |
| Diagnostic-header native C compilation | PASS, object only |
| Diagnostic-header native link | Expected BUILD_FAIL: three missing dl* exports |
| Archive and retained native ELF inspection | PASS, static-only evidence recorded |
| Existing sunlight-elf host regression tests | 11 PASS; doc tests 0; no failures |
| Implemented dl* host/C ABI behavior tests | NOT_RUN: no implementation selected |
| Historical dlfcn guest, 2 and 4 CPUs | NOT_RUN: no linked ELF; status/streams/reap/fault/syscall checks unavailable |
| Full native-C suites and other guest regressions | NOT_RUN in Phase 10: stopped under decision C; retained Phase 9 results below |

No guest acceptance counts change. The retained current matrices are
`results-2026-11-10-dirent-final-{2,4}cpu/results.csv`: original APIs remain
**7 PASS / 11 BUILD_FAIL**; historical guest execution remains **10 PASS /
0 FAIL / 11 NOT_RUN** per CPU count; complete built suites remain **39 PASS**
per CPU count. These are Phase 9 results, not fresh Phase 10 executions.
Phase 8 hardware user-write faults and forced stale-TLB/cross-core tests remain
**NOT_RUN**.

**One verified next blocker:** the unchanged dlfcn source, after diagnostic
header resolution, cannot link `dlopen`, `dlsym`, `dlclose`; truthful success
requires a native main-program symbol resolver and real handle semantics that
the current static startup loader does not supply. No later API is promoted
past this blocker and no working dynamic loading is claimed.

### Phase 10 continuation — main-program export lookup accepted (2026-11-10)

This continuation supersedes the capability blocker above only after the exact
unchanged historical source succeeded in **both isolated guests**. The earlier
audit and its evidence remain historical observations. The source SHA-256 is
still `ff5e3d6930cf1a10bf72af7946e3d396eb56dc556291f5328f316113328598e6`.

The final native executable is ET_EXEC at fixed virtual addresses (load bias
zero). `.symtab`, `.strtab` and section headers survive in the packaged ELF file,
but the symbol/string tables are non-allocated and outside PT_LOAD. There is no
`.dynsym`, PT_DYNAMIC, interpreter or relocation table in this executable.
`guest.py` embeds the entire ELF with `include_bytes!`; the kernel validates and
copies PT_LOAD bytes, not section headers or ordinary symbol tables, into the
process. No file symbol table or loader namespace is exposed to C startup.
The final `malloc` and `free` are GLOBAL, DEFAULT-visibility FUNC symbols in RX
text. Their names and addresses therefore need a separate loaded index.

`tools/native-c/user-space.ld`, selected only by `cc.py`, now emits that index
inside `.rodata` in an R-only PT_LOAD. Each 32-byte record contains a 24-byte
NUL-padded name and a linker-resolved absolute function address. The deliberate
public export set is `malloc`, `free`, `calloc`, `realloc`; linker references
retain their actual implementations through section garbage collection. Adding
an export means adding a record to the linker script, not a branch in dlsym.
The general resolver compares bounded names, validates aligned table bounds,
nonzero size, record-size divisibility, a 4096-byte maximum, and requires the
resolved address to lie between linker-defined text bounds. The current table
has four records (128 bytes). This is an ET_EXEC-specific ABI; ET_DYN/PIE and
nonzero load bias remain unsupported by the startup loader and this interface.
Metadata has process lifetime and is never freed or made writable by dlclose.
The shared Rust service linker script and production kernel loader are unchanged.

The public header adds only RTLD_NOW and dlopen/dlsym/dlclose. NULL-path open
with exactly RTLD_NOW acquires a real process-local main-namespace reference;
all non-NULL paths and other flags fail with NULL. Handles identify that static
namespace, with atomic bounded reference counting. Lookup requires a valid
namespace handle with a live reference; missing/empty/NULL names return NULL.
Invalid handles return NULL from lookup and -1 from close; a final close makes
lookup fail. Repeated opens share the namespace handle, as tested. This is not
per-acquisition stale-handle detection after reopening. C name pointers must be
readable strings. No shared objects, arbitrary program-symbol discovery,
RTLD_DEFAULT/NEXT, TLS/data symbols, dlerror, or error-message lifecycle are
advertised. The probe has no dlerror contract, so no such export was added.

Exact retained acceptance evidence:

- `results-2026-11-10-dlfcn-accepted-{2,4}cpu/` contains full build commands,
  ELF headers, per-case `results.csv`/`results.json`, serial output, fault logs,
  and guest cleanup observations. Both suites completed **41/41 built cases**.
- The unchanged `dlfcn.elf` is byte-identical on both CPU counts, SHA-256
  `78e68ea43dc730d4e4d66533de77d1ea25e62153e42925a5eb7db7a8002a6268`.
  Both executions returned 0, stdout/stderr length 0, `clean=1`, no retained
  descriptors/owned frames/stack, and unchanged surrounding frame/table/handle
  counts. Both `faults.log` files are empty (including unexpected syscall checks).
- `dlfcn-contract.elf` resolves malloc and free, compares each pointer with the
  directly linked C function, allocates, writes and frees memory through those
  pointers, and checks calloc lookup, missing symbols, NULL names, invalid
  handles, unsupported paths/flags, repeated opens and final/double close.
  Both guests returned 0 with empty streams and clean reaping. Its ELF SHA-256
  is `883bfb3c236f67025aae0df7e7e97892123b6badbbbcf05509994341bfb08d45`.
- `results-2026-11-10-dlfcn-index-accepted/`: public C/C++ ABI compilation and
  independent ELF inspection verify each index address equals the actual
  linked symbol, each target lies in RX PT_LOAD, and all index bytes lie in
  R-only PT_LOAD. This inspection does not implement a host runtime resolver.
  An earlier index check caught a C++ void-pointer conversion in the fixture;
  the corrected explicit cast produces the identical guest-tested ELF.
- `results-2026-11-10-dlfcn-regressions/`: libc host tests **114 PASS**, two
  existing native-syscall-only tests filtered; twelve existing C/C++ directory
  ABI checks and canonical-export checks PASS. Existing ELF host tests:
  **11 PASS**, zero doc tests. These host results are separate from acceptance.

Counts after both guest successes: original 18 APIs **8 PASS / 10 BUILD_FAIL**
(previously 7/11); historical 21 including foundation probes **11 PASS / 0 FAIL /
10 NOT_RUN** per CPU count; complete native-C suites **41 PASS** per CPU count
(previously 39, plus historical dlfcn and its focused contract). No other API is
promoted. Phase 8 hardware user-write-fault and forced stale-TLB/cross-core
hardware tests remain **NOT_RUN**. Existing Phase 8/9 code and evidence and the
initial diagnostic audit are preserved; no commit or cleanup/reset was made.

Reproduce with `python3 tools/native-c/tests/run_dlfcn.py --out <new-dir>` and
`python3 tools/native-c/guest.py --suite --cpus 2 --out <new-dir>` (repeat for 4).

Additional fresh regressions: native Rust-std overlay check and kernel check
PASS (`dlfcn-regressions/native-std-check.log`, `kernel-check.log`); production
Rust-std futex contract tests **3 PASS** (`futex.log`). The stdio runner in
`results-2026-11-10-dlfcn-stdio-regressions/` reports **19 stdio, 90 VFS, 17
kernel descriptor host tests PASS**, C adapter output checks, historical tmpfile
adapter, native ELF checks and C++ headers PASS. A final rebuild after rustfmt
changed compiler-generated ELF bytes; both full suites were repeated against
that final runtime in the `dlfcn-accepted-*` directories cited above. Earlier
`dlfcn-final-*` and index attempts are retained separately.

The additional broad `bash tools/test_native_std_foundation.sh` regression
**FAILED** (exit 1): its build and ISO stages succeeded, but the 2-CPU guest
timed out after 240 seconds without an FP-probe verdict. Serial output reaches
login, reports `/bin/sshl` spawn failure and built-in shell readiness, then key
injection completion; it contains no `[NATIVE-STD]` verdict. This observation
does not establish the underlying cause. The runner stopped before its 4-CPU
case (**NOT_RUN**). Exact build/traceback and serial evidence are retained in
`dlfcn-regressions/fp-foundation.log`, `fp-foundation-2cpu-serial.log` and
`fp-foundation-command.log`. No FP foundation acceptance is claimed. The
separate overlay/kernel checks and the final native-C guests passed as above.
The broad build refreshed its stale probe Cargo.lock; only that incidental
change was restored to its initial bytes.

### Phase 10 follow-up — FP timeout triage (2026-11-10)

The broad FP foundation gate remains **FAILED: setup/launch failure**, rather
than an accepted FP run. Its original 2-CPU timeout is preserved; its 4-CPU case
remains **NOT_RUN**. Separate successful executions below do not change those
classifications. No timeout was increased and no session authorization was
weakened.

Read inputs were the preceding Phase 10 record and
`results-2026-11-10-dlfcn-regressions/{fp-foundation-command.log,fp-foundation.log,fp-foundation-2cpu-serial.log}`.
The original command was `bash tools/test_native_std_foundation.sh`, exit 1.
The script builds the native FP ELF, sunlightd, sunshell, a kernel with
`SUNLIGHT_INJECT_PHASE=native-std` and `key_inject,native_std_probe`, creates
`target/native-std-foundation.iso`, then runs `tools/native_std_boot_gate.py`.
That runner gives each guest **240 wall-clock seconds** and stops at the first
failure. It uses a 2048-MiB diskless QEMU guest with virtio RNG; on this host
`/dev/kvm` is absent, so this path uses QEMU's default TCG/CPU configuration.

New evidence is in `results-2026-11-10-fp-triage/`. The retained ISO's SHA-256
and FP ELF hash are in `fp-inputs.log`. Replays use the same ISO and original
QEMU options, adding a local QMP socket solely for observability. Each run has
its exact `command.log`, serial, QEMU output, timed events, final QMP status and
registers, and a result receipt. A sandbox socket-denied attempt is retained
under `replay-2cpu/`; it is not guest evidence. The authorized 100-second
`replay-2cpu-observed/` reproduces the missing launch. The separate
`replay-2cpu-original-budget/` repeats the **original 240-second limit**:

| Wall-clock observation | Original-budget replay |
|---|---|
| Injection enabled | 18.767 s |
| Root login succeeds | 37.475 s |
| `/bin/sshl0` request fails before ELF loading | 37.476 s |
| Injection completes | 70.917 s |
| Guest reaches monotonic 60-second checkpoint | 78.496 s |
| Deadline reached; QMP still reports running | 240.121 s, TIMEOUT |

The diagnostic controller's completion and QEMU's exit 0 reflect deliberate
termination after recording the timeout; they are not a passing gate verdict.
The receipt's `verdict=TIMEOUT` and the original runner's exit 1 govern status.

Neither replay contains an FP-probe start marker or a request to spawn
`/bin/native-fp-probe`. Approximately 169 seconds of the full-budget replay
follow injection completion without any workload launch. This is a setup
failure followed by waiting for an unreachable verdict, not evidence that FP
work consumed 240 seconds. The injector sends its command through the TTY shell;
`services/tty_server/src/main.rs::spawn_tab` uses legacy
`SPAWN_AUTHENTICATED`. In
`kernel/src/arch/x86_64/syscall.rs::handle_spawn_call`, that request reaches the
session-attachment authorization block before process creation. A legacy
authenticated launch without an attached account session is refused there.
The observed request/error pair has no subsequent shell ELF load. The TTY's
"Built-in shell ready" message does not prove an executable foreground shell
exists: `spawn_tab` failed and created no tab. No FP deadlock is observed.

The pre-Phase-10 control is committed HEAD `97255204a`. Exact byte comparisons
in `baseline-source-comparison.log` establish that its kernel entry/injection,
spawn handler/loader, TTY server, userspace linker, FP Rust/assembly and broad
gate scripts match the source used for this diagnosis. Phase 10's dlfcn module
is gated on `native-c`, which the FP probe does not enable; its C-only linker
script and C-suite registration are not used by the broad FP gate. Thus the
demonstrated launch-path defect is independent of the Phase 10 C bridge.

A selected-source `git archive HEAD` snapshot under
`target/std-prephase10-source/` rebuilt the old FP probe offline in 43.887 s.
`baseline-fp-build.log` retains the command and `baseline-fp-comparison.log`
retains both ELF hashes. The independently built ELFs differ, so byte identity
is not claimed. An initial full-snapshot extraction failed for lack of `/tmp`
space (`baseline-snapshot.log`); only that newly created temporary snapshot
was removed, then the selected-source extraction succeeded under `target/`.
No pre-existing temporary directories or workspace evidence were removed.

The existing isolated native guest runner can launch the exact FP ELF without
the desktop login path. `direct-{2,4}cpu/` records the retained FP ELF
`582d8cce13bbb46274bd0e1f1609ca322428e80e17c3f0d51ad03ffa3638e363`
passing **both** x87/XMM/MXCSR isolation and native PDF/vector checks, status 0,
empty stdout/stderr, clean reaping, eight borrower threads reaped and unchanged
surrounding resource inventories. Both fault logs are empty. The separately
rebuilt HEAD probe also passes on 2 CPUs in `prephase10-direct-2cpu/`, hash
`5bc71b2f0eabaabd45a28d3f7da3d56136d7f2f346752721898bbde7bedd08e7`.
These are bounded 90-second TCG/max guest runs, not broad desktop-gate passes.
A clean-HEAD desktop ISO was not rebuilt/run; the control combines exact
launcher-source comparison with an independently built baseline workload.
No Phase-10-induced FP regression was demonstrated or patched.

Reproduce the observed full-budget replay from the repository root with a new
output directory; `replay-driver.log` contains the complete diagnostic script:

```sh
python3 - <new-output-dir> 2 240 < docs/audit/probes/native-foundation/results-2026-11-10-fp-triage/replay-driver.log
python3 tools/native-c/guest.py tools/native-std-probe/target/x86_64-sunlightos/release/native-fp-probe --expect-exit 0 --cpus 2 --out <new-dir>
# Repeat the isolated workload command with --cpus 4 and another new directory.
```

### Phase 11 — initial C locale CODESET query accepted (2026-11-10)

The established historical order in the accepted Phase 10 matrices selects
`langinfo.c` immediately after the already accepted `getrandom.c`.
The unchanged probe is:

```c
#define _GNU_SOURCE
#include <langinfo.h>
int main(void){return nl_langinfo(CODESET)==0;}
```

Its SHA-256 remains
`4e3efb53a172666505159e95e1b7f557c2a093a62e706fafc5eae3ef44babb2d`.
`results-2026-11-10-langinfo-audit/` retains its source, the previous diagnostic,
a fresh missing-header failure, diagnostic-only header/object compilation,
the object's actual unresolved symbol and the resulting native link failure:
**`nl_langinfo` is missing**. The runtime export census and source inspection
were completed before implementing the bridge; a missing header alone was not
treated as a capability requirement. Diagnostic headers remain uninstalled.

The native C runtime has byte-oriented strings and ASCII execution characters,
but no locale selection, locale database or multibyte conversion API. This
entry needs only an initial-locale codeset query; no kernel service or dynamic
locale machinery is needed. `sunlight-libc/src/langinfo.rs` and the public
`langinfo.h` now expose `nl_item` as C `int`, native `CODESET=1`, and
`char *nl_langinfo(nl_item)`. The fixed initial C locale returns **`US-ASCII`**.
Other item values return a non-NULL empty string. Storage has process lifetime,
is read-only to callers, requires no allocation or lock, and leaves errno
unchanged. This is a deliberately narrow native ABI: LANG/LC_* do not select a
locale, no other langinfo items or locale objects are advertised, and the
runtime does not claim Unicode conversion support from Rust/UI Unicode use.
There is no process-wide locale mutation API in this implementation.

`tools/native-c/tests/langinfo.c` independently checks the actual codeset
contents, repeated-pointer/string lifetime, invalid items including signed
integer boundaries, and errno preservation. Its C/C++ ABI fixture checks the
item and function types under base/POSIX/GNU macros and both include orders.
The suite adds this focused contract alongside the unchanged historical source.
The runtime module is enabled for native C and host unit tests only; host tests
do not interpose a host libc export.

Acceptance evidence in `results-2026-11-10-langinfo-accepted-{2,4}cpu/` contains
the full commands, ELF inspection, serial, per-case CSV/JSON, reaping/resource
checks and empty fault logs. **Both full suites pass 43/43 built cases.**
The unchanged `langinfo.elf` is byte-identical across CPU counts, SHA-256
`29604d792b97d031837ff249e085c3fe4b7f8cf70ba20bfebce49a203e3b35c8`.
Both executions return 0, produce empty streams, report `clean=1`, and retain
no descriptors, owned frames or stack. The focused contract also passes on
both CPU counts, identical ELF SHA-256
`8b6c87c8e403722372ab490f5fdf438d64e009edc99503764adac4e9d2575138`.
Surrounding physical-frame/table/handle inventories are unchanged.

Fresh relevant regressions are separate from these guest results:

- `results-2026-11-10-langinfo-host-accepted/`: twelve C/C++ ABI compilations, exactly
  one canonical `nl_langinfo` export, unsupported locale exports absent,
  unchanged historical link, no unresolved symbols/interpreter/dynamic segment.
- `results-2026-11-10-langinfo-regressions-accepted/`: libc **116 host tests PASS**, two
  existing native-syscall-only tests filtered; twelve existing directory C/C++
  ABI checks and canonical-export checks PASS.
- `results-2026-11-10-langinfo-checks-accepted/`: existing dlfcn C/C++ ABI and loaded
  export-index inspection PASS; ELF **11 host tests PASS**; stdio **19**, VFS
  **90** and descriptor **17** host tests PASS, including C adapter output,
  historical tmpfile adapter, three native ELF checks and C++ headers.
  Native Rust-std overlay and kernel checks PASS; production overlay futex
  contract tests **3 PASS**. The earlier kernel check completed in 141.215
  seconds, including its embedded application build prerequisites; exact final
  check duration and command are recorded in the accepted `kernel-check.log`.
- The first overlay-check invocation failed with exit 101 because `--locked`
  refused the stale probe lock refresh (earlier
  `results-2026-11-10-langinfo-checks/native-std-check.log`). That command also
  named nonexistent bin `native-std-probe`, but the retained diagnostic stops
  at lock validation. The corrected `--bin native-fp-probe --release`
  invocation allows the incidental refresh and passes in earlier
  `native-std-corrected.log` and final accepted `native-std.log`. Both attempts
  remain retained. The probe Cargo.lock was restored to its verified initial
  bytes after checking.

The final formatting check moved the new module registration into rustfmt's
required order. The full native-C suites and relevant host/ABI/regression
checks were repeated against that final source in the `*-accepted*`
directories above. Earlier `langinfo-final-*`, host and regression directories
remain separate observations. Final formatting, Python compilation and diff
whitespace checks pass.

Counts change only after both guest successes: original 18 APIs **9 PASS /
9 BUILD_FAIL** (previously 8/10); historical 21 including foundation probes
**12 PASS / 0 FAIL / 9 NOT_RUN** per CPU count; full native-C suites **43 PASS**
per CPU count. The broad FP gate remains **FAILED during setup on 2 CPUs**,
with its broad 4-CPU case **NOT_RUN**. The separate isolated FP tests above are
PASS. Phase 8 hardware user-write-fault and forced stale-TLB/cross-core tests
remain **NOT_RUN**.

**Next verified blocker:** unchanged `locale.c`, SHA-256
`c2634d0aa8bcc412d9af689358b058bba1e9075c215cb4f051ebf465ae00f140`, calls
`setlocale(LC_ALL,"C")`. `results-2026-11-10-langinfo-next-blocker/` retains the
previous and fresh missing-`locale.h` diagnostics; a diagnostic-only declaration
then compiles the unchanged object but fails to link **`setlocale`**. This is
the next required C interface, not evidence for arbitrary locale loading. No
later historical API is promoted and no locale header/selection bridge is
installed in this phase.

Reproduce with new output directories:

```sh
python3 tools/native-c/tests/run_langinfo.py --out <new-dir>
python3 tools/native-c/tests/run_dirent.py --out <new-dir>
python3 tools/native-c/guest.py --suite --cpus 2 --out <new-dir>
python3 tools/native-c/guest.py --suite --cpus 4 --out <new-dir>
```

All incoming uncommitted Phase 10 work and retained evidence are preserved.
An initial tracked/untracked file-hash inventory is retained under FP triage;
the only edits to incoming source files add the new module and suite case, and
the original audit text remains an exact prefix. No commit, reset or workspace
cleanup was performed.
