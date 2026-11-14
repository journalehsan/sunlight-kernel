# SunFS SFS-3 namespace and orphan lifetime

2026-11-14. **SFS-3 COMPLETE: G1–G10 PASS in the bounded hosted scope.**
Machine-readable verdict: [validation-summary.json](../../audit/sunfs/sfs-3/validation-summary.json).
No native mount, physical disk formatting, guest validation, boot/root migration,
Python filesystem port, SunApp packaging, commit or push occurred.

## Baseline and design decision

Incoming branch `main`, revision `24aa8186b8abd55099727d37d3da53e04d83c6be`, clean worktree.
[source-baseline.json](../../audit/sunfs/sfs-3/source-baseline.json) hashes 103,418
incoming regular files. All SFS-0/SFS-1/SFS-2 evidence stays byte-identical. Read the
current architecture, format, recovery/integration contracts, ADRs 0001/0002, prior
reports, codecs, formatter, transaction admission, retained closures, checker and
simulator before editing engine code. The unchanged baseline passed core no_std,
32 tests in each profile and hosted bins/examples. Raw baseline logs/corpora reside
in `raw/baseline-retry/`. An earlier runner omitted legacy evidence directories;
its four output-file errors are retained under `raw/baseline/`, not erased.

[ADR 0003](adr/0003-sfs3-bounded-namespace-lifetime.md) records the preimplementation
decision. **No on-disk layout/version/feature change.** Existing v0.2 explicitly
specifies hierarchical directories, directory parents/nlink, orphan records and
mapped zero-link nodes. The engine now supports that existing representation.
Codecs, formatter, geometry and four independently regenerated golden fixtures
remain byte-identical. Unsupported version rejection still passes the legacy tests.
An SFS-2 engine explicitly rejects hierarchies/orphans; no compatibility upgrade or
silent old-engine reinterpretation is claimed.

## Identity and API

| Concept | Owner and identity |
|---|---|
| Persistent node | `(volume UUID, nid, incarnation)`, monotonic NID, no recycling |
| Directory binding | `(parent NodeId, name bytes)` names one node, no dirent number |
| Open description | Private registration: NodeId, mode, offset, descriptor references |
| Public handle | Opaque mount-session/token, monotonically allocated, never reused |
| Physical extent | Owner/logical block/content generation/segment epoch, separate from name |
| Checkpoint backing | Independently protected union of every complete retained closure |
| Orphan | No binding, nlink=0, mapped complete backing and persistent orphan record |

`root`, `mkdir(parent,name)`, `create_at`, `lookup_at`, `lookup_components`,
`entries_at`, `rename`, `unlink`, `rmdir`, `sync_all`, `reap_orphans` extend the
existing create/lookup/read/write/fsync APIs. Directory listing is a fresh bounded
snapshot, not a retained cookie. Rename changes identity neither for source files
nor directories. Internal objects carry binding fields separately from NodeId;
file parent associations generate directory entries, while the file inode parent
field stays zero as v0.2 requires. Directories serialize their actual parent.

Legacy `FileId` is a trusted linked-node selector, not a capability or lifetime pin.
It remains usable through rename, becomes Stale after unlink, and expires on remount.
Its public identity fields do not grant user authority. Offset-based reads/writes
explicitly address a linked object without changing an open description's position.
The eventual VFS adapter owns credentials, ancestor search/mount checks, public
process descriptor tables and in-flight operation pins. It must map authorized
pins to this single engine lifetime policy, with no parallel reclamation policy.

`open(selector, mode)`, `read_handle`, `write_handle`, `seek`, `position`, `dup`,
`close`, `fsync_handle` implement minimal regular-file descriptions. Independent
opens have independent offsets. Dup creates a registered token sharing mode and
offset; cloning an opaque token does not register another reference. A copied token
can be consumed only once. Duplicate close and closed/remounted tokens return Stale.
Read/write access modes are enforced; they do not implement user authentication.

## Namespace bounds and rename

