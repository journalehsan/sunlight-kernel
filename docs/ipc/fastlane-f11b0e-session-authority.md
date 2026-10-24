# FastLane F1.1B-0E: Account session authority

Date: 2026-10-24, Asia/Tehran. **Fail-closed containment; session closure remains
BLOCKED.** The raw evidence and machine-readable index are under
`target/fastlane-f11b0e-evidence/`. No earlier evidence is overwritten.

## 1. Executive summary

The real Account path has no operation that serializes session withdrawal with
broker insertion. Its service-local `(session, generation)` can repeat after
sessiond restart. A second userspace check, the trusted UAC image role, and the
recipient's VM generation do not establish current session authority.

This phase takes the explicitly permitted safe-stop alternative: the actual
kernel Account mint **and consume** entry point refuses the operation. A live,
trusted UAC handling its exact authenticated IPC caller receives
`SessionAuthorityUnavailable` (Account error 11). Invalid roles/callers receive
Denied. Password, profile and group publication cannot pass that refusal. Account
presentation and independently authorized root-password/VFS delegation retain
their existing contracts. This is an intentional availability reduction for the
three unsafe mutation classes, not implementation of a working revocation lease.

The existing exact token ledger is retained. The VFS grant path now preserves
`TokenHistoryFull` through broker, syscall, IPC library and UAC error reply.
Bounded tests exercise actual broker preparation/publication and rollback at the
real 65,536-entry threshold. Native capacity testing uses an isolated instance
of the production broker, never a reduced identifier space or global entropy
injection. The live global broker is not deliberately exhausted.

Session Authority remains **BLOCKED**. Token-History acceptance remains
**PARTIAL**: the boot-bounded mechanism has boundary evidence, but global native
exhaustion/error delivery and consistent capacity reporting for other issuance
ABIs are not demonstrated. Gate A remains VERIFIED NATIVE, Gate B PARTIAL,
F1.1B-0 BLOCKED and Gate C BLOCKED. No Restricted SHM or production FastLane is
activated.

## 2. Starting F1.1B-0D baseline

Read `fastlane-f11b0d-authority-closure.md` and its retained validation index,
`fastlane-f11b0c-production-grants.md`, `fastlane-f11p-capability-authority.md`,
`fastlane-f11pr4-vfs-authority.md`, and `fastlane-f11pr41-mint-delivery.md`.
D established boot-lifetime token exclusion, checked session/PID exhaustion,
two recipient-exit orderings, 113 kernel tests and 258 service/library tests.
Those are historical results; E's current results are indexed separately.

D explicitly did not establish Account/logout serialization, restart-qualified
sessions, native numeric PID reuse or global resource reconciliation. Historical
D3's `d3_driver.rs:50` mismatch remains unexplained. A passing current D3 replay
cannot explain or supersede that failure. Existing manifest/version/lockfile and
D implementation changes in the worktree were preserved; no destructive Git
operation or commit was performed.

## 3. Account-to-broker call graph

```text
caller -> kernel IPC (badge and reply target established by kernel)
       -> uac_service -> account_service::{dispatch,dispatch_group,dispatch_profile}
          -> session_query_process(badge): current kernel credentials/VM generation
          -> accounts::active_session -> sessiond SESSION_CURRENT_IDENTITY
          -> account/password/revision policy
          -> accounts::try_broker_grant -> syscall 151 / 152
             -> SCHEDULER -> Scheduler::account_grant
                -> authenticate live UAC + exact pending caller
                -> SessionAuthorityUnavailable (no insertion or consumption)
          -> error reply; no account publication

historical successful continuation, now unreachable:
  Account grant -> consume -> private staging -> VFS rename of passwd/group/shadow

independent production path:
  verified root password -> syscall 102 -> recipient-bound AuthSessionGrant
  -> UAC delegation -> syscall 164 -> Scheduler::try_delegate_vfs_grants
  -> CAP_BROKER -> root-proof exchange -> bounded VFS grant batch
  -> exact-call IPC settlement -> VFS AUTHORIZE_EXEC -> one-use consume
```

