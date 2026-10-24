# FastLane F1.1B-0E3: authenticated native session races

**2026-10-24 continuation:** [F1.1B-0E3R execution closure](fastlane-f11b0e3r-execution-closure.md)
records new execution, separately from the E3 history below. E3R ran 11
successful final-snapshot native executions: N1/N2 on 1/2 CPUs, both N3
orders on 2 CPUs, and N7 controlled and overlapping orders. N4–N6, the
4/12-CPU matrix, and native compatibility replay remain NOT_RUN after the
user again requested continuing without tests. Session Authority remains
PARTIAL. E3's historical final-source NOT_RUN classifications below have
not been rewritten as E3R results.

2026-10-24. **Session Authority: PARTIAL. E3 acceptance is incomplete.**
Earlier diagnostic images produced authenticated 2-CPU ordering evidence.
The final fixture image built successfully, but its native matrix and
compatibility replay are **NOT_RUN**. The user subsequently requested continuing
without tests; no further tests or probes were executed. This report preserves
earlier results without treating different source snapshots as final acceptance.

Gate A remains VERIFIED NATIVE; Gate B and Token-History remain PARTIAL;
F1.1B-0 and Gate C remain BLOCKED. Grant mint and consume remain
RESTORED_PARTIAL. Password, profile, and group mutation remain FAIL_CLOSED.
The broker authorization algorithm was not redesigned. Restricted SHM and
production FastLane remain outside this change.

## E3.1 diagnosis before fixture changes

E2 did not attempt authenticated N1–N7 and then time out. Its
`tools/fastlane_f11b0e2_validate.py::assess_session` explicitly assigns all seven
NOT_FORCED. `capability/sessions.rs::native_sequential_witness` constructs a
private broker with synthetic identities. The production-grant image instead
runs ordinary UID-1000 VFS delegation clients. Neither calls the trusted
TTY-to-sessiond registration path. E2's native race workers kill delegation
recipients, not Account. E2 also reverses the N5/N6 descriptions in its table;
E3 uses N5 stale reply and N6 stale session as requested.

| Case | Last reached checkpoint | Missing checkpoint | Root cause | Minimal required correction |
|---|---|---|---|---|
| N1 | Isolated revoke-before-mint | Authenticated pending Account request and authoritative logout before commit | No native authenticated session fixture or rendezvous | Real TTY login, Account request, bounded rendezvous outside locks, sessiond logout |
| N2 | Isolated mint-before-revoke | Live authenticated Account commitment before logout | Same fixture gap | Observe real insertion then sessiond logout and cancelled consumption |
| N3 | Host broker mutex contention | Separate native CPU admission to decisive serialization | No native contention hook; production SCHEDULER precedes CAP_BROKER | Both syscall entrants outside SCHEDULER, controlled release and authoritative transitions |
| N4 | Isolated owner retirement | Real Account/UAC retirement while request pending | Delegation fixture kills recipients, never UAC | Actual SunlightD lifecycle with pending authenticated operation |
| N5 | Delegation recipient stale reply excluded | Old UAC reply across authenticated replacement | No Account replacement/pending-response fixture | Preserve actual call identity through retirement; observe production cancellation/delivery |
| N6 | Isolated new table epoch | Old live session after new UAC activation | Table replacement is not a service restart | Real UAC replacement; old reference denied; separately verify new login |
| N7 | Host/isolated consume versus revoke | Authenticated native one-use consumption and logout | No registered native session; production mutation continuations refuse before consume | Dedicated nonpublishing, opt-in orchestration after existing policy, retaining publication refusal |

E2 logs explicitly say `scope=isolated-production-broker authentication=NOT_TESTED`.
They cannot establish Account activation, session registration, preliminary
validation, race hook reachability, remote progress, or commit order. There are
no E2 N1–N7 barriers to diagnose as stuck. Missing markers are not authorization
successes. No evidence presently identifies a stale embedded binary, parser
timeout, or production authorization defect as the cause of this gap.

