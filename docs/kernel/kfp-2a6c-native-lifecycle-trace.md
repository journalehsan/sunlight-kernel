# KFP-2A.6C — Native lifecycle receipts and R2/R4 proof

**Overall: PARTIAL. R2: COMPLETE. R4: COMPLETE. R6: NOT_RUN.** Production runtime behavior did not change. Production FastLane remains disabled. No commit or push.

The primary R2 objective is established by a dedicated authenticated two-CPU SessionD guest and independently assessed native receipts. R4 exercises the actual owned-endpoint closure path against a real, independently executing SIGKILL contender. Neither a legacy M1 assessor nor a revocation adapter supplies these proofs. This milestone does not establish R6, arbitrary nested exit-hook continuity, global physical resource exactly-once accounting, or the expanded CPU matrix.

## Source, contract and preservation

Revision: `127aee1d938781877264f6d0e13f67c3fcd89327`. The incoming working tree contained package/version/Cargo.lock modifications, the earlier channel fixture and untracked KFP reports. `baseline.json`, `status-before.txt` and `preexisting.patch` preserve it. The required 2A.6, 2A.6A, 2A.6B reports and 2A.6B results were read. `source-integrity.json` checks protected source hashes, historical inputs, version-state diff, and preservation of the incoming fixture's existing guard-release observer and boundary helper. No unrelated changes were reverted. Source snapshots retain relevant kernel, service, tool and build files for the accepted and preceding attempts.

Native self-exit acquires its final scheduler guard, commits Finished/pending, synchronously calls mapping's early Doorbell sweep, closes owned IPC endpoints and later permits pin-gated physical reaping. `channel_participant_terminal` searches the complete PID/incarnation/VM tuple before testing exiting state. The classifier itself is unchanged. The Doorbell callback calls that real classifier and passes its actual result to the unchanged `retire_unavailable` decision.

Source locations below refer to the final source map and readable `source-snapshots/final/` archive. These are real decision boundaries, not fixture-generated events.

| Boundary | Final source location |
|---|---|
| Self-exit and final guard | `kernel/src/sched/mod.rs:4420`, receipt at `:4480` |
| Finished/pending publication and early sweep | `kernel/src/sched/mod.rs:4554` |
| Real participant identity classifier | `kernel/src/capability/channel_native.rs:11` |
| Native Doorbell sweep/classifier callback | `kernel/src/capability/channel_doorbell_native.rs:51` |
| Candidate, terminal store and Retired state | `kernel/src/capability/channel.rs:120`, `:138`, `:142` |
| Native process-retirement hook | `kernel/src/capability/channel.rs:184` |
| Owned endpoint removal and closure | `kernel/src/sched/mod.rs:2742` |
| Physical reaping and gated cleanup | `kernel/src/sched/mod.rs:2775`, `:2913`, `:3098` |
| Real SIGKILL contender and finished-owner rejection | `kernel/src/arch/x86_64/syscall.rs:388` |
| Post-drop observation | `kernel/src/sched/mod.rs:4603` |

`TERMINAL_COMMITTED` follows the actual terminal-reason store. `RETIREMENT_COMMITTED` follows the later channel State::Retired store. Thus the valid contract is **EXIT_STARTED → exact exiting/live sweep → IDENTITY_VALIDATED → TERMINAL_COMMITTED → RETIREMENT_COMMITTED**. The later `ChannelTable::retire_process` hook is distinct. It is called by exit and again by reaping; two hook invocations are required and are not two terminal publications. The native first-committed terminal guard preserves the winner without numerical reason priority.

`close_owned_ipc_endpoints` removes broker endpoints/tokens, invokes recipient retirement, removes the real IPC endpoint and finishes detached peer-closed calls. There is no EndpointClosed channel terminal. Native owner-death closure contributes ProcessRetired through recipient retirement; the real revocation call used later is a separate losing candidate. Endpoint IDs come from the broker's checked, boot-lifetime nonreuse allocator. No new endpoint generation is invented.

## Receipt schema and coordination

