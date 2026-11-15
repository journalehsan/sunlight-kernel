# SFS-5B.2 — VirtIO fencing and safe DMA retention

**PARTIAL. The descriptor-head adversarial guarantee, active-DMA fencing,
resource reclamation and active-SMP qualification remain open. Native writable
SunFS remains BLOCKED.** Production now verifies initial reset acknowledgment,
checks live device status around normal submission/retirement, preserves original
uncertainty provenance, and provides bounded terminal whole-device reset. It does
not release DMA pages, reinitialize a queue or authorize a new queue epoch.

The declared scope is coherent x86_64 legacy PCI VirtIO Block with split queue 0
under QEMU 11.1.2/TCG. No physical-device qualification is claimed. Work began
on local November 15, 2026, Asia/Tehran; raw attempt names use UTC November 14.
Incoming branch was `main`, revision
`22a5947f5fbd4ffe810aee5efee57b0ec18f8915`, with uncommitted SFS-5A/SFS-5B.1
changes. Those changes and their evidence are preserved. No commit, push, native
SunFS mount, root/boot migration, physical formatting, CPython or SunApp change
was performed.

Evidence: [SFS-5B.2 directory](../../audit/sunfs/sfs-5b2/),
[gate ledger](../../audit/sunfs/sfs-5b2/validation-summary.json),
[specification matrix](../../audit/sunfs/sfs-5b2/virtio-specification-matrix.md),
[pre-implementation fencing contract](../../audit/sunfs/sfs-5b2/fencing-contract.md).

| Gate | Verdict | Boundary |
| --- | --- | --- |
| G1 baseline/transport/specification | PASS | Incoming hashes/patch, source audit, actual feature values and normative reset contract |
| G2 reproduced and repaired ABA | PARTIAL | Original failure reproduced and preserved; adversarial same-head/new-status alias remains |
| G3 normal completion ownership | PASS | Conforming-device exactly-once logical retirement; live-status admission/retirement guards added |
| G4 timeout quarantine | PARTIAL | Hosted retention/negative cases pass; required native uncertain-DMA interleavings absent |
| G5 native reset/fencing | PARTIAL | Real idle whole-device reset and status-zero acknowledgment; no active outstanding-DMA qualification |
| G6 safe DMA retirement | UNQUALIFIED | No release justified; all five pages remain pinned |
| G7 native faults/independent witnesses | PARTIAL | Real requests and reset entry traced independently; active-DMA fault/reset races absent |
| G8 progress/SMP | PARTIAL | Bounded hosted reset steps and native idle acknowledgment; AP execution/contention NOT_RUN |
| G9 final-source qualification | PARTIAL | Available regression suites pass as recorded below; mandatory native fault/recovery/SMP scenarios incomplete |
| G10 integrity/accounting | PASS | Required outputs, final source/input seal, failed attempts and earlier evidence accounted for |

## Transport and normative boundary

The native disposable device is `1af4:1001` at `00:02.0`, legacy BAR0 port
`0xc000`, byte status register `0xc012`. Standard QMP PCI/block inventory
independently captures that attachment and its regular-file backing. The guest
records offered features **`0x71006ed4`** and accepted features **`0x240`**:
`VIRTIO_BLK_F_BLK_SIZE` (6) and `VIRTIO_BLK_F_FLUSH` (9). The implementation
allowlist is bits 1/5/6/9; actual unaccepted offered bits are recorded per run.
It uses one 512-byte data descriptor and real type-4 flush commands.

The legacy feature interface exposes only bits 0–31. `VERSION_1` (32),
`RING_PACKED` (34) and `RING_RESET` (40) are not negotiated. There is no supported
queue-reset register path. Geometry continues to report fencing Unsupported:
a diagnostic whole-device reset acknowledgment is not a qualified reusable
fence API. The exact pre-reset queue-size number was not printed; initialization
admits only a power-of-two queue whose full extent fits four pages (4–256).
No numerical queue-size observation is fabricated.

