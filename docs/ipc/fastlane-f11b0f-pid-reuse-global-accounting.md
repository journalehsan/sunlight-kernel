# FastLane F1.1B-0F: PID identity and global authority accounting

2026-10-26. Entry gates: A VERIFIED NATIVE; B PARTIAL;
Session Authority PARTIAL; Token-History PARTIAL; F1.1B-0 BLOCKED; C BLOCKED.
No commit, push, production transport activation or scheduler policy change.

## Executive summary

**F1.1B-0 remains BLOCKED.** The 1/2/4/12-CPU final fixture verifies the existing
ordinary runtime PID non-reuse contract without changing allocation policy,
adds real VFS requests held across recipient exit and timeout/exec, and checks a
serialized kernel broker/IPC census. Genuine same-numeric-PID reuse is not
forced. No production grant-authority defect was demonstrated or repaired.

The remaining grant-specific blockers are unforced essential lifecycle
orderings, incomplete global/service/steady-state reconciliation, and token
history's native global-exhaustion and lifetime/restart acceptance. These
prevent READY_WITH_EXTERNAL_BLOCKERS independently of Session Authority/N3M and
Doorbell/D3. Restricted SHM and production FastLane remain unavailable.

## Original unresolved blockers (inventory frozen before implementation)

Read against R4, R4.1, C, D, E, E2, E3R, E3S and E3T reports and E3T's
`target/fastlane-f11b0e3t-evidence/validation-summary.json`. Earlier absent
production wiring and retired-token history defects were subsequently repaired;
they are not reopened here. Prior report claims retain their recorded scope.

| ID | Invariant | Production path | Existing proof | Missing proof | Minimal work |
|---|---|---|---|---|---|
| F-PID | Old grants/replies never select a new process by numeric PID | `process/pid.rs`, `sched/mod.rs`, `capability/lifecycle.rs`, `ipc/mod.rs` | Host incarnation/VM rejection; native stale identity injection | Actual reuse or verified allocator non-reuse contract, including exhaustion | Audit every allocator caller and slot replacement; observe ordinary spawn/reap sequence, test real allocator boundary without rewriting live identity |
| F-LIFE | Commit/delivery/consume/retire have one terminal owner | `capability/{mod,lifecycle,vfs_grants}.rs`, `process/spawn.rs`, `ipc/mod.rs` | D recipient exit before/after commit; E3 N1/N2/N3/N7; E3S N5/N6 | Recipient exec before delayed reply, consume versus exit/cancel, issuer exec/exit before commit, VFS replacement | Deterministic production-path checkpoints; retain NOT_FORCED for absent boundaries |
| F-GLOBAL | All grant authority and temporary references reconcile | Broker tables, process IPC fields, IPC shards, real UAC/VFS loops, kernel session receipts | Local fixture conservation; E3 session subset; retained capacities | Whole broker identity census, exact IPC reference purpose, service-local storage and repeated-cycle reconciliation | Serialized scheduler/broker/shard census; distinguish immutable identity values from owning references and live inventory from capacity |
| F-HISTORY | Retired tokens never alias during supported lifetime | `capability/{token_history,history_boundary,mod}.rs` | Exact 65,536 boundary, replay/slot recycling and failed preparation tests | Concurrent final-slot issuance, production availability/error/lifetime and restart acceptance | Test actual serialized broker at last slot, document boot-wide budget and unsupported reset; do not clear history |
| X-SESSION | Session authority acceptance remains independent | Account/session protocol and E3T native fixture | N4 cooperative-stop prevention on 2/4/12; N5/N6 replacement login | Original N3M lookup failure unexplained; complete session acceptance | Preserve separate PARTIAL decision; replay established regressions |
| X-D3 | Doorbell graph/recovery acceptance remains independent | SunlightD D3 / RecoveryBlocked harness | Earlier native Doorbell/pin evidence | Recorded D3 failures through E3T; RecoveryBlocked not reached there | Preserve failures and record first failing checkpoint on current replay; no scheduler redesign |
| X-TOOLS | Final-source acceptance must expose unrelated failures | Existing validation tools | Prior scoped compatibility | Current host/native/build identities | Recheck strict Clippy, format, missing WAV, Miri and compatibility independently |

F-PID/F-LIFE/F-GLOBAL are native evidence gaps until a defect is demonstrated.
F-HISTORY is both a bounded availability limitation and an evidence gap.
X-SESSION and X-D3 are independent acceptance limitations. VFS restart admission
is a production availability limitation: no new recovery/adoption policy is
authorized. No demonstrated production defect has been established at entry.

## PID allocation, task incarnation and VM contract

