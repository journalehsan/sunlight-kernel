# FastLane F1.1B-0C: production grant authority closure

Implementation and primary evidence: 2026-10-22 (Asia/Tehran).
Resumed evidence review: 2026-10-24. Source base:
`a9e4d427df34deb3a2262cc91b661350e2a22cff`, with pre-existing version,
manifest and lockfile changes preserved. This report describes an implemented
safe subset and an authorization gate that remains **BLOCKED**. Restricted SHM
and production FastLane are not implemented by this change.

## 1. Executive technical summary

The real UAC daemon's `OP_BASE_CAPS` and `OP_DELEGATE` placeholders are replaced
with a fallible broker exchange of an already delivered root password proof.
The broker binds the resulting one-use VFS authorizations to the current IPC
caller's process incarnation and VM generation. The real VFS daemon consumes
them through `VfsMsg::AUTHORIZE_EXEC`. The consumer validates the current trusted
VFS image and authenticated IPC caller at the consumption boundary.

This is an explicit read/execute **policy authorization operation**. It does not
open a file, execute an image, change credentials, or retrofit the old VFS
OPEN/READ protocol. `runas` and ordinary spawn retain their existing behavior.
Consequently this report does not claim complete executable/file access
mediation from the new operation.

Native probes execute the actual password verifier, UAC dispatch, syscall,
broker, synchronous reply delivery, VFS dispatch, cancellation, exec and reaper.
Remote lifecycle invalidation at the critical boundaries, actual numeric PID
reuse, and legitimate service restart with outstanding grants are separate,
unverified acceptance requirements. CPU count and ordinary concurrent service
activity do not prove those races.

## 2. Original prerequisite blockers and baseline inspection

Read together with `fastlane-f11-grants.md`,
`fastlane-f11b-restricted-grants.md`, `fastlane-f11pr4-vfs-authority.md`,
`fastlane-f11pr41-mint-delivery.md`, the F1.1P/PR/PR2/PR3 secure issuance reports,
`fastlane-f12-spsc-ring.md` and `fastlane-f13nb-doorbell-acceptance.md`.

The R4.1 foundation already supplied exact recipient identity, delivery state,
serialized mint/consume, cancellation retirement and exec/exit cleanup for
Auth/Account. It did not supply production VFS delegation. At the start of this
phase UAC `OP_BASE_CAPS` checked a nonzero administrative value and returned a
prefix count; `OP_DELEGATE` returned the supplied nonzero value. Neither called
the kernel VFS mint gate. Repository search found no production callers of
these placeholder operations. The safe correction is a proof exchange that
fails closed, rather than treating these replies as delegated authority.

Remaining original requirements include real remote issuer/recipient lifecycle
interleavings, native PID reuse, restart ownership and complete session-policy
validation. The original combined D3 guest assertion is still unexplained.
Seven corrected historical D3 replays do not resolve that failure.

### Ordinary token issuance audit (C2)

The source inventory is retained as `issuance-source-inventory.txt`.
`CapabilityBroker::issue_token` is the shared path used by endpoint creation,
derivation, prepared SHM, Auth, Account, legacy VFS and the new VFS exchange.
No production VFS/UAC path found in this audit constructs a token with the old
predictable counter arithmetic. Test-only entropy injection is not enabled by
`production_grant_test`.

| Invariant | Evidence and limit |
|---|---|
| Unpredictability/readiness | `entropy.rs` collects a complete seed from virtio-rng/RDSEED/RDRAND, conditions via ChaCha20, excludes TSC/timing fallback and fails closed when unready. The existing ABI has 62 random payload bits plus type tags; this is not 128-bit bearer strength. |
| Counter exhaustion | `entropy/conditioner.rs::fill` preflights the 64-bit ChaCha block counter without partial output; exhaustion withdraws entropy readiness. Host known-answer/exhaustion regressions pass. |
| Live collisions | All five broker tables share collision checks, including retired SHM whose views remain live. Sixteen candidates maximum, reserved/zero selectors rejected, error returned on exhaustion. |
| Capacity/allocation | Global 8192 tokens, 4096 endpoints, bounded grant tables, fallible reserve before authority publication. Existing rollback tests and the new batch rollback tests assert inventory, not merely no panic. |
| Identity/call namespace | Process and VM incarnation allocation and call generation use checked exhaustion; generation zero is invalid. Exhaustion halts/fails rather than silently wrapping into a replacement identity. |
| Retired-value recurrence | **FAIL** under a repeated synthetic candidate after retirement: live-table collision exclusion is not a historical non-reuse guarantee. |

