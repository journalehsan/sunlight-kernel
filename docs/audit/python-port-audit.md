# CPython port audit — SunlightOS

## Summary (one page)

**Recommendation: Helios first, then Native; neither path has demonstrated a CPython interpreter.** This is a source audit plus compiler/host probes, not a successful guest port. Baseline: **CPython 3.12**, explicitly permitted by the request; newer releases are not certified by this audit. The existing Linux-musl build path avoids creating a native C runtime first (`tools/build.sh:124–130`, `tools/build_docserv.sh:17–30`). Native Rust std is useful infrastructure, but its filesystem implementation has numerous unsupported operations (`tools/native-std/overlay/fs/sunlightos.rs:237–361`).

| Capability | Helios | Native | Evidence |
|---|---|---|---|
| C compiler / usable full C sysroot | READY for static musl compile/link on this host | PARTIAL: C11 compiler works; C runtime probes fail | `probes/results/native-compiler-only.log`, `probes/results/results.csv` |
| Threads / TLS | PARTIAL: Linux thread clone support | PARTIAL: Rust spawn/join/TLS; pthread API unestablished | `compat-linux/src/clone.rs:19–54`; `tools/native-std/overlay/thread/sunlightos.rs:12–35` |
| User signal delivery | MISSING | MISSING | `kernel/src/arch/x86_64/syscall.rs:369–376,10375–10386` |
| CPython hello / imports | UNKNOWN: not built | UNKNOWN: not built | Probe inventory command: `probes/results/inventory-12.log`; no CPython test run |
| TCP sockets / POSIX subprocess | MISSING | MISSING as a demonstrated C/POSIX interface | `kernel/src/arch/x86_64/syscall.rs:7579–7583,3161–3173,6012–6017`; `probes/results/native-socket-build.log` |
| Dynamic C extensions | MISSING in audited execution path | MISSING in audited execution path | `sunlight-elf/src/lib.rs:212–214`; `kernel/src/process/mmap.rs:230–233`; `probes/results/inventory-13.log` |
| C++ runtime | UNKNOWN guest; configured musl C++ compiler absent | MISSING from tested native toolchain configuration | `probes/results/helios-cpp-runtime-build.log`, `probes/results/native-cpp-runtime-build.log` |

**Top five blockers**, ordered for a useful interpreter rather than every optional module:

1. Establish guest pthread/GIL/TLS correctness; no new guest C probe ran (`probes/results/results.csv`).
2. Implement user signal frames, handler entry and return; today handler delivery only logs a TODO (`kernel/src/arch/x86_64/syscall.rs:369–376,10433–10435`).
3. Supply Native C headers, startup, libc/libm/pthread ABI and configure support; native probe failures and target naming show this work is separate from Rust std (`probes/results/native-stdio-build.log`, `tools/targets/x86_64-sunlightos.json:7–19`).
4. Provide process creation/wait semantics for subprocess and Internet sockets for pip/asyncio (`kernel/src/arch/x86_64/syscall.rs:3161–3173,6012–6017,7567–7584`). These are later milestones, not prerequisites for `print(1)`.
5. Close file/metadata/terminal gaps; postpone shared libraries until file mapping and a loader exist (`syscall-matrix.csv`; `kernel/src/process/mmap.rs:230–233`; `sunlight-elf/src/lib.rs:212–214`).

## Scope, evidence and reproduction

Audit date: 2026-11-10. All new files are below `docs/audit/`. Existing source and existing build outputs were read, never rebuilt or modified. The checkout already had substantial uncommitted changes: the inspected commit and initial worktree state are recorded in `probes/results/inventory-00.log` and `inventory-01.log`. Findings describe that worktree, not just the commit.

Evidence notation: repository paths are relative to the repository root; `path:line–line` identifies source lines. Probe/log links are relative to this report. **READY** means the stated narrow capability was demonstrated; **PARTIAL** means source support or only some semantics; **MISSING** means an explicit rejection/stub or missing component in the tested configuration; **UNKNOWN** means no sufficient evidence. A syscall dispatch arm is never treated as a conformance test. Negative searches are scoped, not proof that no implementation exists anywhere.

Commands executed:

- `python3 docs/audit/probes/generate.py`: generates independent C API probes.
- `python3 docs/audit/probes/run.py > docs/audit/probes/results-summary.json`: records exact compiler/runner commands, exit codes, inventory and results. A sandboxed first run could not create host sockets; rerunning outside the sandbox made both socket controls pass. Final logs are from the unrestricted host run, not a SunlightOS guest.
- `python3 docs/audit/make_matrix.py`: generates the 70-row CSV from the cited source audit.
- Source discovery/read commands included `rg --files`, `rg -n 'personality|PT_INTERP|PT_TLS' kernel/src/process sunlight-elf/src`, `nl -ba tools/build_native_std.sh`, and the line-cited files below. Reproducible inventory searches are recorded in `probes/results/inventory-*.log`.

