# T2R concurrent admission and lifecycle race closure

Status: **PARTIAL — bounded admission pilots; five native lifecycle controls remain unproven.**

Starting HEAD: `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`.
Starting canonical source SHA-256:
`18953e76a8fe73ce1ea892bf283419db329417507f58fabd83f6cd322cb69ba5`.
This matches the original T2 final ledger. The initial dirty patch, 230 changed
source-file hashes, complete canonical map, status and original T1/T2 reports
are preserved in `.threading-evidence/t2r/initial/`. Original T2 artifacts are
unchanged. No commit or push is authorized.

The exact control IDs below come from `tools/threading_t2_acceptance.py:RACES`
and the original ledger's `acceptance.missing`, rather than newly assigned IDs.

| Original ID | Requested control | Current proof |
| --- | --- | --- |
| `final_global_slot` | Two creators, last global worker slot | 2 CPUs: both winners pass |
| `final_vm_slot` | Two creators, last per-VM worker slot | 2 CPUs: both winners pass |
| `parent_exit` | Creation versus parent exit | Missing native receipt |
| `vm_teardown` | Creation versus VM teardown | Missing native receipt |
| `exec` | Creation versus legally permitted exec | Missing native receipt/classification |
| `immediate_child_exit` | Immediate exit after publication | 2 CPUs: creator-return-first order passes |
| `concurrent_reap_reuse` | Reaping/slot reuse during creation | Missing native receipt |
| `prepublication_cancel` | Cancellation before publication commit | Missing native receipt |

## Existing accepted evidence

The original report and `.threading-evidence/t2/validation-summary.json` retain
75 injected failures per ABI, three 96-worker recovery cycles, exact 3,035,136-byte
reaped baselines, and 32 native suite rows across 1/2/4/12 CPUs. T1, native std
join/detach/TLS, Hayro, production debug/release and boot are accepted historical
evidence. They do not prove the eight controls above. The original forced-race
ledger is empty and its acceptance result remains PARTIAL.

## Commit and serialization contract

`Scheduler::reserve_thread_slot` through `Scheduler::publish_thread` holds the
same existing scheduler guard. Pre-commit resources are local RAII values;
there is no persistent reservation registry. The commit adopts FD references,
installs a complete Process in exactly one selected slot, and enqueues it once.
The unreaped Process then owns the admission charge. Finished/pinned children
remain charged until physically Reaped. Allocation-free slot insertion and
queue capacity are established before commit. Native rejection is recoverable
raw -2. Linux parent-TID store ordering and T1 stack handoff remain unchanged.

A lifecycle mutation *inside* this guard interval is unreachable by construction.
That source fact does not classify the overall controls unreachable: both
external serialization orders still need native witnesses. Exec borrower
restrictions likewise need native evidence, not a fixture authority bypass.

## Pilot coordination

`thread_lifecycle_race_test` is an opt-in debug-only fixture with immutable
embedded ring-3 code and four exact PID/incarnation/VM-identity registrations.
Global contention fills 95 real workers across four VMs (32/32/15/16), with
owners of the last two VMs competing. Per-VM contention fills 29 ordinary
workers and two worker creators in one VM. Contending creators remain charged
until both results are collected. Immediate-exit mode publishes a real child
which immediately invokes the existing exit syscall.

Entry is recorded before taking the admission scheduler guard. A fixed
2,000,000-spin rendezvous holds no lifecycle/fixture/serial lock and requires
two distinct CPUs. Failure to reach it is a pilot failure, never PASS or
unreachable. There is no production hot-path lock, scheduler freeze, quota
change, or lifecycle mutation authority granted to the fixture. Ring-3 loops
and the host guest runner have finite bounds. This is a pilot design, not yet
accepted final race evidence.

## Recorded native results and reconciliation

The final strict parser and compiled-source audit accept five pilot guests:

| Control | Winner/order | Exact guest receipt |
| --- | --- | --- |
| final_global_slot | actor 1 | `20261104T221411.159453Z-pilot-global-winner1-2cpu` |
| immediate_child_exit | creator return before child exit | `20261104T221455.478314Z-pilot-immediate-winner0-2cpu` |
| final_vm_slot | actor 0 | `20261104T222024.336312Z-pilot-vm-winner0-2cpu` |
| final_vm_slot | actor 1 | `20261104T222102.165739Z-pilot-vm-winner1-2cpu` |
| final_global_slot | actor 0 | `20261104T222149.589348Z-pilot-global-winner0-2cpu` |

All are 2-CPU QEMU TCG guests. Each quota guest records two distinct CPU
entrants before taking the lifecycle guard, exactly one complete publication,
exactly one recoverable raw -2 rejection, and the generation-qualified child's
Reaped/no-stack/no-pin receipt. Independent VM owners report progress; actor
progress is excluded from the final progress count. The immediate-exit guest's
creator success report precedes child exit entry; the reverse order is missing.

