# SFS-5B.4 — Asynchronous device access and scheduler-aware fencing

**PARTIAL. SFS-5B.4 is not COMPLETE. The DMA retirement fence remains
FENCE_UNQUALIFIED. All five attachment DMA pages remain pinned; no new queue
epoch is authorized. Native writable SunFS remains BLOCKED.**

This work qualifies a deterministic event-ordering backend and actual scheduled
reset ownership contention. It does not demonstrate a memory-copy instruction
executing simultaneously with reset, certify physical hardware, or authorize DMA
reclamation. The authoritative ledger is
[validation-summary.json](../../audit/sunfs/sfs-5b4/validation-summary.json), with
all raw attempts retained in [the new evidence namespace](../../audit/sunfs/sfs-5b4/).

## Scope and baseline

The incoming branch was `main`, revision
`22a5947f5fbd4ffe810aee5efee57b0ec18f8915`, already ahead of its upstream by one
commit. The incoming worktree included the uncommitted SFS-5A through SFS-5B.3
changes. The new baseline records 414,227 regular tracked/unignored files,
porcelain status, the incoming binary patch, tool versions, compilation
configuration and environment. Prior SFS artifacts and failed experiments are
preserved byte-for-byte. The final seal reports this milestone's delta
separately from incoming modifications.

The native transport remains legacy PCI VirtIO Block `1af4:1001`, split queue 0,
coherent pinned guest RAM, accepted features `0x240` (BLK_SIZE and FLUSH), and no
negotiated queue-reset feature. Actual offered features, guest queue/request
physical addresses, RequestId, task generations, CPU IDs and PMM counts appear
in each serial log. Every native invocation binds its source inventory, emulator,
ELF, ISO, disposable image, arguments and environment by hash. Fault seed
`0x5b4001` identifies fixed fixture bytes and barriers; it is not a randomized
native race seed. Earlier runners retain their own recorded seeds.

The kernel feature is off by default. Only new, exclusively created marked 4MiB
regular-file images are attached. Sector 62 supplies normal write/flush/readback,
sector 63 supplies the held request, sector 64 is a refused post-closure write,
and sector 61 selects explicit cancellation. No existing boot/root image or host
block device is attached. The fixture never returns to filesystem mounting.

## Evidence models

[transport-assumptions.md](../../audit/sunfs/sfs-5b4/transport-assumptions.md)
separates the claims:

* **NORMATIVE / Model A:** a conforming device obeys the applicable whole-device
  reset and queue-interaction contract. This is conditional on device and
  platform behavior and says nothing about persistence. The retained SFS-5B.3
  OASIS VirtIO 1.4 CS01 analysis supplies the precise section references.
* **INSTRUMENTED_QEMU / Model B:** an isolated patched QEMU 11.1.2 using TCG,
  `ioeventfd=off`, no IOThread, the main AioContext and BQL serialization. Real
  guest descriptors and payload mappings are accessed. Deferred callbacks are
  asynchronous scheduling, not a concurrently executing physical DMA engine.
* **ADVERSARIAL_NEGATIVE / Model C:** a deliberately broken test-only model
  performs a successful post-claimed-fence mapped-memory read. Detecting that
  violation validates the observer, not the broken model.
* **NATIVE_OBSERVED** identifies real guest scheduling, submissions, timeout,
  quarantine and status-port reset receipts. **HOST_MODEL** identifies hosted
  driver and crash-schedule assertions. **UNQUALIFIED** identifies missing
  interleavings, physical DMA, mapping retirement and hardware durability.

The legacy used ring has no authentication data that distinguishes a perfectly
forged identical old completion from a legitimate new one. A software request
or queue generation cannot add missing wire information. Existing ABA fixtures
and that threat boundary remain intact.

## Device backend and timeline

