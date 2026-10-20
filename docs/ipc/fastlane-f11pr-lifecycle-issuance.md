# FastLane F1.1P-R: lifecycle-atomic issuance

Baseline: working tree on `4c70511d09ec42a915f869c33f56c552c6def959`, including
uncommitted F1.1P work. No restricted SHM, ring, Doorbell or scheduling-policy
change. This report distinguishes implementation from the remaining acceptance
coverage. **F1.1B-0 is not declared unblocked.**

## Reproduction and repair

Before editing, ran the frozen actual-broker test executable retained by F1.1B:

```sh
target/fastlane-f11b-evidence/broker-counterexamples-test \
  f11b_prerequisite_ --ignored --test-threads=1
```

Both assertions failed as expected:

```text
f11b_prerequisite_account_publication_after_cleanup_is_rejected ... FAILED
assertion failed: b.mint_account_grant(binding, 1).is_none()
f11b_prerequisite_auth_publication_after_cleanup_is_rejected ... FAILED
assertion failed: b.mint_auth_session_grant(7, 0, 0, 100).is_none()
test result: FAILED. 0 passed; 2 failed
```

Raw output: `target/fastlane-f11pr-evidence/before-counterexamples.log`.
This reproduces the captured broker operation order, not a native exploit.
Original source and executable remain in `target/fastlane-f11b-evidence/`.
Before repair, both syscalls validated under SCHEDULER, dropped it, and then
minted under CAP_BROKER. Cleanup could remove all existing recipient authority
between those steps; subsequent insertion escaped that cleanup.

Both syscall transactions now invoke methods in
`kernel/src/capability/lifecycle.rs`. A shared `grant_requester` checks trusted
issuer, both participants' exit state, nonzero process incarnation, nonzero VM
generation, and the exact pending IPC request/reply relationship. The methods
borrow Scheduler throughout validation and broker insertion. The syscall owns
SCHEDULER throughout that call. Raw auth/account mint methods are private to
the capability module; syscall callers cannot use a captured binding to mint
outside the lifecycle method. No validated identity escapes for later commit.

The audit also found the same defect in Wise Owl delegation and proof creation.
Those syscall transactions now retain SCHEDULER through registry insertion;
the initial delegation also verifies the pending IPC relationship. Session
binding changes retain SCHEDULER through registry mutation. Both kinds of Wise
Owl record are withdrawn when either bound participant retires. This is a
shared publication defect, not a new delegation feature.

## Invariant and linearization

Let L be SCHEDULER, B be CAP_BROKER, D be the delegated-caller registry, and R
be the live process incarnation/VM currently selected under L.

* Issuance owns L from authoritative authorization until insertion under B or D.
  The insertion is its commit/visibility linearization point.
* Exit marks the process exiting under L. All later issuances reject it, even
  when physical reaping must wait for CPU/stack/borrower safety.
* Cleanup owns L through authority withdrawal. It removes all committed
  auth/account records for the retiring PID and delegation/proof records in
  which it is caller or mediator. Successful reaping follows this withdrawal.
* Therefore either insertion precedes withdrawal and is observed by it, or
  lifecycle invalidation precedes authorization and issuance fails. There is
  no interval permitting cleanup to return before an already authorized
  transaction can later insert. Concurrent cleanup is serialized and removal
  uses retain/swap_remove; repeating removal is harmless.
* After successful image preparation, live exec withdraws old-image recipient
  grants and clears old pending/reply identities under L before VM activation. Failed exec preserves the old image and grants. Address-space
  generation allocation and process-incarnation allocation remain checked and
  nonwrapping. Reaped-slot reuse cannot overlap an issuance holding L.

PID is a lookup/cleanup selector, not a free-standing proof of liveness.
No PID tombstones or permanent bans were added. Auth records retain their
existing PID-based ABI/consumption semantics; atomic withdrawal, not a new
per-grant generation namespace, prevents their survival across completed exit
or exec. Account grants retain their VM-generation binding. Wise Owl binds
caller and mediator VM generations. Generation uniqueness is provided by the
existing allocator; no independent physical-root recycling proof is claimed.
Shared-VM borrowers still share a protection domain. This repair creates no
per-thread memory isolation or restricted grants.