| Guest kind | Warm baseline | Reaped worker stacks | Cached FD bytes | Final heap |
| --- | ---: | ---: | ---: | ---: |
| Global, each winner | 2,051,968 | 96 | 983,040 | 3,035,008 |
| Per-VM, each winner | 2,051,968 | 32 | 327,680 | 2,379,648 |
| Immediate exit | 2,051,968 | 1 | 10,240 | 2,062,208 |

The baseline is measured after four owners and bounded task/queue storage are
prepared, before any worker creation. Every remaining byte is attributed to
one retained 10,240-byte FD-table box per Reaped worker slot. The final heap
must equal that baseline plus those boxes. Owners remain live until the single
terminal receipt: four occupied tasks/stacks, 12 FD references and four original
capability entries. All four per-VM worker/borrower counts and the global worker
count are zero. Every Reaped slot has no stack or physical pin, and every queue
membership is unique. Inherited worker state is freed, not counted as cache.
Reserved/pending zero is a source-contract statement: there is no persistent
reservation registry; local transaction state cannot survive the guard scope.
It is not an independently sampled ticket counter. No aggregate balance alone
is promoted to a missing lifecycle or stale-slot reuse proof.

The T2R global baseline is 128 bytes below the historical T2 baseline.
This fixture gives each owner one environment pair; the original T2 fixture
gives each owner two. Each guest checks its own measured baseline and exact
cache bytes; the historical 3,035,136-byte value is not substituted here.

`pilot-receipt-audit.json` and `validation-summary.json` retain the exact ISO,
serial and source bindings and each guest's full child identity. Each runner
also saves the complete source hash map; final compiled inputs match all five
accepted pilot records. This does not establish hardware/topology certification.

## Host, compatibility and production checks

Admission RR/BORE, allocator/unmap failure properties, 171 kernel library tests,
scheduler host tests, descriptor tests, Linux host tests, focused native thread
and mapping APIs, and all 11 T1/T2/T2R parser tests pass. Changed Rust spans and
the complete new fixture pass rustfmt. Full-workspace formatting still reports
baseline diagnostics. Strict Cargo Clippy stops at the unchanged IPC baseline;
direct final-fixture kernel Clippy reports 155 existing diagnostics and zero
new/changed-line diagnostics. These Clippy runs are recorded FAIL with baseline
classification; they are not labelled PASS.

The 2-CPU native std/Hayro quota/join/detach/TLS and Linux clone/futex/FD/TLS gates
pass. A 2-CPU T1 hostile-stack/lifecycle gate passed before the user narrowed the
scope; no further hostile-stack run was made after that request. Later changes
are debug-fixture coordination/receipt changes, a fixture-gated placement hook,
and formatting; T1 assembly, VM retirement and production handoff logic are
unchanged. Every final pilot and the ordinary release image pass the emitted
trusted-stack ABI checker. This is not a new full final-source T1 SMP matrix.

Ordinary production release build and 2-CPU boot pass on the final implementation;
ordinary debug build is restored and passes. The release gate restores the
ordinary supervisor before building. Production fixture-exclusion results are
retained separately. No FastLane benchmark or old exhaustion matrix was rerun.
All command paths, stdout, return codes and timeouts are indexed in the ledger.
The descriptor host test automatically updated its package-version lockfile;
that unrelated generated change was restored to its exact initial bytes.

| New focused race matrix | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
| --- | --- | --- | --- | --- |
| Two quota controls | Not run | Both winners pass | Not run | Not run |
| Immediate exit | Not run | One order passes | Not run | Not run |
| Five lifecycle controls | Not run | Missing | Not run | Not run |

T2 remains **PARTIAL for the original defined security scope**. The smallest
next executable proof is a bounded two-CPU creator-versus-parent-exit fixture
with exact owner/creator/child generations and each legal serialization order.
It was deferred under this run's safer-test scope. Four other lifecycle rows,
the reverse immediate-exit order and the final SMP expansion also remain.

## Scope adjustment and limitations

The user subsequently requested safer tests and asked to skip high-security
checks. This run interprets that as omitting further hostile-stack and forced
exit/exec/teardown/cancellation testing. It continues ordinary host checks,
production validation and bounded quota/cleanup pilots. This is a reduced proof
scope, not a relaxation of the original T2 acceptance criteria.

The five remaining lifecycle controls have no dedicated native witnesses in
this run. None is classified unreachable merely because the scheduler guard
serializes construction. Concurrent owner exec with live borrowers is rejected
by `exec`'s real owner/borrower predicate, but a new authenticated native exec
rejection witness was not produced. No authority bypass was added to obtain one.
The immediate-exit fixture witnesses publication, creator success reporting,
child exit entry and physical Reaped retirement. It does not force a remote
child to exit before the creator returns. That terminal order remains unproven.

