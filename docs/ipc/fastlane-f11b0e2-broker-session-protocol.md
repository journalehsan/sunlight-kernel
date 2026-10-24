# FastLane F1.1B-0E2: broker-authoritative session protocol

2026-10-24, Asia/Tehran. Implementation and partial verification. **Session
Authority: PARTIAL. F1.1B-0 and Gate C: BLOCKED.**

The evidence index is `target/fastlane-f11b0e2-evidence/validation-summary.json`.
It indexes commands, source identities, individual attempts, images, serial logs,
and scope-qualified results. This report does not supersede earlier failures.

## 1. Executive summary

The kernel broker now owns the session reference, authenticated service owners,
principal, activation state, optional deadline, and grant retry receipts. Session
registration consumes a delivered UAC login proof through a new synchronous
kernel operation. Sessiond requests registration and activation; it cannot select
the authoritative ID, epoch, UAC identity, or authenticated principal. Account
continues verifying passwords and applying account policy in userspace.

Account grant mint and consumption now validate authoritative session state
inside the same broker transaction as insertion or consumption. Logout,
expiration, and service retirement use that transaction boundary. The grants are
session-bound, including after delivery: retirement cancels unconsumed grants.
Neither an earlier Account session query nor possession of a session reference
authorizes an operation.

Host tests and native sequential broker witnesses pass. The native witnesses use
an **isolated production broker with synthetic participant identities**. They do
not demonstrate a real authenticated graphical login, remote session races, or
Account replacement. N1–N7 remain NOT_FORCED. Grant primitives are
RESTORED_PARTIAL; password, account-profile, and group publication remain
FAIL_CLOSED. This phase therefore does **not** satisfy the requested full native
acceptance gate. No Restricted SHM or production FastLane was activated.

## 2. Exact starting limitations

Read the E report and its validation index, D and C authority reports, and
F1.1P/R4/R4.1 reports. E deliberately returned
`SessionAuthorityUnavailable` from Account mint and consume after authenticating
the actual UAC/requester. Its local session counters could repeat on restart.
There was no broker-owned registration or serialization of logout with mint.
Password/profile/group publication was unreachable. E recorded 119 kernel and
258 library tests, native containment/compatibility replays, and finite-history
boundary protection. These are historical baseline results, not E2 measurements.

Starting gates were A VERIFIED NATIVE, B PARTIAL, Token-History PARTIAL, Session
Authority BLOCKED, F1.1B-0 BLOCKED, C BLOCKED. Native numeric PID reuse, remaining
service lifecycle races, global grant reconciliation, and finite-history
availability were independent blockers. The unexplained D3 replay at
`d3_driver.rs:50` remains independent of current passing replays.

## 3. Account/UAC/broker trust model

Account handlers are part of the UAC executable, not a separate Account process.
The existing embedded-image launch policy installs `trusted_auth_broker`;
sessiond has `trusted_session_service`; TTY has
`trusted_tty_session_service`. Nameserver discovery and payload PIDs/UIDs do not
install these roles. Existing policy is reused, not replaced by a root-UID or
name check. `withdraw_exec_image_roles` now also clears the sessiond role.

Each service and recipient identity is `(PID, foreground.incarnation,
address_space.identity().generation)`. `grant_requester_for_role` also requires
the exact live synchronous caller incarnation and kernel IPC call generation.
Delivered login-proof metadata now retains the original issuer VM generation,
preventing a replacement UAC image from adopting its predecessor's proof.
Process incarnations use the checked allocator in `process/foreground.rs`.
VM identity is allocated through `process/mm2b_state.rs` from the counter in
`process/address_space.rs`; exhaustion stops allocation rather than wrapping.

Registration requires live trusted sessiond, its exact pending trusted TTY
caller, a proof already delivered to that caller, and the original live trusted
UAC identity. UID/GID come from that one-use proof. A service payload cannot
choose another principal or trusted epoch. These checks execute while holding
the scheduler's lifecycle guard. UAC's password verification remains in
`handle_auth_password`; the kernel does not verify passwords.

## 4. Production call graph and operation audit

Sources are the actual production files, not a second authorization model:

