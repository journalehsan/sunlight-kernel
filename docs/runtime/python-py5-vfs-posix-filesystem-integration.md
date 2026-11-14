# PY-5: VFS and POSIX filesystem integration audit

2026-11-14. **Overall: PARTIAL. PY-5 is not COMPLETE.**

The hypothesis of existing native symlink and authoritative canonical-path support is **not supported by the inspected implementation**. SunShell and Sunlight Files implement lexical path processing over native filesystem syscalls. VFS routes mounts and validates backend handles, but does not implement a common credential-aware component resolver. Native symlink storage/traversal is absent. Full POSIX metadata cannot be translated from the existing subset without inventing fields.

One demonstrated native VFS defect was repaired: `Vfs::handle_path` dropped non-root mount prefixes. No POSIX declarations, fabricated metadata, duplicate resolver, scheduler changes or IPC changes were introduced. CPython compilation remains **165/180**, with all 180 configured objects attempted and the same 15 failures. Interpreter static linking and execution are **NOT_RUN**.

Evidence root (**E**): `docs/audit/probes/python/results-2026-11-14-py5/`. Gate outcomes and the final CPU matrix are recorded in `validation-summary.json` and `cpu-matrix.json`; the final results section below is generated from those records.

## Baseline and integrity

Incoming branch `main`, revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, dirty worktree. `source-baseline.json` records 81,799 incoming tracked/unignored regular-file hashes and the full status before edits. Existing user changes and all previous evidence were preserved. No reset, stash, clean, commit or push occurred.

PY-3/PY-4 reports, PY-4 integrity records and raw diagnostics were inspected. `py4-source-review.json` confirms zero differences from PY-4's validated runtime source map before production changes; `py4-diagnostics-review.json` preserves all error lines and hashes for the 15 failed translation units. `locked-baseline.log` records a successful `bash tools/build_native_c.sh` using its existing release/locked recipe. This uses retained installed toolchains/caches and is not a clean-room reproducibility claim.

The CPython 3.13.0 archive SHA-256 is `086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`. Every regular archive member was compared with `/tmp/sunlight-py1-cpython/Python-3.13.0`; only inherited `configure` and `config.sub` platform recognition differ. `source-identity.json` retains hashes and matching `/tmp/sunlight-r1-cpython/build-host/python` 3.13.0 identity. No upstream edits or version changes occurred. The build interpreter is host tooling, not native Python evidence.

Final validation binds to `validated-source.json`, the incoming manifest plus the sole production delta, per-run commands, fixture hashes and guest manifests. `test-manifest.json` explains the before/after source distinction. Regular-file integrity excludes directory/symlink metadata and ignored generated artifacts. This limitation is explicit rather than an exception allowing prior evidence changes.

## SunShell capability matrix

The actual native shell is Rust `sunshell/src/main.rs`, selected by `tools/build.sh:75`, with noninteractive execution in `sunshell/src/native_script.rs`. `guest/sunshell/sunshell.pas` is not that selected build path. The host `std` backend is a separate implementation.

| Operation | Existing mechanism | Established limit |
|---|---|---|
| cd | `main.rs:1305` calls `normalize_path`, native stat and chdir; `native_script.rs:188` returns a normalized directory after native stat | Lexical cwd handling; no canonical component walk |
| pwd | Interactive `main.rs:1330` prints stored shell cwd; script runtime prints its context | No authoritative reverse lookup; interactive output uses a fixed buffer |
| ls | `main.rs:1341` normalizes arguments/injects cwd, launches `sunlight-utils` | `sunlight-utils/src/main.rs:375` uses native ReadDir; long mode prints fixed permission strings, not authoritative mode bits |
| existence/type | `main.rs:2781` stat helpers inspect File/Directory | Errors collapse to false; no symlink identity |
| readlink/realpath | No native builtins or native supporting API found | Host `host_script.rs:128` calls Linux canonicalize; not guest evidence |
| path completion | No filesystem completion resolver found in inspected native shell/input and TTY shell sources | Bounded negative source finding; completion UI not exercised |
| mounts | Absolute lexical path reaches VFS mount routing | `/tmp` and `/` script navigation tested; no canonical mount-crossing claim |

The trace is command → lexical `normalize_path` (`main.rs:2797`) → Rust libc syscall wrapper → kernel `KERNEL_VFS` → `Vfs::resolve_mount` → tmpfs/FAT. These native operations do not use filesystem-service IPC. Dot/dot-dot/repeated separators are removed before lookup; a nonexistent component can disappear lexically in `missing/../x`. Successful opening therefore does not prove realpath semantics.

