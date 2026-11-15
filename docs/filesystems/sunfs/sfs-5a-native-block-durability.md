# SFS-5A — Native block durability, DMA ownership and fenced I/O

**PARTIAL. Native SunFS writable admission remains BLOCKED.** This milestone
establishes a written native I/O admission contract, repairs a reproduced VirtIO
request-reuse defect, and executes bounded native write/flush/crash/restart tests.
It does not establish an authoritative runtime DMA fence. No SunFS mount, physical
formatting, boot/root migration, scheduler/IPC refactor, CPython work, commit or
push occurred. Physical durability is UNQUALIFIED.

The local qualification date is November 15, 2026 (Asia/Tehran); the raw UTC attempt
names start on November 14. Incoming revision was
`22a5947f5fbd4ffe810aee5efee57b0ec18f8915`, branch `main`, with a clean incoming
worktree. `source-baseline.json` binds incoming tracked/unignored regular files,
including all previous SFS evidence. The final integrity seal checks the allowed
changes and verifies that previous evidence is unchanged. SFS-0 through SFS-4,
the architecture/recovery/VFS contracts, storage reports and kernel progress and
retirement reports were inspected; `raw/references.json` records their hashes.
Their historical passes are not promoted to current native results.

All evidence below is relative to [the evidence directory](../../audit/sunfs/sfs-5a/).
The machine-readable verdict and gates are in `validation-summary.json`.

| Gate | Verdict | Evidence / remaining boundary |
| --- | --- | --- |
| G1 source/inventory | PASS | source-baseline.json; backend-capability-matrix.json |
| G2 block contract | PASS | block-io-contract.md; existing SunFS block types can express D/P/R/R0 |
| G3 VirtIO flush/ordering | PARTIAL | Real negotiated flush and normal ordered byte fixture executed; unsupported/failed flush injection absent |
| G4 DMA/request ownership | PARTIAL | Timeout quarantine repaired; native delayed-DMA, allocation-fit and reusable-generation qualification incomplete |
| G5 timeout/fencing | UNQUALIFIED | No authoritative runtime fence; retain resources and stop submission |
| G6 disposable guest tests | PARTIAL | Twelve successful boots; required fault cases and concurrent AP execution absent |
| G7 native durability | PARTIAL | Exact final cut after SIGKILL and restart; intermediate/failing barrier crash cuts absent |
| G8 security/progress | PARTIAL | Bounded source review completed; queue geometry and lock-context findings remain |
| G9 regressions | PARTIAL | Fresh hosted/storage/kernel checks recorded below; native lifecycle/fault regressions absent |
| G10 integrity/evidence | PASS | source-integrity.json and raw/verify-integrity.py |

Backend capabilities are independent. VERIFIED in the full JSON matrix means the
stated source/model behavior, not physical or native certification.

| Backend | Geometry/alignment | Flush / FUA | Cache / ordering | Partial errors / timeout | DMA / fence | SFS-5A guest |
| --- | --- | --- | --- | --- | --- | --- |
| Legacy VirtIO Block | PARTIAL: 512-byte transfers, capacity only; physical size UNKNOWN | PARTIAL real flush / MISSING FUA | Cache mode UNKNOWN in driver; serialized transfers PARTIAL | Prefix MISSING; bounded poll/quarantine PARTIAL | Pinned bounce/ring PARTIAL; fence MISSING | PARTIAL |
| USB MSC over xHCI/BOT | PARTIAL; READ CAPACITY10 enforces 512; physical UNKNOWN | PARTIAL sync-cache / MISSING FUA | Device cache UNKNOWN; serialized BOT PARTIAL | BOT residue checked, block Io loses provenance; TSC deadline PARTIAL | Pinned/dead transport PARTIAL; fence UNQUALIFIED | NOT_RUN |
| RAM MemDisk | VERIFIED checked slice range and 512-byte sectors | NOT_APPLICABLE persistent flush/FUA | Synchronous volatile copy VERIFIED | Whole-sector bounds VERIFIED | NOT_APPLICABLE | NOT_APPLICABLE |
| Hosted image | VERIFIED checked DeviceInfo, 512-byte alignment, bounded file | PARTIAL sync_all / MISSING FUA | Host cache chain PARTIAL | Uncertain writes PARTIAL; syscall timeout UNKNOWN | NOT_APPLICABLE | NOT_APPLICABLE |
| Hosted simulator | VERIFIED modeled geometry | VERIFIED modeled flush; no native claim | VERIFIED volatile/durable and reordering model | VERIFIED modeled faults | VERIFIED modeled session fence only | NOT_APPLICABLE |
| AHCI/NVMe/ATA | MISSING implementation in inspected paths | NOT_APPLICABLE | NOT_APPLICABLE | NOT_APPLICABLE | NOT_APPLICABLE | NOT_APPLICABLE |