## Production authentication and boundaries

Account is part of `services/sunlight-uac/src/bin/uac_service.rs`. The existing
kernel init → SunlightD launch chain sets `trusted_auth_broker` for the embedded
UAC image in `sys_spawn`. Boot creates the real TTY with its trusted role.
Embedded sessiond receives its existing role in `process/spawn.rs`. Test clients
must receive neither role. Registration authenticates the pending TTY caller,
delivered password proof, original live UAC, and live sessiond in
`capability/lifecycle.rs::register_account_session`.

Sessiond `create_session` registers suspended authority, prepares the real
desktop, and its normal supervision loop activates Running/Degraded sessions.
`session_action(Logout)` synchronously revokes before stop/reply. Mint, consume,
revoke, and process retirement serialize through SCHEDULER → CAP_BROKER.
Hooks must never wait while either guard is held. Final insertion/removal under
those guards is authoritative; entry/precommit markers alone prove neither.

Group AUTHORIZE authenticates caller credentials, active session, administrator
policy, database revision, group policy, and password before broker mint.
Password/profile/group publication remains explicitly unavailable. VFS
delegation uses a separate ownership class and is not a session-bound Account
grant. No VFS replay is substituted for N1–N7.

The E2 production grant fixture did launch and authenticate UAC for VFS
delegation. That does not imply a registered Account session. E2's summaries
do not show failed authenticated race attempts, so NOT_FORCED cannot be
attributed to an observed timeout, stale executable, inactive role, or bad
marker order. The missing authenticated fixture and production timing hooks
were identified before adding the E3 fixture.

## Minimal authenticated fixture

The opt-in build combines kernel `native_session_race_test` with UAC
`session-race-test`. Neither is a default feature. The ordinary service and
kernel dispatches retain their normal authorization and publication refusal.
The fixture boots the actual kernel, init, SunlightD, TTY, sessiond, UAC and VFS.
Normal login and TTY password revalidation establish sessiond's broker session;
the client never inserts a session or sets a trusted role.

`session-race-probe` is an ordinary userspace process with no trusted service
role. These group-administration probes use a real authenticated root session
because the existing policy requires UID 0. They do not establish unprivileged
user coverage for unrelated operations. Snapshot, policy and password checks
use the existing Account request path and VFS policy files. Kernel delivery
binding comes from the actual pending IPC caller's PID, incarnation, VM identity
and call generation, rather than fields supplied by the fixture.

The parent delegates a SEND_ONLY bootstrap endpoint to its ordinary controller
through spawn, and authenticates the returned IPC badge. The controller passes
a public timing endpoint back through normal IPC. It receives no Account or
sessiond role. Account accepts this timing endpoint only after the real request's
policy/password checks. Reserved fixture group targets select timing cases;
they do not create groups. Normal mutation publication still refuses.

Logout goes through real sessiond IPC. Restart goes through the existing
root-authorized `sunlightctl restart uac_service` and SunlightD lifecycle
capability. No test kill syscall or trust allowlist was added. Test-only affinity
places UAC on CPU 0 and sessiond/controller on CPU 1 when SMP is available;
the scheduler algorithm is unchanged.

N7 uses a test-only nonpublishing continuation through the same group-policy
validation and actual broker consumption. It immediately attempts consumption
again and requires CapabilityRejected. This verifies the broker primitive in
the authenticated service path; it does not restore production mutation
publication or prove its future VFS transaction.

## Synchronization and authoritative observations

`kernel/src/capability/session_race.rs` contains timing and observation only.
It has no broker, role assignment, session creation or authorization decision.