**Runtime limitation:** guest results for both targets are UNKNOWN. The existing probe builder writes `target/` (`tools/build_helios_probes.sh:4–13`), the native wrapper stages `target/native-std` (`tools/native-std/prepare.py:12–26`), and the native guest gate runs a fixed ISO and writes `target/` logs (`tools/native_std_boot_gate.py:6–12`). Those scripts were not run because of the requested write boundary. No external VM was contacted. To resolve: provide an isolated guest loader for audit ELFs or authorize a later image rebuild; run with `HELIOS_RUNNER` / `NATIVE_RUNNER` as described in `probes/README.md`. Existing expected-output files establish test intent only (`tools/tests/helios_thread_probe.expected:1–7`).

## 0. Repository, build and execution paths

The workspace is considerably larger than the four members described in AGENTS.md: kernel/IPC/drivers/compat coexist with filesystem, networking, libc, services, applications and std-proof crates (`Cargo.toml:3–56`). Architecture/syscalls and VM remain in the kernel; VFS and network services are workspace members, alongside shared filesystem/network crates (`Cargo.toml:10–48`). Wise Owl build invocations appear in `tools/build.sh:118–122`.

| Layer / port | Observed contract | Evidence |
|---|---|---|
| Kernel and legacy userspace | Default `x86_64-unknown-none`; kernel linker script; legacy applications get a userspace script | `.cargo/config.toml:1–19`; `tools/test.sh:14–22` |
| Helios | Linux x86_64 syscall personality; static non-PIE musl Rust target | `compat-linux/src/lib.rs:1–8,375–471`; `tools/build.sh:124–130` |
| Native Rust std | Custom target `x86_64-sunlightos`, `target_os="sunlightos"`; Rust lld, static relocation, aborting panic | `tools/build_native_std.sh:6–11`; `tools/targets/x86_64-sunlightos.json:7–19` |
| Historical native JSON | Separate `os="sunlight"` file explicitly marked future/Phase 1; do not confuse it with active std target | `targets/x86_64-unknown-sunlight.json:2–16,18–20` |
| Helios Note / “rartui” | Exact `rartui` identity UNKNOWN; the observed package is **ratatui 0.26** with crossterm, compiled musl | `helios-note/Cargo.toml:593`; `tools/build.sh:127–129`; `probes/results/inventory-16.log`. Resolve name with requester/port manifest. |
| Hayro | Native renderer feature and pinned Hayro dependency; legacy MuPDF has a separate musl C/C++ build path | `sunlight-docserv/Cargo.toml:109–135`; `tools/build_docserv.sh:4–30` |
| Tokio | Rabbit enables rt/sync/time only. Existing design uses capability IPC networking, not a native std socket reactor | `rappid-rabbit/Cargo.toml:431`; `docs/rappid-rabbit-native-runtime.md:53–59` (design documentation; not rerun evidence) |

Process creation: `spawn_with_session_attachment` reserves task storage, resolves an embedded ELF, allocates a PID and constructs `Process` (`kernel/src/process/spawn.rs:778–811`). `exec_into_process` selects personality and creates a replacement address space (`kernel/src/process/spawn.rs:141–161`). ELF loading validates the header, plans at most eight load segments, checks collisions, maps segments and commits regions (`kernel/src/process/elf_loader.rs:19–20,48–120`). `build_exec_image` maps a user stack, emits Linux auxv for musl/PT_TLS, builds argv/envp and initializes the entry context (`kernel/src/process/spawn.rs:325–374`). General execution of an arbitrary newly installed audit binary is UNKNOWN; inspect `embedded_bytes_for_path` and the exec path with a guest filesystem fixture before assuming it works (`kernel/src/process/spawn.rs:921`; `kernel/src/arch/x86_64/syscall.rs:3514`).

Personality is ELF metadata, not a shell option: Linux OSABI 3 or `HLNX01` selects Linux; OSABI 0 otherwise selects Native; other values are rejected (`sunlight-elf/src/lib.rs:110–126`). Syscall entry checks the current process and translates Linux numbers; a restricted tagged native bridge also exists (`kernel/src/arch/x86_64/syscall.rs:574–599`). Therefore **restamping the same ordinary musl binary as Native does not make it a native binary**: the dispatch conventions differ (compare native read=42 with Linux translation in `kernel/src/arch/x86_64/syscall.rs:1031` and `compat-linux/src/lib.rs:378`).

## 1. C, libc and C++ readiness

### Toolchain evidence

Host inventory: GCC, clang, lld, musl specs and Rust versions are logged in `probes/results/inventory-03.log` through `inventory-11.log`. Rust is revision `8954863c8`, matching the overlay's hard requirement (`tools/native-std/prepare.py:8–11`; `probes/results/inventory-03.log`). This audit did not run Cargo builds, which would write outside the permitted directory.

Helios: `musl-gcc` is a GCC specs wrapper, not evidence that its reported GCC triple is musl. The specs explicitly select `/usr/lib/musl/include`, `/usr/lib/musl/lib`, startup objects and the musl interpreter (`probes/results/musl-layout.log`). `-print-sysroot` is empty; `-print-file-name=libc.a` alone reports a host path and is insufficient to identify the linked libc (`probes/results/inventory-07.log`, `inventory-09.log`). All 17 static C API probes link via the musl specs (`probes/results/results.csv`). Packaging stamps Linux OSABI/HLNX01 in audit-local ELF files (`probes/run.py`).

