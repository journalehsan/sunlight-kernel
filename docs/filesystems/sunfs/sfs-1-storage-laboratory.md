# SunFS SFS-1 storage laboratory

2026-11-14. Experimental format family 0, revision 2. Hosted implementation and
validation only. The machine-readable verdict and individual gates are in
[`validation-summary.json`](../../audit/sunfs/sfs-1/validation-summary.json).
SFS-1 COMPLETE requires its ten PASS gates and a PASS final integrity manifest.
This milestone supplies reusable codecs, a bootstrap publisher and a storage
laboratory. General filesystem mutations, native mounts and crash-safe fsync are
not implemented or qualified here.

## Baseline and decisions

Incoming revision `1affc823311cb6014e2aecf63b848b2bd4f2af6d`, branch `main`.
Incoming status contained the untracked SFS-0 documentation/evidence directories.
The initial manifest records 88,428 regular files. SFS-0's 88,406 incoming regular
file hashes and its 21 recorded output hashes verified without mismatch before
implementation. The final integrity pass repeats those preservation checks.
Symlink metadata and ignored build artifacts are outside the regular-file baseline.

Inspected the current workspace, `.cargo/config.toml`, toolchain, `sunlight-block`
trait/cache/MemDisk/partition conventions, native 512-byte assumptions, existing
image-test tooling, FAT/VFS error enums and local CRC/record code. Existing CRC32
utilities do not implement the required CRC64 domain; the new codec implements the
specified ECMA polynomial directly. No dependency downloads or upgrades were needed.
Only three workspace members and three local lockfile package entries are added;
the validation runner compares parsed manifests/lockfiles against HEAD to reject
unrelated changes. SFS-0 files remain unchanged.

[ADR 0002](adr/0002-sfs1-experimental-codec.md) fixes executable implementation
choices without changing the recovery protocol: exact v0.2 layout, strict reserved
zeros, bounded tables, dense bootstrap placement and fixed-point summary ownership.
It corrects the prose segment cap to `82 * floor(3976 / 96) = 3362`; the SFS-0
arithmetic evidence already used that value. No existing ADR was rewritten.

## Components and dependency direction

| Component | Responsibility | Normal dependencies |
|---|---|---|
| `sunlight-sunfs` | `#![no_std]` + alloc; codecs, checked geometry, block contract, immutable bootstrap plan/publication | None |
| `sunlight-sunfs-lab` | Volatile/durable simulator; safe hosted image adapter; `sunfs-format` CLI | `sunlight-sunfs` |
| `sunlight-sunfs-check` | Independent read-only graph traversal; `sunfs-check` CLI | Core codec definitions only |

The laboratory has a **dev-only** checker dependency for integration tests and
benchmarks. The core has no host file, thread, clock, network, POSIX or QEMU APIs.
`cargo check -p sunlight-sunfs --target x86_64-unknown-none --offline --locked`
passes with the repository's configured bare-metal target. This is compilation,
not a guest execution result. The checker imports `codec::*`; it does not call
bootstrap planning, geometry calculation, publication or engine recovery selection.

## Block and persistence contract

`DeviceInfo` exposes capacity, logical/optional physical block size, transfer
alignment, maximum transfer, optional identity, media incarnation, read-only state,
flush and FUA availability. Exact reads, write completion and flush have separate
results. `WriteCompletion::Completed` does not promise durability. Failures retain
known completed-prefix counts or uncertain completion; unsupported barriers have a
separate error. No ambiguous write is retried automatically.

The `Sector512Adapter` models eight checked sector requests for a 4 KiB transfer.
Reads stage into a bounded temporary buffer and expose nothing after a sector
failure. Write errors preserve the aggregate completed prefix. Native drivers are
not wrapped or certified by this model; timeout fencing remains a native obligation.

The simulator stores read-visible bytes, durable bytes and a pending write queue
separately. Normal writes affect only visible/cache bytes. Explicit byte fragments
may persist in arbitrary order for independent ranges; overlapping bytes preserve
submission order. A successful flush drains all earlier accepted bytes. Failed
flush schedules permit none, some or all writes to persist while still returning
failure/uncertainty. Power loss discards visible and pending state; reset increments
the incarnation and reconstructs the session from durable bytes. Every operation
records seed, event ID, range, input CRC64 digest, outcome, persisted fragments and
incarnation. Deterministic schedules, not host time or threads, control failures.
A deliberately dishonest flush mode demonstrates lost acknowledged durability and
is explicitly excluded from conforming-device verdicts. FUA is advertised unavailable
by these adapters and returns UnsupportedDurability.

