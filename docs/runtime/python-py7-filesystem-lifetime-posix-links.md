# PY-7: Filesystem lifetime, POSIX metadata and native links

2026-11-14. **Overall: PARTIAL. PY-7 is not COMPLETE.**

The reproduced FAT replacement-write defect is blocked by explicit backend-entry authentication and pre-removal handle revocation in the exercised host/native cases. This is a fail-closed approximation, **not retained POSIX open-unlink access**. No complete retained directory-relative lookup, POSIX stat ABI or native symlink foundation is claimed. Required concurrency and storage-reclamation witnesses remain incomplete.

Evidence root (**E**): `docs/audit/probes/python/results-2026-11-14-py7/`. Every required root JSON deliverable is present. Final regression executions and their source map are under `E/final/`; complete CPython diagnostics are under `E/final-cpython/`. No commit, push, checkout, reset, stash or clean was performed.

## Acceptance gates

| Gate | Status | Evidence boundary |
|---|---|---|
| G1 | PASS | Incoming revision/status, locked baseline, PY5/PY6 source/evidence review and byte-preserving snapshots |
| G2 | PARTIAL | Known unlink/replacement read-write defect blocked in host/native exercised cases; required forced schedules and native exhaustion incomplete |
| G3 | PARTIAL | Revoked-slot close and mount accounting verified; retained backing and final deferred reclamation absent; failure cleanup fail-stop only |
| G4 | PARTIAL | Native open/dup/rename/fstat/close directory subset; pinned lookup and enumeration incomplete |
| G5 | BLOCKED | Full authoritative POSIX metadata ABI/time/link semantics missing |
| G6 | BLOCKED | Directory safety dependency incomplete; native symlink/readlink not implemented |
| G7 | PARTIAL | Existing shared navigation and consumer audit/builds; complete common-fixture and interactive UI convergence not validated |
| G8 | PASS | All original 180 configured CPython objects retried with diagnostics; 165 compile |
| G9 | PASS | Final-source native 1/2/4 CPU and host regressions; unsupported cells excluded |
| G10 | PARTIAL | Integrity and scoped review complete; forced concurrency, raw FAT failure recovery and directory-relative safety remain incomplete |

## Baseline and evidence integrity

Incoming revision `f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`, branch `main`, with extensive existing modifications and untracked PY-1 through PY-6 evidence. PY-5/PY-6 reports, PY-6 acceptance JSON, negative FAT fixtures and diagnostics, native guest runner and final-validation/CPython drivers were read before implementation. The current source, including fd descriptions and raw FAT mutation ordering, was inspected directly.

`source-baseline.json` records the full incoming status and hashes of 86,319 tracked/unignored regular files. The locked incoming `bash tools/build_native_c.sh` passed. `fat-unlink-before.log` freshly reproduces the defect against that baseline: writing `BAD` through the unlinked handle changes the replacement from `new` to `BAD`.

The five changed production/test-runner files have byte-preserved incoming copies under `incoming/`. `py7-production.patch` is relative to those incoming bytes, including existing user changes, rather than HEAD. `source-integrity.json` verifies the incoming manifests, reports every changed/missing incoming regular file and verifies patch application/reversal byte-for-byte. Previous evidence is included in the unchanged-file comparison. Directory/symlink metadata and ignored generated artifacts are outside that integrity claim.

Final tests record commands and source matches before/after execution. Native runs preserve private kernel/image/manifests, raw serial/output, resource reaping and original/final disk copies. Development logs are separate. The initial fixture failed compilation because `dup` is undeclared; its source/log remain preserved. The corrected fixture invokes existing syscall45. Early development runs without complete per-attempt source manifests are explicitly UNQUALIFIED for exact-source acceptance and do not qualify final source.

## FAT ownership audit and selected design

The authority remains application → libc/native syscall → kernel fd/rights boundary → KERNEL_VFS → FatFs → Fat32 → block/cache/device. The separate filesystem service is not substituted for the kernel VFS.