| Checkpoint | Actual production location and meaning |
|---|---|
| account-activated | Existing trusted SunlightD spawn branch after setting the real UAC role in `arch/x86_64/syscall.rs` |
| login-proof-committed / registered | Authorized password proof and TTY/sessiond registration in `capability/lifecycle.rs` |
| UAC policy rendezvous | `account_service/mod.rs` after caller, session, group policy, revision and password validation |
| boundary-ready | N3 syscall entrants before decisive SCHEDULER acquisition; brief role snapshot is released before waiting |
| mint-committed / consume-committed | After successful boot-global broker mutation while lifecycle serialization remains held |
| transition-committed result=3 | After actual authoritative REVOKE; idempotent missing-row requests are not additional commits |
| service-retirement | `sessions.rs::retire_sessions_for`, after removing the actual issuer's session and its grants |
| peer-closed | Actual pending generation-qualified call completion during endpoint closure |
| service-ipc-retired | Actual UAC reaper after clearing pending call, executable reply target and deferred targets |
| fixture-ipc-cleaned | Final fixture reaper after IPC cleanup; added to the final image, not executed |

N1/N2 and N4–N7 use nested synchronous timing IPC at the safe service boundary.
Its timeout is 5 seconds. N3 observes the real role-checked UAC timing call,
arms only once, and admits actual Account and sessiond syscall entrants on
separate CPUs. Both entrants must arrive before the chosen winner proceeds.
The loser proceeds only after the winner's actual state mutation. It then
uses the unchanged SCHEDULER → CAP_BROKER serialization.

No barrier waits while holding those locks. The native loop has both a
50,000,000-iteration ceiling and a local calibrated TSC budget of 3 seconds.
Local TSC is used only for the timeout; sequence numbers under serialization
establish ordering. This does not measure cross-CPU clock drift or lock-wait
duration. A boundary timeout is a failure to force the intended conflict,
never an authorization success. Sequential N1/N2 or N7 evidence is not
relabeled cross-CPU contention merely because QEMU has two CPUs.

Regular identity events are limited to the first 128 observations. The grant
observation map has 32 slots. `grant_fingerprint=boot-local-N` is an opaque
ordinal within that boot, not a cryptographic fingerprint or a reusable
capability. Earlier N1/N2/N7 images predate that observation and have no grant
fingerprint. No password, raw bearer token or session credential is logged.

## Individual native evidence and its limits

All sequences below are from preserved **earlier diagnostic snapshots**, on
2 CPUs. Every fixture registered session 1, epoch 1 in a fresh boot. These
numeric values are not evidence of aliasing across a service restart. The
run directories contain exact QEMU commands, source maps, image/service
hashes, raw serial logs, original classifications and failures.

| Case | Earlier recorded classification | Real observed order | Final-source classification |
|---|---|---|---|
| N1 | FORCED_AND_PASSED | REVOKE seq 8 CPU 0 → mint rejected seq 14 CPU 1, error 11; no grant committed | NOT_RUN |
| N2 | FORCED_AND_PASSED | Mint seq 9 CPU 0 → REVOKE seq 11 CPU 0; session grants and receipt removed | NOT_RUN |
| N3M | FORCED_AND_PASSED | Revoker ready 10 CPU 1, issuer ready 12 CPU 0 → mint 14 CPU 0 → REVOKE 16 CPU 1 | NOT_RUN |
| N3R | FORCED_AND_PASSED | Revoker ready 10 CPU 1, issuer ready 12 CPU 0 → REVOKE 13 CPU 1 → mint rejected 15 CPU 0, error 11 | NOT_RUN |
| N4 | NOT_FORCED in original assessor | Retirement 25 CPU 0 → pending call PeerClosed 26 → old IPC context cleared 27 → replacement activation 28 → old-session mint rejected 31 | NOT_RUN |
| N5 | NOT_FORCED in original assessor | Mint 27 → retirement 29 → pending reply cancelled 30 → old context cleared 31 → replacement 32 → old-session rejection 35 | NOT_RUN |
| N6 | NOT_FORCED in original assessor | Mint 27 → retirement 29 → old call cancelled 30 → context cleared 31 → replacement 32 → old-session rejection 35 | NOT_RUN |
| N7R | FORCED_AND_PASSED | Mint 9 CPU 0 → REVOKE 11 CPU 1 → consumption rejected 17 CPU 0, error 11 | NOT_RUN |
| N7C | FORCED_AND_PASSED | Mint 9 CPU 1 → consume 12 CPU 1 → duplicate denied 16 CPU 1, error 8 → REVOKE 18 CPU 1 | NOT_RUN |