`kernel/src/process/pid.rs::next_pid` computes checked maximum retained PID + 1.
There is no free list or PID release operation. Reaped process rows participate
in this maximum. `Scheduler::add_process_after_reaping` may replace a fully
reaped row, after excluding current tasks, pinned stacks and a reap watcher.
Replacing any row with a dynamically allocated PID preserves a new, strictly
higher maximum. This is an inductive non-reuse contract for ordinary runtime
allocation, not an assertion that this allocator is a persistent counter.

Runtime callers are `process/spawn.rs::spawn_from_path_with_restrictions` and the native
spawn/Linux fork/clone paths in `arch/x86_64/syscall.rs` (the four `next_pid`
sites). They retain SCHEDULER through selection and insertion. Process-table
rows are not removed by runtime `remove`, `retain`, `pop`, `truncate` or
`swap_remove`. `main.rs` does pop isolated scheduler objects and the two
boot-only IPC self-test rows pushed under SCHEDULER before ordinary userland
starts; these are not a runtime PID-release path.
The direct constructors in `main.rs` install boot identities or isolated test
objects; security/MM/scheduler fixtures use separate unpublished or opt-in
identities. This contract does not authorize a future runtime caller to replace
the maximum with an arbitrary smaller PID. `add_process` itself is a kernel
publication primitive, not a userspace-selected PID API.

The native fixture uses normal spawn to create eight `/bin/true` children,
records the installed PID/incarnation/VM, observes real death, and requires eight
actual Reaped transitions with zero exit status. Its arithmetic boundary calls
the real allocator on an isolated iterator: `MAX-1 -> MAX`, and a retained MAX
causes `None`, without wrapping or assigning MAX to any live process. Runtime
spawn returns failure on allocation exhaustion. Native full-width *live-table*
exhaustion is not forced; the isolated native boundary and production caller
source are separate evidence. No allocator selection mode or reuse facility
was introduced. Numeric reuse remains **NOT_FORCED**; ordinary runtime reuse
is **PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT** only for accepted native runs.

`process/foreground.rs::allocate_incarnation` uses checked AtomicU64 allocation
on process publication. A fresh process gets a new incarnation even if its
table index recycles. Exec keeps that incarnation but withdraws image roles and
replaces the VM. `process/mm2b_state.rs::allocate_identity` gives every VM a
boot-global checked generation and PML4 identity; exhaustion panics rather than
wrapping. Broker binding uses the globally unique generation, not a reusable
physical root alone. No task, incarnation or VM field is rewritten by this
fixture. Reaping excludes active address spaces, executing stacks and borrowers
before clearing IPC, VM, kernel-stack and capability state.

## Source audit: complete bound-grant path

| Operation | Authoritative checks and linearization | PID-only boundary |
|---|---|---|
| Create proof / delegate | `capability/lifecycle.rs::grant_requester_for_role`: current trusted image role, non-exiting issuer, nonzero incarnation/VM; requested PID resolves a live Process; exact kernel IPC caller incarnation/generation must match `pending_call` and issuer reply target | PID is a candidate lookup, not authorization |
| Commit | `grant_delivery` captures issuer/recipient PID, incarnation, VM and request generation; scheduler borrow spans `delegate_vfs_grants` or session Account transaction under CAP_BROKER | No validated identity escapes across an unlock |
| VFS batch preparation | Real root proof must already be delivered to the exact recipient and unexpired; reserve/mint fixed policies; failed batch truncates its unpublished rows, preserves proof and burns prepared token identities | Legacy syscall 100 remains a separate transferable bearer policy |
| Delivery | `ipc/mod.rs::deliver_reply_to_target`: find PID, reject exiting/wrong incarnation/endpoint/request generation or terminal outcome; `settle_grant_delivery` also requires exact VM identity and listed tokens | The initial lookup alone cannot publish a reply |
| Consume | Real VFS `AUTHORIZE_EXEC` -> syscall 164 -> `Scheduler::consume_vfs_grant` repeats live issuer/recipient and exact pending caller checks; broker requires Delivered + full recipient identity + valid path/expiry, then removes one row | No payload PID or arbitrary token possession selects owner |
| Cancel | Current recipient identity must equal the bound grant; duplicate removal is idempotent. IPC cancellation settles only pending authority for its exact call | No caller-selected owner PID |
| Exec / exit | `process/spawn.rs::exec_into_process` retires grants and sessions before new VM activation and clears old pending request/role. `sched/mod.rs::reap_process_resources` removes IPC references/endpoints, retires grants and clears process fields before Reaped | Cleanup scans by PID under lifecycle serialization; ordinary non-reuse contract and pre-reuse full retirement matter here |
| Issuer retirement | Pending issuer-owned grants retire; delivered recipient-owned VFS/auth grants may outlive issuer by policy. Session Account grants also depend on the authoritative session row | Old image role is withdrawn; surviving delivery is not fresh issuer authority |
| Endpoint / deferred reply | Endpoint IDs monotonically increment with checked exhaustion; call IDs contain PID/incarnation/nonwrapping request generation. Deferred handles are service-local and checked before publication | Endpoint owner metadata and cleanup remain PID-keyed; no endpoint ID reuse and no numeric process reuse are relied on |

