# FastLane F1.1B-0E3T: N4 ordering and N3M failure classification

2026-10-26. **Session Authority remains PARTIAL. F1.1B-0 and Gate C remain
BLOCKED.** No production authorization algorithm or scheduler policy changes.
Restricted SHM, production FastLane and public Doorbell APIs remain disabled.

Evidence: `target/fastlane-f11b0e3t-evidence/validation-summary.json`. Raw attempt
manifests retain commands, CPU counts, identities, ordering, failures, image
hashes and cleanup observations. Generated result tables below describe actual
executions; this report does not replace earlier evidence.

## 1. Executive summary

N4 now has a genuine Account grant-syscall checkpoint before lifecycle/broker
serialization. The observed 2/4/12-CPU production outcome is
**PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT**, not FORCED_AND_PASSED: a cooperative
stop cannot complete authoritative retirement while that nonpreemptible syscall
is executing. The still-live Account commits before signal delivery; its exit
then removes the session and undelivered grant and cancels the actual IPC call.
The requested row-retirement-before-old-commit interleaving itself was not
forced. One-CPU userspace cancellation remains NOT_FORCED.

N3M means the N3 **mint-first** concurrent ordering (fixture ordinal 3), paired
with N3R, the revoke-first ordering (ordinal 9). The original N3M failure is a
real post-race fixture assertion at an endpoint lookup. Its transport/root cause
remains unexplained. Subsequent passing executions do not resolve that failure.
No production correction is justified by the available trace.

## 2. Confirmed E3S baseline

Read the E3S, E3R, E3, E2, D and C reports and their manifests. At entry the
worktree was clean and all selected source hashes matched E3S's frozen fixture
manifest `build-20261026T082936.888525Z`. E3S recorded 27 successful native
executions, one failed N3M, and one N4 NOT_FORCED. N5, N6 and replacement fresh
login passed at 2/4/12 CPUs; N1/N2/N3/N7 evidence was retained. E3S changed no
production Rust. Its source/image hashes matched and whitespace checks passed.
Those are historical results, not E3T executions.

E3S also recorded a new two-CPU D3 failure, independent of N3M. The original
Doorbell D3 failure remains at
`target/fastlane-f13na-evidence/d1-d2-d3-failed-final-harness/serial-2cpu.log`.
The baseline preservation inventory hashes earlier E3/E3R/E3S and original D3
logs/manifests. None is rewritten. Entry disk availability was 38 GiB, not the
historical 45 GiB estimate. No build-tree or unrelated-file cleanup was used.

## 3. N4 production call flow and lock ordering

1. Kernel init authenticates SunlightD. Only that active manager spawning the
   embedded `/sbin/uac_service` installs `trusted_auth_broker` in `sys_spawn`.
   Identity is PID + process incarnation + VM generation, not the basename.
2. Real TTY password login obtains a UAC proof. Trusted sessiond registers and
   activates a kernel-owned `(session, epoch)` through syscall 165.
3. Account `account_service::dispatch_group` validates the real
   request, active-session presentation, administrator policy, revision and
   password, then calls `ipc::accounts::try_broker_grant` (syscall 151).
4. `sys_account_grant` takes SCHEDULER and calls `Scheduler::account_grant`.
   `grant_requester_for_role` rejects an exiting/untrusted issuer or recipient
   and verifies the actual pending IPC call's PID/incarnation/generation.
   `grant_delivery` binds issuer and recipient VM identities and the call.
5. With SCHEDULER retained, CAP_BROKER protects session lookup, participant
   liveness, active state, principal and operation authorization.
   `commit_account_session_grant` reserves receipt capacity, calls
   `mint_account_grant` to reserve grant/token-history capacity and insert the
   grant, attaches delivery ownership, and installs the retry receipt. This is
   one transaction to all other broker/lifecycle observers. The mint marker is
   after this transaction succeeds, while both guards are still retained.
6. SunlightD's authenticated restart uses its existing eager-service
   `libc::kill` branch. SIGTERM publication is **not** authority withdrawal.
   Default fatal signal delivery occurs after syscall dispatch, before SYSRET.
   SIGKILL instead calls `terminate_process_by_pid` under SCHEDULER, marks
   Finished/cleanup-pending and immediately makes `is_exiting` authorization
   checks fail; it does not permit reaping an executing stack.
