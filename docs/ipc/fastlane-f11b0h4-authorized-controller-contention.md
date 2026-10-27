# F1.1B-0H4: authorized controller and native contention

2026-10-28. **Incomplete; final-source native acceptance NOT_RUN.** The user
narrowed validation to host-only checks, then requested this summary. Testing has
stopped. Evidence is retained in `target/fastlane-f11b0h4-evidence/`, indexed by
`validation-summary.json`. No production gate is upgraded by fixture readiness.

## Executive result

Controller preparation, bounded rendezvous, trace observations and an independent
completion protocol are implemented behind test features. Host tests and both
instrumented and ordinary production builds passed. Clippy, media and formatting
checks have current failures; Miri is unavailable. The final image was not run.

An intermediate two-CPU N4 image demonstrated READY before the grant checkpoint,
ordinary manager acceptance, grant commitment before retirement and guest/host
completion. A later session-owner run observed retirement then grant rejection,
but failed a subsequent fixture assertion. Neither validates the final snapshot.

| Milestone | Final-source result | Earlier snapshot observation |
|---|---|---|
| Prestarted controller READY | NOT_RUN | Observed in N4 and S2 |
| Manager request ACCEPTED | NOT_RUN | Observed in N4 and S2 |
| Reaper contender observed | NOT_RUN | S2 reached target exit; acceptance incomplete |
| Grant-wins ordering | BLOCKED | N4 committed before retirement |
| Retirement-wins ordering | BLOCKED | S2 retired before rejection; later assertion failed |
| Genuine scheduler-lock contention | NOT_FORCED | S2 failed try-lock observed; complete proof missing |
| Guest/host completion handshake | BLOCKED | Earlier N4 receipt passed; final protocol not replayed |
| 2/4/12-CPU verification | NOT_RUN | Only intermediate two-CPU attempts |

No unauthorized grant commitment was demonstrated. That observation does not
establish final-source acceptance or close wider authority gates.

## Initial execution timeline

1. Authenticated root fixture spawns `session-racer` and passes an ordinary send endpoint.
2. Account accepts policy, calls the controller at stage zero, and arms N4.
3. Controller replies; Account enters `sys_account_grant` and `n4_boundary`.
4. `observe_n4_pending` checks live issuer/session/request binding under scheduler then broker guards. Both guards are dropped before the bounded wait. This is observation, not authorization or commitment.
5. Account remains executing in ring 0 on its owning CPU. The controller observes N4 ready.
6. **First unnecessary dependency:** only now does the controller spawn `sunlightctl`, load its executable, perform protected manager lookup, and submit restart IPC.
7. H3's bounded wait expires before the real manager signal. Grant acquires SCHEDULER and commits with live authority.
8. Later, manager stop queues SIGTERM. Process termination, resource reaping and broker retirement are distinct later events.

The repair must prepare the existing authorized command route before step 2, without holding any kernel guard across the coordination wait. CPU ownership also means an actual resource reaper cannot retire the currently executing Account stack; requested winner orders must be classified against that production contract.

## Initial completion diagnosis

H3 N6 logs `fixture-complete`, process exit code zero, `fixture-ipc-cleaned` and process reaping, followed by `[TTY] shell FG_DONE IPC timeout`. The fixture called session logout before completion, and sessiond terminated the shell which owns the foreground completion protocol. The driver's required `exit: /tmp/session-race-probe N6 -> 0` is unreachable through that dead shell. This is a host protocol defect with an independently observed TTY foreground notification failure, not proof that guest security assertions alone constitute an end-to-end pass.

## Controller preparation and authorization

`services/sunlightctl/src/main.rs` adds feature-only
`restart <unit> --h4-prepare <private-bootstrap-handle> <fixture-id>`.
`services/sunlightctl/src/h4_state.rs` tracks Starting, Ready, Submitted and
Terminal. The controller performs its normal protected nameserver lookup and
ordinary status request, resolves the running target PID and prepares the existing
restart wire request before READY. Its private fixture endpoint carries readiness
and release; no manager capability is delegated to `session-racer`.

`services/sunlight-uac/src/bin/session_race_probe.rs` waits for actual READY before
releasing Account's stage-zero rendezvous. The grant checkpoint then releases the
already prepared controller, which submits the ordinary restart and checks its
ordinary reply. `services/sunlightd/src/main.rs` records acceptance after existing
request admission and target resolution, before the existing stop operation.
The kernel correlates this with the actual active manager. READY, submission,
acceptance, authorized signal and reaper acquisition remain distinct events.
Normal manager policy, restart meaning and capability checks are unchanged.

## Safe synchronization and real boundaries

