# FastLane F1.1P-R4.1: mint-to-delivery authority

Status: **safe subset implemented; full R4.1 acceptance and F1.1B-0 BLOCKED**.
Base revision: `6b29dbad1` (uncommitted R4.1 source hashes and kernel/image
identities are captured with the evidence). No restricted SHM, ring, Doorbell
or production FastLane transport is introduced. This is the final bounded
hardening milestone in this sequence, not the start of another redesign.

## Technical outcome

R4 authenticated a VFS issuer at mint but did not establish token delivery.
This phase repairs a concrete gap in the **existing production UAC auth/account
path**: minted grants were immediately broker-valid even if the IPC reply was
cancelled or dropped. They also lacked an explicit process-incarnation binding.
Authentication consumers released lifecycle serialization after their trusted
role check and before consuming a PID-selected grant.

Auth/account records now carry their exact recipient PID, process incarnation,
VM generation, issuer incarnation and originating request generation. State is
embedded in the existing broker records; no second authority registry exists.
Pending authentication tokens are not usable. Successful IPC reply publication
marks the listed grants delivered; omitted or failed grants are retired.
Cancellation/deadline/peer-close retires that request's pending grants. Cleanup
retires recipient records and a retiring issuer's undelivered records. Delivered
recipient-owned grants can outlive their issuer, but not recipient exec/exit.

The account service also uses grants entirely **inside one authenticated UAC
request**, for own-password/profile updates. That existing behavior remains
valid: trusted UAC may consume its own pending account grant only against the
same exact issuer/recipient/request binding. A different request cannot adopt
it. Delivered account grants can be consumed on a later authenticated request
with the same recipient identity and existing scope/session/revision policy.

IPC call IDs now include process incarnation, and call generation allocation
uses checked addition rather than wrapping. Stale immediate/deferred replies
cannot match a replacement process just because PID and request generation
match. Live consumers keep SCHEDULER through role validation, exact recipient
lookup and broker consumption. No userspace wire layout or syscall changes.

These repairs do **not** invent authority for the unimplemented VFS/UAC
base-capability forwarding API. PID-6 plus trusted-role VFS mint restriction is
retained. Production VFS delegated mint/delivery therefore remains blocked.

## Actual service call-flow inventory

All paths below are repository-relative. IPC badges identify the actual caller;
an independent payload PID or UID does not inherit that authentication.