The QEMU source review covers `virtio-blk.c`, `virtio.c`, `virtio-pci.c`,
`virtio-bus.c`, `block-backend.c`, `system/physmem.c`, memory API declarations,
timer cancellation and QEMU's IOThread documentation. Copies, source/archive/
patch/binary hashes, configuration and build logs are retained. The system
emulator is unchanged. The earlier temporary SFS-5B.3 source/build were left
untouched by this work; they were later absent after the interruption described
below. Original patches, identities and replay logs remain preserved. `rebuild-qemu.sh` builds another
isolated instance; it does not install anything.

A real OUT request for sector 63 is fetched and mapped by QEMU. The hook reads
256 actual payload bytes, publishes a test status byte `0xfe` without advancing
used.idx, and holds the real request before backend submission. The guest
observes that byte before timeout. Thus payload-read completion precedes guest
timeout through an explicit cross-boundary handshake, without subtracting guest
and host clocks. The driver still requires a valid used entry and final status
for completion; the diagnostic byte grants no completion authority.

The guest logs a reset request before its synchronous status-zero port write.
The device separately logs reset entry, timer cancellation, detach/unmap,
quiescence, queue clearing and its acknowledgment condition. The mapping guard
finishes before that port instruction returns. The guest subsequently logs
pending and then its actual status-zero readback. These port and memory
handshakes provide synchronization edges between the independent processes.

The machine-readable [event graph](../../audit/sunfs/sfs-5b4/device-event-graph.json)
retains per-device monotonic host timestamps, device sequence, queue incarnation
and descriptor head, and guest logical sequence, task incarnation, CPU and role.
Edges distinguish device BQL order, same-task program order across native yields,
and explicit cross-boundary handshakes. Guest atomic log-counter order across
CPUs is not treated as task happens-before. Each graph is checked for cycles.
The QEMU trace and sector-63 identity correlate the one initial queue epoch; no
legacy on-wire request generation is invented.

The backend has `hold`, `reset-read`, `error`, `negative`, and `release` modes,
plus cancellation and unmodified-emulator controls. `hold` accesses payload before
reset. `reset-read` reads the remaining 256 bytes **inside** reset handling,
before quiescence and acknowledgment. That demonstrates O2 ordering after reset
request, but not O3 simultaneous memory instructions or O4 waiting for an active
asynchronous access. Both remain UNQUALIFIED. The `error` branch records an
injected cancellation condition, not an actual backend EIO.

The held timer has a finite 3000-callback budget at 10ms intervals. Expiry stops
rearming and retains the request. Reset cancels the timer while holding BQL
before freeing its request. No callback waits for another BQL callback. A later
timer observer carries scalar identity only, never an old request or guest
pointer. Its rejected stale-operation event is policy observation, not a real
stale DMA transaction or an original block-backend completion.

No custom unsynchronized worker thread was added. QEMU memory mappings retain
MemoryRegion references until unmap; queue cache access and direct RAM mapping
lifetime are distinct. The selected static-RAM test uses original mapping
pointers for protection after detach, with no hot removal, migration or reuse.
That is insufficient to establish platform DMA-mapping retirement. General
`blk_drain()` has no new wall-clock bound; the held operation was never submitted
to that backend. No arbitrary stuck-backend drain claim is made.

## Negative controls and fencing decision

After generic reset, the negative model performs an asserted successful one-byte
kernel pipe-copy from the still-pinned original payload mapping and records an
illegal post-fence access. Offline graph validation detects that actual access
after the model's acknowledgment event. Safe variants contain no such violation.
The positive pipe-copy control is recorded before the acknowledgment event.
The test then installs PROT_NONE on the original four queue pages and one request
page. Five EFAULT negative probes validate that host virtual-mapping guard. QEMU remains observed for a finite post-ready window.

These probes exercise host CPU virtual addresses. They do not establish an
IOMMU fence, protection from physical device DMA, alias containment or hostile
hypervisor isolation. Merely protecting guest page permissions would not supply
those guarantees either.

[fencing-decision.json](../../audit/sunfs/sfs-5b4/fencing-decision.json) separately
classifies reset acknowledgment, old access termination, callbacks, platform
mapping lifetime, queue ownership, descriptor retirement and persistence. The
serialized hook's cancellation and queue clearing are observed. A general
asynchronous access/backend fence and allocator reuse proof are missing.
Consequently **FENCE_UNQUALIFIED** is the only supported retirement decision.