## Codecs and validation layers

Explicit LE codecs cover the common page header, superblocks/copy IDs/features,
checkpoints/generations, NodeId, BlockRef, time validity, map entries, inode metadata,
directory entries/pages, extents/pages, segment state/bitmap, summaries, orphan
records and bounded table roots/pages/descriptors. No native struct casts or unsafe
Rust are used. Counts and byte lengths are bounded before decoding/allocation;
unknown required versions/features, nonzero reserved bytes/tails and malformed
references are rejected. Context permits zero identities/references only in their
specified optional/internal positions.

CRC-64/ECMA-182 uses polynomial `0x42f0e1eba9ea3693`, non-reflected, init/xorout zero;
`123456789` gives `0x6c40df5f0b497347`. Metadata coverage is all 4096 bytes with
64..71 zeroed. Data checksums include the specified UUID/owner/logical/epoch/content
generation domain. Four golden binary pages are generated independently with
Python `struct.pack` and a separate CRC implementation. Tests verify exact bytes,
field sizes, round trips, cross-endian representation and malformed inputs.
Decode/local structural checks are separate from checker cross-reference traversal
and checkpoint recovery eligibility. A checksum alone is never an eligibility oracle.

## Geometry, reservation and costs

The calculator accepts 4096-byte SunFS blocks, power-of-two logical sectors from
512 through 4096, 64/256/512-block segments and exactly three checkpoint slots.
SBs occupy blocks 0 and 1; CPs 2, 3 and 4; remaining leading blocks are reserved;
segments start at one segment boundary. Only complete segments are allocatable.
Metadata is immutable segment-area allocation, not a separate mutable table region.
All layout arithmetic is checked or bounded by previously validated format limits.

Bootstrap base cost is `6 + segment_table_pages` blocks. Summary count iterates to
`sum(ceil(live_blocks_in_segment / 82))`, including summaries' own placement, before
encoding starts. The planner rejects invalid or over-budget input geometry. Tests
cover placement feedback at segment and 82-entry boundaries and all table caps.

For configured caps of 128 nodes and 32 orphans, admission computes an empty-object
metadata bound from two pages/node (node plus empty content root), three global
roots, node-map/orphan/segment pages and fixed-point summaries. Cleaner reserve is
one full segment victim + three such metadata closures + three publication writes.
Bootstrap plus reserve must fit in the segment area. This is a conservative SFS-1
admission policy for the laboratory, **not** an admission/progress proof for future
arbitrary file/namespace updates. SFS-2 must plan every actual mutation's extents,
directory pages and ancestors; SFS-4 must qualify cleaner relocation and fallback pins.

| Segment blocks | Record bytes | Maximum segments | Smallest default image | Bootstrap blocks | Cleaner reserve blocks |
|---:|---:|---:|---:|---:|---:|
| 64 | 72 | 4510 | 3,932,160 bytes (960 blocks) | 8 | 871 |
| 256 | 96 | 3362 | 6,291,456 bytes (1536 blocks) | 8 | 1063 |
| 512 | 128 | 2542 | 8,388,608 bytes (2048 blocks) | 8 | 1316 |

These minima come from enumerating complete-segment capacities with the stated
metadata caps and reservation formula, rather than selecting a fixed minimum size.
At 1 GiB with 256-block segments there are 1023 segments, 25 segment-table pages,
32 bootstrap metadata blocks (128 KiB), 1135 cleaner-reserved blocks and a 32,736-byte
union bitmap per snapshot. Rewriting the three global roots and full tables at the
configured node/orphan caps costs 126,976 bytes per checkpoint, excluding changed
namespace/data/summary pages. Geometry rejects capacity beyond the 82-page table
limit; it does not silently switch to an incremental tree.

## Formatter and checker

Example hosted commands (create a **new** disposable file):

```sh
cargo build -p sunlight-sunfs-lab -p sunlight-sunfs-check --release --offline --locked --target x86_64-unknown-linux-gnu
target/x86_64-unknown-linux-gnu/release/sunfs-format --image /tmp/new-sunfs.img --size 16MiB --confirm-disposable
target/x86_64-unknown-linux-gnu/release/sunfs-check --image /tmp/new-sunfs.img
```