* `services/sunlight-uac/src/bin/{uac_service.rs,account_service/mod.rs}`;
* `services/sunlight-sessiond/src/main.rs`, `ipc/src/{accounts,account_session}.rs`;
* `kernel/src/capability/{lifecycle,sessions,delivery,mod,vfs_grants}.rs`;
* `kernel/src/arch/x86_64/syscall.rs`, `kernel/src/ipc/mod.rs`;
* `kernel/src/process/{mod,spawn,exec_authority}.rs`, `address_space.rs`;
* `services/sunlightd/src/main.rs`, `services/vfs_server/src/main.rs`.

| Operation | Production entry and authenticated identity | Authority boundary / references |
|---|---|---|
| Authentication completion | UAC `handle_auth_password`, kernel-set IPC badge; existing passwd/shadow verification; syscall 102 | `issue_auth_session_grant` saves UAC PID/incarnation/VM and caller/call; proof is not a registered session |
| Session creation | sessiond `create_session`, trusted TTY caller, delivered UAC proof; `account_session::register`, syscall 165 REGISTER | `Scheduler::register_account_session`; reserve identity/table capacity, consume proof, insert Suspended row under SCHEDULER→CAP_BROKER |
| Activation | sessiond loop `sync_account_session_authority`, syscall 165 ACTIVATE | Exact sessiond owner plus live original UAC; broker row changes to Active |
| Preliminary validation | Account `accounts::active_session`, sessiond `SESSION_CURRENT_IDENTITY`, kernel caller-credential query | Presentation/policy snapshot only; grant path independently resolves the broker row |
| Expiration | Every register/transition/mint/consume calls `expire_sessions(now)` | Sampled scheduler tick; remove expired rows and associated Account grants before authorization |
| Logout | sessiond `session_action(Logout)` after existing owner/root policy | Synchronous REVOKE must succeed before logout success; `begin_stop` revokes before child cleanup |
| Administrative revocation | Existing root-authorized session-control Logout path | Same REVOKE transaction; no new arbitrary-user revoke API |
| Capability mint | Account `try_broker_grant`, syscall 151, `Scheduler::account_grant` | Actual trusted UAC and exact caller; principal/operation/session checks plus token insertion/ownership/receipt in one guard |
| Capability delivery | Existing `ipc_reply` / `settle_grant_delivery` and authenticated reply target | Only matching actual call settles delivery; cancellation or failed settlement removes undelivered authority |
| Capability consumption | Account syscall 152, `Scheduler::account_grant(...Some(token))` | Active session, original live services, caller UID/incarnation/VM, matching binding, and either exact original inline request or delivered recipient; one-use removal under broker guard |
| Cancellation | Existing IPC abandonment, recipient exit/exec, or session suspension/retirement | Broker removal; receipt cannot reissue the removed token for the old request |
| Account exit | Existing scheduler exit/exec capability cleanup | `retire_recipient_grants` first retires owned/issued sessions; pending commits serialized by SCHEDULER |
| Account restart | SunlightD spawn/exit/dependency state; kernel fresh Process/VM identities and launch-chain roles | New instance can authenticate fresh logins; cannot reactivate/adopt old session rows |
| Broker retirement | Static boot-global CAP_BROKER; normal revoke/expire/service-retire removes rows | No production reset/restart API. Isolated replacement tables receive a fresh checked allocator epoch |

VFS remains on its existing path: verified root-password proof → UAC fixed-policy
delegation → syscall 164 → bounded VFS grant batch → exact-call settlement → VFS
one-use consume. Delivered VFS delegation is independent of graphical-session
logout and follows the earlier explicit contract. E2 does not change that class
to session-bound authority.

## 5. Session state machine

```text
Absent -- authenticated registration --> Suspended
Suspended -- owner ACTIVATE + original UAC live --> Active
Active -- owner SUSPEND --> Suspended (unconsumed Account grants cancelled)
Active/Suspended -- REVOKE, expiry, owner/UAC retirement --> Absent terminal ID
Absent terminal ID -- delayed ACTIVATE or mint/consume --> reject
```

