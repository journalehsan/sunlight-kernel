# SunFS SFS-2 persistent transactions and recovery

2026-11-14. Hosted qualification only. The authoritative final verdict and ten gates
are in [validation-summary.json](../../audit/sunfs/sfs-2/validation-summary.json).
**SFS-2 COMPLETE: G1–G10 PASS for the bounded hosted scope.**
No native durability or production integration follows from these results.

## Baseline and format compatibility

Incoming revision `1affc823311cb6014e2aecf63b848b2bd4f2af6d`, branch `main`.
The immutable [baseline](../../audit/sunfs/sfs-2/source-baseline.json) records
88,743 regular files and the incoming worktree status. The existing uncommitted
SunFS crates, workspace/lockfile changes, architecture document, and SFS-0/SFS-1
outputs were present before this work. Core bare-metal compilation and all 24
SFS-1 hosted tests passed before production changes. Separate raw baseline logs
retain the original 112 formatter crash schedules, 480 faults and 4,096 malformed
inputs; the SFS-1 evidence directory is never a destination for SFS-2 tests.

The architecture, on-disk format, recovery contract, implementation roadmap,
ADRs 0001/0002, geometry calculator, explicit codecs, bootstrap planner, independent
checker and simulator were inspected before implementing the engine. The
[pre-implementation decision](../../audit/sunfs/sfs-2/raw/format-decision-before-implementation.json)
and [design snapshot](../../audit/sunfs/sfs-2/raw/design-before-implementation.md)
are retained. **No disk layout, feature bit, version, codec, geometry formula, or
golden fixture changed.** Existing v0.2 bootstrap images mount directly.

SFS-1 already encodes all required objects. Its configured metadata estimate covers
empty content roots, so it cannot alone admit populated extent/directory snapshots.
SFS-2 counts and encodes the complete actual plan, including the summary fixed point,
and additionally enforces the existing superblock transaction bound. A configured
extent cap below the engine cap is also enforced before accepting writes.

## Engine and API

`sunlight-sunfs/src/engine.rs` and `engine/recovery.rs` implement the reusable
`no_std + alloc` engine. No normal dependency was added. Core code has no host file,
thread, clock or OS integration. The API is:

| API | Contract |
|---|---|
| `Filesystem::mount(device)` | Read-only full inspection first; construct a writable or degraded read-only owner |
| `state()`, `generation()` | Explicit mount disposition and selected generation |
| `entries()`, `lookup(name)` | Working root namespace and mount-scoped file handles |
| `create(name)` | Volatile regular file creation in root; explicit duplicate/name/cap errors |
| `write(handle, offset, bytes)` | Entire bounded byte range accepted, or explicit error; zero-filled holes/tails |
| `read(handle, offset, output)` | Working bytes, exact size and short EOF reads |
| `admission()` | Read-only exact plan preview; fsync repeats admission under the exclusive owner |
| `fsync(handle)` | Commit the entire dirty volume epoch and its root binding |
| `accounting()` | Protected union, placement overhead, available segment capacity and separate recovery reserve |
| `into_device()` / drop | Release mount without implied synchronization |

Example for an already formatted exclusively owned simulator:

```rust,ignore
let mut fs = Filesystem::mount(simulator)?;
let file = fs.create(b"hello.txt")?;
fs.write(&file, 0, b"Hello SunlightOS\n")?;
fs.fsync(&file)?;
let mut simulator = fs.into_device();
simulator.power_loss();
simulator.reset()?;
let fs = Filesystem::mount(simulator)?;
let file = fs.lookup(b"hello.txt")?;
let mut bytes = [0; 16];
assert_eq!(fs.read(&file, 0, &mut bytes)?, 16);
assert_eq!(&bytes, b"Hello SunlightOS\n");
```

Limits are enforced and tested: 16 regular files, 64-byte UTF-8 names, 65,536 bytes
per file, at most 16 one-block extents (also subject to the SB extent cap), one
exclusive mutation/commit owner and one dirty volume epoch. Root entries fit one
bounded directory page; the map fits one page; each file uses one extent page and
one extent root. Nonzero gaps are never inferred from disk garbage. Writes preserve
unchanged bytes and the precise EOF; zero-length writes do not extend files.
There is no truncate, rename, unlink, symlink, directory-tree or orphan mutation API.
Valid images outside the supported recovery subset return Unsupported; the checker
retains its existing wider inspection support. Sparse holes within the supported
size bound read as zero and become dense on the next snapshot.

