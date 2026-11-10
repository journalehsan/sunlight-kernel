# KFP-2A.6A — M1 acquisition-boundary coordination

**Verdict: PARTIAL. Production runtime behavior did not change. Production FastLane remains disabled. No commit or push.**

The fresh original, combined-feature and Doorbell-only two-CPU M1/M2/M3 guests pass their unchanged security assessors. All twelve existing M1 mode/configuration pairs obtain an actual failed SCHEDULER `try_lock` on their first armed generation. The fixture no longer relies on rescheduling the contender into the holder's short critical section. This establishes bounded acquisition-boundary contention, not full native R1–R6 closure. Independent unlock receipts, native lifecycle-boundary receipts and independently measured overlapping cleanup remain missing. Missing evidence is not converted into PASS.

Machine-readable evidence is under `.kfp-2a6a-evidence/`, particularly `results.json`, `race-witnesses.json`, `baseline.json`, `source-integrity.json`, `locking-review.json`, and each guest's `results.json`. Final guest/regression status is recorded there; the tables below distinguish the original assessors from this milestone's stronger acceptance contract.

## Source and artifact integrity

Baseline revision: `127aee1d938781877264f6d0e13f67c3fcd89327`. The incoming tree was **dirty**, including `.version_state.json`, Cargo.lock and many package manifests; the untracked KFP-2A.6 report also existed. `status-before.txt` and `preexisting.patch` preserve that state. The required KFP-2A.5 and KFP-2A.6 reports and KFP-2A.6 results were read; their hashes are in `baseline.json`. Historical failures were neither edited nor deleted.

The runtime source delta is confined to the already debug-feature-gated `kernel/src/capability/channel_race.rs`. `channel_race.before.rs` preserves its incoming bytes. `source-integrity.json` compares every captured source/build input against baseline and verifies historical hashes. Its final manifest digest identifies the tested source domain; documentation and dot-evidence files are outside that domain.

Each guest has a separate ISO, ELF, archived payload directory, timestamped attempt identity, commands, source maps, serial hash and watchdog receipt. The runner verifies payload bytes inside the ELF, extracts the ISO kernel and compares it with the ELF, and checks source integrity after building and executing. Each configuration first cleans the kernel package to prevent the previously observed embedded-service cache mismatch. No historical image substitutes for a fresh run.

**Payload qualification:** the three M1 configurations independently archive and verify the same unchanged M1/M2/M3 clients. Their `uac_service` content hash is identical (`5fab6e942f68cb1b9f9197ee23f45eee847f759898b6d2f46a761540171f0024`). The guest image hashes differ. These are independent attempt/artifact identities, **not three unique payload contents or embedded per-attempt build IDs**. The stronger requested unique-payload identity property remains unfulfilled. M4/M5 use their own suite payloads and image manifests.

Requested features are `key_inject,session_launch_test` plus `fastlane_mapping_test` (original), `fastlane_transfer_test` (combined), or `fastlane_doorbell_test` (Doorbell-only). Mapping adds channel and native-session-race fixtures; Doorbell additionally adds the Doorbell dependency/fixture; transfer additionally adds its transfer feature. `feature-sets.json` records the full closures. The M1/M2/M3 UAC requests `kfp-security-runner,fastlane-mapping-test`; M4 and M5 request their respective suite features. Defaults remain empty.

## Why lock-missed occurred

The original sequence was:

1. Both authenticated actors entered, recorded exact PID/incarnation/VM identities under SCHEDULER, dropped the guard and rendezvoused through yielding waits.
2. The contender published READY, then used the yielding `wait(..., "held", ...)`.
3. `prepare_yield` acquired SCHEDULER, retained the exact private continuation and requested scheduling through the reschedule vector.
4. The winner observed READY, acquired SCHEDULER, published HELD and performed its normal operation without a peer wait.
5. Scheduler progress needed to resume the contender could occur only after the holder released SCHEDULER.
6. The late contender observed the historical HELD flag, successfully probed the now-free mutex and emitted `lock-missed`.

READY certified an earlier arrival, not an independently executing contender at the acquisition boundary. Two virtual CPUs did not make the yielding continuation independently runnable while SCHEDULER was held. Sixteen retries were a bounded opportunity to sample natural contention, never proof of it. The prior failures retain their original status.

## Scheduler and interrupt constraints

SCHEDULER is one global `spin::Mutex<Scheduler>`. The mutex itself does not disable interrupts or implement a preemption guard. These syscall continuations already have IF clear, as the existing private-stack assertions and fresh coordination receipts establish. Local maskable interrupt preemption cannot dispatch them at this boundary; scheduling and `prepare_yield` also need SCHEDULER. A different CPU can execute an already-running continuation and issue a nonblocking probe.

