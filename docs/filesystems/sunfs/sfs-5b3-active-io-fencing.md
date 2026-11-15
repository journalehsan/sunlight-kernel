# SFS-5B.3 — Active I/O fencing and independent DMA observation

**PARTIAL. SFS-5B.3 is not COMPLETE. Active-DMA reset, guest DMA retirement,
new-epoch recovery and native scheduler contention remain unqualified. Native
writable SunFS remains BLOCKED.** This milestone establishes a reproducible,
independently instrumented native outstanding-request reset experiment and
preserves the existing production fail-stop behavior.

The scope is native SunlightOS x86_64 in QEMU 11.1.2/TCG using legacy PCI
VirtIO Block `1af4:1001`, split queue 0 and coherent guest RAM. The device's
actual payload is partially consumed, then held before backend submission.
Reset occurs between memory-copy instructions. This is real request and memory
access evidence, but not a simultaneous asynchronous DMA engine/reset experiment.
No physical-hardware fencing, physical durability or hostile-device containment
is established.

Work began on local November 15, 2026, Asia/Tehran; attempt names use UTC
November 14. Incoming branch `main`, revision
`22a5947f5fbd4ffe810aee5efee57b0ec18f8915`, already had the uncommitted
SFS-5A/SFS-5B.1/SFS-5B.2 work. The baseline captures 387,151 tracked/unignored
regular files, the exact worktree status and incoming tracked patch. Previous
SFS evidence and incoming unrelated changes are preserved byte-for-byte. There
was no commit, push, SunFS mount, physical formatting, boot/root migration,
CPython or SunApp change.

All evidence links below are relative to
[the evidence directory](../../audit/sunfs/sfs-5b3/). The authoritative ledger is
[validation-summary.json](../../audit/sunfs/sfs-5b3/validation-summary.json).

| Gate | Result | Qualification boundary |
| --- | --- | --- |
| G1 baseline, transport and threat model | PASS | Exact baseline, official specification and recorded runtime features |
| G2 real active-I/O backend | PASS | Actual submitted descriptor, partial real payload copy, deterministic hold and release control |
| G3 independent I/O and DMA witnesses | PARTIAL | Driver and separate device-side access witnesses; concurrent active DMA/reset remains UNQUALIFIED |
| G4 active-I/O reset | PARTIAL | Reset with a genuine outstanding request; no overlapping memory instruction or full native negative matrix |
| G5 authoritative fence contract | PARTIAL | Conforming-device normative boundary and scoped guarded QEMU cancellation; general reuse unqualified |
| G6 DMA retirement | UNQUALIFIED | Zero guest pages reclaimed; no allocator reuse proof |
| G7 safe terminal outcome | PASS | Closed admission, bounded reset observations, unchanged uncertain owner, retained resources |
| G8 active-SMP contention | PARTIAL | Real AP startup and timer progress; S1–S8 and scheduler-ready progress NOT_RUN |
| G9 final-source regressions | PARTIAL | Available regressions pass; mandatory full native fault/recovery/contention matrix incomplete |
| G10 integrity and blockers | PASS | Sealed inputs, outputs, prior evidence and unsuccessful attempts |

The PASS for G7 means terminal fail-stop behavior only. It does not authorize
queue recovery, resource reclamation or reuse.

The [transport/threat model](../../audit/sunfs/sfs-5b3/transport-threat-model.md)
separates compliant devices, implementation defects, malformed completions,
adversarial devices/hypervisors and software ownership defects. The applicable
reference is official OASIS VirtIO 1.4 CS01 dated April 8, 2026, retained with
its source hash. Sections 2.4.1/2.4.2 define reset acknowledgment and subsequent
queue-interaction restrictions; 3.3/3.3.1 govern exposed live-queue buffers.
Legacy initialization/status provisions are identified separately from modern
queue reset in the [normative contract](../../audit/sunfs/sfs-5b3/normative-fencing-contract.md).
A reset acknowledgment does not establish media persistence or physical posted
DMA drain. The driver negotiates no RING_RESET, VERSION_1 or packed-ring feature.
It accepts `0x240`; offered values vary with the exact QEMU profile and are
recorded per invocation in `instrumented-backend.json`. The legacy queue-size
fit guard remains unchanged; the numeric queue size and runtime BAR port were
not newly printed, so neither is invented here.

If an adversarial device can forge a completion perfectly indistinguishable
from a legitimate completion, the driver cannot identify it using information
absent from the wire protocol. The split used-ring head does not authenticate
a software request generation. The preserved same-head/new-status ABA fixture
continues to document this limit; its successful regression assertion is not a
repaired adversarial identity guarantee.