The persistent identity remains `(UUID, nid, incarnation)`, independent of physical
mapping. Published node IDs are monotone and never reused; rewrites preserve the
tuple. The high-water mark is committed with the namespace. Tentative creation
identities have no survival guarantee when their entire uncommitted epoch is lost.
Handles additionally contain a private, nonwrapping runtime mount-session token:
all old handles expire on remount, including handles from abandoned creates. Lookup
returns a fresh handle with the same persisted tuple for a committed file. Stale
sessions, wrong UUIDs and wrong incarnations return `Stale`. The exclusive device
ownership requirement is an adapter/caller contract; cloning a device behind a live
owner is not supported. Test-only shared adapters inject faults under the harness.

## Admission and allocation

Each fsync builds a complete immutable volume snapshot. Before any persistent I/O,
it calculates data, file inode/root/extent pages, root directory inode/root/page,
node map page/root, orphan root, segment root/pages, all summary pages (including
their own ownership), checkpoint bytes, fallback ownership, and placement overhead.
Checked counters reject offset, generation and Node ID overflow. Remaining products
and indices are bounded by validated codec/geometry limits and the enforced runtime
caps. Every dependency is encoded and all core publication containers are allocated
before the plan enters persistent publication. The retained vector reserves three
slots at mount. The publication loop allocates no core metadata or file buffers.
The standard Rust allocator's process-abort behavior on global OOM is not converted
into a recoverable ENOMEM protocol; heap failure injection is outside this campaign.

Only segments outside the union of **all** valid retained closures are eligible.
Each snapshot has private segment placement and a new generation-qualified segment
epoch, preventing mixed segment epochs. Blocks left unused in a protected segment
are explicit placement overhead, not hidden owners. Allocation checks the exact
segment placement cost plus the untouched recovery/cleaner reserve. There is no
fallback to a protected block. Current-CP free counts describe its exact reachable
closure; runtime available capacity separately subtracts retained segment placement.
The recovery reserve is reported separately from that capacity and excluded by
admission. Full snapshots deliberately trade capacity and write amplification for
simple retained ownership.

Admission errors return without persistent writes or a freeze. Transient plans drop;
working changes remain volatile and may be discarded by dropping the mount.
Persistent I/O errors retain the complete plan as quarantine and freeze the owner.
No automatic retry, rollback write, or reuse follows uncertain completion. Read-only
recovery follows drop and a fenced/reset device session. The synchronous simulator
fences all outstanding work on reset; native timeout/DMA fencing is unqualified.

## Checkpoint protocol and fsync

Under the exclusive owner:

1. Prepare and encode the complete plan, excluding every retained closure.
2. If three valid checkpoints survived an earlier interruption, zero the oldest
   slot and flush R0 before discarding that root. Planning still excluded all three.
3. Write new file data and all immutable metadata to the admitted locations.
4. Flush D; a failure stops before checkpoint submission.
5. Write the new checkpoint to the already empty slot and flush P.
6. Zero the oldest previous slot and flush R, while preserving its closure until
   that barrier succeeds.
7. Install the new runtime committed snapshot and acknowledge fsync.

The normal final state has two complete generations in three slots. R0 and R are
actual durable slot invalidations, not header-age assumptions. A failed retirement
can leave the new generation durable while returning EIO; this is tested. The full
plan and all old ownership remain held by the failed owner. Mount reconstructs
ownership from media after reset, rather than trusting those volatile counters.

Successful fsync covers the complete working volume cut, including newly created
bindings and unrelated dirty files. Repeated clean fsync emits no I/O. Close/drop
has no durability effect. These are the bounded SFS-2 semantics, not a general POSIX
compatibility claim. Success is qualified by exact data recovered from the durable
image after power loss, not by counting barrier invocations.

## Recovery and independent checking

Recovery reads without writing. It validates both SB copies, compatibility and
geometry, then every checkpoint's entire metadata and raw-data closure. It checks
reference address/epoch/UUID/parent CRC/birth, node identity and high-water mark,
root bindings, extent order/coverage and EOF, checksummed zero tails, segment bitmap
bits, summary chains and exact ownership, duplicate blocks, orphan state and counts.
Cross-generation immutable block identities must agree. Only fully valid candidates
participate in selection and the protected union; the newest is selected.

`CleanWritable` requires two or three complete candidates, matching valid SBs, no
damaged nonempty slot and supported write/flush capabilities. Missing/corrupt
fallbacks or bad newer closures produce `DegradedReadOnly` when a valid older closure
remains. No valid closure is Corrupt; incompatible format is Unsupported; read/media
errors and conflicting identities are explicit Io/Indeterminate outcomes. Corrupt
slots are never silently cleared. Arbitrary media corruption may lose acknowledged
newest data; fallback is diagnosis, not a promise to repair that loss.