7. `finish_current_process`/safe reaping invokes `close_owned_ipc_endpoints` or
   `revoke_endpoints_owned_by` -> `retire_recipient_grants` ->
   `retire_sessions_for`. Session removal and associated grant/receipt removal
   under SCHEDULER -> CAP_BROKER are authoritative row retirement.
8. Endpoint removal gathers the real `IpcCallId`s.
   `finish_peer_closed_calls` completes generation-qualified PeerClosed.
   Safe reaping clears pending/reply/deferred contexts before replacement
   authentication. Exec separately retires grants and withdraws image roles.

There is no independently authorized asynchronous pending-mint object between
final authorization and insertion: SCHEDULER remains borrowed through CAP_BROKER
mutation. A syscall entrant can be pending before those locks; no cached
checkpoint identity confers authority. Safe resource reaping refuses an owned,
queued or pinned execution stack. A remotely Finished issuer fails subsequent
production liveness validation. A cooperative SIGTERM is applied after the
running syscall, so its commit can legitimately precede retirement.

## 4. Why E3S N4 was NOT_FORCED

E3S paused at Account's stage-zero userspace timing IPC before syscall 151.
Restart destroyed that continuation. Replacement B's later old-session denial
was not A resuming a grant attempt. The earlier apparent PASS was an evidence
acceptance error already superseded by E3S. E3T retains that historical trace
and does not recertify it as a kernel precommit race.

## 5. Deterministic N4 fixture

All additions are behind `native_session_race_test` or the existing
`session-race-test` fixture binary. A real UAC policy rendezvous arms observation
for the actual session. The ordinary controller releases UAC and observes a
read-only test checkpoint through DebugLog. This does not confer authority.
`observe_n4_pending` uses the production requester/call lookup, verifies live
session participants, and records the original issuer, recipient and call before
any metadata reservation/commit. The grant syscall then waits **outside all
locks** for the real manager's queued SIGTERM, with a bounded same-CPU TSC and
iteration watchdog. It neither injects nor consumes a signal.

The initial CPU-0 wait timed out before SunlightD could send the signal; the
client then observed an ordinary grant reply instead of PeerClosed and failed.
This is retained as a failed fixture diagnostic, not a passing authorization
race. CPU 0's nonpreemptible syscall also prevents BSP timekeeper progress.
The correction uses a separate N4 image with **initial** UAC affinity on CPU 1
and controller affinity on CPU 0, using the existing test placement pattern.
No running-task state, scheduler algorithm, timeout or security decision changes.
A build-time environment flag selects this image and its manifest records it;
the driver rejects using it for other race cases.

At the real SIGTERM observation, Account still owns its execution CPU and is
not exiting. The barrier returns; the unmodified production transaction runs;
normal signal handling exits Account before it can return the grant to the
client. The fixture uses actual SunlightD restart, actual PeerClosed and actual
replacement. No old reply or retired task is resurrected.

## 6. Exact ordering and its limits

The accepted prevention sequence is:

```
original Account syscall entry -> authenticated recipient/call checkpoint
-> manager SIGTERM on a different CPU -> pending SIGTERM, Account execution held
-> legitimate mint under live authority -> Account exit/session retirement
-> bound PeerClosed -> old IPC context retired -> independently authenticated B
-> B denies old session -> both fixture processes reaped -> terminal zero inventory
```

The precommit checkpoint precedes full final broker authorization; it is not a
claim that a grant was already authorized or reserved there. The successful
ordinary commit establishes the request was legitimate at commitment.

This demonstrates deferred cooperative retirement plus cancellation of the
undelivered result. It does **not** execute an old broker commit after session
row retirement. It does not force the separate remote SIGKILL-before-validation
branch; source/host coverage of that denial must not be described as this native
interleaving. N4's requested ordering is prevented in the exercised production
stop path, and its classification remains distinct from a forced ordering.

## 7. N4 CPU matrix