The formatter plans before creation, requires confirmation, creates exclusively,
refuses existing files/devices/symlinks and noncanonical ancestor aliases, and walks
parent components using pinned Linux directory descriptors with O_NOFOLLOW.
Its hosted adapter is bounded to 128 MiB, and has no overwrite option. Size parsing
is checked. UUIDs use the host random source; deterministic tests inject fixed IDs.
A newly sized zero file receives both SBs, root directory, node map, segment metadata,
empty orphan root and self-accounted summaries; D0 makes dependencies durable; then
CP1/CP2 are written and P0 flushed. Any I/O failure stops publication immediately.
No general file creation or mutation API is provided.

The checker opens a regular image read-only. Its independently implemented walker
checks geometry, all SB/CP candidates, versions/features, parent CRCs, epochs/births,
map identities, root, sorted directory names and targets, parent cycles, link counts,
extents/data CRC/EOF tails, orphan separation, segment bits, exact summary ownership,
duplicate live references, counts and immutable identity across fallback closures.
Reads are bounded to 65,536 pages per candidate and directory bindings to the node
cap. It independently orders complete generations and protects the union. A newer
checksummed CP with a bad closure is rejected and an older valid generation reported
as degraded. The nonempty fixture is assembled by test code; it does not represent
an executed filesystem mutation engine.

| Result / exit | Exact scope |
|---|---|
| CLEAN / 0 | Matching valid SB copies; two or three distinct complete recoverable generations; no malformed nonempty CP slot; known read-only feature policy |
| DEGRADED_RECOVERABLE / 1 | At least one complete closure; missing/damaged SB or CP, only one generation, or read-only-compatible feature; inspection only |
| CORRUPT / 2 | No complete recovery closure under known readable format, including unformatted/incomplete images |
| UNSUPPORTED / 3 | Unknown required format/version/feature; no recovery advice |
| INDETERMINATE / 4 | Read/budget failure, conflicting valid SBs, duplicate/conflicting generations or incompatible shared immutable identity |

CLEAN describes image structure under the hosted model, not permission to mount a
native device writable. The checker never repairs, retires checkpoints or writes.
CLI image SHA256 before/after checking is unchanged.

## Executed validation and limitations

Each final host profile executes **24 Rust tests**: 8 codec/golden tests, 3 geometry
suite tests and 13 laboratory tests. Geometry includes a 1500-input grid plus
boundary/minimum/maximum-capacity cases; accepted layouts are actually planned. Maximum-capacity sparse-reader fixtures are
also checked, including a 64-block layout whose metadata/summaries cross segment
boundaries without allocating a multi-GiB host buffer.
The exact test names and subcase markers are retained in the raw logs. Existing
storage regressions are 29 `sunlight-block`, 9 `sunlight-fat` and 101 `sunlight-fs`
tests: **139 passed**. Core bare-metal check, hosted CLI builds and Clippy with
warnings denied pass. There is one pre-existing workspace duplicate-bin manifest
warning outside these crates. Cargo.lock contains no unrelated package changes.

Per profile, the finite crash campaign covers 14 formatter operation events before
and after each boundary with four seeds: **112 cases**, each independently replayed
and compared byte-for-byte and trace-for-trace. Full traces and crash-image CRC64
identities are retained. Another **480 cases** cover prefix/suffix tears at
0,1,511,512,1024,1536,2048,2560,3072,3584,4095,4096, known partial completion, failed
writes, uncertain writes and flush failure with none/some/all persistence. Exact
expected checker classifications are asserted for every such schedule. All six
permutations of three independent writes are tested, with same-block ordering tested
separately. This is bounded enumeration, not exhaustive filesystem crash safety.

Corruption tests include 16 selection/classification cases, seven resealed empty-
graph corruptions and nine nonempty semantic/data corruptions. They test both CRC
damage and independently checksummed but inconsistent references/accounting.

