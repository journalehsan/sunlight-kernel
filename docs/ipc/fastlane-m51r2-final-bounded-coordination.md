# M5.1R2 final bounded coordination evidence

2026-11-03. **COMPLETE within the fixed diagnostic scope.** Demonstrated mechanism
and final repair are COMPLETE; original historical attribution is UNRESOLVED.
Production FastLane remains disabled.
No transfer functionality, performance work, scheduler policy change, desktop/audio
change, commit or push is included.

Evidence is retained in `.fastlane-evidence/m51r2/`, outside Cargo's target.
Final source-bound receipts are under `accepted/`. Earlier attempts retain their
original paths, manifests, logs and image identities. The directory name alone
never establishes acceptance; inspect each original receipt and the aggregate.

## Historical recovery and attribution

The fixed recovery search inspected the three documented M3.1B attempt paths,
2,290 retained log/report/manifest files under M5.1, M5.1R, M5R legacy and the
known M5R state directory, and the retained member index of the already verified
M5.1 archive. No files were excluded for size. The 36,189-member retained index
contains none of the historical attempt names. One final direct read-only scan
of all 52,860 archive member names also found no original attempt-name matches.
It completed in 187.60 seconds within a fixed 240-second cap, with no extraction;
the compressed archive SHA-256 matches its earlier exhaustive verification.
`archive-member-scan.json` retains the new scan and exact hash. The earlier
payload verification is not relabeled as a new per-member payload verification. No filesystem-wide undelete, snapshot or backup hunt was
attempted. `historical-recovery.json` records exact scope, seven matching retained
files, hashes, matches and absent paths.

What remains is documentary evidence of the traced consumer-only witness,
a separate untraced 2-CPU panic, an aborted orchestration with no guest result,
retained pre-correction source, and later controlled/final guests. The earlier
controlled serial is a later guest, not the historical serial. These observations
are kept separate.

The original failed image, full serial, exact executable/service identity,
producer affinity/queue/owner, IPC-ready/dispatch history, CPU 0 execution,
observed interrupt states and saved/resumed continuations cannot be reconstructed.
The historical `held=0` excludes a selected lifecycle winner at that rendezvous;
it cannot establish every other actor's lock ownership. MAX in the second actor
CPU field means **no entry observed**, not a CPU identifier.

**C — Historical causal attribution: UNRESOLVED.** The demonstrated same-CPU,
IF-clear run-slot stall explains how the documented witness can arise. It does
not establish that the historical producer had the controlled placement, or
that the M5R interrupt-state defect contributed to that old guest.

## Bounded controls

`tools/fastlane_channel_validate.py --coord-control` explicitly selects
`spin-same`, `spin-unpinned`, `corrected-same` or `corrected-unpinned`. All controls
imply the existing debug-only M1 gate, use the ordinary Account authentication,
independent native actors, full handle/identity binding and actual channel
operations. They introduce no alternate authority path.

The spin control reconstructs only the relevant **mode-2 entry rendezvous** from
the archived pre-correction source: atomic predicate, IF-clear busy wait,
100,000,000-iteration cap and five-second monotonic deadline. Later lock/contention
phases retain final coordination. This is a diagnostic isolation of the entry
mechanism, not an exact replay of the deleted guest or the entire old fixture.
The old complete fixture control remains preserved under M5.1R.

Immediately after successful ordinary mode-2 arming, both exact actors receive
mask 2 for the artificial same-CPU control, or MAX for unrestricted scheduling.
The latter includes consumer eligibility on every online CPU. Placement uses the
normal park/dispatch path; no actor is made Ready or dispatched by a diagnostic
shortcut. Other modes keep the established fixture configuration.

Coherent entry/arm snapshots record both identities, state, masks, queue, owner
and actual queue occurrence count under the existing scheduler guard. The
consumer's reply-before-attach makes the producer Ready; the entry snapshot
observes that actual scheduler state. Producer rendezvous entry independently
proves its dispatch and continuation with the logged CPU/IF. Three fixed atomics
record aggregate CPU 0 dispatch count and its last selected PID/dispatcher IF.
These count dispatch-site selections, including possible same-task selections,
rather than distinct completed context switches. Scalars sampled outside the
scheduler guard are not an atomic continuation-frame snapshot.
Spin start/completion/deadline samples record progress and current observer IF.
A deadline snapshot uses one nonblocking try-lock and explicitly reports an
unavailable snapshot. No guard crosses a spin iteration or scheduler yield.
`broker_owned=0` describes the observer's guard ownership, not a global claim.