Final results, failed attempts and actual participating CPU identities appear in
the generated tables below. One CPU cannot run the two real kernel contenders
concurrently; that result stays NOT_FORCED. Four/twelve-CPU expansion follows
successful two-CPU prevention evidence. Repeated twelve-CPU runs are separate
bounded guest boots, not steady-state multi-cycle allocation measurements.

## 8. Original N3M failure reconstruction

`original-n3m-reconstruction.json` retains the exact original QEMU command,
2-CPU/2048-MiB configuration, feature set, service hashes, image identity,
parser output, failure and preceding fixture invocations. The original serial
SHA-256 is
`d25d69f627d7dfc67bfad85612121038849bb13b78ff1cb05cd30a55c72b27e8`.
The old ISO is
`696c1fb27f4c2a4a27614e4bb6a0d090efe1ab898f93ca4107d10d6fc84a6d2c`;
all recorded image/service/source identity checks were true.

Both real contenders reached the boundary on different CPUs. Account
`(pid=20, incarnation=32, vm=51)` committed sequence 15 for `(session=1, epoch=1)`;
sessiond `(29,41,69)` revoked at 17. The genuine recipient was `(39,51,87)`,
call generation 4. Grant/receipt/session inventory was zero after revocation.
The controller was reaped. The client received the expected mint reply and
then panicked at original `session_race_probe.rs:13`, the endpoint helper.

That helper collapses timeout, transport failure and non-GRANT reply into None.
The call flow identifies a `uac` lookup during either the stale AUTHORIZE check
or the following retired-grant consumption check; the old trace cannot
establish which lookup failed. Neither preliminary Account denial necessarily
enters the broker, so absence of a new mint marker does not distinguish them.
There is no observed UAC exit or explicit matching lookup-denial message.

The driver detected guest failure, not its QEMU watchdog. It terminated QEMU
before the failed client's final reaper was captured. Its last observed zero
broker inventory is not proof of complete failed-attempt cleanup. N1/N2 preceded
N3M in the driver, but each case booted a separate guest; their guest state was
not shared. Host invocation order is retained separately.

## 9. N3M classification and first divergence

**UNEXPLAINED_POST_RACE_ENDPOINT_LOOKUP_FAILURE**. The failing assertion and its
location are demonstrated; the underlying cause is not. The captured broker
ordering is correct. No authorization defect, scheduler defect, parser error,
image mismatch, particular timeout, or harmless fixture-only cause is proven.
The passing comparator differs in controller/GUI teardown timing around the
mint reply, but both paths reap the controller before stale checks; that
observation is not a causal explanation.

The diagnostic fixture preserves the same 2000-ms lookup and error behavior,
recording grant/non-GRANT/transport outcome and elapsed time without bearer
values. A new occurrence can therefore distinguish branches the old trace
cannot. Passing diagnostic attempts cannot recover missing historical facts.

## 10. N3M reproduction matrix

Before source edits, the exact E3S image and matching source were replayed as
N1/N2/N3M/N3R on 2 CPUs, then isolated N3M on 2 and 4 CPUs. All six executions
passed; N3M itself passed three times. Final-source diagnostic N3M then passed
once each on 2/4/12 CPUs: six new N3M passes total, with no reproduction of the
original failure. This finite sample does not settle its cause. The first matrix reproduces original
host fixture order; it correctly keeps the fixtures in separate guests.
Final-source diagnostic replays and additional ordering/CPU comparisons are
listed below, including every failure and retry. No unbounded stress, changed
lookup timeout, disabled SMP or altered authorization was used.

If these bounded runs do not reproduce the failure, the issue remains open.
Plausible unproven classes remain lookup transport/deadline/non-GRANT failure
and their underlying scheduling/lifecycle causes. There is no evidence basis
to select one. The historical Doorbell D3 failure is a separate issue.

## 11. Minimal code changes

The kernel adds test-only entry observations, a bounded N4 barrier, an
observation query, an authenticated manager-signal marker and N4 image placement.
The fixture adds checkpoint synchronization and bounded lookup diagnostics.
The evidence parser distinguishes prevention from forced ordering, binds the
pending call and old issuer, and retains failed attempts in aggregate audits.
Changed Rust files receive rustfmt; pre-existing formatting differences within
those files are formatting-only. Production grant/session decisions, ordinary
service policy and scheduler algorithms are unchanged. No capability subsystem,
channel, Account database mutation or production syscall was introduced.

