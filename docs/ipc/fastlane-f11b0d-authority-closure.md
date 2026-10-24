# FastLane F1.1B-0D: production grant authority closure

Date: 2026-10-24, Asia/Tehran. This is a **partial implementation with an
explicit BLOCKED acceptance decision**, not production FastLane enablement.
The machine-readable evidence index is
`target/fastlane-f11b0d-evidence/validation-summary.json`; each invocation has an
independent timestamp directory. Earlier C and historical D3 artifacts remain
untouched. The pre-existing version/manifest/lockfile worktree is preserved.

## 1. Executive summary and original blockers

The four original blockers were (1) retired bearer reassignment, (2) Account
session authorization at commit, (3) unforced lifecycle/PID/restart races, and
(4) incomplete global grant/resource accounting.

This change repairs the first defect within one kernel broker lifetime using a
bounded exact token history. It also repairs session identity saturation and
PID allocation exhaustion. Two native recipient-exit orderings now have explicit
userspace rendezvous fixtures on the real UAC/kernel path. Broker inventory,
table capacity, history storage, heap usage and physical free frames are
observed separately. These changes do **not** close Account commit serialization,
actual native numeric PID reuse, service restart, the remaining critical
interleavings, or global allocator/reference reconciliation.

Gate A remains **VERIFIED NATIVE**; Gate B remains **PARTIAL**; F1.1B-0 remains
**BLOCKED**; Gate C remains **BLOCKED**. No Restricted SHM rights, mapping,
revocation, production Doorbell syscall or cross-process Channel is added.

## 2. Baseline and evidence boundaries

Read alongside `fastlane-f11b0c-production-grants.md`,
`fastlane-f11p-capability-authority.md`, `fastlane-f11pr4-vfs-authority.md`,
`fastlane-f11pr41-mint-delivery.md` and
`fastlane-f13nb-doorbell-acceptance.md`. The C final raw serial logs under
`target/fastlane-f11b0c-evidence/20261021T231849.705180Z` confirm 11/22/44/132
commits at 1/2/4/12 CPUs, zero final fixture inventory, 160-byte VFS records and
retained empty VFS capacities 4/4/8/32. They explicitly said remote races were
NOT FORCED. These are historical baseline results, not D acceptance results.

The historical D3 raw log
`target/fastlane-f13na-evidence/d1-d2-d3-failed-final-harness/serial-2cpu.log`
still contains the actual `d3_driver.rs:50` assertion, 53759/0xD1FF versus
53632/0xD180. Passing current compatibility runs cannot explain that failure.

## 3. Retired-token threat model

An attacker retains a previously valid raw u64 and presents it after removal of
the associated record. Previously `issue_token` excluded only live values. A
synthetically repeated entropy candidate therefore recreated an old bearer in
a new endpoint record. Strong ChaCha output did not establish lifetime safety.
`token-before.log` retains the failing actual-broker regression; there is no
production entropy override in this reproduction.

| Reuse category | Actual policy and remaining boundary |
|---|---|
| Raw bearer bytes | Exact full-u64 non-reassignment for every prepared broker token this boot, including failed preparations. No guarantee across reboot. |
| Internal Vec storage/position | Recycling is allowed; equality lookup uses the token, not Vec index. Controlled equal-capacity recycle tests reject the old value. |
| Grant identifier | These tables have no separate persistent grant ID. The raw token is the lookup identity. Adding a slot generation alone would not repair raw equality. |
| PID | Recipient uses PID plus process incarnation and VM generation. Normal allocation remains max-existing-PID + 1; overflow now fails. Actual native numeric reuse is NOT FORCED. |
| VM slot/root | Address-space identity allocation uses checked nonzero generations; successful exec retires old grants. Host replacement coverage is distinct from native race proof. |
| Issuer/service identity | Trusted image roles are incarnation-specific and withdrawn on exec; undelivered issuer-owned records are retired. Restart cannot reset kernel token history. Actual restart races remain unforced. |
| Consumed/cancelled/retired record | Removal cannot remove token history. Repeated consumption/cancellation cannot reconstruct a row. |
| Different permission/path/session scope | A previously prepared raw value is rejected before any replacement scope is published. Legitimate exact-rights derivation may return the same still-live authority. |

## 4. Token correction, costs and lifetime

`kernel/src/capability/token_history.rs` retains a sorted, fallibly allocated
Vec of at most **65,536 u64 words**, shared across all five token tables through
`CapabilityBroker::issue_token`. Lookup is binary search; insertion moves at
most 65,535 words under the existing broker guard. Reservation precedes entropy
generation. Recording precedes returning a prepared token. Batch rollback and
dropped SHM preparations burn identity permanently. No cleanup operation prunes
history; it is never consulted to authorize a caller.