`kernel/src/process/fd_table.rs:289` owns shared offset/status/refcount descriptions. Duplication retains a description; `take` removes the fd before backend cleanup and marks only the last VFS owner for release. VFS owns packed mount/local-generation handles and mount open counts. The raw FAT backend has directory entries and cluster chains, without retained open objects. Previously FatOpen cached first-cluster/size plus a pathname: reads used the cache, writes/truncate re-resolved the name, and unlink freed backing immediately.

`fat-design.json` evaluates deferred deletion, shared pinned objects, explicit revocation and existing lifetime mechanisms. The selected increment reuses mount incarnation/local-generation retirement and adds the raw short-entry location `(LBA, offset)` as an **internal authentication key**. It is not an inode or generation-qualified backend lifetime identity by itself. Explicit pre-removal revocation prevents reuse of the same location from resurrecting an old handle.

Implementation:

- `sunlight-fat/src/fat/write.rs:390` exposes entry location under exclusive volume ownership; it exposes no POSIX identity.
- `sunlight-fs/src/vfs.rs:275` authenticates the location and refreshes cluster/size on read, write, truncate and fstat. Another descriptor therefore observes successful extension/truncate without stale cluster access.
- Unlink revokes all matching open aliases **before** the first backend mutation. Read/write/truncate/fstat through those handles fail with BadHandle. New opens are separate generations and cannot reactivate revoked slots.
- Rename revokes the destination object’s handles, then updates every live source handle by source entry identity, including case/separator aliases. Alias-to-self rename is a no-op, avoiding the raw driver’s delete-self behavior.
- Directory rename conservatively revokes every other existing FAT handle. This sacrifices continuity and is an explicit remaining limitation, not complete retained-directory semantics.
- Close checks slot generation without requiring a live namespace entry. Revoked/poisoned handles close exactly once and keep mount open counts until close; duplication still uses the existing description refcount.

Raw FAT mutations can fail after publication, deletion or partial chain changes. Any mutating backend error therefore poisons the mounted adapter. Further backend opens/data operations fail; valid closes remain possible. Existing unmount barriers keep the poisoned mount/cache installed on error, even after handles close. No recovery, crash consistency, leaked-chain repair or exactly-once backing-reclamation guarantee is asserted. External disk mutation or parallel raw access is outside exclusive adapter ownership.

## FAT witnesses and lifetime boundary

| Witness | Status | Qualification |
|---|---|---|
| FAT-1 | PASS | Host aliases and native rejected old reads; replacement bytes checked |
| FAT-2 | PASS | Host and native rejected writes; replacement bytes checked |
| FAT-3 | PASS | Regular-file source rename retains handle I/O; directory subset separately qualified |
| FAT-4 | PARTIAL | Destination handle revoked, source handle reads own bytes after replacement; guest extended object identity unavailable |
| FAT-5 | NOT_RUN | No forced unlink/read-write overlap |
| FAT-6 | NOT_RUN | No forced concurrent close/unlink |
| FAT-7 | PARTIAL | Host name/slot reuse plus inherited local-generation exhaustion test; native internal generation exhaustion not injected |
| FAT-8 | PASS | Native duplicated descriptor rejects stale write and closes correctly |
| FAT-9 | PARTIAL | Host zero open-count and clean unmount; native zero leaked fds/frames. No deferred storage model or cluster accounting at final close |
| FAT-10 | PARTIAL | Host flush failure after deletion forces mount fail-stop and preserves valid close; arbitrary native I/O interruption NOT_RUN |

Final native witnesses use an explicitly supplied disposable FAT image, copied privately by the opt-in `guest.py --fat-fixture` runner and attached through virtio-blk. The existing explicit legacy-state test setting mounts it at `/state`; syscall policy and uid1000 remain active. No ordinary developer disk or existing VM is attached. Root Tmpfs and actual FAT coexist in this guest.

Replacement contents are checked after rejected stale reads/writes, including duplicated descriptors and rename destination replacement. Source descriptors continue I/O after ordinary file rename. Host tests additionally cover case/separator aliases, refreshed size, truncate followed by another file’s cluster allocation, duplicate-close rejection, open-count zero/clean unmount and fail-stop after an injected unlink flush error.