Keep 16 regular objects total, including orphans; 64-byte UTF-8 names; 65,536 bytes
per file and the configured extent limit. Add 16 nonroot directory objects including
orphans, 16 entries per directory, depth 8 (root depth 0; files count), 64 supplied
components, min(16, SB orphan cap), 32 descriptions and 64 registered handles.
Superblock node, orphan and exact transaction bounds still apply. All 33 nodes,
16 full orphan records, and 64-byte names are exercised together. A maximum-size
combination can exceed the configured transaction bound and safely return NoSpace.
Nothing inflates that bound or consumes cleaner reserve to make a test pass.

Each component walk checks the current directory before interpreting dot/dot-dot;
missing/.. and regular-file/.. fail. Root .. clamps to root. Stored names cannot
be dot, dot-dot, slash, NUL, invalid UTF-8 or empty. No lexical symlink emulation,
symlinks, hardlinks, traversal outside the root or recursive graph walk exists.
Creation checks resulting depth. Rename checks every moved descendant's resulting
depth and rejects moving a directory below itself. Duplicates and bounds are checked
before binding changes. Empty rmdir and same-kind empty-directory replacement work;
nonempty destination replacement and file/directory type mismatch fail.

Rename changes source/destination bindings, replacement orphan state, parent and
link counts together under the exclusive mutable owner. One full-volume checkpoint
publishes that epoch. It never executes two independently durable directory updates.
An open source retains its identity; an open overwritten destination retains its
original node while new lookup resolves to the source node. Same-binding rename is
a no-op. Path selectors do not retain removed directories: directory opens, pinned
relative lookup/openat and retained dot-dot parent contracts are explicitly unsupported.

## Open-unlink, last-close and reaping

Unlink removes the binding and queues a zero-link orphan, after reserving orphan
record capacity. The record stores unlink generation/reason=1 and all backing stays
in the node map/segment ownership. Even a closed unlinked object first becomes a
pending orphan; no special direct-free path exists. Capacity exhaustion returns
Limit before namespace mutation. Existing descriptions retain read/write access.
Creating a new object under the same name allocates a new NID; old handles cannot
reach it. Data writes through an unlinked handle participate in full-volume fsync.

Closing a token consumes one descriptor reference. Only the final reference to a
description removes its object pin; other independent descriptions still pin it.
Last-close queues eligibility, emits zero I/O, and never frees a physical block.
`reap_orphans` removes all eligible objects and their records in one normal admitted
checkpoint. It skips pinned orphans, returns zero without committing if none qualify,
and safely rejects duplicate/stale close. NoSpace restores working objects/dirty
state and emits no writes. Uncertain I/O freezes the owner and retains the complete
reservation/quarantine. Close may still consume references in that frozen owner.

Reaping removes current-map ownership only. Older published generations retain their
complete backing independently; allocation excludes every retained closure until
its slot is durably invalidated. The single allocator is the only physical reuse
owner. Runtime reads use owned memory, not disk pointers, and every snapshot copies
all remaining objects including pinned orphans. Thus handles do not require another
physical retention allocator or hidden backing outside the checkpoint union.

Mount is read-only inspection. It validates every persistent orphan with all other
closure invariants, restores zero volatile descriptions, and exposes every cold
orphan as a reap candidate. **Recovery reaping is explicit**, via `reap_orphans` on
a writable owner; mount does not perform a hidden cleanup write. After that commit
and remount, selected-map orphans are gone; fallback generations still validate.
Read-only/degraded mounts never reap. No cleaner or guaranteed progress at full
snapshot/reserve exhaustion is claimed; safe NoSpace can defer retirement.

## Persistence and independent resource proof

The SFS-2 protocol is unchanged: complete plan/encoding, optional oldest-slot R0,
dependency writes, flush D, checkpoint write, flush P, oldest-slot zero, flush R,
then acknowledgment. All planning excludes the initial union even if R0 later
retires one root. No publication-time allocation or direct orphan free is added.
Failures freeze mutation rather than retrying uncertain I/O.