Terminal records are removed rather than retained indefinitely. Non-reused IDs
make removal a permanent terminal state for that reference. REVOKE on a missing
reference succeeds idempotently because that reference confers no authority;
ACTIVATE cannot create it. Suspension is intentionally reversible using existing
trusted sessiond unlock policy, but does not restore previously cancelled grants.
No separate asynchronous REVOKING state is needed: the small transition is
synchronous under one broker guard.

## 6. Session identity and epoch design

`SessionRef { id, epoch }` is a **nonsecret reference**, not a bearer capability.
Every mutation of it separately authenticates the actual trusted service owner.
GrantScope's existing `session` and `session_generation` carry these two words;
there is no userspace-selected generation in registration.

A table lazily acquires an epoch from a kernel-global checked AtomicU64
allocator; row IDs increase without refund even on a subsequent failure. Neither
counter wraps. Different table instances in one kernel lifetime cannot alias.
Sessiond's restart-reset ID counters have been removed from its production
allocation path. Existing pure library counter tests remain historical helpers.

The row records original UAC and sessiond identities plus the account principal.
Sessions have **account-wide scope**, not membership of one desktop client.
Each capability separately binds a specific caller incarnation, VM, and call.
The trusted login TTY is not kept alive by a session reference. There are no raw
Process or VM pointers held across a syscall and no new VM lifetime references.

## 7. Grant commit linearization point

`Scheduler::account_grant` holds lifecycle serialization, authenticates the exact
pending recipient, and calls
`CapabilityBroker::commit_account_session_grant`. It checks participant liveness,
the broker row, activation, UID, operation policy, and expiration while retaining
SCHEDULER→CAP_BROKER. For own-password/profile grants target/member must equal
the session/caller UID; group grants require UID zero in addition to UAC policy.

Receipt capacity and Account grant capacity are reserved before mint. Existing
secure token allocation and retained-token-history checks remain in place.
Successful insertion, delivery ownership attachment, and retry-receipt update
complete before the guard is released or a token returned. That completed
insertion is the commit point. Failure publishes no grant; identity/token
history may conservatively retain burned IDs, never return them to reuse.

## 8. Revocation linearization point and retries

Broker row removal and removal of associated unconsumed Account grants under
CAP_BROKER are the authoritative revoke point. Production transitions also hold
SCHEDULER, matching grant admission and lifecycle retirement. Logout invokes this
direct synchronous kernel operation before replying success. There is no
broker-to-Account callback, outstanding revocation worker, or acknowledgment poll.

The kernel transition has no IPC timeout. At the outer session-control IPC
boundary, a caller that loses a reply has **unknown outcome**, not successful
logout by inference. Repeating logout/revoke is safe; a stale/missing session
cannot regain authority. Registration retry within the same authenticated call
returns its existing row after the proof has been consumed. A failed CREATE uses
`PendingAccountSession` cleanup; a successfully owned desktop remains sessiond's
responsibility even if its caller abandons the reply.

Mint retry in the same authenticated IPC call and same scope returns only its
original still-live, unexpired token. Consumption, cancellation, expiry, or a
changed scope prevents that call from minting another token. A later call is a
new authorization request, not a transparent retry of an arbitrary account
mutation. E2 supplies no cross-request database transaction idempotency claim.

## 9. Commit versus revoke correctness and synchronization

Both operations require the same exclusive broker guard. If revoke removes the
row first, commit cannot resolve Active and rejects. If commit inserts first,
revoke subsequently removes both row and unconsumed grant. Consumption racing
with revoke either removes the valid delivered grant first or finds no active
session/grant. At most one successful consumption occurs. Consumption does not
make later filesystem publication atomic with logout; that separate lifecycle
requirement is why account mutations remain disabled.

The production lock order is SCHEDULER→CAP_BROKER→existing entropy conditioner.
Exec already uses SCHEDULER→PMM→CAP_BROKER. Session methods add no PMM operation,
user-memory access, IPC delivery, cross-service callback, scheduler change, or
wait. Account/sessiond perform their ordinary blocking IPC before or after the
kernel transaction with no kernel spinlock held. Native isolated witnesses wait
on no barriers. Host concurrent tests place their thread barrier before lock
acquisition. UAC's single-threaded request state and sessiond's event-loop state
are not acquired by a broker callback. No circular service dependency was added.
Existing recipient retirement also takes CAP_BROKER→DELEGATED_CALLERS for its
separate caller table; E2 adds no reverse acquisition or nested service lock.