The [backend](../../audit/sunfs/sfs-5b3/instrumented-backend.json) is built from
the exact official QEMU 11.1.2 archive in `/tmp`, with an isolated two-file patch.
No system emulator is replaced. Archive/source/binary/patch hashes, configure
output, compiler/tool versions and build logs are retained. `rebuild-qemu.sh`
applies the patch in a new directory; it performs no installation. Each guest
invocation records its actual emulator, ELF, ISO, runner, source and image hashes,
argv, cache setting and test environment. This is an exact input record, not a
hermetic or bit-reproducible-build attestation.

Only a block device with serial `SFS5B3-disposable`, a 4MiB marked regular-file
image and sector-63 512-byte OUT request enters the hook. The guest checks image
capacity and the old disposable marker before the test write. The hook logs the
real queue/head, consumes the first 256 bytes of the actual mapped data
scatter-gather vector into the buffer used by its later backend write, and holds
the real `VirtIOBlockReq`. The remaining 256-byte read, backend operation, status
write, used publication and interrupt notification have not happened. An explicit
release control consumes the rest, submits those same 512 bytes to the real
backend, completes, flushes and reads the sector back. Unmodified-QEMU controls
execute the normal driver path against separate images. A timer by itself is
not the DMA witness: the actual payload copy and real release-control data path
supply that evidence.

Witness A is the native driver: attachment incarnation 1, queue incarnation 1,
write generation 2, descriptor head 0, Deadline/Uncertain receipt, quarantined
owner, reset initiation, status-zero readback, retained first uncertainty and
refused new admission. Witness B is the separate QEMU process: descriptor pop,
request/sector handling, actual 256-byte payload read with value `0xb3`, outstanding
partial transfer, reset entry, canceled timer, element detach/unmap and cleared
queues/status. Raw traces independently show that the held request never publishes
a used entry. Whole-image comparison confirms that the held operation never
reaches disk; release controls show actual byte transfer. The custom `guest=`
address is a host RAM mapping pointer, not an independently printed guest GPA.

The final QEMU revision installs `PROT_NONE` on the original host mappings of
all four old queue pages and the request page after generic reset clears queue
state. A nonblocking kernel pipe-copy probe reads the original payload before
protection, then must fail with EFAULT on each of the five protected pages. The
guest subsequently observes reset acknowledgment, refuses new I/O and survives
a fixed half-second host observation window with those mappings protected.
This can catch access through the original mappings, including uninstrumented
pointer access. It does not prevent aliases/remapping by a hostile hypervisor,
prove an asynchronous DMA worker was running at reset, or prove safe allocator
reuse. These limits are explicit in `dma-access-witness.json`.

Reset cancellation is a test-model behavior: with BQL/main-context serialization,
it cancels the held-request timer, restores sliced iovecs, detaches/unmaps the
element and frees that device-side object before reset returns. The normal QEMU
reset path drains backend work and clears queues. Source review includes
`virtio-pci.c`, `virtio.c`, `virtio-blk.c`, `virtio-bus.c`, `block-backend.c` and
timer deletion. A detached device-side object is not reclaimed guest DMA memory.
No reset API or free path was added to the production driver.

The terminal driver protocol remains: close admission and retain the current
owner, write status zero once, observe at most 64 status reads, retain all pages
for every terminal outcome, and refuse all subsequent I/O. Repeated begin/poll
calls cannot restart the budget, erase uncertainty or reopen the queue. Each
qualified native held request acknowledges in one read. Delayed/withheld and
unexpected-status behavior is covered only by fresh hosted fixtures. The bounds
are software work bounds, conditional on port instructions completing; no hard
physical wall-clock bound is claimed.

The [ownership/accounting ledger](../../audit/sunfs/sfs-5b3/quarantine-accounting.json)
retains four queue pages and one header/data/status page: **20,480 bytes per
attachment**. One uncertain request record remains owned by that old attachment.
Native PMM free counts are sampled before the request and after reset; all
held-request samples are unchanged. These samples are independent allocator
observations, but they do not demonstrate individual frame retirement. Zero
pages are released, zero mappings transfer to a new epoch and zero uncertain
request authorities retire. One serialized attachment admits at most one
outstanding request. Repeated terminal calls allocate nothing. Global quarantine
capacity and multi-attachment pressure are unqualified; existing initialization
allocation exhaustion may halt rather than recover. No infinite retry/recovery
loop was introduced.

