# PY-6: Unified filesystem foundation

2026-11-14. **Overall: PARTIAL. PY-6 is not COMPLETE.**

This implementation adds a permission-aware existing-object component resolver to the authoritative VFS, explicit metadata provenance and mount-lifetime Tmpfs object identity, native canonical-path queries, C `realpath`, and shared directory navigation for SunShell and Sunlight Files. It also repairs local handle-generation wrap and final-directory search authorization in `chdir`.

Full all-backend pinned lookup, native symlinks, POSIX stat/lstat/fstat and complete application convergence remain unfinished. Two host FAT lifetime failures are preserved: another descriptor retains stale size; a descriptor whose pathname was unlinked can write into the replacement object. **G10 FAIL** records these unresolved violations. Passing ordinary regressions does not qualify those contracts. Files UI comparison is **NOT_RUN**.

Evidence root (**E**): `docs/audit/probes/python/results-2026-11-14-py6/`. Final native runs are under `E/final/`; final CPython diagnostics are under `E/final-cpython/`. Earlier attempts at E's root are preserved and excluded from final-source regression totals.

## Acceptance gates

| Gate | Result | Evidence boundary |
|---|---|---|
| G1 | PASS | Baseline locked build, source/ownership inventory, design before production, preservation |
| G2 | PARTIAL | Explicit provenance and Tmpfs lifetime identities; FAT stale/pinning failures and missing fields; no native identity ABI |
| G3 | PARTIAL | Native authoritative query/chdir walk validated; directory-handle pinning, all syscall convergence and mutation schedules incomplete |
| G4 | NOT_RUN | Native symlink implementation and tests remain unfinished; FAT unsupported |
| G5 | PARTIAL | Real native realpath implemented and exercised; full stat/lstat/fstat and links unavailable |
| G6 | PARTIAL | Shell cd and Files navigation use shared API; Files real UI comparison NOT_RUN; other path helpers remain |
| G7 | PASS | Application consumers audited with explicit migration status |
| G8 | PASS | Pinned complete CPython compilation repeated; compilation itself165/180 FAIL |
| G9 | PASS | Final-source1/2/4 CPU existing and new native regressions, host VFS/FAT and ABI/vendor/archive checks |
| G10 | FAIL | Final integrity verified but FAT object-lifetime violations remain demonstrated; forced security schedules unqualified |

## Baseline, inventory and preservation

Incoming revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, branch `main`, dirty worktree. `source-baseline.json` records 83,740 tracked/unignored regular-file hashes and full incoming status. The original locked `bash tools/build_native_c.sh` passed in `locked-baseline.log`. This uses installed toolchains and caches; it is not clean-room reproducibility.

PY-4/PY-5 reports, PY-5 validation/evidence drivers, filesystem-boundary/storage audit, VFS grant-authority and native lifecycle documentation were read against current source. The earlier roadmap's heap-vector description is stale: current RamFs is an alias of page-backed Tmpfs. `filesystem-architecture.md` records the ownership inventory before production edits; `resolver-design.md` records the preimplementation design. Files/locale build dependency and final-search findings are recorded separately.

`source-integrity.json` compares every incoming regular file. Fourteen modified incoming files are saved under `incoming/`, with original hashes checked against this run's baseline. Dirty originals were reconstructed from preserved prior snapshots/patches or matching Git blobs, rather than treating HEAD as the incoming state. `py6-production.patch` applies to those originals and reverses byte-for-byte. One new production source file is added. Directory/symlink metadata and ignored build artifacts are outside this regular-file integrity claim. No reset, stash, clean, commit or push occurred.

## Authoritative architecture and ownership

```text
Applications
  -> sunlight-libc native Rust client / native C ABI
  -> SYSCALL
  -> kernel path/credential/capability/fd boundary
  -> KERNEL_VFS / Vfs mount table
  -> FsNode::Ram (RamFs == Tmpfs) or FsNode::Fat
  -> Tmpfs PMM inode payload / FatFs -> sunlight-fat -> block/cache/device
```