Four queue pages and one request/header/data/status page, **20,480 bytes per
attachment**, stay pinned. Zero guest DMA pages, descriptors or uncertain request
authorities are released. Native logs retain the exact old RequestId and first
uncertain result and sample PMM counts independently. Those counts are not a
per-frame allocator reuse demonstration. Normal release controls legitimately
retire their completed requests before idle terminal reset, while retaining the
five attachment pages. Conditional retirement, fresh queue allocation, new-epoch
initialization and fresh post-reset I/O remain NOT_RUN.

## Staged driver and scheduled coordinator

`sunlight-virtio/src/blk.rs` now shares submission and completion validation
between the existing synchronous interface and staged `start_write`,
`poll_write`, and `timeout_write`. Each staged call performs bounded work under
exclusive attachment ownership. Request metadata survives between calls. A
stale token cannot consume the used ring, and terminal paths cannot reopen it.
No frame-release, reinitialization or fence-authorization interface was added.

Review reproduced two defects in the initial staged implementation. Reset or
cancellation before timeout could omit uncertainty provenance. A second admission
could interpret an already-arrived completion owned by the first request. The
retained red tests demonstrate both. Terminal quarantine now records the pending
write's uncertain result before reset, and busy admission rejects before ring
inspection. The original first uncertainty is retained, including its request,
operation, LBA, unknown amount and durability. Green tests cover those repairs,
stale tokens, exactly-once completion, timeout/late-ring refusal and unchanged
live DMA bytes. These are new staged-API repairs, not a claim that the original
synchronous baseline exhibited those interleavings.

The off-by-default `kernel/src/sfs_reset.rs` coordinator closes admission and
selects one trusted task incarnation with CAS. Tickets qualify attachment and
queue generation. Each `step` uses try_lock and advances one state:

`RUNNING -> QUIESCING -> RESET_OWNER_ESTABLISHED -> RESET_PENDING ->
RESET_ACKNOWLEDGED -> FENCE_UNQUALIFIED -> TERMINAL_QUARANTINE`.

The normal block completion path polls the used ring; no block interrupt handler
acquires this coordinator. Its bounded steps do not depend on guest interrupt
completion. This is not validation of a future IRQ-driven queue-retirement path.
The staged fixture waits at least 20 BSP ticks and requires the device byte
handshake before declaring timeout; the existing synchronous polling budget is
preserved. A global two-million-call fixture budget and the host boot deadline
bound failed rendezvous attempts.

The driver quarantines before requesting reset. Polling remains limited to 64
status observations. Explicit cancellation retains the old owner and pages
without claiming acknowledgment. There is no reset owner transfer, new epoch or
resource retirement. The coordinator is diagnostic pending full race and
involuntary worker-exit qualification; it is not integrated into the normal
storage service as an automatic recovery mechanism.

`kernel/src/sfs5b4_test.rs` constructs actual ring-3 fixture tasks, explicitly
enqueues them using the existing scheduler, pins their CPU masks, and uses normal
native yield calls between bounded kernel steps. Scheduler identity locks are
dropped before device operations. No reset operation runs under SCHEDULER, no
callback depends on a reset-held guest lock, and no new scheduler architecture is
introduced. No production scheduler deadlock was reproduced or claimed repaired.

Before issuing reset, the owner must yield until every online CPU's timer count
and an independent scheduled worker advance beyond snapshots taken **after**
reset ownership was established. The uncertain request remains outstanding in
that interval. This proves progress during live coordinated ownership rather
than only after termination. Logs include actual CPU IDs, PIDs, task generations,
Running states, the competing Closed receipt and acquired-step state receipts.
A failed try_lock can also be logged, but it is not assumed to occur.

