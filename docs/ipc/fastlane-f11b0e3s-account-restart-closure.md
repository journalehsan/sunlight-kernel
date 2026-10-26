# FastLane F1.1B-0E3S: Account retirement and restart acceptance

2026-10-26. **Session Authority: PARTIAL. N4's required broker ordering remains
unverified. Restricted SHM and production FastLane remain disabled.**

Evidence index: `target/fastlane-f11b0e3s-evidence/validation-summary.json`.
Individual attempts retain their commands, serial logs, classifications,
checkpoints, source maps, image hashes, failures and cleanup observations.

## 1. Executive summary

This phase changes validation tooling only. It does not change kernel or service
authorization, introduce authority tables, or enable Account database mutations.
The frozen-image results are 27 FORCED_AND_PASSED executions, one
FORCED_AND_FAILED N3M attempt and one NOT_FORCED N4 attempt. These are execution
counts, not 27 distinct security requirements. N5 and N6 pass at 2/4/12 CPUs;
replacement fresh login also passes at each of those CPU counts.
The existing native lifecycle fixture successfully exercises authenticated
SunlightD replacement, cancellation of a genuine pending Account call, and
replacement rejection of the old session. A fresh login under replacement B
also establishes a distinct broker session and exercises one-use consumption.

The old parser incorrectly accepted N4's userspace policy pause as the required
broker precommit race. Its apparent PASS has been superseded, without editing
the earlier raw evidence. The stricter final replay is **NOT_FORCED**: A never
enters the broker grant syscall, and B's later rejection is not A resuming.
This is an acceptance/fixture defect, not an observed production authorization
defect. The full combined scenario remains blocked on that missing ordering.

## 2. Starting E3R evidence

Read `docs/ipc/fastlane-f11b0e3r-execution-closure.md`, its evidence index,
`fastlane-f11b0e3-native-session-races.md`,
`fastlane-f11b0e2-broker-session-protocol.md`, and
`fastlane-f11b0c-production-grants.md`. E3R's eleven final-image executions cover
N1/N2, both two-CPU N3 outcomes, controlled N7 and both overlapping N7 outcomes.
They do not cover N4–N6, 4/12-CPU session races or final native compatibility.
Their host totals are historical: 131 kernel, 258 library/service and 19
SunlightD runtime tests. New executions are listed separately below.

The starting checkout contains widespread pre-existing version-manifest and
Cargo.lock changes. Those inputs differ from E3R's image and were preserved;
the old image was not relabeled as current. `environment-audit.json` inventories
57 earlier E3/E3R summary/serial files for a final preservation check.

The requested E3S evidence root also already contained an October 24 build,
one empty native attempt and two unsuccessful N4 probes (pre-checkpoint QEMU
exit and guest failure). They remain historical, with their original summaries
and logs retained. The new aggregate separates them from October 26 execution
counts and final-source acceptance. They are not additional current runs or
valid N4 coverage.

## 3. Real Account retirement architecture

Account is implemented inside UAC:
`services/sunlight-uac/src/bin/{uac_service.rs,account_service/mod.rs}`.
`services/sunlightd/src/main.rs` declares the eager `uac_service.service` with
embedded `/sbin/uac_service`, root identity and existing restricted service
capabilities. `kernel/src/process/spawn.rs::embedded_bytes_for_path` selects
embedded executable bytes. In `kernel/src/arch/x86_64/syscall.rs::sys_spawn`,
only the authenticated active service manager launching that path installs
`trusted_auth_broker`. A basename, nameserver lookup or UID-zero caller does
not confer that role.

The kernel authenticates a service as `(pid, foreground.incarnation,
address_space.identity().generation)`. `capability/lifecycle.rs` checks these
identities and live roles, plus the exact recipient incarnation and IPC call
generation, while retaining SCHEDULER through CAP_BROKER mutation. Account's
snapshot/password/policy checks precede this final authorization boundary.

## 4. Trusted issuer lifecycle contract