Actual sources: `services/sunlight-uac/src/bin/account_service/mod.rs`,
`services/sunlight-uac/src/bin/uac_service.rs`, `ipc/src/accounts.rs`,
`ipc/src/vfs_grants.rs`, `kernel/src/capability/{lifecycle,vfs_grants,delivery}.rs`,
`kernel/src/arch/x86_64/syscall.rs`, and `services/vfs_server/src/main.rs`.

## 4. Session trust and ownership

| Identity or authority | Origin and meaning |
|---|---|
| Account UID/GID | Kernel caller credentials and passwd/shadow authentication; a payload UID or name is never a privilege assertion. |
| Authentication proof | Actual password verification in UAC; short-lived AuthSessionGrant, distinct from a graphical session and cached runas authorization. |
| Graphical session | sessiond owns `ServiceState.active` / `ActiveSession.record`; Account accepts Running/Degraded, not Locked/Stopping/Stopped. |
| Session ID/generation | `allocate_session_identity` checks both counters without wrap. sessiond initializes them to 1 after replacement; no Account activation epoch is conveyed. |
| Account service incarnation | UAC's kernel Process foreground incarnation; account handlers are part of UAC, not a separately restarted daemon. |
| Trusted executable role | Kernel-installed `trusted_auth_broker` from the permitted launch chain, withdrawn on exec; neither root UID nor numeric PID suffices. |
| Session owner incarnation | The separate sessiond Process with `trusted_session_service`; its activation is not currently bound into Account GrantScope. |
| Delegation authority | Root password proof and fixed VFS read/execute policy, not a cached UID or session tuple. |
| Grant owner | Exact pending caller with process incarnation, VM generation and IPC call generation in GrantDelivery. |
| Recipient VM | Address-space generation; exec changes it. It does not describe sessiond or UAC restart. |
| Consumption | One-use broker removal for supported grants; separate from delivery and later filesystem publication. |

`ipc::reply_target_matches_pending_request` authenticates the candidate requester
against the kernel's saved reply target, caller incarnation and pending call.
The kernel replaces sender badges; user-controlled message fields do not select
privileged issuer identity. Own-password change verifies the old password and
uses the kernel caller UID; own-profile change requires that UID to match the
active session; group authorization additionally requires the root/admin policy
and password proof. These preliminary policies do not supply broker-current
session authority. The production VFS probes retain UID/GID 1000 clients.

Service discovery is a separate boundary. `accounts::active_session` looks up
`sunlight.session.v1` and accepts the session reply fields; it does not receive a
kernel-attested activation incarnation. `ipc::mediate_nameserver_register`
authenticates endpoint ownership and `registration_authorized` checks the process
name/service-name pairing; its special supervision/display checks are not a
session activation protocol. No exploitability or arbitrary name-spoofing claim
is inferred here. Endpoint discovery alone is not treated as proof of current
privileged session authority, and future activation must establish that binding
through authenticated kernel identity.

## 5. Commit/revocation state machine

There is **no active Account authority state machine** to certify. Current safe
state is `AuthorityUnestablished`; mint and consume both refuse without mutation.
Neither service startup nor a session tuple can transition it to Active.

The missing protocol would require the following states and transitions, as a
specification only, not a test model or implemented API:

```text
Unestablished -- authenticated owner/activation agreement --> Active(epoch, session)
Active -- broker-serialized invalidate/expiry/owner retirement --> Retired
Retired -- delayed activate/mint/reply --> reject
unknown or interrupted agreement --> Pending/Unestablished, issuance refused
```

A trustworthy replacement owner and nonaliasing epoch would be required for a
new activation. Merely resetting independent process mutexes is insufficient.

## 6. Linearization points and proof boundary