S4 competing reset ownership, S8 admission against closed state, and S10 timer/
worker progress have scoped native witnesses. The 1-CPU profile is a serial
reference. The 2/4-CPU profiles identify distinct owner/contender CPUs and
independent workers on every participating CPU. S1 exact admission/closure,
S2 completion/ownership, S3 timeout/publication and S7 IRQ/retirement cuts are not
forced. S5, S6 and S9 remain partial: retention is observed, callbacks are limited
to the instrumented timer, and explicit cancellation is not involuntary worker
exit/reaper integration. The complete SMP gate therefore remains PARTIAL.

## Native matrix, regressions and integrity

The corrected v2 emulator's final native sweep is
`raw/native-20261115T103331.975665Z`: **42/42 scoped boots PASS**, with identical
before/after source inventories. Each CPU profile (1, 2 and 4) covers seven modes
with both `cache=writeback` and `cache=none`. All six deliberate negative-control
cases detect the illegal model access. All event graphs are acyclic; no payload
operation is active at reset entry. The 28 SMP boots record distinct reset
owner/contender CPUs and independent workers on every online CPU. These are
S4/S8/S10 witnesses, not completion of the ten-race matrix.

The source-equivalent replay of the earlier `cpu-2-writeback-unmodified` stall
passes with the unchanged 90-second bound. Its recovered ELF/ISO differ from
the original inputs. `raw/early-boot-stall-replay.json` records that distinction;
the earlier failure and its unknown cause remain preserved and unqualified.
The recovered guest ELF and ISO are compressed inside the evidence namespace,
so another Cargo cleanup will not remove these replay inputs.

The completed recovery validation is `raw/regression-20261115T104116.151350Z`.
All ten recorded commands pass. Storage contributes **183 tests**, including
**30 VirtIO tests**; kernel host tests contribute **212**. SunFS contributes
**54 tests and 13,142 validated crash schedules per profile**: 3,491 SFS-2,
6,691 SFS-3 and 2,960 SFS-4. Every schedule reports PASS and selects an expected
outcome; the debug and release corpus files have identical hashes.

The release suite and remaining checks ran after recovery. The debug command
completed earlier in this milestone, in
`raw/regression-20261115T083526.956696Z`, before that full sweep later failed
during release evidence generation with ENOSPC. Its complete debug log/corpus
are explicitly reused, not presented as a post-clean rerun. The complete
current source inventory matches its starting inventory; `debug-reuse.json`
records that qualification. Both the completed recovery sweep and final native
sweep have PASS input-integrity records with no changed inputs.

Fresh earlier-milestone native replays on this same frozen source also pass:
12 SFS-5B.1 normal/cold-restart boots (including SFS-5A checks), six SFS-5B.2
reset/ABA boots, and 18 SFS-5B.3 held-request boots. These retain their original
milestone scopes and emulator identities. They are not relabeled as asynchronous
memory-overlap or new-epoch tests. The sandbox-denied QMP attempts remain saved
alongside the successful bounded replay.

Default and diagnostic kernel builds, no_std checking, scoped Rust formatting,
scoped Clippy and whitespace checks pass. Existing IPC naming and manifest
warnings remain in their logs. The final diagnostic ELF hash is still
`879ea1d4326c157df8434e9b313b82883d663bfc97ef304ce1e1670665ca683d`,
identical to the guest used for all 42 final native boots. Native compilation
records additionally retain 448 file hashes, 17 directory-watch markers, the
build-script output and fingerprints. These are source/dependency and effective
binary records, not a hermetic toolchain attestation.

| Gate | Verdict | Qualified scope or missing obligation |
| --- | --- | --- |
| G1 | PASS | Incoming inventory, declared transport and source identities |
| G2 | PARTIAL | Safe deferred/event-order backend; no concurrent memory worker |
| G3 | PARTIAL | O1/O2 observed; O3/O4 UNQUALIFIED |
| G4 | PASS | Independent guest/device ordering witnesses within the declared scope |
| G5 | UNQUALIFIED | General asynchronous/backend/platform retirement fence missing |
| G6 | PASS | Required quarantine retained; zero DMA reclamation |
| G7 | PARTIAL | Scheduled coordinator subset; involuntary exit/full race coverage absent |
| G8 | PARTIAL | Real S4/S8/S10 contention; remaining native cuts missing |
| G9 | PARTIAL | Available regressions separate from incomplete fault matrix and prior unexplained stall |
| G10 | PASS | Required artifacts, preservation audit and final evidence/source seal |