| Production trace | Authentication / authorization and recipient | Commit, transport, acceptance and lifetime |
| --- | --- | --- |
| `services/sunlight-uac/src/bin/uac_service.rs::handle_auth_password` → `SYS_MINT_AUTH_SESSION_GRANT` → `kernel/src/arch/x86_64/syscall.rs::sys_mint_auth_session_grant` → `capability/lifecycle.rs::issue_auth_session_grant` | UAC verifies supplied credentials, then uses the kernel badge as candidate requester. Kernel requires current live `trusted_auth_broker`, matching immediate reply target and the recipient's pending call, including incarnation. UID/GID are UAC's trusted authentication result, not authorization supplied by an untrusted requester | Broker creates a VM-bound single-use auth record, Pending and recipient-owned. UAC returns it with `with_cap`. `ipc_reply_and_wait` → kernel reply delivery validates the same call and marks that token Delivered. Login/session/lock or authenticated spawn consumes it with exact live recipient identity; expire, consume, recipient exec/exit or pending cancellation closes it |
| `services/sunlight-uac/src/bin/account_service/mod.rs` AUTHORIZE → `ipc/src/accounts.rs::broker_grant` → `sys_account_grant` → `Scheduler::account_grant` | Actual synchronous badge plus current trusted UAC role. UAC checks authenticated credentials/session/account policy. Scope includes operation/target/member/revision/session and session generation; kernel resolves live recipient, incarnation and VM | Pending account token returned in IPC reply and delivered by the same kernel hook. Later MUTATE_GROUP consumes once under a new authenticated request with identical recipient VM/incarnation and matching policy binding |
| Same account service: CHANGE_OWN_PASSWORD / UPDATE_OWN_PROFILE | Current request and trusted issuer authenticate the operation; service rechecks active session and account state | Service mints then consumes without transporting the intermediate token. Exact same pending issuer/recipient/request binding is required. The grant is removed before the existing filesystem publish operation. No new mint/delivery API is implied |
| Auth consumers: `sys_lock_auth_consume`, `sys_session_auth_consume`, `handle_spawn_call` authenticated branch | Consumer role and non-exiting state checked under SCHEDULER; presenter/owner PID is candidate lookup only, never identity proof. Broker requires Delivered plus exact recipient incarnation/VM. Authenticated spawn uses the actual current caller | Broker removes the grant even on mismatch/expiry, preserving existing fail-closed one-use behavior. Consumer check and consumption no longer have an unlock gap. Subsequent service-specific actions remain their existing protocols |
| `services/vfs_server/src/main.rs::handle_request` → `open_path` / `read_handle` | Legacy daemon backend handles and existing filesystem policy; this is not syscall-100 capability minting. No recipient-incarnation grant record is created | Small replies carry data/handles. Large reads allocate legacy SHM and return DATA_SHARED with bearer token. Neither that token nor raw handle is a new recipient-bound VFS capability. Existing client/server view cleanup remains the legacy policy; failed-delivery backing transfer is not proven here |
| `uac_service.rs::handle`, OP_BASE_CAPS / OP_DELEGATE | Existing code checks only a non-invalid admin token and target UID from payload; no verified delegation-to-mint chain | BASE_CAPS returns a count; DELEGATE echoes a supplied token. Real kernel mint forwarding remains TODO. Neither proves delegated administrative authority or a newly minted VFS capability. Preserved, not promoted into a trusted contract |
| `sys_grant_capability_syscall` → `Scheduler::issue_vfs_capability` → broker VFS push | Current caller must satisfy retained legacy slot and trusted launch role, live incarnation/VM. No recipient argument exists | Transferable bearer returned in a register to issuer; no combined target delivery, no per-holder recipient binding or generic per-token VFS revoke. No production consumer of `vfs_allows` was found. It is not enforced end-to-end filesystem authorization |
| `ipc/mod.rs::handle_ipc_call`, `IpcBus::enqueue_call`, receive, reply/deferred reply | Kernel constructs call PID/incarnation/nonwrapping generation and overwrites sender badge. Registration checks endpoint ownership/service identity. Public discovery SEND rights are not mint authority | Transport carries token bytes; only pending auth/account records tied to this exact call participate in the new delivery transition. Ordinary IPC/VFS/SHM bearer lifetime remains unchanged |

Trusted service startup is unchanged: bootstrap identity → supervised sunlightd
→ embedded `/sbin/uac_service` sets the auth-broker marker. Exec withdraws that
marker. VFS daemon startup/registration does not grant UAC mint authority.
Production role installers are not weakened for probes; the pre-existing opt-in
isolated fixture installs synthetic roles only in its own private processes.

## State, ownership and publication

| State / transition | Authoritative fact | Owning cleanup |
| --- | --- | --- |
| Prepared | Userspace verified credentials/input, but no kernel grant or validated recipient reference is retained across an unlock | No grant to retire |
| Committed / Pending | Broker insertion and identity attachment under the same scheduler and broker guards. Recipient owns the record; issuer has token bytes for delivery. Auth is not usable yet | Recipient cleanup or retiring issuer, terminal request cancellation, omitted/failed reply, expiry/consume |
| Delivered | Kernel validated exact live call and VM and stored the reply with that token. Delivery state changes under the same guards before waking recipient | Recipient exec/exit, expiry or one-use consumption; issuer exit does not revoke delivered recipient authority |
| Pending account consumed internally | Same live trusted issuer and exact original request, recipient incarnation/VM and scope | Record removed once; never becomes a transported grant |
| Cancelled / Revoked / Closed | Record removed from its authoritative vector | Duplicate removal/settlement does nothing; there is no separate retired-record or reservation list |