| Operation | Existing boundary / present E decision |
|---|---|
| Preliminary session validation | sessiond produces a reply from its userspace record. This is a snapshot, not authorization at a later broker commit. |
| Account grant commit | Disabled. `Scheduler::account_grant` returns an error before acquiring CAP_BROKER, drawing entropy or inserting a record. |
| Account consumption | Disabled through the same entry point, including stale delivered tokens. |
| Logout | sessiond `session_action(Logout)` calls `begin_stop`, assigns Stopping, then replies success. No broker withdrawal acknowledgment exists. |
| Lock/invalidation | Userspace state change independently makes `active_session` fail; it does not serialize with CAP_BROKER. |
| Expiration | Account record deadlines were checked at consumption. No broker-authoritative graphical-session expiry/lease exists. |
| UAC retirement | SCHEDULER serializes live issuer admission with exec/exit. This does not establish graphical-session withdrawal. |
| Supported VFS batch | SCHEDULER -> CAP_BROKER spans proof validation, insertion and proof removal. On failure, appended rows are truncated and the proof remains owned. |

Cases A (revoke first), C (overlap), and D (Account unavailable before commit)
cannot produce a new Account grant because every attempt is refused. Case B
(successful Account commit before revocation) is unavailable; it is not reported
as a forced passing race. This prevents the unsafe operation but does not prove
a useful shared commit/revocation ordering.

## 7. Cross-service failures and lock order

Account's preliminary synchronous IPC to sessiond runs in userspace without a
kernel spinlock. Syscall paths take SCHEDULER, then CAP_BROKER, then the existing
entropy conditioner only where needed. No IPC, callback, userspace fault or wait
for another CPU occurs while these locks are held. Account refusal takes no
broker lock. Existing VFS consumption and lifecycle retirement retain their order.

A future revocation must become authoritative at the broker transition, and
sessiond must wait for its acknowledgment before acknowledging completed
withdrawal to Account clients. Communication interrupted before proof of that
transition needs Pending/Unknown, with issuance stopped. Current `begin_stop`
has no such acknowledgment. Broker outage and Account/sessiond outage confer no
authority. E introduces no cross-process mutex fiction, lease-renewal polling,
two-phase commit engine or supervisor restart policy.

## 8. Expiration semantics

Graphical-session validity is a state policy, not the cached UAC SessionStore's
30-minute TTL. AuthSessionGrant's tick expiry and the derived VFS deadline are
also distinct. E retains their existing comparison semantics. Historical Account
grants had a 500-tick one-use deadline, which did not validate current session
state at insertion. E refuses Account authorization at every tick, including
u64::MAX; a time check cannot enable it. A future broker lease must define its
trusted clock, expiry comparison and serialization with commitment explicitly.

## 9. Previously committed grant policy

| Class | Present lifetime |
|---|---|
| Account password/profile/group | Unsafe session authorization is unavailable. Even a historical delivered row cannot be consumed through production. Cleanup still retires rows normally. No in-place kernel upgrade/restart imports such rows. |
| AuthSessionGrant | Recipient/VM/call-bound, one use, existing short tick expiry. Delivered recipient ownership survives issuer retirement under the established policy. |
| Production VFS grant | Derived only from a delivered root password proof, inherits its deadline; process/VM-bound, one use, owner cancellation and lifecycle cleanup. Graphical logout is not a universal revocation of this independent proof. |
| Legacy IPC/VFS bearer and SHM | Established class-specific transfer, destruction and shared-backing lifetime preserved. No universal retroactive revocation added. |

The VFS rename after successful historical Account consumption was a separate
publication boundary. This phase makes it unreachable; it does not assert that
rename became logout-atomic. Reenabling Account needs an explicit decision about
whether authorized work can publish after withdrawal, plus enforcement for the
chosen policy. Independently delegated authority is not silently reclassified.

The separate `uac_service::migrate_development_shadow` startup maintenance path
can still call `publish_shadow` to migrate the existing development database. It
is trusted-service boot maintenance, not a session-authorized Account RPC.
`syscall.rs::current_fs_actor` supplies the trusted UAC filesystem actor; VFS
rename does not universally require an Account token. This change therefore
contains the audited Account handlers, not arbitrary code executing inside the
trusted UAC service. That independent filesystem trust boundary is preserved.

## 10. Account restart and activation