There is no independent generation-qualified VFS slot handle: lookup is by the
opaque token plus exact recipient binding. Slot recycling safety therefore also
depends on boot-lifetime retired-token exclusion. Endpoint/legacy bearer transfer
is not retrofitted with recipient binding by these tests.

## Native fixture and lifecycle matrix

Only `production_grant_test` includes kernel observation/census code. VFS's new
`production-grant-test` feature adds a bounded rendezvous for two fixture
requests. Normal builds exclude both. Actual boot-launched UAC authenticates the
development root password and mints through its real syscalls; actual boot VFS
consumes. The fixture installs no trusted roles. One extra IPC cap is stripped
only by the opt-in VFS wrapper; the ordinary request shape and kernel caller
checks execute unchanged afterward.

`exit-consume` holds a request received by VFS before its consume syscall. The
controller uses ordinary SIGKILL on the real recipient. `exec-consume` holds a
different received request while its real deadline cancels the call; the owner
then execs `/bin/true`. Kernel assertions observe two grants retired before VM
activation, distinct old/new generations and denied old-token consumption. VFS
resumes with its original reply target and returns denial. A subsequent ordinary
VFS call proves independent service-loop progress and late-reply completion.
No spinlock is held across these userspace waits. No claim of overlap *inside*
SCHEDULER/CAP_BROKER consumption follows from these orderings.

| Case | Current path / precise proof boundary |
|---|---|
| P1 initial owner | Real UAC commit/delivery, identity census and VFS success on the exact caller; native fixture asserts one-use replay denial |
| P2 exit before commit | Existing production UAC precommit rendezvous and ordinary SIGKILL, replayed with F fixtures; no committed VFS authority |
| P3 exit after commit before delivery | Existing production UAC postcommit rendezvous; two committed records retire and old reply is excluded |
| P4 exit during consumption | New native exit-before-kernel-consume ordering; interleaving inside the serialized consume operation **NOT_FORCED** |
| P5 old grant after genuine PID reuse | **NOT_FORCED**; actual runtime allocator contract prevents that setup. Host incarnation replacement remains synthetic |
| P6 old pending reply after genuine PID reuse | **NOT_FORCED** for numeric reuse; exact call identity and endpoint non-reuse audited, stale lifecycle replies tested separately |
| P7 old VM after exec | Existing delivered-grant exec probe plus new held old VFS request across real timeout/exec; old token denied and late consume rejected |
| P8 slot recycle | Existing real production/model slot-replay tests and isolated native broker history tests; no old-token reassignment this boot |
| P9 issuer replaced | E3S/E3T Account replacement evidence is historical; final-source N5/N6 replay recorded separately. Numeric issuer reuse not forced |
| P10 concurrent cancellation/retirement | New real deadline -> exec -> late consumer sequence; earlier N5 cancellation/retirement. All internal simultaneous orderings not claimed |

Remaining essential gaps include a demonstrated native boundary for issuer exec
before VFS grant commitment, both consume/cancel winner orderings under the
supported execution model, and recipient VM replacement before *grant reply*
publication (a delayed VFS consume reply is a distinct operation). VFS replacement
with outstanding authority remains **BLOCKED**: only embedded boot VFS receives
the trusted consumer role; this task introduces no replacement/adoption policy.
N4's cooperative-stop prevention is not broadened to untested lifecycle variants.

## Broker, VFS, UAC and IPC ownership model

| Store | Current authority / references | Terminal policy and retained storage |
|---|---|---|
| Broker Auth/Account/VFS vectors | Sole authoritative token rows; Pending or Delivered with immutable issuer/recipient identities and request; Account rows include session scope | Consume/cancel/retire removes row exactly once; Vec capacity remains reusable |
| Broker token history | All successfully prepared identities, including unpublished/aborted ones; never consulted to authorize a caller | Intentionally retained until boot ends, never evicted or freed on retirement |
| Broker sessions | Session owner and issuer identity values, registration, state, bounded per-request receipts | Session retirement removes bound grants/receipts; no owning Process/VM pointer |
| VFS AUTHORIZE_EXEC | No independent authorization-grant registry; stack request token is presented to kernel | No grant ownership escapes the kernel decision; legacy handles/shared reads remain separate |
| UAC delegation | Stack proof and reply copies; proof is exchanged atomically by broker, exact request target lives in kernel IPC | Failed/omitted publication retires pending rows; stale copies are unusable, not automatically erased from all userspace bytes |
| UAC SessionStore | Fixed 16-entry UID/time/active cache, no grant-token ownership; not the kernel Account session table | Cache retention does not resurrect a broker token |
| Account handlers | One real UAC service loop, local request/grant/temporary storage values | Unsupported production mutation publication remains disabled; no new adoption/recovery semantics |
| sessiond | Active SessionRecord and bounded/last-closed metadata; kernel session registration is the authority boundary | Service-local retirement/capacity snapshot is not atomic with this kernel census |
| IPC | Process `pending_call`, terminal reply, immediate/deferred reply targets; per-shard queued calls and waiters | Exact cancellation/reaper removes transient entries; immediate stale reply target can intentionally remain a rejectable one-entry tombstone |