Instrumentation is test-only and bounded by four fixture orders, sixteen attempts
per order and fixed entry/deadline observations; there is no per-iteration output,
heap allocation, trace queue or new global lock. Instrumentation can perturb dispatch timing. Aggregate CPU 0 samples are not
an exhaustive interrupt/dispatch history, and dispatcher IF is not a sampled
idle/userspace IF. Missing historical observations remain missing.

## Final pre-correction and placement results

`accepted/control-assessment.json` joins the twenty final-source diagnostic guests
to original native status, manifest/serial hashes, entries, snapshots and bounds.
An expected diagnostic failure remains a native FAIL. No failed guest is replaced
by a later passing guest.

| Scenario | CPUs | Guests | Both runnable | Both enter mode 2 | Scheduler / bound outcome |
|---|---:|---:|---|---|---|
| Old entry spin, both masks 2 | 2 | 1 | Yes | No, consumer only | CPU 0 dispatch count 3 → 3,386; CPU 1 consumer IF=0; five-second deadline |
| Old entry spin, both masks MAX | 2 | 3 | Yes | 2/3 guests | Two complete; one CPU 0 consumer IF=0 with producer Ready/queued on CPU 0 reaches 100M-iteration bound |
| Old entry spin, both masks MAX | 4 | 3 | Yes | 3/3 | Complete, no entry deadline/exhaustion |
| Old entry spin, both masks MAX | 12 | 3 | Yes | 3/3 | Complete, no entry deadline/exhaustion |
| Corrected, both masks 2 | 2 | 1 | Yes | Rejected before wait | Deterministic `both actors require one CPU` assertion; no entry timeout |
| Corrected, both masks MAX | 2 / 4 / 12 | 3 each | Yes | 9/9 | All complete through normal scheduling; no deadline/exhaustion |

The controlled same-CPU guest records the exact historical witness shape:
`entered=1 cpus=(1,MAX) mode=2 held=0 contended=0 finished=0`, with the producer
Ready, owner 255, queue 1, exactly one queue occurrence and mask 2. The consumer
owns neither scheduler nor broker guard throughout its wait. Its deadline is
24,280 ms after a 19,280 ms start. CPU 0 progress excludes a machine-wide scheduler
halt in this guest. A nonblocking final scheduler snapshot is unavailable and is
reported as such; the deadline's CPU/IF and progress counters are still recorded.

The unrestricted 2-CPU iteration failure has the same **local** IF-clear run-slot
condition but does not prove the sole-eligible-CPU mechanism: another CPU is
eligible for the producer, and its final dispatch/lock state is unobserved.
Thus unrestricted placement does not guarantee progress for the old spin entry,
and neither its passes nor this new failure establish the historical placement.
The corrected equivalent unpinned controls all complete. Serial output can
change timing; these are observations of the instrumented controls.

**A — Demonstrated pre-correction entry failure mechanism: COMPLETE.** The old
entry wait can monopolize an IF-clear run slot while its peer is Ready; the
same-CPU case demonstrates why that peer cannot run. This statement is separate
from the exact cause of either the new unrestricted failure or the historical
incident. The M5R saved-IF repair is present in all current controls.

## Final source and security review

The final executable/configuration source map has 1,369 files and canonical
SHA-256 `2c3bc9b0880e343462b1d6f9c147c3a0a4472754b04ac19794436b4f4e84abf4`.
The base revision remains `04ac8dfac5b675d64c9066be39dffe8b1f6b7e2b` with the
existing M5.1R changes plus five diagnostic/harness files. Every native receipt
joins its complete source map, immutable image/service hashes and original serial.
`accepted/source-audit.json` compares against the final M5.1R source, retaining
exact five-file diffs and unchanged critical boundaries.