This audit supports the existing fallible conditioned-issuance subset; it does
not waive the strict non-reassignment prerequisite or certify a stronger token
entropy policy than the actual 64-bit ABI provides.

## 3. Production VFS call trace

| Stage | Actual source and contract |
|---|---|
| Origin | Requester authenticates through `sunlight_uac::auth::authenticate_password_for_session`, then calls the registered `uac` endpoint. No recipient PID is part of the new delegation ABI. |
| Delegation request | `services/sunlight-uac/src/bin/uac_service.rs::handle` dispatches opcodes 3/4 to `services/sunlight-uac/src/delegation.rs::handle`. It validates shape, target UID zero, one proof and a bounded canonical exact path where requested. |
| Kernel entry | `ipc/src/vfs_grants.rs::mint` invokes syscall 164. `kernel/src/arch/x86_64/syscall.rs::sys_vfs_grant` copies at most 63 path bytes before taking lifecycle serialization. The IPC raw syscall ABI places argument four in `r8`. |
| Commit | `kernel/src/capability/lifecycle.rs::Scheduler::delegate_vfs_grants` resolves the current trusted Auth issuer's actual reply target and the recipient's matching pending request. While holding SCHEDULER it takes CAP_BROKER and calls `CapabilityBroker::delegate_vfs_grants`. |
| Proof | Delivered, unexpired, uid-0 Auth proof belonging to that exact recipient incarnation and VM. Payload UID and token nonzeroness do not authorize issuance. |
| Rights/scope | Fixed `READ_EXECUTE` base policies `/bin` and `/usr/bin`, or one canonical exact command path. Base matching requires a `/` component boundary; exact command matching is equality. No write, map, credential, transfer or spawn authority. |
| Creation | Fallible conditioned-token issuance in the existing bounded VFS table. The entire batch is inserted under one broker guard; the proof is consumed only after all entries exist. Partial batch failure truncates only unpublished additions. |
| Publication | Records initially have `GrantDelivery { delivered: false, issuer identity, recipient identity, request generation }`. They cannot be consumed or resolved by the legacy bearer resolver. |
| Delivery | `kernel/src/ipc/mod.rs::deliver_reply_to_current_target` / `deliver_reply_to_target` validates exact pending call, incarnation, live state and deadline. `settle_grant_delivery` enables only the tokens actually listed in the reply. |
| Consumption | Requester calls real VFS `handle_request`, opcode `AUTHORIZE_EXEC` (17), with one token and a path. VFS invokes `ipc::vfs_grants::consume`; syscall 164 validates the current trusted VFS role and its actual current IPC caller before broker one-use consumption. |
| Withdrawal | Recipient syscall cancellation, IPC call cancellation/deadline/peer closure, recipient exec/exit, issuer pending-delivery cleanup and lazy expiry pruning use the existing table and lifecycle machinery. |

The existing syscall 100 remains a legacy bearer mint operation, restricted to
the historical broker slot **and** current trusted Auth role/live identity.
It is not a recipient-specific delegation operation. The new code never adopts
legacy VFS tokens into the bound consumer. Legacy VFS OPEN uses file handles;
large READ replies use the existing transferable SHM transport. Those are not
Restricted SHM grants and remain outside this new authorization operation.

## 4. UAC/Auth/Account call traces and lifetime separation