Error 11 is SessionAuthorityUnavailable; error 8 is CapabilityRejected.
N1/N2/N7 are in `native-20261024T135311.993834Z`, using image
`build-20261024T135125.832316Z`. Their Account identities respectively were
PID/incarnation/VM 18/30/47, 22/34/55, 19/31/49 and 22/34/55.
This older image predates final cleanup assertions, grant observations and
additional N2 stale-authority assertions. Its original passes are retained,
not expanded to cover those later assertions.

N3 successes are in `native-20261024T141034.207493Z`, image
`build-20261024T141001.594697Z`. N3M's issuer was 18/30/47 and sessiond
26/38/63; N3R's issuer was 19/31/49 and sessiond 28/40/67. Both genuine
entrants reached the boundary on separate CPUs before the first mutation.
Only the chosen valid serialization order committed. Each reached a zero
session/grant/delivery subset. The final image additionally suppresses repeated
readiness observations for idempotent logout and requires both fixture reaps;
those additions have not been run.

N4–N6 are in `native-20261024T140437.092889Z`, image
`build-20261024T140311.037308Z`. The original assessor did not yet recognize
service replacement acceptance and explicitly returned NOT_FORCED. An earlier
post-run `diagnostic-reassessment.json` recognized their detailed lifecycle
orders as FORCED_AND_PASSED using a different parser snapshot. That is
diagnostic interpretation, not a rerun or replacement of the original result.
The final parser adds fixture cleanup requirements absent from these logs.

For N4 the old Account was 24/36/59 and replacement 42/54/93. It was held
after real preliminary authorization and before broker mint. SunlightD
retirement cancelled that pending execution; no old grant committed. The
replacement's fresh request carrying the old session reached the real broker
and was denied with error 11. No synthetic old-service continuation was run.

For N5 the old Account was 18/30/47; for N6 it was 19/31/49. Both replacements
were 42/54/93. A legitimate mint existed before retirement, identified as
`boot-local-1`, and the broker recorded one undelivered Account reply.
The waiting recipient was PID 39, incarnation 51, request generation 4;
PeerClosed completed its real call on endpoint 14 for N5 and 16 for N6.
Old Account reply targets and deferred targets were cleared before replacement
activation. Each replacement independently obtained its trusted role through
the existing launch policy, then denied the old session in the broker.

Early generic PeerClosed instrumentation also printed synthetic IPC boot-test
recipients with incarnation 0. Those events are preserved but excluded from
the lifecycle evidence above. The final hook filters actual fixture recipients.

## Restart, stale replies and session identity

The production IPC mechanism explains why a stale A response was cancelled
rather than delivered after B activation. `ipc::deliver_reply_to_target`
validates the recipient incarnation and pending call identity. Endpoint closure
uses `finish_peer_closed_calls` to finish the actual pending call once. The
reaper clears A's executable reply/deferred context, and SunlightD waits for
retirement before activating B. N5's earlier raw evidence observes those
mechanisms and the waiting client's typed PeerClosed result. It does not
inject an impossible reply or claim that a stale A reply reached B.

`capability/sessions.rs` binds authority to the original Account identity.
Checked boot-global session IDs/epochs are not reset by UAC restart. N6's
old reference was denied after distinct B authentication. Fresh session
registration under B, actual native epoch exhaustion, and actual numeric PID
reuse remain unverified here. `process/pid.rs::next_pid` uses checked monotonic
allocation over retained process IDs; ordinary restart does not force reuse.
Host arithmetic checks are not native PID-reuse evidence.