| Boundary | Review |
|---|---|
| M1 authority | Handle/token/generation, role, session/epoch, issuer, incarnation/VM, expiry and terminal admission unchanged; controls act only after exact-creator arm succeeds |
| M2 / reclamation | Mapping rights, backing ownership, NX, alias restrictions, TLB retirement and memory implementations byte-identical to M5.1R |
| Integrity | Memory safety and private snapshot stability remain provided; coherent publication and application authenticity remain unprovided |
| M4 / M5 / M5.1 | Doorbell, single-message and repeated transfer implementations byte-identical; no new transfer operation |
| M5R / scheduler | Interrupt assembly and saved-IF repair byte-identical; existing dispatch sites have only cfg-gated scalar observations, with no queue/selection/wake policy changes |
| Corrected coordination | Default retains private-stack F3 yields, exact identity/IF assertions, no peer wait with a lifecycle guard, real failed try-lock contention and bounded missed-window receipts |
| Locks / lifetime | No new global lock, unsafe pointer or allocation; diagnostic snapshots use existing guards transiently and retain only scalar values |
| Production | Diagnostic features imply the existing debug-only M1 feature; normal fixture/production do not enable them; independent ordinary image and release exclusion checks PASS |

## Preservation

The initial Cargo manifest placement error, sandbox QMP socket denials,
interrupted drivers and source-freeze whitespace correction remain preserved.
The sandbox driver had no booted guest; its interrupted matrix write is retained
verbatim alongside an interruption receipt. The final driver serializes atomic
manifest writes. These infrastructure failures are not native mechanism evidence.
The unpinned spin control also retains a 2-CPU iteration-exhaustion failure:
the consumer entered on CPU 0 with IF clear while the unrestricted producer
was Ready in CPU 0's queue. Its start snapshots show `ms=20950`; there is no
final time or scheduler snapshot at the iteration cap. The source restricts
global clock advancement to CPU 0 timer interrupts, so an IF-clear CPU 0 spin
can prevent the five-second clock deadline from advancing. This is a
source-backed explanation, not an observed final clock value or proof that an
eligible other CPU could never dispatch the producer. The native panic is
`channel race exhausted`, distinct from `channel race deadline`. Neither is
reclassified as a passing regression.

Expected diagnostic guest failures remain original FAIL receipts; an independent
mechanism classification never changes them into accepted native regressions.

## Corrected fixture matrix

**B — Correctness of the final coordination repair: COMPLETE.** The default
fixture passes nine independent final-source native guests, three on each topology.
`accepted/coordination-assessment.json` independently checks original authority
acceptance, every exact actor binding/result/terminal outcome, both winner orders,
real contention before winner return, bounded misses, all wait phases, deadlines,
IF assertions, absence of held guards across yields and complete trace accounting.

| CPUs | Guests passed | Fresh pair attempts | Qualified races | Misses | Actor 1 / 2 wins | Scheduler yields | Rendezvous timeouts |
|---|---:|---:|---:|---:|---:|---:|---:|
| 2 | 3/3 | 22 | 12 | 10 | 6 / 6 | 154 | 0 |
| 4 | 3/3 | 17 | 12 | 5 | 6 / 6 | 119 | 0 |
| 12 | 3/3 | 29 | 12 | 17 | 6 / 6 | 263 | 0 |

Identities are scoped to each boot/attempt before joining generation and
PID/incarnation/VM; equal numeric IDs in different guests are not the same actor.
All 68 actual syscall pairs and 136 operation results remain joined to their
exact source/actor/channel identities. The 32 missed windows complete their normal
lifecycle checks but receive no concurrent coverage credit. All four orders qualify
once per guest. All 536 resumed scheduler continuations assert IF=0; 330 additional
yield pairs beyond the first sample in each wait are explicitly unsampled. Trace
overflow and missing-record counts are zero. The same-CPU corrected negative
control rejects impossible placement deterministically, before waiting.

Build manifest:
`accepted/m1/build-20261103T062353.276434Z/validation-summary.json`.
ELF SHA-256: `a9b603f23a4a33048f965fb1b206f49dd126f398e92fde412007ce142ff5df63`.
ISO SHA-256: `904587fcaf6196432f13d437dd38ee54ae589679655a1e201949fddb699a99dc`.
All eight embedded service binaries are byte-identical to the final M5.1R M1
build. The overall kernel/image hashes differ; no binary identity with the prior
milestone is asserted. New passes are not substituted into the old matrix.

## Exact regression matrix

All **51 regression guests PASS** on the same final source map. They comprise
nine default M1 repetitions, nine M4/M5/M5.1 guests and 33 broader guests. The
separate twenty diagnostics comprise seventeen native PASS receipts and three
retained native FAIL receipts: same-CPU spin deadline, unrestricted spin iteration
exhaustion, and corrected impossible-placement rejection. Diagnostic assessment
PASS means the fixed controls completed and their outcomes were classified;
it does not convert those three failures into native acceptance.

