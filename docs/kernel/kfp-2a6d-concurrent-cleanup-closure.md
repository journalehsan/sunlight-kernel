# KFP-2A.6D — Concurrent cleanup and nested exit-guard closure

**Verdict: BLOCKED. Native R6 overlap and nested exit-guard continuity remain unproved. This milestone made no production runtime changes. Production FastLane remains disabled. No commit or push.**

This is a dependency audit and fresh regression checkpoint. No safe three-participant coordinator was implemented. R6-A is blocked before launch; the requested R6-B–E, NG-1–5 and CPU expansion gates remain closed. The fresh two-CPU R2 guest is a baseline control, **not an R6-A attempt**. Existing R2/R4 traces are not relabeled as three-operation overlap. No confirmed security invariant violation was established.

Machine-readable findings and executions are in `.kfp-2a6d-evidence/`: `results.json`, `baseline.json`, `lock-dependencies.json`, `coordination-design.json`, `required-witness.json`, `r6-blocker.json`, `source-references.json`, `test-matrix.json`, `guest-integrity.json` and `source-integrity.json`. Raw guest logs and build manifests remain under `native/`. Scripts in this evidence directory reproduce the audit and controls; none is kernel instrumentation.

## Source baseline and preservation

Revision: `127aee1d938781877264f6d0e13f67c3fcd89327`. The incoming tree was dirty, including kernel/service/test-runner modifications, package manifests, Cargo.lock, version state, untracked lifecycle receipt modules and earlier reports. `status-before.txt`, `preexisting.patch`, `preexisting-index.patch`, and `source-baseline/` preserve the incoming state. `baseline.json` records 1,660 source/build inputs, 16,791 tracked file hashes, and 3,726 historical files from 2A.6A/B/C (excluding generated Python bytecode). Required 2A.6A/B/C reports and 2A.6C results were read and hashed.

This milestone changes no existing runtime or test-runner source. Historical receipts, assessors, assertions and failed attempts are preserved. All source references below refer to these unchanged baseline bytes; `source-references.json` includes line numbers and whole-file hashes. New deliverables are this report and `.kfp-2a6d-evidence/`.

External telemetry and address-space edits appeared in the shared live workspace during the first combined attempt. That guest executed and completed its raw security checks, then the original runner rejected the attempt on the **post-run source-integrity mismatch**. Its source-to-image binding is unqualified; the later baseline replay does not cancel this executed failure. Those edits were left untouched. The remaining controls run in `isolated-baseline/`, a private local clone of the same existing revision with the captured incoming patch and three untracked source files restored **only inside that clone**. All 1,660 source/build hashes match the captured baseline. No new commit was created. The executed source rejection, clone build failures, output-path setup failure, and separately executed baseline controls are retained; no replay overwrites a failed attempt. Final live-workspace validation is **NOT_RUN**. Statements about tested source in this report mean the captured baseline, not the externally edited live tree.

## R6 lock and scheduling dependencies

`SCHEDULER` is a global non-reentrant `spin::Mutex<Scheduler>` (`kernel/src/sched/mod.rs:4611`). It does not itself establish a resumable lifecycle transaction. The production methods take `&mut Scheduler` and are invoked with exclusive scheduler ownership. Releasing the mutex beneath a live method borrow would allow another CPU to create a conflicting mutable borrow. Dropping a guard and later rebuilding a reference is not an ownership-safe pause protocol.

