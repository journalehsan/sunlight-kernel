# Native runtime R2B.3: thread groups, reclamation and races

Date: 2026-11-14. Branch `main`; baseline/final Git revision
`f6b5ccb24d3ef2d2077eaada148a9208e3e2c738`. Implementation remains uncommitted.

**Overall: PARTIAL. R2B.3 is not COMPLETE.** Public handle ABA and reclamation
ownership were repaired, and C Control admission is now bounded. Initial-thread
exit still kills live borrowers; autonomous detached cleanup and exact TLS/once
race closure remain open. No kernel patch, condition variables, fake POSIX
symbols, full CPython port, commit or push.

Evidence root (**E**):
`docs/audit/probes/native-foundation/results-2026-11-14-runtime-r2b3/`.
All execution results below are fresh and bound to the final runtime source.
Historical CPython results are explicitly distinguished.

| Gate | Status |
|---|---|
| G1 | PASS |
| G2 | BLOCKED |
| G3 | PARTIAL |
| G4 | PARTIAL |
| G5 | PARTIAL |
| G6 | UNQUALIFIED |
| G7 | PARTIAL |
| G8 | PASS |
| G9 | PASS |

G4 is deliberately PARTIAL despite passing public ABA and reserved-join
witnesses: exact detach/exit/sweep and creator-publication schedules remain
unqualified. G5/G6 do not inherit PASS from functional concurrency tests. G7
includes a source review and selected real teardown tests, without claiming
unexecuted session, capability or retirement-fault coverage.

## Baseline and ownership decision

Read the R2B.1/R2B.2 reports and relevant lifecycle, race, build and runner
evidence. `source-baseline.json` records the initial revision, branch, complete
Git status, 57990 regular-file hashes and
5638 symlink targets. Existing libc, lock,
vendor, test and evidence changes were preserved. The original
`bash tools/build_native_c.sh` passed before production edits and after them.
This reproduces the installed-toolchain locked build; it is not a clean-room or
bit-identical rebuild claim.

`ownership-model.md` was established before production edits, then refined with
additional source references. Its graph and authority table cover Process/group,
original owner, borrowers, mappings/page tables, Controls, stack/TLS, scheduler
pins, IPC, capabilities, descriptors, session/terminal state and retirement.
`kernel-exit-contract.json` records the current syscall contract and migration.

The generation-qualified VM/ledger handle is an identity, not a group lifetime
object. The original Process owns VM reclamation. Reaping waits for every borrower
to be Reaped, outgoing execution pins to clear, queued/current ownership to end
and the VM active mask to reach zero. Existing retirement owner tokens, TLB
acknowledgments, fail-stop assertions and quota accounting remain unchanged.

Two native owner-kill branches exist: `sched/mod.rs:4580` on finish and `:2864`
in the reaper. Self-exit also closes caller-owned endpoints and publishes parent
completion; waitpid accepts Finished. External termination rejects Finished
owners. Fatal user exceptions route through the same finish routine. Thus simply
removing the first kill branch, retaining an owner as Finished, or transferring
ownership to a survivor would not establish the required contract.

**Kernel repair: BLOCKED under the mission's larger-redesign stop condition.**
The concrete migration in `ownership-model.md` evaluates the existing Linux
retained-leader pattern and proposes explicit native group-alive/terminating/
retiring state independent of schedulable-task death, with retained original
resource authority and exact identity. It separates task endpoints/capabilities
and private descriptors from shared VM/opt-in descriptor/quota lifetimes. Parent
completion, forced targeting after initial task death, session cancellation and
final retirement need distinct transitions and native witnesses. No authority is
transferred to an arbitrary borrower. No new syscall is proposed; internal fatal
termination scope must remain distinct from syscall 20 thread-only exit.

## Runtime modifications

Only `sunlight-libc/src/pthread.rs` and `sunlight-libc/include/pthread.h` changed
relative to the initial file manifest.