| Native regression | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Default M1 authority/contention repetitions | PASS 3/3 | PASS 3/3 | PASS 3/3 |
| M1/M2/M3 compatibility, mapping/retirement and native word probes | PASS | PASS | PASS |
| M4 authenticated Doorbell | PASS | PASS | PASS |
| M5 single-message mediated transfer | PASS | PASS | PASS |
| M5.1 repeated mediated transfer | PASS | PASS | PASS |
| IPC deferred/reply race, VM and SHM retirement | PASS | PASS | PASS |
| Account authority contention S1 and S2 | PASS 2/2 | PASS 2/2 | PASS 2/2 |
| Successful/failed exec, three cycles | PASS | PASS | PASS |
| Actual exit modes 11, 12 and 13, three cycles each | PASS 3/3 | PASS 3/3 | PASS 3/3 |
| Scheduler lifecycle | PASS | PASS | PASS |
| R2D remote wake / physical handoff | PASS | PASS | PASS |
| Existing kernel-native Doorbell mechanism | PASS | PASS | PASS |

`accepted/exact-native-matrix.json` enumerates all 71 final-source guests with
exact original manifest paths/hashes, CPU counts, QEMU commands, serial hashes
and image provenance. Nested M3/mapping receipts for the same serial are joined
as one guest, not counted twice. Older Account receipts retain their own
FORCED_AND_PASSED, terminal-protocol and host-ACK schema rather than an invented
original scalar status. Direct scheduler receipts inherit the frozen broad-driver
source map and carry their own image hashes and commands; they are not claimed
to contain a separate full source map. `accepted/matrix.json` and the retained
broad `orchestration.json` both PASS, with no failed regression substitution.

All 252 FastLane parser controls PASS. The established M5.1 host checks and
broader mapping/authority/scheduler checks PASS. No Miri execution was added;
its previously unavailable / NOT_RUN status remains independent.

## Production exclusion and final security outcome

`accepted/production-exclusion-audit.json` independently verifies the ordinary
feature list is empty, the ordinary debug ELF equals the ISO-extracted ELF, and
both ordinary debug and full release ELFs contain the exact final normal service
bytes. Experimental authority, coordination, diagnostic-control, mapping,
Doorbell and transfer markers are absent from both images. The ordinary debug
symbol audit additionally excludes the coordination/yield/diagnostic helpers,
counters and experimental adapters. Symbol inspection of the stripped release
ELF is NOT_RUN; it is not reported as a successful symbol inspection.

The supported ordinary release **build PASS** is separate from the deliberately
rejected release check with `fastlane_coord_spin,fastlane_coord_same_cpu`.
That original command remains exit 101 with the required debug-only M1 diagnostic;
the independent release exclusion gate PASS does not rewrite its command result.
The archived old full control and current interrupt assembly have identical hashes,
confirming the M5R saved-IF repair remains present in the entry-spin experiment.

No new production correctness or security defect was established. Current
admission, lifecycle serialization, restricted mapping, reclamation, private-copy
integrity boundaries and native transfer regressions pass; production enablement
remains disabled. Historical M4 failures, historical global SHM census drift,
previously unavailable Miri and prior D3/N3M/service/global-reconciliation security
debt are not discharged by this milestone.

## Readiness decision and stop conditions

**Recommend beginning a separately gated experimental M5.2 slice.** The current
security invariants and native transfer regressions pass. Irrecoverable historical
M1 attribution remains explicitly UNRESOLVED and does not require another
diagnostic milestone before that gated experiment.

This recommendation is not production readiness: production FastLane remains
DISABLED and production enablement remains BLOCKED by its independent prerequisites.
No M5.2 functionality or production enablement is implemented here.

The investigation stops here: the bounded old-entry controls are complete, the
corrected fixture passes the required repeated 2/4/12-CPU matrix, available known
historical evidence is exhausted, and the preservation regressions are complete.
No additional diagnostic milestone is proposed to force historical attribution.
The machine aggregate is `.fastlane-evidence/m51r2/validation-summary.json`;
all original failures and exact source identities remain outside Cargo target.
No commit or push was made.