## 10. Account activation, restart, and incarnation retirement

SunlightD's real lifecycle uses `spawn_named_with_identity`,
`clear_dead_instance_for_start`, `spawn_service_at`, `apply_confirmed_exit`,
`stop_service`, `poll_service_exits`, `deps_ready`, and `autostart_services`.
The existing kernel image/launch policy establishes service roles and fresh
process/VM identity. E2 changes none of these dependency/readiness decisions.
Existing `recovery_blocked_loop` remains authoritative on blocked recovery.

No session authority is created merely by service startup. Registration requires
a fresh authenticated login proof. Exit/exec retirement removes sessions owned
by either original service and cancels their Account grants. Exact lifecycle
serialization prevents a grant from committing after authoritative retirement.
A stale delivered login proof cannot be converted into a new session by a new
UAC incarnation or a new VM at the same numeric PID. Delayed reply settlement
does not create a missing session or migrate its participants.

Restart does not transparently adopt an existing desktop session. Its graphical
record may remain for presentation, but Account grants refuse; logout/new login
is required to establish replacement authority. This intentional availability
restriction avoids inventing a restart reconciliation protocol. Native service
replacement and delayed-old-reply interleavings remain unverified.

## 11. Broker reinitialization and lifetime

CAP_BROKER is a static kernel mutex initialized at boot. It is not a restartable
SunlightD service. No production code resets its authority or token-history table
while processes remain alive. Other constructed brokers are isolated checks or
test fixtures, not replacement global brokers.

Host isolated replacement and native controlled boundary tests verify distinct
table epochs and exhaustion rejection. They do not establish hot replacement of
the production broker. Sessions and all authority are boot-scoped; a reboot
destroys processes, session references, grants, and IPC completions. No supported
persistence or recovery of a preboot session is promised. Epoch-allocator reset
within a live boot is not supported.

## 12. Expiration, suspension, and logout

The authoritative time is `Scheduler::global_tick`, sampled under lifecycle
serialization for each transaction. At `now >= deadline`, a session is removed
before a grant can commit or consume. Time cannot advance through another
scheduler operation while this guard is held. Grant TTL is independently checked
at mint retry and consume. No wall-clock or cross-vCPU timestamp comparison is
used in the security decision.

Current graphical sessions register with `u64::MAX`: they have no automatic
idle-expiration lease. Their supported lifetime ends at logout, service
retirement, or boot. The privileged registration primitive also accepts a finite
deadline, exercised by host/native isolated expiry tests. It does not promise an
existing graphical auto-expiration policy. Expired records are reclaimed lazily
at the next session operation or service retirement; no expired row can authorize
a grant in the interim. Table capacity remains bounded.

Sessiond maps Running/Degraded to ACTIVATE, intermediate/locked states to
SUSPEND, and stopping/failed states to REVOKE before replying. Explicit logout
requires successful synchronous revoke. Failed preparation revokes its suspended
registration using the pending-session guard. Unlock reuses existing trusted
sessiond policy; it cannot revive a terminal reference or cancelled token.

## 13. Consumption authorization policy by class

| Class | Current-session requirement and lifecycle |
|---|---|
| ChangeOwnPassword | Active original broker session; exact UID/target/member; UAC credential policy; recipient process/VM bound; one-use inline or delivered consumption as described below. Public mutation still refused. |
| UpdateOwnProfile | Same active-session/ownership/recipient rules. Public mutation still refused. |
| GroupCreate/AddMember/RemoveMember/Delete | Active UID-zero session and privileged UAC policy; recipient/VM bound, one-use inline or delivered consumption. Public group publication still refused. |
| Login AuthSessionGrant | Delivered one-use authentication proof. Not itself a graphical session. Session registration additionally requires original live UAC PID/incarnation/VM and exact trusted TTY caller. |
| VFS delegation | Existing independent root-authorized read/execute delegation; exact recipient/VM, one-use consumption/cancellation. Graphical logout does not retroactively revoke this independently delegated class. |