`handle_auth_password` maps the password transport, verifies passwd/shadow,
zeroizes credentials, and calls `SYS_MINT_AUTH_SESSION_GRANT` only for the
session-proof variant. It passes the kernel-written IPC badge, but
`Scheduler::issue_auth_session_grant` additionally requires that badge to match
the current incarnation-tagged reply target and pending call under SCHEDULER.
UID/GID returned by the verifier enter the broker proof. The new VFS exchange
accepts only uid zero and inherits the proof's original expiry; it does not
renew the password proof.

`account_service::{dispatch,dispatch_group,dispatch_profile}` obtains caller
credentials through `session_query_process(msg.badge)`, obtains the active
session from sessiond, checks account policy/password/revisions, and invokes
`ipc/src/accounts.rs::broker_grant`. `sys_account_grant` decodes the fixed
operation, revision, target/member and session scope. `Scheduler::account_grant`
binds it to the actual pending caller, incarnation and VM and mints/consumes via
CAP_BROKER. Account grants either return through ordinary reply settlement or
are consumed by the same authenticated UAC request before reply. The latter is
permitted only when the pending `GrantDelivery` exactly equals the current one.

Account session/revision authorization is still partly a userspace transaction.
An `active_session()` IPC check and later broker commit/storage publication are
not one atomic lifecycle transaction. The kernel compares the stored scope to
the supplied scope; it does not independently attest that sessiond's authority
is still active at that instant. Native withdrawal races and the storage commit
boundary remain **BLOCKED**, not inferred secure from scope equality. The
Wise Owl active-session registry has its own fixed-purpose policy and must not
be silently repurposed as Account session authority.

The `SessionStore` cached runas decision, active graphical login session, root
password proof and image trust have distinct lifetimes. Existing OP_RUNAS and
OP_CHECK/OP_CHECK_EXEC are cached/policy decisions, not bearer grant issuance
and not proof that a spawn is mediated by the new VFS operation.

## 5. Issuer authentication and authorization

Kernel IPC reply targets identify the actual requester; a numeric PID in message
words alone is insufficient. `grant_requester_for_role` requires live issuer
identity (nonzero process incarnation and VM generation), the current role, a
live recipient, and matching pending call generation/incarnation.

Trusted Auth provenance is installed only through the existing kernel-created
init -> trusted sunlightd -> `/sbin/uac_service` launch chain. The new trusted
VFS consumer marker is installed only on the kernel-loaded embedded VFS image
in `kernel/src/main.rs`. Both Process constructors default it to false;
`withdraw_exec_image_roles` clears it. A basename, reused PID or ordinary exec
cannot install it. New VFS consumption does not confer mint privileges.

SCHEDULER remains held from the final role/identity validation through broker
insertion or consumption. Exec/exit uses the same lifecycle serialization.
Thus invalidation ordered before commit is rejected; commit ordered before
invalidation follows the explicit ownership rules below. The native remote
interleavings must still be forced to validate the actual runtime integration.

## 6. Recipient identity and VM binding

`RecipientIdentity` is `(pid, foreground.incarnation, address_space generation)`.
`GrantDelivery` also carries issuer PID/incarnation and request generation.
The VFS delegation recipient comes exclusively from `ipc_reply_target.call.pid`;
no detached PID lookup is returned to userspace for later minting. Auth/Account
numeric requester arguments are cross-checked against the current pending
request, rather than accepted as delegation authorization.

No new grant is explicitly transferable. Successful recipient exec retires
these records before activating the replacement VM. Exit retires them before
slot reuse. A process-scoped cached UI decision is not a VM-bound kernel grant.

## 7. Broker state and commit/consume contract

No parallel grant registry or redundant state enum was added.

| Existing representation | Meaning and terminal transitions |
|---|---|
| Local prepared values | No authoritative record or observable token; failure returns without proof consumption. |
| Table record + `delivered=false` | Committed and visible to cleanup, pending publication. Consume denies. Issuer or recipient retirement/call cancellation removes it. |
| Table record + `delivered=true` | Listed in a successful exact-recipient reply. Eligible until expiry, recipient retirement, explicit cancellation or successful one-use consume. |
| Record absent | Consumed/retired/cancelled. Late settlement and repeated cleanup cannot reconstruct a record. |

