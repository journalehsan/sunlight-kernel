# FastLane M2: restricted native mappings

2026-11-01, Asia/Tehran. **M2 COMPLETE for restricted native mapping only.** Production
FastLane remains disabled. This milestone has no ring transfer, reference-based
payload API, userspace Doorbell, delegation, resizing or reconnect operation.
M1's independent Account authorization and immutable attachment remain required.
No commit or push is authorized.

## Memory contract and permission matrix

The layout has three independent, page-aligned, 4096-byte regions. Each is one
ordinary SHM backing object; its backing token stays kernel-private. Returned
addresses need not be adjacent or equal between participants. There is no shared
header interpreted by the kernel and no userspace pointer retained by it.
In each region, offsets 0 through 4095 are opaque userspace bytes with the rights
below; there is no kernel-owned prefix, hidden control word or writable padding
shared by the two roles. New backing is zeroed before its first view is exposed;
a later view observes the same backing, including any peer modifications. M2
defines the complete page-access layout, while structured counter fields and
their access semantics belong to the next representation milestone.

| Region index | Future purpose | Producer | Consumer | Executable |
|---|---|---|---|---|
| 0 | Producer publication state | Read/write | Read | Never |
| 1 | Consumer reclamation state | Read | Read/write | Never |
| 2 | Payload storage | Read/write | Read | Never |

No physical page may be writable by both roles. The read-only view is an
observation of an adversary-controlled page: its contents can change at any
instruction because the other role retains write permission. Read-only mapping
is **not** integrity, validation, immutability, ownership of a Rust reference,
or permission to trust lengths, offsets, counters or discriminants. All bytes
on all three pages are untrusted. M2 uses single assembly loads/stores in fixtures;
it constructs no shared slices, references or ring objects from these bytes.
A future payload milestone needs a separate hostile-memory representation audit.

The kernel alone owns channel token/generation, issuer/session identity,
producer/consumer PID/incarnation/VM generation, attachment and mapping flags,
terminal state and first reason, SHM tokens, frame identities, view counts,
software mapping ledger, PTE permissions, TLB epochs/requests/ACKs and allocator
ownership. No userspace byte can change these values. Root alone, token possession
alone and legacy bearer SHM authority cannot grant this mapping authority.

## Admission, bounds and rollback

The private debug selector remains `0xf1c1`. Only `fastlane_mapping_test` adds
operation 8 (`arg = role * 3 + region`); its result is one page VA and size.
Operation 9 reports a debug-only restricted-object inventory. Ordinary images
have no selector and default features remain empty. Account's separate
`fastlane-mapping-test` feature includes M1's actual requester policy; it adds
no trusted participant or authentication bypass.

Every map authenticates the current process and exclusive native VM, sweeps
expired/dead/replaced/session-retired channels, checks the exact channel token
and generation, checks the exact immutable recipient, and requires that role's
successful attachment. Wrong role, foreign caller, invalid region, stale or
terminal handle and duplicate view fail before mapping. Each role may obtain
one view of each region. A failed operation consumes no mapping slot and may be
retried. There is no API to request another target process, arbitrary physical
frame, writable flag or backing token.

Eight channel slots bound backing to 24 data frames and 48 views. Allocation
uses existing token-history, broker, region-ledger and page-table limits.
Mapping-tracking storage, token preparation and backing-vector storage use
fallible reservation. A new frame is sanitized using the existing user-frame
sanitizer. PTE/ledger installation uses the existing transactional shared mapper
with a kernel-selected read-only or read/write protection. Publication follows
successful installation. On mapping failure, unpublished backing returns to the
PMM; an earlier successfully mapped region remains owned and usable. Prepared
token history stays intentionally burned under the existing boot limit.

`ShmEntry::restricted` denies legacy resolution and validation even with the
exact internal SHM token. Explicit legacy free also refuses restricted entries.
The shared owner-managed ledger rejects mprotect, anonymous munmap and fixed
anonymous replacement. Ordinary process/VM cloning exclusions from M1 persist;
a retired row with undrained backing still counts as VM membership and cannot
be recycled. Native mapping never exports a physical or bearer alias.

## Withdrawal and physical retirement

No second reclamation scheme is introduced. A terminal channel first fences all
new maps. Its three objects pass through `revoke_shared`, then each existing
view passes through `free_shared_page`, including PTE/frame/ledger preflight,
actual page removal, synchronous `tlb::shootdown`, required remote handler ACKs,
ledger removal, mapping-record removal and `SharedLifetime::release`. Only the
last view transfers frame ownership to the PMM. An invariant failure stops the
kernel with backing pinned; cleanup never silently drops an uncertain view.
The channel clears each private backing identity only after the broker reports
that object gone. Repeated retirement cannot release it again.