## Session-bound consumption and grant lifetime

N7R observes authoritative logout removing the session-bound grant before a
real consume attempt reaches the broker. N7C observes one consume commit and
an immediate same-grant duplicate rejection before logout, followed by further
denial. Neither order publishes a password/profile/group change. Overlapping
N7 entrants at the kernel serialization boundary, beyond these controlled
service-boundary orders, have not been forced.

N2's session grant is removed on revocation under its declared session-bound
policy. This does not change the independent lifetime of VFS delegations or
other ownership classes. The final fixture also checks a further stale mint
and retired-grant consumption for N2/N3M; those assertions are NOT_RUN.

## CPU matrix

| CPUs | Required scope | Earlier E3 diagnostic coverage | Final-source result |
|---|---|---|---|
| 1 | Sequential mint/logout, consumption, restart, teardown | NOT_RUN | NOT_RUN |
| 2 | Cross-CPU N3, retirement and all controlled case orderings | Earlier results above; N4–N6 originally NOT_FORCED | NOT_RUN |
| 4 | Concurrent sessions, stale delivery and cancellation | NOT_RUN | NOT_RUN |
| 12 | Simultaneous valid sessions and repeated lifecycle with deterministic checkpoints | NOT_RUN; no E3 stress claim | NOT_RUN |

N3 requires at least 2 CPUs and is NOT_RUN at 1 CPU. Booting any CPU count does
not force a case. The final-source matrix has no observed CPU, Account identity,
epoch, grant fingerprint, delivery result or terminal inventory; those fields
are null in the aggregate rather than copied from earlier images.

## Resource inventory and failed attempts

Successful earlier controlled cases ended with sessions=0, receipts=0,
account_grants=0 and pending_account_deliveries=0. Retained session table
capacity was 4. N4–N6 recorded one owner-retired session rather than one
logout. N3M recorded one mint and zero consumes, N3R zero/zero; N5/N6 one/zero.
Observed token history at their terminal snapshots ranged from 106 to 109
of the unchanged 65,536-entry limit. Older N1/N2/N7 snapshots do not measure
the mint/consume/history counters; their absent fields remain unmeasured.

This is **session subset accounting**, not global Broker/VFS/UAC reconciliation.
Pending account deliveries do not enumerate all IPC deliveries. There is no
global pending-commit gauge or allocator/frame-drift measurement. The final
reaper's `pending_commits=0 snapshot=serialized` describes the fixture at its
serialized teardown point, not all concurrent processes. Final fixture endpoint
and reference cleanup observations have not executed.

All original failures remain available:

| Attempt | Actual obstacle | Correction / retained limitation |
|---|---|---|
| Initial diagnostic launch | Sandbox denied QEMU socket creation before boot | Approved native launch; preserved command and qemu.log; no authorization result |
| First service build | Incompatible `-nostartfiles` link flag | Existing native linker flags used |
| Second service build | Case u32 versus IPC word u64 | Explicit type conversion; original compiler log retained |
| N1 13:35 run | Protected `/bin` placeholder rejected; fixture never launched; driver timed out | Writable `/tmp` launch alias; no filesystem authorization relaxation |
| N1 13:39 run | 1 GiB guest failed child ELF load with NoMemory | Provisioned 2 GiB; PMM/loader algorithms unchanged |
| Native 13:44 driver | Incorrect guessed image-manifest path; FileNotFound before QEMU | Use actual recorded build manifest; no native case attempted |
| N1 13:47 run | Controller's arbitrary nameserver lookup denied | Existing public endpoint delegation; no lookup allowlist widening |
| 13:59 service / 14:00 kernel builds | IpcError/IpcCallError and kernel/userspace IpcMsg type mismatches | Correct types; failed builds retained |
| N3 14:04 runs | Only issuer reached boundary; controller tried dependent lookup after releasing issuer; boundary timed out | Resolve session endpoint first; test-only controller CPU affinity; no timeout increase |
| First parser command | Wrong unittest namespace invocation | Correct discover invocation; parser-only tests subsequently passed |