Existing Account mechanics support mint and immediate consumption within the
same exact authenticated UAC request before an IPC reply, used by the disabled
own-account mutation continuations. Otherwise consumption requires successful
delivery to the exact recipient process/VM. In both modes E2 first enforces the
same current authoritative session and original UAC identity; inline consumption
is not an untrusted bearer-only path. The host tests exercise both modes.

Exec and recipient exit retire Account delivery records and receipts. Cancellation
removes unconsumed authority. Revoke, suspension, expiration, or either service's
retirement cancels session-bound unconsumed grants. A consumed grant is already
removed; session cleanup does not double-retire it.

## 14. Restored Account functionality

| Operation | Acceptance | Boundary |
|---|---|---|
| Grant mint | RESTORED_PARTIAL | Real kernel path implemented; positive host and isolated native primitive evidence; authenticated graphical-session end-to-end native path not demonstrated |
| Grant consumption | RESTORED_PARTIAL | Same authoritative session check and one-use production primitive; real end-to-end native path not demonstrated |
| Password mutation | FAIL_CLOSED | Explicit Account handler refusal after existing policy checks |
| Account-profile mutation | FAIL_CLOSED | Explicit Account handler refusal after ownership checks |
| Group mutation | FAIL_CLOSED | Explicit refusal before consume/publication; separately authorized grant creation uses broker session checks |

Read-only account presentation and the earlier independent password/VFS
delegation path retain their existing scope. No UI success or real user mutation
is inferred from host tests or compilation.

## 15. Still-disabled functionality

`require_verified_mutation_publication` in Account returns
`SessionAuthorityUnavailable` for password/profile/group publication. Existing
current-password, own-account, administrative, revision, and storage policy checks
have not been replaced with a generic bypass. Boot development-shadow migration
is a separate existing maintenance path, not restoration of user password change.

Broker consumption is not a lease for a later userspace database rename. Full
mutation restoration needs its own publication/lifecycle contract and real
authenticated login/mutation verification, including any logout/restart after
consume. This report does not grant authorization to bypass those conditions.

## 16. Production source changes

* New `ipc/src/account_session.rs` and syscall 165; wire-compatible two-word
  session references; synchronous transition outcomes.
* New `kernel/src/capability/sessions.rs`: bounded authoritative state,
  nonwrapping IDs/epochs, principal and participant checks, commit/consume,
  revocation/expiry/retirement, and bounded request receipts.
* `capability/{mod,lifecycle,delivery}.rs`: integrate table, original UAC VM,
  exact caller registration and production grant entry, lifecycle cleanup.
* `process/exec_authority.rs`: clear session-service role on successful exec.
* sessiond production main: register/activate/suspend/revoke authoritative
  sessions; retire failed creation; broker-generated record identity.
* Account handler: retain explicit publication refusal independently of restored
  broker primitives. IPC Account error documentation updated.
* Host lifecycle fixture fields, new `broker_session_tests.rs`, native opt-in
  sequential witness, and E2 evidence runner/parser tests.

No scheduler algorithm, general IPC dispatch, SHM policy, Doorbell internals,
audio transport, Helios ABI, or DriverKit design was changed for E2. Other dirty
worktree changes predate this task and remain intact. No commit/push or forbidden
Git operation was performed.

## 17. Host regressions and S1–S18 evidence

Final kernel suite: **131 passed**. Service/library suite: **258 passed**.
SunlightD dependency-enabled runtime tests: **19 passed**. The focused scheduler
suite passes **63 tests**, and evidence parsers pass **14 tests**. Actual commands
and logs are indexed.