Native: clang can compile a freestanding x86_64 C11 object, including 64-bit double/pointer assertions; that is **not** a running C toolchain (`probes/results/native-compiler-only.log`, `probes/compiler.c:1–4`). The selected native Rust target uses LLVM `x86_64-unknown-none-elf`, rust-lld and a custom userspace startup/link contract (`tools/targets/x86_64-sunlightos.json:7–19`; `sunlight-libc/src/crt0.rs:1`). A complete installed native C sysroot is UNKNOWN; none was supplied by the audited build scripts, and the tested native configuration cannot link the runtime probes (`probes/results/native-stdio-build.log`, `native-string-build.log`). To resolve: identify a distributable native compiler driver, headers, crt objects and `libc.a`, then rerun without host fallback.

### libc and configure / Modules needs

Native libc is a custom no_std Rust crate, with allocation, errno, memory/string, mapping, thread, clock and TLS components; it is not established as musl or relibc (`sunlight-libc/src/lib.rs:1–56`). Many public Rust functions are not a POSIX C export merely because their names resemble POSIX (`sunlight-libc/src/lib.rs:101–126`). A scoped search found no pthread_create, setlocale, nl_langinfo, dlopen/dlsym or C++ exception/atexit runtime exports in libc/overlay/third_party (`probes/results/inventory-13.log`). The table describes **available build/API evidence**, not a complete link-symbol inventory of a native libc archive.

| API family and representative requirements | Helios build / Linux-host execution | Native build result / gap | Guest result |
|---|---|---|---|
| stdio: FILE, tmpfile, fputs, fgetc, fclose | PASS / PASS | FAIL: stdio.h absent | UNKNOWN both |
| string: memcpy, strcmp | PASS / PASS | FAIL linking strcmp; Rust memory/string implementation exists | UNKNOWN both |
| locale: setlocale; langinfo: nl_langinfo(CODESET) | PASS / PASS each | FAIL: headers absent | UNKNOWN both |
| math: sqrt, fabs/libm | PASS / PASS | FAIL: math.h absent | UNKNOWN both |
| time: realtime/monotonic, nanosleep | PASS / PASS | FAIL: time.h absent | UNKNOWN both |
| dirent: opendir/readdir/closedir | PASS / PASS | FAIL: dirent.h absent | UNKNOWN both |
| termios: tcgetattr | PASS / PASS for tty or ENOTTY | FAIL: termios.h absent | UNKNOWN; interactive tty untested |
| poll/select | PASS / PASS empty zero-timeout sets | FAIL: poll.h absent | UNKNOWN; guest select dispatch absent |
| socket: AF_INET stream | PASS / PASS | FAIL: sys/socket.h absent | UNKNOWN; Helios source rejects AF_INET |
| pthread: create/join and C _Thread_local | PASS / PASS | FAIL: pthread.h absent | UNKNOWN both |
| dlfcn: dlopen(NULL), dlsym(malloc) | PASS / **FAIL** static musl | FAIL: dlfcn.h absent | UNKNOWN both |
| signal: sigaction + raise + user handler | PASS / PASS | FAIL: signal.h absent | UNKNOWN; source cannot invoke user handlers |
| wchar: mbrtowc ASCII | PASS / PASS | FAIL: wchar.h absent | UNKNOWN; UTF-8 conversion untested |
| atexit; getrandom; anonymous mmap/mprotect/munmap | PASS / PASS each | FAIL: required headers absent | UNKNOWN both |

Every cell above comes from its named probe and exact command logs under `probes/results/`; consolidated results: [results.csv](probes/results/results.csv). Probe bodies assert narrow behavior only: locale is C locale, wchar is ASCII, mmap is 1 MiB, termios accepts ENOTTY, and poll/select tests no descriptors. These must not be read as complete POSIX validation. The syscall matrix explains why musl's complete headers cannot establish guest completeness. A complete missing-function list against **every** configure/Modules call is UNKNOWN until a pinned CPython source/configure run produces `config.log` and a symbol audit. Start with the families above and the requested syscall list; do not invent HAVE_* answers.

