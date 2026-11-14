# SunFS implementation roadmap and evidence gates

2026-11-14. SFS-0 is design/audit only. Every execution below is future work unless
explicitly recorded in `docs/audit/sunfs/sfs-0/validation-summary.json`.
No performance or crash-safety claim follows from a document gate.

## Hosted laboratory (Stage A)

Propose three crates outside production dispatch: a no_std+alloc engine/codec with
bounded allocations, a hosted device/fault adapter, and command-line format/inspect/
check frontends. Exact crate names/paths are SFS-1 choices after workspace inspection.
Reuse the sunlight-block semantics via adapters; do not couple engine algorithms to
host File, POSIX pwrite or kernel globals. Share codecs/constants with checker only;
checker graph reconstruction and ownership logic must remain independent.

The image device has separate read-visible volatile state and a durable backing
image. Every request records operation ID, type, byte range, input digest, completion,
persistence fragments, flush cut and seed. Normal writes update volatile state;
scheduler chooses when allowed fragments reach durable bytes. Reads observe latest
completed write within the live device session. Explicit flush drains all previous
accepted writes in the permitted order, then syncs the host backing file before
returning success. Host fsync is adapter behavior, not proof that a physical target
honors SunFS barriers. A simulated power loss drops volatile state/pending requests
and creates a new media session from durable bytes only. No cached engine state
survives remount. Keep the durable image separate from generated expected results.

Fault scenarios: disabled/failed flush; persistent cache with arbitrary pre-barrier
write reordering; byte/sector torn writes; known short transfer; error before/after
partial completion; successful writes lost before flush; flush error after some/all
bytes persisted; power loss after submission, completion, persistence fragment and
flush return; media incarnation change; timed-out late completion fenced before reuse.
No scheduler may move an acknowledged durable write back into volatile-only state.
Model configurations and excluded hardware faults are explicit in each run.

First vertical test is exactly:
`format → mount → create → write → fsync → crash → remount → verify`.
SFS-1 builds lab/format/check fixtures; the first engine-backed vertical test belongs
to SFS-2, with a minimal one-level namespace sufficient for create. SFS-3 generalizes
namespace/lifetime; no fake create fixture counts as the vertical test.

Use deterministic seeds (initial corpus0,1,2,0x534653), explicit operation sequences
and stable schedules. Enumerate every operation boundary and each persistence cut
for bounded workloads, plus representative tear offsets0,1,511,512,4095,4096 and all
sector boundaries. Exhaustively enumerate allowed reorderings for small queues;
record the bound, do not call a larger state space exhaustive. Add crafted corrupt
metadata with recomputed checksums to exercise structural rejection. Larger seeded
stress supplements, never replaces, bounded enumeration.

The oracle records only acknowledged mutation epochs and successful fsync cuts;
failed sync may yield old or new complete state. Verify acknowledged bytes survive
model power loss; interrupted updates retain the old committed state or a new fully
consistent state; atomic rename has no mixed parents; no two live owners share a
block; all pointers/checksums/epochs are valid; no invalid checkpoint is accepted;
all retained fallbacks remain readable; no automatic repair writes occur. Hash
images before/after read-only mount/check. A corrupt nonzero CP may yield degraded
read-only fallback under v0.2; writable admission must fail exactly as specified.

Each failure preserves seed, operation and persistence trace, starting and crash
image hashes, selected CP/rejection reasons, checker output, source revision/hashes
and toolchain identity. Minimize failing operation/schedule sequences deterministically,
preserving original attempts as well as the reduced trace. Never replace observed
failures with later passing runs or mock measurements.

## SFS-1 — Laboratory, codecs, format and check fixtures

Scope: geometry/capability/error interface and eight-sector adapter model, deterministic
volatile/durable device, field codecs and golden vectors, fresh-image formatter,
read-only inspector/checker foundation. No VFS/boot integration or filesystem mutation
engine. Depends on SFS-0 ADR and explicit bounded format proposal.

Acceptance: independently checked byte offsets/lengths/CRC vectors; roundtrip every
record and reject malformed lengths/reserved/version/feature bits, overflow and bad
pointers; format produces two complete empty-root checkpoints and matching SBs;
checker validates complete bootstrap graph and union accounting. Compare segment
sizes64/256/512 for metadata bytes, minimum volume, bitmap memory, dirty bounds,
reserve feasibility and deterministic workloads. No benchmark labels for arithmetic.
Freeze only tested codec revision; update proposal for demonstrated defects.

Crash scenarios: interrupted formatting at every durable boundary, torn SB/CP,
short/error flush, cache reordering, all-zero/malformed slots. Safety: formatter only
accepts an explicit new disposable image; no host device auto-discovery or repair.
Evidence: runnable host tests, seed/trace corpus, golden images/hashes, checker output,
commands and source manifest. Stop on inconsistent codec/checker, false flush success,
undefined retirement, reserve nonconvergence or inability to isolate durable bytes.

## SFS-2 — Minimal persistent engine and checkpoint recovery

Scope: bounded node/map/segment/orphan tables, regular-file data/extent I/O, minimal
root directory create/lookup, full checkpoint/fsync, read-only fallback. Depends on
SFS-1 codec and lab proof. Introduce no GC service or general POSIX adapter.

Acceptance: first real vertical test above; overwrites and multi-block files, holes,
zero-tail truncate and full-table rewrites; independent checker passes after every
accepted mount; complete CP closure validation rejects structurally valid bad graphs.
Both retained checkpoints remain protected through repeated commits/restarts. Explicit
ENOSPC admission and latched I/O errors. Security: bounded counts, no duplicate owner,
no unchecked address arithmetic, no runtime identity reuse after close/remount.