Existing tests at `main.rs:3113` assert lexical relative-dot output. `tools/test_shell_phase2_host.py` uses the Linux host backend and cannot establish native link behavior. Native `sshl -c` probes here observe `/tmp`, dot/dot-dot/repeated separators, `/`, and a missing directory on 1/2/4 CPUs. They do not cover interactive PTY behavior, external ls, completion or native links. Full details: `shell-path-audit.json`.

## Sunlight Files capability matrix

The actual file manager is `sunlight-files/src/main.rs`, built through `tools/build_svg_apps.sh` (`tools/build.sh:102`). It combines a private fixed-capacity `PathBuf`, shared `libc::file_ops`, direct native metadata syscalls, directory snapshots and application-launch helpers.

| Operation | Existing mechanism | Established limit |
|---|---|---|
| navigation | `navigate_to:1322` → `load_directory:1355` → `file_ops::read_directory` → ReadDirFrom148 | Stores requested path after successful listing; no canonical result |
| breadcrumbs/parent | `PathBuf::parent:713`, breadcrumb hit-test/display at2345/2900 | String components, not VFS ancestry |
| path entry | `PathBuf::set:603` trims whitespace, roots relative text, removes dot/dot-dot/repeated separators | Private lexical convention; differs from shell cwd joining |
| properties/type | `main.rs:2707` native stat, directory-entry type, metadata-valid state | Native subset and cached observations; no separate link/target metadata |
| open file | `open_row:1256` → `sun_open::open_path` | “Resolver” selects MIME/application association, not canonical filesystem target |
| copy/move | `sunlight-libc/src/file_ops.rs:109,151,194` | File/directory only; depth64/node8192 limits, exclusive copies and no-replace rename; no link-preservation semantics |
| mounts | Volume entries and `/mnt` enumeration at4964 | VFS dispatch available; native UI crossing not observed here |
| errors/cache | Listing produces generic “Cannot read folder”; thumbnails compare before/after stat (`thumbnails.rs:609,631`) | No POSIX errno fidelity or atomic identity guarantee |

No native link representation, link badge, separate target metadata, canonical target output or cycle detection exists in this path. Native FT_FILE/FT_DIR do not advertise links. The explicit lexical thumbnail-path tests at `main.rs:5775` do not establish physical resolution.

`lexical-probe.py` compiles exact source slices from the shell normalizer and Files PathBuf on the host. It confirms both normalize `/tmp//./../tmp` to `/tmp`, but Files turns `relative` into `/relative` while the shell at `/tmp` produces `/tmp/relative`; Files trims surrounding whitespace while shell preserves it. This is host execution of actual lexical code, not a Files UI or native lookup test. **Click-driven Files comparison: NOT_RUN.** See `file-manager-path-audit.json`.

## Authoritative filesystem ownership and missing contracts

`sunlight-fs/src/vfs.rs:81` defines `FileStat`: type, size, uid, gid, mode and nlinks. FileType has only File and Directory. `path.rs:3` requires an absolute nonempty path without NUL, dot/dot-dot or trailing separators. It does not itself reject repeated separators. Mount dispatch selects the longest component-boundary prefix (`vfs.rs:1238`); it is not a component resolver.

Tmpfs (`tmpfs.rs:98`) uses exact full-path lookup. FAT's `Fat32::stat_path` **does** walk components (`sunlight-fat/src/fat.rs:295`), skips empty separators, and performs case-insensitive directory lookup (`fat.rs:270`). It returns cluster/size/type, not authoritative name spelling, symlink traversal or credential-checked ancestry. This existing backend lookup should be reused in a future resolver design; it cannot itself satisfy realpath across all mounts/backends.

Kernel `resolve_current_path:1417` prefixes cwd for open and some compatibility operations. Native StatPath61 and ReadDir60/148 do not use it. Native chdir checks directory type but does not implement ancestor Execute authorization. Native open checks final-entry read/write permission; ReadDir checks target-directory Read (`syscall.rs:9246`). No shared credential-aware ancestor search policy was found. Actor-based mutation policy and protected mount admission are separate existing restrictions and must remain intact.

`services/vfs_server/src/main.rs` maintains a **separate VFS namespace**, a read-only boot share and32-byte IPC paths. STAT returns a weaker reply; special `/etc/passwd`/`group` FSTAT re-stats a remembered path (`:690`). It is not a drop-in authoritative adapter for kernel fd identity. AUTHORIZE_EXEC consumes kernel grants; successful file navigation does not substitute for that capability authority. Native Rust std overlay `lstat`, `readlink`, `symlink` and `canonicalize` explicitly return unsupported (`tools/native-std/overlay/fs/sunlightos.rs:334–356`).