| Transition | Actual production serialization / authoritative point |
|---|---|
| Termination request | SunlightD `stop_service` sends an ordinary signal. Sending or accepting it is not broker session retirement. |
| Process authority becomes unusable | `finish_current_process` or `terminate_process_by_pid` marks Finished/cleanup-pending under SCHEDULER. `Process::is_exiting` and `grant_requester_for_role` then deny issuance, even if a stored role bit has not been cleared. |
| Execution ends | Self-exit drops current ownership; remote termination asks the owning CPU to deschedule. Reaping requires no executing core, pinned stack, ready-queue ownership or active VM CPU. |
| Session/grant retirement | `close_owned_ipc_endpoints` or safe reaping calls `CapabilityBroker::revoke_endpoints_owned_by` → `retire_recipient_grants` → `retire_sessions_for`. Row removal and associated Account grant removal under CAP_BROKER, with SCHEDULER retained, are authoritative. |
| Pending reply cancellation | Endpoint removal gathers actual `IpcCallId`s; `ipc::finish_peer_closed_calls` matches recipient PID/incarnation, endpoint and request generation, then `finish_call(PeerClosed)` settles cancellation. |
| Old reply contexts destroyed | Safe reaping in `sched/mod.rs::reap_process_resources` clears pending calls, reply targets and deferred contexts before the observed `service-ipc-retired` checkpoint. |
| Replacement authenticated | After confirmed old exit, SunlightD `apply_confirmed_exit`/`spawn_service_at` uses the existing trusted spawn chain. A fresh process/VM receives its own role. Activation is not new session registration or readiness by itself. |
| New session authority | Real password proof delivered to trusted TTY; trusted sessiond calls `register_account_session`. The broker consumes the original issuer-bound proof and inserts a new Suspended row, then the authenticated owner activates it. |
| Exec authority withdrawal | Successful `process/spawn.rs::exec_into_process` retires grants and calls `withdraw_exec_image_roles` before new VM activation. Failed preparation preserves the old image contract. |

Important audit correction: this eager UAC unit uses SunlightD's existing
`libc::kill(pid, signal)` branch. `signal_service` uses generation-qualified
`kill_instance` for **OnDemand** units. The old fixture comment overstates that
distinction. The fixture requests manager-owned restart rather than killing a
PID itself; these executions do not establish numeric-PID reuse safety for eager
service supervision. Kernel grant/session and reply recipient identities still
use incarnation/VM/call generations. No SunlightD redesign was attempted.

Committed Account grants remain session-bound even after delivery; session or
issuer retirement removes unconsumed grants. Consumed grants have already been
removed. Transferable VFS delegation follows its separate existing lifetime
policy; this phase does not convert it into Account session authority.

## 5. N4: retirement before pending broker commitment

The existing stage-zero `race_checkpoint` is after legitimate Account password
and policy checks, but before `api::try_broker_grant` and syscall 151. UAC A
blocks in a real timing IPC. Root-authorized SunlightD restart retires A; the
requester receives real PeerClosed; B rejects a subsequent request naming A's
old session. No grant is published, and final observed session inventory is
zero. This establishes cancellation of a policy-accepted request before broker
entry, with an authenticated service replacement.

It does **not** satisfy N4's requested old-issuer broker entrant/resume/rejection
ordering. `tools/fastlane_f11b0e3_validate.py` now requires the original A's
`boundary-ready` and post-retirement rejection, ordered around the actual
`service-retirement` event. The final attempt is NOT_FORCED with that exact
missing-checkpoint explanation. The diagnostic attempt's old PASS remains in
its original summary and is explicitly superseded in the aggregate.

A naive barrier inside A's running syscall cannot wait for A's safe reaper:
`reap_process_resources` refuses a still-owned/pinned execution stack. Actual
retirement destroys the userspace continuation. No fake service identity,
manual session-table removal, role-withdrawal bypass or scheduler change was
introduced to manufacture the requested ordering. A valid lifecycle fixture or
an explicitly agreed prevention-based N4 acceptance criterion is still needed.