Token bytes observed early do not make a Pending auth grant eligible. Transport
publication means storing the kernel reply and terminal outcome, not proof that
userspace has scheduled or read a register. Delivery winning cancellation leaves
the delivered record recipient-owned; cancellation cannot erase that terminal
result. Lifecycle cleanup still withdraws it if the recipient exits before
running. Cancellation winning removes pending authority before a late reply is
dropped. No copy-out into a supplied userspace buffer is involved in these grant
returns; partial fallible copy-out is inapplicable to this scalar register ABI.
This does not prove unrelated shared-buffer transfer rollback.

Record removal never overwrites a live token. Existing conditioned entropy,
active collision exclusion, 16 candidate attempts and capacity limits remain.
Historical token recurrence after removal is not proven impossible, and this
phase does not add a historical-token ledger or claim generation-bound restricted
memory authority from an ordinary bearer.

## Lifecycle order and locks

SCHEDULER serializes identity/role checks, commit, delivery decision, cancellation
and recipient exec/exit. Broker guards precede IPC shard guards. Reply, reply-wait,
deferred completion and cancellation explicitly receive the already-borrowed
broker; no new broker acquisition occurs while holding a shard. Foreground
cancellation obtains broker before shard. Deadline expiry obtains broker only
when an actual deadline fires; no new timer poll/retry loop was added.
Peer-close obtains broker after endpoint queue removal has released its shard.

The critical work is bounded by the existing two grant tables (64 records each)
and token candidate limit. New settlement scans those tables; it performs no
allocation, entropy wait, userspace fault/copy, blocking IPC or TLB wait. VM/SHM
reclamation and exec kernel-stack handoff are unchanged. Lock-hold distributions
are unavailable; compatibility safety gates do not establish unchanged latency.

| Ordering | Guaranteed result / evidence boundary |
| --- | --- |
| Recipient cleanup before mint | Live lookup rejects; existing post-cleanup host/native regressions retained |
| Mint before cleanup | Committed Pending or Delivered record belongs to recipient; serialized cleanup sees it |
| Cleanup after mint, before delivery | Record removed; stale reply cannot adopt new incarnation; host verified. Remote native recipient-exit boundary remains unverified |
| Recipient exec before delivery | Old record withdrawn; exact VM mismatch independently rejects settlement. Host verified; real remote exec-before-reply native race remains unverified |
| Delivery before recipient exec | Delivered account records withdrawn before new VM activation; native fixture checks actual boundary |
| Issuer exec before commit | Role withdrawal excludes new commit under same scheduler serialization; host coverage plus native role withdrawal, no remote native precommit pause |
| Issuer exit during delivery | Pending issued records retired; Delivered recipient records survive until recipient cleanup. Host verified |
| Cancel before delivery | Pending records removed, terminal Cancelled, late reply dropped. New native controlled boundary verified |
| Deliver before cancel | Delivered terminal state wins; duplicate cancellation cannot reclaim a token that the recipient may have observed. Host state coverage and existing native terminal-result regressions |
| PID reuse | Call incarnation plus nonwrapping call generation prevents old reply/request adoption. Host exact same-generation replacement tested. Native injects stale incarnation at real reply boundary, not actual process-slot reuse |

## Implementation and host regressions

Changes are limited to auth/account broker records and consumption, IPC call
identity/terminal settlement, existing foreground cancellation callers, native
fixtures, scheduler proof stub signature, and this phase's validation runner.
Raw PID-only auth/account consumption is compiled only for existing raw broker
unit tests; production consumers use the bound methods. Existing VFS bearer
semantics, UAC policy, service launch rules and userspace ABI are retained.