## Global reconciliation schema and limits

The new census holds SCHEDULER, then CAP_BROKER, then visits one IPC shard at a
time. Ordinary synchronous request identity/lifecycle transitions cannot race
the snapshot. It performs no IPC wait and releases every guard on return.
IRQ notifications may change unrelated queue traffic between shards; the census
counts authenticated ordinary call records, not a globally frozen notification
stream. The fixture's final boundary requires all clients, controller and eight
children actually reaped. Resident service activity is still distinct.

It checks all bound grant identities against the current process/VM table and
requires a live pending request/issuer for non-retiring Pending rows. Delivered
rows may legally outlive issuers. It separately records a retiring subset.
Every ordinary queued/waiter call must resolve its exact process incarnation
and pending endpoint/request. Immediate/deferred targets are counted separately
from the subset of intentional non-live tombstones. A tombstone is a scalar
checked again at use, not a task pin or usable authorization.

The disjoint authoritative state equation is:

`broker_live = ipc_tokens + shm + pending_bound + delivered_bound + legacy`.

`retiring_subset`, `deferred_subset`, `tombstones_subset`, cumulative commits,
deliveries, consumes and retirements are **not** additional disjoint states.
Final standard-client conservation remains `11 = 4 consumed + 3 owner-cancelled
+ 2 failed-delivery-retired + 2 exec-retired` per client. The two held-consume
clients contribute four additional committed VFS grants retired by exit/exec;
the existing postcommit-exit client contributes two. Password proofs and ordinary
IPC/SHM tokens are counted in the global table census, not this VFS subtotal.

**Full global reconciliation remains BLOCKED.** Kernel census consistency does
not provide a synchronized observation of UAC/VFS/sessiond heaps, service-local
SessionRecord retirement, Account staging, or allocator allocation/free events.
The exact missing boundary is a supported service-loop quiescence/acknowledgment
for these stores, coupled to a broker epoch. Current primitives do not export
that boundary. Source shows no second VFS/delegation authority store, but this
does not measure every temporary copy or service-local retained resource.
No whole-system leak-free claim or indefinitely blocked system IPC is used to
bridge that gap. Repeated snapshots within a single bounded fixture do not
constitute repeated quiescent multi-session steady-state cycles.

## Retained capacity, resource drift and token lifetime

`[GRANT-D] tables/accounting` preserves record sizes, lengths, capacities, heap
used/free and free PMM frames. `[GRANT-F] census` adds IPC queue/waiter capacity
bytes, reference counts, sessions and receipts. PMM/heap samples are taken outside
the broker census to preserve lock ordering; they are not atomic allocator
transactions. Startup resident-service allocations can explain growth in type,
but without attribution cannot be subtracted and labelled a leak-free result.
VFS currently uses a 64 KiB bump allocator whose deallocation is a no-op. This
is a separate finite service-heap availability limitation, not evidence that
broker grant records leaked. No VFS allocator replacement is made.

History has exactly 65,536 prepared-identity slots, not 65,536 retirements in
addition to live grants. All five token classes spend the same boot budget.
At most 524,288 requested token-word bytes plus Vec/allocator overhead are
retained, in chunks of 256 words. At the boundary `TokenHistoryFull` occurs
before entropy consumption or authority publication. Failed preparations/batch
rollback can spend the budget; retirement never refunds it. Existing authority
can still be looked up, consumed and retired, but transport or authentication
requiring fresh tokens can fail before reaching that operation.

The real broker's isolated native boundary checks the oldest/newest retired
token replay, slot recycling, existing live authority and exact exhaustion.
The added host test starts two issuer threads at the last real slot and requires
one successful endpoint mint, one `TokenHistoryFull`, unchanged unused entropy,
and permanent refusal after the winner retires. This is host concurrency,
not native global-broker exhaustion. Existing real failed-batch tests prove
rollback preserves a proof and never reassigns burned identities.