| Metadata | Availability | Qualification |
|---|---|---|
| type/size | Authoritative and exposed native subset | File/Directory only; FAT fd size comes from open-slot state |
| mode/uid/gid | Stored in tmpfs; weaker policy contract in FAT | FAT policy is not on-disk POSIX ownership/mode |
| nlinks | Weaker native convention | File1/directory2; unlinked dynamic tmpfs fd0; no hardlink contract |
| inode | Authoritative but internal in tmpfs | Seed index+1 survives materialization, dynamic monotonic IDs, implicit root0; no common backend/fd export |
| mount identity | Internal tag/source | No defined stable POSIX dev_t contract |
| timestamps | Missing from native shared/backend metadata contract | No authoritative tmpfs time-update policy |
| symlink identity | Missing | No storage/type/traversal |
| descriptor metadata | Native handle-backed subset | Generation checks exist; concurrent close pin/wrap semantics unqualified |

Linux compatibility `synthetic_ino`/`fill_linux_stat` (`syscall.rs:6453,6466`) hashes pathnames and supplies fixed/computed/zero fields. Those values are **not reused as native authoritative POSIX metadata**. FAT cluster numbers alone cannot identify all objects correctly, including empty files and rename lifetimes. A backend-owned identity/time policy and versioned ABI are required. Details: `vfs-capability-matrix.json`, `filesystem-metadata-matrix.json`.

## Reuse decision and scoped production repair

The pre-implementation `posix-adapter-design.md` compares direct VFS reuse, minimal owner extensions, a shared client and libc translation. Existing VFS/native metadata mechanisms are reused in tests. A client or libc wrapper cannot supply missing authority; full metadata and component resolution require a broader all-backend contract. Those portions are **BLOCKED**, with independent audit/regression work continued.

| POSIX API | Existing native mechanism | Missing requirement | PY-5 action |
|---|---|---|---|
| stat | StatPath61 → Vfs::stat | Full metadata, search permissions, link-follow policy | Retain native subset; no false POSIX success |
| lstat | Only File/Directory metadata | Link storage and final no-follow | BLOCKED |
| fstat | syscall48 → generation-qualified VFS/backend handle | Full fields, all-backend identity, concurrent fd pin qualification | Existing subset tested; POSIX BLOCKED |
| realpath | App lexical normalizers; handle-associated path | Existing-component canonical walk with credentials and links | BLOCKED; no lexical substitute |

The only production file modified is `sunlight-fs/src/vfs.rs`. Its `handle_path` condition treated every absolute backend-local path as already global. Since backend-local names start with `/`, `/boot/HELLO.TXT` incorrectly became `/HELLO.TXT`. `handle-path-before.log` preserves the failing regression. The repair lets the root mount return the local name, maps a non-root local `/` to the mount root, and prefixes other non-root local names. The comment now documents tmpfs current-entry versus FAT open-spelling lifetime and explicitly disclaims canonical/authorization semantics.

All91 filesystem unit tests passed after the repair, including root/mounted/mount-root paths, short buffer with no writes and stale handle rejection. A final-source native fixture exercises mounted `handle_path` through the existing getdents64 compatibility adapter, checks regular/directory/missing/invalid paths, and verifies native fstat after rename, after unlink with nlinks0, and after close with EBADF. It is a native C actor calling an existing compatibility entry; it is not a Linux-process ABI qualification or POSIX metadata claim. Persistent FAT execution and forced close/rename races remain NOT_RUN.

The fixture initially failed to compile because C rename/chdir are undeclared. Its original source/log are preserved. The corrected fixture calls existing native syscalls66/63; no declarations or fake functions were added. That correction does not implement those C APIs.

Realpath allocation, caller-buffer and errno scenarios remain **NOT_RUN** without implementation. No unknown-sized buffer safety claim or `realpath(path,NULL)` ownership claim is made. The repaired handle pathname does not validate every component, authorize access, preserve a name after unlink, or choose authoritative FAT case spelling.

## CPython build continuation

Fresh cross-build directory `/tmp/sunlight-py5-cpython/cross-static` uses the exact pinned source, matching build interpreter, native cc.py target driver, static flags and unchanged Setup.local/cache selections. `configure-manifest.json`, configured Makefile/pyconfig/config.log, genuine make-k log, exact extracted commands and independent unlimited-error diagnostics are retained. All180 original configured translation units, including unicodedata, remain in the denominator.

Result: **165 PASS, 15 FAIL, zero unattempted; zero newly successful**. `cpython-object-matrix.json` records every unit and its raw compiler log. The same failed units remain:

- Filesystem: `Modules/_io/fileio.o`, `Modules/_stat.o`, `Modules/getpath.o`, `Modules/main.o`, `Modules/posixmodule.o`, `Objects/obmalloc.o`, `Python/fileutils.o`, `Python/marshal.o`.
- Signals: `Modules/_threadmodule.o`, `Modules/faulthandler.o`, `Modules/signalmodule.o`.
- Calendar/time: `Modules/timemodule.o`, `Python/pytime.o`.
- Entropy: `Python/bootstrap_hash.o`.
- Environment: `Python/pylifecycle.o`.