Nine additional host regressions bring the kernel suite from 82 to 91:
Pending usability, successful delivery/consumption, failed/omitted/duplicate
settlement, cleanup/exec/VM mismatch, stale incarnation with identical pending
generation, early token observation, delivered authority inheritance, delivered
token identity mismatch, 64 bounded metadata cleanup cycles, and UAC's internal
same-request account consumption. Several cases share a matrix test. Existing
issuer role/exec, reservation/entropy/capacity, multi-grant and commit/cleanup
barrier tests remain. The issuer-exit preservation test now explicitly delivers
the grant before checking the recipient-owned lifetime.

| Requested host case | Scope established |
| --- | --- |
| 1 VFS success | Blocked: production delegation policy/wiring absent |
| 2 UAC success | Production issuance/settlement/consumption methods verified with synthetic scheduler; no host userspace/service loop claim |
| 3–5 invalid role, issuer exec, cleanup before commit | Existing deterministic regressions retained; incarnation check added |
| 6–9 cleanup after commit, recipient exec, issuer exit, PID reuse | New Pending/Delivered state matrices and exact replacement identity tests |
| 10–11 failed/interrupted IPC delivery | Authoritative settlement methods verified on host; actual transport cancellation exercised natively |
| 12 partial copy-out | Inapplicable for scalar register grant replies; shared-buffer publication remains separate |
| 13–15 duplicate cancellation, revocation/delivery, concurrent grants | New idempotent state matrices plus existing bounded threaded commit/cleanup barriers; not every ordering is true concurrent execution |
| 16–17 reservation failure and capacity | Existing broker/issuance regressions retained |
| 18 repeated cycles | 64 synthetic identity/VM/metadata cycles; does not measure physical frames |

An initial host run exposed a changed pending-versus-delivered expectation in the
issuer-exit test; it was corrected by explicitly completing delivery, not by
waiving cleanup. Scheduler host initially failed because its isolated foreground
mock used the old cancellation signature; the mock now accepts the broker borrow.
Original failure logs and interrupted development validation attempts are retained.
The final frozen-source run is the acceptance evidence for the safe subset.

## Native assertions and limitations

The opt-in fixture uses real isolated user address spaces, real syscall entry,
real IPC receive/reply and trusted kernel issuance/consumption, with synthetic
launch roles. It is **not** the production password-checking UAC daemon and is
not a production VFS mint flow.

The first actual issuer reply is a deterministic boundary: two grants have
committed but neither is delivered. The hook attempts the same reply with a
wrong process incarnation, asserts that the real request and its two records
remain Pending, calls the production IPC cancellation handler twice, then asserts
Cancelled and zero records. The original reply is dropped. The real userspace
recipient observes cancellation, retries, and completes sixteen ordinary
mint/reply operations. The trusted-consumer syscall consumes fifteen delivered
auth grants. One auth grant and sixteen account grants remain for actual exec
withdrawal and terminal reaping checks.

Required markers include `[CAP-R41] START` with PID/incarnation/VM/request,
`boundary stale-incarnation=denied real-call-cancel=retired duplicate=stable
pending=0`, and `COMPLETE delivered_auth_consumed=15 ... pending=0
active_auth_account=0 cpus=N`. The existing grant totals change to 17 auth and
17 account commits because the cancelled first call also committed. Native
exec's preactivation withdrawal now observes 17 records (one auth and sixteen account), rather than
32 undelivered auth/account records. This preserves the original lifecycle
regression while exercising actual delivery and auth consumption.

| Probe | Disposition |
| --- | --- |
| A real authorized production service | Partial: actual kernel-mediated synthetic auth/account flow and authorized use; production UAC credential-service loop and VFS flow unverified |
| B recipient exit during delivery | Unverified; cancellation is not relabelled as exit |
| C issuer exec trust withdrawal | Existing actual exec role withdrawal checked; remote issuer precommit ordering unverified |
| D incarnation change | Stale-incarnation reply injected at real native boundary; actual PID-slot replacement during delivery unverified |
| E controlled delivery failure | Actual pending-call cancellation, duplicate cancellation, late reply drop and zero pending grant inventory verified in the new fixture |

