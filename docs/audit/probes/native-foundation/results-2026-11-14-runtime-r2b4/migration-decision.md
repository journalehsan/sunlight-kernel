# Pre-implementation checkpoint

Selected: Option A, retain the original Process as shared resource authority,
serialized by the existing SCHEDULER guard. Separate native group terminal
publication from executing-task Finished. Borrowers remain identified by exact
AddressSpaceIdentity (root + generation); retained owner cannot reach Reaped or
slot reuse until all borrowers are Reaped. No new reference counter or lock.

Option B: independent ThreadGroup/AddressSpace owner would require moving page
ledger, mapping-placement state, descriptors, quota and wait/supervision receipts
out of Process, adding allocation/failure and adapter paths. Reject for scope.
Option C: reuse existing VM ledger/retirement identity alone cannot express group
termination or parent completion. Reuse its identity, retirement barriers and
scheduler pins, but do not reinterpret an identity as a reference-counted owner.
The Linux retained leader is the existing structural precedent for A; Linux
semantics themselves remain separate.

Invariants: task death never implies native group death while authorized live
members remain; group terminal status is accepted once under SCHEDULER; forced
owner/group exit prevents further admission and terminates every same-VM member;
final group publication never depends on the initial task executing again;
physical owner reap waits for every borrower Reaped, all physical pins/queues/
current ownership clear, and VM active mask zero. Preserve mapping retirement,
TLB acknowledgment, fail-stop and quota code. No borrower acquires owner PID,
incarnation, trusted-service flags or endpoint receive rights. Private endpoint
closure remains task-scoped. Shared descriptors/VM/quota retain the owner row.
Parent completion is group-scoped. A stale VM generation cannot select another
group. Existing raw numeric PID APIs are not advertised as new generation APIs.

Expected production files: process/mod.rs (group state), sched/mod.rs and a small
native group adapter (terminal transition, task/group exit, forced targeting),
sched/thread_admission.rs (retained owner admission), arch/x86_64/syscall.rs
(explicit exit scope, wait and signal liveness), process/foreground.rs (retained
session authority), native_c_guest.rs (test-only physical-handoff observation).
No ABI change, production spin loop, extra blocking lock or callback protocol.
Existing scheduler -> broker/IPC and reaper -> PMM orders are retained.

Failures: creation remains a prepublication RAII transaction; bounded scans count
unreaped members without arithmetic wrap; terminal contention preserves first
accepted code. Uncertain retirement retains existing panic/quarantine ownership,
never converts an incomplete cleanup into Reaped. Existing task-bound endpoint
and grant policies must be reviewed rather than globally changing is_exiting.

Validation: locked runtime build, negative baseline, W1/W2/final borrower and
forced-after-owner-death witnesses on 1/2/4 CPUs; explicit exit/main-return and
native 73 C + 21 C++ suites; review native security/retirement/session fixtures.
Unexecuted exact contender/fault/session/generation schedules stay NOT_RUN or
UNQUALIFIED. Roll back only the new R2B.4 patch, preserving every baseline hash;
no commit, push, reset or broad refactor. Stop if safe terminal authority requires
moving IPC/capability ownership rather than adapting existing boundaries.

Checkpoint refinement before final-source validation: preserve ProcessIsAlive
(syscall25, including PTY incarnation qualification) as group liveness. Explicit
session termination must find a retained initial owner; ordinary task endpoint
liveness remains is_exiting. Migrate existing ProcessExit::exit and Rust libc
exit wrappers from 20 to the already supported 157; worker/pthread trampolines
retain 20. Added production files are ipc/src/lib.rs, sunlight-libc/src/{lib,sys}.rs
and process/foreground.rs. Update the existing foreground host adapter to compile
and exercise the exact-session retained-owner scan. First pilot/matrix runs remain
intermediate, never final-source substitutes.

## Comparative review

| Criterion | A: retained Process (selected) | B: new authority object | C: existing VM/retirement identity alone |
|---|---|---|---|
| Existing ownership | Leaves fields and resource adapters in their current owner | Moves/splits fields and every consumer | Identifies existing resources, owns no group state |
| Migration complexity | Explicit task/group predicates and terminal adapter | New allocation, attachment and resource migration | Small only if it omits required completion/authority semantics |
| ABI | Existing20/157; fix helper intent, no new number | Can be internal, requires wider implementation | Internal, but cannot supply exit contract alone |
| Synchronization | Existing SCHEDULER serializes membership/receipt/reap | Needs synchronized member/reference and authority protocols | Existing token locks serialize retirement, not group membership |
| Lock order | Existing scheduler/broker/IPC/PMM order, receipt before callbacks | New object locks must be ordered across all adapters | Reusing retirement locks for scheduler membership would cross boundaries |
| Identity/generation | Exact VM identity plus retained original PID/incarnation | Needs new generation-qualified group identity and VM linkage | Already exact VM identity; not task/parent/session identity |
| Final retirement | Existing borrower-Reaped/pin/active-VM barriers | Requires transferring every cleanup obligation | Has backing retirement barriers but no final live-member transition |
| IPC/capability | No owner promotion; task authority remains explicit | Must preserve task versus group authority through migration | Cannot alone classify task endpoint/grant authority |
| Session/PTY | Adapt group liveness and exact-session retained targeting | Requires new session-to-group authority adapter | Session identity is not VM identity |
| Failure behavior | Existing prepublication RAII and fail-stop retention | Adds fallible group allocation/attachment rollback | Existing quarantine useful, missing terminal authority |
| Performance | Bounded table scans; one inline receipt, no new allocation/lock | Potential O(1) member count, additional objects/references | Cheap identity but insufficient contract |
| Maintainability | One resource owner; explicit separate task/group facts | Clear conceptual split at cost of broad migration | Overloading a retirement identity would hide missing ownership |

A is selected for the proven original-owner/shared-VM defect. The complete
security/SHM/session contract still needs its native authority witnesses; selection
is not a declaration that unexecuted G4-G7 properties PASS. Borrower-created SHM
views still use per-task tracking and require a separate qualified authority
adapter if group-wide continuity is required. Do not silently broaden that into
an unrelated IPC/VM rewrite.
