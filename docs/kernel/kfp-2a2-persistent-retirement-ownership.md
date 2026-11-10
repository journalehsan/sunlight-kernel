# KFP-2A.2 — Persistent retirement ownership and guest fault accounting

## Verdict and scope

**PARTIAL. Production runtime behavior changed.** Anonymous `munmap` now reserves
static journal capacity before its first destructive mapping transition. Exact
frame/swap identities remain in the journal during failed synchronization;
fail-stop remains mandatory. This is a scoped implementation, not closure of
all VM/SHM/exit/quota obligations. No automatic retry, background reclamation,
commit, push, capability relaxation, or production FastLane enablement.

The authoritative run results are `.kfp-2a2-evidence/results.json`, normal guest
results, fault results, and their raw logs. Historical KFP-2A.1 passes are not
counted as current-source results. This document's generated validation section
below records the actual completed runs, including any failed attempts.

## Source baseline and prior failures

HEAD and SHA-256 hashes are captured in `source-before.json`. `status-before.txt`
and `preexisting.patch` preserve the pre-existing LAPIC/TLB bounded-send changes,
the MM-2D security fixture correction, and previous untracked reports/evidence.
The frozen build manifest is `source-build.json`; ELF/ISO hashes are recorded per
run. `source-integrity.json` checks the final tracked inputs against both captures.
The previous milestone's results and the KFP-1/KFP-2A/KFP-2A.1 reports were reviewed.

Existing limitations include incomplete activation/epoch and paging-structure
cache proofs, serializer fairness, absent recovery, and unexecuted full native
FastLane M1–M5 security matrices. KFP-2A's original SHM fixture prerequisite panic
is preserved in its original evidence. It is not a new safety finding here.

An initial build began before the new diagnostic Cargo feature was declared and
failed against stale feature metadata; `build.log` preserves this invalid build
attempt. `build-retry.log` is an intermediate build, not the frozen guest artifact.
Only `build-final.log` and per-FI builds are used for guest evidence. Dependency
warnings remain in logs. No expected markers or security assertions were removed.

## True retirement boundary and inventory

The complete ten-field transition inventory is
`.kfp-2a2-evidence/ownership-inventory.json`. Important distinctions:

| Path | Authoritative backing owner / transition | Remaining boundary |
|---|---|---|
| Anonymous munmap | VM leaf → static journal → PMM/ZRAM after synchronization | Implemented for frame/swap backing; whole-request capacity reserved first |
| CoW/swap replacement | Old leaf overwritten; expected mapping remains local | Pending old-frame operation still stack-owned |
| Single-page unmap | Leaf cleared; physical address local until synchronous return | No durable per-view operation added |
| SHM view retirement | Broker still owns frames/quota; process still owns view metadata | Persistent backing is not a persistent pending invalidation receipt |
| Final SHM backing | Broker entry removed → `ReclaimedShared` | Stack-owned frame vector can lose identifiable ownership on drop |
| Forced revoke / restricted FastLane | Retired row and broker retained until every actual view drains | View operation journal and final-backing handoff remain open |
| Mapping rollback | Caller owns new frame; VM owns tables | Withdrawal and table addresses can remain local |
| Empty table reclaim | VM hierarchy owns tables until parent unlink | Parent-withdrawal/cache lifetime proof not closed |
| Exit / exec | Process or retired AddressSpace owns root and user hierarchy | New journal blocks root reclaim; no comprehensive old-image journal |
| Permission withdrawal / kernel VMM | Existing backing remains allocated / local-only mapper helper | No generalized SMP permission or kernel-mapping recovery proof |

`ReclaimedShared` deliberately prevents automatic quota refund with
`ManuallyDrop`, and dropping a physical-frame descriptor does not free the PMM
frame. Nevertheless dropping the vector can lose the exact backing list: that
is a retained allocation without a durable identifiable retirement owner. The
new anonymous journal must not be advertised as fixing this separate handoff.
A broker view/reference count reaching zero is valid only after the actual view
shootdowns; removal from a mapping ledger alone never authorizes PMM reuse.

## Implemented model and ownership transitions

`memory/retirement.rs` contains an allocation-free table with eight slots, each
holding up to 32 exact entries. A record identifies physical root, VM generation,
VA, physical frame or swap block, shootdown sequence, target and received masks,
completion state, failure reason/CPU, and per-entry release receipts. Anonymous
munmap has no SHM capability/session or shared-quota reservation to transfer;
those fields are deliberately absent. VM-owned tables remain attached to the
root until successful leaf synchronization; the root-reclaim guard retains that
owner while journal obligations exist. Exact detached table identities are not
added to the journal in this patch.

1. `reserve`: obtain one slot for the **whole request**, before any chunk changes
   a PTE. Root/generation and same-VM checks prevent overlapping journal ownership.
   Capacity or generation exhaustion returns `MmapError::NoMemory`; valid
   mappings remain installed. No later chunk needs another slot allocation.
2. `stage`: copy all exact identities for a bounded chunk into static storage
   before its first leaf withdrawal. The local array is a snapshot, not a second
   authority. Existing scheduler/PMM exclusion prevents mapping changes between
   validation and withdrawal.