`kernel/src/capability/session_h4_gate.rs` implements a bounded one-shot gate:
GRANT_READY, EXIT_READY, GRANT_DONE, RETIRED and ABORTED. Timeout abort releases
both waiters; late events cannot rearm it. `session_h4.rs` connects these states
to actual transitions. No cross-CPU wait holds a scheduler or broker guard.
Interrupts are not disabled for ordering and runnable state is not manually changed.

The S cases restart ordinary `sunlight-sessiond`, the session owner, while Account
remains alive. S1 releases grant first and waits for its result before exit;
S2 releases exit first and waits for actual broker retirement before grant;
S3 requires both safe boundaries before releasing both. These final fixtures
remain without native acceptance. Test placement uses CPU zero for manager,
controller and session owner and CPU one for Account; actual participants are logged.

`kernel/src/arch/x86_64/syscall.rs` observes actual grant acquisition/result;
`kernel/src/sched/mod.rs` observes target exit acquisition/release. Test-only
failed `try_lock` observation falls back to the ordinary lock. The final parser
requires a failed acquisition inside the other identified contender's recorded
acquired-to-release interval. Signal-lock conflicts, unrelated contention and
sequential operations cannot satisfy this requirement.

## Trace identity and reaper path

The bounded H3 buffer is reused: schema 1, capacity 256, CPU IDs and event sequence.
READY is retained before authenticated Account arming. Applicable events retain
fixture identity, process incarnation, session epoch, endpoint identity and IPC
call generation. Endpoint identity and call generation are separate. Actual lock,
grant, broker retirement, peer-close, cancellation and terminal transitions supply
observations. No credentials or raw bearer capabilities are logged. Recording
does not perform blocking serial I/O under critical guards.

Captured N4 followed Account self-exit, broker retirement, endpoint queue removal,
peer-call closure, then resource cleanup. S2 followed sessiond self-exit, broker
session retirement, Account grant rejection and resource cleanup. It did not
capture the required peer-call closure case. Historical **peer-close-first remains
NOT_FORCED**. A later test needs the genuine resource-reap trigger with matching
endpoint and pending-call lifetime; changing restart path selection is no substitute.

## Complete attempted native ledger

All directories below are under the evidence root; directory timestamps are UTC.
Each manifest preserves exact QEMU commands, source/image identities, serial
hashes, errors and raw observations. None is final-source acceptance.

| Attempt | Recorded classification | Outcome |
|---|---|---|
| `native-20261027T210200.669816Z`, N4/2 | FORCED_AND_FAILED | Sandbox denied QMP socket bind before boot; empty serial |
| `native-20261027T210901.174169Z`, N4/2 | PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT | Prepared controller, manager acceptance, grant-first, earlier completion receipt; no independent reaper contention |
| `native-20261027T210949.338455Z`, H4R/2 | FORCED_AND_FAILED | READY; rendezvous expired before observed acceptance; legitimate grant committed; assertion failed |
| `native-20261027T211510.576873Z`, S2/2 | FORCED_AND_FAILED | Retirement then grant rejection; later fresh-request assertion expected wrong denial class; no terminal receipt |

In N4, READY was sequence 1, grant checkpoint 5, submission 6, acceptance 7,
grant acquisition 10, commitment 11, exit acquisition 13, retirement 15,
endpoint removal 16 and peer-call closure 17. Account PID/incarnation 18/30 ran
on CPU one; manager 11/23 and controller 41/53 ran on CPU zero. Session/epoch
was 1/1, endpoint 15, call generation 4. There was one commitment before retirement.

In S2, controller target sessiond 31/43 ran on CPU zero and Account 18/30 on
CPU one. Exit acquired at sequence 12, retirement committed at 14, grant try-lock
failed at 16, grant acquired at 18 and rejected at 19. Initial broker rejection
was `SessionAuthorityUnavailable` (11). The subsequent fresh request can follow
existing session policy/route behavior and return `Unavailable` (1) or
`SessionChanged` (5). That fixture expectation was corrected afterward; precise
lock release interval observations were also added. Neither correction was
replayed. The older parser's false contender flags and errors remain preserved;
raw transitions do not override its failed terminal outcome.

The H4R failure and intermediate nonzero inventory remain failures. No timeout
extension or further escalation test was performed after scope narrowed.

## Completion protocol repair

The original N6 diagnosis above distinguishes guest security completion from
the failed foreground notification. The revised test-only runner uses independent
existing kernel cleanup observations rather than requiring the terminated shell
to report completion. `tools/fastlane_f11b0e3_validate.py` and
`tools/fastlane_f11b0h4_validate.py` require correlated assertions, normal fixture
exit, reaped participants, zero fixture IPC references, complete bounded trace
and matching source/image/service identities. Guest parsing remains pending host
receipt. Only then can the host write and read back `.host-ack.json`, bound to
fixture, case, CPU count and serial/ISO/ELF digests.