Crash scenarios: every D/P/R/R0 cut, torn writes and all bounded reorderings, three
valid slots, retirement failure, fsync EIO with new durable state, interrupted truncate,
corrupt newest/fallback, missing dependency. Evidence: actual durable-image replay,
acknowledged epoch oracle and independent graph results. Stop on any acknowledged
loss, mixed snapshot, premature reuse, silent writable fallback or fabricated repair.

## SFS-3 — General namespace, unlink and retained lifetime

Scope: multiple directories, atomic rename/replacement, empty rmdir, open-unlink,
dup/last-close ownership, committed orphan reaping and directory pins. Depends on
SFS-2 and bounded namespace transaction reservation. Minimal SFS-2 create remains
an explicit dependency; the vertical test is not postponed behind full namespace.

Acceptance: identity unchanged across rename, stale name cannot reopen, old open file
remains own object after replacement, distinct nlink/open counts, deterministic
close/unlink/read-write interleavings, crash orphan reclamation with allocation counts,
directory parent-cycle rejection and generation exhaustion. No hardlinks/symlinks yet.
Crash scenarios: both-parent rename, unlink before/after commit, last-close/reap,
replacement and orphan table full. Security: retained authority, no path relookup,
mount pins and bounded resource accounting. Evidence: traces and exact backing
reclamation after last close AND fallback retirement. Stop on aliasing, lost ownership,
unbounded orphan accumulation or non-atomic directory graph.

## SFS-4 — Foreground cleaner and crash-safe GC

Scope: summary-guided victims, validated liveness, ordinary relocation transactions,
reserve admission and full-disk bounded progress. Depends on SFS-2/3; no background
service required. GC must work for internal map/table/summary pages as well as data.

Acceptance: pinned readers survive relocation, both fallback closures preserved,
repeated overwrite/unlink workloads reclaim measured blocks, cleaner reserve cannot
be consumed by users, fallback-only victims are skipped, impossible progress returns
bounded ENOSPC. Crash scenarios: each relocation publication/retirement, crash with
CLEANING state in RAM, summary corruption, exhausted reserve, metadata-only commits.
Security: no early free or double reservation; no capacity inflation to force a pass.
Evidence: checker, liveness/accounting traces and independent progress events. Stop
on reserve deficit, starvation, unbounded waits or reclaiming any fallback reference.

## SFS-5 — Secondary SunlightOS volume and native consumers (Stages B/C)

Scope: qualify block/cache/partition/driver flush and timeout fencing, SunFS VFS variant,
object/mount pins, bounded I/O without global spinlock waits; disposable secondary
mount only. Depends on SFS-4 and native directory/lifetime/block prerequisites.

Acceptance: actual QEMU volume persists across guest restart; native SunShell and
Files create/read/write/rename/unlink through authoritative syscalls, fd/dup/close
and mount policy hold; actual Files clicks separately observed. Preserve FAT/root
regressions. Device absence/removal, unsupported flush and write failures fail closed.
Crash scenarios: QEMU process power cut at instrumented barriers, cache-mode changes,
late completion/reset, unmount/close/GC races on supported CPU counts. Security:
existing device/mount/secret grants enforced; no service/raw bypass or DMA reuse.
Evidence: source-bound guest serial, images before/after, fd/resource accounting,
CPU matrix and UI logs. Real hardware remains distinct NOT_RUN until exercised.
Stop on unqualified barriers/quiescence, lock cycles, boot-path dependency or authority
regression. Do not modify boot/root to make a secondary mount test pass.

## SFS-6 — POSIX metadata, directory-relative paths and symlinks (Stage D)

Scope: authoritative versioned metadata/time validity, stable inode/device mapping,
retained openat/enumeration, symlink/readlink/stat/lstat/fstat/realpath and shared
resolver/native std convergence. Depends on SFS-3/5, native time and authorization
contracts. No declarations-only POSIX compatibility claims.

Acceptance: actual APIs with missing/intermediate/dangling links, 40-link loop bound,
mount boundaries, permission failures, race schedules, caller buffers/allocated
realpath ownership, timestamps/provenance and directory cookies. Crash scenarios:
link create/rename/unlink and metadata updates at each commit boundary. Security:
capability ownership and ancestor checks survive aliases and TOCTOU; no link escape.
Evidence: native C/Rust/application common fixtures and independent persistent checks.
Stop on fabricated fields, pathname-hash inode, lexical canonicalization or unresolved
retained directory authority. CPython Stage F may resume after this gate with unchanged
configured build/object set; it need not wait for RootFS installation.

## SFS-7 — RootFS and installer preparation (Stage E)

Scope: separate design/authorization for installer, crash-safe provisioning, RootFS,
boot and rollback strategy, recovery media and FAT ESP coexistence. Depends on SFS-6
and qualified hardware persistence; SFS-0 authorizes none of these migrations.
Acceptance: disposable-install/reboot/rollback/recovery exercises preserve prior media;
explicit review of upgrade and failure recovery. Crash scenarios: installer copy,
boot selection and interrupted upgrade, without destroying prior recoverable root.
Security: least-privilege device grants, no automatic host-device formatting. Evidence:
actual installation matrix and recovery demonstrations. Stop before any migration
without its separate authorization or when rollback cannot be demonstrated.

## SFS-8 — Measured optimization and optional features

Scope: measured map/index optimization, GC policy, packing/hot-cold/prefetch and only
then optional compression; encryption/backups/snapshots require separate ADRs.
Depends on complete correctness regression suite and workload measurements.
Acceptance: unchanged crash/security oracles plus captured nonempty performance,
write-amplification/memory/latency results against the baseline. Every feature gets
compatibility flags and old-reader rejection tests. Crash scenarios expand to every
new publication/encoding path. Stop on correctness regressions or unmeasured claims;
retain the known-correct format/engine path and do not trade recovery for speed.