3. Clear the staged leaves, mark `Withdrawn`, and enter the existing serialized
   shootdown. Before any remote delivery, bind the nonzero checked sequence and
   actual active CPU target mask to the journal.
4. `Pending` → `Synchronized` only after local invalidation and all exact-sequence
   remote ACKs. A one-CPU/no-target operation completes with zero remote bits.
5. Release backing/swap slots, mark each release once, reclaim eligible tables,
   then clear the completed chunk while retaining the reserved slot. Release the
   slot after the last chunk. The mapping ledger then commits the completed plan.
6. Delivery failure or timeout records observed ACKs and the exact failed
   operation in `Quarantined`, then takes the existing nonreturning panic path.
   Quarantined records reject late completion, release, reuse, and finalization.

Tokens are non-Clone locators qualified by a monotonically checked slot generation.
Losing or dropping a token does not remove its record. There is no token `Drop`
reclamation, retry API, journal scan that frees frames, or continuation policy.
Per-entry release receipts prevent state-machine double-finalization; PMM calls
are still executed by the existing synchronous munmap path. There is no claim of
crash-consistent atomicity across a PMM call and its following receipt update.
A panic in that interval is terminal and must never be replayed as recovery.

`reclaim_user_space` refuses a VM with any journal record, including a prepared
record abandoned before its first withdrawal. This is conservative containment,
not an exit routine that can make progress after failure. The mechanism persists
in kernel memory across stack/owner termination attempts, not across reboot.

## Production and diagnostic changes

Production: `memory/retirement.rs`, its module declaration, `process/mmap.rs`,
`memory/tlb.rs`, and `process/address_space.rs`. Normal munmap now has a bounded
capacity failure path, metadata locking, and explicit lifetime receipts. Existing
LAPIC failure semantics and terminal panic behavior remain.

Diagnostic only: Cargo feature `kfp2a2_fault_test`, additional LAPIC polling
instrumentation, PMM bitmap/event observations, and pre-stop receipts. Controls
require both `debug_assertions` and this explicit feature. The compile-time
`KFP2A2_FAULT` value is part of each build receipt. Release/default production
builds contain no active injection. The host library includes the actual journal
module rather than a reimplementation. Existing FastLane native entry gates in
`arch/x86_64/syscall.rs` remain conditional on their original debug features;
this feature does not enable them.

## Failure containment and independent observation

FI-1 forces the busy status bit in the actual production polling loop, after a
journal-owned remote retirement starts. The helper must exhaust exactly 4096
reads and return `NotSent`. It does not emulate a physical APIC becoming stuck.
FI-2 injects a delivery `NotSent` outcome after real PTE withdrawal and operation
publication, before ICR writes. FI-3 suppresses the expected handler ACK after
mailbox claim/flush, with a diagnostic 100,000-scan budget. The production missing
ACK budget remains 100,000,000 scans. None returns successful retirement.

Before deliberate stop, the observer snapshots the static journal, then checks
exact backing identities against the PMM allocation bitmap and packed owner map.
A separate PMM-side per-frame observer counts `free_frame` calls and subsequent
`claim_frame` reuse after registration, independently of journal release flags.
The munmap fixture also checks/prints actual PTE absence. The parser requires
matching withdrawn VAs, nonzero remote targets, quarantine state, allocated
backing, zero releases/reuses, complete receipts, and the expected terminal
failure. A missing receipt, watchdog, unrelated panic, or success marker fails
the test. Captures occur while the initiator retains PMM exclusion; they do not
reacquire a lock held by the stopped CPU.

This observer covers one serialized diagnostic munmap chunk. It is not a global
allocator census, does not observe every detached page-table identity, and does
not exercise concurrent guest cleanup or quota handoff. A retained frame with a
journal owner is distinguished from a lost vector allocation. No post-stop
allocator progress or successful recovery is inferred from any receipt.

## Locking and forward progress

Existing order is SCHEDULER → PMM → optional CAP_BROKER → SHOOTDOWN_SERIALIZER.
The new OWNERS metadata lock is acquired/released in short bounded sections:
slot scan (8), entry staging (32), one receipt update, or state transition. TLB
binding/quarantine adds SERIALIZER → OWNERS; VM root reclaim adds PMM → OWNERS.
No OWNERS critical section calls PMM, scheduler, broker, shootdown, serial output,
or an allocator. There is no OWNERS → outer-lock edge. No new lock is held over
remote ACK waits. Remote fixed/NMI handlers never acquire OWNERS.

There are no new allocations in IRQ-off, nonpreemptible, or spinlocked paths.
Munmap still runs under existing exclusion and cannot yield during synchronization.
The existing serializer itself still covers remote waits and has unproved fairness;
this patch does not add a second operation-wide global lock. Panic keeps existing
outer guards held. Full-system halt, fairness, bounded MMIO wall time, epoch retry
termination, and a whole-kernel lock-order proof remain unproved.

## Performance observations