The existing conditioned entropy source, 62 payload bits, two routing tag bits,
reserved selectors and 16-candidate retry bound remain. Historical candidate
equality now counts as a rejected collision. This is a deterministic equality
guarantee combined with probabilistic guessing resistance, not new cryptography
or an increase in bearer strength.

The maximum requested backing is 524,288 bytes, 3.125% of the 16 MiB kernel
heap, plus the Vec header and allocator bookkeeping/rounding. Reservation grows
in 256-word chunks. This storage is intentionally retained security metadata,
not reusable grant capacity. A full history returns `TokenHistoryFull` and
permanently refuses fresh issuance. Existing authority can still be consumed,
cancelled and retired; exact-rights reuse can still return an existing bearer.
Allocation failure returns a capacity error before publication.

**Availability limitation:** an unprivileged create/destroy workload can spend
the finite global boot budget. No adequate long-running issuance budget or
principal admission policy is demonstrated. Increasing the constant merely
changes that limit. Sorted insertion's worst-case lock residence is unmeasured.
These are explicit operational/security acceptance limitations of this safe
partial repair, not a claim of unrestricted production readiness.

The global `CAP_BROKER` is constructed once; repository search found replacement
only in host tests. UAC/VFS restarts do not reconstruct it. An in-boot kernel
broker reset is unsupported and would violate this contract. Reboot discards
both authority and history; an externally retained raw word could recur in a
later boot probabilistically. There is no persistent boot-qualified token ABI.

## 5. Other concrete identity corrections

`sunlight-sessiond::allocate_session_identity` checks both counters before
advancing either. Zero or exhausted inputs fail without mutation; session
creation returns ERR_LIMIT. The previous saturating increment could repeatedly
issue u64::MAX. The final value is conservatively reserved as an exhaustion
sentinel. Session service restart still initializes counters to one: this repair
does not supply an authenticated activation epoch across restart.

`kernel/src/process/pid.rs::next_pid` preserves existing max-existing-PID policy
under SCHEDULER. Kernel spawn, native spawn/thread and the supported clone path
now reject exhaustion. The embedded spawn path previously ignored usize::MAX
through `filter_map(checked_add)` and could select an occupied maximum; other
paths wrapped or saturated. No PID is forced onto a live task and no PID-reuse
allocator or scheduler policy is introduced. The host arithmetic witness
preserves the old aliasing expression as well as testing the production helper.

## 6. Account/session authority model

| Authority component | Source and lifetime |
|---|---|
| Authenticated account UID/GID | Password verifier over passwd/shadow; not a client payload UID assertion. |
| Current graphical session | sessiond `SessionRecord`; Account accepts Running/Degraded through `accounts::active_session`, not Locked/Stopping/Stopped. |
| Session ID/generation | Service counters, now checked; not independently attested to the broker, not restart-qualified. |
| Service process incarnation | Kernel Process foreground incarnation and VM identity, live trusted issuer role at broker entry. |
| Trusted executable role | Kernel init/trusted SunlightD launch chain for UAC; boot-embedded VFS role. Constructors default false and exec withdraws roles. UID zero alone is insufficient. |
| Delegation rights | Root password proof exchange issues only fixed read/execute path policies. Account scope selects operation, revision, target/member and session tuple. |
| Recipient | Actual kernel IPC reply target plus matching pending request, process incarnation and VM. Numeric message PID is cross-checked. |
| Lifetime | Auth proof has existing short tick expiry; VFS inherits it. Account uses its existing short expiry. None establishes logout-atomic storage mutation. |
| Consumption | Broker one-use removal under serialization, separate from service storage rename. |

The cached UAC SessionStore/runas decision, password proof, Account grant,
graphical login and trusted executable role remain distinct. No Wise Owl active
session registry is repurposed as Account authority.

## 7. Commit/revocation linearization: what is and is not established

Existing issuer/recipient lifecycle validation and broker insertion linearize
under **SCHEDULER -> CAP_BROKER**. Exec/exit uses the same lifecycle lock. An
already exiting recipient/issuer or withdrawn image role cannot pass the later
live admission check. Successful exchange inserts a full batch and consumes the
proof under one broker guard. Failed batch issuance rolls back only unpublished
rows and leaves the original proof eligible for retry.