Proof exchange and batch insertion linearize within the same CAP_BROKER guard,
inside SCHEDULER. A failed allocation/entropy/collision/capacity reservation
rolls back only the unpublished batch; the original proof remains eligible for
retry. A successful batch consumes the original proof once. `VfsGrant` extends
the existing VFS table with delivery/expiry metadata; it holds identity values,
not owning Process/VM references or frame references.

## 8. Cancellation and ambiguous publication

Delivery eligibility is recorded before the synchronous reply becomes runnable,
under the same lifecycle lock. Settlement removes omitted or unsuccessfully
published pending entries, including a partially listed batch. Duplicate
settlement/cancellation is idempotent. Delivered records are not rolled back
merely because the issuer later sees failure: token bytes may already have been
observed. The recipient can explicitly cancel its exact-bound record; exec/exit
also retires it. Legacy bearer records cannot enter this consumer.

**Unresolved non-reuse requirement:** `issue_token` excludes currently active
token values across all broker tables, but keeps no retired-value history.
Conditioned random generation makes recurrence probabilistic, not impossible.
An isolated host counterexample compiles the actual broker and repeats a synthetic
entropy candidate after endpoint destruction; the non-reassignment assertion fails.
Raw source, command and failure are retained in `token-reuse-counterexample/`.
The first harness resolution failed because cached spin 0.9.8 was yanked; copying
the existing workspace lockfile into the isolated harness permits the offline
reproduction. No production entropy source or live identity is forged.
Retirement must not be described as a deterministic guarantee against reissuing
a previously observed value. This is a precise secure-issuance prerequisite
blocker under this task's strict non-reassignment requirement. It is not repaired
by increasing retries, hiding cancellation failures or weakening identity checks.

## 9. Exec/exit policies

| Class | Lifetime and owner |
|---|---|
| Trusted Auth/VFS role | Image-specific and non-inherited; withdrawn on successful exec, unusable once exiting. |
| Auth password proof | One-use, exact process incarnation + VM, short expiry (existing 500 tick TTL), pending issuer-owned until delivery. Does not claim cached runas/graphical session lifetime. |
| New VFS authorization | One-use, exact process incarnation + VM, inherits proof expiry, recipient-owned after delivery, no onward transfer. |
| Account grant | One-use operation/revision/session scope plus recipient incarnation/VM. Same-request UAC consumption allowed; active-session withdrawal/storage serialization remains unproven. |
| Legacy VFS/SHM bearer | Existing transfer/inheritance policy preserved. This phase does not claim active mapping revocation. |

Issuer exit/exec retires its pending records. Already delivered recipient-owned
VFS grants survive issuer exit until their own expiry/consumption/retirement.
Recipient exit or successful exec retires both pending and delivered records.
Repeated cleanup removes no references twice. `exec_into_process` withdraws
roles and grants before replacement VM activation; failed preparation preserves
the old image's authority. Remote pin release and reaper safety remain governed
by existing lifecycle code.

## 10. PID reuse coverage

Host tests substitute a new incarnation/VM under the same numeric PID and reject
old tokens and stale reply generations. Existing R4.1 fixtures additionally
exercise stale-incarnation reply cancellation. These do not demonstrate native
numeric PID reuse. Normal native process **slot** reuse was observed and exposed
a bug in the first probe's completion observer; slot reuse is not numeric PID
reuse. N5 remains **NOT FORCED** until supported native PID allocation/reuse
executes a replacement with a genuinely new kernel identity.

## 11. Restart coverage