Engine accounting separately reports current checkpoint blocks, fallback-only union,
pending reserved blocks, protected segments, placement overhead, available capacity,
cleaner reserve and reclaimable capacity after reserve. Runtime accounting separates
namespace nodes, orphans, pinned orphans, eligible candidates, descriptions and tokens.
Reclaimable capacity describes whole-segment allocation eligibility, not GC progress.

The independent checker continues its own semantic traversal, importing codecs only.
It already validates tree reachability/parents/cycles, names, identities, links,
orphan separation, extents/data, exact summaries/bitmaps, duplicate ownership and
fallback eligibility. New inspection fields expose every directory's entries,
parent/nlink and persistent orphans. Test oracles reconstruct paths and file bytes
from those physical references, compare handcrafted expected identities/data/orphans,
and recompute union/current/fallback/segment/free/reserve counts. They compare every
initially protected block byte-for-byte and reject duplicate planned allocation.
Cold images cannot reveal runtime handles; live pin assertions use separate known
open/dup/close sequences. No cold checker invents open-reference counts.

## Final qualification

Final attempt: `raw/qualification-20261114T191407.625977Z`. Compiled source SHA256 and a source archive
bind the tested bytes. Debug/release each passed **43 tests**: all 24 SFS-1,
all eight SFS-2, and eleven SFS-3 tests. Existing storage regressions: **139 PASS**
(block 29, FAT 9, VFS 101). Core x86_64-unknown-none check, both hosted builds,
Clippy with warnings denied, scoped rustfmt and unchanged golden regeneration pass.
Only the existing unrelated duplicate startup-fixture build-target warning remains.

Each profile executes **6616 new namespace fault schedules**, plus **75** new
reverse-dependency/sector-prefix schedules, and all **3,491** legacy SFS-2 schedules:
**10182 mutation schedules per profile**. The legacy SFS-1 formatter
112 crash cuts/480 faults and 4096 malformed-input campaign also rerun; these are
separate evidence, not added to the mutation count. Profile corpora match exactly.
Seven new CRC-resealed malformed namespace/orphan images and all nine SFS-2 corrupt
images pass expected degraded fallback checks; no repair writes occur.

Fifteen new scenarios cover mkdir/create/write, same/cross rename, open replacement,
directory move/replacement, unlink/read, orphan write, rmdir, last-close retirement,
cold reaping and three-retained replacement/reaping. Every persistent write/flush
has before/after cuts, failed/prefix/torn/uncertain writes, and failed barriers with
none/one/all pending writes durable. D/P/R/R0 assertions require exact independently
derived generation, complete namespace/bytes/orphans, engine/checker recovery
agreement and protected backing safety. Writable recovered orphans are actually
reaped and rechecked after another crash. This is finite deterministic coverage,
not exhaustive arbitrary hardware reorder exploration. Thread races are NOT_RUN;
sequential active-reader/dup-close schedules are not labeled thread concurrency.
A future barrier-controlled race plan is retained in crash-matrix.json.

Development observations remain separate: omitted baseline output directories,
8 MiB fixtures correctly rejected by snapshot/reserve admission, a Clippy test
Drop warning, and an interrupted qualification while correcting the transient peak
retained-backing observation. Each is preserved separately before final qualification. Earlier passing snapshots do not
validate later source. Raw traces/images are losslessly gzip-compressed with
verified original/compressed hashes; commands, fault rows and source hashes remain.

## Metadata and performance observations

The 256-block-segment fixture has two small regular files and three nonroot
directories. These release simulator observations are diagnostic, not hardware
latency forecasts or optimized results. Metadata includes checkpoint/retirement.

| Operation | Metadata bytes | Data bytes copied | Flushes | Elapsed ns | Protected blocks |
|---|---:|---:|---:|---:|---:|
| rename | 98304 | 8192 | 3 | 1298890 | 49 |
| unlink | 102400 | 8192 | 3 | 1406807 | 49 |
| retirement | 86016 | 4096 | 3 | 1146205 | 45 |