`core-serial.log` independently reproduces the first genuine serial make blocker: undeclared realpath at mimalloc `alloc.c:880`. Other diagnostics remain independent requirements; fatal includes can hide later failures. No declaration-only readiness improvement is claimed. Static interpreter link and execution are **NOT_RUN** because mandatory objects do not compile; no omitted objects, dummy symbols or fabricated interpreter output were used.

## Security and PY-6 scope

The mount-prefix repair adds no locks, IPC, callbacks, recursion, retry loop, authority or ABI layout. It uses the existing VFS lock and tagged handle checks. The fixed256-byte backend buffer and caller length check remain; no partial output is copied on a short caller buffer. Mount pins, actor policy and capability consumption remain unchanged. No scheduler, signal, threading or entropy production changes occurred.

Existing limitations remain material: ancestor Execute policy is missing; fd lookup releases the scheduler before acquiring VFS and has no demonstrated in-flight descriptor pin; finite generations can wrap; service FSTAT can use a path round trip; FAT open spelling/size can become stale; directory paging is not a snapshot. Native concurrent-size tests do not force a precise overlap. Concurrent close, rename/mount interleavings, persistent FAT and non-root permission failure are unqualified or NOT_RUN. Canonical names would remain TOCTOU observations and must never authorize later operations by themselves. `security-review.json` retains these findings.

Recommended **PY-6: native filesystem contract foundation**: define one credential-aware component resolver that reuses backend lookup, preserves mount/actor/capability policy, validates existence/search access and returns authoritative spellings; specify backend inode/device identity and timestamp lifetime/update policy; introduce versioned metadata with explicit supported fields; define fd pinning across metadata queries. Native symlink representation, follow/no-follow and bounded-loop policy need explicit backend scope before exporting them. Then implement honest libc POSIX metadata/realpath and qualify shared fixtures through actual shell, Files clicks and C guests. Keep signals, entropy, environment and calendar blockers separately scoped.

Report: `docs/runtime/python-py5-vfs-posix-filesystem-integration.md`. Required JSONs, design, source manifests, patch, compiler logs, guest logs, unsupported cases and drivers are under E. No commit or push.

## Final gate and regression results

| Gate | Status | Evidence boundary |
|---|---|---|
| G1 | PASS | Pinned identity, unchanged PY4 inputs/diagnostics, locked baseline, incoming preservation |
| G2 | PASS | Source-backed shell and Files audit complete; runtime/UI limits explicit |
| G3 | PARTIAL | Native subset and owners traced; authoritative component resolver/full POSIX metadata absent |
| G4 | BLOCKED | Missing authoritative POSIX metadata and symlink contracts; native subset only |
| G5 | BLOCKED | No authoritative canonical permission-checked component resolution |
| G6 | PARTIAL | Native shell/C and host Files lexical comparison; Files UI shared-fixture comparison NOT_RUN |
| G7 | PASS | Complete identical180 configured units retried with independent diagnostics;165 compile |
| G8 | PASS | Final-source1/2/4 CPU regression matrix, filesystem host/native cases, ABI/vendor/archive and supported TCP |
| G9 | PARTIAL | Narrow delta reviewed and integrity verified; ancestor permission and descriptor/mutation security qualification remain open |

| CPUs | C | C++ | Lifecycle | PY-3 functional | PY-4 | PY-5 | TCP |
|---|---|---|---|---|---|---|---|
| 1 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 40/40 PASS | 5/5 PASS | 1/1 PASS | NOT_RUN |
| 2 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 40/40 PASS | 5/5 PASS | 1/1 PASS | PASS |
| 4 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 40/40 PASS | 5/5 PASS | 1/1 PASS | PASS |

Lifecycle17 is included in PY-3 functional40. All46 functional cases pass per CPU. Each functional run includes1780 formatted-I/O oracle cases, heap/ABI and TLS/once controls. Native SunShell path scripts separately pass on1/2/4 CPUs. TCP at1 CPU remains unsupported/NOT_RUN. Host VFS91/91 and FAT9/9 pass. Locked runtime rebuild, pthread ABI, C++ vendor provenance/archive ownership/ELF checks and libm comparison with retained pinned source pass. Final incoming-source integrity and production-patch round trip pass; no prior-evidence/source exceptions. Artifact hashes are retained in `artifact-hashes.json`.

**PY-5 remains PARTIAL**: G3/G6/G9 are PARTIAL, G4/G5 BLOCKED. No full POSIX metadata, realpath, Files UI comparison, interpreter link or interpreter execution is claimed.