`receipt-schema.json` specifies every field and event. Each immutable receipt carries test generation, operation, PID/incarnation/VM, Account session id/epoch, channel generation, CPU, event, sequence, outcome/state/detail and IF. Endpoint events carry the actual endpoint ID in detail. Context is bound by an exact-creator authenticated arm; process events require full participant equality and channel events require exact session/channel equality. No tokens, secret seeds or bearer grant material are logged.

The final storage is a static 1024-slot append-only buffer: 120 KiB for receipt/publication words plus 88 bytes of context/counters. It never reallocates or reuses slots during a boot. One atomic reservation assigns sequence; Release publishes each immutable slot and readers use Acquire. There are no recording-path allocations, sleeps, synchronous IPC, serial output or lock acquisition. Overflow rejects proof. Actual lifecycle mutations already serialize under SCHEDULER; pre-lock contention and post-drop observations use atomics with the existing release/acquire acknowledgment edges. Wall-clock timestamps are not ordering evidence.

R2 adds no pause point. Its ordering follows the production self-exit call graph under the existing locks. The surviving creator waits outside kernel lifecycle locks for the actual cleanup-completed receipt; waitpid alone can return at Finished. Dumping occurs later in the authenticated diagnostic query. Identity observations cover live-channel classification; every actual terminal candidate, rejection and publication remains recorded after retirement.

R4 extends the existing debug acquisition fixture with mode 6 for native self-exit versus native SIGKILL. Before READY, waits retain validated private task continuations without lifecycle locks. The already-running remote contender has IF clear and a distinct CPU; it performs a nonblocking try_lock, records the real failure and acknowledges before taking serial/scheduler locks. The holder's finite existing 100-million-iteration acknowledgment wait depends only on these remote atomic instructions, not interrupt delivery or scheduling. No timeout was increased. Exhaustion is COORDINATION_BLOCKED, never proof. `coordination-review.json` records locks, interrupts, preemption, CPU dependencies, bounds and release/fail-stop paths.

## R2 native causal trace

Build `r2-20261110T151904.335452Z`; test generation `1794323944448370`; identity `{'generation': 1794323944448370, 'operation': 1, 'pid': 40, 'inc': 85, 'vm': 98, 'session': 1, 'epoch': 1, 'channel': 1}`. The independent FL-BIND authorization/attachment receipt matches the same participant, session and channel. At sweep observation the exact process is exiting and the channel is still live. The actual classifier returns ProcessRetired=2, not IdentityChanged=5.

| Sequence | Actual event | CPU | Outcome | State | Detail |
|---|---|---|---|---|---|
| 57 | EXIT_GUARD_ACQUIRED | 1 | 0 | 1 | 0 |
| 58 | EXIT_STARTED | 1 | 0 | 3 | 1 |
| 59 | SWEEP_OBSERVED | 1 | 1 | 3 | 1 |
| 60 | IDENTITY_VALIDATED | 1 | 2 | 3 | 1 |
| 62 | TERMINAL_COMMITTED | 1 | 2 | 3 | 0 |
| 63 | RETIREMENT_COMMITTED | 1 | 2 | 4 | 0 |
| 64 | ENDPOINT_CLOSE_ENTERED | 1 | 0 | 0 | 0 |
| 65 | RETIRE_PROCESS_ENTERED | 1 | 2 | 0 | 0 |
| 68 | ENDPOINT_BROKER_REMOVED | 1 | 0 | 0 | 33 |
| 69 | ENDPOINT_CLOSED | 1 | 0 | 0 | 33 |
| 70 | EXIT_GUARD_RELEASED | 1 | 0 | 0 | 0 |
| 75 | CLEANUP_STARTED | 0 | 0 | 3 | 1 |
| 76 | RETIRE_PROCESS_ENTERED | 0 | 2 | 0 | 0 |
| 81 | CLEANUP_COMPLETED | 0 | 0 | 4 | 0 |