SunlightD launches embedded UAC and sessiond through the existing trusted chain.
`services/sunlightd/src/main.rs::{start_service,stop_service}` and activation
runtime track process generations and reject unconfirmed stop/restart. Kernel
spawn assigns fresh foreground incarnations; VM identity changes independently.
Exec withdraws privileged image roles. These primitives authenticate processes,
but do not qualify Account's service-local session counters.

No new Account privileged activation is permitted in E, including after successful
service replacement. Consequently stale replies, reset session counters or a
reused PID cannot open this path. This is refusal, not native proof of a working
restart-qualified identity. RecoveryBlocked remains unchanged: its loop does
not load units or reconstruct ownership, and no fallback bypass was added.

## 11. Broker restart

CAP_BROKER is a single kernel static for the entire boot. There is no supported
in-boot broker reset/restart protocol. Host tests construct isolated brokers and
reset their private test global; that is not a production recovery facility.
UAC/sessiond/VFS replacement does not clear global token history. Do not replace
CAP_BROKER to recover capacity: that could recreate old raw bearer values.
Reboot destroys all authority and history together, but the ABI does not provide
persistent cross-boot token exclusion. An old externally retained u64 could
probabilistically recur in another boot. No stronger lifetime is claimed.

## 12. Token-history exhaustion design

Retain Strategy A: exact sorted `Vec<u64>` with at most 65,536 prepared tokens,
shared by all five token tables. `issue_token` reserves history before entropy,
checks both active and previously prepared values, then records before returning.
Dropped SHM preparations and rolled-back batches permanently spend identities.
No eviction or cleanup prunes the ledger. Active table slots can recycle without
reassigning an old bearer in this broker lifetime. Candidate retry remains bounded
at 16; bearer unpredictability remains conditioned entropy with 62 payload bits
and two routing tags. No cryptography or unchecked wraparound is introduced.

At exactly 65,536 entries, every fresh preparation fails TokenHistoryFull before
entropy or publication. Existing lookup/consume/retire remains usable. The VFS
batch preserves the exact error, rolls back its unpublished rows, and retains
the original proof; a burned first identity is not refunded. UAC forwards
`REPLY_ERR.words[0] = IssuanceError::TokenHistoryFull` (2). Denied (1), active
capacity (3), and source/unavailable failures (4) remain distinguishable through
`try_mint`; the old Option convenience wrapper still fails closed.

## 13. Capacity, memory and availability

At most 524,288 requested token-word bytes, plus Vec and allocator overhead;
reservation grows by 256 words. Allocated capacity and history length are
reported separately. History is retained security metadata, not a live-grant
leak. Sorted insertion can move 65,535 words under the broker guard; worst-case
concurrent lock latency has not been measured.

The boot-wide budget is globally spendable, including by ordinary endpoint/SHM
churn. At one prepared token per second, an initially unused budget lasts about
18.2 hours; at one per minute, about 45.5 days. Failed preparations and routine
boot consumption shorten those ceilings. These are arithmetic bounds, not
measured production rates. Increasing the constant is not an admission policy.
No retry loop, automatic reset or automatic restart is added. Consumption and
cleanup can continue after exhaustion; fresh issuance cannot recover this boot.
Broker-level consumption does not establish end-to-end service availability:
operations that additionally require a new endpoint, reply capability or proof
can fail before reaching a consumer. Global full-history transport was not
exercised. A controlled full reboot discards all outstanding authority but provides only
the documented new-boot probabilistic bearer boundary. Long-running availability,
principal budgets and remaining issuance ABIs' generic errors remain limitations.

## 14. Production changes

* `capability/lifecycle.rs`: deny Account mint/consume; Result-based VFS delegation.
* `syscall.rs`: explicit Account error reply and typed VFS issuance status.
* `ipc/accounts.rs`: error 11 and `try_broker_grant`; UAC handlers preserve it.
* `capability/vfs_grants.rs`: preserve batch failure cause and rollback.
* `ipc/vfs_grants.rs` and UAC delegation: preserve capacity error through IPC.
* Raw Account mint/consume and unchecked delivery fixture are host-test-only.
  Historical metadata tests are explicitly labeled; no native feature restores
  the unsafe production path.