## 12. Evidence-parser regressions

Counterexamples cover missing authentication, wrong issuer/recipient/VM/session,
missing/late precommit, missing retirement/signal/ownership/cancellation, reversed
order, forbidden mint, missing terminal inventory, retained IPC state, stale
image/source hashes and driver timeout. Prevention never becomes
FORCED_AND_PASSED. Existing userspace-only N4 stays NOT_FORCED. Aggregate audits
reject omission or reclassification of the historical failed N3M. Synthetic
parser examples are not native evidence. Earlier formats remain supported;
prevention requires the new explicit checkpoints.

## 13. Lifecycle and grant accounting

For accepted N4 prevention runs, one live-authority mint becomes one undelivered
pending grant; retirement removes it without consumption or delivery to the
client. Retirement is counted once; later logout is idempotent. The bound
PeerClosed generation matches the precommit call. Original and replacement
service incarnations/VMs differ. Both fixture reapers and old-service reply
context retirement are required. Final sessions, receipts, Account grants and
pending Account deliveries are zero. One-use behavior is independently replayed.

Stored role bits in a terminal slot are not usable issuance authority:
production requires `!is_exiting` plus live incarnation/VM/call checks. The old
continuation is destroyed; B receives independent launch-chain authentication.
No claim of numeric PID reuse follows from process-slot reuse or replacement.

Retained empty session capacity and grant capacity are distinct from active
records. Metadata observations, delivered-unconsumed and derived retired counts
remain in each raw trace. The reaper's pending-commit zero is a serialized local
observation, not a global pending-commit gauge. Global IPC references, all
service/proof references, deferred work outside these fixtures, allocator usage
and global Broker/VFS/UAC reconciliation remain UNMEASURED. Token history retains
prepared values with its existing 65,536-entry limit. No global drift acceptance
is inferred from separate boots or empty local tables.

## 14. Compatibility replay

Final-source host and native results are listed below. The established validators
cover capability/session logic, Account, VFS, process exec/exit, SunlightD
D1/D2/D3, RecoveryBlocked, scheduler pin release, native Doorbell, VM/TLB,
unmap/protection, opt-in SHM retirement and native TCP/TLS certificate validation.
Each suite retains its own image/features and raw evidence; a compatibility
image is not relabeled as an authenticated session-race image. D3 passes cannot
explain its independent historical failure.

## 15. Source and build consistency

The final ordinary production image additionally boots on two CPUs with
PMM/VMM/SMP and real SunlightD/UAC readiness, without session-test markers.
The archived init, SunlightD, TTY, UAC, sessiond and VFS binaries are byte-for-byte
identical to E3S; only the diagnostic session-race client changed.

Final N4 and ordinary session fixtures use the same selected final source map,
with different recorded test environment settings. Their immutable stripped
ELFs, ISOs and embedded Account/UAC/VFS/service executables are hashed and
checked per attempt. Mutable Cargo outputs can be reused by compatibility
builds; immutable fixture archives remain authoritative. A final source
comparison follows execution. The selected map includes Rust, TOML, linker,
Python, shell, assembly, lock and boot configuration inputs; it is not a claim
of hermetic hashing of every repository asset.

Earlier-source diagnostics, the failed build and original-image reproduction
are distinct from final-source acceptance. Every failure is retained. Production
and opt-in prerequisite images keep transport blocked; ordinary image feature
isolation is checked separately.

## 16. Historical and current failures

The current D1/D2/D3 replay passes one CPU and fails two CPUs at
`d3_driver.rs:50`: graph worker 4 receives ERROR/53759 with DEPENDENCY/7,
expected OK/53632. The current RecoveryBlocked command fails its one-CPU D3
prerequisite at `d3_driver.rs:18` (`d3-leaf` phase 3, STALE/6); this is a
failed combined command, and the RecoveryBlocked scenario itself is NOT_RUN,
not a demonstrated recovery-authority regression. Later CPU counts are NOT_RUN
after those failures. No unrelated D3 repair or assertion weakening is made.