Explicit revoke drains before returning. Successful exec drains the old local
VM through existing exec SHM cleanup and drains peer views before the exec
trampoline returns. Failed image preparation retains old roles and views.
Self-exit/fault cleanup and the reaper drain peer mappings; debug returning-syscall
and timekeeper-tick maintenance also catches logical death, identity changes,
session changes and expiry. The existing scheduler lock serializes admission
and lifecycle mutation. Locks follow `SCHEDULER -> PMM -> CAP_BROKER`; no new
mutex is introduced. When no restricted backing exists, maintenance performs
only an atomic load. Diagnostic ACK handlers use atomics without locks or output.

A zero remote target mask is distinct from a remote acknowledgement. A target
that has already left the VM can acknowledge without flushing; the context-switch
epoch protects reentry. A still-running peer scenario separately requires a
matching handler observation with `flushed=1`. No VM borrower, fabricated CPU
membership or injected ACK is used to obtain that witness.

## Native evidence protocol

`tools/fastlane_mapping_validate.py` builds and archives the experimental kernel,
ISO, extracted ELF, embedded Account and userspace fixture, executable source
map and commands. It first runs the complete authenticated M1 fixture, then
launches `FL-M2` from the same real authenticated shell. The M1 assessor sees
only its own prefix, so expected M2 faults cannot hide a regression in M1.

Each hostile access occurs in a fresh disposable independently authorized VM.
It announces the VA and actual load/store before executing it. The normal page
fault handler records PID/incarnation/VM/CR2/error bits and terminates it normally;
the separate supervisor collects exit status 139. There is no expected-fault
recovery, instruction skip or supervisor impersonation. Permission faults require
hardware error 7; stale loads/stores require errors 4/6. Unannounced faults fail
assessment. Separate producer/consumer cases check allowed reads/writes, full-page
zero initialization, forbidden writes, and mprotect/replacement/unmap refusal.

Additional cases cover foreign and duplicate mapping, pre-attachment rejection,
revoked/stale handles, failed/successful exec, a real anonymous VA collision after
one successful region, a running remote peer, and six same-guest lifecycle cycles
alternating revoke and peer exit. Each cycle also exercises real channel-row
reuse and rejects the retained old handle. Census and per-object traces reconcile
restricted objects/views/frames; global SHM totals are compared across cycles.
This is a channel-specific resource proof, not a claim of whole-system heap
stability. Retained process/broker capacity and token-history nonreuse are separate.

`[FL-BIND]` authenticates participants. `[FL-MAP]` identifies generation, region,
role, PID/incarnation/VM, VA, backing frame, write/NX rights and unmap boundaries.
`[FL-TLB]` gives request sequence, target mask, independently recorded handler
ACK/flush observations and completion. `[FL-FRAME]` records real PMM final free
and observed normal reuse. `[FL-ROLLBACK]` reconciles the allocator before/after
failed installation; `[FL-CENSUS]` records ownership inventory. Raw tokens and
receipts are never logged. `*.physical-objects.json` groups these records and
requires every view's matching TLB completion before its frame's unique free.

## Validation and next step

All eight M2 acceptance criteria pass on the unchanged final source. Authenticated
peers obtain the fixed role-restricted views; actual forbidden stores and stale
loads/stores fault in disposable VMs; foreign, unattached, duplicate, stale and
revoked admissions fail. Peer exit, successful exec and revoke withdraw both
participants' views. Every backing lifetime has a unique final physical free
after all required unmaps and remote acknowledgements. Repeated lifecycle
censuses show no frame drift, the requested regressions pass, and production
FastLane remains disabled.

The matrix below comprises **33 accepted native QEMU guests**, eleven per CPU
configuration, on x86_64 TCG with multiple execution threads. PASS requires the
suite's actual workload and serial assessment; authenticated suites also retain
their original independent host acknowledgements.