Stage 2 was not executed. Stage 3's new 1/2/4/12-CPU race matrix was not expanded:
the original prerequisite that all eight controls pass or have evidence-backed
unreachable classifications is unsatisfied. Existing T2 SMP results remain
historical accepted evidence, not substitutes for these missing controls.
No new T6 waitpid design, scheduler policy, quota policy, FastLane, desktop or
audio changes are included. No commit, push, privileged disk operation or
external-service message was performed.

## Source and ownership review

Admission rechecks the creator's exiting/cancelled/forced-exit state and full
VM root/generation under the existing scheduler guard. A Finished Linux leader
may retain a live group VM under the existing contract. Reaped slot selection
requires no watcher, queue owner, CPU owner, current-task reference or physical
stack pin. TIDs and publication incarnations cannot wrap.

`live_address_space_borrowers` counts Finished unreaped workers by full VM
identity. Native owner retirement terminates borrowers and defers VM retirement
until their physical reaping. Exec rejects nonowners and owners with borrowers.
Linux completion clearing follows physical pin/current/queue checks before
slot reuse. These are reviewed source contracts and accepted historical gates;
they are not new forced native lifecycle witnesses in this run.

## Failed and superseded attempts

Every runner command and guest lives in a new timestamped directory, retaining
argv, timeout/exit code, source map, output and frozen-image/serial hashes.
`failed-initial-build.json` records the initial helper-feature gating error.
The initial per-VM attempts hit their bounded rendezvous assertion. The decisive
snapshot records the other creator Ready, unpinned, affinity=2 but queued on
CPU 0. Changing affinity at rendezvous time left a dependency on the occupied
CPU's local queue. The fixture now chooses placement at initial publication;
actor identity follows fixed CPU placement, rather than registration arrival
order. No production queue policy changed.

One diagnostic freeze raced its pending build and captured the immediate-exit
image. Its `attempt-classification.json` marks it INVALID_WRONG_FROZEN_IMAGE.
A later prototype reached its native marker but lacked the strengthened heap
receipts; it is marked OBSOLETE_RECEIPT_SCHEMA. Early terminal printing could
repeat while QEMU exited; final completion has one claim and the parser rejects
truncated output. The fixture Clippy pass initially found a unit-valued binding;
that binding was removed. Formatting initially required module-order correction.

`pilot-receipt-audit.json` rechecks every pilot with the final strict parser,
compares serial hashes and compares compiled source inputs against the current
source. Old PASS labels, changed fixtures and missing receipts are not promoted
to final proof. Failed/obsolete attempts and original T2 artifacts are retained.

## Reproduction

Run from the repository root, one build/freeze wrapper at a time:

```sh
python3 tools/threading_t2r_pilot.py --case vm --winner 0
python3 tools/threading_t2r_pilot.py --case vm --winner 1
python3 tools/threading_t2r_pilot.py --case global --winner 0
python3 tools/threading_t2r_pilot.py --case global --winner 1
python3 tools/threading_t2r_pilot.py --case immediate --winner 0
python3 tools/threading_t2r_validate.py command host -- python3 tools/threading_t2_host.py
python3 tools/threading_t2r_validate.py command allocation -- python3 tools/threading_t2_allocation_host.py
python3 tools/threading_t2r_validate.py command parsers -- python3 -m unittest discover -s tools -p 'test_threading_t*.py'
python3 tools/threading_t2r_validate.py command formatting -- python3 tools/threading_t2r_format.py
python3 tools/threading_t2r_regressions.py production --cpus 2
python3 .threading-evidence/t2r/audit-receipts.py
```

The pilot wrapper waits for build completion, freezes ELF and ISO, runs the T1
emitted ABI check, then starts a 120-second bounded QEMU TCG guest. A failed step
stops that wrapper. Source/image/receipt identities are retained; synthetic
parser tests never count as native proof. The race helper adds no production
hot-path lock. Its fixed entry/winner waits hold no lifecycle, fixture or serial
lock. It never waits for an actor stranded on the same initial queue by changing
affinity at the rendezvous. Failure to obtain two physical CPU entrants is a
failure, not an unreachable classification.

## Remaining T3–T7 findings

T3's dense APIC/logical CPU assumption, T4 Linux futex comparison atomicity,
T5 robust owner-death cleanup, T6 retained waitpid receipts and T7 guard mappings
remain open as documented in the November 4 audit. T1 addressed child-TID
completion ordering, without closing the remaining T4 work. No new native T6
failure was claimed or hidden as part of T2R. Final physical retirement receipts
protect stack reuse; they do not establish retained waitpid status semantics.