The original N3M and original D3 failures remain unresolved. E3S's newer two-CPU
D3 failure remains preserved. Strict Clippy, workspace-wide formatting, missing
WAV fixture and unavailable Miri are reported as failures/unavailability, never
PASS. Available baseline diagnostics are compared without restoring an old Git
tree: **MATCHED BASELINE UNAVAILABLE** for a matched old-tree replay.

E3T retains the initial BSP-blocking N4 diagnostic, a test-hook borrow-check build
failure corrected by sampling CPU configuration before mutably borrowing the
child, and a sandbox QMP socket bind failure before guest execution. The socket
failure is a harness/environment failure, not a guest race; its retry uses
approved local-socket execution. All appear in the machine-readable inventory.

## 17. Session Authority acceptance decision

**PARTIAL.** Current D3 compatibility failure and the RecoveryBlocked scenario
blocked by its D3 prerequisite also prevent complete final-source acceptance.
Native prevention evidence and successful later races cannot
causally resolve the original N3M failure. No upgrade follows from unrelated
compatibility passes. N4 prevention and N4 forced ordering stay separate. Any
additional required final-source failure/missing evidence is retained below.
Gate A remains VERIFIED NATIVE absent a demonstrated new scheduler regression;
Gate B and Token-History remain PARTIAL; F1.1B-0 and Gate C remain BLOCKED.

## 18. Remaining F1.1B-0 blockers

| Invariant / requirement | Evidence boundary | Minimum next action |
|---|---|---|
| N4 old commit after authoritative row retirement | Not forced; native cooperative-stop prevention tested | Retain prevention category; additional lifecycle variants require their own native evidence |
| Original N3M post-race lookup failure | Cause unexplained; old trace lacks outcome branch | Reproduce with retained lookup outcome diagnostics; fix only a proven cause |
| Recipient numeric PID reuse | NOT_FORCED | Native incarnation-bound reuse fixture |
| Remaining essential grant lifecycle races | Incomplete | Targeted production-path interleavings |
| Global Broker/VFS/UAC reconciliation | Incomplete; local session subset only | Inventory active references separately from retained capacity/allocator |
| Token-history lifetime/capacity | PARTIAL | Address lifetime/exhaustion acceptance independently |
| Native Doorbell D3 failure and compatibility | Historical cause unexplained; current two-CPU D3 replay also fails | Investigate its own trace; rerun the blocked recovery prerequisite after a demonstrated correction |
| Restricted SHM/production transport | BLOCKED | No activation in this phase |

## 19. Handoff

Do not automatically create another Session Authority phase. The unresolved N3M
lookup/progress question can be isolated while independent F1.1B-0F native PID
reuse and grant-accounting work proceeds, retaining Session Authority PARTIAL
and production gates BLOCKED. Keep independent work off the failing D3 graph
path and retain that compatibility prerequisite. Preserve the final diagnostics and avoid changing
security semantics to make the fixture pass. No Restricted SHM work begins.

<!-- E3T EXECUTION RESULTS -->

### Recorded execution results

The machine-readable index is authoritative for every command, image and retry. The socket-denied attempt is NOT_RUN at the execution layer; its original parser failure is preserved.