| Native suite | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| M1 authenticated authority and all four attach/revoke/actual-exit orders, followed by M2 hostile mappings | PASS | PASS | PASS |
| IPC reply/lifecycle races, VM/SHM preflight and rollback, 32 retirement cycles, remote TLB ACK gates | PASS | PASS | PASS |
| Account S1/S2 authority contention, two cases per topology | PASS | PASS | PASS |
| Account failed/successful exec and physical retirement, three cycles per topology | PASS | PASS | PASS |
| Completion before actual caller exit, mode 11, three cycles per topology | PASS | PASS | PASS |
| Actual caller exit before completion, mode 12, three cycles per topology | PASS | PASS | PASS |
| Concurrent completion/exit entry, mode 13, three cycles per topology | PASS | PASS | PASS |
| Scheduler SMP progress and accounting | PASS | PASS | PASS |
| R2D remote wake and physical pin-release handoff | PASS | PASS | PASS |
| Existing kernel-native Doorbell forced races | PASS | PASS | PASS |

The generic VM/SHM suite independently exercises remote ACK masks with 1, 3 and
11 remote CPUs respectively. A channel VM excludes borrowers; its dedicated
mapping traces instead distinguish the real running peer target from zero-target
unmaps. These are different witnesses and are not substituted for one another.

Final host/artifact checks:

| Check | Result |
|---|---|
| Kernel host tests, including M1 and restricted mapping authority/provenance | PASS, 153 |
| IPC host tests | PASS, 62 |
| Account host tests | PASS, 18 |
| Focused scheduler production state/lifecycle/accounting/policy tests | PASS |
| All FastLane parser tests | PASS, 178, including 21 M2 rejection controls |
| Ordinary release kernel check | PASS |
| Ordinary ELF, extracted ISO ELF and embedded service exclusion | PASS; no experimental features or M1/M2 markers |
| Final whitespace and executable/configuration source identity | PASS; all 1,340 inputs unchanged |

The aggregate decision and exact commands are in
`target/fastlane-mapping-evidence/validation-summary.json` and
`regressions/orchestration.json`. Under the evidence root, original regression
manifests are:

* IPC: `regressions/ipc/native-20261031T181732.762324Z/validation-summary.json`.
* Account S1/S2: `regressions/authority/native-20261101T072706.600934Z/validation-summary.json`.
* Exec: `regressions/exec/native-20261101T073104.641280Z/validation-summary.json`.
* Exit mode 11: `regressions/exit-11/native-20261101T073426.595618Z/validation-summary.json`.
* Exit mode 12: `regressions/exit-12/native-20261101T073803.059391Z/validation-summary.json`.
* Exit mode 13: `regressions/exit-13/native-20261101T074131.652484Z/validation-summary.json`.
* Scheduler/R2D/Doorbell: `regressions/{scheduler,r2d,doorbell}/results.json`.
* Host checks: `checks-20261101T074502.305234Z/validation-summary.json`.
* Production exclusion: `regressions/production/production-20261031T181425.731238Z/validation-summary.json`.

The ordinary production ELF SHA-256 is
`14e80ebfee487e4629d2c22d53c7d44dd8c3940eca0034396a3c53fabb7798aa`;
the ordinary ISO is
`221ade34c8a25fda2862a17cdcb278f3f9980fa2a306942bdd582de155152fbb`.
The final source/report archive and its verified manifest are under `final/`;
the root `artifact-index.json` records hashes for all retained evidence files. It does not
rewrite original failed results or issue replacement host acknowledgements.

**FAIL** development attempts are retained below. **BLOCKED** remains the state
of production transport and the independent historical broad security/Doorbell
gates. Ring transfer and userspace Doorbell are **NOT_IMPLEMENTED / NOT_RUN**;
bare-metal execution and unrelated historical audit exceptions are **NOT_RUN**
in this milestone. M2 closure does not promote those gates.

After M2 closure, the next smallest milestone is **M3.0: hostile-memory-safe SPSC
representation and bounded payload access**. Define a raw-byte page format and
bounded snapshot/copy operations; test adversarial counters, lengths, overflow
and concurrent mutation before enabling transfers. Keep userspace Doorbell and
production transport disabled. Existing cooperative host ring tests do not
authorize references into malicious-peer-controlled memory.

## Security audit notes

| Boundary | Implementation and evidence requirement |
|---|---|
| Authenticated channel identity | M1 real Account request/consent/receipt flow retained; mapping lookup additionally requires exact generation, role, incarnation and VM |
| Kernel authority versus hostile bytes | Mapping adapter never reads any shared-page byte; layout metadata is entirely kernel-owned |
| Shared-page integrity | Explicitly not promised; a read-only observation remains mutable by its sole authorized writer |
| Alias escalation | Restricted broker provenance, no token export, owner-managed ledger, native denied mprotect/fixed replacement/unmap followed by real protection fault |
| Frame ownership | Existing SHM `SharedLifetime`; no extra frame reference count or retirement queue |
| Terminal admission fence | Revoke/expiry/session/process/image transitions converge on terminal rows; backed tombstones cannot recycle |
| Remote translation lifetime | Actual PTE removal followed by synchronous existing shootdown and independently recorded handler ACK; per-object ordering assessed |
| Partial failure | One page per map operation; real next-VA collision after an earlier successful region; backing allocation/free and earlier mapping usability reconciled |
| Resource bounds | Eight channels, three one-frame objects per channel, two views per object; existing metadata/page-table/token-history quotas and fallible allocation |
| Fault containment | Six separate disposable participants per topology; normal hardware fault termination plus separate waitpid status |
| Production exclusion | New debug feature depends on M1; ordinary selector absent, default features empty, ELF/ISO and embedded-marker audit required |