## 6. N5: stale IPC response prevention

The stage-one fixture first commits a legitimate grant through the real broker,
observes one pending Account delivery, and blocks A before the normal reply.
SunlightD retires A. Endpoint closure produces an actual generation-qualified
PeerClosed for the same bound call; retirement removes the pending grant and
session; safe reaping destroys A's reply context before B activation.

This forces the production cancellation boundary. It proves prevention through
endpoint closure and executable/context retirement, rather than execution of
an old server reply after the server is dead. No reply packet was fabricated or
injected. `ipc/mod.rs::deliver_reply_to_target` independently rejects unmatched
recipient incarnation/endpoint/request generations; that branch is covered by
host tests, not claimed as a newly forced late-delivery native branch here.

The parser matches the original issuer-bound request generation to the actual
cancelled call, requires retirement caused by that Account PID, independent B
activation, old-session error 11, service context cleanup and final zero
inventory. Missing real pending delivery or cancellation cannot certify N5.

## 7. N6: real restart and old-session isolation

The original lifecycle fixture establishes A's legitimate broker session and a
pending session-bound grant, uses actual SunlightD restart, and asks B to
authorize with A's old session. B returns broker
`SessionAuthorityUnavailable` (11). Different service incarnation and VM are
required; a different numeric PID alone is insufficient.

The optional `--replacement-relogin` extension then exits the shell and uses
normal TTY password login to register a new session under B. Its N7C continuation
requires an actual B-issued grant, one successful consumption, immediate repeat
denial, logout and both fixture reapers. The parser uses recorded serial slices
from the same guest and B's actual activation line; it does not synthesize an
authentication event. New `(id, epoch)` tuples must differ. The observed new
session may share the same boot-global table epoch: uniqueness is the tuple,
not a requirement that every login increments the epoch.

## 8. Combined lifecycle scenario

**BLOCKED** on N4's missing broker precommit ordering. N5/N6 combine pending
committed delivery, retirement, cancellation, replacement and old-session denial;
the extended N6 adds fresh authority and one-use consumption. These useful
subsets are not relabeled as the full precommit combined scenario. No combined
failure is cancelled by an isolated pass.

## 9. Four-CPU matrix

Execution results and observed CPU identities are in the generated results
section below. A configured CPU count is not the number of race participants.
The existing test-only affinity keeps UAC on CPU 0 and sessiond on CPU 1;
readiness events must demonstrate both contenders before the decisive
transition. The remaining CPUs provide SMP scheduling context.

## 10. Twelve-CPU matrix

Only supported existing fixtures are extended. No scheduler benchmark or large
unrelated guest workload is introduced. Repeated boots are independent bounded
lifecycle cycles, not proof of steady-state multi-cycle allocation stability.
Concurrent multi-session request load and the complete N4-dependent lifecycle
stress remain unverified. Native PID reuse remains NOT_FORCED.

## 11. Final-source N1–N3/N7 replay

Both N3 orders and both overlapping N7 orders require distinct actual CPUs and
both ready events before the first broker transition. Controlled N1/N2 and
N7R/N7C preserve their original outcomes and one-use rejection. Historical E3R
logs remain historical; only new attempts with the frozen source map qualify
as final-source runs.

## 12. Source and image identity

The final fixture build rebuilds init, SunlightD, TTY, UAC, sessiond and VFS
separately with the established service flags, then builds the test kernel.
It archives all seven relevant embedded executables, including the ordinary
race client, verifies their actual offsets in the kernel, and hashes the
kernel, stripped ELF, immutable ISO and selected source files. Each attempt
rechecks the immutable ELF, ISO, embedded archives and selected source map.
The unstripped executable digest is a build-time observation; compatibility
builds reuse its mutable Cargo output path. Immutable ISOs are reused.