| Cases | Real primitives exercised / limits |
|---|---|
| S1, S3, S8 | Scheduler registration/activation, broker mint, exact-call retry, one-use consume; post-revoke unconsumed token rejected |
| S2, S5, S7 | Broker revoke/logout transition before commit, duplicate revoke, stale ACTIVATE refused |
| S4 | Host threads overlap actual broker commit and revoke; one exclusive guard chooses an order. No native service-race inference |
| S6 | Finite deadline checked exactly at boundary for consume/mint/activation |
| S9 | Controlled row-ID and epoch allocator exhaustion without wrap; bounded table capacity |
| S10–S14 | Actual Scheduler policy with synthetic process fixtures: exit flag, changed UAC/sessiond/caller incarnation and VM, same PID, old reference rejected |
| S12 qualification | Stale authority and existing delivery settlement tests; not a real delayed old-service IPC response across replacement activation |
| S15, S16 | Actual delivered Account grant consumption racing broker revoke/recipient cleanup; one-use removal; cancellation cannot remint the old request |
| S17 | Isolated production table replacement acquires another epoch; old reference rejected |
| S18 | Actual bounded admission/reservation and entropy failure, plus failed IPC delivery: no partial grant publication; real allocator OOM is not injected |

Additional tests cover registration-role/original-UAC-VM checks, suspended grant
cancellation, principal policy, and 256 successive calls retaining one receipt
per live recipient. Native CPU identities and real service restart are not
provided by these host fixtures.

The first expanded suite exposed a fixture that changed recipient VM while
retaining its old pending IPC call. The production exec path withdraws that call.
The fixture was corrected accordingly; the failed run (including subsequent
shared test-lock poison failures) is retained. It is not presented as a passing
attempt or evidence of a kernel fix.

## 18. Forced native race matrix

| Case | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs | Evidence limit |
|---|---|---|---|---|---|
| N1 revoke wins | NOT_FORCED | NOT_FORCED | NOT_FORCED | NOT_FORCED | Sequential isolated revoke-before-mint passes; no actual Account/service remote rendezvous |
| N2 commit wins | NOT_FORCED | NOT_FORCED | NOT_FORCED | NOT_FORCED | Isolated commit then revoke rejects consume; real service path not forced |
| N3 concurrent boundary | NOT_FORCED | NOT_FORCED | NOT_FORCED | NOT_FORCED | Host contention only |
| N4 Account exit | NOT_FORCED | NOT_FORCED | NOT_FORCED | NOT_FORCED | Isolated production retirement only; separate native recipient-exit probes are not Account exit |
| N5 Account restart | NOT_FORCED | NOT_FORCED | NOT_FORCED | NOT_FORCED | No authenticated replacement plus pending session grant |
| N6 stale reply | NOT_FORCED | NOT_FORCED | NOT_FORCED | NOT_FORCED | No delayed old UAC reply after replacement activation |
| N7 logout/consume | NOT_FORCED | NOT_FORCED | NOT_FORCED | NOT_FORCED | Sequential isolated/session host evidence only |
| N8 controlled reuse boundary | FORCED_AND_PASSED | FORCED_AND_PASSED | FORCED_AND_PASSED | FORCED_AND_PASSED | Isolated real table ID/epoch exhaustion and replacement epoch only; no global live-broker restart |

The missing fixture must route a real verified UAC login through the real trusted
TTY→sessiond registration path, release all kernel locks before a bounded
rendezvous, and then independently observe trusted revocation/replacement
progress on another CPU. Existing native production grant probes authenticate
ordinary delegation callers; they are not trusted TTY login callers. Installing
a TTY/sessiond role on those clients would bypass the production trust boundary
and has not been done. Merely adding more isolated broker workers would still
not certify the requested native service cases.

## 19. Native 1/2/4/12 CPU results

The authority-enabled image passes E's full lifecycle/refusal/history markers
plus E2's four sequential witnesses on 1/2/4/12 CPUs. The witnesses run on actual
CPU 0 in each guest, with synthetic issuer incarnation 60, owner incarnation 80,
recipient incarnation 70, VM 71. Logs explicitly state
`scope=isolated-production-broker authentication=NOT_TESTED`. Those constants
are fixture identities, not observations of live services. No remote CPU session
race is claimed. The no-entropy boot test refuses user authority publication.
The isolated expiry witness supplies controlled tick values to the production
primitive; it does not prove a live graphical idle-expiration workflow.

Other production and compatibility runs are listed individually in the final
evidence index with actual CPU counts, image hashes, commands, and raw logs.
Generic boot PASS cannot upgrade any N1–N7 classification.