| Operation | Actual boundary and lock order | CPU, interrupts, preemption | Completion owner |
|---|---|---|---|
| A: self-exit | `finish_current_process`, `sched/mod.rs:4420`; scheduler guard; mark Finished/pending; synchronous mapping/Doorbell sweep; owned-endpoint close | Exiting CPU; syscall FMASK clears IF; maskable timer preemption cannot run locally within this boundary | Exit caller owns logical finish and immediate endpoint closure; later reaper owns physical teardown |
| A: remote retirement | `send_signal_generation`, `syscall.rs:388`; SCHEDULER; authorization; `terminate_process_by_pid`, `sched/mod.rs:3286` | Signaling CPU; native syscall IF clear; running victim gets reschedule request | Signaler marks Finished/pending; victim execution/stack handoff must end before reclamation |
| B: Doorbell sweep | `channel_doorbell_native.rs:51`; SCHEDULER → CAP_BROKER; exact participant classifier inside `retire_unavailable` callback | Caller CPU; periodic sweep comes through timekeeper timer, exit sweep through exiting CPU; observed IF clear | Synchronous caller classifies and retires row; broker is dropped before wake completion |
| C: owned-endpoint close | `sched/mod.rs:2742`; SCHEDULER → CAP_BROKER; remove endpoint/token records and retire recipient grants; release broker; acquire IPC shard to remove real endpoint; release shard before completing detached calls | Exit owner CPU; no independently schedulable cleanup task is published here | Same exit caller owns real broker and IPC removals |
| C: public endpoint destroy | `syscall.rs:2585`; SCHEDULER → CAP_BROKER for owner/receive authorization and removal; release broker; IPC removal and peer completion while retaining SCHEDULER | Caller CPU; IF clear; public syscall provides no outside-lock cleanup decision boundary | Authenticated endpoint owner caller |
| C: physical reaper | `sched/mod.rs:2775`, eligible cleanup receipt at `:2913`; retains SCHEDULER; transient FD/VFS/IPC locks; later PMM → CAP_BROKER for residual grant/shared-page cleanup | Eligible scheduler CPU, potentially timer/idle continuation; observed IF clear; never assume a live task-private continuation | Scheduler owner after all pin, owner, queue, current-task, borrower and active-VM gates |

This table describes the relevant acquisition paths, not a whole-kernel lock-order proof. Broker and IPC locks in endpoint closure are separate scopes. The sweep returns from its broker section before the mapping drain takes PMM → CAP_BROKER (`channel_mapping.rs:31`). No new lock order or scheduler policy is introduced.

SYSCALL programs FMASK `0x44700`, including IF (`syscall.rs:218`). Timer work itself acquires SCHEDULER before `sched.tick` (`arch/x86_64/interrupts.rs:1139`). Timer-driven sweeps/reaping therefore do not escape scheduler serialization. Process affinity remains its actual `cpu_mask`/owner placement; no fixture pins a new participant. Physical cleanup may run on a different CPU after the outgoing stack pin and active execution are gone.

### Why the previous pause cannot be reused

The dependency cycle is:

```text
C owns SCHEDULER at CLEANUP_STARTED
  -> C waits for B's actual sweep observation
  -> B needs SCHEDULER to enter sweep/classifier
  -> SCHEDULER is released only when C proceeds
```

Pausing B inside the classifier is also unsafe: it retains both SCHEDULER and CAP_BROKER, while A/C need those locks to reach a decision. Pausing C after releasing CAP_BROKER but before IPC removal still retains SCHEDULER. A pre-lock atomic flag does not change either dependency.

The inherited `channel_race` two-actor fixture waits for a remote nonblocking-probe acknowledgment while retaining SCHEDULER and with IF clear (`channel_race.rs:375–408`). Its prior scoped forward-progress argument assumes an already executing remote actor that needs only atomics and `try_lock`. This is **not** a design satisfying 2A.6D's stricter prohibitions on scheduler-held waits and IF-disabled blocking. It remains unchanged solely for the explicitly requested existing regressions. No third participant or new pause is added to it; its regression PASS cannot certify new coordination safety.

On two CPUs, retaining both CPUs in IF-clear spins would additionally prevent a third ordinary actor from being dispatched. Increasing deadlines or iteration counts would not repair that dependency. No timeout was increased.

### Safe boundaries considered

1. User-space rendezvous before each syscall has no lifecycle locks and permits normal scheduling. It establishes arrival, not native decision overlap or cleanup ownership.
2. A syscall pre-lock boundary has no scheduler guard, but its continuation still has IF clear. Spinning there violates this milestone's blocking rule. Merely publishing entry and continuing leaves no controlled decision overlap.
3. The existing private-stack yielding helper drops guards and validates Running/current/owning-core, exact incarnation/VM, private RSP and stack top (`channel_race.rs:218–259`). Its `prepare_yield` must acquire SCHEDULER. It cannot be invoked while retaining SCHEDULER, nor can its proof be transferred to Finished/reaper/interrupt continuations. `CLEANUP_STARTED` is deliberately after those live-owner prerequisites have ended.
4. After normal exit guard release there is an ownership-safe boundary, but the early sweep, channel decision and immediate endpoint closure have already completed. That is too late to establish the requested competing decisions.
5. Public `endpoint_destroy` is a genuine authorized path, but also holds SCHEDULER throughout. Its broker method destroys the checked endpoint; it does not by itself invoke the process-wide `retire_recipient_grants` hook. It cannot be silently substituted for the native owner-death cleanup contender.