## Lock order and design choice

Reviewed existing contracts: `docs/kernel/scheduler-audit.md` (Lock and interrupt
safety), `docs/fastlane-phase0-architecture-assessment.md`,
`docs/ipc/fastlane-f11p-capability-authority.md` (Fallible publication), and
`docs/ipc/fastlane-f11-grants.md` (lifecycle/TLB review).

| Operation | Existing acquisition order / serialization |
| --- | --- |
| Process lookup, exit, exec, reaping | SCHEDULER; process table has no separate lock |
| Grant commit | SCHEDULER → CAP_BROKER → entropy conditioner |
| Wise Owl commit | SCHEDULER → delegated registry; entropy draws are outside registry guard |
| Combined grant withdrawal | SCHEDULER → optionally PMM → CAP_BROKER → delegated registry |
| IPC publication | SCHEDULER → CAP_BROKER → one IPC shard |
| SHM allocation/map/cleanup | SCHEDULER → PMM → CAP_BROKER; VM ledger and TLB mutation inside existing transaction |
| Scheduling | SCHEDULER → one core-state lock; no policy/queue changes |
| Entropy bootstrap | Boot-only collection before user authority; runtime conditioner takes no scheduler, broker or PMM lock |
| VM/TLB completion | Existing exact-VM protocol, fixed/NMI remote handlers do not acquire SCHEDULER |

Option A was selected. Per-class auth/account tables are bounded at 64 entries;
there are at most 8192 active broker tokens, and at most 16 random candidate
attempts. Runtime entropy draws consume the already seeded conditioner; they
do not collect hardware entropy or wait for readiness. Metadata reservation is
fallible and small. No large backing allocation, IPC, user copy, VM walk or TLB
wait was added to these issuance critical sections. There is no reverse
registry → broker or entropy → lifecycle edge. All production registry calls
were inspected; temporary registry guards end before any subsequent scheduler
acquisition. Existing syscall IRQ-off requirements remain unchanged.

Option B would add reservation inventory, cancellation and lifetime obligations
without removing substantial work here. No reservation subsystem, busy polling,
cleanup wait or process-lifetime pin was introduced. Scheduler hold-time tails
remain a measurement limitation; bounded work is not a measured latency bound.
The existing reaper's allocation and synchronous TLB waits were not redesigned.

## Issuance-path inventory

Locations name functions to remain useful as line numbers move.