| Case | CPUs | Classification | Source scope | Raw evidence |
|---|---:|---|---|---|
| N3M | 2 | FORCED_AND_FAILED | HISTORICAL_ORIGINAL_N3M_FAILURE | `target/fastlane-f11b0e3s-evidence/native-20261026T083411.699898Z/N3M-2cpu.serial.log` |
| N4 | 1 | NOT_FORCED | EARLIER_E3T_DIAGNOSTIC | `target/fastlane-f11b0e3t-evidence/native-20261026T095047.821588Z/N4-1cpu.serial.log` |
| N4 | 2 | FORCED_AND_FAILED | EARLIER_E3T_DIAGNOSTIC | `target/fastlane-f11b0e3t-evidence/native-20261026T095212.697651Z/N4-2cpu.serial.log` |
| N4 | 2 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T095657.652109Z/N4-2cpu.serial.log` |
| N4 | 1 | NOT_RUN | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T095817.355057Z/N4-1cpu.serial.log` |
| N4 | 1 | NOT_FORCED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100111.238077Z/N4-1cpu.serial.log` |
| N1 | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100148.267494Z/N1-2cpu.serial.log` |
| N2 | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100148.267494Z/N2-2cpu.serial.log` |
| N3M | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100148.267494Z/N3M-2cpu.serial.log` |
| N3R | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100148.267494Z/N3R-2cpu.serial.log` |
| N7R | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100148.267494Z/N7R-2cpu.serial.log` |
| N7C | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100148.267494Z/N7C-2cpu.serial.log` |
| N7OC | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100148.267494Z/N7OC-2cpu.serial.log` |
| N7OR | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100148.267494Z/N7OR-2cpu.serial.log` |
| N4 | 4 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100207.247609Z/N4-4cpu.serial.log` |
| N4 | 12 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100314.927573Z/N4-12cpu.serial.log` |
| N4 | 12 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100413.879374Z/N4-12cpu.serial.log` |
| N1 | 1 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100552.123664Z/N1-1cpu.serial.log` |
| N2 | 1 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100552.123664Z/N2-1cpu.serial.log` |
| N7R | 1 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100552.123664Z/N7R-1cpu.serial.log` |
| N7C | 1 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100552.123664Z/N7C-1cpu.serial.log` |
| N3M | 4 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100925.893966Z/N3M-4cpu.serial.log` |
| N3R | 4 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100925.893966Z/N3R-4cpu.serial.log` |
| N7OC | 4 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100925.893966Z/N7OC-4cpu.serial.log` |
| N7OR | 4 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100925.893966Z/N7OR-4cpu.serial.log` |
| N3M | 12 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100925.893966Z/N3M-12cpu.serial.log` |
| N3R | 12 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100925.893966Z/N3R-12cpu.serial.log` |
| N7OC | 12 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100925.893966Z/N7OC-12cpu.serial.log` |
| N7OR | 12 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T100925.893966Z/N7OR-12cpu.serial.log` |
| N5 | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T101440.265635Z/N5-2cpu.serial.log` |
| N5 | 4 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T101440.265635Z/N5-4cpu.serial.log` |
| N5 | 12 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T101440.265635Z/N5-12cpu.serial.log` |
| N6 | 2 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T102016.426297Z/N6-2cpu.serial.log` |
| N6 | 4 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T102016.426297Z/N6-4cpu.serial.log` |
| N6 | 12 | FORCED_AND_PASSED | FINAL_SOURCE | `target/fastlane-f11b0e3t-evidence/native-20261026T102016.426297Z/N6-12cpu.serial.log` |
| N3M | 2 | FORCED_AND_PASSED | ORIGINAL_E3S_REPRODUCTION | `target/fastlane-f11b0e3t-evidence/original-isolated/native-20261026T094444.653534Z/N3M-2cpu.serial.log` |
| N3M | 4 | FORCED_AND_PASSED | ORIGINAL_E3S_REPRODUCTION | `target/fastlane-f11b0e3t-evidence/original-isolated/native-20261026T094444.653534Z/N3M-4cpu.serial.log` |
| N1 | 2 | FORCED_AND_PASSED | ORIGINAL_E3S_REPRODUCTION | `target/fastlane-f11b0e3t-evidence/original-source/native-20261026T094137.500791Z/N1-2cpu.serial.log` |
| N2 | 2 | FORCED_AND_PASSED | ORIGINAL_E3S_REPRODUCTION | `target/fastlane-f11b0e3t-evidence/original-source/native-20261026T094137.500791Z/N2-2cpu.serial.log` |
| N3M | 2 | FORCED_AND_PASSED | ORIGINAL_E3S_REPRODUCTION | `target/fastlane-f11b0e3t-evidence/original-source/native-20261026T094137.500791Z/N3M-2cpu.serial.log` |
| N3R | 2 | FORCED_AND_PASSED | ORIGINAL_E3S_REPRODUCTION | `target/fastlane-f11b0e3t-evidence/original-source/native-20261026T094137.500791Z/N3R-2cpu.serial.log` |

### Final-source host checks

| Check | Result | Log |
|---|---|---|
| kernel-host | PASS | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/kernel-host.log` |
| services-host | PASS | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/services-host.log` |
| sunlightd-runtime | PASS | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/sunlightd-runtime.log` |
| scheduler-host | PASS | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/scheduler-host.log` |
| parsers | PASS | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/parsers.log` |
| strict-clippy | FAIL | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/strict-clippy.log` |
| media-host | FAIL | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/media-host.log` |
| workspace-format | FAIL | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/workspace-format.log` |
| diff-check | PASS | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/diff-check.log` |
| miri | FAIL | `target/fastlane-f11b0e3t-evidence/checks-20261026T095657.646360Z/miri.log` |