The fuzz campaign per profile uses seeds 0,1,2,0x534653: **2048 mutated images** plus
**2048 arbitrary pages**, with **16,384 additional typed-payload decode attempts**.
There are no observed panics. Corpus rows record input identity, seed, index and
classification; profile classifications match. Two CLEAN mutations hit only the CRC
field before recomputation and therefore restored the original page; the test
requires every CLEAN mutated image to equal the original bytes. Zero panics is not
a correctness proof. `tools/sunfs_reproduce.py` reconstructs a corpus case and checks
its identity against the retained corpus before creating a new output file.

Development attempts are retained separately: `lab-tests-initial.log` has one
invalid test bitmap fixture (popcount changed before semantic checking), followed
by the corrected passing runs. `source-integrity-first-attempt.json` records the
symlink-scope error in the validation harness. `strict-fault-oracle.log` and
`strict-oracle-all-tests-failure.log` preserve an overly strict torn-CP expectation:
a 511-byte prefix already contained every nonzero CP byte, and the untouched fresh
slot tail was zero. The corrected oracle compares complete resulting page bytes;
it does not require an atomic/full-length transfer for recovery. None of these
attempts is erased or presented as an originally passing run.
`segment-state-reproduction.log` records a real local validation defect found during
source review: RETIRED could carry current live bits and an OPEN segment could have
epoch zero. The codec now requires no current ownership in RETIRED and a nonzero
epoch in every non-FREE state; regression assertions exercise both conditions in
the final suite. Formatting and Clippy development logs are also retained.

## Diagnostic measurements and next milestone

Raw measurements record both repository dev (opt-level 1) and release (opt-level 3,
LTO) profiles. On this host, release codec encode/decode cost is approximately
10 microseconds per 4 KiB page over 10,000 iterations. The 8/16/64 MiB simulator
format medians are approximately 0.3/0.3/0.4 ms and checker means approximately
0.25/0.30/0.68 ms. The 16 MiB CLI run including process launch and host sync is on
the order of a few milliseconds for format and one millisecond for check; exact
latest observations are in `performance-baseline.json`, not portability promises.
Simulator buffer storage is about twice image size plus pending byte arrays and
per-byte persistence masks (16,859,136 bytes for the 8 MiB workload); trace storage
and allocator overhead are excluded from that figure.

Reevaluate full-table rewriting before crossing 1 MiB of global tables/commit,
metadata closures above 10% of usable capacity, the explicit 82-page/6724-node
limits, or measured intended-hardware sync p95 above 50 ms. The latency threshold is
a proposed future decision trigger; hardware latency is NOT_RUN here. Full table
rewrites remain intentional, with no architecture change made for these diagnostics.

SFS-2 should implement the minimal persistent create/write/fsync engine with exact
transaction admission, zero-tail/hole semantics, two retained checkpoint closures,
D/P/R/R0 cuts, fail-stop uncertainty and the real format → mount → create → write →
fsync → crash → remount → verify workload. It must extend the independent checker
fixtures and bounded recovery schedules after every transaction. General namespace
lifetime, GC progress and native integration remain later milestones.

Remaining unqualified contracts include actual native flush honesty, timeout/DMA
quiescence, exclusive volume ownership, arbitrary mutation reservations, retention/
reclamation, cleaner progress, VFS authorization/pins and native POSIX metadata.
CRC detects accidental corruption under the documented collision assumptions; it
is not authentication. Hosted Linux image safety is not a portable native adapter.
No native guest, UI or real-hardware result is claimed. No production mount,
physical formatting, boot/root change, installer work, commit or push occurred.

## Evidence and reproduction

Run `python3 tools/sunfs_validate.py` for the hosted suite, CLI exercises, diagnostic
measurements and manifests. It overwrites only its own regenerated final evidence;
initial attempts remain separate. Run `python3 tools/sunfs_validate.py
--integrity-only` after report/evidence edits to bind the final bytes. The integrity
manifest excludes its own hash and ignored artifacts, records only Cargo.toml and
Cargo.lock as changed incoming files, and verifies that every SFS-0 output remains
unchanged. An actively redirected runner log must be finalized before the last
integrity-only pass.

All required JSON deliverables and raw logs/corpus/traces are under
[`docs/audit/sunfs/sfs-1/`](../../audit/sunfs/sfs-1/). `commands.json` binds command
arguments, exit codes, logs and compiled-source hashes. `source-integrity.json`
binds final source/report/evidence hashes. Generated reproducer images are compressed
with deterministic gzip timestamps. No test result from an earlier source snapshot
is used to validate later implementation edits.