| Path / exact source function | Classification and boundary |
| --- | --- |
| `arch/x86_64/syscall.rs::sys_mint_auth_session_grant`, `capability/lifecycle.rs::issue_auth_session_grant` | Vulnerable before; repaired with L held through B insertion |
| `syscall.rs::sys_account_grant`, `lifecycle.rs::account_grant` | Vulnerable before; mint and consuming authorization now remain under L→B |
| `syscall.rs::sys_wiseowl_delegate_caller`, `sys_wiseowl_validate_delegated_caller` | Same demonstrated source gap; repaired through D insertion; native race coverage pending |
| `syscall.rs::sys_wiseowl_set_active_session` | Session mutation now remains under L→D, serializing it with issuance |
| `capability/mod.rs::issue_token` | Not recipient-bound; conditioned entropy, cross-table equality exclusion, 16 attempts, fallible callers retained |
| `syscall.rs::endpoint_create` | Already lifecycle-atomic: current process checked not exiting under L, endpoint/owner token committed under B |
| `capability/mod.rs::mint`, `derive`; `syscall.rs::endpoint_bind` | Not recipient-bound: endpoint-bound bearer authority; B serializes source validation, insertion and endpoint destruction; checked nonwrapping endpoint ID |
| `ipc/mod.rs::handle_ipc_call`, `mediate_nameserver_register` | Already under L→B→shard; tokens copied as bearer words, not newly recipient-bound records; sender badge/registration ownership remain checked |
| `capability/mod.rs::grant_vfs_capability`, `grant_elevated_vfs`, `sys_grant_capability`; `syscall.rs::sys_grant_capability_syscall` | Not recipient-bound: transferable bearer lifetime intentionally survives issuer exit; batch rollback retained. Numeric broker-PID authorization is a separate authorization concern, not repaired here |
| `syscall.rs::sys_shm_alloc`, `memory/shared.rs::alloc_shared_region`, `capability/mod.rs::prepare_shared_region` | Already owner-publication-atomic: L→PMM→B; borrowed preparation retains B through complete backing/PTE setup and commit; legacy transferable peers retain mappings |
| `capability/mod.rs::mint_shared_page` | Trusted internal backing helper, not a userspace recipient-grant API; frame remains caller-owned on failure |
| `sched/mod.rs::terminate_process_by_pid`, `close_owned_ipc_endpoints`, `reap_process_resources` | L invalidates exit state; B withdrawal now also removes D participant records; physical reaping can defer safely while all new grant admission rejects exiting tasks |
| `syscall.rs::sys_exec`, `process/spawn.rs::exec_into_process` | After successful preparation, live replacement (`activate_on_success=true`) retires auth/account and both Wise Owl participant relationships before activation; clears old pending/reply identity; preparation failure does not retire old grants |
| `process/spawn.rs::exec_into_process` unpublished spawn | `activate_on_success=false` may already hold B; it performs no new grant withdrawal or broker acquisition. Only the live sys_exec caller uses true and owns L→PMM without B |
| `sched/mod.rs::add_process_after_reaping`, `process/foreground.rs::allocate_incarnation`, `process/mm2b_state.rs::allocate_identity` | Audit only: checked identity allocation; reuse restricted to safe reaped slots. Native targeted PID/VM reuse security matrix remains pending |
| `capability/mod.rs::revoke`, `destroy_endpoint`, `revoke_endpoints_owned_by`, `retire_recipient_grants` | B serializes revocation with insertion; recipient withdrawal requires outer lifecycle serialization; no revocation-tree or active SHM mapping withdrawal added |
| `syscall.rs::sys_lock_auth_consume`, `sys_session_auth_consume`, authenticated spawn | Consumption removes records, does not insert a grant. Some validation/consume boundaries release L; subsequent credential use is a separate authorization/lifetime audit, not proven by this repair |
| Legacy VFS daemon backend object IDs | Separate authorization/lifetime protocol; audit only, no claim of recipient-incarnation security |

Existing authority transfer is not treated as recipient-bound issuance merely
because a token word is sent to another task. Recipient cleanup, issuer cleanup,
endpoint revocation, backing retirement and live mapping withdrawal retain their
distinct contracts. Committed auth/account grants belong to the recipient; issuer
exit does not retroactively withdraw them. Wise Owl explicitly binds both parties,
so either party's retirement removes those records. RecoveryBlocked is unchanged.

## Transaction and failure behavior

No externally usable prepared authority exists: validation → capacity check →
fallible small Vec reserve → secure candidate selection → insertion → register
return. Raw mint failure inserts nothing. Reserved Vec capacity can remain
allocated for reuse; it is not a live grant or leaked backing reference.
Collision retries exclude every active broker table and never overwrite a
record. Entropy unavailable/exhaustion has no predictable fallback. Account
expiration pruning remains existing policy. Successful cleanup releases each
record once; no backing/grant reference subsystem was added.

The repaired grant syscalls publish one or two register words, not fallible
userspace copy-out. Frame register assignment cannot fail after commit. Issuer
exit after commit leaves recipient-owned grants accountable to recipient cleanup
(and existing single-use/expiry rules); it cannot publish another grant because
new admission rejects an exiting issuer. Ordinary endpoint/VFS/SHM rollback
contracts and existing tests are retained. A hypothetical new copy-out API must
supply its own rollback; this report does not infer one from register returns.
Reservation cancellation is inapplicable: there are no lifecycle reservations.
No token is deliberately recycled. Recurrence of a removed bearer is still a
probabilistic 62-bit risk, not a deterministic historical-token exclusion claim.

## Validation and acceptance boundary

Raw evidence and machine-readable results are under
`target/fastlane-f11pr-evidence/`. Run:

```sh
python3 tools/fastlane_f11pr_validate.py --native --regressions
```

The host suite compiles the production lifecycle issuance methods and broker.
`capability/lifecycle_host.rs` models the process table, lifecycle mutex and IPC
identity inputs; it does not compile the hardware scheduler or real Process.
`auth_publication_vs_cleanup` and `account_publication_vs_cleanup` use explicit
barriers at the old post-validation/pre-insertion boundary. They assert cleanup
cannot acquire lifecycle serialization, inspect an empty broker before commit,
resume, inspect the committed record, perform cleanup and verify zero records
and rejection afterward. These are the correct commit-before-cleanup alternative
to the original broker-only assertions. Trusted raw broker acceptance of a PID
is not itself prohibited; the raw methods are now private. Native opt-in assertions also check that the
SCHEDULER lock is unavailable immediately before each broker insertion. On one
CPU this deterministically detects the old unlocked syscall boundary. This is
a nonblocking test assertion, not a new production lock acquisition.

Additional host coverage exercises cleanup before issuance/during external
preparation, repeated cleanup, exiting issuer and recipient, modeled exec
withdrawal and new incarnation/VM generation, multiple recipients, concurrent
grants and cleanup, 64-record exhaustion, reservation failure, entropy failure
and bounded collision failure. Additional tests reject invalid/zero identities,
stale IPC generations, concurrent issuer/recipient exit, repeated Wise Owl
participant withdrawal and prepared proof publication after session cancellation. Existing broker tests cover generation mismatch,
consume-on-mismatch, rights attenuation, cross-table collisions, rollback and
endpoint reuse. Host mocks and direct retirement are not native exec/exit proof.

The native opt-in fixture uses the actual IPC request, auth/account syscalls,
process table and reaper. Sixteen requests each mint both grant types, leave all
32 records unconsumed, then recipient exit must remove them. After confirmed reaping, the still-live
issuer tries both actual grant syscalls again and requires rejection. Other isolated
ring-3 workers concurrently exercise real endpoint creation/derivation/destroy,
unauthorized endpoint access and legacy SHM. Completion asserts both successful
commit counts and zero total tokens/mappings plus zero data/page-table frame
drift. The fixture is compatibility/SMP evidence, not a cross-CPU pause/cleanup race at the vulnerable boundary, a native grant
cancellation matrix or a
restricted-memory protection test.

Fresh result tables and remaining blockers follow after validation. No host
result will be represented as native execution. The missing repository media
WAV is recorded separately from security failures.

## Independently exposed blockers

An added actual Ring-3 exec probe loaded `/bin/true`, then produced:

```text
[SYSCALL] exec path=/bin/true, argc=0, envc=0
[EXEC] Loaded ELF entry=0x400030, stack=0x7fffffffefb0
[FAULT] #14 pid=18446744073709551615 rip=0x0 rsp=0x7fffffffe530 err=0x0
```

Retained image/log: `target/fastlane-f11pr-evidence/20261020T155200.202280Z/authority.iso`
and `authority-1cpu.serial.log`. This attempt was interrupted at the failed
native boundary; it is not a passing run. Source audit identifies a wider
prerequisite: `syscall_entry` builds its frame and executes Rust on the user
stack, while `exec_into_process` activates the replacement VM before returning.
The old syscall stack is not preserved in that VM. This is not a demonstrated
new grant race, and a complete syscall-entry/exec repair is outside this phase.
Grant withdrawal was moved before activation, so it does not rely on successful
return through this unsupported transition. The opt-in diagnostic remains
available with `SUNLIGHT_CAP_EXEC_PROBE=1`; the ordinary positive lifecycle
fixture exercises exit instead of representing the failing exec as covered.

The first D1/D2/D3/RecoveryBlocked run also failed at
`services/sunlight-startup-fixture/src/d3_driver.rs:256`: activation returned
53759 instead of 53632. Its raw recovery directory is retained in the first
run's evidence. The failure is an activation ERROR label, not a free-frame
measurement. Causality has not been established and it is not labelled an
unrelated passing regression. Final-source rerun results are recorded below.