Only `tools/fastlane_f11b0e3_validate.py` and
`tools/test_fastlane_f11b0e3.py` change implementation inputs. No production Rust
source changes. Four new parser counterexample tests cover lifecycle call
binding/cleanup, the false N4 acceptance, missing fresh login, and service PID
observations remaining independent of recipient PID-reuse acceptance. Parser
strings are not native fixtures. Source maps are the existing tool's selected
Rust/TOML/linker/Python/shell/assembly/lock/config inputs, not a hermetic hash of
every asset in the repository. Final artifact identities appear below and in
the JSON index.

## 13. Resource and authority accounting

The observed scope includes live broker sessions, registration receipts,
Account grants, pending Account delivery records, mint/consume counts, session
identity bindings, retained session/grant storage and fixture/old-service IPC
contexts. Each accepted teardown requires zero live sessions/receipts/grants/
pending Account deliveries and cleared fixture pending/reply/deferred contexts.
One retirement counter increment and repeated idempotent logout distinguish
actual retirement from duplicate attempts.

An empty table may retain capacity 4 (704 session metadata bytes); grant storage
may retain 576 bytes after its live entries disappear. This reusable capacity
is separate from leaked live metadata. Token history intentionally retains
burned/retired values and its 65,536-entry bound is unchanged.

The global pending-commit gauge, global IPC reply/reference inventory, retained
authentication-proof issuer references and global allocator remain **UNMEASURED**.
Reaper `pending_commits=0` is a serialized local
observation, not a new global gauge. No complete Broker/VFS/UAC reconciliation,
multi-session steady-state drift claim or independent global lifetime acceptance
follows from these fixtures.

## 14. Compatibility regressions

The existing E3 driver runs the established host checks and delegates production,
authority, core, lifecycle and network replays to the E2 validation tool. The
ordinary image additionally requires absence of session-test markers. Detailed
results are generated below. Native TCP/TLS uses local ephemeral certificates
and the existing certificate-verification matrix, not upstream availability.
Production delegation, authority/history, scheduler, pin-release, Doorbell,
VM/TLB/unmap/protection, RecoveryBlocked and TCP/TLS replays pass their recorded
scopes. D1/D2/D3 passes one CPU and fails two CPUs as detailed below; its
four-CPU attempt is NOT_RUN. The network resident gate also runs the opt-in
SHM retirement/32-lifecycle prerequisite on 12 CPUs, with transport still BLOCKED.

The new host execution passes 131 kernel tests, 258 service/library tests,
19 SunlightD runtime tests, 63 focused scheduler tests and 28 parser tests
(14 in the E3 parser module). Media separately records 36 passes and one
missing-fixture failure. These are new executions of the frozen source;
host success is not substituted for a native lifecycle assertion.

The legacy scheduler host tool writes
`target/scheduler-b-evidence/focused-host.log` at its fixed shared path. This
invocation reused that path; the resulting detailed log is also archived inside
the new checks directory as `scheduler-focused-host.raw.log`. Earlier E3/E3R
summary and serial files are separately checked for preservation. No broad
cleanup or destructive Git operation is performed.

## 15. Current-only and baseline failures

Strict Clippy fails with `-D warnings`; workspace formatting differs; media
has 36 passes and the existing missing repository WAV fixture failure; Miri is
not installed. These are actual failures/unavailability, not PASS with caveats.
The unchanged production source and previous reports support baseline attribution;
no old Git state was restored to manufacture a matched build:
**MATCHED BASELINE UNAVAILABLE** for a new old-tree replay.

One diagnostic service build attempted to combine TTY and services owning their
own allocators; Cargo feature unification produced conflicting global allocators.
The validation driver now follows `tools/build.sh` and builds those packages
separately. That retry passed; the original failed build log is preserved.
The original N4 assessor's false PASS is a current-discovered validation defect;
its corrected result is NOT_FORCED. Any additional execution failure is retained
below. A successful D3 replay cannot explain its historical unexplained failure.