Each nonempty orphan table adds one 4 KiB record page plus its mandatory 4 KiB root;
records are 32 bytes, bounded to 16 (512 bytes of records). Last-close adds no writes.
Eligible retirement costs another whole-volume checkpoint. Traversal timing is
captured for 10,000 two-component lookups, alongside peak retained block counts,
in [performance-baseline.json](../../audit/sunfs/sfs-3/performance-baseline.json).
Peak retained backing is independently reconstructed from pre-commit closures plus
the new private closure that coexisted at flush P, before retirement. It includes
the temporary three-generation union, not just the two post-commit roots.
No memory allocator or cleaner optimization is included.

## Security, SunApp and remaining scope

Review checked bounded names/counts/walks, checked generation/token arithmetic,
opaque mode-restricted registrations, stale identity rejection, transactional
replacement/orphan capacity, input typing and fail-stop ownership. CRC is accidental
corruption detection, not authentication. Global allocator OOM retains the inherited
fail-stop contract; recoverable heap exhaustion is unqualified. The hosted mode
model supplies no multiuser credentials, process FD table, POSIX locking or native
VFS integration. Runtime device/session exclusivity and reset fencing remain adapter
obligations; actual driver honesty/DMA quiescence are unqualified.

[SunApp contract](../../audit/sunfs/sfs-3/sunapp-installation-contract.md) reviews
write/verify/fsync/rename/directory-fsync activation and old-open backing. Atomic
regular-file replacement is a useful foundation. Nonempty directory replacement
is unsupported, so directory package activation needs a future indirection or
exchange contract. Filesystem rename does not authenticate packages or isolate apps.

Remaining limits are full-snapshot memory/write amplification and whole-segment
tail waste, bounded objects, explicit reaping, no full-disk progress guarantee,
no directory pins/openat, no symlinks/hardlinks/truncate, no thread race or native
physical durability qualification. Recommend **SFS-4: bounded foreground cleaner,
relocation and reserve/progress proof**, preserving identities, runtime ownership
and every retained closure, with independent crash/resource oracles before tuning.
Directory pin/VFS integration needs its own retained-relative-operation contract.

## Deliverables, gates and reproduction

Implementation: `sunlight-sunfs/src/engine.rs`, `engine/namespace.rs`,
`engine/handles.rs`, `engine/recovery.rs`; checker inspection:
`sunlight-sunfs-check/src/lib.rs`; hosted tests:
`sunlight-sunfs-lab/tests/namespace.rs`; driver: `tools/sunfs3_validate.py`.
All requested JSON artifacts, raw evidence and the SunApp review are in
[docs/audit/sunfs/sfs-3](../../audit/sunfs/sfs-3/). The final source integrity manifest
checks all incoming files and hashes every changed/new output except itself.

| Gate | Result | Basis |
|---|---|---|
| G1 | PASS | unchanged baseline, format/fixture preservation |
| G2 | PASS | bounded tree/dot/type/depth/entry and maximum-object tests |
| G3 | PASS | one-checkpoint rename/replacement and exact crash namespace oracles |
| G4 | PASS | modes, shared/independent offsets, stale/cap tests |
| G5 | PASS | old-open/new-name identities, close order, retained backing |
| G6 | PASS | persistent orphan states, explicit cold reap, no early reuse |
| G7 | PASS | finite hosted D/P/R/R0 matrix; thread races NOT_RUN |
| G8 | PASS | independent full-closure traversal and reconstructed accounting |
| G9 | PASS | final builds, 43 tests/profile, 139 regressions, lint and scoped review |
| G10 | PASS | compiled-source/output hashes, earlier evidence preservation |

Run `python3 tools/sunfs3_validate.py` for a fresh isolated qualification.
Replay: `python3 tools/sunfs3_validate.py --replay replace-seed-1397117747-event-0-power-before`.
Verify: `python3 tools/sunfs3_validate.py --integrity-only`.