* Public pthread handles now contain nonwrapping serial tokens, with initial
  thread token 1 and worker tokens starting 2. Join/detach resolve under the runtime
  registry lock instead of dereferencing caller-supplied allocation addresses.
  Exhaustion rejects creation rather than wrapping. pthread_self returns the
  immutable token; the TCB still retains its private Control pointer.
* A join reserves unique ownership before waiting. Concurrent detach or another
  join cannot reclaim the Control. Lookup, detach observation, queue insertion,
  registry removal and release share the same lock, closing the earlier observer
  interval where a sweeper could free a detacher's pointer.
* At most 64 live, joinable-retained or detached-retained Controls are admitted
  before malloc/spawn. The existing 64-entry queue cannot silently drop another
  admitted Control. Metadata admission is refunded on malloc/spawn failure.
* TID/stack/TLS bookkeeping is published before sweep eligibility. A worker's
  self-detach/exit before creator publication remains retained until publication.
  TLS callbacks remain outside the registry lock and precede EXITED.
* Reaped acknowledgment still precedes stack/TLS unmapping. Failed final unmaps
  retain the Control/registry/queue owner; successful partial unmaps clear their
  stored base before retry. Generic spawn rollback failure injection is still
  unqualified and is not included in an unconditional physical-memory bound.

The pointer-sized opaque `pthread_t` typedef, calling convention and public
function signatures are unchanged. Private Control grows 72 to 96 bytes. Native
consumers relink the complete static archive; old numeric handles must not cross
runtime versions. The old white-box accounting source assumed public handles
were Control pointers; it was preserved and a new fixture uses verified private
symbol binding instead. Header comments now accurately say only later create
sweeps detached entries. No autonomous helper was introduced: an immortal helper
would interfere with final-thread semantics, while a retiring helper needs its
own stack-retirement owner.

## Exit scenarios

| Case | Result and evidence boundary |
|---|---|
| E1 initial pthread_exit | BLOCKED; required behavior FAIL on 1/2/4 CPUs. Worker alive and initial TLS cleanup are established, then kernel logs `address-space-owner-exit`; no worker survival/TLS completion. |
| E2 worker pthread_exit | PASS on 1/2/4: main survives, TLS destructors and join complete in focused/controlled fixtures; detached callbacks also measured. |
| E3 return main | PASS on 1/2/4: live worker, one atexit callback, exit 17, group reaping and physical cleanup. |
| E4 exit / _Exit | PARTIAL: exit 23 and one callback pass on 1/2/4; callback-free raw 157 exits 29 without callbacks. `_Exit` declaration and export still fail fresh compile/link probes. Raw157 is not a public _Exit substitute. |
| E5 final surviving worker | BLOCKED: initial-owner thread-only exit is unavailable; ordinary process reaping does not qualify this case. |
| E6 forced termination | PARTIAL: borrower-issued authorized raw SIGKILL on the live original owner passes on 1/2/4, exit 137 and final frame/FD/kernel-stack cleanup. Forced kill after original task-only death and endpoint/session authority cases remain unqualified. |
| E7 PTY/session owner exit | NOT_RUN native tab/session fixture; relevant foreground/session source paths reviewed. |
| E8 identity reuse | PARTIAL: public token reuse rejection and scheduler slot reuse measured; stale kernel-generation targeting NOT_RUN. |

Every observation has stdout, serial transitions, kernel/ELF identity, result CSV
and runner logs. `exit-scenario-matrix.json` records each cell independently.
Runner success for E1 means the negative observation completed, never that the
pthread contract passed. Forced-exit qualification includes actual
`NATIVE-C-REAP-END` owned_frames=0, fds=0 and stack=0 plus before/after resource
checks; it is not inferred from disappearance.

## Detached accounting and bounds

Four waves of 12 workers run on each CPU configuration. Every worker's callback
completes and syscall 156 acknowledges Reaped before the retained snapshots.