* Opt-in native gate exercises refusal and isolated capacity; normal builds omit
  the fixture, deterministic candidate source and checkpoint logging.

## 15. Host deterministic results

The actual broker and production Scheduler methods compile in the existing host
shim. Six new tests cover Account tuple refusal without entropy/records,
replacement/expiry/stale-delivery refusal, concurrent refusal/cancellation, exact
capacity with oldest/newest replay rejection, last-slot concurrency, and failed
VFS batch rollback with existing-grant consumption at full history. Historical
collision, cancellation, consumption, PID/VM replacement, exec and exhaustion
tests remain. Their unchecked Account helper tests only metadata mechanics.

The frozen-source replay passes **119 kernel tests** and **258 service/library
tests**: IPC 62, UAC 18, sessiond 27, filesystem 73, SunlightD 29, audio 25,
ELF 11, Ring 8 and Doorbell 5. The scheduler host runner and 12 evidence parser
tests pass. The separate SunlightD runtime binary passes 19 tests.
The evidence index provides exact commands, logs and final-source results.
No host test is labeled as a native session commit/revocation interleaving.

| Requested host case | Result boundary |
|---|---|
| Valid session / successful Account commit | BLOCKED; issuance intentionally refused. |
| Revoke before authorization; after preliminary authorization; commit before revoke; overlap | BLOCKED for useful session ordering. Refusal tests pass without creating authority. |
| Expiry versus commit; logout versus delivery; two grants versus revoke; revoke versus cancellation | BLOCKED for session protocol. Concurrent refusal/cancellation containment passes. |
| Session counter rollover/exhaustion | Existing production allocator host tests pass; no restart epoch inferred. |
| Account restart before/during/after commit; old reply after replacement | BLOCKED for working lifecycle; obsolete-authority refusal host matrix passes. |
| Numeric PID reuse; old session token; activation failure | Containment rejects all such inputs. Supported native service activation/replacement remains BLOCKED. |
| Broker restart with old authority | BLOCKED/unsupported in boot; constructing a fresh host broker is not recovery evidence. |
| Exact history limit; concurrent final slot | PASS on actual broker and bounded host barriers. |
| Reused slots; cancel/retire; repeated consume; collision injection | Existing actual-broker regressions pass; Account metadata cases are explicitly unchecked host storage tests. |
| Full-history VFS batch rollback and existing-grant consume | PASS through production Scheduler/broker methods with synthetic host identities. |
| Restart across token lifetime | No deterministic cross-boot exclusion; no in-boot reset API. BLOCKED for a stronger restart claim. |

## 16. Required forced native cases

| Case | Classification and scope |
|---|---|
| N1 revoke before Account commit | BLOCKED: useful ordered revoke/commit protocol absent. |
| N2 Account commit before revoke | BLOCKED: Account commits disabled. |
| N3 genuine concurrent revoke/commit | BLOCKED: no two authoritative transitions to force. |
| N4 Account exit between authorization/commit | BLOCKED for requested service race; refusal is independently checked. |
| N5 replacement receives stale reply | BLOCKED; legitimate restarted service fixture absent. |
| N6 session identity reuse after restart | BLOCKED; no trusted session activation epoch implemented. |
| N7 revocation delivery interruption | BLOCKED; no acknowledged withdrawal protocol exists. |
| N8 exact history boundary | Actual classification per CPU in index; isolated production broker only, not global syscall/error-delivery coverage. |

The opt-in ring-3 capability fixture invokes real syscalls 151 and 152 with its
established trusted issuer and live pending caller: 34 explicit authority-unavailable
refusals plus one retired-recipient denial, zero Account commits. These are
refusal assertions, not N1–N7 race passes or actual UAC/sessiond service restart.
Native end-to-end Account mutation RPC/error-reply coverage is NOT_RUN; the
fixture invokes the kernel entry points directly. Account service code is
target-built and host policy-tested, which does not replace that native gap.
Native logs record issuer incarnation, VM, CPU, global broker identity and
`session_epoch=UNESTABLISHED`, `commit=none`. They contain no fabricated epoch,
revocation checkpoint, grant fingerprint or raw bearer. The isolated N8 loop
runs on CPU 0 in each image; repeated CPU-count coverage does not turn it into
a concurrent limit test. Competing final-slot issuance is host-tested only.