Account `active_session()` is a synchronous userspace request to sessiond.
`Scheduler::account_grant` authenticates the service/caller and compares the
supplied scope at consume; it has no Account active-session authority registry.
Session logout commits userspace `SessionState::Stopping` in `begin_stop`,
independently of the scheduler/broker transaction. A logout ordered between the
preliminary reply and mint is **not excluded**. A later check cannot establish
the required shared linearization point.

There is a further boundary: password/profile/group handlers consume a grant,
stage storage, and publish through VFS rename. Session invalidation may occur
between consume/check and rename. No cross-service serialization ties that
publication to session withdrawal. The implementation does not pretend that
broker insertion alone commits an Account database mutation.

The minimal missing contract is an authenticated session authority owned by a
specific service incarnation, with ordered activate/invalidate/expiry and broker
mint/consume transitions, plus an explicit policy for already committed grants
and the subsequent VFS publication. This requires a real shared transaction or
explicit ordered lease/commit protocol across these owners. Current independent
IPC checks provide neither. Implementing a second ad hoc session registry here
would not solve storage publication and would require a policy redesign.
**D3/D4 remain BLOCKED**, as permitted when the primitive is absent.

In particular, a restarted sessiond can recreate a service-local session
ID/generation tuple. Delivered grants use recipient ownership and tick expiry;
neither alone proves that an old Account scope cannot match that new session
tuple before expiry. No restart-qualified epoch or forced production test
excludes that semantic aliasing here.

## 8. Actual production call flows

* Password proof: `uac_service::handle_auth_password` -> syscall ->
  `Scheduler::issue_auth_session_grant` -> broker bound proof -> synchronous
  reply settlement.
* VFS delegation: `sunlight_uac::delegation::handle` -> `ipc::vfs_grants::mint`
  -> syscall 164 -> `Scheduler::delegate_vfs_grants` -> broker proof exchange
  -> exact-call delivery settlement.
* VFS consume: real `vfs_server` AUTHORIZE_EXEC -> syscall 164 -> trusted VFS
  role plus actual pending caller -> `consume_vfs_grant` -> one-use removal.
* Account: `account_service::{dispatch,dispatch_group,dispatch_profile}` ->
  kernel `session_query_process` credentials -> sessiond `active_session` ->
  account/password/revision policy -> `accounts::broker_grant` ->
  `Scheduler::account_grant` -> broker mint/consume -> separate VFS publication.
* Exec: `exec_into_process` retires recipient/pending issuer grants and withdraws
  trusted roles before replacement VM activation. Exit rejects new admission
  when marked Finished and the ordinary reaper clears records/IPC references.

There is no VFS userspace delegation authority table for AUTHORIZE_EXEC: it
forwards to the kernel broker. UAC temporarily holds message/token words on its
stack during delivery. Existing UAC policy caches and Account staging state are
different stores and are not included in the broker's live-token count.

## 9. Host replay and generation tests

The actual broker is compiled in the kernel host library; the lifecycle tests
use the existing host-only Scheduler/Process shim. Controlled candidates force
historical equality without waiting for chance. Coverage includes endpoint
destroy/recycle, consume/cancel/retire, recipient exit/exec, pending issuer exit,
same PID/new incarnation, VM generation replacement, host service replacement,
path/UID/Account scope changes, exhausted history, stuck source, failed batch
followed by successful retry, dropped SHM preparation and concurrent
consumption/retirement. Both old rejection and unchanged replacement inventory
are asserted. Existing delivered-recipient ownership survives issuer exit.

Inventory deltas/idempotent absence are lifecycle assertions; they are not an
allocator free-count or an independent exactly-once retirement event ledger.
Host replacement identities are never reported as actual native PID reuse or
native service restart. Miri execution is not inferred from these tests.

The final-source host replay passes **113 kernel tests** and **258 service/library
tests**: IPC 62, UAC 18, sessiond 27, filesystem 73, SunlightD 29, audio 25,
ELF 11, Ring 8 and Doorbell 5. The focused scheduler runner and ten C/D evidence
parser tests also pass. The separate SunlightD runtime binary passes 19 tests.
The parser's synthetic negative fixtures validate classification only; they are
never used as native evidence.

## 10. Account concurrency matrix