Host production-method tests retire pending issuer-owned grants, preserve
already delivered recipient ownership and reject a roleless replacement issuer.
The compatibility replay separately exercises SunlightD ownership and
RecoveryBlocked. Neither is a native UAC/VFS restart with outstanding grants.
N7 remains **NOT FORCED**; no recovery/adoption authority is granted by this
phase. A restarted VFS also needs legitimate consumer-role installation; only
the existing embedded boot VFS receives the new role in this implementation.

## 12. Narrow changes and runtime work

New modules: `ipc/src/vfs_grants.rs`, `kernel/src/capability/vfs_grants.rs`,
`services/sunlight-uac/src/delegation.rs`. Existing issuance, settlement and
retirement tables/locks are reused. Syscall 164 and VFS opcode 17 expose only
this fixed-purpose proof exchange/one-use authorization. Test code is in
`production_tests.rs`, opt-in `production_gate.rs`, the feature-gated
`grant_probe` binary and `tools/fastlane_f11b0c_validate.py`.

A new authorization call acquires SCHEDULER then CAP_BROKER and performs bounded
existing-table searches. The VFS table limit remains 64; global active-token
limit remains 8192. Reply settlement and lifecycle retirement now inspect this
bounded VFS table in addition to Auth/Account. There is no IPC delivery or
userspace copy while mint/consume owns these locks. No production polling was
added. Test-only timer/reaper observations do not change dispatch policy, pin
release, lock admission, or ready queues. No throughput or latency improvement
is claimed; matched lifecycle lock timing is not measured.

## 13. Host regression evidence

Raw logs and exact commands are retained below `target/fastlane-f11b0c-evidence`.
The final kernel host suite has 104 passing tests, including 13 new production
method tests. These compile the actual broker/lifecycle methods with host-only
process/VM/IPC identity shims, not the full native scheduler.

| Required scenario | Host evidence/boundary |
|---|---|
| Issuer exit/exec before commit; recipient exit before commit | Barrier orders completed invalidation before the issuance thread; no insertion or authority remains. |
| Role/identity check -> commit | Hook pauses actual method before broker insertion while lifecycle mutex remains held; cleanup cannot acquire it. |
| Recipient exit/exec after commit before delivery, PID reuse | Bound broker records retired; late settlement cannot resurrect; new incarnation/VM cannot consume. |
| Cancellation vs exit; retirement vs consume | Existing bounded interleavings plus 32 threaded consume/retire starts; exact final inventory and replay denial asserted. No native race claim. |
| Partial/failed/ambiguous publication | Empty/subset/full token lists, failure after success, duplicate settlement; only actually delivered eligibility survives. |
| Restart | Pending issuer-owned removed, delivered recipient-owned preserved, replacement image role denied. |
| Two issuers / one VM | Two host threads serialize distinct issuer identities and deliver four grants to one recipient; issuer retirement does not adopt delivered ownership. |
| Allocation/entropy/batch collision | Fault injection leaves proof available and no partial VFS batch. |
| Capacity/cycles | 64-record exhaustion, 128 create/deliver/consume/cancel cycles, no inventory or vector-capacity growth after warmup. |

A newly added two-issuer test initially used a stale modeled reply target and
failed, poisoning its shared host test mutex. The fixture was corrected to
establish each issuer's current pending reply target. That failed run and the
passing replay are retained. Production authorization was not weakened.

UAC/IPC/VFS/Ring/Doorbell and established service host suites are separately
recorded. Parser tests use synthetic transcript text solely to verify evidence
rejection; synthetic text is never native grant evidence.

## 14. Native matrix and probe boundaries

The final native `production_grant_test` fixture launches UID/GID 1000 clients
without trusted service roles, pinned to actual CPUs. Earlier development
attempts used the constructor default UID zero without service roles; they do
not establish the final non-root requester property. They use the real nameserver, development password verifier,
UAC and VFS. The fixture never installs trusted roles, changes entropy readiness
or adds debug syscalls. The third real UAC request is cancelled after broker
commit and before reply, through the normal IPC cancellation operation twice.
Actual successful recipient exec replaces the VM; the old token is probed and
denied. A test-only reaper observation records completion before normal slot
reuse can overwrite the old Process record.