The contender previously required scheduler progress to reach the probe after yielding. The new contender does not. A one-CPU holder with IF clear cannot simultaneously execute another ordinary thread at this boundary. The existing assessor explicitly requires distinct CPUs; it was not relaxed. A native single-CPU sequential-interleaving fixture is still needed.

The BSP alone advances `monotonic_ms`. A deadline based solely on that clock is inadequate if the BSP is held in this IF-clear section. The new boundary wait therefore has a finite instruction-iteration bound independent of timer interrupts.

## Test-only coordination and forward progress

After the final yielding entry wait, the contender asserts IF clear, publishes its **current** READY_CPU and then release-publishes READY. From that point until acknowledgement, it executes only:

`atomic HELD polling → SCHEDULER.try_lock → atomic sequence/probe/ack publication`.

It does not yield, acquire a blocking lock, allocate, access CAP_BROKER, log, await an interrupt, or require a wakeup. IF clear prevents migration between READY and acknowledgement. The winner waits for READY **outside** SCHEDULER, then acquires the real mutex, validates its exact current identity and verifies READY_CPU differs from its own CPU. It records lock-held before release-publishing HELD. It retains that guard until it observes the failed-probe acknowledgement.

The contender publishes its sequence and acknowledgement **before serial output**. Therefore the holder's peer wait does not depend on the serial lock. The winner's lock-held output finishes before HELD becomes visible. No scheduler-dependent peer continuation is needed while the winner waits under SCHEDULER.

Both atomic boundary waits terminate after at most 100,000,000 predicate/pause iterations. Exhaustion panics with `COORDINATION_BLOCKED`; it does not execute a terminal operation or create a witness. This count is a failure bound, not a contention proof, retry-based success criterion or guaranteed wall-clock latency. Host vCPU starvation can still cause bounded coordination failure. The outer QEMU watchdog bounds guest execution and terminates/reaps QEMU; the guest is discarded after failure. No post-panic in-guest recovery is claimed.

There is no new production scheduler policy, affinity assignment, lock implementation, capability authority, terminal priority or cleanup behavior. This module and its callers require debug assertions and `fastlane_channel_test`. The changed timing is confined to explicitly armed authenticated test actors.

Existing yielding arrival/unlock waits retain their private-stack checks and original bounds. Their fairness and global scheduling latency are not newly proved. The new polling wait can delay interrupts and other scheduler users for its finite budget; the source-extracted absent-peer test exercises exhaustion. It introduces no dependency cycle on the contender's scheduler dispatch. No lost-wakeup protocol exists: the handshake uses acquire/release atomics.

An inherited userspace comment describing the old natural sampling remains historical commentary; the code now intentionally prolongs the **test-only** holder until a scheduler-independent nonblocking probe acknowledges. The sixteen-attempt security assertion is unchanged, but each fresh tested configuration needs only its first attempt per mode.

## Contention receipts and their limits

The real probe is statically bound to the same global SCHEDULER instance whose live guard is retained by the winner. Authentication and arm eligibility still select exact immutable identities and current handle generation under the ordinary locks. `observed` remains exact-creator authorized. The sequence is assigned at the failed acquisition boundary, before acknowledgement. No HELD-only or late successful probe sets PROBE or CONTENDED.

All three fresh configurations record these boot-local sequences:

| Existing M1 mode | Channel generation | Held < failed probe < winner returned | Final terminal |
|---|---|---|---|
| 1 attach before revoke | 5 | 4 < 5 < 6 | Revoked=1 |
| 2 revoke before attach | 6 | 13 < 14 < 15 | Revoked=1 |
| 3 attach before exit | 7 | 22 < 23 < 24 | ProcessRetired=2 |
| 4 exit before attach | 8 | 31 < 32 < 33 | ProcessRetired=2 |

`race-witnesses.json` retains both exact participant tuples, CPU placements, all events, authority bindings, terminal observations and coordination traces. CPUs 0 and 1 participate. Successful eventual contender return through the guarded source path implies the winner released SCHEDULER. **There is no independently sequenced guard-drop event**, so the stronger complete witness contract remains partial.

A further inherited combined-feature limitation is that `finish_current_process` drops the channel hook's guard before entering `bell_race::acquire`. All fresh outcomes pass, but the gap must not be described as uninterrupted exclusion from the channel boundary through logical exit. It needs explicit measurement or a test-only guard-preservation design before stronger lifecycle-order claims.

## R1–R6 disposition

First committed terminal outcome wins. This milestone does not change that arbitration or force ProcessRetired over an earlier valid outcome. The prior Doorbell exact-exiting correction remains intact: match the complete participant tuple first, then classify exact exiting state as ProcessRetired. A genuine successor tuple is IdentityChanged; no successor identity is adopted.