| ID | Required case | Current classification / missing evidence |
|---|---|---|
| S1 | Invalid before authorization | BLOCKED for full production proof; service fail-closed state checks audited, no forced login/logout fixture. |
| S2 | Invalidate after check, before broker commit | BLOCKED: no shared session/broker serialization. |
| S3 | Revoke versus commit | BLOCKED: no defined common linearization order. |
| S4 | Invalidate after valid commit | BLOCKED: session-bound postcommit ownership policy incomplete. |
| S5 | Session expires during delivery | BLOCKED: grant tick expiry is not graphical session expiration. |
| S6 | Logout with outstanding delegation | BLOCKED: no broker/sessiond withdrawal agreement. |
| S7 | Account/session service restart | NOT_FORCED native; kernel history persists but session authority epoch is incomplete. |
| S8 | Session generation reuse | Host checked exhaustion passes; restart-qualified production/native closure BLOCKED. |
| S9 | Invalidation versus cancellation | BLOCKED: independent authority owners. |
| S10 | Concurrent grants in one session | BLOCKED: each needs a valid shared session commit ordering. |

No passing model is substituted for these missing production transitions.

## 11. Forced lifecycle inventory

Abbreviations: G=`Scheduler::delegate_vfs_grants`/`grant_requester_for_role`;
D=`ipc::deliver_reply_to_current_target`/`deliver_reply_to_target` and broker
`settle_grant_delivery`; C=`Scheduler::consume_vfs_grant`;
R=`retire_recipient_grants`; X=`exec_into_process`; E=ordinary signal,
`terminate_process_by_pid` and reaper. Every race needs live authenticated
participants and an applicable pending/delivered record.

| ID | Functions / forced ordering / expected invariant | Existing proof and native remainder |
|---|---|---|
| L1 | E completes before G; no VFS commit | New native rendezvous; per-CPU result in index. Host invalidation-before-mint retained. |
| L2 | G -> E -> D; pending grant never reaches recipient | New native rendezvous, real reaper, later UAC progress and zero recipient inventory. |
| L3 | E versus C; one terminal authority owner | Host serialized concurrent consume/retire. Native NOT_FORCED. |
| L4 | X during D; old VM cannot receive/consume | Host binding/late settlement and native exec-after-delivery only. Critical native ordering NOT_FORCED. |
| L5 | VM replacement before C; old grant rejected | Host same VM slot/new generation, sequential native exec; concurrent native NOT_FORCED. |
| L6 | Issuer E before G; no stale role/commit | Host forced ordering; remote native NOT_FORCED. |
| L7 | Issuer X withdraws role before G | Host ordering and native role-withdrawal compatibility; remote critical ordering NOT_FORCED. |
| L8 | Retire recipient A, numeric PID reused by B, stale D/C/cancel | Host controlled incarnation replacement; actual native numeric PID reuse NOT_FORCED. |
| L9 | Retire issuer A, numeric PID reused by B | Host roleless replacement rejection; native NOT_FORCED. |
| L10 | VFS restart with pending/delivered grants and C | No legitimate restarted VFS role installation path demonstrated; native BLOCKED. |
| L11 | UAC/Account restart with session/outstanding grants | Host pending versus delivered ownership; native NOT_FORCED; session epoch BLOCKED. |
| L12 | Cancel versus C; no resurrection/double ownership | Native ordinary cancel-before-publication fixture and host terminal races; concurrent native C/cancel NOT_FORCED. |
| L13 | D completion versus R; late completion cannot reinsert | Host ambiguous/partial/double settlement; L2 covers exit path only. Other native orderings NOT_FORCED. |
| L14 | Pending D references exiting task; no old Process borrow escapes | L2 new native path; source stores identity values, not owning Process references. Other deferred callbacks NOT_FORCED. |
| L15 | Old service generation completes after replacement | Host stale delivery identity/generation coverage; legitimate native replacement NOT_FORCED. |

## 12. Native fixture design and limitations

The existing `production_grant_test` kernel feature and UAC
`production-grant-test` feature gate all new hooks. Normal images exclude them.
The ordinary non-root grant probe adds a same-ELF unprivileged controller
(`grant-racer`) and two recipients. Their kernel constructors do not install
Auth/VFS/service-manager roles. These are fresh distinct PIDs, not a PID reuse
test. The controller is pinned to the last CPU; actual commit/signal/reaper CPU
IDs are recorded, and CPU count alone is not remote-race proof.

The retained `[GRANT-C]` counters and its legacy `remote_race=NOT_FORCED`
classification describe the original per-CPU clients. The additional D pair
uses separate recipients, checkpoints and results; passing that pair does not
retroactively reclassify the original C probes.

The recipient obtains the controller's SEND capability through normal discovery
and transfers it in the fixture envelope. Only the opt-in, fixed-recipient
fixture removes that transport capability before the unchanged production
dispatcher validates the one-proof request. It grants no privilege: the real
password verifier, issuer role, pending request, process/VM identity and broker
policy checks still execute. No nameserver permission/profile is relaxed.