Per final client: six real password-proof exchanges produce eleven VFS records;
nine are delivered, four consumed, three explicitly cancelled, two retired by reply
cancellation, and two retired at exec. Final fixture VFS inventory is zero.
Counters observe actual operations, with an explicit balance assertion. Serial
markers record issuer PID/incarnation/role, recipient incarnation/VM, request,
commit batch, delivery, consumption, lifecycle CPU, reaper and inventory. No
raw authority values are printed.

Native matrix results and attempt history are recorded in the final evidence
index. N1 (base and exact-path delegation), N3 (exec **after** delivery), and N6 (real reply
cancellation) can pass for the exercised scope. N2, remote exec during delivery,
N4, N5 and N7 remain **NOT FORCED**. An inter-CPU commit/delivery log is not a
forced remote lifecycle race. The final native client exercises both `OP_BASE_CAPS` and exact-path
`OP_DELEGATE`, including proof replay rejection, exact-path sibling denial and
one-use consumption. Earlier development images exercised base delegation only.

Final run: `target/fastlane-f11b0c-evidence/20261021T231849.705180Z/`.
The source hashes remained unchanged throughout this run. All requester clients
asserted UID/GID 1000 and had no trusted service roles.

| CPUs | Result | Committed | Delivered | Consumed | Owner / delivery / exec retirement | Final inventory |
|---|---|---:|---:|---:|---|---:|
| 1 | PASS, exercised subset | 11 | 9 | 4 | 3 / 2 / 2 | 0 |
| 2 | PASS, exercised subset | 22 | 18 | 8 | 6 / 4 / 4 | 0 |
| 4 | PASS, exercised subset | 44 | 36 | 16 | 12 / 8 / 8 | 0 |
| 12 | PASS, exercised subset | 132 | 108 | 48 | 36 / 24 / 24 | 0 |

The exact replay command was
`CARGO_INCREMENTAL=0 python3 tools/fastlane_f11b0c_validate.py --checks --native --cpus 1 2 4 12`.
Kernel target check, production service/kernel builds, ISO creation and all
host checks in that invocation passed. This matrix does not upgrade any of the
explicitly NOT FORCED lifecycle probes.

`target/fastlane-f11b0c-evidence/validation-summary.json` indexes the final run,
all earlier attempts and compatibility evidence. Completed images are retained
as verified lossless gzip archives to limit disk use. `archived-images.json`
records original and archive SHA-256 values. Where a standalone ELF archive was
deduplicated, its identical bytes were verified inside the retained ISO at
`boot/sunlight-kernel.elf`; its index entry identifies that ISO archive. A logged
original image pathname may therefore need reconstruction before replay.

The initial single-CPU probe passed. Its first matrix timed out on 2/4 CPUs
because the observer assumed all reaped Process records remained present at one
timer sample. Serial logs showed successful exec, zero grant inventory and real
reaping followed by normal slot reuse. The old 12-CPU attempt happened to pass;
these are retained as fixture failures, not discarded retries or authorization
successes. The corrected observer records the actual reaper transition.

## 15. Metadata and lifetime accounting

The new record owns no frames or process/VM references. Token/grant counts refer
to the fixture's recipient-bound VFS records, not the entire live OS capability
table. The daemon's endpoints, ordinary SHM password transport, resident services
and kernel metadata remain live and are not part of a fictitious global zero.

Host cycles assert empty Auth/VFS inventories and stable Vec capacities after
warmup. Native logs also record actual VFS record size, table length/capacity and
Auth table length/capacity; retained Vec capacity is not called zero allocation
drift. Native counters balance committed = consumed + owner-retired +
delivery-retired + exec-retired, with no pending fixture grants or identity
references left. This does **not** measure allocator bytes, total kernel metadata
drift, physical frame drift, or independent pending lifecycle-cleanup queues.
Those quantities are null/unmeasured in the JSON and remain acceptance gaps.
Capability record retirement is not active revocation of existing SHM mappings.