The cancellation/stale-reply orderings are deterministic sequential transitions
under serialization, not simultaneous CPU execution. SMP runs also retain the
existing parallel attacker/issuance workload. No serial logging occurs in a
production timed hot path; probe tracing is bounded and opt-in.

## Independent gate and handoff

**F1.1B-0 BLOCKED**, with these current-boundary prerequisites unresolved:

1. Production VFS/UAC delegated mint-and-delivery policy and wiring. Nonzero
   admin token plus echo/count is insufficient authority; the legacy PID slot
   is not a service identity. `vfs_allows` is not a production filesystem gate.
2. Production UAC credential-service native flow through intended recipient
   delivery and actual service consumer, including failure handling, has not
   been validated end to end.
3. Native controlled recipient exit/exec before delivery, issuer exec before
   commit, actual process-slot reuse and repeated fresh-process resource cycles
   remain unverified. Stale-incarnation/cancellation probes narrow this gap but
   do not substitute for those assertions.
4. Legacy VFS large-read SHM/handle failed-delivery lifetime is not covered by
   the new auth/account state. It cannot be used as a secure restricted-backing
   transport by assumption.

Secure ordinary token construction, entropy readiness, bounded collision and
capacity failure, existing transactional endpoint/SHM preparation, exec role
withdrawal and lifecycle commit/cleanup remain separate validated/audited facts.
Pending-to-delivered auth/account authority and exact consumer identity are the
repairs in this phase. A green compatibility suite does not close the four
prerequisites above.

Future F1.1B **implementation requirements**, not completed features or extra
reasons by themselves to forbid starting a safely gated implementation, are:
restricted backing provenance, exact per-recipient rights, RO/RW/NX PTEs,
legacy writable-alias prevention, active mapping revocation, clone/shared-VM
policy and TLB-backed mapping lifetime. They remain deferred and unavailable.
Historical token nonrecurrence, deferred-reply handle counter exhaustion and
broader daemon backend authorization require separate, scoped security review;
this task does not recursively implement them.

Handoff: retain this verified auth/account subset and remediate the unresolved
production service policy/transport and native lifecycle tests as a separate
security workstream. Native restricted FastLane mapping stays unavailable.
The next independent FastLane stage may be a host-only SPSC library, without
claiming secure native transport readiness; it is not implemented here.

## Issue classification

| Finding | Classification and action |
| --- | --- |
| Pending auth/account grant usable or stranded after cancelled reply | Critical current-boundary defect; repaired and host/native cancellation validated |
| Missing recipient incarnation in grant and call identity | Critical current-boundary defect; repaired, host replacement and native stale-incarnation rejection validated |
| Trusted auth consumer check unlocked before broker use | Critical current-boundary defect; repaired with existing scheduler/broker order |
| UAC internal mint/consume without transport | Legitimate production contract, not a defect; retained with exact same-request authorization |
| VFS/UAC delegation TODO and nonzero admin-token check | Critical unresolved production integration prerequisite; no new mint privilege granted, separate service-policy workstream required |
| Legacy VFS backend handle and SHM transfer authorization/lifetime | Independent service/legacy-memory workstream; not covered by the auth/account fix and cannot justify restricted transport readiness |
| Historical removed-token recurrence and deferred handle exhaustion | Future hardening/review; active collision exclusion remains, no new historical guarantee claimed |
| Remote native exit/exec and actual PID replacement not forced | Test-infrastructure/validation limitation; required acceptance stays blocked |
| Scheduler foreground proof used old signature | Test-infrastructure adjustment completed; original failure retained |
| Missing WAV and existing formatting differences | Unrelated fixture/baseline failures, retained separately |