`ipc/src/vfs_grants.rs::try_mint` distinguishes Denied=1, TokenHistoryFull=2,
Capacity=3 and Unavailable=4; UAC forwards the exact error in its reply. The
Option wrapper and other legacy ABIs can collapse errors while failing closed.
The whole production broker at capacity, with real transport still delivering
these errors, is not exercised by filling an isolated object.

CAP_BROKER is static for the entire boot. Service replacement does not reset
history. Hot broker replacement/clearing is unsupported. A reboot destroys
processes, grants, sessions and IPC, but no deterministic persistent exclusion
of arbitrary user-retained preboot token bytes is implemented. No hot-reset or
cross-boot non-alias claim is made. Supported security lifetime is one broker
boot and at most 65,536 preparations, ending in permanent fresh-issuance denial.
Token-History therefore remains **PARTIAL**, not VERIFIED BOUNDED under the
requested restart/global-availability acceptance criteria.

An initially unused budget lasts 18.20 hours at one preparation/second,
1.82 hours at ten/second, or 45.51 days at one/minute; boot and failed work reduce
these ceilings. These are arithmetic estimates, not workload measurements.
Sorted insertion can move 65,535 words (524,280 bytes) while serialized. Native
full-capacity contention latency and allocator overhead are unmeasured. Raising
the constant alone exchanges more heap for finite lifetime and does not solve
globally spendable availability. The smallest safe follow-up is an explicit
supported issuance budget/admission policy and real transport exhaustion test;
retain exact non-reassignment and fail-closed exhaustion meanwhile.

## Validation results

Raw attempts, including failures and retries, remain
under `target/fastlane-f11b0f-evidence/`.

## Native F results and resource observations

Final acceptance attempt: `native-20261026T121712.233589Z`. All four runs
use the same frozen image and pass F, production delegation and D lifecycle
parsers. Each has matching source/ELF/ISO/embedded hashes and a terminal
completion marker, with no collector timeout.

| CPUs | Standard VFS grants committed / consumed | Extra exit/exec VFS grants retired | Spawn/reap cycles | PID non-reuse | Held exit/exec consume |
|---|---|---|---|---|---|
| 1 | 11 / 4 | 6 | 8 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | Both FORCED_AND_PASSED |
| 2 | 22 / 8 | 6 | 8 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | Both FORCED_AND_PASSED |
| 4 | 44 / 16 | 6 | 8 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | Both FORCED_AND_PASSED |
| 12 | 132 / 48 | 6 | 8 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | Both FORCED_AND_PASSED |

The six extra retirements are two after UAC commit/before delivery and four
after VFS receipt/before consume. Standard clients execute four base cycles,
one exact-path exchange and one delivered-grant exec. Twelve CPUs means twelve
real standard clients plus five lifecycle/controller tasks, not twelve
concurrent UAC service threads. Serial markers preserve actual CPU identities.

| CPUs | Exit case: commit / reap / late consume CPUs | Exec case: commit / exec / reap / late consume CPUs |
|---|---|---|
| 1 | 0 / 0 / 0 | 0 / 0 / 0 / 0 |
| 2 | 0 / 1 / 0 | 0 / 0 / 1 / 0 |
| 4 | 0 / 3 / 2 | 2 / 0 / 1 / 0 |
| 12 | 4 / 11 / 2 | 4 / 4 / 11 / 6 |

These CPU observations show cross-CPU lifecycle execution where the IDs
differ. They do not establish simultaneous entry into the broker critical
section. Each run publishes eight children with distinct incarnations and VM
identities; all 32 reap markers match their own boot's exact publication. Full tuples and
request/fingerprint histories are in the validation manifest.

| CPUs | Final IPC tokens / SHM tokens | Bound Pending / Delivered | Sessions / receipts | Current calls / queued / waiters | Stale queue identities | History len / capacity |
|---|---|---|---|---|---|---|
| 1 | 56 / 1 | 0 / 0 | 0 / 0 | 0 / 0 / 0 | 0 | 101 / 256 |
| 2 | 60 / 4 | 0 / 0 | 0 / 0 | 0 / 0 / 0 | 0 | 134 / 256 |
| 4 | 60 / 4 | 0 / 0 | 0 / 0 | 1 / 1 / 1 | 0 | 180 / 256 |
| 12 | 60 / 4 | 0 / 0 | 0 / 0 | 0 / 0 / 0 | 0 | 364 / 512 |

The remaining calls are validated ordinary resident IPC, not zero references.
Their application-purpose attribution and service-heap snapshots remain missing;
this table does not promote F-GLOBAL. No stale fixture reply/deferred target or
bound grant remains after the required real reaps.