The independent checker still imports only low-level codecs. Its semantic traversal,
closure selection and allocation reconstruction do not call engine code. It now
exposes validated per-generation block lists, nodes, sizes, extents and root bindings.
The harness derives file bytes directly from those physical mappings and durable
image bytes, then compares them with the operation oracle and engine remount.
The checker independently verifies every retained generation, including data CRCs,
namespace, orphan separation, bitmap/summary exactness, duplicate ownership and
free counts. No shared semantic traversal was introduced.

## Deterministic correctness evidence

The final validation runner executes both debug and release profiles with separate
corpora and trace directories and requires identical results between profiles.

| Campaign | Cases per profile | Assertions |
|---|---:|---|
| Transaction with two retained generations | 1,538 | D/P/R/R0, exact durable generation, bytes/bindings, ownership protection |
| Transaction starting with three complete generations | 1,930 | R0 plus all later publication/retirement boundaries, same oracles |
| Reverse dependency persistence and sector fragments | 23 | Every reverse-prefix cut keeps the previous committed generation |
| Deliberately corrupt mutation images | 9 | Older complete selection, degraded read-only, no repair |

Total mutation crash/persistence schedules: **3,491 per profile**, plus nine corruption
images. Four seeds label deterministic replays of fixed schedules; they do not
claim four independent random order explorations. Every write/flush has before/after
power-loss cases. Write failures include zero/partial/full prefixes, representative
byte/sector tears and known/uncertain completion. Flush failures persist none, one,
or all pending writes. Reverse-order prefix persistence covers dependency reordering;
SFS-1 additionally preserves its six permutations of three independent sector writes.
This is a finite bounded campaign, not exhaustive enumeration of arbitrary hardware
or all permutations of the larger transaction queue.

The final oracle derives a singleton durable generation from the exact persistence
schedule, independently of fsync's return and recovery's observation. It then checks
the durable selected generation, exact file bytes and root names, checker/engine
agreement, every retained closure, no duplicate plan allocation and byte-for-byte
preservation of all initially protected blocks. Runtime free/placement/reserve counts
are also recomputed from checker-enumerated references. No row is PASS based only
on a returned error, CRC-valid checkpoint, or allocation counters.

The first successful vertical test uses exactly `hello.txt` and
`Hello SunlightOS\n` (16 bytes), on all three segment geometries. Other tests cover
empty/multiple/full-size files, zero writes, holes, boundary and multi-block writes,
eight overwrites with reuse, repeated fsync, volatile creation/writes disappearing,
low-space admission, metadata-heavy snapshots, configured extent cap, arithmetic
exhaustion, stale identities and expired handles. Every mount read in a valid
mutation fixture is independently failed. Unsupported revisions, readonly features,
missing checkpoints, missing fallback closure and unsupported flush are covered.

Seven of the nine corruptions are resealed with valid ancestor CRCs: node identity,
nlink/orphan inconsistency, out-of-bounds extent, duplicate ownership, future extent
generation, stale directory binding and allocation bitmap inconsistency. Two more
damage checkpoint/data CRCs. Each rejected newest closure falls back to generation 3
read-only and is preserved as an image. Images, exact schedules, raw traces,
compiled source hashes/snapshots and command logs remain under the selected attempt.
Images and raw traces are preserved with lossless gzip compression;
`trace-compression.json` records the verified uncompressed hashes. The single-case
replay `two-seed-0-event-0-power-before` also passed independently.

Early development attempts are retained separately. The first failed three fixtures
(expected buffer size, geometry-specific test arithmetic, too-small setup image);
the second exposed an existing-name error in the new three-slot test setup. The
third passed six mutation tests. Clippy findings and an intermediate handle-field
build error are retained. A first full seven-test qualification passed both profiles;
its source snapshot and artifacts remain historical evidence. Final qualification
adds expired-handle checks, stricter singleton persistence expectations, independently
recomputed runtime free space, and the configured extent-cap/exhaustion coverage.
Earlier successes do not qualify later source bytes.

## Builds, regressions and performance

Final qualification requires core `x86_64-unknown-none` compilation, 32 hosted tests
per profile (24 existing plus eight mutation tests), debug/release bins/examples,
Clippy with warnings denied, independent golden regeneration with unchanged bytes,
and all 139 existing storage regressions (`sunlight-block` 29, FAT 9, VFS 101).
No QEMU or guest validation is required or claimed. The workspace already emits an
unrelated duplicate-build-target warning for `sunlight-startup-fixture`; it does not
indicate a SunFS compiler warning or failed gate.