Negative host tests reject absent assertions, cleanup or acknowledgement;
timeout; incomplete/overflowed trace; stale/wrong fixture or case; and incorrect
identity. Synthetic transcripts test the parser only, not native behavior.
Final-source end-to-end completion remains BLOCKED.

## Resource inventory

Intermediate N4 recorded zero sessions, receipts, Account grants and pending
Account deliveries, one registration and one retired session. Failed S2 also
recorded zero in those four categories, without terminal completion. H4R retained
one session, receipt, grant and pending delivery at its incomplete checkpoint.
Allocator and service heaps are UNMEASURED. Final-source inventory is NOT_RUN.

## Final-source host and compatibility checks

Exact commands, environment, exit codes and logs are in
`checks-20261027T213912.314594Z/validation-summary.json` and
`host-focused/validation-summary.json`.

| Check | Result |
|---|---|
| Kernel capability/IPC host library | PASS, 133 tests |
| IPC/UAC/sessiond/FS and related service libraries | PASS, 258 tests |
| SunlightD runtime | PASS, 19 tests |
| Scheduler synchronization host checks | PASS |
| Existing/H4 parsers | PASS, 49 methods including 6 H4 methods |
| Controller / rendezvous / bounded trace | PASS, 2 / 3 / 1 focused tests |
| New H4 module formatting / git diff --check | PASS |
| Changed kernel files / workspace formatting | FAIL_CURRENT |
| Strict Clippy | FAIL_CURRENT; virtio/IPC diagnostics; matched baseline unavailable |
| Media fixture | FAIL_CURRENT, 36 passed / 1 failed; missing WAV |
| Miri | UNAVAILABLE; cargo-miri absent |
| Ordinary production kernel/services/ISO | PASS |
| Final-source N4/N6, S1/S2/S3, D3/RecoveryBlocked | NOT_RUN |
| Final-source 2/4/12 CPU matrix and fresh login | NOT_RUN |

Historical H, N3M, D3, earlier N4, strict Clippy, WAV, formatting and Miri issues
remain preserved. Similar diagnostics do not prove matched baseline equivalence.
No general architecture audit or cross-service census was added.

## Source and image provenance

Final instrumented manifest: `build-20261027T213832.904821Z/validation-summary.json`.
Kernel features are `native_session_h4_test,key_inject`; sunlightctl, sunlightd
and UAC use `session-h4-test`. Environment:
`SUNLIGHT_INJECT_PHASE=desktop_login`, `SUNLIGHT_SESSION_N4_AP=1`.

| Artifact | SHA-256 |
|---|---|
| Unstripped kernel | `a7ed31b85e98ff7b7cb3a06f5e556871713ab9877ace78eab64afa5298cf4666` |
| Kernel ELF | `f2ff39786b19e38281dbb64c8ec739a05076beecee0f8bc32fc9422cc15b712e` |
| Instrumented ISO | `637ae6c73f789b6d67c27dae35ae08e4680fa914920e9df696c08b6a78691edd` |
| Embedded Account/UAC | `9dcf5b70ee456caf81ce4ca7ac41d00c989ff51a61775e8265adbd2872131564` |
| Embedded VFS | `c2237b80067fd06f5b65dd1d74539c94dd9f5753ba6ec7fb506465038eecbcca` |
| Embedded sessiond | `9d29183c16b97ca8d484bf32afa1124488b1280807712085fb391797b75a427b` |
| Embedded sunlightctl | `c5c36332676642f676153c4ddfe95bd9f0db31b63cbcbd232f002a9e61cf1826` |

Manifests retain 1,295 build-input hashes and embedded executable hashes, sizes
and offsets. Production evidence is
`normal-20261027T213940.086681Z/validation-summary.json`; its source map matches
the final instrumented build. Summary creation rechecks that map and archived
ISO/ELF hashes. Documentation is outside the build-input map and this report has
a separate digest in the aggregate JSON. The newly built final image **was not
executed**. Earlier images remain comparison evidence only. Disk was checked
before builds. No commit/push or destructive Git operation was performed.

## Remaining gates and smallest next executable step

Scheduler Gate A remains VERIFIED NATIVE; Token-History remains VERIFIED BOUNDED,
65,536 identities per boot. Reaper/Endpoint Ordering, Global Authority, Grant
Lifecycle, Session Authority and Doorbell Gate B remain PARTIAL. F1.1B-0 and
production Gate C remain BLOCKED. Restricted SHM, public Doorbell and production
FastLane remain disabled.

The immediate remaining acceptance gap is a completed final-source two-CPU
session-owner fixture with corrected expectations, precise contender lock
intervals, required cleanup and host acknowledgement. If native validation is
resumed later, that is the smallest next executable step, before more orderings
or 4/12 CPUs. It was not run for this summary. Peer-close-first and wider
authority gates continue to require separate evidence.