Upstream baseline: CPython 3.12 requires C11 and thread support; cross builds need a build Python matching the target Python version. See [3.12 configure documentation](https://docs.python.org/3.12/using/configure.html#cross-compiling-options). Host Python here is **3.14.8**, not 3.12 (`probes/results/inventory-02.log`). A matching 3.12 build-python is UNKNOWN; none appears in the `/usr/bin/python*` inventory command executed in this audit. Supply an exact-version host interpreter alongside the pinned target source.

### C++

`cpp-runtime.cpp` tests std::string, iostream, throw/catch, RTTI/dynamic_cast, thread_local, thread creation and atexit. Host g++ builds/runs it successfully. Helios's default `x86_64-linux-musl-g++` is absent, so build FAIL; Native cannot find `<iostream>`, so build FAIL (`probes/results/*cpp-runtime*.log`). **Neither guest C++ runtime ran.** The legacy MuPDF build expects a musl g++ but does not prove it is installed (`tools/build_docserv.sh:17–29`).

libc++/libc++abi/libunwind port status, exception unwinding, C++ thread-local destructors, and cross-translation-unit static initialization ordering are **UNKNOWN for both guests**. No repository-native runtime implementation was identified by the scoped searches (`probes/results/inventory-12.log`, `inventory-13.log`). Rust `panic=abort` is not evidence about C++ exception support (`tools/targets/x86_64-sunlightos.json:12`). Resolve by supplying the intended C++ sysroot, running this probe plus multiple-TU constructor/destructor ordering and unwind-across-C-boundary tests. **C++ is not an M1 acceptance requirement**: the selected CPython baseline is a C11 build; keep the C++ runtime as a separate extension ecosystem milestone (upstream build requirements link above).

## 2. Syscall and kernel surface

The complete requested table is [syscall-matrix.csv](syscall-matrix.csv): 70 rows, CPython use, per-personality status, source evidence and explicit guest-verification column. It covers each requested group, including proc/devices, stack, destructors and overcommit. Status describes the audited interface, not all possible services.

Selected distinctions:

- Translation exists for open/openat, positional I/O, directory enumeration, stat, rename/unlink/mkdir, descriptors, poll/epoll, clocks, entropy and thread operations (`compat-linux/src/lib.rs:375–468`). Neither statx nor ftruncate/fsync/select/alarm/rlimit translations appear in that exhaustive match; unmatched calls return SHIM_ENOSYS (`compat-linux/src/lib.rs:470`). Native file sync syscalls 153/154 exist, but that does not supply Linux fsync (`kernel/src/arch/x86_64/syscall.rs:1047–1048`).
- Helios readlink is not implemented link traversal: it returns EINVAL for an existing path (`kernel/src/arch/x86_64/syscall.rs:6870–6893`). TCGETS accepts only fd 0/1/2 and writes a 60-byte layout; compatibility with musl's requested wire buffer needs a canary probe. FIONREAD has no branch; default ioctl returns EINVAL (`kernel/src/arch/x86_64/syscall.rs:8453–8474,8559–8562`).
- mmap is private anonymous only; fd/offset mappings are rejected. RX is supported, RWX is rejected. Native ordinary mappings allocate frames eagerly; Linux non-PROT_NONE mappings can reserve demand-backed regions. Large allocation/OOM behavior is UNKNOWN until guest stress tests; do not assume Linux overcommit policy (`kernel/src/process/mmap.rs:180–213,216–239,287–305,420–478`). mremap is a native stub and lacks Linux translation (`kernel/src/process/mmap.rs:1560–1567`; `compat-linux/src/lib.rs:375–471`).
- Both clock paths support REALTIME and MONOTONIC; REALTIME reports zero nanoseconds, not high-resolution wall time (`kernel/src/arch/x86_64/syscall.rs:7246–7277,11515–11546`). rseq returns ENOSYS intentionally (`kernel/src/arch/x86_64/syscall.rs:6942–6943`).
- Thread-shaped clone flags are validated and the kernel installs child FS_BASE; process-shaped clone/fork/vfork are rejected (`compat-linux/src/clone.rs:19–54`; `kernel/src/arch/x86_64/syscall.rs:3161–3197,3324`). Native Rust spawn/join and timed futex-backed sleep exist (`tools/native-std/overlay/thread/sunlightos.rs:12–35,52–63`). None of this proves pthread cancellation, robust owner-death or CPython GIL behavior; those are UNKNOWN and need guest stress tests.
- Linux sigaction/mask shims store state, but the handler delivery path never creates a userspace signal frame. Native sigaction/mask/return are explicit stubs (`kernel/src/arch/x86_64/syscall.rs:8065–8175,369–376,10375–10386,10433–10435`). Main-thread Python signal dispatch is therefore not ready; verify targeting, blocked-thread interruption and restoration after implementing frames.
- Initial user stack and native minimum thread stack are 2 MiB (`kernel/src/process/layout.rs:1–6`; `tools/native-std/overlay/thread/sunlightos.rs:7,23`). Automatic stack expansion, rlimit compatibility and safe deep Python/C recursion are UNKNOWN; inspect page-fault policies and run guard-page/recursion tests.
- Socket syscall supports only AF_UNIX SOCK_STREAM (`kernel/src/arch/x86_64/syscall.rs:7567–7584`). Rust capability-based TCP/TLS used by applications does not automatically implement POSIX socket (`docs/rappid-rabbit-native-runtime.md:53–59`). `/proc/self/*` and the requested device nodes are UNKNOWN: the scoped search yielded no implementation; verify the actual guest mount table and opens (`probes/results/inventory-14.log`).

## 3. Native std, naming, loader and filesystem

| Rust std feature | Source-supported scope / unsupported scope | Evidence |
|---|---|---|
| fs | File open/read/write/seek and limited attributes; directory iteration/mutations, timestamps, sync, truncate, permissions, canonicalization and links unsupported | `tools/native-std/overlay/fs/sunlightos.rs:46–64,237–361` |
| io | stdin/stdout/stderr through native fd operations; flush is a no-op | `tools/native-std/overlay/stdio/sunlightos.rs:21–40` |
| thread/sync/TLS | Spawn/join/detach, timed-futex sleep, upstream futex synchronization selection; 255 usable TLS keys and up to five destructor passes | `tools/native-std/overlay/thread/sunlightos.rs:7–63`; `tools/native-std/prepare.py:45–56`; `tools/native-std/overlay/thread_local/key/sunlightos.rs:6–51` |
| time | SystemTime/Instant call native clocks | `tools/native-std/overlay/time/sunlightos.rs:12–15,35–36,52–59` |
| env/args/paths | Process-local environment store, argv adapter and getcwd/chdir; current_exe unsupported | `tools/native-std/overlay/env/sunlightos.rs:4–23,37–72`; `tools/native-std/prepare.py:41–44`; `tools/native-std/overlay/paths/sunlightos.rs:4–14,64–65` |
| net | Existing port documentation explicitly says std::net unsupported; overlay adds no net backend | `docs/rappid-rabbit-native-runtime.md:53–59`; `tools/native-std/prepare.py:40–44` |
| process | No target-family unix in target JSON; staged std selects unsupported fallback | `tools/targets/x86_64-sunlightos.json:1–21`; command: `nl -ba target/native-std/sysroot/lib/rustlib/src/rust/library/std/src/sys/process/mod.rs` (lines 1–21) |

Native TLS starts with a one-page TCB, FS self-pointer and errno; this is not proof of a general compiler-generated ELF TLS layout (`sunlight-libc/src/tls.rs:1–44`). Validate C `_Thread_local`, executable PT_TLS, TLS alignment/initializers and destructor ownership before sharing a C/Rust runtime. Helios auxv explicitly supports musl locating PT_TLS (`kernel/src/process/spawn.rs:338–359`).

Native CPython `sys.platform` is **UNKNOWN / not chosen**. Rust cfg naming is established above, but it does not set Python's platform name. Proposed policy (not implementation): `sys.platform="sunlightos"`, `--host=x86_64-unknown-sunlightos`; decide SOABI and wheel platform independently. Helios proposal: retain Linux/musl Python conventions for Linux ABI libraries, while documenting the restricted syscall contract. Upstream 3.12 cross configure accepts Linux patterns but rejects unrecognized operating systems; Native needs an explicit port case ([configure.ac](https://raw.githubusercontent.com/python/cpython/3.12/configure.ac), lines 505–532 and 565–589). Actual config.guess/config.sub recognition of a pinned Sunlight triple is **UNKNOWN**: source was not available locally and the upstream config.sub fetch failed. Resolve by vendoring a selected release under audit scratch and executing `sh config.sub x86_64-unknown-sunlightos`; run config.guess in a guest only after uname is defined. Do not infer recognition from clang accepting an object target.

Dynamic loading: the process ELF planner rejects PT_INTERP; no dlopen/dlsym export was found in the native runtime scope; static musl's dlopen probe fails even on Linux (`sunlight-elf/src/lib.rs:212–214`; `probes/results/inventory-13.log`; `probes/results/helios-dlfcn-host-run.log`). The audited paths do not provide usable loadable extensions. **Proposed v1:** build required C extensions into the interpreter with Modules/Setup static mode, use frozen startup/encodings and an uncompressed stdlib zip; this avoids a shared-library loader, but cannot make arbitrary third-party binary wheels load. Upstream exposes static/shared module modes ([Modules/Setup.stdlib.in](https://raw.githubusercontent.com/python/cpython/3.12/Modules/Setup.stdlib.in), lines 18–22). Static-link feasibility for each selected extension remains UNKNOWN until link and guest import tests.

Filesystem semantics shared beneath both personalities:

- `/` paths and NUL rejection are explicit in VFS validation; native libc has MAX_PATH=256 and native cwd uses a 256-byte buffer (`sunlight-fs/src/path.rs:3–15`; `sunlight-libc/src/lib.rs:69–73`; `tools/native-std/overlay/paths/sunlightos.rs:4–9`). This is a bounded native interface, not evidence of Linux PATH_MAX=4096 compatibility. Test boundary lengths on each mount.
- Case behavior varies by backend: TmpfsDir compares exact bytes, FAT lookup ignores ASCII case (`sunlight-fs/src/tmpfs.rs:46–50`; `sunlight-fat/src/fat.rs:270–273`). Do not describe the entire OS as case-sensitive.
- VFS FileType has only File/Directory and FileStat has uid/gid/mode/nlinks but no timestamps (`sunlight-fs/src/vfs.rs:50–54,80–88`). Unix permission bits exist, but enforcement/persistence across every backend is UNKNOWN without per-mount tests (`sunlight-fs/src/vfs.rs:56–78`). Symlink support is MISSING in this exposed model; actual mtime semantics are UNKNOWN, and Native std explicitly returns unsupported (`tools/native-std/overlay/fs/sunlightos.rs:56–63,334–339`). Test pyc invalidation, rename atomicity, executable permissions and long/non-ASCII paths before stdlib/install claims.

## 4. CPython acceptance checklist

These are **port readiness** labels, not upstream CPython module availability. No CPython interpreter was built or run; implementation-independent modules are UNKNOWN rather than optimistically READY. Use exact-version CPython source and record config.log, Setup.local, pyconfig.h and module manifests. Every row's next test is a proposal.

| Requirement | Helios | Native | Evidence / how to resolve |
|---|---|---|---|
| `python -c 'print(1)'` | UNKNOWN | UNKNOWN | No CPython build/run in `probes/results/results.csv`; build M1 and capture exit/stdout |
| `_io`, `posix`, `_stat` | PARTIAL | PARTIAL | File/descriptor kernel support but API gaps: `syscall-matrix.csv`; C headers fail Native; import and file roundtrip |
| `time` | PARTIAL | PARTIAL | Both clock implementations above; C Native time probe fails; exercise monotonic/sleep |
| `_thread` | PARTIAL | PARTIAL | clone/native spawn exist; no guest pthread/GIL run; `kernel/src/arch/x86_64/syscall.rs:3176`; native thread overlay |
| `_signal` | MISSING functional handlers | MISSING functional handlers | `kernel/src/arch/x86_64/syscall.rs:369–376,10375–10386` |
| `math` | PARTIAL | MISSING in tested C sysroot | `probes/results/helios-math-build.log`, `native-math-build.log`; build/import and IEEE edge tests |
| `_locale` | PARTIAL | MISSING in tested C sysroot | locale/langinfo probes; guest C/UTF-8 locale and coercion tests |
| `errno`, `_sre`, `itertools`, `_collections`, `_functools`, `_struct`, `_weakref`, `_abc`, `marshal`, `_codecs`, `atexit` | UNKNOWN each | UNKNOWN each | No target interpreter/module link; test each builtin import and behavior, not just importlib lookup |
| zlib (optional) | UNKNOWN | UNKNOWN | No C zlib target build tested; static library/link/import needed |
| zipimport / frozen modules / encodings / site | UNKNOWN each | UNKNOWN each | No Python installation fixture; M2 frozen bootstrap then read-only zip, site on/off |
| Unicode / UTF-8 mode (`PYTHONUTF8=1`) | PARTIAL | PARTIAL | Host ASCII wchar/C locale only; test non-ASCII argv/env/path/stdio and surrogateescape in guest |
| Basic REPL / readline | PARTIAL / UNKNOWN | UNKNOWN / UNKNOWN | Helios ioctl limitation above; no readline library test; interactive PTY/edit/resize/Ctrl-C |
| pip / ensurepip | UNKNOWN / UNKNOWN | UNKNOWN / UNKNOWN | No interpreter or wheel bootstrap; test offline ensurepip first, then HTTPS pip; network blocker above |
| venv | PARTIAL | PARTIAL | Paths/file APIs exist but symlink/metadata gaps; test `--copies`, relocation, script shebang and activation |
| ssl | MISSING socket prerequisite; C OpenSSL UNKNOWN | MISSING tested C socket interface; C OpenSSL UNKNOWN | `kernel/src/arch/x86_64/syscall.rs:7579–7580`; `native-socket-build.log`; Rust TLS is not C OpenSSL; build dependency then handshake test |
| sqlite3 | UNKNOWN | UNKNOWN | Target sqlite C library/locking/durability untested; link and transactional filesystem tests |
| ctypes | MISSING general shared-library loading | MISSING general shared-library loading | Loader and dlopen evidence above; libffi/callback support UNKNOWN; test static symbols separately |
| select module | PARTIAL | MISSING tested C API | Helios poll/epoll shims, no select translation; `compat-linux/src/lib.rs:398,463–465`; `native-poll_select-build.log` |
| socket module | PARTIAL: Unix stream only | MISSING tested C API | `kernel/src/arch/x86_64/syscall.rs:7567–7584`; `native-socket-build.log` |
| subprocess | MISSING standard Linux process path | MISSING demonstrated Python backend | fork/vfork/wait4 rejection; Native std process unsupported; implement spawn/wait integration |
| multiprocessing | MISSING standard process modes | MISSING demonstrated Python backend | Same process gaps; also file-backed/shared mmap rejected; test spawn and shared resources after M4 |
| asyncio | PARTIAL foundations only | PARTIAL foundations only | Clocks/threads exist, socket/signal interfaces incomplete; test selector timers/cancellation/socketpairs before TCP |
| Matching build-python | UNKNOWN for 3.12 | UNKNOWN for 3.12 | Available host Python 3.14.8 (`probes/results/inventory-02.log`); provision exact 3.12 source-version host binary |
| test.regrtest allow-list | UNKNOWN / NOT RUN | UNKNOWN / NOT RUN | No target Python; proposed allow-list below |

Proposed staged test command (not executed): `./python -m test -j1 test_grammar test_int test_float test_unicode test_codecs test_struct test_itertools test_collections test_functools test_weakref test_atexit test_errno test_math test_time test_io`. First confirm names in the pinned release and install its `test` package. This invokes the standard test runner/regrtest machinery; record skips and failures per test, and require no unexpected skip for the selected contract. Add `test_threading`, `test_queue`, `test_select`, `test_asyncio`, `test_socket`, `test_subprocess`, `test_venv`, `test_ensurepip`, `test_ssl`, `test_sqlite3`, and `test_ctypes` only at the relevant milestone. Missing optional modules must not be reported as test passes.

## 5. Risk register

Scores are **planning estimates**, not measured probabilities: likelihood and impact 1–5, ranked product. Owners are suggestions. Evidence identifies why a risk exists; mitigations are proposed work.

| Rank | Risk | L×I | Evidence | Mitigation / suggested owner |
|---|---|---|---|---|
| 1 | Incomplete signal frame/return and main-thread delivery | 5×5=25 | `kernel/src/arch/x86_64/syscall.rs:369–376,10433–10435` | Design saved context, altstack, masks/restorer, interruption and targeting; kernel scheduler/arch owner; **possible kernel redesign** |
| 2 | Native C runtime/API contract absent from tested toolchain | 5×5=25 | `probes/results/native-stdio-build.log`, `native-pthread-build.log` | Publish sysroot + crt/libc/libm/pthread ABI and symbol inventory; toolchain/libc owner |
| 3 | Process model cannot supply fork/vfork/wait4 | 5×5=25 | `kernel/src/arch/x86_64/syscall.rs:3161–3173,6012–6017` | Prefer safe spawn+exec+blocking wait for v1; implement compatible fork only with sound VM ownership; process/VM owner; **possible kernel redesign** |
| 4 | No POSIX Internet sockets | 5×4=20 | `kernel/src/arch/x86_64/syscall.rs:7579–7580` | Bridge service TCP/UDP/DNS to fd/socket semantics; network + Helios owners |
| 5 | Loader/file mmap blocks extensions and shared memory | 5×4=20 | `sunlight-elf/src/lib.rs:212–214`; `kernel/src/process/mmap.rs:230–233` | Static modules first; later file/shared mapping lifecycle, relocations, loader/TLS; VM/loader owner; **possible VM redesign** |
| 6 | Thread/GIL/futex/TLS mismatch under load | 4×5=20 | `compat-linux/src/clone.rs:19–54`; `tools/native-std/overlay/thread_local/key/sunlightos.rs:6–51`; no guest results | pthread mutex/condvar/timedwait/keys and GIL churn, join/exit/races; scheduler/libc owner; **validate design before expanding it** |
| 7 | Metadata/durability/links break stdlib/venv/sqlite | 5×3=15 | `sunlight-fs/src/vfs.rs:50–88`; native fs overlay `:237–361` | Define per-mount contract; pyc, fsync, rename, permissions, truncate and link tests; VFS owner |
| 8 | Terminal ABI corruption or unusable Ctrl-C | 4×3=12 | `kernel/src/arch/x86_64/syscall.rs:8453–8474,369–376` | Canary TCGETS buffers against musl ABI; PTY integration and signal tests; tty/Helios owner |
| 9 | Memory/stack limits differ from Linux assumptions | 3×4=12 | `kernel/src/process/mmap.rs:420–478`; `kernel/src/process/layout.rs:6` | Bound interpreter arena/stack budget; large reservation vs touch/OOM tests; VM owner; **overcommit/stack policy review** |
| 10 | Cross-config and dependency version drift | 4×3=12 | `tools/native-std/prepare.py:8–11`; `inventory-02.log`; upstream cross-config rejection | Pin CPython, build-python, musl, overlay compiler, dependency archives and config cache; build/release owner |

## 6. Milestones, effort and done criteria

Sizes below are **estimates/inferences**, not calendar promises: S=one narrow integration, M=several bounded components, L=several subsystems, XL=platform design plus integration. Confidence reflects the evidence gaps, especially lack of guest C runs. No user-visible C++ extension guarantee is included in M1.

| Milestone | Helios size/confidence | Native size/confidence | Done criterion (proposed) / evidence motivating work |
|---|---|---|---|
| M0 toolchain | M / medium | XL / low | Pinned source + exact build-python, reproducible sysroot, C/pthread/libm probes run in isolated guest; `probes/results/results.csv` |
| M1 static hello | L / low | XL / low | ET_EXEC static interpreter prints `1`, exit=0, clean repeated startup/shutdown; all requested required builtins linked, including functional threads and signals; loader `sunlight-elf/src/lib.rs:129,212` |
| M2 stdlib import | M / low | L / low | Frozen bootstrap, encodings and read-only stdlib zip; site on/off and early regrtest allow-list; bounded paths/metadata evidence in section 3 |
| M3 threads/asyncio | L / low | XL / low | GIL/condition/TLS-dtor stress, timers/cancellation, selector pipe/socketpair loop; no claims of TCP until M4; clone/TLS gaps in section 2 |
| M4 sockets/subprocess | XL / low | XL / low | TCP/UDP/DNS, nonblocking deadlines, safe spawn+exec+stdio+wait+reap, process-mode documented limitations; socket/fork/wait rejections above |
| M5 pip/venv | L / low | XL / low | Offline ensurepip, venv `--copies`, pure-Python wheel install, HTTPS certificate verification; no native binary-wheel claim; filesystem and socket gaps above |
| M6 dynamic extensions | XL / low | XL / low | dlopen/dlsym, relocations, permissions, TLS constructors/destructors, load/unload cycles, built extension ABI + ctypes/libffi; file mmap/PT_INTERP rejections above |

**Order:** complete Helios M0→M1→M2 first, while specifying Native libc interfaces from actual CPython configure/link failures. Then reuse shared kernel fixes for Native. This recommendation follows the measured successful musl C links and failed Native runtime links, plus existing static musl app packaging (`probes/results/results.csv`; `tools/build.sh:124–130`). Parallel design discussions are reasonable; two unverified interpreter integrations would duplicate unknowns. Shared fixes: signal delivery/return, VM ownership and mapping semantics, safe spawn/wait primitives, VFS metadata/durability/path contract, FD/poll readiness and a network service socket adapter (`kernel/src/arch/x86_64/syscall.rs` handlers and `syscall-matrix.csv`). ABI wrappers, startup/TLS layout and configure naming remain personality-specific.

### Regression and differential matrix

**The literal “same binary under both personalities behaves identically” is not currently a valid executable contract.** Personality selects different syscall numbering and errno behavior; changing an ELF brand alone is insufficient (`sunlight-elf/src/lib.rs:110–126`; `kernel/src/arch/x86_64/syscall.rs:574–599,1029–1032,1214–1217`). Proposed acceptance: (1) same source/tests with personality-correct builds and identical observable results, (2) the identical stamped Helios binary on Linux-musl reference and SunlightOS Helios, and (3) if literal byte-identical dual-personality operation is mandatory, separately design an explicit dual-ABI runtime/loader interface. Do not claim (3) by passing (1).

| Test set (future) | Differential observations / normalization | Configurations |
|---|---|---|
| Startup, print, builtin imports, shutdown | Exact stdout/stderr/exit; repeat startup and atexit order | Both correct-ABI builds; Linux reference |
| Byte/text I/O, seek/pread/pwrite, fd duplication/pipe/CLOEXEC | Same bytes, errno and EOF/EAGAIN; no shared offset change for pread | Both; ram/tmp/FAT backend fixtures |
| Imports, UTF-8 argv/env/files, encodings, zip/frozen/site | Same values/surrogateescape; path-limit failures explicitly recorded | Both; C locale and UTF-8 mode |
| Metadata/rename/unlink/truncate/fsync | Same documented per-mount effects and failure codes; isolate timestamp resolution | Both; writable snapshot only |
| Clock/sleep | Monotonic, no early timeout; tolerate scheduler granularity; compare wall-clock resolution separately | Both, 1/2/4 CPUs |
| Thread/TLS/GIL/dtors | Same deterministic counter, key isolation, destructor count; 1,000 joins and detach churn without stale wake | Both, 1/2/4 CPUs |
| Signals/REPL | Main-thread handler, Ctrl-C→KeyboardInterrupt, altstack restoration, EINTR/restart, tty canary integrity | Both; attached and detached tty |
| Allocation | 1 MiB and 64 MiB reserve/touch/unmap, PROT_NONE→RW, RX, rejected RWX/file-map rules, bounded OOM cleanup | Both, constrained guest RAM; faulting checks in child fixtures |
| Async/select/socket | Pipe/socketpair readiness, timer cancellation, TCP/UDP/DNS timeout, EOF/HUP/error | Both; loopback/deterministic local service |
| Processes | spawn/exec/env/stdio, missing executable, exit and signal status, wait/reap, no zombies | Both once M4; unsupported fork recorded explicitly |
| Install/extensions | ensurepip offline, venv copies, pure wheel, TLS trust failure, sqlite transaction, extension/ctypes cycle | Both at M5/M6; no internet-dependent gate |

Record source revision, binary hashes, brand, compiler/sysroot, pyconfig/Setup, guest image hash, CPU/RAM, logs, exit status, expected skips and leak/resource counters per run. Platform identifiers and unsupported extensions must be explicit expected differences; do not normalize errno/bytes/data-loss failures away. This is a proposed test policy, not evidence of passed tests.

## 7. Next 10 tasks (ordered proposals)

1. Pin one CPython 3.12 patch release/source hash and provide its matching build-python; inventory shows only host 3.14.8 (`probes/results/inventory-02.log`).
2. Add an isolated audit-ELF guest execution fixture in a separately authorized implementation change; run supplied static musl probes (current guest results UNKNOWN).
3. Stress musl pthread mutex/condvar/TLS keys/clear-tid/futex and inspect failed syscalls before assuming GIL readiness (`compat-linux/src/clone.rs:19–54`).
4. Implement and test signal frame entry/return, masks, main-thread targeting and interruption (`kernel/src/arch/x86_64/syscall.rs:369–376,10433–10435`).
5. Run real Helios cross configure/link with a static module manifest; retain all failed checks and resolve actual required symbols rather than synthesizing answers (section 1).
6. Reach M1 guest hello/builtin acceptance, then package frozen/zip encodings+stdlib and run the M2 allow-list (section 4; current interpreter UNKNOWN).
7. Define Native C ABI/sysroot/startup/TLS contracts and implement the measured missing header/export families; rerun every C probe (`probes/results/native-*-build.log`).
8. Close shared filesystem and terminal ABI gaps: stat timestamps, truncate/sync, paths/links policy and TCGETS canaries (`sunlight-fs/src/vfs.rs:80–88`; `kernel/src/arch/x86_64/syscall.rs:8453–8474`).
9. Add POSIX socket adapters and safe spawn/exec/wait contracts, then selectors/asyncio, subprocess and offline ensurepip/venv (`kernel/src/arch/x86_64/syscall.rs:7567–7584,6012–6017`).
10. Implement dynamic mapping/loader/extension ABI and separately qualify C++ libc++/ABI/unwind/TLS/initialization tests (`kernel/src/process/mmap.rs:230–233`; `sunlight-elf/src/lib.rs:212–214`).