No inspected production decision boundary permits the required safe pause. This is a concrete limitation of the current call graph and validated continuations, **not a proof that all future test-only coordinators are impossible**.

## Coordination design and missing witness contract

`coordination-design.json` records a candidate protocol and its rejection **before implementation**. Exact authenticated actors and subjects would publish immutable, generation-qualified invocation/claim receipts with Release; observers would read with Acquire. Suspension must retain an owned continuation without lifecycle guards or mutable scheduler/process borrows, and must not block with interrupts disabled. Resume must revalidate authority and generation through the normal production path. Decision owners must never await peer progress. Exhaustion must preserve the failed attempt and publish no terminal outcome.

That protocol needs a validated resumable entry/claim/resume mechanism that these synchronous methods do not expose. Adding such a split or enabling production reentrancy solely to make the fixture work is not justified by an established correctness defect. No production repair is authorized by this audit's findings. Consequently no coordinator is installed and no new kernel wait exists.

The required receipt schema is specified, **not implemented**, in `required-witness.json`. A valid witness needs actual per-invocation entry, meaningful-boundary/claim, completion/rejection, actor PID/incarnation/VM, subject PID/incarnation/VM, real endpoint, session/epoch/channel generation, guard acquisition epoch/owner, CPU/IF, candidates, real commit and resource dispositions. An independent Acquire observation/acknowledgment must establish that all three invocation intervals contain the relevant boundary. R6-D additionally needs two distinct native failed synchronization attempts while the third owns the identified decision boundary. Timestamps, simultaneous spawning and historical flags cannot satisfy these predicates.

The existing append-only lifecycle log reserves a sequence atomically and publishes immutable slots with Release; readers use Acquire (`lifecycle_receipts.rs`). Recording itself has no allocation, serial output, locks or waiting. Its limits matter:

- `operation=1/2` means R2/R4 **scenario**, not independent invocation identity.
- The immutable PID/incarnation/VM tuple denotes the target subject. It is not three actor identities. The actual R4 signal actor is correlated through the separate existing acquisition fixture.
- The log has sweep observations, channel candidates, terminal stores, endpoint removals and one cleanup start/completion, but no independent sweep/cleanup claim and return intervals.
- `retire_process(pid)` entry receipts themselves match the context PID. Stronger generation provenance comes from exact channel/process bindings and surrounding serialized observations, not an invented generation argument on that hook.
- Post-drop receipts occur after real guard destruction; they do not instrument the mutex's internal unlock instruction.

The fresh R2/R4 evidence below is assessed under its existing contract. It is explicitly rejected as an R6 or nested-callback proof.

## R6 scenario disposition

| Scenario | Native result | Missing evidence |
|---|---|---|
| R6-A retirement-first | **BLOCKED before launch; NOT_RUN** | No safe independently owned sweep and endpoint-cleanup decision intervals while retirement is in flight |
| R6-B cleanup-first | **NOT_RUN** | R6-A gate closed |
| R6-C sweep-first | **NOT_RUN** | R6-A gate closed |
| R6-D mixed contention | **NOT_RUN** | No safe three-actor coordinator; inherited single-contender probe is insufficient |
| R6-E repeated cleanup | **NOT_RUN** | R6-A gate closed; ordinary exit/reaper repeat candidates are narrower controls |

There is no native R6 overlap witness. No winner, candidate, cleanup claim or actor is fabricated to fill these rows. First valid committed channel outcome remains the production contract; no numerical terminal-reason priority is introduced. R2/R4's existing exit and reaper hook repetitions remain their existing evidence, not R6-E.

## Nested exit-hook guard ownership

The final `EXIT_GUARD_ACQUIRED` receipt follows actual acquisition of the guard used to inspect the current process (`sched/mod.rs:4477–4480`); it is not emitted before acquisition. The scheduler guard owns the subsequent Finished/pending transition, synchronous Doorbell callback, owned-endpoint cleanup and final wake/state operations. Recording does not itself reenter or acquire the scheduler.

However, in combined/lifecycle fixtures, `finish_current_process` explicitly drops the channel acquisition guard before calling `bell_race::acquire` (`sched/mod.rs:4444–4448`). Even when the nested fixture hook is inactive, `bell_race::acquire` acquires SCHEDULER again before deciding actor zero. The real guard release/reacquisition gap exists in the debug fixture. A later final-guard receipt cannot prove uninterrupted ownership across it. This milestone does not change that handoff.