The full matrix separately records logical/physical sectors, transfer alignment,
identity, flush, FUA, write cache, request ordering, partial completion, timeout,
DMA ownership, reset/fencing and native qualification. RAM lifetime and simulator
session reset must not be described as native persistent-device behavior.

The [native contract](../../audit/sunfs/sfs-5a/block-io-contract.md) reuses
`sunlight-sunfs/src/block.rs`: `DeviceInfo`, `WriteCompletion`, `IoFailure`, known
prefix and certainty already exist. The native `sunlight-block` trait supplies
512-byte operations, flush and capacity, but lacks the full metadata and error
provenance. No speculative replacement API or native adapter was added. Device
identity/incarnation must be pinned through each operation. A completed write is
not durable. Reads expose a complete validated result; a partial/uncertain write
freezes the volume. Unsupported flush forbids writable admission. Offline status
does not resolve old write uncertainty. FUA is optional and unused.

For D/P/R/R0, complete dependency transfers and acknowledge flush D before writing
the checkpoint; acknowledge P before claiming the checkpoint durable. Complete
retirement zeroing and acknowledge R before releasing the old closure, subject to
last pins and request quiescence. On recovery of three checkpoints, R0 must durably
empty the oldest slot before reuse. Failed/uncertain barriers preserve ownership
and stop mutation; a failed R does not prove P was absent. Submission, execution,
completion and persistence order remain distinct. Partition flush covers the
parent device. The write-back cache drains dirty slots before calling its inner
barrier; cache acceptance/readback and eviction alone are not persistence.

The actual VirtIO path is future adapter → KernelDisk/cache → partition view →
KernelBlkDev/VIRTIO_BLK mutex → VirtioBlk → legacy split ring → PMM bounce page →
used ring/status → caller. `sunlight-virtio/src/blk.rs` negotiates FLUSH bit 9,
submits request type 4 with a header and writable status descriptor, waits for the
used entry and checks status. `KernelBlkDev` and `VirtioBootDisk` return Unsupported
when the feature is absent and propagate false as Io. There is no successful
no-op flush. Completion uses polling rather than a block interrupt handler. CPU
fences govern coherent DMA visibility and cannot replace the storage command.
Capacity is read once, write-cache/topology/GET_ID are not exposed, and feature
negotiation is broader than an explicit supported-feature allowlist.

The reproduced production defect was timeout followed by reuse of the same request
page/descriptor head. `raw/ownership-before.log` retains two failing tests. The
first overwrote a pending write's DMA bytes; the second let a late used entry
alter a newly submitted request's queue. The repair is one terminal `uncertain`
latch, checked before any subsequent write copy or submission. Timeout, unsolicited
used entries and invalid used-index/head combinations quarantine the driver. Valid
completion requires exactly the next used index and head 0. The two tests now pass.
Host tests suppress only port notification and shorten the polling budget; they
are fake-ring ownership tests, not actual device late-DMA races. Normal native
I/O/flush/restart tests execute the production port-I/O path. The other code changes
are an off-by-default `sfs5a-test` kernel feature and its fixture/runner.

PMM allocates four contiguous ring pages and one request page. The device sees the
bounce page, never the caller's stack/slice. The CPU prepares data/descriptors,
publishes the available index with barriers and notifies the device. A valid used
entry returns ownership for serialized reuse. Timeout means
TIMED_OUT_UNCERTAIN; allocations and descriptors stay pinned and no future request
is admitted. FENCING/FENCED/RETIRED are unavailable transitions in this driver.
Dropping the handle does not stop DMA or free those PMM frames. Native kernel
ownership keeps them for the kernel lifetime; retention is deliberately not a
successful fence. USB similarly pins controller allocations and marks timed-out
transport dead; BOT reset is not a general DMA fence. `dma-ownership.json` and
`timeout-fencing.json` detail every transition and unavailable operation.

The test ISO attaches exactly one new exclusive regular 4 MiB image, identified by
capacity, a fixed test-only content marker and QEMU-configured serial. The guest
does not query the hardware serial. The fixture verifies the marker before writes
and stops before FAT/VFS probing or AP startup. No boot/user data disk or host
physical device is attached. Exact whole-image checks cover all write targets and
all unchanged bytes. Each invocation records QEMU/cache configuration, source
hashes, ELF/ISO hashes and logs; compressed image bytes are retained losslessly.

| Configured vCPUs | cache=writeback initial / restart | cache=none initial / restart | Execution boundary |
| --- | --- | --- | --- |
| 1 | PASS / PASS | PASS / PASS | BSP only |
| 2 | PASS / PASS | PASS / PASS | BSP only, AP startup not reached |
| 4 | PASS / PASS | PASS / PASS | BSP only, AP startup not reached |