The matrix is scoped to this fixed layout and these tested lifecycles. It does
not prove future ring index arithmetic, safe payload borrowing, notification
semantics, cancellation of in-flight payload leases or general service-heap
accounting. Normal frame allocation still sanitizes every newly exposed backing;
full-page zero reads are checked in fresh userspace mappings. A later same-frame
reuse trace is separate from that initialization test and must not be invented.

## Authenticated native receipts

Final mapping source snapshot: 1,340 executable/configuration inputs, unchanged
through the accepted executions. Relative evidence root:
`target/fastlane-mapping-evidence/`.

At task entry, all 1,334 M1 executable/configuration inputs matched their
completed manifest. All 6,749 M1 artifact-index entries verified without a
mismatch; `m1-baseline-verification.json` records that comparison. The preserved
M1 source archive supplies the baseline for the isolated M2 delta below.

| Configured CPUs | M1 authority/races | M2 mapping | Hardware faults | Backings / views retired | Remote-target unmaps / handler flush ACKs | Zero-target unmaps |
|---|---|---|---|---|---|---|
| 2 | PASS | PASS | 6 | 30 / 58 | 16 / 16 | 42 |
| 4 | PASS | PASS | 6 | 30 / 58 | 19 / 19 | 39 |
| 12 | PASS | PASS | 6 | 30 / 58 | 22 / 22 | 36 |

The 2-CPU receipt is in
`native-20261031T181114.327314Z/2cpu.host-ack.json`; 4/12 receipts are in
`native-20261031T181238.029721Z/{4,12}cpu.host-ack.json`. Each directory contains
its original serials, per-object `*.physical-objects.json`, image/source identity
and artifact index. Acknowledgements above are actual matching handler events
with `flushed=1`, not target counts or elapsed-time inference. Individual channel
VMs have no borrowers, so their remote target is the independently running peer;
the generic VM regression's larger masks are separate supporting evidence.

Each topology executed six repeated cycles with equal census values after
physical reaping (SHM frames / page-table frames / free frames):

* 2 CPUs: 1304 / 391 / 257306 for all six cycles.
* 4 CPUs: 1304 / 391 / 256774 for all six cycles.
* 12 CPUs: 1304 / 391 / 254746 for all six cycles.

Restricted objects/views/frames were zero at every completed census. All 90
backing lifetimes have exactly one final free after their 174 view unmaps
and matching TLB completion. Physical addresses can be reused across these
generation-qualified lifetimes. Normal allocator claims of recently retired frames
are recorded separately; a claim alone does not prove reexposure to userspace.
Full-page initialization checks are actual user loads on fresh mappings.

The accepted build manifest is
`build-20261031T181040.291895Z/validation-summary.json`. `M2-only.diff` isolates
this milestone from the pre-existing uncommitted M1 tree; its baseline archive
hash and changed-file inventory are in `M2-source-delta.json`.

Preserved development attempts:

* `native-20261031T180713.693699Z`: FAIL before guest execution; local QMP socket
  creation was blocked by the sandbox. Native mapping in that attempt is NOT_RUN.
* `native-20261031T180743.062484Z`: original host assessment FAIL, despite guest
  completion, because it required the early nonblocking PID diagnostic to equal
  the locked fault witness. Contended `try_current_pid` legitimately emits
  `u64::MAX`; the authoritative `[FL-PF]` identity was exact. The final assessor
  permits that early sentinel but still requires the matching hardware error,
  exact locked PID/incarnation/VM/VA and independent exit receipt. No kernel
  fault recovery or protection was changed to make this pass. This attempt also
  sampled some process resources before physical reaping; the accepted fixture
  explicitly waits for physical reaping before the resource census.
* Initial build/check iterations and two negative-control test coding failures
  remain in their original development logs; only the unchanged final source
  and original accepted host receipts above count for native acceptance.