| Snapshot | Queued Controls | Control bytes | Stack bytes | TLS bytes |
|---|---:|---:|---:|---:|
| Each wave after kernel reaping | 12 | 1152 | 25165824 | 98304 |
| Same wave after requested 100 one-ms idle sleeps | 12 | 1152 | 25165824 | 98304 |
| Later explicit create/sweep and joined sentinel | 0 | 0 | 0 | 0 |
| Full queue after 64 completed/reaped threads are detached | 64 | 6144 | 134217728 | 524288 |
| Full queue after the idle observation | 64 | 6144 | 134217728 | 524288 |
| Full-queue explicit create/sweep and joined sentinel | 0 | 0 | 0 | 0 |

The 65th retained-Control creation returns EAGAIN before spawn. After reclaiming
all 64, subsequent creation/join succeeds and all 64 stale handles are rejected.
This proves actual full-capacity admission/queue behavior, not a proposed cache.
The 64 retained Controls are built one worker at a time with Reaped acknowledgment;
this is not evidence of 64 concurrently admitted workers and does not explain the
prior R2B.2 failed 72-worker attempt. Kernel admission limits were not changed.
Queue cardinality reaches 64 in native runs; the old overflow reference-loss path
was removed. There is no instrumented overflow-event counter; no queue-invariant
panic occurs in the completed tests.

Stack allocation is 2 MiB per C worker. TLS extent is 8192 bytes in these fixtures;
for another linked ELF the general bound is 64 times `tls::area_size()`, not a
universal 512 KiB TLS limit. Control bytes exclude allocator overhead. Extents are
read from actual Controls under quiescent conditions and linker aliases are
verified against the final ELF. Per-wave RSS/PTE/PMM census was not instrumented;
final process physical cleanup is separately checked by the existing runner.

**Autonomous reclamation FAILS:** the unchanged idle snapshots prove that the
new bound and ownership repair do not provide independent cleanup progress.
G3 remains PARTIAL. An event-driven typed retirement adapter with admitted
notification storage, helper/last-thread teardown and exact VM identity is the
migration proposal, not an implemented feature.

## Public handle and native concurrency witnesses

Baseline `handle-baseline` reproduces immediate address reuse: old and current
public handles are the same allocation pointer, stale detach returns 0, and the
guest exits 3. The runner correctly rejects that attempt. It is preserved.

Final `handle-reuse-cpu-*` executes 256 reuse/churn cycles per CPU. Old join and
detach return EINVAL while a newer worker is gated alive; newer joins retain the
correct return values. Token reuse does not occur. `handle-admission-cpu-*`
retains 64 joinable Controls after kernel reaping, rejects the 65th create, releases
them and verifies recovery plus stale-token rejection.

`handle-claim-cpu-*` performs 32 deterministic ownership intervals per CPU. A
white-box observer acquires the actual runtime lock and observes joining=1 while
the target is held alive. It releases that lock, performs detach and a second
join, and requires both to return EINVAL before releasing the target. The chosen
join completes; the registry is empty after every round. Final ELF symbol sizes
and addresses for the lock/registry are verified. Runtime instructions are not
patched and no production sleeps or test hooks are added. This proves an active
join-ownership overlap, not simultaneous execution of locked critical sections.

Exact detach/exit/sweep scheduling, early self-detach publication and injected
unmap retry remain unqualified. Nonwrapping token exhaustion is source-qualified,
not exhaustively executed. `handle-identity.json` retains these limits.

## TLS and once/futex closure

On 1/2/4 CPUs, T1/T2/T3 prove populated old-generation cells survive across key
slot reuse without exposure: old get is NULL, old set returns EINVAL and the new
key sees no stale value. T4 holds a real selected destructor after clearing its
cell and completing reentrant create/set/get/delete. T5 deletes/reuses the key
while that callback is held; T6 completes afterward; T7 records two joins, one
callback and no live keys. Thread/public token identities and key generations
are printed. Final process resource checks remain separate from these markers.

Exact create/delete, delete/get, delete/set registry request schedules and
controlled detach/exit/sweep ordering remain UNQUALIFIED. G5 is PARTIAL.
`race-closure-plan.md` explains why holding a registry lock until a competing
operation returns would deadlock and proposes bounded request/acquisition/
linearization markers outside the conflicting lock ordering.