These are twelve boots across six images, not twelve CPUs or an SMP concurrency
proof. Each initial guest executes one-sector round-trip, eight sequential sector
transfers, real flush acknowledgments around D/P/R/R0-shaped byte patterns, bounds
rejection and sixteen write/flush/read cycles. The host SIGKILLs QEMU after the final
flush marker, checks the complete image and cold-boots the same image for exact
readback. The expected durable-state set is the one fully acknowledged final cut.
The fixture does not contain SunFS records or run its recovery engine. It does not
cut power to the host or physical device, or crash between protocol barriers.

B2, B12 and B13 PASS within those bounds. B1/B3/B4/B6/B9/B14 are PARTIAL for missing
metadata, single-sector API, normal-only barriers, bounds-only errors and absence
of native wrap/fault cuts. B5/B7/B8/B11 are NOT_RUN; B10 is UNQUALIFIED. The first
native attempt omitted VirtIO RNG and halted at secure bootstrap before block I/O.
Its original logs/failure rows and interrupted runner are retained, not replaced
by the later passing attempt. This preserves entropy/capability fail-closed policy.

Level 1 protocol correctness and Level 2 virtual-device durability are PARTIAL:
normal barriers and an acknowledged final-cut restart are evidenced, while failed
flush, unsupported capability, real timeout/late completion, malformed completion,
reset and removal schedules remain absent. Level 3 physical durability remains
UNQUALIFIED. QEMU successful flush cannot certify the complete persistence chain.

The security review found no new capability grants or weakened retirement policy.
The timeout repair preserves caller buffer isolation and bounded fail-stop behavior.
Open findings include fixed ring allocation with no queue-size-fit guard (the exact
arithmetic requires 16,390 bytes for 512 entries versus a 16,384-byte allocation),
non-stable capacity discovery, lost error provenance, absent hotplug incarnation and
runtime fence, and no IOMMU containment claim. The allocation counterexample is
source arithmetic; unsafe out-of-bounds device DMA was not deliberately executed.
Qualification stays restricted to the recorded default QEMU queue configuration.

Device polling is finite but holds the device spin mutex, and existing VFS/lifecycle
paths can retain outer locks. Hardware DMA can complete independently of maskable
interrupts, but that does not establish scheduler latency, absence of all dependency
cycles or concurrent teardown safety. No scheduler/PMM ownership policy was changed.
An appropriate native lifecycle test remains NOT_RUN. See `security-review.json`.

Fresh final compiled-source checks passed 54 SunFS tests in each debug/release
profile and 13,142 crash schedules per profile (SFS-2: 3,491; SFS-3: 6,691;
SFS-4: 2,960). The two profile crash corpora are equal and every selected recovery
state belongs to its expected set. Existing block/FAT/VFS tests passed 139 tests;
the two new VirtIO tests make 141 in that combined run. Kernel host tests passed
212 tests. The supplemental standalone fake-ring run passed four tests, including
the same two ownership tests and two additional unsupported/unsolicited-entry
checks; these are not four new unique production tests. Default and diagnostic
kernel builds, no_std checks, scoped formatting, Python compilation and whitespace
checks passed. Native USB, runtime FAT mount/partition lifecycle replay and native
timeout/reset/concurrent lifecycle regressions were NOT_RUN; G9 stays PARTIAL.

The hosted runner's final broad source-manifest assertion FAILED: while the hosted
suites ran, the runner script was corrected to supply VirtIO RNG for the native
retry. Both manifests and the original runner are preserved. Their only difference
is `tools/sunfs5a_validate.py`; every compiled Rust/Cargo/build input matches its
before/after and final hash. Every hosted test/check command returned zero. The
native retry independently passed its unchanged full source-manifest check. This
scoped verification does not erase the hosted harness failure; see
`regression-results.json` and `raw/hosted-input-integrity.json`.

Before any native SunFS integration, the recommended SFS-5B is **native fence and
fault qualification**: first validate queue allocation/geometry and capability
negotiation, preserve typed error/uncertainty and media incarnation, establish a
specification-backed device fence with actual reset/late-DMA evidence, and qualify
submission outside scheduler-critical locks. Exercise unsupported/failed flush,
IOERR, timeout/late completion, duplicate/stale used entries, queue wrap, removal
where supported, and D/P/R/R0 crashes at every barrier cut. Run on active 1/2/4 CPU
kernels with independent progress witnesses. Keep resources quarantined whenever a
fence cannot be proven. Complete these remaining SFS-5A gates before scheduling a
separate disposable SunFS VFS mount milestone.

Modified production files are `sunlight-virtio/src/blk.rs`, `kernel/src/main.rs`
(feature-gated hook only), and `kernel/Cargo.toml` (off-by-default feature). New files
are `kernel/src/sfs5a_test.rs`, `tools/sunfs5a_validate.py`, this report and the evidence
directory. Nothing was committed or pushed. To verify the sealed deliverables:
`python3 docs/audit/sunfs/sfs-5a/raw/verify-integrity.py`.