| CPUs | Heap used start / final bytes | Free frames start / final | Retained VFS / Auth capacity entries | IPC queue/waiter capacity bytes |
|---|---|---|---|---|
| 1 | 285472 / 1472352 | 69371 / 34872 | 8 / 8 | 6528 |
| 2 | 329520 / 1688696 | 68540 / 30940 | 8 / 8 | 7072 |
| 4 | 442832 / 1733200 | 66822 / 30408 | 8 / 8 | 7168 |
| 12 | 846864 / 1857000 | 60030 / 28360 | 32 / 16 | 10336 |

These start samples precede resident-service startup. The large frame/heap
changes are **unattributed startup plus workload growth**, not a measured leak
and not a clean steady-state delta. Allocations/frees and repeated quiescent
resource stability remain UNMEASURED. History byte lengths are 808/1072/1440/2912;
retained history capacities are 2048/2048/2048/4096 bytes.

## Host regressions and tooling limitations

The final host sweep is `checks-20261026T121214.837189Z`; exact commands,
environment, exit codes and logs are in its manifest. Kernel host tests pass
133/133. Service libraries pass 258/258: IPC 62, UAC 18, sessiond 27, filesystem
73, SunlightD 29, audiod 25, ELF 11, Doorbell 8 and ring 5. Scheduler host checks
and all 37 Python parser tests pass. The four F parser methods exercise multiple
negative mutations: missing/duplicate observations, wrong PID/incarnation/VM,
invalid CPU, invented cases, wrong ordering, inconsistent
inventory and unsupported identity attestation. Synthetic parser input is unit
coverage, not native evidence.

Strict Clippy fails. All 14 source files named by those diagnostics are
byte-identical to HEAD (`clippy-source-comparison.json`); diagnostics include
kernel build and existing dependency code. This is a current failed check with
an unchanged-source comparison, not a claim that every warning was accepted.
Workspace formatting also fails on existing differences. Added Rust lines have
no rustfmt differences (`added-line-format.json`); no broad formatting rewrite
was made. Media tests pass 36 and fail one with the missing WAV fixture
(`NotFound`). Miri is unavailable for the installed nightly. `git diff --check`
passes. No missing tool or fixture is represented as PASS.

## Compatibility and source scope

Final compiled-source core replay `core-20261026T115544.939413Z` passes all
15 native runs: pin release, native Doorbell and scheduler on 1/2/4/12 CPUs;
VM/TLB on 12 CPUs; unmap and protection on four CPUs. Authority replay
`authority-20261026T120221.077315Z` passes entropy-unavailable refusal on one CPU
and isolated lifecycle/session/history checks on 1/2/4/12 CPUs.

Lifecycle replay `lifecycle-20261026T120457.057710Z` passes D1/D2/D3 and
after-ready RecoveryBlocked on 1/2/4 CPUs. Recovery evidence includes retained
identity, rejected stale endpoints, blocked activation, exact reap and bounded
restart. External lease/dependency state remains unknown by that suite's
contract. All its build and native commands return zero. Its whole-source
stability flag is **false**: only `tools/fastlane_f11b0f_validate.py` and
`tools/test_fastlane_f11b0f.py` changed while it ran. All compiled kernel,
service and configuration files match the final source. Those two files are
not dependencies of the lifecycle runner or its images; F parser/native
acceptance is rerun separately. This is explicitly scoped compatibility
rather than a fabricated unchanged whole-tree flag.

Earlier core replay `core-20261026T114718.817382Z` is diagnostic: fixture source
changed and embedded staging could overlap another image build. Its passes are
not used for final compatibility. Subsequent image builds are serialized; each
accepted image has its own archived bytes and identity manifest.

## Session, network and restart regression results

`native-20261026T121803.005541Z` passes N1/N2/N3M/N3R/N5/N7R/N7C/N7OC/N7OR
on two CPUs with verified source/image identities. The independently preserved
`native-20261026T121932.473612Z` passes N6 replacement isolation and a fresh real
TTY login under the replacement Account. These are current replays, not a reuse
of E3S's results. N7OC/N7OR include observed contention under the existing
fixture contract; this phase adds no scheduler intervention.

Network replay `network-20261026T121612.041977Z` passes the resident twelve-CPU
prerequisite and TCP/TLS on 1/2/4/12 CPUs. Each local certificate fixture exercises
unknown issuer denial, CA installation, verified IP/DNS connections, SOCKS domain
preservation and wrong-host denial with expected command exit statuses. No TLS
verification was disabled. Raw serial, packet captures and exact commands are
preserved. Local sockets required sandbox escalation, not certificate relaxation.