The failed N3 runs ended with one session, receipt, Account grant and pending
delivery in their last snapshot. The guest was torn down after failure; these
are **unreconciled attempts**, not zero-drift passes. Timeout classification is
retained separately from authorization rejection. A QEMU process exit of 0
alone is not a case pass.

The 1 GiB boot exposed roughly 230 MiB PMM availability from a 493 MiB pool
with a roughly 263 MiB kernel image; the 2 GiB fixture exposed roughly 1,254 MiB
from 1,516 MiB. These are provisioning observations, not allocator reconciliation.
No production authorization defect was established from these harness failures.

## Code changes and production safety

The changes add feature definitions, the observation module, gated hooks in
the existing broker/syscall/IPC/reaper paths, authenticated UAC timing and
nonpublishing consumption continuations, the ordinary native fixture binary,
and an evidence driver/parser. Actual commit, revocation, issuer authentication,
one-use consumption and trusted lifecycle decisions retain their existing path.
No independent authorization state machine was added.

The driver hashes sources and actual embedded bytes, uses per-attempt images,
preserves failures, and checks role/session/incarnation/order/cleanup markers.
Missing boundaries remain NOT_FORCED, observed invariant failures are
FORCED_AND_FAILED, and unexecuted work is NOT_RUN. Parser counterexample tests
are parser checks only. The final driver waits for the ordinary shell return
and both fixture reaps before acceptance; this stronger driver is unexecuted.

The shared release UAC staging file was restored to the preserved ordinary
E2 executable after fixture building. Its SHA-256 is
`b0ec7bb870fb1758cf90be25ad80df485fcb6af8622c016e6798b0a6e4a7df6d`.
This restoration is recorded in `ordinary-staging-restoration.json`. It is
not a final-source production rebuild. Test images and embedded test services
remain under the E3 evidence directory; no production image was enabled.

Only the two regenerable incremental directories
`target/debug/incremental` and `target/x86_64-unknown-none/debug/incremental`
were removed at the user's disk-cleanup request. About 52 GiB was freed;
about 46 GiB remained free after building. Source, raw evidence, historical
manifests and release services were preserved. `disk-cleanup.json` records the
exact command and before/after observations. No destructive Git command,
commit or push was used.

## Compatibility, baseline failures and current diagnostics

These rows distinguish completed earlier work from final-source acceptance.
No additional test or probe commands were run after the user's instruction.

| Check | Existing evidence | E3 final-source result |
|---|---|---|
| Kernel host, broker authority, IPC/service, session, token replay/exhaustion | E2: 131 kernel tests passed | NOT_RUN |
| UAC/Account and VFS delegation host libraries | E2: 258 library tests passed | NOT_RUN |
| SunlightD runtime | E2: 19 passed | NOT_RUN |
| Scheduler pin release | E2: 63 focused host tests and documented native replays | NOT_RUN |
| Native Doorbell | E2 compatibility replay; historical unresolved D3 remains independent | NOT_RUN |
| SunlightD D1/D2/D3 and RecoveryBlocked | E2 documented native replays | NOT_RUN |
| Exec/exit lifecycle | E2 documented native replays | NOT_RUN |
| VM/TLB and shared-memory retirement | E2 documented native replays | NOT_RUN |
| Certificate-verified native TCP/TLS | E2 documented native replays | NOT_RUN |
| Audio/media compatibility | E2: 36 media tests pass, 1 missing WAV fixture fails; native replay qualifications retained | NOT_RUN |
| E3 parser counterexamples | Seven parser-only tests passed before test stop; `parser-final.log` | PASS, parser scope only |
| Test-enabled kernel target build | Final `build-20261024T141606.266566Z/kernel.log` | PASS, compilation only |
| Test-enabled UAC/fixture build | Same run's `services.log` | PASS, compilation only |
| Ordinary production kernel/services build | Ordinary E2 staging preserved/restored | NOT_RUN for E3 final source |
| Strict Clippy | Historical FAIL; no suppressed-category result substituted | NOT_RUN |
| Whole-workspace formatting | Historical differences remain | NOT_RUN |
| Miri | Historically unavailable | NOT_RUN |
| git diff --check | Passed before the user stopped validation; not rerun after report edits | NOT_RUN for completed report tree |

