# SunFS SFS-5B.1 — Native block contract and request identity

**PARTIAL. Native writable SunFS remains BLOCKED.** The direct VirtIO path now returns
explicit receipts, validates geometry and queue allocation, and tracks a generation-qualified
logical owner. Terminal quarantine is preserved. Essential wire-identity and integration
limits remain, so this is not SFS-5B.1 COMPLETE.

Work was performed locally on November 15, 2026, Asia/Tehran; raw attempt names use UTC.
Incoming branch `main`, revision `22a5947f5fbd4ffe810aee5efee57b0ec18f8915`, already contained
uncommitted SFS-5A production, fixture, runner and evidence changes. No commit or push,
SunFS mount, physical formatting, root/boot migration, CPython or SunApp changes occurred.

Evidence lives in [sfs-5b1](../../audit/sunfs/sfs-5b1/). The authoritative gate ledger is
[validation-summary.json](../../audit/sunfs/sfs-5b1/validation-summary.json).

| Gate | Result | Boundary |
| --- | --- | --- |
| G1 baseline/inventory | PASS | Full incoming file hashes, status, patch, incoming production/runner copies and source audit |
| G2 geometry/capabilities | PASS | Direct backends report qualified values or Unknown/Unsupported; incompatible legacy wrappers refuse the new query |
| G3 typed semantics | PARTIAL | Direct VirtIO receipts and kernel bridge implemented; USB and cache/partition/FAT/VFS migration incomplete |
| G4 request identity | PARTIAL | Software token/generation/resource validation passes; legacy used-ring head cannot echo a generation |
| G5 DMA/quarantine | PASS | Bounded fail-stop ownership; old SFS-5A retention tests preserved and rerun; no reclamation/reset path added |
| G6 VirtIO completion/flush | PARTIAL | Native success plus hosted failures; legacy length cannot independently establish short transfer |
| G7 negative tests | PARTIAL | T1–T20 scoped evidence; wire reuse, short-count and actual removal boundaries remain |
| G8 native scope | PARTIAL | Twelve normal boots plus four native negative cases; timeout/stale/removal faults absent; normal I/O on BSP before AP startup |
| G9 regressions/builds | PASS | Fresh debug/release SunFS, storage, kernel host, no_std and production/diagnostic builds |
| G10 final integrity | PASS | Final source/output seal and independent previous-evidence comparison |

G5 is restricted to preserving terminal quarantine and logical retirement of a conforming
completed request. It does not certify adversarial wire identity, delayed real DMA,
removal/shutdown handling or authoritative fencing. Those limits remain under G4/G7/G8.

## Baseline and source integrity

`source-baseline.json` hashes 279,473 incoming tracked/unignored regular files, including
all prior SFS evidence. Symlink metadata, ignored build artifacts and Git internals are
outside this manifest. `raw/incoming.patch` and `raw/incoming/` preserve SFS-5A production
and runner bytes. Inventory and exact toolchain/build/reference hashes are recorded in
`raw/source-inventory.json`. Final changes are compared to this incoming baseline, rather
than attributing the existing SFS-5A work to this milestone.

The SFS-5A hosted runner's changed RNG argument remains a recorded failed broad integrity
assertion. Its earlier logs, before/after manifests and repaired/failing ownership tests
are untouched. This runner includes `virtio-rng-pci,disable-modern=on` from the outset and
records exact argv, environment overrides, toolchain versions, feature, source hashes,
ELF/ISO hashes and QEMU cache mode. Native boot security entropy is not replaced with a
fixed test entropy source. `0x5b1001` is a scenario label, not a PRNG seed. Negative cases
use deterministic tuples without randomness; hosted SFS-1/2/3/4 seeds are separately
recorded. Documentation and audit outputs are excluded from each execution-input
manifest so they can be written during testing; the final seal includes the deliverables.

`raw/build-inputs.json` also records the native kernel depfile's 445 regular-file inputs,
17 directory dependencies, five Limine artifacts and executed ELF/ISO hashes. This extra
capture happened after the native build and before the default build; it is explicitly
not a before-build or hermetic-build attestation. The per-invocation ELF/ISO identities
bind the actual executed images independently of that retrospective input inventory.

## Geometry and backend capabilities