## 17. SMP matrix

Run fresh 1/2/4/12-CPU images for production UAC/VFS delegation and the separate
capability/refusal/boundary gate. CPU IDs and independently parsed checkpoints
are retained. D's recipient exit pair stays separately classified; its ordinary
per-CPU probes still say remote_race=NOT_FORCED. A CPU count never supplies missing
Account concurrency. Final per-run outcomes and failures/retries are in the index.

| CPUs | Actual UAC/VFS delegation | Recipient exit before/after commit | Account syscall refusal | N8 private production broker | N1-N7 |
|---|---|---|---|---|---|
| 1 | PASS | Both FORCED_AND_PASSED | 35 refusals, 0 commits | FORCED_AND_PASSED | BLOCKED |
| 2 | PASS | Both FORCED_AND_PASSED | 35 refusals, 0 commits | FORCED_AND_PASSED | BLOCKED |
| 4 | PASS | Both FORCED_AND_PASSED | 35 refusals, 0 commits | FORCED_AND_PASSED | BLOCKED |
| 12 | PASS | Both FORCED_AND_PASSED | 35 refusals, 0 commits | FORCED_AND_PASSED | BLOCKED |

The recipient-exit pair concerns independently delegated VFS authority; it is not
an Account revoke/commit or numeric PID-reuse test.

## 18. Focused accounting

Account refusal allocates no session, epoch, broker authorization row, pending
revocation, in-flight commitment or delivery reference. It cannot establish how
many live sessions sessiond owns; those counts are **unmeasured**, not zero.
There is no new session authority registry whose capacity could hide live state.
Historical test rows are retired through existing idempotent cleanup.

The boundary fixture retains exactly the configured history length, checks no
extra entropy draw/publication at full capacity, preserves a live endpoint and
then retires it. Host VFS tests check zero partial batch, one retained proof,
existing grant consumption, zero final active rows and retained history length.
The production matrix separately records broker tables, pending/delivered rows,
history length/capacity, heap and free frames. Its final grant inventory does not
prove allocator/reference reconciliation across VFS/UAC or all system services.
The independent global reconciliation blocker remains open.

Concrete unmatched stores: kernel `CapabilityBroker::{capabilities,shared_regions,
vfs_caps,auth_session_grants,account_grants}` and `token_history.words`;
`GrantDelivery` issuer/recipient/call identity values; Process `pending_call`,
`ipc_reply_target`, `ipc_call_outcome` and IPC bus settlement; UAC
`SessionStore<16>::sessions`, stack-held messages/proofs, replacement buffers and
`/etc/.shadow-uac-new`, `/etc/.group-uac-new`, `/etc/.passwd-uac-new` staging;
sessiond `ServiceState.active` and its SessionRecord. VFS `AUTHORIZE_EXEC` forwards
consumption to the kernel and has no second grant table; its separate
`State::{vfs,boot,open_meta}` / `OpenMeta::{handle,len,path}` inventory describes
legacy filesystem handles, not capability grants. No equality between those
independent resource counts is asserted.


Final production-probe measurements (resident services still own unrelated
IPC/SHM authority) distinguish live rows from retained capacity. These are
observations, not a global before/after leak proof:

| CPUs | All broker live | Pending / delivered grants | Auth / Account / VFS rows | History length / capacity | Heap used bytes | Free frames |
|---|---|---|---|---|---|---|
| 1 | 57 | 0 / 0 | 0 / 0 / 0 | 93 / 256 | 1468272 | 34904 |
| 2 | 60 | 0 / 0 | 0 / 0 / 0 | 121 / 256 | 1530976 | 31534 |
| 4 | 64 | 0 / 0 | 0 / 0 / 0 | 172 / 256 | 1701448 | 30440 |
| 12 | 64 | 0 / 0 | 0 / 0 / 0 | 356 / 512 | 1826688 | 28392 |

Exact table element sizes/capacities and start samples are retained in the index.