The historical D3 activation failure remains UNEXPLAINED at
`target/fastlane-f13na-evidence/d1-d2-d3-failed-final-harness/serial-2cpu.log`.
E2's strict Clippy failures and formatting differences are preserved in its
baseline comparison. No E3 sweep established the absence of new diagnostics.
The concrete E3 compile errors above were corrected and the final test build
succeeded; that does not imply strict Clippy, ordinary production compilation,
whole-workspace formatting or compatibility acceptance.

## Final source and image identity

The final build is `build-20261024T141606.266566Z`. Its manifest records 1,280
source/harness input hashes before and after building, unchanged. The root
aggregate independently records the final source map and artifact hashes as
file-identity bookkeeping, without executing tests. The source selection covers
tracked/unignored Rust, TOML, linker scripts, Python, shell, assembly, Cargo.lock
and Limine configuration. It does not claim a hermetic rebuild of every
previously staged dependency/service. Embedded executable bytes are identified
separately, including services inherited from the existing build staging.

| Artifact | SHA-256 |
|---|---|
| Final kernel executable | `f29d3f875c3aa31b5fdb79a7c41581db9cad71eea2d4ce07d6091b3a268adff6` |
| Stripped kernel.elf | `bbb48d735c04d4f2088330b597438cf5675f54a6b71bd676ff4e6d4633369d09` |
| Final fixture.iso | `15ae6cf69f2b288eb1e33e5bd65c3353d2540f3dab86466b06e33b5885683749` |
| Embedded init | `d31e6bb3fe21c42a98cc2a76064007b2f94752df7e190887be486d83f0c87c7e` |
| Embedded SunlightD | `4ad1fcaff41fc595e17d49e72a3bb5430a9891cd22d9f3d7fc8a1ed6aa0e3c30` |
| Embedded TTY | `f7282c1dc82e8cdf0f87d6841b45d2e791c77a84063f0ddcd1022ee6937f3c4f` |
| Embedded test UAC | `017ff8d127c2c5c05eec1daf5fc96cbde9cd4d06656f8943b5b5d868d4d65c63` |
| Embedded sessiond | `9d29183c16b97ca8d484bf32afa1124488b1280807712085fb391797b75a427b` |
| Embedded VFS | `bca2aba5b01fad205459b52f3776fa1fd05c14ef952d9533d1c00cf491ff2a83` |
| Embedded fixture | `07183b9c2514b501a50ff77e9ede0ec5231146ccc8ef6f260b166a4b17a993f7` |

All seven listed service executables are archived in the final build's
`embedded/` directory, with actual embedding offsets in its manifest. Earlier
native images have different source and image identities; the aggregate lists
each delta and does not accept them as one final-tree run. No native result
was produced from the final image. A final ordinary image has not been rebuilt.

The complete index is
`target/fastlane-f11b0e3-evidence/validation-summary.json`. Original per-attempt
summaries and logs remain authoritative records of their original classifications.
The index records all captured markers, commands, failures, retries, resource
observations and unavailable evidence. It does not rerun the final assessor
against old logs to manufacture acceptance.

## Acceptance and exact F1.1B-0F handoff