The branch emitting `EXIT_NESTED_REJECTED` runs **after** scheduler acquisition and pinning when the current process is already Finished/Reaped (`sched/mod.rs:4481–4512`). It handles a remote termination winning before self-exit; it is not a recursion-depth guard or a rejection before recursive mutex acquisition. A recursive call retaining the outer guard would attempt the same non-reentrant mutex first. There is no inspected reachable production callback that safely requests this recursive operation. Installing such a callback or raw recursion solely for proof would introduce unsupported reentrancy. The early-return branch also bypasses the normal final explicit post-drop receipt, so absence of that receipt would not alone prove a leaked Rust guard.

The real sweep callback takes the existing scheduler borrow and calls the exact participant classifier; it does not recursively call self-exit. Repeated process retirement through the reaper is serialized and uses the existing pending flag. The already-finished SIGKILL rejection in R4 is an independent losing retirement contender, not a nested callback. Neither substitutes for NG-1/2/4.

| Scenario | Native result | Qualification |
|---|---|---|
| NG-1 direct nested entry | **NOT_RUN** | R6-A gate closed; raw recursive mutex acquisition is not a safe rejection fixture |
| NG-2 reentry during Doorbell observation | **NOT_RUN** | Existing callback performs classification, not reentrant exit |
| NG-3 repeated cleanup after retirement | **NOT_RUN** | R2/R4 repeated hooks are retained scoped controls, not a nested-guard pilot |
| NG-4 callback plus remote retirement | **NOT_RUN** | R4 has a remote contender but no nested callback |
| NG-5 generation turnover | **NOT_RUN** | Source/host generation checks do not establish this native nested-guard scenario |

`RecipientIdentity` comparison requires PID/incarnation/VM before classifying exiting state. The reaped-owner receipt correlates the original VM only with exact PID/incarnation and Reaped/pending/stack conditions. Slot reuse follows complete reaping and pin/current-task checks (`sched/mod.rs:578–631`). These are relevant source/host protections; this milestone does not claim successor rejection or resource ownership survival for unexecuted R6/NG orderings.

## Terminal and resource ownership assessment

`Channel::retire` admits the first candidate only while the row is live, writes `terminal`, then exposes State::Retired (`channel.rs:120–145`). Later actual candidates are recorded and rejected without overwriting the winner. Broker serialization protects those mutations. Logical process Finished/pending, channel terminal store, endpoint broker removal, real IPC removal and final process Reaped are separate commits.

Reaping requires pending work and excludes pinned/current/queued/owned/active-VM resources. It removes remaining IPC references and endpoints, retires grants, tears down memory, removes the kernel stack, clears pending, stores the original VM receipt and exposes Reaped. That serialized path explains idempotence. It does not provide an independent per-resource cleanup claim ledger or PMM free count under three-operation contention. Exactly-once **trace-visible terminal publication and cleanup-path completion** can be established for fresh R2/R4; exactly-once physical resource disposal across R6-A–E remains **NOT_PROVEN**. No resource-loss or stale-authorization defect was demonstrated by the audit, and unexecuted scenarios remain unqualified.

## Guest controls, regressions and source integrity

The existing Account/SessionD runner remains unchanged. It launches a real Account-inherited shell, executes the original native clients and collects actual shell/client exit status (`kfp_security_runner.rs`). Kernel package cleaning precedes each guest to avoid stale embedded services. Each attempt retains separate build output, archived embedded binaries, ELF, extracted ISO kernel, ISO, serial log, source map, requested features, generation, CPU count and watchdog outcome. No payloads or guest output are generated to simulate missing lifecycle evidence.

The runner uses `key_inject,session_launch_test` plus the requested fixture: lifecycle trace for R2/R4, mapping for original M1/M2/M3, transfer for combined and M5, Doorbell for Doorbell-only and M4. Production defaults remain empty. R2/R4's actual client payload must contain its intended case, unique build ID and generation and must match the bytes embedded in the ELF. The unchanged M1–M5 payloads have archived content hashes and per-attempt image/build manifests; they do **not** embed distinct per-attempt build IDs/generations. That stronger integrity property is not claimed for legacy payloads.

`guest-integrity.json` independently rechecks manifest source maps, image and extracted-kernel hashes, the nine selected runner payload hashes/ELF offsets, serial hashes and lifecycle client build/generation strings. `source-integrity.json` verifies the isolated tested baseline, records live-source drift separately, and rehashes protected historical evidence. Optional unrelated embedded assets/services are not exhaustively independently source-qualified by this runner. Documentation and evidence scripts are outside the runner's runtime-source map and have separate artifact hashes.