The validation runner intentionally stops QEMU after the required serial
assertions. This is not evidence of a clean guest shutdown. Process/service
cleanup is supported only by each probe's explicit exit/reap/resource assertions.

## 20. Focused resource accounting

Each native sequential fixture records four registrations, two revocations, one
expiration, one service retirement, zero final live sessions, zero receipts, and
zero Account grants. Session Vec capacity is retained and explicitly reported;
it is not a live orphan. Two isolated table epochs are allocated. Exhaustion
attempts publish no replacement authority. The real global broker is not reset.

The implementation admits at most 16 session records and 64 recipient receipts
per session. A later monotonic call replaces its recipient's earlier receipt;
recipient exec/exit reclaims it. Session retirement drops its whole receipt Vec.
No pending commit, external consumption reference, asynchronous revocation,
protocol IPC completion queue, retired-row list, or Process/VM reference object
is allocated by this synchronous protocol. Their zero counters describe this
design/isolated fixture, not unmeasured global IPC/service resources. Checked
epoch identity numbers intentionally remain spent; their scalar allocator is not
an ever-growing allocation list.

No global Broker/VFS/UAC reconciliation claim follows. Native live graphical
session creation/retirement and high-concurrency resource drift remain missing
acceptance evidence. A stale session can remain a bounded metadata row until
cleanup/expiry access, but every grant resolves live participants and refuses it.

## 21. Compatibility replay

Final validation covers production kernel/services, capability/broker/IPC/VFS
host suites, process exit/exec checks, pin-release, native Doorbell, scheduler,
SunlightD D1/D2/D3 and RecoveryBlocked, VM/TLB/unmap/protect, resident boot, and
TCP/TLS with certificate verification. Results, failed attempts, and exact
per-suite CPU coverage are in the machine-readable index. The requested native
session replacement/concurrency cases remain outside these compatibility
successes. Media results retain their separate missing-fixture failure.

| Replay | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|---|
| Authorized UAC/VFS delegation and existing recipient-exit orderings | PASS | PASS | PASS | PASS |
| Unregistered Account refusal, finite history boundary, isolated E2 sequential primitives | PASS | PASS | PASS | PASS |
| Pin-release | PASS | PASS | PASS | PASS |
| Native Doorbell | PASS | PASS | PASS | PASS |
| Scheduler | PASS | PASS | PASS | PASS |
| D1/D2/D3 and RecoveryBlocked after-ready | PASS | PASS | PASS | NOT_RUN |
| VM/TLB | NOT_RUN | NOT_RUN | NOT_RUN | PASS |
| VM unmap/protect | NOT_RUN | NOT_RUN | PASS | NOT_RUN |
| Resident audit and ordinary production boot | NOT_RUN | NOT_RUN | NOT_RUN | PASS |
| TCP/TLS certificate/hostname/SOCKS checks | PASS | PASS | PASS | PASS |

The existing D1 gate supports 1/2/4 CPU inputs; no 12-CPU D1/D2/D3 result is
invented. TCP/TLS initially failed before starting the guest because sandbox
socket creation was denied. The retained second attempt permits local fixture
sockets and passes all eleven cases at every CPU count, including untrusted
issuer and wrong-hostname rejection. This is an environment retry, not a TLS
verification relaxation.

## 22. Baseline failures and evidence boundaries

Strict Clippy fails with `-D warnings`, without suppressing existing categories.
It stops in existing dependency/build-script diagnostics; this is not proof of
a clean strict analysis of the new kernel module. Workspace formatting remains
failing, including a new module-ordering suggestion in
`capability/production_tests.rs`; changed-file differences are not all attributed
to baseline. The new standalone session module, host test module, and IPC module
pass their individual rustfmt check. The existing repository WAV test fails with
file-not-found; the other 36 media tests pass. Miri is unavailable in the installed
nightly toolchain.
The comparison in the index matches diagnostic source identities/messages to E
where available; changed-file line offsets are not treated as new findings by
themselves. None of these failures is relabeled PASS.

Historical unexplained D3 failure is preserved even if current D1/D2/D3 replay
passes. Gate B remains PARTIAL. Prior failed runs, the E2 fixture failure, and all
current retries remain indexed. Parser fixture strings validate evidence
classification only and are never substituted for guest execution measurements.