Session Authority remains **PARTIAL**: real production-path evidence improved,
but the required final-source native and compatibility acceptance was not
completed. Grant mint and consume remain RESTORED_PARTIAL. Password mutation,
profile mutation and group mutation remain FAIL_CLOSED. Gate A remains VERIFIED
NATIVE, Gate B PARTIAL, Token-History PARTIAL, F1.1B-0 BLOCKED, Gate C BLOCKED.
No operation availability is promoted on the strength of diagnostic races.

| Unsatisfied invariant | Relevant source | Production implementation / host evidence | Native forced evidence | Minimum remaining work | Gate impact |
|---|---|---|---|---|---|
| Final-source N1–N7 order and complete fixture teardown | `capability/session_race.rs`, `capability/lifecycle.rs`, `sched/mod.rs`, E3 driver | Existing broker; E2 host validation; final hooks compile | Earlier 2-CPU snapshots only; N4–N6 originally NOT_FORCED; final NOT_RUN | Complete E3 from a frozen ordinary/test build, run supported 1/2/4/12 matrix with exact current cleanup assertions | Session Authority stays PARTIAL |
| Replacement session registration independently accepted; no old/new alias | `capability/sessions.rs`, `sunlight-sessiond/src/main.rs` | Identity/checked epoch implementation; E2 host checks | Earlier old-reference denial after B activation; no fresh B-session registration/exhaustion | Real fresh login under B; preserve old-reference denial; identify checked exhaustion separately | Session Authority acceptance evidence incomplete |
| Actual recipient PID reuse and remaining lifecycle/restart conflicts | `process/pid.rs`, `capability/delivery.rs`, `ipc/mod.rs` | Incarnation/VM/call binding implemented; host checks exist | Actual PID reuse NOT_FORCED | Establish legitimate allocation/exhaustion boundary; force remaining delivery/cancel/exec/retirement boundaries without a PID-only bypass | F1.1B-0 BLOCKED |
| Full broker/VFS/UAC inventory and lifetime accounting | `capability/mod.rs`, `capability/sessions.rs`, `services/vfs_server/src/main.rs` | Subset counters and lifecycle logic exist; no complete global inventory | Session subset only; failed attempts unreconciled | Reconcile all ownership classes, pending calls, retained metadata and allocator/frame effects | F1.1B-0 BLOCKED |
| Token-history lifetime capacity availability | `capability/token_history.rs`, `capability/history_boundary.rs` | Finite 65,536 fail-closed policy; host exhaustion checks | Earlier isolated exhaustion boundary; no global boot-lifetime acceptance | Preserve rejection; establish supported lifetime/capacity policy and global saturation evidence | Token-History PARTIAL; F1.1B-0 BLOCKED |
| Mutation publication authorization and durable lifecycle | `sunlight-uac/src/bin/account_service/mod.rs`, VFS publication path | Intentionally unavailable; policy host checks do not prove publication | No native production mutation acceptance | Separate authorized publication/rollback proof; retain current refusal meanwhile | Password/profile/group FAIL_CLOSED |
| Final-source compatibility and ordinary build isolation | E3 build/parser, existing E2 compatibility drivers | Final fixture build only; E2 suites are older evidence | Final-source checks NOT_RUN at user direction | Only when validation is resumed: rebuild ordinary/test artifacts consistently, replay required compatibility, preserve failures | Session Authority PARTIAL; other gates independent |
| Historical D3 activation uncertainty | `services/sunlight-startup-fixture/src/d3_driver.rs`, historical raw log | Production lifecycle exists; host/runtime checks exist | Earlier unexplained failure despite later replays | Explain/reproduce exact historical failure without erasing it | Gate B PARTIAL |
| Restricted SHM mapping, active revocation and secure transport | `kernel/src/memory/`, existing FastLane authority reports | Outside E3; not enabled | Required native acceptance absent | Separate authorized phase after prerequisites | Gate C BLOCKED |

These are handoff facts, not authorization to start F1.1B-0F. E3 acceptance
remains unfinished at the user's instruction to omit further tests. No broader
authorization architecture project or scheduler redesign is needed to explain
the evidence gap found here.