Functional once contention, publication, nested initialization, initializer
pthread_exit/retry and repeated completed fast paths pass on 1/2/4. They do not
show kernel registration at the contested instant. Completion-before/during/
after registration, installed multiple waiters, registered-waiter abandonment,
contender termination and rapid transition schedules remain UNQUALIFIED. No
stress-only result is used as exact proof. Existing debug T4A hooks were examined
but were not armed or substituted for once-integrated C guest evidence.

G6 is UNQUALIFIED. The follow-up plan reuses pre-lock/compared/registered/blocked/
post-lock stages, permits only pre-lock entry announcements while SCHEDULER is
held, and never waits there for wake completion. On 1 CPU, another user actor
cannot run inside the waiter's nonpreemptible registration critical section;
that exact publication-during-registration schedule belongs on 2/4 CPUs.

## Native matrix and security review

| CPUs | C guests | C++ guests | Suite |
|---|---:|---:|---|
| 1 | 73/73 | 21/21 | PASS |
| 2 | 73/73 | 21/21 | PASS |
| 4 | 73/73 | 21/21 | PASS |

Heap/ABI runs pass on 1/2/4; TCP loopback passes on 2/4. TCP 1 is NOT_RUN because its
unchanged runner accepts only 2/4. The ordinary native runner accepts 1/2/4 only,
so 12 CPUs are NOT_RUN. Pthread ABI/header checks and C++ ABI/vendor checks pass;
static initialization, unwind, RTTI and expected fatal C++ cases are individual
guest suite cells, not inferred from compilation. `cpu-matrix.json` and
`regression-results.json` preserve each result and expected nonzero exit.

All 164 libm numerical/support files match the pinned compiler-builtins/libm
source; all vendored files match the R2B.3 baseline. Raw comparison against the
old R2A manifest still differs in Cargo.toml, as already recorded by R2B.2.
`vendor-integrity.json` and `libm-packaging.diff` preserve that raw failure and
the documented test-only no-panic dependency removal. It is not silently
reported as an exact raw-manifest match.

`security-regressions.json` records inspection of IPC endpoint closure,
capabilities, PID/incarnation/VM identity, retirement/TLB ordering, group exit,
foreground/session/PTY ownership, scheduler progress and fail-stop rules. Kernel
sources are unchanged. No security check, retirement assertion or capacity was
weakened/inflated. Executed guests retain bounded completion and final physical
cleanup evidence. Authenticated endpoint/session/PTY regressions, stale kernel
identity targeting and retirement/quarantine fault injection remain unexecuted;
G7 is PARTIAL. Individual forced death of a runtime-lock holder is not advertised
as a robust-lock/pthread cancellation contract.

## CPython, integrity and remaining work

CPython 3.13.0 is still BLOCKED. Configure PASS and core compilation FAIL from
R2B.1 are historical evidence. No fresh interpreter compile/link/run was attempted:
no missing condition-variable, strncpy, _Exit or demonstrated platform/header
API was supplied. Current worker TLS/once/handle evidence does not establish
interpreter readiness. `cpython-compatibility.json` assesses each affected area.

`source-integrity.json` compares every initial regular file and symlink target.
Only the two intended pthread files differ; previous user source/evidence,
lockfiles, vendors, kernel, native driver/target and std overlay are preserved.
Validation started with the final runtime hashes and checked they stayed fixed;
all required execution evidence refers to that source state. Baseline negative
ABA/initial-exit observations and fresh missing-_Exit failures remain available.
New files are this report and E's fixtures, scripts, logs and JSON. Guest build
artifacts stay ignored; new standalone ELFs are under target. No reset, stash,
clean, commit or push occurred.

Next milestone: implement and qualify the explicit native group lifetime/
termination migration, then an event-driven bounded retirement owner and the
remaining exact registry/futex schedules. Require E1/E5, forced kill after original
task death, session/PTY/IPC authority and retirement-fault witnesses before
claiming those gates. Condition variables and CPython integration remain separate
subsequent work.