## 23. Stable final-source identity

The E2 runner snapshots 1,276 tracked and untracked Rust/TOML/linker/Python/shell/ASM,
Cargo.lock, and boot configuration sources. Each validation section compares its
snapshot afterward; the aggregate verifies the final tree again. Ordinary
services are built and staged before production kernel embedding; test-enabled
services/images are distinct. Each image records kernel ELF, stripped ELF, ISO,
compressed archive, and embedded-service SHA-256 identities. Commands retain
their actual environment, feature selection, paths, exit codes, and timings.
The first incremental kernel build used previously staged service binaries; it
is excluded from final production identity. The production section stages fresh
ordinary services before kernel embedding. The final ordinary build additionally
checks the actual ELF payloads of init, SunlightD, UAC, sessiond, and VFS inside
the kernel and verifies the E2 test marker is absent. Its 12-CPU ordinary boot
passes. All section source snapshots and the final source manifest match.
The final ordinary kernel SHA-256 also matches the earlier correctly staged
ordinary production build. Test-enabled images have their own distinct hashes.

Raw logs and manifests are under the requested `target/...` directory. Build
outputs and archived ISOs are in `/tmp/f11b0e2-build` because the workspace
filesystem has less than 350 MiB free. Archive paths/hashes are explicit; `/tmp`
is ephemeral and is an artifact-retention limitation. The compressed ISO can be
hashed after decompression to match the recorded ISO identity. No unrelated
artifacts were deleted to make space. Documentation/index creation is separately
identified and does not alter the validated production source snapshot.

## 24. Remaining authorization blockers

* Real authenticated login→registration→activation→positive Account mint/consume
  native end-to-end evidence.
* N1–N7 bounded native rendezvous and actual remote CPU progress, including UAC
  and sessiond retirement/replacement and delayed old reply.
* Native numeric PID reuse with distinct process/VM incarnations; host fixtures
  and monotonically allocated guest PIDs do not establish this.
* A complete account-mutation publication contract and independent operation
  acceptance before removing their explicit refusals.
* Global Broker/VFS/UAC accounting and remaining lifecycle/exec/restart races.
* Finite retired-token-history availability and global native exhaustion/error
  delivery; other issuance APIs still have less precise capacity reporting.
* Historical D3 acceptance issue and the separately recorded tooling/fixture
  limitations.

## 25. Gate decisions

| Gate | Decision | Reason |
|---|---|---|
| A: scheduler pin-release | VERIFIED NATIVE | Final 1/2/4/12 replay passes; no scheduler redesign |
| B: native Doorbell | PARTIAL | Historical D3 issue is unresolved |
| Session Authority | PARTIAL | Real protocol implemented; host/isolated sequential evidence; required authenticated native races absent |
| Token-History | PARTIAL | Prior finite ledger preserved and replayed; independent availability/exhaustion conditions remain |
| F1.1B-0 production grant authority | BLOCKED | Native PID reuse, lifecycle and global accounting prerequisites remain |
| C: production FastLane | BLOCKED | No Restricted SHM, public Doorbell, or production cross-process FastLane activation |

Account statuses are independent: mint and consumption RESTORED_PARTIAL;
password, profile, group mutation FAIL_CLOSED. No full functionality or full E2
native acceptance is declared.

## 26. Handoff for F1.1B-0F

Do not start Restricted SHM. First finish the missing E2 authenticated session
fixture and N1–N7, with bounded unlocked rendezvous, actual CPU identities,
source/image identity, and per-attempt cleanup counters. Keep those missing E2
acceptance conditions visible rather than renaming them completed F0F work.

Then F1.1B-0F should force recipient numeric PID reuse with old deliveries and
new process/VM incarnations; finish pending issuer/recipient exec/exit/restart
and lost-reply interleavings; reconcile broker, VFS and UAC live/retired ownership
across cancellation, consumption and failure; and resolve or explicitly bound
finite token-history availability. Re-run the full stable-source SMP and
compatibility matrix and preserve the historical D3 issue. Account mutation
publication needs separate per-operation acceptance. This is a focused handoff
plan only; F1.1B-0F was not started here.