For L1 UAC blocks in a bounded IPC rendezvous immediately before production
dispatch/mint. The controller performs normal SIGKILL, observes the target no
longer live and acknowledges. UAC attempts the real mint and reports abort.
For L2 the real mint first commits two pending rows; UAC blocks before returning
the reply. The controller's signal-path observation asserts two VFS rows and
the matching recipient identity, then the ordinary termination executes. A
second rendezvous confirms UAC resumed. A later ordinary UAC request proves the
single-threaded service advanced past the stale reply attempt. Completion also
requires both recipients and controller to reach the real Reaped transition.

All waits are existing deadline-backed IPC (10/20 seconds); no sleep chooses an
interleaving and no test barrier blocks while holding a kernel lock. Signal-path
and reaper hooks only observe/assert. The precommit fixture still has one live
password proof; it has zero VFS rows. That proof must also retire on exit.
The postcommit fixture has consumed the proof and owns two VFS rows.

Logs contain actor incarnations, VM generations, CPU IDs, checkpoint names,
logical fixture batch identity and inventory. They do not emit raw tokens or
session secrets. These password-proof cases have **no Account session
generation**; logs explicitly say `session_generation=none`. They cannot count
as S1–S10, issuer lifecycle, actual PID reuse or service restart tests.
Individual grant fingerprints are not recorded; the observed two-row batch is
identified by its fixed case and exact recipient incarnation/VM. The requested
per-grant D6 trace contract therefore remains incomplete even where the two
recipient-exit orderings are explicitly forced and pass.

## 13. PID reuse and service restart findings

Native PID allocation has no supported deterministic reuse hook. It selects
the maximum numeric PID plus one while considering existing process records,
including reusable slots. Reusing a Process table slot is not reusing its PID.
The exhaustion correction refuses allocation instead of fabricating reuse.
Required closure remains a safely supported real allocator reuse plus stale
delivery/consume/cancel/deferred-reply assertions for both incarnations.

UAC can receive a fresh trusted role only through the authenticated launch
chain. Constructors and ordinary exec do not inherit it. Pending records are
issuer-owned and retire on its exit/exec; delivered bound grants are
recipient-owned and may outlive the issuer until their own retirement/expiry.
Kernel token history outlives either service. Account session ownership lacks
the separate ordered withdrawal policy described above.

Only the boot-embedded VFS gets `trusted_vfs_service=true` in `main.rs`; a new
process with the same basename/PID does not inherit consumer privilege. No
authenticated VFS relaunch/role restoration contract is added. SunlightD
RecoveryBlocked remains independent; this phase does not adopt resources,
transfer old private issuance authority or implement general recovery.

## 14. Global accounting model

The broker snapshot partitions current token records into mutually exclusive
**IPC, SHM, pending bound grants, delivered bound grants and legacy unbound
grants**. It asserts their sum equals the actual live record count and that
every live raw token exists in history, with no duplicate across tables.
Endpoint records are object metadata, not another token state. Retired SHM
with outstanding views remains a SHM record until final verified release.
Here `broker_live` counts retained records, not unconditional authorization:
expiry, binding, role and delivery checks still apply at each use.

Absent grant rows are terminal as authority, but the snapshot cannot distinguish
consume/cancel/expiry/rollback/exit without transition instrumentation. History
is a separate disjoint lifetime ledger of prepared identities; its count is not
a sum to add to current authority. Delivered state overlaps neither pending
state nor terminal absence. Existing fixture transition counters are reported
independently and do not establish global ownership conservation.

| Owner/store | What is accounted | Remaining reconciliation |
|---|---|---|
| Broker five token tables/endpoints | Lengths, capacities, record sizes, state partition, history membership and uniqueness | Independent per-grant transition/retirement reason ledger absent. |
| History | Prepared identities, used and retained capacity bytes | Intentional bounded monotonic retention; no release on retirement. |
| Broker Process/VM identity | Plain scalar values in GrantDelivery, no owning Process/VM/frame refs for Auth/Account/VFS | Deferred IPC/lifecycle owners still need independent census. |
| VFS AUTHORIZE_EXEC | No separate grant table; kernel is authority owner | Full VFS file/staging/legacy resource accounting not measured. |
| UAC | Local reply/token copies and existing policy/session caches | No synchronized live operation/copy/staging inventory exported. |
| sessiond/Account | Service SessionRecord, Account request and staged storage | Missing ordered authority epoch and cleanup/commit census. |
| IPC/deferred cleanup | Normal cancellation/reaper exercised | No all-queue cross-owner generation-qualified reference snapshot. |
| Heap/PMM | Whole-kernel heap used/free, free physical frames at start/final | Startup/other resident activity is not isolated; no attribution or allocation/free event counts. |