Journal preparation performs no heap allocation. Capacity is static, not grown
under locks. The host reports actual `size_of` values in `retirement-host.log`:
record 1088 bytes; table (eight records plus generations) 8776 bytes, excluding
mutex overhead. Munmap also stages bounded stack snapshots and walks expected
leaf state again before clearing; successful retirement adds short metadata
critical sections and per-release receipts. Exact costs are not assumed free.
No new lock spans the remote wait. No before/after latency distribution, lock-hold
sample, preparation timing, or reclamation throughput measurement was collected.
Guest boot/fixture seconds are not shootdown latency.

## Remaining recovery and completion requirements

Integrate a distinct pending-view obligation with existing broker backing/quota
ownership; persist final `ReclaimedShared` handoff, CoW/swap replacement,
rollback, and old-exec-image ownership before their destructive transitions.
Do not duplicate SHM physical ownership in each view record. Include capability,
session, recipient incarnation/VM generation and exactly-once quota obligations
where needed. Add exact table identities and close parent-unlink/cache lifetime
and activation overlap proofs. Then execute guest FI-4 through FI-7, real
concurrent retirement, cleanup/refund interleavings, and full applicable native
M1–M5 security/lifecycle matrices. Existing host protocol controls cannot replace
those guests. Any future continuation also needs a separately proved quiescence
and exclusion protocol; persistent metadata alone grants no permission to resume.

## Executed validation receipts

Baseline HEAD: `882b40a65f55951b5cd8b9978c05580e71202488`. Frozen source drift: `[]`.

Host: journal 9/9; kernel library 194/194; actual extracted ICR 5/5;
actual mailbox 10/10; ring/doorbell/IPC/UAC 163/163; scheduler controls PASS.
FastLane parser controls: 137 passed, 67 skipped, not native security execution.
The concurrent journal test uses real host threads and overlapping target sets;
it does not execute simultaneous guest VMM retirement. Quota tests belong to
the existing host broker suite; no new guest quota proof is inferred.

| CPUs | Normal MM-2D | FI-1 | FI-2 | FI-3 |
|---:|---|---|---|---|
| 1 | PASS | N/A: local-only normal gate | N/A: local-only normal gate | N/A: local-only normal gate |
| 2 | PASS | PASS | PASS | PASS |
| 4 | PASS | PASS | PASS | PASS |
| 12 | PASS | PASS | PASS | PASS |

Fault PASS means complete retention receipts followed by the expected panic,
never successful retirement or recovery. The one-CPU row has no fabricated
remote target or ACK. Guest FI-4 (delayed), FI-5 (stale/duplicate), FI-6
(concurrent), and FI-7 (cleanup following failure) are NOT_RUN. Corresponding
host controls have narrower protocol/state-machine scope.

| Case/CPUs | VM / sequence / targets | Retained physical frames | Release / reuse calls |
|---|---|---|---|
| FI-2 / 2 | 14 / 94 / 2 | 274268160 | 0/0 |
| FI-2 / 4 | 14 / 94 / 14 | 274268160 | 0/0 |
| FI-2 / 12 | 14 / 94 / 4094 | 274268160 | 0/0 |
| FI-1 / 2 | 14 / 94 / 2 | 274268160 | 0/0 |
| FI-1 / 4 | 14 / 94 / 14 | 274268160 | 0/0 |
| FI-1 / 12 | 14 / 94 / 4094 | 274268160 | 0/0 |
| FI-3 / 2 | 14 / 94 / 2 | 274268160 | 0/0 |
| FI-3 / 4 | 14 / 94 / 14 | 274268160 | 0/0 |
| FI-3 / 12 | 14 / 94 / 4094 | 274268160 | 0/0 |

Each identity is qualified by its own boot, frozen artifact, root/VM generation
and shootdown sequence in `fault-results.json`. Equal physical addresses across
boots are separate allocations. FI-1 receipts additionally require exactly
4096 forced-busy reads. FI-3 records MissingAck quarantine; late ACKs cannot
complete that journal state. Guest delayed/stale attempts were not performed.

Default production cargo check: PASS. Journal preparation allocates nothing; static table is 8776 bytes excluding mutex. Latency and throughput distributions were not measured.

Applicable current-source regressions executed: MM-2D normal matrix, kernel
host VM/broker/SHM/quota controls, mailbox/ICR controls, IPC/ring/doorbell/UAC
libraries, scheduler state controls, and parser controls. Full native SHM forced
revocation, IPC mapping closure, FastLane M1–M5 authentication/authorization,
and scheduler guest/performance matrices remain NOT_RUN. No critical violation
was observed in the executed guest cases; this is not an exhaustive safety proof.

**Final verdict: PARTIAL.** The implemented anonymous retirement preserves a
durable identifiable owner and retained backing on injected failure. Coverage
of every unfinished resource obligation, guest FI-4–FI-7, independent table and
quota failure accounting, and applicable native security regressions is incomplete.
Recommended next milestone: KFP-2A.3, persistent SHM/view/final-backing and old-image
handoffs with real cleanup/concurrent/stale-ACK guests, retaining fail-stop until
a separate safe-quiescence proof exists. Production runtime changed; production
FastLane remains disabled; no commit or push occurred.