## 19. Regressions and baseline failures

The validation and compatibility indexes retain commands, exit codes, raw logs,
image hashes and source hashes for kernel/service tests, process exec/exit,
PID exhaustion, pin release, native Doorbell, IPC regressions, SunlightD D1/D2/D3,
RecoveryBlocked, VM/TLB/unmap/protect, TCP/TLS, media/audio and builds.

| Final regression | Result |
|---|---|
| Kernel / service libraries | PASS: 119 / 258 tests. |
| Scheduler host / evidence parsers / SunlightD runtime | PASS; 12 parser and 19 runtime tests. |
| Production UAC/VFS and controlled recipient exits | PASS on 1/2/4/12 CPUs; scope in section 17. |
| Account refusal / isolated exact history boundary | PASS on 1/2/4/12 CPUs; no useful session protocol inferred. |
| Exec/exit, checked PID exhaustion, IPC | Existing host/native fixture regressions pass; native numeric PID reuse NOT_FORCED. |
| Pin release / Doorbell / scheduler | Native matrices PASS on 1/2/4/12 CPUs; Gate B remains PARTIAL. |
| VM/TLB / unmap / protect | PASS on 12 / 4 / 4 CPUs. |
| SunlightD D1/D2/D3 / RecoveryBlocked after-ready | PASS on 1/2/4 CPUs; historical D3 still unexplained. |
| TCP/TLS | PASS on 1/2/4/12 CPUs with UnknownIssuer and wrong-name rejection plus verified IP/DNS handshakes. |
| Target and ordinary services / Account control panel | PASS; default kernel target check and dependent UI target check. |
| Audio / media | 25 audio tests pass; media 36 pass, 1 unchanged missing-fixture failure. |
| Strict Clippy / workspace format / Miri availability | FAIL; diagnostic/source baseline comparison retained. |

Strict Clippy, workspace formatting, missing media/WAV fixture and unavailable
Miri are compared with D evidence by diagnostic and source identity. No warning
suppression is added. Media reports 36 passing tests and the one unchanged missing-fixture failure.
Strict failure remains failure even for unchanged sources;
there is no manufactured matched clean baseline. New failures and timeouts remain
visible. Current D3 success never promotes Gate B.

E development failures are separate from that baseline. The first opt-in
capacity image failed to compile because `usage()` was not visible under
`cap_issuance_test`; its cfg was corrected. The next authority matrix panicked
on all four CPU counts at a retained exec fixture assertion: it expected 17 old
Auth/Account rows, while containment correctly left one Auth row and no Account
row. Only the opt-in assertion/marker was corrected to that policy. Those failed
raw runs and images remain indexed. An earlier pin image full-file comparison
also failed to establish binary equivalence; it is not accepted as final-source
identity proof. Fresh final-source replay supplies the acceptance evidence.
The dependent Account control-panel target also type-checks; no UI runtime
result is claimed. Audio library tests pass; the media fixture failure remains. No human-audible
playback or real recording/capture result is claimed.

## 20. Final-source identity

The final index names the stable primary run, all earlier attempts, SHA-256
source manifests, HEAD, build environment, embedded service identities and
kernel/ISO identities. Relevant source hashes must match after native and
compatibility execution. Earlier source-changing attempts are not silently
promoted to final-source acceptance. Reports/index assembly do not alter compiled
source. Changed-line formatting and `git diff --check` are retained separately
from the known failing workspace formatting command.


The final frozen input inventory contains 1271 code/configuration
files. HEAD is `3aa1d302fc85799f93835b2b33d273c442d5e85c`; the dirty worktree is identified by
`source-sha256.json` SHA-256
`848a02f5d7e586c74c153a4b3687aaf83986c10b2f6c9efa3510685faf28554a`, not by HEAD alone.

Primary runs under `target/fastlane-f11b0e-evidence/`:

* Host: `20261024T110300.513870Z`.
* Actual production UAC/VFS: `20261024T112359.296012Z`.
* Refusal/exact-boundary image: `compat-20261024T110254.832282Z`.
* Complete compatibility replay: `compat-20261024T110528.539658Z`.