<!-- EXECUTION_RESULTS -->
| Fresh two-CPU native control | Runner / independent assessment | Qualification |
|---|---|---|
| r2 | PASS; independent PASS | Watchdog: complete; not an R6/NG pilot |
| r4 | PASS; independent PASS | Watchdog: complete; not an R6/NG pilot |
| original | PASS; M3 PASS | Watchdog: complete; not an R6/NG pilot |
| combined | PARTIAL; M3 PASS | Watchdog: complete; not an R6/NG pilot |
| doorbell | PASS; M3 PASS | Watchdog: complete; not an R6/NG pilot |
| m4 | PASS | Watchdog: complete; not an R6/NG pilot |
| m5 | PASS | Watchdog: complete; not an R6/NG pilot |

Combined is **PARTIAL across attempts**: its selected source-verified baseline replay passed, while the original executed combined guest remains **FAIL / SOURCE_INTEGRITY_FAILURE**. Raw security markers from that rejected guest do not qualify it as source-bound evidence. The replay does not cancel this failure.

| Fresh host/build control | Result |
|---|---|
| production-check | PASS |
| kernel-host | PASS (212 tests) |
| ipc-host | PASS (94 tests) |
| doorbell-host | PASS (9 tests) |
| scheduler-host | PASS |
| m1-host | PASS |
| Process-exit/cleanup host subset | PASS; included in kernel-host, see `process-exit-host.json` |
| Receipt assessor host controls | PASS; 32 controls over fresh native logs |

Kernel host coverage compiles the actual broker and pure lifecycle logic through `mm2a_host.rs`, with hardware/process adapters. Its process-exit, grant-cleanup, stale-incarnation and resource-inventory tests are host controls; they do not execute the production scheduler reaper. Scheduler controls likewise retain their stated host adapter limits. The production check is compilation only; no production boot is inferred.

R2/R4 native trace counts (scope: these executions only):

- R2: build `r2-20261110T153905.927799Z`, generation `1794325146027014`, subject `40/85/98`; 1 terminal store(s), 1 cleanup completion(s), nested exit `NOT_OBSERVED`. Full candidates and rejected contenders: `r2-causal-trace.json`.

| Sequence | Native boundary | CPU | Outcome | State | Detail |
|---|---|---|---|---|---|
| 43 | EXIT_GUARD_ACQUIRED | 1 | 0 | 1 | 0 |
| 44 | EXIT_STARTED | 1 | 0 | 3 | 1 |
| 45 | SWEEP_OBSERVED | 1 | 1 | 3 | 1 |
| 46 | IDENTITY_VALIDATED | 1 | 2 | 3 | 1 |
| 48 | TERMINAL_COMMITTED | 1 | 2 | 3 | 0 |
| 49 | RETIREMENT_COMMITTED | 1 | 2 | 4 | 0 |
| 50 | ENDPOINT_CLOSE_ENTERED | 1 | 0 | 0 | 0 |
| 54 | ENDPOINT_BROKER_REMOVED | 1 | 0 | 0 | 33 |
| 55 | ENDPOINT_CLOSED | 1 | 0 | 0 | 33 |
| 56 | EXIT_GUARD_RELEASED | 1 | 0 | 0 | 0 |
| 89 | CLEANUP_STARTED | 0 | 0 | 3 | 1 |
| 95 | CLEANUP_COMPLETED | 0 | 0 | 4 | 0 |

- R4: build `r4-20261110T154222.861643Z`, generation `1794325342986019`, subject `40/85/98`; 1 terminal store(s), 1 cleanup completion(s), nested exit `NOT_OBSERVED`. Full candidates and rejected contenders: `r4-causal-trace.json`.