The official OASIS VirtIO 1.4 CS01 specification dated April 8, 2026 is retained
as exact HTML/text with hashes. Sections **2.4.1/2.4.2** define device reset
acknowledgment by reading status zero and forbid subsequent queue interaction
and notifications until reinitialization. Sections **3.3/3.3.1** distinguish
live queues and exposed-buffer ownership after device reset. Section
**4.1.4.10** supplies the legacy BAR0 status layout; 4.1.4.11.0.1 explicitly
identifies its byte-zero reset write. General initialization plus its legacy
exception are in **3.1/3.1.2**. Modern common-configuration and queue-reset
requirements are separately identified and are not imported into this transport.

For a conforming device, observed reset acknowledgment establishes the stated
old-queue interaction boundary. It does not establish storage persistence,
hostile-device compliance, physical PCI posted-DMA drain, IOMMU invalidation or
platform translation/reuse safety. Native reset was performed only after all
four fixture requests completed. No independent active-DMA drain witness exists.
All allocations are therefore retained even after acknowledgment.

## Preserved descriptor-head failure and normal completion

`raw/descriptor-aba-failing.log` records the deterministic failing safety
witness, using the current incoming fake-ring driver. Device incarnation 1,
queue incarnation 1, A generation 1 and B generation 2 both use head 0. A
advances used index 0→1 and retires. After B publication, the injection advances
used index 1→2, writes head 0, length 1 and B's status slot to zero. B passes
current-token/chain/status validation and is incorrectly accepted as Completed
under the requested adversarial interpretation. The failure includes exact
request identities, indices and invalid acceptance. An isolated rerun on archived
incoming driver bytes replaces the demonstrated-limit assertion with a safety
assertion; it fails with B already Retired. Its log, complete reproducible crate
and exact source bytes are retained under `raw/aba-pre-repair-assertion.*` and
`raw/aba-reproducer/`. The initial pre-production failure marker is also retained.

This is a malicious completion after **legitimate completed-request reuse**.
It is not evidence of timeout reuse, repeated consumption of one used entry,
index wrap corruption, post-reset state leakage or compliant hardware emitting
stale messages. A head-only duplicate cannot supply B's newly initialized status;
that negative case quarantines. The stronger old-head-plus-B-status case is
physically indistinguishable from a real B completion. Software generation
counters do not solve it. The original demonstrated-limit test remains in the
suite, and its passing assertion is not labeled a repaired safety guarantee.
G2 remains PARTIAL.

Normal conforming completion keeps the existing exact-next-used-index, head,
status, chain-address/length/flags/link and active owner/generation/queue checks.
`Owner::complete` and `retire` reject stale tokens and repeated retirement. The
used index advances once; status is initialized to `0xff`; every descriptor used
by the new operation is rebuilt before publication. Protocol indices wrap at
16 bits; request generations refuse overflow. New status guards quarantine
before publication or retirement if the device no longer reports the expected
legacy Running status. Stale interrupt notification itself has no authority:
completion is polled from the used ring. Legacy used length remains diagnostic,
with the reference corrected to 5.2.6.3; no arbitrary short-count authority is added.

## Implemented terminal reset and retained ownership

`begin_terminal_reset` requires exclusive mutable ownership of the entire PCI
device. It closes admission and quarantines the current owner before issuing a
single whole-device status-zero write. Repeated begin calls return the same
attempt state; they neither issue another write nor restart its budget.
`poll_terminal_reset` performs at most one register read per call, with 64 reads
maximum. No timer, interrupt, completion-handler or scheduler dependency is
introduced, and no queue memory is changed by these operations.

Zero enters `VerifiedRetained`. Exhausted budget enters `TimedOutRetained`;
status other than zero or the prior Running value enters
`UnexpectedStatusRetained`, including possible-removal `0xff`. All are terminal
for admission. Late observations cannot rewrite the terminal result. If a caller
abandons a Waiting attempt, resources remain retained and I/O remains refused.
There is no OldEpochRetired/Reinitializing/RunningNewEpoch transition.
Register-instruction completion itself assumes a responsive platform; a finite
number of reads is not a hard wall-clock bound on an unresponsive device/emulator.