The broader outstanding security boundaries also include privileged role
admission across exec/PID reuse (the VFS gate still uses a numeric broker PID),
subsequent credential consumption/use after lifecycle lock release, and the
legacy VFS daemon object-ID protocol. These are audit-only findings/limits,
not claims of demonstrated exploitation or reasons to weaken the repaired
publication invariant. Complete native identity/reuse and Wise Owl syscall
race coverage remain required before accepting the entire prerequisite gate.

## Final defensive results

The final user direction requested synthetic defensive regression fixtures and
no unnecessary further exploit reproduction. No further exec fault probe was
run. Four additional synthetic tests cover explicit preparation barriers,
recipient ownership after issuer death, withdrawal versus permanent PID bans,
and capacity recovery with new incarnation/VM/IPC generations.

| Result | Classification / scope |
| --- | --- |
| Kernel host library | Implemented and validated: **75 pass**, including 26 capability tests; `final-kernel-host.log` |
| IPC, Audiod, SunlightD host libraries | Implemented and validated: **115 pass** (61 + 25 + 29) |
| Scheduler host and bare-metal kernel check | PASS |
| Actual auth/account issuance, serialization assertion, recipient reaping and subsequent rejection | Implemented and validated at targeted native scope: **1/2/4/12 CPUs pass**, 16 commits per class per run, zero post-reap records |
| Resident boot/security/legacy SHM/Init/SunlightD fixtures | **1/2/4/12 CPUs pass** |
| Scheduler/VM native fixtures | **1/2/4/12 CPUs pass** |
| Final native D1/D2/D3/RecoveryBlocked | **1/2/4 CPUs pass** on the final run; first attempt's D3 activation failure remains retained |
| TCP/TLS, certificate rejection, local CA and SOCKS fixtures | **12 CPUs pass**, against final-source resident ISO; `network-final.log` |
| Media library | Separate known fixture failure: 36 pass, 1 fails because repository WAV is missing |
| Wise Owl lifecycle commit/withdrawal | Implemented but insufficiently validated: registry tests pass; full native syscall/interleaving matrix pending |
| Exec preactivation withdrawal | Implemented but insufficiently validated: source/model checks; actual successful native exec remains blocked at stack/VM transition |
| Issuance inventory, lock graph and independent authorization boundaries | Audit only |
| Restricted SHM/rings/Doorbell/live mapping revocation | Deferred, no APIs added |

Native evidence for the final production implementation is in
`20261020T155440.105512Z/`. Four host-only defensive tests were added after that
image build. The original run's source-hash mismatch is preserved: only
`capability/lifecycle_tests.rs` changed, under `cfg(test)`. The final full host
suite was rerun after those additions. The aggregate summary separately verifies
all native production-source hashes and records final host-source hashes; it
does not rewrite the raw run to pretend the test file never changed.

At 12 CPUs the isolated fixture used 24 concurrent endpoint/SHM workers and
3072 ordinary issuance cycles, alongside the 32 recipient-bound commits.
Completion reported 65417 free frames before/after, zero shared data-frame and
page-table-frame delta, zero mappings and zero active broker tokens. This is
bounded fixture accounting, not proof that all heap vector capacity was freed.
No lifecycle reservation records exist. No stale recipient grant survived the
fixture's completed cleanup.

Existing local broker timing samples remain in raw logs: at 12 CPUs, ordinary
`mint` guest-TSC p50/p95/p99 were 1251/4274/8238; endpoint creation including UART
was 196457/299615/584249. These are not auth/account syscall timings, not lock
hold durations, and not matched before/after measurements. Lifecycle issuance,
cleanup latency and scheduler/broker hold-time distributions remain unavailable
(JSON null). No performance gain or production latency bound is claimed.

**Acceptance remains blocked.** The two observed auth/account publication gaps
are repaired and covered by defensive modeled interleavings and targeted native
checks. Complete F1.1P-R/F1.1B-0 acceptance still requires a safe native exec
transition, native Wise Owl and full identity/failure matrix coverage, and
resolution of independently unaudited authorization/lifetime boundaries. The
earlier intermittent D3 failure is not erased by a subsequent pass. No restricted
grant work is started automatically.