| Sequence | Native boundary | CPU | Outcome | State | Detail |
|---|---|---|---|---|---|
| 75 | RETIREMENT_CONTENDER_PROBED | 0 | 1 | 0 | 5 |
| 76 | EXIT_GUARD_ACQUIRED | 1 | 0 | 1 | 0 |
| 77 | EXIT_STARTED | 1 | 0 | 3 | 1 |
| 78 | SWEEP_OBSERVED | 1 | 1 | 2 | 1 |
| 79 | IDENTITY_VALIDATED | 1 | 2 | 2 | 1 |
| 81 | TERMINAL_COMMITTED | 1 | 2 | 2 | 0 |
| 82 | RETIREMENT_COMMITTED | 1 | 2 | 4 | 0 |
| 83 | ENDPOINT_CLOSE_ENTERED | 1 | 0 | 0 | 0 |
| 87 | ENDPOINT_BROKER_REMOVED | 1 | 0 | 0 | 33 |
| 88 | ENDPOINT_CLOSED | 1 | 0 | 0 | 33 |
| 89 | EXIT_GUARD_RELEASED | 1 | 0 | 0 | 0 |
| 96 | RETIREMENT_CONTENDER_ACQUIRED | 0 | 0 | 3 | 98 |
| 97 | RETIREMENT_CONTENDER_REJECTED | 0 | 1 | 3 | 98 |
| 98 | CLEANUP_STARTED | 1 | 0 | 3 | 1 |
| 104 | CLEANUP_COMPLETED | 1 | 0 | 4 | 0 |


Tested baseline integrity: **PASS**; independently rechecked accepted guest artifacts: **PASS**. All 3726 captured historical files rehashed; changed historical files: 0. No milestone runtime-source delta in the tested baseline.

The live workspace changed externally during the first combined attempt. That guest executed, completed its raw security checks, and was then rejected by the post-run source-integrity guard. Its original manifest and serial trace remain at `.kfp-2a6d-evidence/native/combined/m2-20261110T154626.364723Z/results.json`; its source-to-image identity is not qualified. The first four private-clone native builds and first production check were separately rejected before execution because an ignored `hello-linux.elf` input was missing. Its exact bytes were then copied and verified inside the accepted baseline R2 ELF (`ignored-build-inputs.json`). The next native builds exposed duplicate linker-script arguments: Cargo inherited both ancestor and nested configuration arrays. Final control commands supply the exact baseline kernel `RUSTFLAGS` once; existing explicit service flags still override them for services. No source or linker-script modification was made. Clone build failures remain pre-guest failures; the original combined failure is an executed source-rejected guest. Host tests retain their earlier baseline passes because the ignored binary and kernel-only link flags are not host-library inputs. The private clone's 1660 source inputs match the captured baseline exactly. No replay erases any failure. The final live tree has 8 changed incoming tracked files relative to baseline; those edits were preserved. **Final live-workspace validation: NOT_RUN.** See `external-source-drift.json`, `attempt-history.json`, `final-control-driver.json` and `source-integrity.json` for observed hashes, build environment and all rejected attempts.

Every execution retains its actual command, complete feature closure, build/image/payload hashes, real generation, CPU count, raw receipts and watchdog outcome. The legacy M1–M5 payload identity limitation above remains explicit. A successful guest watchdog completion is not an in-guest recovery or shutdown-cleanup receipt.
<!-- END_EXECUTION_RESULTS -->

## CPU matrix and final boundary

| CPUs | R6 / nested pilot status | Qualification |
|---|---|---|
| 1 | **NOT_RUN**, prerequisite closed | Physically simultaneous three-CPU execution is **NOT_APPLICABLE**. Logical overlapping invocations would still need validated suspension outside incompatible locks. |
| 2 | **BLOCKED / NOT_RUN** | Fresh R2/R4/security controls are distinct from an R6 or NG pilot. |
| 4 | **NOT_RUN** | Two-CPU R6 and nested pilots have not passed. |
| 12 | **NOT_RUN** | Same prerequisite; no source/host substitute for native SMP generation evidence. |

**BLOCKED** is required because native three-operation overlap, all required orderings, nested guard continuity, contention-qualified exactly-once resource ownership, stronger legacy payload identity and the CPU matrix are incomplete. This is not a security FAIL: no confirmed invariant violation was established. No new kernel lock, wait, interrupt manipulation, preemption control, affinity policy or production reentrancy was introduced. Production runtime behavior did not change as a result of this milestone. External live-workspace runtime edits remain unvalidated. FastLane remains disabled; no commit or push.

Recommended next milestone: **KFP-2A.6E — ownership-safe lifecycle continuation feasibility**. First establish a real, bounded, outside-lock continuation boundary with generation revalidation and explicit actor/invocation/cleanup ownership, or demonstrate that the requested orderings are incompatible with the existing serialized contract and revise the verification contract explicitly. Separately preserve the actual scheduler guard through inactive nested fixture hooks and instrument acquired ownership across that path. Do not infer a production correctness defect from the fixture gap. Proceed to a fresh two-CPU R6-A only after the continuation and lock-safety prerequisites are validated; only its independently verified overlap may open R6-B–E, NG and the CPU matrix.