`services/vfs_server` owns a separate RamFs and bounded boot snapshot. Its IPC STAT/FSTAT paths are not the ordinary kernel file syscall authority. No new filesystem IPC transport or service was introduced. `sun_open` chooses a program/MIME association and remains distinct from filesystem lookup.

| Operation | Current owner and authoritative boundary |
|---|---|
| open/close | Kernel fd/rights layer owns descriptions; Vfs packs mount/local generation handles; backend owns open object state |
| read/write/seek | Backend offset-based I/O; kernel fd descriptions own offsets and seek state |
| create/remove/rename | Existing actor/capability mutation policy plus Vfs/backend operations; no-replace publication retained |
| enumeration | Path-based native ReadDir/ReadDirFrom snapshots; no new pinned iterator or directory-fd API |
| metadata | Existing24-byte native Stat ABI plus new internal Rust metadata/provenance API |
| mounts | Existing global Vfs longest component-prefix routing, protected/runtime admission and mount incarnation tags |
| lexical normalization | Application helpers remain for some operations; they are not authorization checks |
| canonical lookup | New Vfs existing-object walker reached through syscall179; migrated query/navigation only |
| cwd | Kernel shared-process string plus shell/script context; not a retained directory object |
| notifications | No filesystem change-notification contract found in the audited Vfs/FileSystem/native client interfaces |

Key definitions: `sunlight-fs/src/vfs.rs:124`, `sunlight-fs/src/vfs.rs:791`, `sunlight-fs/src/tmpfs.rs:30`, `kernel/src/process/fd_table.rs`. No independent client or parallel filesystem abstraction was added.

## Metadata capabilities and identities

New definitions live in `sunlight-fs/src/vfs/lookup.rs:10`. `Stored`, `Policy`, `Unavailable` and `Unsupported` are distinct. `Metadata.native` retains the existing subset and explicitly requires consulting its provenance/capabilities.

| Field | Tmpfs/RamFs | FAT |
|---|---|---|
| type/size | Stored type/data length | Path lookup type/size; per-open cached size has demonstrated divergence |
| object identity | Backend inode identity within this mount lifetime | None; cluster/handle/path never substituted |
| mount identity | Existing mount incarnation tag+1, scoped to this VFS instance | Same scope; not a permanent POSIX device number |
| ownership/mode | Stored seed/dynamic fields; implicit-root defaults marked Policy | Existing mount/user-storage policy, not on-disk POSIX permissions |
| link count | Existing native convention, not full POSIX directory accounting | Unsupported POSIX count |
| atime/mtime/ctime/birth | Unavailable; no invented time values | Not exposed by current shared contract; no invented values |
| symlink/target | Unsupported by current implementation | Unsupported; no custom file encoding |
| pinned descriptor metadata | Tmpfs retained object survives rename/unlink | Unavailable full guarantee; negative probes FAIL |

Tmpfs seed identity is index+1, implicit root identity0, and dynamic objects use existing monotonic IDs. Materialization preserves the existing ID; identity is not allocated from a handle number. The `(mount_id, object_id)` pair is an observation of a backend lifetime, not an on-disk inode promise or an authorization token. The new API is internal Rust; no extended metadata wire ABI or POSIX struct stat was invented.

`metadata_handle` obtains the backend's retained handle object, not a second pathname lookup. Host tests verify Tmpfs rename/unlink/name replacement/close and mount qualification. Existing native fd subset fixtures cover rename/unlink/close on final1/2/4 CPU guests. Forced close/duplication/rename interleavings and guest-visible extended object IDs remain UNQUALIFIED/NOT_RUN.

`generation-before.log` reproduces a stale local capability becoming valid again at wrap. Both Tmpfs and FatFs now retire exhausted slots, matching the existing fail-closed mount-incarnation approach. Host exhaustion tests cover both backends. Each slot has at most65,535 incarnations; eventual slot exhaustion reduces available open capacity and returns TooManyOpenFiles rather than aliasing a stale handle. No generation capacity was inflated. Native internal-token exhaustion was not injected.