The [24-case fault matrix](../../audit/sunfs/sfs-5b4/native-fault-matrix.json)
records expected and observed interleavings, mechanism, actual memory-access
scope, reset type, witnesses, resource accounting, failure behavior and final
state. An available assertion passing does not qualify a missing race. Native
reset withholding/removal, original late backend completions, actual failed/
uncertain flush reset cuts, conditional retirement and new-epoch I/O remain
NOT_RUN or partial as recorded. The original uncertain write is never converted
into durable success by reset acknowledgment. Hosted failed-write/flush
provenance is kept separate from native evidence.

Development attempts are retained individually, including unsafe-link archive
extraction refusal, the fixture that published process records without enqueuing
them, a build-before-emulator-ready attempt, both staged-API red tests, and
pre-repair validation sweeps. Review also found an unrecorded positive mapping
read after the v1 emulator's acknowledgment marker. That observer gap is retained
as FAIL with its binary, patch and source copies. The corrected emulator moves
and records that control before acknowledgment; all native cases are replayed
against its new binary/patch identity. Input-integrity failures from edits during those
sweeps remain failures. A later passing run does not retroactively qualify their
inputs. The final source is sealed with exact copies of changed source, source
manifests, compilation/dependency records and all evidence hashes. No historical
SFS result is presented as freshly executed unless a new raw run exists.

At the user's request, unrelated native C, embedded, Yazi, FastLane and old R2B Cargo
build caches under `target/` were removed during validation. Available space
increased by approximately 8.7GiB across both deletions. SFS namespaces, active build targets, source
trees and earlier evidence/reproducer logs were retained. The exact removed
paths and measured space changes are in `raw/target-cleanup-summary.json` and
its two referenced records.

The user subsequently ran `cargo clean` after system hangs. `target/` was
absent on resumption, and temporary emulator trees were also absent. Disk free
space was approximately 60GiB. Raw evidence, patches and earlier failures
survived. Recovery restores the pinned emulator archive and unchanged v2 patch
and limits compilation to one job. The early boot stall is not attributed to
ENOSPC without evidence; failed and interrupted attempts remain excluded.
The first recovered kernel build also records a missing-embedded-binary failure.
The 23 baseline prerequisites were then restored from existing sources using
recorded commands; no application implementation or unrelated build-system
repair was introduced. A post-recovery inventory confirms all 11,879 recorded
source/configuration inputs match the completed debug run's starting inventory.

Reproduce with `CARGO_BUILD_JOBS=1 RUST_TEST_THREADS=1 python3 tools/sunfs5b4_validate.py native`
and the same environment for `regression`, after building the recorded isolated
QEMU. The recorded recovery sweep uses `raw/resume-regressions.py` to identify
the explicitly reused debug command and execute all remaining checks sequentially. Recompute derived classifications with
`python3 docs/audit/sunfs/sfs-5b4/derive.py`; verify the final seal with
`python3 docs/audit/sunfs/sfs-5b4/verify-integrity.py`.

## Remaining work

Recommended **SFS-5B.5**: review an AioContext/block-backend access-lifetime design
that can expose a true asynchronous memory operation; force the missing native
completion, timeout, IRQ and involuntary-exit cuts; and qualify mapping lifetime
and exactly-once allocator/descriptor accounting before retirement. A larger
threaded test model must carry a reviewed mapping and callback ownership proof,
not borrow freed request structures or manufacture unsynchronized host races.

Until then retain terminal quarantine and closed admission. Keep perfect-forgery
limitations and trusted-device assumptions explicit. Physical DMA fencing,
hardware persistence, native writable SunFS and all unresolved block-contract
boundaries remain separate gates. There was no native SunFS mount, physical
formatting, boot/root migration, CPython/SunApp work, unrelated kernel redesign,
commit or push.