The separate final-source `fat-accounting.rs` host probe decodes the flushed raw FAT image: directory slot byte1056 and cluster4 are reused exactly. Stale read/write/truncate return BadHandle and leave the entire raw disk byte-for-byte unchanged. Allocated clusters move2 →3 →2 across create/write/unlink; final close remains2. This proves immediate release for the successful exercised case, not deferred final-close reclamation or failure recovery.

The native tests establish real syscalls and backend behavior, not native internal entry-key/generation telemetry or final cluster accounting. They do not force FAT-5/FAT-6 interleavings. No deferred backing exists: successful unlink reclaims immediately after revocation. Consequently neither G2’s full requested matrix nor G3’s retained/reclamation contract is PASS.

## Directory handles and resolver convergence

`directory-test.c` checks the real 24-byte native fstat layout/alignment, directory open, dup, rename, fstat on both descriptors, valid close and EBADF after close. Results are in `directory-handle-lifetime.json`; these are directory descriptor subset tests, not pinned enumeration/lookup tests.

The authoritative component walker remains `sunlight-fs/src/vfs/lookup.rs`. Existing syscall paths are not fully converged: `linux_resolve_at` at `kernel/src/arch/x86_64/syscall.rs:6574` checks fstat, separately reconstructs handle_path, joins text and later opens it. `getdents64` similarly enumerates a derived path. Namespace changes can occur between those operations. TmpfsDir is a pathname index, not a retained directory-relative resolver. No second resolver or broad syscall rewrite was introduced. This dependency keeps broad link/application work blocked.

## POSIX metadata and native symlinks

The existing native ABI remains 24 bytes: u64 size at0; u32 uid/gid at8/12; u16 mode at16; u8 type at18; reserved byte19; u32 nlinks at20. `directory-test.c` asserts size24/alignment8/type offset18/link offset20. This is **not struct stat**. The scoped syscall change makes fstat preserve backend errors through the existing native/Linux mappers; revoked FAT handles return native -5/EBADF.

`metadata-abi.json` distinguishes stored Tmpfs metadata, FAT mount-policy ownership/mode, internal mount identity, unavailable shared inode/device identity, unsupported FAT POSIX link count and unavailable timestamps/nanoseconds. The raw entry key is not exported as st_ino. FAT fstat uses authenticated relookup, not an independently retained backing object. Full stat/lstat/fstat remain BLOCKED; `posix-stat-probe.log` reproduces missing sys/stat.h. No timestamps, permissions, devices or inode values were invented.

A remaining policy issue is visible in source: FAT user_metadata derives ownership from path spelling, although lookup is case-insensitive and tolerates repeated separators. Alias-consistent ownership requires authoritative policy resolution. This is recorded as a source-level security concern; this milestone does not claim a verified confidentiality exploit or complete permission closure.

Native symlink/readlink representation and traversal remain unimplemented. Tmpfs could own a quota-accounted target but needs coordinated FileType/resolver/syscall handling and retained directory semantics. FAT has no approved representation. All requested direct/relative/absolute/intermediate/dangling/chain/loop/directory/mount/race/buffer/stat-versus-lstat link witnesses are NOT_RUN. No lexical substitute, recursive resolver or FAT encoding was added.

## Applications and native std

No application/native std source changes occur in PY-7. Existing SunShell cd and Files navigation use shared directory queries; other shell argument helpers, file-picker PathBuf/save helpers and download output helpers remain private/lexical. Native std readlink/symlink/lstat/canonicalize remain unsupported, and ReadDir is uninhabited. Editor/media/document consumers still use their established libc/native std paths.

Final native Files and SunShell builds and noninteractive shell cases are separately recorded. Click-driven Files, interactive shell, file-picker, editor/media/download/archive UI and common cross-application identity/link fixtures are NOT_RUN. Builds do not qualify interactive behavior. `application-convergence.json` preserves those boundaries.

## CPython 3.13.0

**Configured 180; attempted 180; compiled 165; failed 15; unattempted 0.** The configured object set is unchanged. Newly compiled objects: 0. No improvement over the PY-6 165/180 count is claimed.