## Final validation matrix and resource accounting

Full compatibility evidence: `20261021T082704.986379Z` under the R4.1 evidence
root. Source hashes matched at the end of that run. Final authority evidence:
`final-authority-20261021T084255.976987Z`. The latter tightens only the opt-in
fixture to retain one delivered auth grant for native exec revocation. The four
subsequently changed files are the fixture assembly, feature-only gate, feature-
only exec assertion count, and validator markers. Production issuance, IPC,
consumption, foreground, identity and cleanup core hashes are unchanged from the
full compatibility run. Both image/source identities are retained; earlier runs
are not relabelled as tests of the final fixture.

| Gate | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
| --- | --- | --- | --- | --- |
| Final mint/delivery/cancellation/use/exec fixture | PASS | PASS | PASS | PASS |
| Resident boot / VFS service registration / SHM retirement | PASS | PASS | PASS | PASS |
| Scheduler SMP gate | PASS | PASS | PASS | PASS |
| D1/D2/D3 + D3.6R RecoveryBlocked gate | PASS | PASS | PASS | Not run |
| VM/TLB SMP | Not run | Not run | Not run | PASS |
| VM unmap / protection | Not run | Not run | PASS | Not run |
| TCP/TLS certificate/hostname verification | Not run | Not run | Not run | PASS |
| Production VFS/UAC delegation + remote exit/exec races | BLOCKED | BLOCKED | BLOCKED | BLOCKED |

Host results: **91 kernel**, **115 IPC/service** (61 IPC, 25 audiod, 29 sunlightd),
**14 UAC**, **11 ELF** tests passed. Scheduler host and kernel target checks
passed. Media: **36 passed, 1 failed**, missing repository WAV fixture. Workspace
format check still fails with baseline and retained touched-file formatting
differences; the full formatter output is preserved. Final
`git diff --check` passes. Supported per-package host commands and exact target,
ISO, QEMU and network commands are in the machine-readable summary/raw logs.

| CPUs | Parallel fixture workers | Free physical frames before / after | Shared backing delta | Page-table delta | Final broker tokens / auth-account records |
| --- | --- | --- | --- | --- | --- |
| 1 | 2 | 68184 / 68184 | 0 | 0 | 0 / 0 |
| 2 | 4 | 67926 / 67926 | 0 | 0 | 0 / 0 |
| 4 | 8 | 67414 / 67414 | 0 | 0 | 0 / 0 |
| 12 | 24 | 65366 / 65366 | 0 | 0 | 0 / 0 |

Per final native run, 34 grants committed (17 auth + 17 account): cancellation
retires 2, actual consumer calls consume 15, and exec retires 17. Conservation
of 34 is derived from those asserted measurements, not a separate retired-grant
counter. Pending and final active auth/account inventory are asserted zero;
ordinary token inventory, mappings, frames, shared data and page-table deltas
are checked independently. Metadata vector capacity may remain allocated for
reuse; its byte capacity and allocator drift are unavailable. Equal frames do
not prove whole-kernel metadata leak freedom.

Grant/mint-to-delivery/failed-delivery latency distributions, lock-hold times,
cumulative retired-record gauge, in-flight gauge and outstanding lifecycle-
reference gauge are unavailable, not zero. New records contain identity values,
not extra process/VM lifetime pins or an issuance reservation. Existing endpoint
TSC samples are not UAC delivery latency. No FastLane throughput claim is made.

Development attempts `20261021T082313.571953Z` and `20261021T082544.853370Z`
were interrupted for source/mock corrections before accepting a final snapshot.
Their logs and initial host/target failures remain. The full frozen run's only
aggregate failure is formatting; the WAV failure is separately recognized and
retained. All native compatibility and final focused authority assertions passed
on their first complete attempt. The final focused run reports explicit timeout
flags (all false); interrupted development runs are not counted as passes.