| Backend | Geometry/identity | Persistence and limits |
| --- | --- | --- |
| Legacy VirtIO | 512-byte protocol sectors, checked capacity bytes, one-block/512-byte requests; physical size Unknown; unique boot-local attachment incarnation | Negotiated FLUSH, no FUA; write cache Unknown, conservatively treated as volatile; fencing Unsupported |
| USB BOT/SCSI | READ CAPACITY10 last LBA + 1; only 512 accepted; READ16 sentinel refused; one-block/512-byte limit; physical size Unknown; unique attachment incarnation | Read-only, cache and flush support Unknown; Synchronize Cache10 attempted; transport reset is not a DMA fence |
| MemDisk | Slice length/512, checked offsets, one-block/512-byte API; physical size Unknown; instance incarnation | Synchronous volatile copy; persistent flush/FUA Unsupported; legacy flush provides RAM visibility only |
| Hosted image | Existing SunFS DeviceInfo: 512-byte sectors/alignment, max 4096-byte transfer, checked exclusive regular-file size; physical size Unknown | `File::sync_all` under the host filesystem/storage assumptions; no physical durability guarantee or bounded syscall deadline |
| Cache/partition/KernelDisk | New geometry query refuses with Unsupported | Their acceptance, parent barrier scope and typed propagation require a deliberate adapter |

The backend enum is a diagnostic identity; incarnation is not a persistent media serial.
The configured QEMU serial is recorded by the runner but not read from the device.
No persistent identity is inferred from PCI location or registry device index. USB errors
permanently retire the current object; runtime VirtIO media change/reset requalification
is not implemented. No AHCI/NVMe implementation was fabricated or modified.

`Geometry::{validate,check}` cover nonzero/power-of-two block/alignment values, optional
physical size, checked capacity bytes, LBA+count, offset/length multiplication, bounds,
zero and maximum transfers, exact buffer length and pointer/offset alignment. Typed
VirtIO read/write signatures enforce one full block and validate before touching DMA
state. Buffer alignment is one because the transport bounces through its own pinned page.
A future SunFS adapter must also require its own format, capability and incarnation
conditions; this metadata alone cannot admit a writable mount.

## Receipts, identity and ownership

The [typed contract](../../audit/sunfs/sfs-5b1/typed-io-contract.md) describes submission,
completion, failure, partial representation, uncertainty, refusal, unavailability and
virtual barriers separately. Each direct VirtIO receipt preserves identity, operation,
LBA/count/bytes, known amount, status source and durability scope. Device status failure
retains Unknown transferred amount; failed writes/flushes have uncertain durability.
Successful write completion is not durable, successful readback is not a persistence
barrier, and failed flush does not prove prior writes were absent.

The narrow legacy bridge returns explicit `BlockError::Uncertain`, `Unavailable`,
`Failed` and `Unsupported`, and never succeeds for ambiguous receipts. Kernel VirtIO
adapters use it. USB runtime faults now distinguish uncertainty from subsequent permanent
unavailability, but have no complete typed per-command provenance. FAT/VFS still collapse
errors; there is no completed native SunFS adapter through them.

A `RequestId` contains backend+attachment incarnation, immutable queue incarnation,
checked request generation and owned descriptor head. For this serialized queue, the
generation also serves as descriptor ownership epoch and submission sequence; duplicate
counters would add no authority. `Owner` enforces Created → Prepared → Submitted →
Completed → Retired and exactly-once retirement. Complete checks active identity, device,
queue, state and actual descriptor addresses/lengths/directions/links. Timeout records a
transient TimedOutUncertain and enters terminal Quarantined, retaining the active token.
FencingPending/Fenced are unavailable transitions. Reinitializing a live/quarantined
attachment violates the unsafe initialization contract.

Software old-token, wrong-media, wrong-queue, duplicate and generation-exhaustion tests
pass. `u64` identity counters refuse exhaustion; `u16` protocol indices may wrap and are
not identities. Queue recreation is not exposed. Tests cover ring wrap separately.

**Essential G4 limit:** a legacy device returns only a descriptor head and used length.
After legitimate retirement and head reuse, a forged old completion arriving after the
new publication is indistinguishable from the new completion. The regression
`reused_wire_head_has_no_generation_echo_demonstrated_limit` demonstrates that acceptance.
The software token model rejects stale tokens, but the device never echoes such a token.
No decorative generation field is claimed to solve this wire limitation. A broader
transport/reuse/fencing design is needed to satisfy the requested adversarial guarantee.