There are **18 final guarded-revision native boots**: six held-request resets,
six instrumented release controls and six unmodified-QEMU baselines across
1/2/4 vCPUs and `cache=writeback`/`cache=none`. Their scoped assertions pass;
`native-fault-matrix.json` separately classifies N1–N15. N1–N5 pass within the
stated partial-transfer scope. N6 is PARTIAL, N7 UNQUALIFIED, N8–N12 NOT_RUN,
N13 PARTIAL and N14–N15 NOT_RUN. A control write before reset is never called
fresh post-reset/new-epoch I/O. H1–H22 likewise retain missing or model-only
interleavings explicitly, rather than manufacturing device evidence from fake
rings. Reproduce with `python3 tools/sunfs5b3_validate.py native` after rebuilding
the isolated emulator at the recorded path.

APs really start before submission. Every held-request run has increasing timer
counters on every configured CPU during the outstanding request. The only new
timer hook performs an off-by-default atomic counter increment before scheduler
readiness checks. The fixture explicitly prints `SCHEDULER_READY=false`.
Consequently these are CPU/interrupt progress witnesses, not scheduler fairness,
independent reset-context or active contention proofs. I/O and reset both execute
on the BSP through an exclusive mutable driver owner. The synchronous API holds
that ownership throughout polling; another reset context cannot safely acquire
it mid-call. A staged/coordinated interface and a post-scheduler fixture must be
qualified before claiming S1–S8. No unsafe shared mutable owner or broad scheduler
change was introduced to force a passing race.

The original uncertain write remains `Completion::Uncertain`, `Amount::Unknown`
and `Durability::Uncertain` through reset and later refusal. Attachment, queue
and request identity survive. The held image remaining unchanged follows from
this particular pre-backend cut, not a general guarantee of side-effect-free
timeouts. Failed/unsupported write/flush and uncertain flush provenance remain
covered by hosted driver fixtures; native uncertain/failed-flush reset is NOT_RUN
here. Reset acknowledgment is never transformed into durable success. Typed
USB/cache/partition/FAT/VFS propagation and authoritative legacy short-transfer
semantics remain separate gates. Future SunFS recovery must still validate
complete durable checkpoint closures.

Fresh available regressions pass on the final Rust/runner source: **54 SunFS
tests per debug/release profile, 178 storage/backend tests and 212 kernel host
tests**. The existing crash matrix produces **13,142 schedules per profile**
(SFS-2: 3,491; SFS-3: 6,691; SFS-4: 2,960), with passing selections and identical
debug/release corpora. Default and SFS-5B.3 diagnostic kernel builds, relevant
no_std checks, scoped Rust formatting, block/VirtIO Clippy, Python syntax and
whitespace checks pass. These are available regression results, not the missing
full native fault, recovery and contention matrix. Exact commands, counts,
corpus hashes and integrity assertions are in `regression-results.json`.

The first 18 native boots used the initial patch without mapped-page protection;
those passes remain limited to that scope. A subsequent guarded attempt failed
its `process_vm_readv` negative-probe assertion and was interrupted after three
recorded cases. The native errno was not logged. A separate standalone probe
returns EPERM for that syscall and EFAULT for a protected-page pipe copy; this
comparison does not retroactively identify the native errno. Its patch, identity,
logs and incomplete results are preserved. The final guard uses bounded pipe-copy
positive/negative controls instead. The failed attempt is not erased, counted as
PASS or given an input-integrity PASS. `raw/development-attempts.json` identifies
all three revisions. Only the final complete matrix qualifies the guarded claim.

Recommended **SFS-5B.4**: qualify a genuine asynchronous DMA/reset interleaving,
retain independent access guards, establish a post-scheduler staged reset
coordinator, force S1–S8 and native negative schedules, then prove exact guest
resource retirement before considering a new epoch. Keep native writable SunFS
blocked until those gates and the remaining block-contract boundaries close.

This milestone changes only feature declarations/hooks in `kernel/Cargo.toml`,
`kernel/src/main.rs` and `kernel/src/arch/x86_64/interrupts.rs`; it adds
`kernel/src/sfs5b3_test.rs`, `tools/sunfs5b3_validate.py`, this report and the
SFS-5B.3 evidence directory. All other incoming production edits are prior user
work. To verify the seal and preservation of incoming files, run
`python3 docs/audit/sunfs/sfs-5b3/verify-integrity.py`.