## Resolver and native API

The walker is `sunlight-fs/src/vfs/lookup.rs:98`; kernel boundary is `kernel/src/arch/x86_64/syscall.rs:5710` and `kernel/src/arch/x86_64/syscall.rs:5764`.

The walk validates every existing component before later `..` can discard it. It handles absolute/relative paths, cwd, `.`, `..`, repeated separators, trailing-directory checks and per-component mount routing. Each traversed parent must be a directory and pass existing Execute uid/gid/root policy. Whitespace is preserved. Input/path payload is bounded to255 bytes; output includes NUL. FAT case spelling is obtained from backend enumeration; Tmpfs stays exact-name sensitive.

The kernel's callback checks existing private-secret/KV capability restrictions before component metadata access, including a component later removed by `..`. Caller authority/cwd are captured before KERNEL_VFS acquisition. The walker performs no scheduler acquisition or service IPC under that VFS borrow. It uses iterative components and fallible bounded path allocation, without recursive link traversal or new polling.

Syscall179 ABI: rdi=NUL-terminated input, rsi=output, rdx=capacity including NUL, r10 bit0=must be a searchable directory; other flag bits are invalid. Success returns payload length. Insufficient output capacity writes no partial path. `sunlight_libc::resolve_path` and `resolve_directory_path` share this boundary. Navigation uses the latter; kernel chdir independently checks final-directory type/search while still under VFS serialization.

`chdir-search-before.log` records an actual uid1000 guest entering the existing mode0000 directory before the repair. Final native tests reject entry and reject traversal through that directory before `..` removal.

Limits remain significant: cwd is textual, lookup does not retain a directory object, no final-follow/no-follow policy exists without link objects, and existing open/stat/readdir/exec still have separate normalization/authorization paths. Canonicalization grants no access to a later operation. FAT long-name enumeration has the existing64-byte VFS entry bound; nested mounts under case-insensitive ancestor spellings are not qualified. Mount/permission/rename races and lock dwell under adversarial FAT paths were not forced or measured. G3 therefore remains PARTIAL.

## Symbolic links and scope boundary

Native symlink creation, readlink, final no-follow and traversal/loop handling are **not implemented**. All requested link scenario tests are **NOT_RUN**, not passing unsupported-backend tests. RamFs's dynamic inode/quota model could own a real target, but the current FileType and operation consumers need coordinated migration. It would be unsafe to enable traversal while alternate operations still authorize different path spellings and descriptor-backed directory ownership is incomplete. FAT remains unsupported without an invented encoding.

The full descriptor-backed portion stops at the demonstrated FAT ownership dependency. FatOpen stores a cluster/size snapshot and a pathname; writes/truncate re-resolve that pathname, and unlink frees/reuses backend storage. A common retained inode/open-description and directory-object lifetime is needed before claiming all-backend pinned fstat/lookup, including rename replacement and mount/directory lifetime integration. No broad process/fd/scheduler ownership redesign was attempted. Independent canonical-query, capability reporting, regression and CPython work continued. G4 remains NOT_RUN; the milestone is explicitly incomplete.

## POSIX adapter results

Native C `realpath` is implemented in `sunlight-libc/src/c_file.rs:126`, declared in stdlib.h and backed by the component walker. It does not call a lexical normalizer. NULL output allocates exactly the canonical result plus NUL with libc malloc; caller uses free. Caller-owned output must supply PATH_MAX256 bytes. C cannot discover the allocation capacity, so this caller responsibility remains documented; the implementation copies at most256 bytes after a successful full lookup.

Final guest coverage includes cwd, dot/dot-dot, repeated separators, missing intermediate components, file-as-directory, whitespace in actual filenames, allocation, buffer canaries and relevant errno. The isolated native guest has one root Tmpfs; /tmp is a directory, not a separate mount. Native mount crossing is NOT_RUN. The fixture stdout label includes “mount”, but that label is not evidence of mount traversal; mounted FAT/name and mount-identity checks are host-only. Six C/C++ base/POSIX/GNU header-profile checks pass, with one native archive definition. Complete invalid-pointer/error-fault injection is not claimed.