Kernel PMM keeps four queue frames plus one request bounce frame pinned for the kernel
lifetime. Requests never DMA into caller slices. After timeout, malformed index/head,
invalid status or descriptor ownership mismatch, the queue latches quarantine before any
future request buffer copy or descriptor publication. Late completion never reopens it.
No frame-free, reset, retry, quarantine eviction or capacity inflation was added. Logical
request retirement after a valid terminal completion permits serialized reuse; it does
not free the five PMM frames. Hosted stack pages are fake DMA resources and are labeled so.

## VirtIO integration and flush

Feature acceptance is now an explicit SIZE_MAX/RO/BLK_SIZE/FLUSH allowlist. Initialization
samples capacity with a bounded repeated read, rejects zero/byte overflow, refuses an
advertised non-512 block profile and an insufficient SIZE_MAX, validates queue power-of-two
and minimum size, and checks the entire descriptor/available/used-ring extent before
publishing the PFN. A 256-entry queue fits; a 512-entry ring needs 16,390 bytes and is
refused rather than overflowing the 16,384-byte allocation.

Request header remains type/reserved-zero/sector, with read-only header, appropriately
directed 512-byte data descriptor and status initialized to 0xff. Flush is the real
negotiated type-4 header/status chain with no data descriptor. Completion requires exactly
one next used index, head zero, expected owned descriptors and status 0/1/2 before
retirement. Coherent x86 memory fences bracket queue publication and completion reads.
These fences do not establish storage durability.

Used length is retained for diagnostics. OASIS VirtIO 1.2 sections 2.7.8.1 and 5.2.6.3
recommend ignoring legacy used lengths. The nearby source comment references 5.2.6.2;
the precise legacy exception is 5.2.6.3. The driver therefore does not fabricate a partial
prefix from this field or impose a false short-count guarantee. Full success follows
validated status semantics, assuming a conforming device. Host tests demonstrate the
length limitation; native successful reads reported used length 513.

Hosted tests execute unsupported, failed and uncertain flush paths. Native tests execute
real successful flushes, whose receipt identifies the preceding queue generation. A separate
QEMU blkdebug `flush_to_disk` EIO case returns typed Failed at the flush assertion; full
post-error durability/ownership fields are not printed by that fixture.
A VirtualBarrier covers the recorded virtual device under `cache=writeback` or `cache=none`
and the host flush chain. It is not physical storage power-loss qualification.

## Negative and native evidence

`negative-tests.json` has T1–T20 rows with injection, expected/observed result, ownership,
retirement, actual-DMA flag, source/runner identity and deterministic seed scope.

| Cases | Evidence scope | Result |
| --- | --- | --- |
| T1–T4 | Hosted geometry and pre-submission arithmetic/alignment/length validation | PASS |
| T5 | USB hosted short decoding; legacy VirtIO length/status limitation | PARTIAL |
| T6–T10 | Hosted fake-ring failed/uncertain I/O and unsupported/failed/uncertain flush | PASS |
| T11–T16 | Hosted late/duplicate/token mismatch/invalid head and ownership | PASS within recorded model/fake-ring scope |
| T17 | Old software token refused; reused wire head can still alias | PARTIAL |
| T18–T19 | Original quarantine regressions, counter exhaustion and separate ring wrap | PASS |
| T20 | Outstanding ownership retained on modeled removal/shutdown | PARTIAL; actual hot-unplug NOT_RUN |

No negative test counts as real DMA fencing. Four supplemental 1-vCPU native cases use
the existing final ISO: a queue-size512 guard, a logical-block-size4096 guard, a read-only
write rejection and a QEMU blkdebug flush EIO. All four produced their expected markers.
The first two refuse initialization and all three refusal images remain byte-identical.
The read-only case executes the marker read before rejecting the write. The flush case
executes a completed sector60 write then returns typed Failed at fixture line20. Its
compressed image exactly contains the marker plus the prior sector60 bytes, demonstrating
write visibility after failed flush without a durability claim. Expected assertion panics
are retained, with exact harness/command/config/source/ISO identities. Native ownership
retirement after that error is not directly printed and is not promoted from hosted tests.
Native unsupported flush, late DMA, malformed completion, timeout quarantine and actual
device removal remain NOT_RUN.

| Configured vCPUs | writeback initial/restart | none initial/restart | Execution |
| --- | --- | --- | --- |
| 1 | PASS / PASS | PASS / PASS | BSP before AP startup |
| 2 | PASS / PASS | PASS / PASS | BSP before AP startup |
| 4 | PASS / PASS | PASS / PASS | BSP before AP startup |