The current N4 first attempt, `native-20261026T122127.825360Z`, fails on two CPUs:
sequence 10 enters `n4-precommit`, sequence 12 records `boundary-timeout`, sequence
15 commits with the session still active, the client exits 1, and only sequence
18 records the manager signal. The matrix stops before 4/12. This is a current
controlled-boundary synchronization failure, not a forced retirement-before-
commit pass or proof of stale authority. The isolated retry `native-20261026T122546.550117Z` uses the same frozen image
and unchanged bound and passes on 2/4/12 CPUs as
PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT. Both results remain indexed.
The originally requested retirement-before-commit race is still not forced. No cause is
attributed to host overlap without a separate causal experiment.

## Current attempts, failures and historical limits

Every failed/retried attempt remains separately indexed. No earlier manifest or
serial file was edited to make a later pass replace it.

- `native-20261026T114056.602881Z` and `native-20261026T114404.976571Z` fail
  in the original controller's wait path; the former records client failure and
  controller exit 1. The precise userspace panic/error is not logged. Source
  permits an exit receipt to disappear when an ordinary startup process replaces
  an unobserved Reaped slot before a wait retry. That is a source-grounded
  explanation candidate, not a demonstrated scheduler causality result. The
  fixture now observes bounded process liveness and independently requires real
  kernel Reaped markers. No scheduler behavior changed.
- `native-20261026T114316.394565Z` has a completion marker but the original
  collector waits until its timeout, and source changes invalidate that attempt.
  The collector now stops on the established terminal marker, terminating only
  its own QEMU subprocess. Diagnostic timeouts remain recorded.
- `native-20261026T114651.875340Z` passes 1/2 CPUs and fails four CPUs: the
  exit-before-consume recipient hit a deadline intended for the separate exec
  case before the controller's SIGKILL. The exit case now uses its ordinary
  blocking call, bounded by controller and collector; the exec case retains its
  actual timeout. The matrix stopped before twelve CPUs on that failure.
- The first network replay has four loopback socket `PermissionError` failures;
  its resident twelve-CPU prerequisite passes. The first session N1 replay exits
  before guest execution because sandbox policy rejects its QMP Unix socket.
  Both attempts are retained, followed by explicitly approved local-socket
  replays. Neither socket failure is attributed to guest TLS or session code.
- Strict Clippy, workspace formatting, the missing WAV test and unavailable Miri
  remain current failures/limitations as described above.

Historical E3T evidence remains authoritative in its own scope: 453 prior
artifacts, six N3M replays, unexplained original post-race endpoint lookup failure,
D3 failure and RecoveryBlocked not reached. E3S's 27 successful executions,
N5/N6 on 2/4/12 CPUs and replacement login are preserved historical evidence.
The current D3/RecoveryBlocked passes do not explain older failures or promote
Gate B. Any current session replay is likewise independent of the unresolved
N3M investigation. Preservation is checked by recorded artifact hashes, not
by merely counting the files that happen to exist now.

## Final source, images and ordinary-build isolation

The final F image manifest is `build-20261026T121623.117449Z`, accepted by
`native-20261026T121712.233589Z`. All source hashes match before and after
1/2/4/12-CPU execution. Its ELF and seven embedded executables are byte-identical
to the previous successful F build; the ISO hash changes with image metadata.
The final parser is included in the final source manifest.

| Image | ELF SHA-256 | ISO SHA-256 |
|---|---|---|
| F test | `3a2f2e6f086ead95c0cdb652b5400b211c42c65c08cfc010d24ac6aa6ccbe954` | `1d288fafd4490dd0702bdf7beefc3790bf5551c6c2e3b5729e70fd2793e06f19` |
| Ordinary | `77a508874fc7f310a5a5dc9cb130cf87ab5e21c1b9c72c730d2870efd7ea7e18` | `f0b847259c4585dffb1b8d958b6c0edcd89d102fe05b3c3766ec6d64c6f77992` |

Each manifest includes per-file source hashes, actual build commands and embedded
service SHA-256/size/offset verification. Session fixtures use separate image
manifests and feature settings; they are not mislabeled as the F image.
The ordinary final build is `normal-20261026T121852.210431Z`; all six production
service builds and the kernel target build pass. Its ELF excludes GRANT-F,
GRANT-C, GRANT-D and SESSION-E3 marker families, checked in
`normal-instrumentation-absence.json`. The new VFS feature is default-off and
all kernel observation hooks are feature-gated.

`normal-native-final` boots that ordinary image on one CPU and reaches real
SunlightD and login readiness with matching hashes. This proves boot/service
readiness, not interactive UI behavior, production grant races or audio.
The separate SunlightD dependency-runtime binary host test passes 19/19.