One frozen-source two-CPU N3M attempt reached both real contenders, then mint
sequence 15 before revoke sequence 17, but the client subsequently failed at
`session_race_probe.rs:13`, its bounded nameserver lookup of `uac` for the stale
authority check. The driver correctly stopped and retained FORCED_AND_FAILED.
`nameserver_lookup_timeout` returns None for transport failure, timeout or a
non-GRANT reply, so that log does not identify which occurred. There is no
matching explicit kernel lookup-denial message or UAC exit in the captured log.
Host contention is not established as the cause. No timeout increase, disabled
SMP, weakened authorization or production repair is used to hide this attempt.
Any later success remains a separate execution and cannot explain this failure.

The new D1/D2/D3 compatibility command passes at one CPU and fails at two CPUs
in `services/sunlight-startup-fixture/src/d3_driver.rs:164`: warm ACTIVATE of
`d3-parent` returns ERROR (53759), expected OK (53632), after a retained
generation reports `failure=6` (STALE). The command stops before its four-CPU
attempt. This is a new recorded failure in the historically uncertain D3 area,
not proof of its root cause or of a regression caused by this tooling-only
change. Its source is unchanged; no weakening or unrelated repair is made.

## 16. Remaining Session Authority limitations

N4's required original-issuer precommit/resume ordering, the full combined
scenario, and concurrent multi-session/steady-state lifecycle accounting remain
unverified. Root-authenticated group-operation fixtures do not prove ordinary
UID-1000 Account group grant issuance. Native UAC/VFS delegation is a separate
policy/transport compatibility class. No new password/profile/group publication
authorization is asserted; fail-closed mutation behavior remains intact.

Service-incarnation replacement does not establish recipient numeric PID reuse.
The eager UAC manager stop branch's PID-based signaling is not upgraded into
generation-qualified supervision acceptance by these tests. RecoveryBlocked
has its own replay and remains a separate authority boundary.

## 17. Gate decisions

| Gate | Decision |
|---|---|
| A: Scheduler | VERIFIED NATIVE; current scheduler/pin-release native replay passes |
| B: Native Doorbell | PARTIAL; historical and current D3 uncertainty remains independent |
| Session Authority | PARTIAL; N4 and complete lifecycle acceptance remain unresolved |
| Token-History | PARTIAL |
| F1.1B-0 | BLOCKED |
| C: production FastLane | BLOCKED |

No Restricted SHM, public userspace Doorbell API, production FastLane transport,
insecure mapping, arbitrary trusted registration or test-issued authority is
enabled. No commit or push is performed.

## 18. F1.1B-0F handoff

E3S is not acceptance-complete: resolve the N4 fixture/criterion and its combined
and resource acceptance before treating Session Authority as VERIFIED NATIVE.
Do not silently move these E3S obligations into a completed gate.

| Unresolved invariant | Actual implementation / evidence | Smallest remaining acceptance work |
|---|---|---|
| Recipient PID reuse cannot adopt old grants/replies | `kernel/src/process/pid.rs`, `process/foreground.rs`, `process/address_space.rs`, `capability/delivery.rs`, `ipc/mod.rs`; generation checks and host tests exist, native numeric reuse NOT_FORCED | Observe real numeric reuse with distinct incarnation/VM and force an old bound grant/reply rejection through production paths; preserve the real recipient identities. |
| Old Account precommit cannot publish after retirement | `capability/lifecycle.rs::account_grant`, `session_race.rs`, `sched/mod.rs::reap_process_resources`, UAC `race_checkpoint`; policy-stage cancellation now native, required broker interleaving absent | Select a reachable existing production boundary; demonstrate the original entrant and authoritative retirement with safe stack lifetime, or agree an explicit prevention criterion. Then rerun N4 and the combined lifecycle case on relevant SMP configurations. |
| All remaining grant/delivery lifetimes reconcile | `capability/{mod,sessions,vfs_grants,delivery}.rs`, `ipc/mod.rs`, `process/spawn.rs`; session-subset native cleanup and compatibility probes | Inventory actual issuer/recipient/session/call ownership, pending/delivered/consumed/retired grants and deferred IPC across the smallest missing lifecycle interleavings; no isolated-pass substitution for combined failure. |
| Global Broker/VFS/UAC resources remain bounded | Existing bounded tables/session receipts, metadata snapshots; global pending/reply/allocator gauges unmeasured | Reconcile live and reusable retained capacity across repeated same-guest multi-session cycles, with exact pending references and allocator scope; explain all residual inventory. |
| Token-history lifetime/capacity availability | `capability/token_history.rs`, `history_boundary.rs`; 65,536 retained-token bound, host/native isolated exhaustion evidence | Establish production lifetime exhaustion/error-delivery policy and capacity acceptance without clearing retained history or weakening collision rejection. |
| Independent production prerequisites | Doorbell D3 history, Restricted SHM/revocation/transport gates | Resolve those independently; Account restart verification grants no authorization to enable them. |