Twelve normal boots on six exclusive disposable 4MiB images execute the final driver and new
typed fixture, then repeat the unchanged SFS-5A barrier/crash-cut fixture. New writes are
restricted to sector60 after capacity and content-marker checks. Each initial run ends
with host SIGKILL after an acknowledged final cut, a whole-image expected-byte comparison,
and cold restart with another exact whole-image comparison. Logs, QEMU commands, cache
modes, manifests and lossless compressed images are retained. These cuts do not run SunFS
records or the recovery engine. They do not exercise intermediate barrier failures.

The hook runs at `init_block_and_fat`, before `smp::start_aps`, and halts there. Configured
CPU count, AP startup, I/O after AP startup, secondary initiation and concurrent I/O have
separate fields in `smp-scope.json`. The latter four are NOT_RUN; concurrent qualification
can remain SFS-5B.3 work. Native USB persistence remains UNQUALIFIED.

## Progress, security and remaining work

VirtIO waits remain bounded at 50,000,000 polling iterations, versus16 in fake-ring tests.
Capacity sampling is bounded at8 tries. No new allocation occurs per operation and no
hidden retry is introduced. A poisoned device visibly refuses requests while retaining
20KiB of pinned DMA frames. Existing PMM allocation failures remain explicit panic/refusal;
no allocator or capacity policy was changed to force progress.

Polling holds the device mutex; VFS/lifecycle callers can retain the outer VFS lock.
This does not prove scheduler latency, contention fairness or safe concurrent teardown.
There are no new scheduler/PMM lock acquisitions in the submit path and no new capability
grants, raw-device ABI, IOMMU isolation claim or relaxed memory-safety assertion. Invalid
used heads never index arbitrary host descriptors. Source review and hosted lifecycle
models are distinct from native progress evidence.

The final regression and integrity artifacts record only fresh executions. Historical
SFS-5A failures remain preserved, and no historical test is promoted to a final-source
pass. Two evidence-deriver syntax failures and an ad-hoc corpus checker selecting the
runner log instead of the attempt directory are retained under raw development/checker
artifacts. The corrected checker verified every selected recovery state and byte-identical
debug/release corpora; those tooling mistakes are not erased or labeled suite failures.
Fresh final-source results are 54 SunFS tests in each debug/release profile, with
13,142 deterministic crash/reordering schedules per profile (SFS-2:3491, SFS-3:6691,
SFS-4:2960). The totals include 98 explicit reordering rows; the fault-boundary-only
count is13,044. Every selected recovery state belongs to its expected set, and both
profiles produced byte-identical schedule corpora. Storage/FAT/VFS/VirtIO passed168
tests, including43 block tests and15 VirtIO tests. Kernel host lifecycle/security
models passed212 tests. Default and `sfs5b1-test` kernel builds and no_std checks
passed. Scoped Rust formatting, Python compilation and whitespace checks passed;
whole-existing-main.rs formatting and native FAT lifecycle replay are not claimed.
Existing unrelated workspace manifest/IPC naming warnings remain in the build logs.

All three execution-input checks passed with identical source/runner snapshots:
normal native, supplemental native negative and hosted. The final seal includes the
report, all16 required artifacts, raw evidence and final source copies, and verifies
prior SFS evidence byte-for-byte against the incoming baseline. Recheck with
`python3 docs/audit/sunfs/sfs-5b1/derive.py --check`. Regeneration uses `derive.py`
followed by `--seal`; changing output files after sealing requires a new seal.

SFS-5B.2 should first design a specification-backed quiescence/fence receipt binding
attachment, queue and all outstanding DMA resources, then exercise actual reset/removal,
late DMA and allocation reuse with independent progress witnesses. Generation validation
must remain separate from DMA authority and unresolved media side effects. Retain terminal
quarantine whenever the fence cannot be proven. Resolve legacy wire-identity and complete
typed USB/wrapper admission before native SunFS mounting. SFS-5B.3 can qualify I/O after
AP startup and concurrent secondary-CPU submission without weakening those gates.

Modified production paths are `sunlight-block/src/{contract,lib,usb_msc}.rs`,
`sunlight-virtio/{Cargo.toml,src/blk.rs}`, `kernel/src/main.rs` and `Cargo.lock`.
The new production dependency is the existing internal `sunlight-block` crate; there
is no new external crate. The off-by-default test feature is in `kernel/Cargo.toml`;
the new fixture and runner are
`kernel/src/sfs5b1_test.rs` and `tools/sunfs5b1_validate.py`. Exact deltas and hashes are in
`production-changes.json`; incoming SFS-5A changes remain separate. No commit or push.