The trace has exactly one terminal store and one cleanup completion. Exit and reaper retirement hooks each produce a real rejected repeat candidate. The later authorized Revoked=1 candidate is rejected with ProcessRetired=2 preserved. Cleanup entry follows the real pin/current/owner/queue/active-VM checks; completion follows resource teardown, pending=false, kernel stack removal and Reaped. The original VM identity is retained across address-space teardown. This is exact process/endpoint ownership and cleanup-path evidence, not an independent per-frame PMM free counter.

## R4 native closure and losing retirement contender

Build `r4-20261110T151744.708691Z`; test generation `1794323864867739`. Operation 2 selects R4; the dedicated case is FL-LC-R4. The common client retains KFP-R2 informational markers, while the receipt operation, controller suite and KFP-R4 completion marker disambiguate execution.

| Sequence | Actual event | CPU | Outcome | State | Detail |
|---|---|---|---|---|---|
| 81 | RETIREMENT_CONTENDER_PROBED | 0 | 1 | 0 | 5 |
| 82 | EXIT_GUARD_ACQUIRED | 1 | 0 | 1 | 0 |
| 83 | EXIT_STARTED | 1 | 0 | 3 | 1 |
| 84 | SWEEP_OBSERVED | 1 | 1 | 2 | 1 |
| 85 | IDENTITY_VALIDATED | 1 | 2 | 2 | 1 |
| 87 | TERMINAL_COMMITTED | 1 | 2 | 2 | 0 |
| 88 | RETIREMENT_COMMITTED | 1 | 2 | 4 | 0 |
| 89 | ENDPOINT_CLOSE_ENTERED | 1 | 0 | 0 | 0 |
| 90 | RETIRE_PROCESS_ENTERED | 1 | 2 | 0 | 0 |
| 93 | ENDPOINT_BROKER_REMOVED | 1 | 0 | 0 | 33 |
| 94 | ENDPOINT_CLOSED | 1 | 0 | 0 | 33 |
| 95 | EXIT_GUARD_RELEASED | 1 | 0 | 0 | 0 |
| 102 | CLEANUP_STARTED | 0 | 0 | 3 | 1 |
| 103 | RETIRE_PROCESS_ENTERED | 0 | 2 | 0 | 0 |
| 108 | CLEANUP_COMPLETED | 0 | 0 | 4 | 0 |
| 109 | RETIREMENT_CONTENDER_ACQUIRED | 0 | 0 | 4 | 0 |
| 110 | RETIREMENT_CONTENDER_REJECTED | 0 | 1 | 4 | 0 |

The failed native SIGKILL acquisition probe precedes exit's final guard. Native owned-endpoint broker removal and IPC closure complete before the SIGKILL contender acquires and rejects the already-Finished/Reaped process. Self-exit code zero remains unchanged; SIGKILL's code 137 does not replace it. No channel terminal candidate is manufactured for a signal operation rejected before channel retirement. The channel's early sweep already won ProcessRetired; closure's recipient-retirement candidate and a later real Revoked candidate cannot overwrite it. This proves the observed ordering, not a universal rule that closure always wins. No outstanding IPC peer-call outcome is claimed when the endpoint has no detached calls.

A Reaped process has an emptied address-space identity. The recorder qualifies its original owner using the existing native service_reap_receipt[0], exact PID/incarnation, Reaped/pending/stack conditions, and records current VM in detail. It never substitutes a successor process or grants authority. Host controls reject changed incarnation, live VM replacement and mismatched retired VM.

## Failed attempts and evidence limits

All attempts remain in `attempt-history.json` and their original directories. A private-interface compilation failure, a build-ID assertion aimed at the authority daemon instead of the actual client ELF, and compiler-folded decimal generation text were rejected before guest launch. The payload now includes an explicit generation string and the actual client ELF is checked.

The first executed R2 snapshot was drained before physical cleanup; its assessor correctly rejected missing cleanup receipts. A later assessment incorrectly required a single retirement-hook invocation. Its original result is retained; the corrected assessor requires the real exit and reaper hooks, both losing to one earlier terminal store. Original M1/M2/M3/M4/M5 assertions and expected outcomes were not changed.