F1.1B-0F work is not started here.

The current `process/pid.rs::next_pid` allocates checked max-existing-PID + 1,
including retained reaped slots. Reusing a process-table slot is therefore not
numeric PID reuse. A real reuse acceptance path must first account for that
allocator contract; fabricated PID assignment would not close the gate.

<!-- EXECUTION_RESULTS -->

## Recorded execution results

Final-source native executions: **27 FORCED_AND_PASSED, 1 FORCED_AND_FAILED, 1 NOT_FORCED**. 12 overlapping broker races passed; 13 overlaps were observed including the failed N3M teardown attempt. 3 replacement fresh-login replays passed.

This section is generated from the retained per-attempt summaries. Failed attempts remain failures; historical diagnostic runs are excluded from the final-source matrix.

| Case | 1 CPU | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|---|
| N1 | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | NOT_RUN | NOT_RUN |
| N2 | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | NOT_RUN | NOT_RUN |
| N3M | NOT_RUN | FORCED_AND_FAILED (1 pass / 1 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) |
| N3R | NOT_RUN | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) |
| N4 | NOT_RUN | NOT_FORCED (0 pass / 0 fail / 1 not forced) | BLOCKED | BLOCKED |
| N5 | NOT_RUN | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) |
| N6 | NOT_RUN | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (2 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (2 pass / 0 fail / 0 not forced) |
| N7R | NOT_RUN | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | NOT_RUN | NOT_RUN |
| N7C | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | NOT_RUN | NOT_RUN |
| N7OC | NOT_RUN | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) |
| N7OR | NOT_RUN | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) | FORCED_AND_PASSED (1 pass / 0 fail / 0 not forced) |

N4 on 4/12 CPUs is blocked by the missing fixture ordering. Cross-CPU 1-CPU cases were not invoked. N6 entries include the extended fresh-login executions when recorded.

| Lifecycle attempt | CPUs | Account PID/incarnation/VM | Registered session tuples | Final sessions/receipts/grants/pending delivery |
|---|---|---|---|---|
| native-20261026T083005.736881Z/N6 | 2 | 20/32/51 -> 42/54/93 | (1, 1), (2, 1) | 0/0/0/0 |
| native-20261026T083315.328754Z/N4 | 2 | 19/31/49 -> 42/54/93 | (1, 1) | 0/0/0/0 |
| native-20261026T084315.919804Z/N5 | 2 | 18/30/47 -> 42/54/93 | (1, 1) | 0/0/0/0 |
| native-20261026T084821.060412Z/N5 | 4 | 23/35/57 -> 42/54/93 | (1, 1) | 0/0/0/0 |
| native-20261026T084821.060412Z/N6 | 4 | 24/36/59 -> 42/54/93 | (1, 1) | 0/0/0/0 |
| native-20261026T084821.060412Z/N5 | 12 | 24/36/59 -> 42/54/93 | (1, 1) | 0/0/0/0 |
| native-20261026T084821.060412Z/N6 | 12 | 24/36/59 -> 42/54/93 | (1, 1) | 0/0/0/0 |
| native-20261026T085758.792874Z/N6 | 4 | 24/36/59 -> 42/54/93 | (1, 1), (2, 1) | 0/0/0/0 |
| native-20261026T085758.792874Z/N6 | 12 | 24/36/59 -> 42/54/93 | (1, 1), (2, 1) | 0/0/0/0 |