The final native VFS record size was 160 bytes. Empty VFS tables retained
capacities 4/4/8/32 on 1/2/4/12 CPUs respectively; empty Auth tables retained
capacities 4/4/4/16. These are observations of retained table storage, not a
measurement of total allocator drift.

## 16. Compatibility replay and known baselines

The retained compatibility driver runs the existing R4.1 secure issuance and
lifecycle gate, entropy-unavailable fail-closed boot, resident security/SHM,
scheduler SMP, VM/TLB/unmap/protect, SunlightD RecoveryBlocked, pin-release and
native Doorbell matrices, plus D1/D2/D3. Test-specific init/sunlightd/startup
ELFs are rebuilt with ordinary features before later resident/transport tests;
Cargo embedding does not itself prove userspace source freshness.

Final replay statuses, command arguments, image/source hashes, CPU configurations,
failures and retries are indexed in `validation-summary.json`. TCP/TLS requires
its own certificate-verifying native replay and is never inferred from boot.
Known historical Clippy/format/media failures must be compared to source and
retained diagnostics; a historical label alone does not waive a new failure.
Miri availability is recorded. No Git state is restored to manufacture a
baseline; when no matched reproduction is available the status is
**MATCHED BASELINE UNAVAILABLE**.

| Replay | Current result and boundary |
|---|---|
| Final kernel host | PASS: 104 tests, including 13 production-method tests. |
| Final service host | PASS: audiod 25, ELF 11, Ring 8, Doorbell model 5, FS 73, IPC 62, UAC 18, SunlightD 29. |
| Scheduler host / parser | PASS; parser has five evidence-rejection tests. |
| R4.1 native authority / resident | PASS at 1/2/4/12 CPUs; entropy-unavailable boot fails closed as expected. |
| VM/TLB / unmap / protect | PASS at 12 / 4 / 4 CPUs respectively. |
| Scheduler SMP / RecoveryBlocked | PASS at 1/2/4/12 and 1/2/4 CPUs respectively. |
| Pin-release / native Doorbell | PASS at 1/2/4/12 CPUs; actual forced-case records retained separately. |
| SunlightD D1/D2/D3 | PASS at 1/2/4 CPUs, 18 dependency cycles; original historical D3 assertion remains unexplained. |
| TCP/TLS | PASS at 1/2/4/12 CPUs: untrusted issuer and wrong certificate hostname rejected; repeated verified HTTPS and SOCKS hostname paths pass. |
| Changed Rust formatting / whitespace | PASS for 23 changed Rust files' added lines; `git diff --check` PASS. |
| Whole-workspace formatting | FAIL; retained existing formatting differences. No complete matched baseline replay. |
| Strict Clippy | FAIL at unchanged build-script range check and existing IPC `native_poll` unit-error API. |
| Media host | FAIL: 36 pass, one missing repository WAV fixture (`NotFound`); decoder source unchanged. |
| Miri | Unavailable for the installed nightly; no Miri execution claimed. |

`baseline-comparison.json` records HEAD-equal hashes for the failing build
scripts and UAC capability/session modules, plus the byte-identical existing
`native_poll` helper. The range and unit-error diagnostics also appear in the
retained R2D log. A diagnostic run allowing only the first existing kernel
category stopped at another unchanged nested SVG build-script diagnostic,
before linting the kernel. Service libraries passed a separate diagnostic run
with four explicitly recorded existing lint categories allowed. This is not a
strict Clippy pass, and no production lint suppression was added.

D3's whole-source hash check was false because the opt-in production probe and
its two evidence tools changed during D3 execution. The four differing files
were `production_gate.rs`, `grant_probe.rs`, `fastlane_f11b0c_validate.py` and
`test_fastlane_f11b0c.py`; they are not executed by the dependency-test image.
The D3 build snapshot and final production snapshot are retained separately.
This qualification is preserved rather than reporting a stable final-tree D3
run. No production delegation source changed during that replay.