No new lock, reset-time allocation, retry loop, descriptor pool inflation or PMM
free path was added. Existing synchronous submission still polls up to
50,000,000 iterations under the external device mutex, and callers may retain
outer VFS locks. Native contention fairness and scheduler latency remain open.
The new reset steps do not wait for work requiring those locks, but the milestone
does not establish active-SMP progress.

Each production attachment keeps **four queue pages plus one bounce/request
page: 20,480 bytes**. Normal completion retires logical request ownership, not
those frames. An uncertain request retains its token, descriptors, header,
status, data, queue memory and bookkeeping through reset. Zero pages are freed,
zero DMA mappings are dismantled, and no resources transfer to a new epoch.
The hosted fixture uses two aligned 16KiB arrays for fake queue/request memory;
its byte-retention assertions are not native PMM or real DMA observations.

The original uncertain receipt is now retained separately from subsequent
refusals and reset observations. This includes timeout uncertainty and a
terminal failed/unsupported write or flush whose durability remains uncertain.
Reset never turns them into definite failure or durable success. An idle terminal
reset conservatively closes the existing uncertainty/admission latch even though
no request is outstanding; that latch alone is not an uncertain-request count.
A future SunFS adapter still needs fenced recovery and independently validated
durable checkpoint closures before mutation.

## Native observations and independent witnesses

Final native reset evidence contains six disposable-device runs: 1/2/4
configured vCPUs, each with QEMU `cache=writeback` and `cache=none`. Each boots from
ISO, checks the exclusive 4MiB image marker/capacity, reads the marker, writes
sector 61, flushes, reads it back, requests idle reset, observes zero, refuses a
sector-62 write and checks repeated reset initiation. Whole-image comparisons
verify only sector 61 changed; compressed image bytes, serial output, exact argv,
source/ELF/ISO hashes and trace configuration are retained.

Witness A is the driver receipt/owner/reset-readback path. Witness B is QEMU's
trace backend recording actual block request handling/completion and reset
entry, plus standard QMP attachment/backing inventory. Each trace identifies the
VirtIO block object independently. The final DRIVER_OK-to-reset window contains
four completed fixture operations. Earlier firmware/pre-driver I/O remains in the
raw trace and is excluded explicitly; the checker failure that initially counted
all traffic is preserved. A reset-entry trace is not an independent reset-completion or DMA-drain
observation. The witnesses are not promoted to G6 PASS.

The first native attempt was prevented from binding its QMP socket by the
sandbox. The first unrestricted observer used experimental
`x-query-virtio-status` after reset and hit a QEMU 11.1.2 assertion in
`visit_start_struct`. Its raw logs and runner are retained. Its original guest/
image PASS rows are not observer PASS results. The corrected runner uses trace
records and standard QMP inventory, checks observer errors and requires expected
host termination. Matching QEMU implementation-source retrieval failed; no
source-level QEMU DMA-drain proof is claimed.

F1–F22 are explicitly inventoried with model/native boundaries. Hosted tests
exercise timeout retention, late ring writes, invalid heads/chains/status,
index wrap, delayed/withheld reset acknowledgment, unexpected/removal register
values, idempotent initiation, pressure, unchanged buffers and uncertainty
provenance. They do not exercise real DMA or real transport reset. Native active
DMA during reset, injected delayed/withheld reset, old completion in a new epoch,
actual unplug, concurrent reset initiation, completion/reset races and safe
reinitialization remain absent or partial as recorded. No missing case is
manufactured into PASS. Four additional final-source native regressions repeat the
512-entry queue and 4096-byte block guards, read-only write refusal, and real
QEMU blkdebug flush EIO. Their expected refusals/error assertions pass. The flush
case preserves a completed sector-60 write in the host image; it demonstrates
visibility after failed flush, not durability or rollback. Its full post-error
owner/first-uncertainty state is not printed and remains a separate hosted proof.