It is therefore possible to have a stale token copy outside the authoritative
inventory; absence makes that copy unusable, not nonexistent. Cleanup and
history exclusion prevent that copy acquiring a later grant in this boot.
An empty fixture inventory is not proof that no other owner retained metadata.

## 15. Resource and memory measurement contract

Native `[GRANT-D] tables` rows remeasure `size_of` and capacities for endpoints,
IPC entries, SHM entries, Auth, Account and VFS, including SHM frame-vector
capacity bytes. `[GRANT-D] accounting` rows record the state partition, history
length/capacity/bytes, heap used/free and free physical frames. PMM is sampled
before broker acquisition to preserve lock order. Samples are observations at
distinct points, not a simultaneous global allocator transaction.

An empty Vec's capacity is reusable storage, not a leak. Token history is
intentionally non-reusable retained storage. Backing bytes are record size
times capacity (plus nested frame Vec capacity), not allocator-consumed bytes:
headers, rounding and allocator fragmentation are separate. Whole-heap/PMM
start-to-final differences include boot services and fixture teardown and have
no demonstrated matched quiescent baseline. No zero global drift is claimed.

The missing instrumentation is a consistent generation-qualified inventory of
UAC/sessiond/Account staged operations and pending replies, VFS temporary
objects, all deferred IPC/reaper references, per-grant terminal transitions,
allocator allocation/free events and a repeated quiescent phase baseline. Until
those inventories reconcile, D9 remains BLOCKED even when broker fixture rows
return to zero.

## 16. Failure injection and retained development failures

Existing real-broker tests cover entropy unavailable, repeated candidates,
bounded live/history capacity, prepublication capacity rejection, invalid proof,
invalid recipient/role, batch rollback, delivery omission/cancellation, expired
consume, duplicate cancel and exec/exit cleanup. New tests cover burned identity
after failed mint/prepared SHM drop and permanent history exhaustion. These do
not force every allocator OOM, final retirement callback failure, cross-service
session fault or all acknowledgment/publication failure orderings natively.

Development attempt `20261024T091727.526717Z` stopped because normal UAC lookup
restrictions correctly denied the private fixture endpoint. The fixture was
corrected to transfer a legitimate SEND bearer; permission policy was retained.
Attempt `20261024T091904.980606Z` reached postcommit exit/reaping but the
precommit observation incorrectly expected zero **all-class** grants while the
recipient still owned its delivered Auth proof. The assertion was corrected to
distinguish zero VFS plus one proof from two post-exchange VFS rows. Both raw
failures and verified ISO archives remain. Attempt
`20261024T092033.738440Z` passes the one-CPU development subset, but is not the
stable final-source acceptance replay.

The first broad validation started before the late PID exhaustion repair. Its
source mismatch must remain visible; later passing rows do not make that run a
stable snapshot. The final accepted evidence subset must match all source
hashes before and after execution.

## 17. Native SMP and compatibility results

The stable primary run is `20261024T100914.239860Z`. Both source manifests
match. All four production delegation subsets pass; the two synchronized recipient
exit orderings pass at every CPU count. No other critical race is promoted.

| CPUs | L1 before commit | L2 after commit/before delivery | Commit/signal CPU | Ordinary + extra committed VFS grants | Final fixture grants |
|---|---|---|---|---|---|
| 1 | FORCED_AND_PASSED | FORCED_AND_PASSED | 0 / 0 | 11 + 2 | 0 |
| 2 | FORCED_AND_PASSED | FORCED_AND_PASSED | 1 / 1 | 22 + 2 | 0 |
| 4 | FORCED_AND_PASSED | FORCED_AND_PASSED | 0 / 3 | 44 + 2 | 0 |
| 12 | FORCED_AND_PASSED | FORCED_AND_PASSED | 10 / 11 | 132 + 2 | 0 |

The extra two grants are L2's pending batch. L1 commits none; its previously
delivered Auth proof also retires. All three extra processes reach the real
reaper. Checkpoint process incarnations, VM identities, CPU IDs, ordered raw
markers and serial hashes are in each per-CPU JSON result. Different checkpoint
CPUs document actual placement, not simultaneous execution or a latency claim.
The final 2-CPU commit and signal checkpoints both ran on CPU 1; this is not
cross-CPU commit-versus-exit checkpoint evidence. They differ at 4 and 12 CPUs.
Account session generation is explicitly `none` in these password-proof tests.

Final broker/table observations (the residual broker records are ordinary
resident IPC/SHM, not unexplained Auth/Account/VFS fixture grants):

