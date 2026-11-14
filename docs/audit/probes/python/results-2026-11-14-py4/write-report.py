from pathlib import Path
import json
E=Path(__file__).resolve().parent;R=Path.cwd()
def read(n):return json.loads((E/n).read_text())
s=read('validation-summary.json');m=read('cpu-matrix.json');x=read('cpython-object-matrix.json');integrity=read('source-integrity.json');files=read('modified-production-files.json')
gaterows='\n'.join(f"| {k} | {v['status']} | {v['reason']} |" for k,v in s['gates'].items())


cpulines='\n'.join('| '+cpu+' | '+' | '.join(f"{v[k]['passed']}/{v[k]['total']} {v[k]['status']}" for k in ['C','CXX','lifecycle','existing_functional','PY4'])+' | '+v['TCP']['status']+' |' for cpu,v in m['cpus'].items())
failed='\n'.join('| `'+u['object']+'` | '+u['ownership_layer']+' | `'+u['first_failing_diagnostic'].replace('/tmp/sunlight-py1-cpython/Python-3.13.0/','').replace('|','\\|')+'` |' for u in x['units'] if u['compile_result']=='FAIL')
filelines='\n'.join('* `'+r['path']+'`' for r in files)
text=f'''# PY-4: native filesystem, process, signal and entropy contracts

2026-11-14. **Overall: PARTIAL. PY-4 is not COMPLETE.**

All **180 configured CPython 3.13.0 objects were retried: {x['passed']} PASS, {x['failed']} FAIL, zero unattempted**. The baseline independently reproduced PY-3's 163/180 result. Real C-locale metadata and native group-identity interfaces repair `Modules/_localemodule.o` and `Python/sysmodule.o`. No interpreter static link or native Python execution occurred. Filesystem, required signal, nonblocking entropy, mutable environment, calendar execution and buffered stdio contracts remain incomplete. No fake POSIX behavior or insecure random fallback was added.

Evidence root (**E**): `docs/audit/probes/python/results-2026-11-14-py4/`.

| Gate | Status | Evidence boundary |
|---|---|---|
{gaterows}

Blocked capabilities are not counted as passing gates. G6 assesses the complete build retry and preserved diagnostics; compilation itself fails. G3 and G5 remain PARTIAL independently of G2/G4. G8 reviews these seven changes and their integrity, without closing inherited runtime security/race milestones.

## Reproduced baseline and source identity

Branch `main`, revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, incoming dirty worktree. `source-baseline.json` records status and {integrity['incoming_regular_files']:,} existing tracked/unignored regular-file hashes before production changes. Existing modifications were preserved. No reset, stash, clean, commit or push occurred.

The pinned archive `/tmp/sunlight-r1-Python-3.13.0.tar.xz` SHA-256 is `086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`. Every regular archive member was compared with `/tmp/sunlight-py1-cpython/Python-3.13.0`: only inherited PY-1 `configure`/`config.sub` recognition differs. No new upstream edits were made. `source-identity.json` retains the source hash inventory; `toolchain-identity.json` records the matching `/tmp/sunlight-r1-cpython/build-host/python` 3.13.0 executable identity and native compiler/tool versions. That Linux build interpreter is not native Python evidence.

`locked-baseline.log` records the successful canonical `bash tools/build_native_c.sh` rebuild with the existing `--release --locked` recipe. Clean cross-configure/build in `/tmp/sunlight-py4-cpython/cross-static` reproduced 180 attempts, 163 successes and the same 17 failed units; raw evidence is in `baseline-build/`. This is an installed-toolchain rebuild using retained caches, not clean-room or byte-identical reproducibility. Before edits, `dependency-graph.json` grouped every failure by contract/ownership; its original inventory is retained in `baseline-build/dependency-graph.json` and final progress is appended separately.

## Filesystem metadata and realpath

**POSIX stat/lstat and realpath: BLOCKED.** The existing native metadata subset is exercised in the guest; that does not satisfy G2.

`sunlight-fs/src/vfs.rs` defines only File/Directory and a `FileStat` containing size, uid, gid, native mode and nlinks. `sunlight-libc/src/lib.rs:Stat`, syscall `sys_stat_path` and native `sys_fstat` share a 24-byte, 8-byte-aligned ABI: size u64 at 0, uid/gid u32 at 8/12, mode u16 at 16, type u8 at 18, pad at 19, nlinks u32 at 20. Device/inode identity, authoritative timestamps and special-file/symlink identity are absent. The Linux compatibility adapter's synthetic inode/stat values are not an authoritative native substitute. Its layout is not reused.

RAMFS/tmpfs expose their stored subset metadata. The existing FAT adapter explicitly supplies fixed root-owned 0755 policy, not on-disk POSIX ownership/permissions; no new fabrication was introduced. A correct extended metadata contract must define stable backend-owned identities, mount incarnations, time update policy and a versioned wire ABI before introducing POSIX `struct stat`. Existing fstat destination layout was preserved. Generic backend fstat failures still collapse to EIO; accurate complete POSIX error mapping is not claimed.

`sunlight-fs/src/path.rs:validate_absolute` rejects relative paths, dot/dot-dot and trailing separators. `Vfs::resolve_mount` chooses the longest component-boundary mount prefix. Cwd prefix expansion exists in `resolve_current_path` for some operations, including open, but **StatPath61 does not use it**. Neither prefix expansion nor `handle_path` supplies a permission-checked canonical component walk. No native symlink/readlink/loop interface exists. A lexical `realpath` would invent existence/authority guarantees and remains absent, including allocation and caller-buffer variants. Path canonicalization would still not authorize a later raced open.

`filesystem-test.c` exercises root RAMFS directories and mounted tmpfs, a writer worker alongside repeated regular-file metadata reads, final size, type/mode/link invariants, missing paths, rejected relative/dot paths and cleanup. PY-3's retained fixture rechecks fd/path agreement, errors and unlink. The worker/read fixture is a functional concurrent-caller control; precise write/stat overlap is not forced and remains UNQUALIFIED. Permission-denial injection, concurrent rename/mount changes, persistent FAT execution and symlink chains/loops are NOT_RUN. Extending every backend and traversal authority is an independent VFS milestone; a libc declaration would not repair it. See `filesystem-contracts.json` and `realpath-validation.json`.

## Process identity and exit

**Native group identity implemented; full process/signal gate PARTIAL.** A scoped read-only syscall 178 exposes selector 0 retained group PID, 1 stored group PPID, and 2 calling task TID. It matches the complete existing `AddressSpaceIdentity` through `Scheduler::native_group_owner_index`, including generation, and reads the result under SCHEDULER. The retained owner remains available after initial-thread pthread_exit until borrowers reap. No target PID, user pointer, new allocation, nested subsystem lock or cached reference is accepted.

Libc `getpid`, `getppid` and native-extension `gettid` return the existing int `pid_t`, preserve errno on success, and reject overflow with EOVERFLOW. Kernel/ipc/libc numbers match. Historical raw syscall33 and Rust `crate::getpid` keep task-identity behavior; the C group query uses a separate ABI. PID observations do not authenticate a process or grant authority to a stale numeric identifier.

`getppid` returns the retained group's stored parent. The guest fixtures have parent 0; there is no orphan-reparenting implementation or live-child reparenting test. It is a bounded native stored-parent contract, **not a full POSIX parent-lifecycle claim**. Enormous-PID overflow and adversarial identity reuse remain unexercised. No process/group lifetime refactoring was performed.

`process-test.c` verifies common owner/worker group identity, distinct task IDs, stored-parent equality, repeated reads, invalid selectors and stable identity after owner pthread_exit. A raw syscall156 observation independently emits the existing kernel `owner_finished=1 quiescent=1 group_live=1` witness before the surviving worker's identity/progress output. Delays alone do not qualify owner death. The source/guest evidence is in `process-identity-design.json`, `process-exit-contracts.json` and per-CPU serial logs.

Existing `_Exit` and `_exit` remain callback-free process-wide syscall157 paths; `exit` and main return retain normal process-wide behavior, and pthread_exit remains caller-thread-only. Existing 17 lifecycle cases and immediate-exit controls are rerun. Competing termination/retirement interleavings and exactly-once fault-injection qualification remain separate inherited milestones; functional results do not prove those schedules.

## Signals and entropy

**Required signal delivery: BLOCKED.** Source review of `signal.rs`, `native_signal.rs` and syscall174 confirms initial-thread synchronous SIGUSR1/2 registration/raise, default/ignore, same-signal deferred/coalesced pending state and validated handler return context. Workers and cross-signal nesting are rejected. kill/sigprocmask explicitly fail ENOSYS. Required SIGINT/SIGTERM/SIGPIPE, asynchronous process/thread targeting, masks, interruptible waits/EINTR, fatal handlers and alt-stack/SA behavior are absent; SIGALRM is not advertised. Existing signal tests exercise the real subset and restoration; PY-4 adds native kill/mask rejection checks. No new signal stubs/constants or scheduler delivery were added. Safe async delivery requires its own lifecycle, identity, context and wait-coordination milestone. See `signal-contracts.json`.

**Secure nonblocking entropy: BLOCKED.** `kernel/src/entropy.rs` collects a 40-byte approved-source boot seed from virtio-rng, RDSEED or RDRAND, publishes a ChaCha20 conditioner before readiness, and fails closed without a complete source. Source trust rests on host/device or CPU/hypervisor behavior; timing and PIDs are excluded. `rand_service` refuses registration without readiness and a complete acceptable seed, serializes its ChaCha20 stream and periodically reseeds from the kernel conditioned stream. The kernel itself has boot-only seeding: service reseeds are **not fresh external physical entropy collection**.

The existing flags-zero C adapter uses capability IPC in 32-byte replies, with 2-second lookup and 5-second per-chunk bounds; failed requests wipe destination and report generic EIO. Timed waits do not meet a nonblocking ready-or-EAGAIN contract. Rust bit1 selects NONCRYPTO; C deliberately rejects it and must never alias it to GRND_NONBLOCK. No new readiness shortcut, kernel-stream substitution, fake /dev/urandom or weak fallback was introduced. Existing random guest tests exercise actual service calls and boundaries; the new rejection fixture preserves canaries. Statistical output is not a cryptographic proof. Cold-source, service-fault and concurrent cross-process isolation attacks were not forced. See `entropy-contracts.json` for construction/reseeding/isolation assumptions and limits.

## Locale, environment and stdio

**C-locale metadata completed; G5 PARTIAL.** `langinfo.h` and `langinfo.rs` add 47 real C metadata items: full/abbreviated weekday/month names, calendar formats, AM/PM, decimal point and empty thousands/currency metadata. CODESET=1 is unchanged and tracks LC_CTYPE. New constants occupy native values 256..302, preserving original reserved values, including 2. All accepted locale names retain the installed C calendar/numeric/monetary baseline; en_US.UTF-8 does not imply a complete US locale.

Strings are immutable, process-lifetime storage; no allocation or caller-buffer retention occurs. Unknown items and integer extremes return empty without changing errno. Native tests cover all names/abbreviations/formats, selection, errno, stable pointers and four concurrent readers. C and C++ signature checks pass with base/POSIX/GNU feature macros. These format strings do not implement strftime, tm/timeval conversions, timezone policy or calendar execution.

Mutable environment remains BLOCKED: `env.rs` returns immutable static Rust borrows into startup envp, while `tools/native-std/overlay/env/sunlightos.rs` owns a separate mutex-protected mutable snapshot. Its Rust setenv/unsetenv do not update libc envp. Safe setenv/unsetenv/putenv need a shared ownership/synchronization/inheritance design. Freeing or mutating live static borrows, or indefinitely leaking copied environments, is not a correct cleanup contract. No mutable API was added.

Full and line-buffered stdio remain BLOCKED. Existing real unbuffered I/O and setvbuf(_IONBF) are retained; buffered modes fail ENOSYS rather than accepting flags without behavior. Correct buffering requires owned pending storage plus flush, seek, close and exit integration under existing stream locks. Existing real I/O/stream-policy tests are rerun; buffered-mode ownership/concurrency tests are NOT_RUN without an implementation. See `locale-environment-buffering.json`.

## CPython compilation and linking

Final clean isolated configure/build: `/tmp/sunlight-py4-final-cpython/cross-static`, exact retained upstream source and matching 3.13.0 build interpreter. `configure-manifest.json` records the actual native driver, static flags, llvm-ar/ranlib and unchanged cross-cache/Setup.local selections. No missing feature was forced present. All 180 original configured units, including selected unicodedata, remain in the denominator.

`core-make.log` captures genuine make -k; independent unlimited-error compiles preserve every command, result, first diagnostic and relevant missing identifiers/contract dependencies in `cpython-object-matrix.json`. The exact upstream shell recipe handles getbuildinfo substitutions. Fatal includes may still mask later requirements: this is not a complete linker-symbol inventory. `core-serial.log` independently reproduces the realpath first failure through make -j1.

| Failed object | Owner | First final diagnostic |
|---|---|---|
{failed}

**Static interpreter link: NOT_RUN. Native interpreter execution: NOT_RUN.** Mandatory objects are absent, so no real linker invocation, unresolved-symbol inventory, map, interpreter ABI/TLS/relocation conclusion or native Python sentinel exists. The dry-run prospective linker command is not a link attempt. No weak fallback symbols, fake APIs or object omissions were used. `static-link-results.json` preserves these distinctions. Archive ownership checks confirm one definition of each new identity API/nl_langinfo and existing _Exit/_exit; that is runtime archive evidence only.

## Final-source regression matrix

| CPUs | C | C++ | Lifecycle | Existing PY-3 functional | New PY-4 | TCP |
|---|---|---|---|---|---|---|
{cpulines}

Regression status: **{m['status']}**. Lifecycle17 is included in functional40, not an extra disjoint total. Each functional run includes the unchanged formatted-I/O fixture checking 1,780 host-oracle cases in the native guest, heap/ABI, TLS/once, stream/math/header controls and PY-3's conversion/filesystem/exit cases. Five new cases bring the functional total to45. TCP at1 CPU is unsupported by the retained runner and NOT_RUN.

Target pthread/C++ ABI, vendor provenance, archive ownership and historical ELF checks remain separate from guest runtime. `validated-executions.json` binds exact commands and results to `validated-source.json`, checked after each command; `cpu-matrix.json` includes actual per-case results. Guest directories preserve serial/stdout/stderr, image/source hashes, completion and resource/reap receipts. Generated ignored images/ELFs are not source changes.

## Security, integrity and next milestone

`security-review.json` reviews the seven production deltas: no new target authority, scalar identity overflow handling, generation-qualified owner lookup, one existing scheduler lock, immutable locale ownership, unchanged VFS authorization, signal context, RNG readiness, IPC ownership, memory retirement and fail-stop boundaries. No new production polling or dependency cycle was introduced. Stored-parent/reparenting, retirement faults, deterministic competing schedules and group-integrated security witnesses remain explicitly unqualified.

`source-integrity.json`: **{integrity['status']}**. Every incoming regular file is compared, with exactly these authorized production changes, no missing incoming files, no changed CPython source or previous evidence. The standalone `py4-production.patch` applies to saved incoming contents and reverses byte-for-byte in a temporary copy. Directory/symlink metadata and ignored build products are outside this regular-file manifest. Focused rustfmt, whitespace and ABI checks have separate logs.

Production files:

{filelines}

The report is `docs/runtime/python-py4-native-platform-contracts.md`; all required JSONs, raw compiler/configure/guest logs, incoming snapshots, standalone patch and validation/assessment drivers are under E. No commit or push.

Recommended **PY-5: authoritative native filesystem metadata and component resolution**. Define backend-owned inode/device identity and timestamp policy plus a versioned stat ABI, then implement one permission-preserving component resolver with cwd, mount and existing-object semantics before libc stat/lstat/realpath. Keep symlinks explicitly unsupported until their native representation and loop contract exist. Validate actual mount/path/error/concurrent mutation behavior in the guest. Follow with independent async-signal and secure nonblocking-entropy milestones, shared environment ownership, calendar execution and buffering. Reattempt the identical complete CPython object set; link only after mandatory compilation succeeds, then require actual native interpreter output.
'''
(R/'docs/runtime/python-py4-native-platform-contracts.md').write_text(text)
print('report generated',len(text.split()),'words')