All native fixtures stop before `smp::start_aps`. AP execution, secondary-CPU
progress, post-AP queue operations and I/O/reset contention are **NOT_RUN**.
Configured vCPU count is not an SMP qualification. Active reset contention needs
an execution/coordinator harness beyond the existing synchronous pre-AP fixture.

## Final-source regressions and integrity

All available final-source executions passed: **54 SunFS tests in each debug and
release profile, 178 storage/backend tests (including 43 block and 25 VirtIO), and
212 kernel host tests**. SFS-2/3/4 produced **13,142 crash/reordering schedules per
profile** (3,491 + 6,691 + 2,960), comprising 13,044 fault-boundary rows plus 98
explicit reordering rows. Every selected recovery state belongs to its expected
set, and debug/release schedule corpora are byte-identical. These are hosted
model schedules, not native or physical storage crashes.

Final native executions comprise **12 SFS-5A/SFS-5B.1 normal/cold-restart boots,
6 terminal-reset boots, and 4 supplemental native guard/error cases**. All
configured CPU/cache combinations in the declared scope passed their specific
assertions. The normal native fixture uses raw byte/barrier patterns; it does
not run writable SunFS or its recovery engine.

Default, `sfs5b1-test` and `sfs5b2-test` kernel builds, relevant no_std checks,
scoped Rust formatting, Python compilation, whitespace checks and scoped
VirtIO/block Clippy completed successfully. Existing Clippy documentation/style
warnings and workspace/IPC warnings are retained; this is not a clean whole-tree
lint claim. The current T1–T20 hosted matrix remains 17 scoped PASS and 3 PARTIAL
(T5/T17/T20). F1–F22 records 12 hosted PASS, 7 PARTIAL and 3 NOT_RUN, with each
case's native/DMA/reset boundaries explicit. None of these totals closes the
missing native fault/SMP/retirement gates.

The final runner snapshots bind actual source and argument hashes. Correcting the
observer and then the uncertain-error provenance required later final-source
executions; older mismatching attempts remain preserved and are excluded from
final qualification. No previous build or prior milestone PASS qualifies a new
source tree. The initial baseline witness invocation accidentally selected zero
tests because `--exact` omitted the module path; the selected rerun and actual
failing witness are separately retained.

The final seal verifies incoming prior SFS evidence byte-for-byte and identifies
this milestone's delta independently of pre-existing work. It also seals exact
final production/fixture/runner copies and the incoming patch. Post-execution
tool/Limine hashes and a post-build depfile inventory record binaries and
generated/ignored payload dependencies; it is explicitly
not a before-build or hermetic attestation. Per-invocation ELF/ISO hashes bind the
actual images. Recheck using
`python3 docs/audit/sunfs/sfs-5b2/derive.py --check`.

## Remaining blockers and SFS-5B.3

SFS-5B.3 should first provide an independently observable active-I/O reset backend
and a justified queue-epoch/resource-retirement protocol. It must reproduce the
actual reset/late-completion interleavings, qualify platform DMA reuse, then
exercise post-AP contention and progress. A conforming-device completion protocol
and an arbitrary malicious same-head/status writer require separate security
claims; the latter cannot gain a generation through software bookkeeping.

Typed USB/cache/partition/FAT/VFS propagation, legacy short-transfer authority,
USB MSC qualification and actual media-removal uncertainty remain separate block
contract blockers. T5/T17/T20 are not closed by the new reset observation.
Physical storage durability remains UNQUALIFIED. Native writable SunFS remains
prohibited even if a future VirtIO-only fencing milestone completes.

This milestone modifies `sunlight-virtio/src/{blk,lib}.rs`, adds the
`sfs5b2-test` feature/hook in `kernel/{Cargo.toml,src/main.rs}`, and adds
`kernel/src/sfs5b2_test.rs` and `tools/sunfs5b2_validate.py`. The report and all
19 required deliverables plus raw evidence live under the paths linked above.
Earlier block/USB/lockfile changes belong to incoming SFS-5A/SFS-5B.1 work and
remain untouched.