| CPUs | Broker live (IPC/SHM) | VFS length/capacity | History length/capacity | History used/capacity bytes | Heap used bytes | Free physical frames |
|---|---|---|---|---|---|---|
| 1 | 57 (56/1) | 0/4 | 93/256 | 744/2048 | 1468176 | 34901 |
| 2 | 60 (58/2) | 0/4 | 122/256 | 976/2048 | 1533680 | 31531 |
| 4 | 64 (60/4) | 0/8 | 172/256 | 1376/2048 | 1699968 | 30437 |
| 12 | 64 (60/4) | 0/32 | 356/512 | 2848/4096 | 1817480 | 28389 |

Remeasured record sizes are Endpoint 16, IPC 16, SHM 64, Auth 88, Account 136,
and VFS **160 bytes**. All Auth/Account/VFS final lengths are zero. Exact
retained capacities for all tables and nested SHM frame vectors, both start and
final heap/PMM samples, and the broker partition are retained in JSON/raw logs.
This VFS/password-proof matrix does not exercise Account session operations;
the empty Account table is not Account cleanup or session-validation proof.
These boot-to-completion observations do **not** establish allocator drift or
cross-owner reconciliation.

The compatibility run details below retain every check, including failed or
blocked attempts; exact commands, image/service identities and raw outputs are
in their independent directories. Functional TCG execution is not real-hardware
or performance proof. Current D3 passes do not resolve the historical D3 failure.

* `compat-20261024T093144.889694Z`: final source equality **True**, before/after equality **True**.
* entropy-unavailable: 1 CPU **PASS**.
* authority: 1 CPU **PASS**, 2 CPU **PASS**, 4 CPU **PASS**, 12 CPU **PASS**.
* pin-release: 1 CPU **PASS**, 2 CPU **PASS**, 4 CPU **PASS**, 12 CPU **PASS**.
* doorbell: 1 CPU **PASS**, 2 CPU **PASS**, 4 CPU **PASS**, 12 CPU **PASS**.
* scheduler: 1 CPU **PASS**, 2 CPU **PASS**, 4 CPU **PASS**, 12 CPU **PASS**.
* vm-tlb: 12 CPU **PASS**.
* vm-unmap: 4 CPU **PASS**.
* vm-protect: 4 CPU **PASS**.
* resident: 12 CPU **PASS**.
* Image/service/tool commands: 37/41 passed. Exact per-command results remain in JSON.
* d1-d2-d3: **PASS**, exit 0.
* recovery-blocked: **PASS**, exit 0.
* tcp-tls-1cpu: **BLOCKED**, exit 1 (sandbox socket denial; no TLS execution).
* tcp-tls-2cpu: **BLOCKED**, exit 1 (sandbox socket denial; no TLS execution).
* tcp-tls-4cpu: **BLOCKED**, exit 1 (sandbox socket denial; no TLS execution).
* tcp-tls-12cpu: **BLOCKED**, exit 1 (sandbox socket denial; no TLS execution).
* `compat-20261024T100138.255777Z`: final source equality **True**, before/after equality **True**.
* resident: 12 CPU **PASS**.
* Image/service/tool commands: 8/8 passed. Exact per-command results remain in JSON.
* tcp-tls-1cpu: **PASS**, exit 0.
* tcp-tls-2cpu: **PASS**, exit 0.
* tcp-tls-4cpu: **PASS**, exit 0.
* tcp-tls-12cpu: **PASS**, exit 0.

## 18. Baseline failures and exceptions

Strict Clippy remains a failing check when existing dependency/build-script
diagnostics prevent reaching the changed code; a category-exception diagnostic
run is not a strict pass. Workspace formatting differences remain failures;
changed additions need their own evidence. The media suite's missing repository
WAV remains a failed test, not generated replacement data. Miri availability is
separate from Miri execution. Detailed signatures, unchanged-source checks and
any unavailable matched baseline are retained in the final evidence index.

The final strict kernel-host check has **72 errors**, all matching historical
message/path signatures with unchanged diagnostic source files. The media suite
has **36 passed, one failed** because
`docs/songs/onaldin_music-catch-the-sunlight-333176.wav` is missing. Miri is
**UNAVAILABLE** (`cargo-miri` is not installed for the toolchain). The whole
workspace formatting check fails. Bare-metal kernel check, production service
builds, ISO creation and `git diff --check` pass. The combined runner exits one
because these failing/unavailable checks remain; it is not a clean overall pass.