The first R4 receipt filter rejected observations after real VM reclamation; its missing receipts remain missing in that trace. A new guest records the native retired-owner correlation. A subsequent 256-slot replay overflowed and remains rejected even though its required events appeared. It is not canceled by an isolated pass. The final source uses the explicitly larger fixed 1024-slot test budget, retains all candidates, and is replayed with overflow rejection intact. Native assessors never fill missing events or relabel legacy traces.

## R6 and nested exit-hook continuity

R6 is NOT_RUN. R4's two real active invocations and the sequential sweep/close/reap stages are not three-operation cleanup overlap. `r6-blocker.json` records the specific missing mechanism: the current coordinator validates two private-stack actors, while physical reaping begins under SCHEDULER. Pausing at CLEANUP_STARTED and waiting for a sweep needing that same guard would deadlock. Yielding a reaper/interrupt continuation through a task-private helper is not validated. No such pause or fabricated callback was added. This is a limit of the implemented coordinator, not a claim that every safe future design is impossible.

Initial final-guard acquisition and post-drop observations are recorded. The latter is an upper bound after real drop, not the mutex's internal unlock instruction. The existing channel-to-Doorbell guard handoff is preserved; continuous exclusion across that handoff is not claimed. R4 proves an independent already-finished signal guard rejection, not nested self-exit or arbitrary reentrant cleanup. Event 13 is emitted only if the actual nested/already-finished self-exit branch occurs; no such witness is synthesized. Direct recursive entry into the non-reentrant scheduler mutex is not a safe rejection test.

A next R6 implementation needs an ownership-safe, bounded three-actor coordinator outside lifecycle locks, actual invocation/claim/completion intervals, independent resource disposition receipts and a real reachable nested-entry/reentrancy contract. No SHM/VMM/MAP_FIXED development or production exit change was introduced to facilitate it.

## Integrity, regressions and CPU matrix

`guest-integrity.json` independently rehashes each final ELF, ISO and extracted ISO kernel, rechecks archived embedded payload bytes/offsets, and records source revision/map hash, requested features, build ID, payload hashes, test generation, command and CPU count. Dedicated R2/R4 client payloads contain their intended case, build ID and generation, and match the bytes embedded in the ELF. Each configuration cleans the kernel package before building to avoid stale service embedding. `feature-sets.json` records requested and manifest-expanded configurations. All final selected guests match the final source map; older snapshots retain their own qualification.

| Final-source check | Result |
|---|---|
| original | PASS |
| combined | PASS |
| doorbell | PASS |
| m4 | PASS |
| m5 | PASS |
| production-check | PASS |
| kernel-host | PASS |
| ipc-host | PASS |
| doorbell-host | PASS |
| m1-host | PASS |
| scheduler | PASS |
| Receipt/assessor negative controls | PASS (14 R2 and 11 R4 retained-native controls) |

Kernel host tests: 212, including four receipt/identity tests; IPC: 94; Doorbell: 9. Scheduler focused lifecycle/state/accounting and extracted policy controls are host evidence. The production check is compilation, not a production boot. Native security suites retain their unchanged assessors. Test details and all commands are in `test-matrix.json` and the final driver logs.

| CPUs | Evidence |
|---|---|
| 1 | NOT_RUN: R6 gate open; physically simultaneous orderings may be inapplicable under single-CPU synchronization |
| 2 | Final R2/R4 lifecycle pilots and security regressions; R6 NOT_RUN |
| 4 | NOT_RUN: R6 gate open |
| 12 | NOT_RUN: R6 gate open |

Only explicitly gated debug instrumentation, fixture coordination/payloads and host assessment tooling changed. Production capability, exit, arbitration, scheduling and cleanup semantics remain unchanged. The capacity change is test-only static storage. Production FastLane remains disabled; no commit or push occurred. Overall remains PARTIAL until native R6, nested continuity, applicable CPU coverage and its regressions are complete. The recommended next step is the validated outside-lock R6 coordination/ownership design, followed by a two-CPU native witness before CPU expansion.