| Host check | Outcome |
|---|---|
| kernel-host | PASS (exit 0) |
| services-host | PASS (exit 0) |
| sunlightd-runtime | PASS (exit 0) |
| scheduler-host | PASS (exit 0) |
| parsers | PASS (exit 0) |
| strict-clippy | FAIL (exit 101) |
| media-host | FAIL (exit 101) |
| workspace-format | FAIL (exit 1) |
| diff-check | PASS (exit 0) |
| miri | FAIL (exit 1) |

| Compatibility section | Checks | Native assertions |
|---|---|---|
| authority | 4/4 commands passed | entropy-unavailable: PASS, authority-1cpu: PASS, authority-2cpu: PASS, authority-4cpu: PASS, authority-12cpu: PASS |
| core | 19/19 commands passed | pin-release-1cpu: PASS, pin-release-2cpu: PASS, pin-release-4cpu: PASS, pin-release-12cpu: PASS, doorbell-1cpu: PASS, doorbell-2cpu: PASS, doorbell-4cpu: PASS, doorbell-12cpu: PASS, scheduler-1cpu: PASS, scheduler-2cpu: PASS, scheduler-4cpu: PASS, scheduler-12cpu: PASS, vm-tlb: PASS, vm-unmap: PASS, vm-protect: PASS |
| lifecycle | 11/12 commands passed |  |
| network | 8/8 commands passed | resident-12cpu: PASS |
| production | 9/9 commands passed | production-1cpu: PASS, production-2cpu: PASS, production-4cpu: PASS, production-12cpu: PASS |

Compatibility command logs also include nested gates such as D1/D2/D3, RecoveryBlocked and TCP/TLS; their wrapper exit status must be read alongside the raw nested results.

| Frozen artifact | SHA-256 |
|---|---|
| Kernel executable | `6591e6efed2b466bd93fbb81efeae246ca04cd5699dab55e6619f1830f298823` |
| Stripped ELF | `0943b0107058d808c484d863659bf476df1614730ec4ebdbc6ef6160d6b4f800` |
| ISO | `696c1fb27f4c2a4a27614e4bb6a0d090efe1ab898f93ca4107d10d6fc84a6d2c` |
| Embedded sunlight-init | `d31e6bb3fe21c42a98cc2a76064007b2f94752df7e190887be486d83f0c87c7e` |
| Embedded sunlightd | `4ad1fcaff41fc595e17d49e72a3bb5430a9891cd22d9f3d7fc8a1ed6aa0e3c30` |
| Embedded sunlight-tty-server | `0da0ae52ffcd22338451e4abe944068236c26c43ce005bb7474afcd5b76a5481` |
| Embedded uac_service | `75f53791995beec0591961854cad0e1e5b5428cc5c4c76be43acb6a41ba2cdec` |
| Embedded sunlight-sessiond | `ad0869ab82f7e6e629faf322eea4007f0d7dfb91fa106fcaae56c8e12a19cf21` |
| Embedded sunlight-vfs-server | `6f65822d2417499288a9f8542bd24e95a021e16d5f399d7489b0ffdf15e5f2aa` |
| Embedded session-race-probe | `e71f74e83a0b5d56e9f39ad4159f72e449a6b76b50ad9b8853bd2c3ce655cc69` |

Final source/artifact identity: `{"elf_matches": true, "iso_matches": true, "services_match": true, "source_matches": true}`.
Earlier E3/E3R evidence inventory: 57/57 hashes unchanged.
Disk free at consolidation: 37.66 GiB; no cleanup performed.

The JSON index retains each authoritative transition, actual CPU participants, service incarnation/VM, session tuple, grant result, cleanup snapshot and failed attempt. Global resource drift remains unmeasured.