The TCP/TLS evidence uses the retained resident image, with its SHA-256 checked
again on resume. The first attempt could not create sandbox sockets. The first
approved replay lost keyboard characters in a long HMP `sendkey` command and
timed out waiting for the exact command marker; that is retained as FAIL. A
local copy of the existing gate replaced timer-based key holds with explicit
paired QMP key-down/key-up events. Its commands, certificate checks and 180-second
marker timeout were preserved. The original tool's source was not changed.
The paired-event fixture passed on 1/2/4 CPUs on October 22. Its first 12-CPU
run was interrupted and never established a complete result; it is retained as
BLOCKED. A fresh-certificate replay passed on 12 CPUs on October 24.
`tcp-tls-certificate-assertions.json` independently indexes the actual
`UnknownIssuer`, `NotValidForName` and verified IP/DNS handshake markers for all
four completed runs. No interrupted run or expected nonzero exit is treated as
an entire successful certificate-verification replay.

Native Doorbell Gate B remains **PARTIAL**, even if every new grant probe and
corrected compatibility replay succeeds. The original combined D3 assertion
has no demonstrated root cause. F1.2 and F1.3 implementation source is unchanged.

## 17. Remaining security limitations and exact blockers

1. Retired observable token values are not permanently excluded from subsequent
   issuance. Demonstrate an appropriate bounded, exhausted-safely non-reuse
   contract without weakening conditioned bearer unpredictability; active-only
   collision checks are insufficient for the strict requirement here.
2. Force actual remote issuer exec/exit before commit and recipient exit/exec
   between commit and delivery, with independent progress and real CPU IDs.
   Current SCHEDULER serialization gives a source/host contract, not that runtime
   proof. Hooks must not wait for another CPU while holding the lifecycle lock.
3. Exercise native supported numeric PID reuse and present old authority to the
   replacement incarnation. A new VM generation or reused array slot alone is
   insufficient evidence.
4. Perform legitimate UAC/VFS restart with pending and delivered grants. Preserve
   RecoveryBlocked and establish the trusted restarted VFS consumer through a
   valid image launch policy, rather than accepting a name/PID.
5. Close and validate Account active-session withdrawal and storage publication
   boundaries. Root password proof lifetime must remain distinct from cached
   runas or graphical session lifetime.
6. Record actual metadata allocation/reference and pending-cleanup accounting
   through repeated native cycles. Grant-table zero and host vector capacity
   alone do not establish this.
7. Demonstrate the complete required production-entry matrix, including failure after observable publication and the required
   compatibility replays with their evidence qualifications resolved.

The explicit VFS policy operation does not retrofit legacy file I/O or spawn
mediation; that remains a scope limitation, not an instruction to start a new
general-purpose access-control redesign. Doorbell Gate B's unresolved D3 failure
is a separate activation constraint and does not itself decide F1.1B-0.

## 18. Explicit gate decision

**F1.1B-0: BLOCKED.** Independently safe UAC/VFS wiring, fail-closed proof exchange,
exact-caller binding and tested cleanup are preserved. This report does not
certify production grant authority closure while the above security requirements
remain defective or unverified.

Scheduler pin-release Gate A's historical status is VERIFIED NATIVE, with current
replay results recorded separately. Native Doorbell Gate B is PARTIAL.
Production FastLane Gate C is BLOCKED. No current result silently upgrades any
of those gates.

## 19. Restricted SHM handoff

Before starting Restricted SHM, satisfy each blocker with source changes and
bounded evidence tied to actual production call paths. Then reassess F1.1B-0
against all fifteen acceptance conditions. An UNBLOCKED prerequisite would
permit starting F1.1B; it would not mean that phase was implemented.

Backing provenance, peer RO/RW/NX mappings, shared-VM/clone policy, legacy SHM
bypass prevention, active mapping revocation, verified unmapping, TLB
acknowledgement and safe mapping teardown remain entirely in the later phase.
Production activation also requires the separate Doorbell Gate B decision.