**POSIX stat/lstat/fstat remain BLOCKED.** The original24-byte native descriptor metadata is retained unchanged. Mandatory trustworthy fields and native link semantics are missing; full POSIX headers/symbols were not filled with fake data. Readlink/symlink C APIs remain unimplemented. Rust native std canonicalize/lstat/readlink/symlink remain unsupported; this is a remaining convergence item.

## Application integration

SunShell interactive cmd_cd and native-script Runtime.directory now call shared directory resolution before updating canonical cwd. Parser syntax/quoting is unchanged. The final noninteractive native shell guests compare canonical pwd and rejection of missing/../ paths; interactive PTY, completion, comprehensive whitespace and all shell helpers are not qualified. ls/external argument and existence paths retain lexical helpers. G6 does not claim complete shell convergence.

Files PathBuf now stores the input without stripping whitespace or dot components. navigate_to joins relative input against the current Files directory, queries shared searchable-directory resolution, then stores canonical output for successful navigation. The actual native-svg Files build passes. Exact-source host storage checks pass; `cargo test -p sunlight-files ...` selected zero test targets and is **NOT_RUN**. Click-driven Files UI, properties/identity comparison, breadcrumb behavior, mount crossing and link presentation are **NOT_RUN**. Existing QMP-based specialized desktop gates were discovered, but no Files workflow was exercised. UI behavior is not inferred from its build.

Other consumers were inventoried in `application-migration-inventory.json`: Edit, Light Lens and Melody Mina already use native libc I/O but are not fully migrated to canonical semantics; Docview/Docserv cross the native std boundary; common dialogd retains private PathBuf and save-name trimming; sunlight-fetch still anchors relative output to `/root` and has a copy-based native rename helper. Terminal/PTY protocol was left unchanged. These workflows were not refactored. The shared file_ops transfer normalizer also remains lexical.

The Files build initially exposed an inherited feature mismatch: native-std wide formatting references locale while the module was gated to native-c. Enabling that existing module for native-std (already implied by native-c) repairs the actual configured application build. Original failure, retry and final build logs are preserved. No locale data or metadata was fabricated.

## CPython3.13.0 repeat

Archive SHA256 `086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`, retained upstream source and matching3.13.0 host build interpreter are recorded in source-identity.json. Only inherited configure/config.sub platform-recognition patches differ from the archive. The real native cc.py/sysroot, static cross-configure and original configured object set were used. Tool/configuration hashes are in toolchain-identity.json.

**Configured180; attempted180; compiled165; failed15; unattempted0.** Object set unchanged. The undeclared-realpath diagnostic in mimalloc is resolved, but Objects/obmalloc.o reaches missing abort and sys/syscall.h. Thus the object success count stays165. The first serial blocker is captured in final-cpython/core-serial.log. No fake abort/syscall interface was added.

| Remaining object | First diagnostic |
|---|---|
| `Modules/_io/fileio.o` | variable has incomplete type 'struct stat' |
| `Modules/_stat.o` | use of undeclared identifier 'S_IFDIR' |
| `Modules/_threadmodule.o` | use of undeclared identifier 'SIGINT' |
| `Modules/faulthandler.o` | use of undeclared identifier 'SIGFPE' |
| `Modules/getpath.o` | variable has incomplete type 'struct stat' |
| `Modules/main.o` | variable has incomplete type 'struct stat' |
| `Modules/posixmodule.o` | incomplete definition of type 'struct stat' |
| `Modules/signalmodule.o` | use of undeclared identifier 'SIGHUP' |
| `Modules/timemodule.o` | incomplete definition of type 'struct tm' |
| `Objects/obmalloc.o` | call to undeclared function 'abort'; ISO C99 and later do not support implicit function declarations [-Wimplicit-function-declaration] |
| `Python/bootstrap_hash.o` | use of undeclared identifier 'GRND_NONBLOCK' |
| `Python/fileutils.o` | conflicting types for '_Py_fstat_noraise' |
| `Python/marshal.o` | variable has incomplete type 'struct stat' |
| `Python/pylifecycle.o` | call to undeclared function 'setenv'; ISO C99 and later do not support implicit function declarations [-Wimplicit-function-declaration] |
| `Python/pytime.o` | incomplete definition of type 'struct timeval' |