| Required ordering | Linearization points | Evidence and result |
|---|---|---|
| R1 retirement wins | ProcessRetired terminal store before competing sweep classification | Source-extracted method control PASS; independent native boundary witness NOT_PROVEN |
| R2 exact exiting participant | Finished/pending exit mark before exact sweep, sweep before later retirement hook | Source-extracted classifier/control PASS; guest modes 3/4 end ProcessRetired, but exact early-sweep receipt NOT_PROVEN |
| R3 sweep precedes exit | Live exact sweep before exit mark, then retirement publication | Source-extracted ordered control PASS; independently coordinated native receipt NOT_PROVEN |
| R4 endpoint closure wins | Actual endpoint revocation/recipient retirement before process retirement | NOT_PROVEN. Host case is explicitly a **Revoked adapter**, not actual endpoint closure. There is no EndpointClosed channel terminal enum; owned endpoint teardown publishes ProcessRetired, while IPC peer-close is a separate outcome. |
| R5 generation mismatch | Old immutable tuple compared against actual successor before terminal publication | Host incarnation/VM/absence controls PASS; native successor-generation boundary NOT_PROVEN |
| R6 concurrent cleanup | Independent arrivals, one terminal store, idempotent endpoint/Doorbell/process cleanup | Repeated source-method arbitration control PASS; actual native overlap, terminal-store count and cleanup-once receipts NOT_PROVEN |

The scheduler serializes lifecycle mutations under its guard. Host ordered adapters are useful controls but do not prove independent native arrivals, native authentication or physical cleanup overlap. No MAP_FIXED or lifetime-accounting work was added.

## Host controls and security validation

`witness_controls.py` has fourteen synthetic receipt controls, including HELD-only, missing witness/release/completion, wrong lock, wrong/current generation, wrong exact identity, duplicate events and late acquisition. A late successful probe never classifies as contention; a purported failed probe sequenced after release is incorrect. The strict receipt validator is a host control and is **not substituted for the native assessor or an independent unlock instrument**.

`boundary_controls.py` extracts the actual new wait helper and runs it with host atomic/mutex adapters: a failed nonblocking probe while the holder owns the mutex, holder release, contender completion, late-success rejection, and absent-peer finite failure. The initial harness compilation failed because placeholder replacement also entered a diagnostic string. `boundary-attempt-1/` preserves that failure. The corrected harness passes. These controls do not establish native scheduling guarantees.

The source-extracted exact classifier, retire_unavailable and retire controls pass with their original adapter limitations. Fresh host regressions pass: kernel library 208 tests, IPC 94, Doorbell 9, scheduler host controls, and default production kernel check. Capability identity, process retirement, cleanup and terminal arbitration controls are included in the kernel tests; they are not counted twice. Existing sunlight-net dependency warnings remain.

| Fresh two-CPU guest | Original assessor | Milestone qualification |
|---|---|---|
| Original M1/M2/M3 | PASS | Four first-generation real contention witnesses; R1–R6 incomplete |
| Combined M1/M2/M3 | PASS | Four first-generation real contention witnesses; exit hook gap not independently measured |
| Doorbell-only M1/M2/M3 | PASS | Prior coverage exhaustion avoided in this fresh pilot; R1–R6 incomplete |
| M4 | PASS | Separate suite payload and assessor |
| M5 | PASS | Separate suite payload and assessor |

## CPU matrix and final boundary

| CPUs | Result |
|---|---|
| 1 | Native NOT_RUN. Simultaneous ordinary-thread contention at this IF-clear boundary is architecturally unavailable; sequential source-extracted controls pass only in host adapters. |
| 2 | Existing M1 contention/security pilots PASS in all three configurations. Full required R1–R6 pilot PARTIAL. |
| 4 | NOT_RUN; full required native two-CPU lifecycle pilot incomplete. |
| 12 | NOT_RUN; same prerequisite. |

**PARTIAL** is required despite twelve real failed probes. The missing native R1–R6 witnesses, independent unlock event, unique per-attempt embedded payload identity and CPU matrix prevent COMPLETE. No confirmed security invariant violation was observed. The current evidence is not a proof of universal fairness, host vCPU progress, maximum interrupt-disabled latency or exactly-once physical cleanup.

Recommended next milestone: finish **KFP-2A.6A lifecycle receipts and unlock closure**. Add test-only identity/generation-qualified observations at actual exit, sweep classification, endpoint recipient retirement, first terminal store and cleanup boundaries; preserve the actual guard through nested inactive fixture hooks; add an independently sequenced unlock observer and per-attempt payload build identity. Establish native R1–R6 and an applicable one-CPU sequential fixture before expanding to 4/12 CPUs. Production FastLane stays disabled.
