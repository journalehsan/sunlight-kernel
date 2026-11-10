# CPython platform probes

Run from the repository root:

```sh
bash tools/build_native_c.sh
python3 docs/audit/probes/generate.py
python3 docs/audit/probes/run.py
python3 docs/audit/make_matrix.py
```

The runner writes only beneath `docs/audit/probes/`. A compiler may use its
normal temporary directory for intermediate files. It does not invoke Cargo,
rebuild an ISO, modify a disk or contact an existing VM. Generated binaries and
objects are ignored; source, exact command logs, exit codes and CSV are retained.
The runner reports build failures rather than hiding missing compilers/headers.

The native C Phase 1 follow-up uses the canonical runtime via
`tools/build_native_c.sh` and `tools/native-c/cc.py`. Run it separately so the
original CSV and failed logs remain intact:

```sh
python3 docs/audit/probes/native-foundation/run.py \
  --out docs/audit/probes/native-foundation/results-local-001
# Recheck the original API suite through its updated native C driver:
python3 docs/audit/probes/run.py --targets native \
  --out docs/audit/probes/native-foundation/results-api-local-001
```

The output directory must be new. This builds a small native static archive
using the existing Rust target/overlay, reproduces the original 18 default
native failures, and records preprocessing, C-to-assembly compilation,
assembly, executable linking and ELF validation separately. It also builds
the header-free compiler probe and the entry/write/exit probes. Its exit code
checks the foundation only; inspect `after.csv` for the still-failing original
API probes. The original suite has **17 C probes and one C++ probe**.

`sunlight_runtime.h` declares existing `sl_std_write` / `sl_std_exit` exports;
these are the native runtime ABI, not POSIX write/exit. The canonical startup
initializes its runtime and calls main. `exit.c` expects guest status **23**;
`write.c` expects `[NATIVE-C] write OK` on guest stderr. This runner does not
execute ELFs itself. Phase 4 adds `tools/native-c/guest.py --suite` for real
guest acceptance; see `../../../tools/native-c/README.md` and
`../std-progress.md` for commands, results and the retained Phase 1 evidence.

Targets:

- `host`: host cc/g++, a control for probe correctness.
- `helios`: static non-PIE musl C; optional `HELIOS_CC` / `HELIOS_CXX` override
  the drivers (defaults: musl-gcc / x86_64-linux-musl-g++).
- `native`: C defaults to `tools/native-c/cc.py`, using the prepared canonical
  runtime archive, native linker script and repository/builtin headers. Build
  the archive first as above. Native C++ still defaults to bare ELF clang++
  without a C++ runtime. `NATIVE_CC` / `NATIVE_CXX` can override drivers;
  the runner still passes `-nostdlib -nostdinc`. No host libc is selected.

Linux-host execution of a musl binary is logged separately from SunlightOS.
The runner executes the Helios image on Linux **before** stamping its audit-local
ELF with OSABI 3 and HLNX01. It never executes a native ELF on the Linux host.
A static musl dlopen failure is expected and is still recorded as FAIL.
`termios.c` accepts ENOTTY: it does not prove an interactive terminal works.

Optional guest execution:

```sh
HELIOS_RUNNER='/path/to/isolated-guest-runner --personality helios' \
NATIVE_RUNNER='python3 tools/native-c/guest.py' \
python3 docs/audit/probes/run.py
```

These environment variables are argv prefixes parsed with shlex, never shell
evaluated. The runner appends the absolute ELF path. The guest runner must load
that ELF into an isolated SunlightOS guest with personality-correct startup,
return its exit status, print the guest transcript, and use immutable/snapshot
media. The native implementation is now `tools/native-c/guest.py`; each
invocation retains an isolated ISO, serial transcript and exit/reclamation
records in a unique directory under `target/native-c-guest-runs/`. A Helios
implementation is still not supplied. The original CSV and build-only runners
preserve UNKNOWN for unexecuted cells; the Phase 4 suite records acceptance
FAIL with guest NOT_RUN for builds that cannot produce an executable.
Inspect `results/results.csv` and individual logs; do not reinterpret host PASS
as guest PASS. Typical socket creation may be blocked by a host sandbox; final
recorded audit results came from an approved unrestricted-host rerun.

Probe scope is deliberately small. The programs exercise the C API families
requested by the audit; they do not exhaust CPython configure, POSIX or libc++.
The C++ test covers constructors/string/iostream, RTTI, throw/catch, thread-local
storage, a thread, and atexit, but not multi-translation-unit initialization
ordering or exception unwinding across a C ABI boundary. `compiler.c` is
compile-only and not an executable runtime test.