`cpython-object-matrix.json` contains exact commands, complete diagnostics and ownership classification. Full static interpreter link and native interpreter execution are **NOT_RUN** because mandatory objects still fail. No objects were omitted from the denominator.

## Final-source regression matrix

| CPUs | C | C++ | Lifecycle | Existing functional | New filesystem | TCP | Native shell |
|---|---|---|---|---|---|---|---|
| 1 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 46/46 PASS | 1/1 PASS | NOT_RUN | PASS |
| 2 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 46/46 PASS | 1/1 PASS | PASS | PASS |
| 4 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 46/46 PASS | 1/1 PASS | PASS | PASS |

Lifecycle17 is included in the existing46 functional cases. New total47 includes one PY6 filesystem case. Each supported CPU checks1,780 formatted-I/O oracle cases plus heap/ABI, TLS/once and existing language/lifecycle behavior. VFS97/97 and FAT9/9 host tests pass. Final locked build, C/C++ header/archive and vendor checks have separate logs. TCP1 CPU and12-CPU guests are NOT_RUN/unsupported; they are not counted as passes.

Every final execution records its command and before/after production-source match against final/validated-source.json. Guest directories retain serial/stdout/stderr, kernel/image/manifests, hashes and cleanup receipts. First guest, generation failure, FAT negatives, malformed combined fixture, interrupted initial legacy run and Files build failure remain preserved at E's root. The corrected combined run uses the actual uid1000 denied-directory fixture, replacing an initial incorrect expectation for absent /etc/sunlight.

## Security, remaining work and recommended PY-7

Security review is FAIL overall because the FAT lifetime violations remain real. Scoped new checks preserve existing capability/mutation policy, use bounded path memory, retire exhausted capabilities and avoid a new IPC/lock cycle. Functional scheduler progress is covered by the regression matrix; deterministic races, permission-change overlap, lock latency and filesystem service migration are not qualified by that result.

Recommended PY-7: retained filesystem object and directory-handle ownership, beginning with FAT open/unlink/rename-replacement and shared metadata coherence. Add a versioned validity/provenance-aware metadata ABI and genuine backend time policy, then quota-owned Tmpfs symlinks with one bounded VFS traversal algorithm and coordinated syscall/client migration. Complete shared transfer/picker/native std semantics and click-driven Files/SunShell/libc identity comparisons. Close the unfinished PY-6 gates before claiming completion, then repeat the same CPython set and link only after mandatory compilation passes.

## Modified production files and artifacts

* `sunlight-libc/src/c_file.rs`
* `ipc/src/lib.rs`
* `kernel/src/arch/x86_64/syscall.rs`
* `sunlight-files/src/main.rs`
* `sunlight-fs/src/tmpfs.rs`
* `sunlight-fs/src/vfs.rs`
* `sunlight-fs/src/vfs/storage_tests.rs`
* `sunlight-libc/include/errno.h`
* `sunlight-libc/include/limits.h`
* `sunlight-libc/include/stdlib.h`
* `sunlight-libc/src/lib.rs`
* `sunlight-libc/src/sys.rs`
* `sunshell/src/main.rs`
* `sunshell/src/native_script.rs`
* `sunlight-fs/src/vfs/lookup.rs`

Main report: `docs/runtime/python-py6-unified-filesystem-foundation.md`. All requested root JSON/Markdown deliverables, raw logs, source hashes, incoming snapshots, patch and validation/assessment drivers are under E. `validation-summary.json` is the machine-readable verdict; `source-integrity.json`, `cpu-matrix.json` and `security-review.json` preserve distinct evidence boundaries. No commit or push.