Performance observations are diagnostic single-host measurements, separate from
gates. The example records payload/data/metadata bytes, write amplification,
checkpoint count, flush count, checker pages/time, engine recovery time, exact plan
and segment overhead, and peak incremental allocated heap. Heap measurements include
simulator pending-write/trace overhead; they exclude prior heap, allocator bookkeeping
and stack and must not be called a core-only memory budget. See
[performance-baseline.json](../../audit/sunfs/sfs-2/performance-baseline.json) for final
profile values. Representative small snapshots write about 60–64 KiB for 16 logical
bytes, use one checkpoint publication and three flushes, and reserve a full segment.
This deliberately high amplification and tail placement cost is not optimized here.

Final release observations (one checkpoint and three flushes in each row):

| Blocks/segment | Payload | Written | Metadata incl. CP/R | Checker reads | Placement overhead | Peak incremental heap incl. simulator |
|---:|---:|---:|---:|---:|---:|---:|
| 64 | 16 B | 65536 B | 61440 B | 28 | 50 blocks | 175082 B |
| 64 | 8193 B | 73728 B | 61440 B | 35 | 48 blocks | 215066 B |
| 256 | 16 B | 61440 B | 57344 B | 26 | 243 blocks | 162762 B |
| 256 | 8193 B | 69632 B | 57344 B | 33 | 241 blocks | 202746 B |
| 512 | 16 B | 61440 B | 57344 B | 26 | 499 blocks | 162762 B |
| 512 | 8193 B | 69632 B | 57344 B | 33 | 497 blocks | 202746 B |

Release commit observations were 0.77–1.17 ms; engine recovery after the second
commit was 1.12–1.23 ms. These are captured simulator/process measurements, not
hardware latency forecasts.

## Integrity, limitations and SFS-3

[Source integrity](../../audit/sunfs/sfs-2/source-integrity.json) compares every incoming
regular file against the immutable baseline. Only the core module export/docstring
and checker inspection additions modify incoming files. New implementation files
are engine/recovery, transaction tests, the performance example, validator, this
report and SFS-2 evidence. Incoming workspace/lockfile changes, application document,
codecs, geometry, golden fixtures, kernel, other storage sources, and all SFS-0/SFS-1
outputs remain byte-identical. The manifest excludes its own hash and ignored build
artifacts; no commit or push occurs.

Remaining limits: bounded root namespace and file sizes; full snapshot memory/write
cost; whole-segment placement can return NoSpace with unused tail blocks; no cleaner,
repair tool, lifetime/orphan mutation, general namespace or native adapter; global
allocator exhaustion is fail-stop; checksums assume no collision/forgery; honest
flush and fenced device sessions are assumptions qualified only in the simulator.
No physical media, kernel/VFS, boot/root or CPython changes were made.

Recommended SFS-3 follows the existing roadmap: bounded general namespace and retained
file lifetime, atomic rename/replacement, unlink/open/last-close semantics, committed
orphan reaping and directory pins, with exact admission and independent D/P/R/R0
oracles extended first. Keep GC for SFS-4 and native secondary-volume qualification
for SFS-5. This milestone authorizes neither expansion.

Reproduce all qualification with `python3 tools/sunfs2_validate.py`. Replay a recorded
case with `python3 tools/sunfs2_validate.py --replay two-seed-0-event-0-power-before`.
Verify final source/evidence bytes without rerunning tests using
`python3 tools/sunfs2_validate.py --integrity-only`. Each qualification/replay creates
a new directory; earlier failing observations and SFS-0/SFS-1 evidence are preserved.

## Final acceptance gates

Final source campaign: `raw/qualification-20261114T181049.913356Z`.
Both profiles passed 32 tests and identical exact-state corpora; all 139 storage
regressions passed. The final source and all retained evidence hashes were rechecked.

| Gate | Status | Evidence |
|---|---|---|
| G1 Baseline/compatibility | PASS | source-baseline.json, baseline-results.json, format-compatibility.json |
| G2 Engine/API | PASS | mutation-tests.json |
| G3 Admission/reservations | PASS | transaction-admission.json |
| G4 Checkpoint/fsync | PASS | checkpoint-commit.json, durable images and schedule oracles |
| G5 Recovery | PASS | recovery-selection.json |
| G6 D/P/R/R0 | PASS | crash-matrix.json, exact-state schedule corpora and traces |
| G7 Independent checker | PASS | checker-agreement.json |
| G8 Ownership/accounting | PASS | resource-accounting.json |
| G9 Builds/regressions | PASS | regression-results.json |
| G10 Integrity | PASS | source-integrity.json, validation-summary.json |