Free workspace disk was rechecked before builds: initially 32 GiB, subsequently
27 GiB, with 26 GiB left after the final runs. Existing source-identical caches were reused. No historical evidence,
image, unrelated work or build cache was deleted; no broad cleanup ran.

## Remaining security blockers and readiness decision

| Required grant-specific property | Decision and evidence boundary |
|---|---|
| Secure issuance and correct trusted issuer | Existing real production methods, entropy refusal and isolated broker regressions pass; no trusted-role injection |
| Recipient incarnation and applicable VM binding | Real commit/delivery/consume checks and exit/exec probes pass; runtime numeric PID non-reuse is proven within its audited contract |
| Safe one-use consumption | Real VFS success and denied replay pass; token-history lifetime remains bounded |
| Cancellation and retirement | Tested sequential/held-boundary orderings pass; all critical internal consume/cancel/retire winners are not forced |
| All remaining critical grant lifecycle races | **Missing** issuer image replacement before commit and recipient VM replacement before grant-reply publication; do not substitute the delayed consume reply |
| Global authoritative inventory reconciles | **Missing** cross-service snapshot boundary, residual resident-reference purpose attribution and repeated quiescent steady-cycle resource accounting |
| Retired-token lifetime and availability | Isolated no-alias/boundary tests pass; **missing** whole-production-broker capacity/error-transport and accepted lifetime/restart procedure |
| Native SMP support | F tests pass 1/2/4/12; twelve real recipients exercise bounded authorized work. This does not prove every omitted interleaving |
| Final compatibility | Individual results and source scopes above; no blanket strict-Clippy/format/media/Miri PASS |
| Unexplained grant-authority correctness defect | None demonstrated; missing proofs remain blockers rather than assumed correctness |

The smallest grant-specific follow-up is limited to three items:

1. Add authenticated test-only rendezvous for the remaining issuer/recipient
   lifecycle boundaries and observe the actual competing operation. Where
   production serialization prevents a race, test and classify that mechanism.
   Exercise both relevant consume/cancel/retire winners; do not change scheduler
   policy or admit an untrusted replacement service to manufacture coverage.
2. Add bounded service-loop epoch acknowledgments and per-store observation
   sufficient to relate Broker, UAC/sessiond, VFS and IPC at quiescence. Record
   purpose and release of residual references, then repeat steady cycles with
   allocation/free and retained-capacity separation. Keep normal IPC progressing.
3. Test whole-broker near/exact capacity and error delivery natively, including
   concurrent final-slot issuers; adopt an explicit boot-lifetime issuance
   budget/admission and restart policy. Preserve permanent exclusion and
   fail-closed exhaustion. A larger constant alone does not close this item.

| Gate | Final decision |
|---|---|
| A Scheduler Pin-Release | **VERIFIED NATIVE** |
| B Native Doorbell | **PARTIAL** |
| Session Authority | **PARTIAL** |
| Token-History | **PARTIAL** |
| F1.1B-0 grant authority | **BLOCKED** |
| Overall production authorization prerequisite | **BLOCKED** |
| C Production FastLane | **BLOCKED** |

F1.1B-0 does not qualify for READY_WITH_EXTERNAL_BLOCKERS: grant-specific
requirements remain missing even before the independent session and Doorbell
tracks are considered. This phase grants no production data-path enablement.

## Restricted SHM prerequisites and handoff

Keep three tracks separate: (1) investigate the original N3M lookup failure and
complete Session Authority acceptance, (2) investigate historical D3 causality
and finish Native Doorbell acceptance, (3) close the three grant-specific items
above. Passing one track does not authorize the others.

Restricted SHM design can reference the audited tuple
`recipient PID + process incarnation + VM generation`, exact authenticated IPC
call ownership, fixed issuer policy and one terminal grant owner. It must still
specify map-right scope, target-VM validation at commit, cancellation ownership,
active PTE/TLB revocation, mapping-borrow retirement, generation-qualified
mapping handles and authorized teardown. These are design prerequisites only.
No Restricted SHM, shared mapping, public syscall, production Doorbell/channel,
audio/Helios/DriverKit migration or recovery/adoption mechanism was implemented.

The reproducible evidence assembly command is
`python3 target/fastlane-f11b0f-evidence/finalize_assessment.py` after all attempts
finish. It invokes the F consolidator, records the scoped source comparison,
attaches full inventories/resource observations and keeps every raw attempt
manifest immutable. Exact invocations and their child command environments are
included in `validation-summary.json` and `invocations.json`.

Artifact reconciliation confirms all 453 historical evidence artifacts and 11
required audit inputs unchanged (464 total checked hashes). An initial evidence
assembly failed when a legacy native result had a null validator; its traceback
is retained under `development/final-assembly-first-attempt.log`. The assembler
now accepts that documented legacy shape, and no raw result was altered.