Supplementary strict service lint reports five errors: one IPC signature matches
the retained historical logs; four filesystem diagnostics are absent from those
selected logs, but both filesystem source files equal HEAD byte-for-byte.
Strict native-kernel lint stops at the historically matched
`kernel/build.rs:999` range-check diagnostic; that file also equals HEAD.
Neither check reaches a complete changed-code lint verdict. The 15 changed/new
Rust paths pass the added-line formatting comparison; pre-existing whole-file
differences remain separately recorded. A matched full isolated baseline is
**MATCHED BASELINE UNAVAILABLE**.

The C 2/4-CPU timeout history is preserved. Its report identifies a completion
observer assuming Reaped records remained present until timer sampling; real
exec/cleanup/reaping followed by slot reuse invalidated that assumption. The
current fixture still records the actual reaper callback. This explanation
does not excuse a new timeout or explain the unrelated historical D3 assertion.

## 19. Final-source verification and remaining security issues

The D runners hash all tracked/untracked Rust, manifests, scripts, linker files,
assembly, Cargo.lock and Limine configuration before execution and again after.
The production runner retains source bytes, worktree patch, tool versions,
environment, embedded service hashes, stripped kernel and ISO identities, and
a verified lossless ISO archive containing `/boot/sunlight-kernel.elf`.
Compatibility images are transient with exact hashes and raw execution logs;
they are not relabeled as production images. No source edit after a validated
snapshot can inherit its acceptance result.

The final accepted source snapshot hashes **1266 files**.
Its canonical manifest SHA-256 is
`51926fef4ef21b03aca6a10a130532621004bd814fbe285320835b5e0290c762`
(sorted compact JSON of the path-to-SHA-256 map). Opt-in production-path test kernel:
`0bd88614a53a478febb7d49959f78086ac2bd5a17d6eb06d9d88c3e20db772ad`; ISO:
`dd7470f27d241ebdbd9bc0b540f6b252468c89e7432ebf5027e647c25e6fa327`.
The losslessly verified ISO archive and embedded production/test service image
hashes are in the stable primary directory. The source map matches before,
after, and again during final index creation. Compatibility source equality is
reported individually above. Earlier source-changing development runs remain
disqualified as stable acceptance evidence.

The disk had insufficient space for unrestricted duplicate images/compiler
cache growth. Only unused incremental compiler cache was relocated to /tmp
with access preserved at its original path, recorded in `cache-relocation.json`.
Development standalone ELF archives were deduplicated only after extracting the
identical ELF from their retained ISO and verifying both hashes. Old C and D3
failure evidence was not altered.

Material remaining issues are Account commit/storage withdrawal ordering,
restart-qualified session identity and ownership policy, missing native PID
reuse/service restart/critical lifecycle interleavings, incomplete global
reference/allocator reconciliation, and the finite boot token budget/reboot
non-aliasing boundary. None is waived by the new forced recipient-exit tests.

## 20. Readiness decision and Restricted SHM handoff requirements

**F1.1B-0: BLOCKED.** Minimum closure evidence is:

1. A reviewed production session authority/commit/revocation/storage contract
   and real synchronized S1–S10 transitions, including service incarnation.
2. Safe actual native PID reuse for issuer and recipient plus stale delivery,
   consumption, cancel, deferred reply and VM identity assertions.
3. Legitimate UAC/Account/VFS restart with outstanding pending/delivered grants,
   every required critical ordering, and no inherited private issuer privilege.
4. Remaining forced lifecycle races on the required SMP configurations, with
   observed participant/checkpoint identities, individual grant fingerprints
   and independent terminal states.
5. Cross-owner inventories, terminal transitions and quiescent allocator/frame
   measurements accounting for every relevant retained reference/allocation.
6. A sustainable and reviewed token non-reassignment/availability lifetime
   contract, including the boot budget and explicit reboot boundary.
7. Stable final-source regression results with new failures investigated and
   baseline exceptions explicitly bounded.

**Gate A: VERIFIED NATIVE. Gate B: PARTIAL. Gate C: BLOCKED.** Gate B's original
historical D3 assertion remains unexplained. No Restricted SHM implementation
may rely on a fictitious UNBLOCKED decision from this report.

A later authorized Restricted SHM phase must consume a trusted broker
authorization interface with exact recipient/VM binding, explicit pending versus
delivered ownership, independently attenuated producer/consumer mapping rights,
idempotent retirement, page-table permission checks and acknowledged TLB
retirement before frame reuse. Active revocation, VM borrower policy,
failure/teardown behavior and cross-process channel validation remain separate
requirements. This is a prerequisite checklist, not an implementation handoff
claim that the authority gate is complete.