| Retained image | ISO SHA-256 |
|---|---|
| Production probe | `96f7f6ba6928e5e65e7718946967dca152190e9be9a9791ad92166ab2a75365c` |
| Refusal/boundary probe | `9f7b8faa9b910da0ea2d39d1b0559ad61f84c59e50c3770fc3f782da3b73eb04` |
| Resident compatibility | `88b8e45a1e0750ec496f3766320849d6ca3165695cfe770d7883d4025f31fd5d` |

The indexes also retain stripped ELF and embedded-service hashes. Native probe
features are explicitly selected for testing. Ordinary services are restored;
the default kernel target check passes and the rebuilt ordinary UAC binary has
no opt-in checkpoint markers. No ordinary-image native boot is inferred from
that check. Earlier failed/source-changing attempts remain independently indexed.

## 21. Remaining F1.1B-0 blockers

1. No working authenticated session activate/revoke/expiry transaction ordered
   with Account commitment; the safe refusal must stay until this exists.
2. No restart-qualified Account/sessiond activation epoch or stale-reply lease
   validation. Current refusal prevents exploitation, not missing functionality.
3. Actual native PID reuse, remaining issuer/recipient exec/exit/cancel/delivery
   windows, legitimate restarted VFS role installation and service restart races.
4. Global Broker/VFS/UAC allocation, outstanding reference and deferred cleanup
   reconciliation remains incomplete.
5. Finite globally spendable token budget, inconsistent capacity reporting in
   other issuance ABIs, no persistent cross-boot token identity and no native
   global exhaustion/error-delivery demonstration.
6. Historical unexplained D3 activation failure remains independent.

## 22. Gate decisions

| Gate | Decision |
|---|---|
| A Scheduler Pin-Release | VERIFIED NATIVE; current replay recorded separately. |
| B Native Doorbell | PARTIAL; historical D3 is unexplained. |
| Session Authority | BLOCKED; safe-stop fallback, no useful commit/revoke or restart protocol. |
| Token-History | PARTIAL; exact boot-bounded safety tests, remaining native/API availability coverage stated above. |
| F1.1B-0 Production Grant Authority | BLOCKED. |
| C Production Native FastLane | BLOCKED. |

## 23. Precise handoff

Before a useful Session Authority subgate can be accepted, design the smallest
explicit authenticated sessiond-to-broker activation/withdrawal acknowledgment
protocol, bind it to the owner process incarnation and a checked broker epoch,
and specify Account publication after consumption. Reconcile all session state
transitions (Running/Degraded, Lock, Logout, shell failure, owner exit/exec,
expiration and activation failure). A delayed reply must name the old authority;
cleanup must never erase information needed to reject its reactivation. Keep
issuance stopped for unknown communication outcomes. This is unfinished E work,
not something to conceal inside an F scope change.

For F1.1B-0F, do not start implementation here. Inventory needed fixtures:

* Native supported numeric PID reuse for issuer and recipient, with old token,
  pending reply, cancellation and same-generation-call replay against replacement.
* Recipient consume versus exit; exec between commit and delivery; same VM slot
  with new generation; issuer exit/exec before commit; cancellation versus consume;
  late settlement after retirement; old service completion after replacement.
* Legitimate UAC/sessiond/VFS replacement with pending/delivered grants and an
  authorized VFS consumer role; interrupted stop/restart and RecoveryBlocked.
* Bounded commit-boundary rendezvous that releases competing locks, records real
  CPU/incarnation/VM/call identities and proves independent participant progress.
* Broker table/retained-history capacity versus live rows; VFS request/deferred
  delivery ownership; UAC proof/message/staging ownership; sessiond live records;
  allocator bytes/frames and issuer/recipient incarnation references. Current
  zero grant-table observations do not account for those independent stores.
* Global capacity-error delivery fixture or narrower authorized domain with
  identical error path, plus reporting for proof/endpoint/SHM issuance, before
  promoting Token-History acceptance.

Restricted SHM, active mapping revocation, final Doorbell acceptance and verified
cross-process transport remain independent later prerequisites.