The original archive hash remains `086de5882e3cb310d4dca48457522e2e48018ecd43da9cdf827f6a0759efb07d`. Only inherited configure/config.sub platform recognition differs from the pinned archive. The matching host build Python remains3.13.0. Fresh configure, generated configuration, unchanged configured object set, actual compiler commands and complete diagnostics are retained. `cpython-source-integrity.json` verifies the pinned sources again after compilation; `cpython-toolchain-identity.json` records unchanged libc/header/driver inputs and current compiler identity.

The diagnostics again require real filesystem metadata, signals, calendar/time, entropy and environment APIs. `Objects/obmalloc.o` reaches undeclared abort and mimalloc’s generic Unix sys/syscall.h include. A declaration-only abort, exit134 replacement or arbitrary Linux syscall header would not establish abort/signal/runtime correctness. None was added. Upstream CPython was not modified to bypass APIs. Static interpreter linking and bounded native interpreter initialization are NOT_RUN because15 mandatory objects still fail.

## Final-source regressions

| CPUs | C | C++ | Lifecycle | Existing functional | PY6 | PY7 FAT | Directory subset | TCP |
|---|---|---|---|---|---|---|---|---|
| 1 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 46/46 PASS | 1/1 PASS | 1/1 PASS | 1/1 PASS | NOT_RUN |
| 2 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 46/46 PASS | 1/1 PASS | 1/1 PASS | 1/1 PASS | PASS |
| 4 | 73/73 PASS | 21/21 PASS | 17/17 PASS | 46/46 PASS | 1/1 PASS | 1/1 PASS | 1/1 PASS | PASS |

VFS host tests: **101/101**; FAT host tests: **9/9**. Each supported functional CPU run includes the existing1780 formatted-I/O oracle cases, heap/ABI and TLS/once checks. Lifecycle17 is included in existing functional46. PY6 adds one case for47 total; PY7 FAT and directory subsets run separately. C/C++ header/archive/provenance checks, Files/SunShell builds and supported TCP are recorded in the execution manifest. Libm is freshly compared byte-for-byte with the incoming pinned vendor source; this is not a network provenance refresh.

TCP at1 CPU and12-CPU guests are unsupported/NOT_RUN. No unsupported cell is counted as PASS. `cpu-matrix.json` and raw guest logs are the authority for actual final results, including any failures; no earlier source run substitutes for them.

## Security findings, blockers and PY-8

No new lock, scheduler path, IPC dependency, unbounded wait, permission bypass or handle-generation capacity increase was introduced. Authentication/revocation occurs under the existing exclusive VFS borrow. Native reaping/open-count checks support resource ownership for the tested cases; they do not measure arbitrary VFS lock latency or prove every concurrent schedule.

Remaining blockers are retained directory-relative resolution/enumeration, FAT partial-failure recovery and backing accounting, deterministic unlink/read-write and close/unlink schedules, alias-consistent FAT policy, full metadata/time authority, native symlink representation/traversal and actual UI/common-fixture convergence. G10 remains PARTIAL.

Recommended PY-8: **retained directory capabilities and transactional FAT retirement**. First establish independent directory/object ownership with deterministic namespace/close/mount schedules, explicit failure recovery or durable fail-stop ownership, and measured final storage reclamation. Then add a validity-aware metadata ABI with real time policy, followed by Tmpfs links through the one authoritative resolver. Re-run the unchanged CPython object set and link only when mandatory objects compile.

## Modified files

- `sunlight-fat/src/fat/write.rs`
- `sunlight-fs/src/vfs.rs`
- `sunlight-fs/src/vfs/storage_tests.rs`
- `kernel/src/arch/x86_64/syscall.rs`
- `tools/native-c/guest.py`

Main report: `docs/runtime/python-py7-filesystem-lifetime-posix-links.md`. Evidence: `E/validation-summary.json`, `fat-object-lifetime.json`, `fat-stale-descriptor-witnesses.json`, `directory-handle-lifetime.json`, `metadata-abi.json`, `posix-stat-validation.json`, `symlink-contracts.json`, `symlink-witnesses.json`, `application-convergence.json`, `cpython-object-matrix.json`, `static-link-results.json`, `cpu-matrix.json`, `security-review.json`, `modified-production-files.json`, plus raw logs, drivers, snapshots and integrity records. No commit or push.