### Final-source compatibility

| Suite | Native results | Failed commands | Manifest |
|---|---|---|---|
| authority-20261026T100550.962663Z | {'PASS': 5} | none | `target/fastlane-f11b0e3t-evidence/compatibility/authority-20261026T100550.962663Z/validation-summary.json` |
| core-20261026T101637.340876Z | {'PASS': 15} | none | `target/fastlane-f11b0e3t-evidence/compatibility/core-20261026T101637.340876Z/validation-summary.json` |
| lifecycle-20261026T100925.898526Z | {} | d1-d2-d3, recovery-blocked | `target/fastlane-f11b0e3t-evidence/compatibility/lifecycle-20261026T100925.898526Z/validation-summary.json` |
| network-20261026T102639.885583Z | {'PASS': 1} | none | `target/fastlane-f11b0e3t-evidence/compatibility/network-20261026T102639.885583Z/validation-summary.json` |
| production-20261026T100148.268201Z | {'PASS': 4} | none | `target/fastlane-f11b0e3t-evidence/compatibility/production-20261026T100148.268201Z/validation-summary.json` |

### Lifecycle nested results

| Suite | CPUs | Result | Boundary |
|---|---:|---|---|
| d1-d2-d3 | 1 | PASS | combined D1/D2/D3 completed |
| d1-d2-d3 | 2 | FAIL | d3_driver.rs:50 graph worker 4 ERROR/53759 DEPENDENCY/7 instead of OK/53632 |
| d1-d2-d3 | 4 | NOT_RUN | driver stopped after two-CPU failure |
| recovery | 1 | FAIL | D3 prerequisite d3_driver.rs:18 d3-leaf phase=3 STALE/6; recovery scenario not reached |
| recovery | 2 | NOT_RUN | driver stopped after one-CPU D3 prerequisite failure |
| recovery | 4 | NOT_RUN | driver stopped after one-CPU D3 prerequisite failure |

### Final artifact verification

* ordinary_session: ISO `daeae68386dc2a7b3ae0850a91e093e52026ab4f440a418afea7e994d112fbf5`, ELF `b602c89a87694d3b54164d88754292102f7880f4f84ebdb3b691c0faeb42c77f`; archived service hashes, source map and post-run identity match: True.
* n4_ap: ISO `1f1b0aeb8ef29bf9d01b95ae03ab998bf57eb5acd280c9ccc0041970cfd28b8b`, ELF `af504e9829eb36902e45bfb175be367c961bfdf2c8605cfef3686357d1ddc8f1`; archived service hashes, source map and post-run identity match: True.
* Ordinary production image: ISO `2ed99964ee5745a4d370eec79b440c8a11698d9391c1bd54f241e07104670813`, archived ELF/services/source match: True; session-test markers absent: True.

Ordinary native two-CPU boot: **PASS**, PMM/VMM/SMP and real SunlightD/UAC ready, session-test markers absent; raw log `target/fastlane-f11b0e3t-evidence/normal-boot.serial.log`.

Final selected sources unchanged: **True**. Historical files preserved: **453/453**. Attempt inventory errors: **0**.

Final-source native classifications: `{'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT': 4, 'NOT_RUN': 1, 'NOT_FORCED': 1, 'FORCED_AND_PASSED': 26}`. N3M new attempts: **6**; its original causal classification remains unexplained. Compatibility failures: **2**. See individual manifests for CPU coverage and nested lifecycle/network results.
