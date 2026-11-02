# M5R native Doorbell regression closure

**M5R PARTIAL**, 2026-11-02. Production FastLane remains disabled.

## Evidence loss during disk exhaustion

The development filesystem filled while the first corrected diagnostic build
was running. The user ran `cargo clean` to recover space. That deleted the entire
`target/` tree, including all original M4/M5 evidence, M5R's integrity receipt,
unchanged-image replay and first IF diagnostic control. The descriptions below
record observations made before deletion, **not currently retained artifacts**.
The conversation tool outputs contain those observations; no replacement log or
manifest will be fabricated. Independent exact-image re-audit is now BLOCKED
unless an external backup is recovered. No backup was found in the checked
project/tmp paths. M5R cannot be COMPLETE on this evidence basis.

New evidence is stored in `/home/ehsantor/.local/state/sunlightos/m5r/`, outside
Cargo targets. `/tmp/m5r_guard.py` records command/source identity and monitors
free space while commands run, stopping new work below a 12-GiB reserve.
The cleanup utility `~/.local/bin/manjaro-clean-safe` defaults to preview and
excludes projects, build/evidence trees and Cargo registries. Cleanup was not
applied by this task.

## Historical reconstruction (observed before deletion)

`target/fastlane-m5r-evidence/historical-integrity.json` previously recorded independent
SHA-256 verification of all 4,129 indexed artifacts in the original M4 image,
first native attempt and diagnostic replay. All indexed hashes matched at that inspection. The subsequent cleanup
deleted these artifacts. All three executable source maps match the starting tree
exactly, despite the later repository HEAD `228bb82c85e58eace0f6df21bba7a6736097eb17`.
The recorded historical revision is `91a28f22aaa17d3e365398cc31f7d7db662e54e5`.

Both failed guests used:

- ELF SHA-256 `87b4c53ce33dca68e5e1c376d49fe49b8c9cd3d4af52a781b1c86219281454d9`.
- ISO SHA-256 `6c5dd39a4241ec37c54955e4135af652a836a31d685b862b510ef41004fcde37`.
- Features `fastlane_doorbell_test,key_inject`, QEMU TCG multi-thread, CPU max,
  four CPUs, 2048 MiB, no NIC, virtio RNG, host deadline 180 seconds.
- Producer PID 40/incarnation 52/VM 89 on CPU 0; consumer 41/53/91 on CPU 1.
  These bindings are recorded by authenticated kernel events, not inferred from
  the unavailable early page-fault PID.

| Original attempt | Fixture / expected outcome | Last observations | Missing observations |
|---|---|---|---|
| `m4/native-20261102T122333.183105Z` | generation 9 producer-exit-wait; consumer collects Terminal (4), controller receives DONE, cleanup zero | Consumer registered and physically parked (BlockedOnIo, owner/queue 255, zero pins/ready occurrences); actual producer exit commits Terminal at CPU 0; producer physically reaped on CPU 1; fetch #PF RIP 0, RSP `0x7fffffffd3c0`, error `0x10` | Fault PID is `u64::MAX`; fault CS/CR3, resumed frame, consumer dispatch and collection absent; retirement was initiated but consumer receipt/terminal inventory not collected |
| `m4-replay4/native-20261102T122843.668828Z` | generation 2 registration-race mode 1; notify sequence 1 and consumer collects success | Both real actors entered on CPUs 1/0, registration lock held/contended; consumer registered; producer committed sequence 1 and both race operations returned; channel state 3 Attached, terminal 0, both roles attached | No physical park required for this forced pre-park race; no saved context/dispatch/ICR/consumer collection; host deadline expired without a fault or guest assertion |

Paths in the table are relative to `target/fastlane-m5-evidence/`.
These are two FAIL observations. Neither completion proves subsequent userspace
execution. Original logs do not distinguish delayed dispatch, continuation
corruption or a later coordination stall. A successful replay cannot supply
missing observations in an earlier guest.

## Investigation

Before the disk incident, an identical-image, unchanged-source four-CPU replay passed in
`target/fastlane-m5r-evidence/original-replay/native-20261102T130348.261580Z/`.
This preserves the two original FAIL classifications.

Source inspection identified an interrupt-state invariant violation candidate:
`reschedule_entry` unconditionally ORs IF into the outgoing saved RFLAGS,
including a CPL0 software interrupt from the shared kernel-stack trampoline.
The trampoline later restores its original syscall continuation on the process's
private userspace stack. Such a continuation requires IF clear until SYSRET.
A bounded opt-in resume observation checks that invariant before restoring the
private stack. Native control evidence and final conclusions follow below.

## Independent debt

Historical global SHM census drift remains unresolved. M1 debug rendezvous timeout
remains unresolved. Miri is unavailable, execution NOT_RUN. These are independent
of M5R and are not discharged by any passing replay.


## Demonstrated interrupt-state defect and minimal correction

Before deletion, the diagnostic control reached the first real sleeping wait
on four CPUs. Nonwaiting returns logged RFLAGS `0x2` / `0x6`; the resumed
consumer PID 41/incarnation 53/VM 91 logged `0x246`. The opt-in invariant
assertion stopped with `Doorbell kernel continuation resumed with IF set`,
left 512, right 0. This demonstrated a real syscall continuation invariant
violation independently of the two historical failure symptoms.

The correction preserves outgoing saved RFLAGS in `reschedule_entry`, removing
its unconditional IF insertion. The hardware timer's existing policy is
unchanged. Initial runnable contexts and hardware IRQ frames already have IF
set. A software reschedule from CPL0 retains IF clear through the trampoline's
return to its private userspace syscall stack, until the established SYSRET
return enables user interrupts. Scheduler selection, queue policy, stack pins,
dispatch requests, notification arbitration and timeouts are unchanged.

The opt-in `fastlane_doorbell_trace` observes bounded operation, publication,
registration/removal, wake-ready state, saved/selected context, incoming IRQ
CS/RFLAGS, ICR sent/busy, closure, and resumed continuation state. Existing
completion/collection events supply exact sequence outcomes. Generation,
PID/incarnation/VM join channel events. ICR records are CPU transport observations
with generation/PID zero, not falsely attributed to a single channel.
There are at most 4,096 trace records and one overflow marker per boot. Missing
events/overflow do not prove absence of an operation. The resume observation
captures flags then disables interrupts before its own locking/assertion; it
asserts the observed incoming state, so it cannot silently hide the defect.
The assembly diagnostic block is assembled out without the trace feature.
`fastlane_doorbell_if_control` restores the pre-repair IF write only in an
explicit debug experimental control image and implies that diagnostic assertion.

The unsafe window follows directly from the stack/interrupt protocol:
SYSCALL masks IF and runs its dispatcher on the process-private userspace stack.
The Doorbell temporarily moves to the kernel stack to suspend safely through a
CPL0 software interrupt. Forcing IF into that saved interrupt frame makes the
resumed continuation interruptible before it restores the original private RSP
and finishes SYSRET. An IRQ arriving after that restoration is still a CPL0
interrupt, so it uses the current private stack rather than entering through
TSS RSP0. A scheduler CR3 switch can then invalidate the very Rust/IRQ return
stack still in use; an IRQ while the continuation owns SCHEDULER can instead
re-enter that lock. Preserving IF clear until SYSRET closes this window, and a
later CPL3 IRQ enters through the normal per-process kernel stack.

These are source-backed failure mechanisms, not reconstructed execution traces
of the two historical guests. This defect provides a plausible mechanism for
continuation corruption or scheduler-lock deadlock after a wait. It **does not establish the exact causal
chain in either historical guest**. Their unavailable saved frames and dispatch
observations preclude evidence-backed closure; root attribution remains PARTIAL.

## Fresh paired control and repeated native evidence

Durable root: `/home/ehsantor/.local/state/sunlightos/m5r/`.
The final executable/configuration input map contains 1,364 files, canonical
sorted compact JSON SHA-256
`8189558cc7767142610bc2ea8f66500e39a3053a3c3592926ae27cc5edccf879`.
HEAD is `228bb82c85e58eace0f6df21bba7a6736097eb17` plus the uncommitted M5R edits.
Reports are outside that executable input map.

- Corrected diagnostic: `diagnostic/native-20261102T133727.851549Z/` PASS on
  four CPUs, all 30 pairs / 33 registrations / 32 completions and collections.
- Explicit pre-repair IF control: `control/native-20261102T133818.273066Z/`. Expected
  native FAIL at the first sleeping wait. Consumer 41/53/91 resumes at CPU 1
  with RFLAGS `0x246`; the assertion fails with 512 versus 0. The command FAIL
  is retained; the independently classified invariant reproduction is PASS.
- `interrupt-state-binary-audit.json` and the three `*-reschedule.asm` files
  verify the saved-IF insertion exists in the control and is absent in both
  corrected diagnostic and ordinary M4 ELFs. Their source maps are identical;
  the control differs by its explicit build features.
- Five independent ordinary four-CPU guests in `m4-repeat4-{0,1,2,3,4}/` PASS.
  All run without tracing or the pre-repair control feature. They do not cancel
  either historical FAIL.

The corrected diagnostic hit its explicit 4,096-event cap: 3,754 transport
records consumed most of the budget, then one overflow marker was emitted.
`diagnostic-observations.json` records 33 native registrations but only 29
registration trace records, with four missing after overflow. There are 35
recorded resume observations with IF clear and 29 recorded CPL0 interrupt frames.
The invariant assertion still executes on every return after trace exhaustion,
and the independent full fixture completed. This is not complete tracing of
every operation: dropped records cannot prove absence or exact delivery order.
There was no observed physical busy-ICR condition in this traced guest; the
independent extracted-code tests cover that mechanism. Historical missing
observations remain missing.

Ordinary M4 ELF SHA-256:
`9902ccf0d1acbecbb24d148c006b798986d8baa47e07ade883bbe9edae83dbf6`.
Ordinary M4 ISO SHA-256:
`36783699a2523b02e1d8473e137aeba863f8857794ade5168344c4aa0180d281`.
Build manifest: `m4/build-20261102T133854.485290Z/validation-summary.json`.

Fresh 19 M4/M5 parser controls, 29 extracted dispatch/handoff tests, focused
scheduler production state/lifecycle/accounting/policy tests and the new
module/shared trampoline formatting check PASS. Miri availability check fails
because cargo-miri is absent; Miri test execution remains NOT_RUN.

## Security audit of the final change

| Boundary | Source review |
|---|---|
| M1 authority | Exact handle/token/generation, issuer/session/epoch, incarnation/VM and role checks remain in existing admission paths. Tracing cannot authorize an operation |
| M2 mapping | No mapping, rights, alias exclusion, NX, backing ownership or TLB retirement algorithm changed |
| M3 integrity | Hostile representation/private-copy implementation unchanged. Memory safety and private snapshot stability remain the contract; coherence and application authenticity remain unprovided |
| M4 semantics | Sequence check/registration transaction, notification completion, cancel/close arbitration, exhaustion and one process-private receipt unchanged |
| M5 transfer | No transfer source changes; still one publication per channel and kernel-mediated bounded word access |
| Scheduler | Preserve the suspended continuation's IF state; no queue, priority, pin-release, handoff, coalescing, retry-budget or wake-policy redesign |
| Lifetime / unsafe | New raw reads are only two fixed scalar slots in the live CPU interrupt frame supplied by naked entry (CS/RFLAGS at words 16/17). Resume hook reads the existing private copied SyscallFrame. No peer-data pointers, retained references or uninitialized storage added |
| Diagnostics | Existing serial sink, constant atomics, no extra authority object, per-message allocation or global scheduler hot-path lock. Diagnostic assembly and trace module excluded without explicit debug feature. Logs include stack addresses for diagnosis but never tokens/consent receipts |
| Production | Experimental feature/release exclusion must pass independently; no production routing or enablement is introduced |

The historical fetch fault's `0x10` has INSTRUCTION_FETCH set and USER_MODE
clear (the installed x86_64 crate defines these as bits 4 and 2). It is consistent
with a supervisor continuation using the private stack, rather than evidence of
a particular userspace actor jumping to zero. This is an inference from the
recorded error bits, not a recovered outgoing/resume frame or a proven PID.

## Evidence recovery limitation

The home filesystem is Btrfs, but accessible snapshot paths were absent.
`btrfs subvolume list -s /home` is denied and noninteractive sudo requires a
password. No snapshot was mounted or altered. The user confirmed no known backup after the emergency cleanup. No inaccessible
snapshot is assumed absent, but original failure re-audit remains BLOCKED without
an available copy.

## Failure-class discrimination

| Candidate | Evidence-backed disposition |
|---|---|
| Lost wakeup | Both historical rows committed the expected completion, but scheduler delivery/execution was not logged. Model completion is established; a scheduler lost wake is neither demonstrated nor conclusively excluded |
| Delayed scheduler progress | Replay has no collection before 180 seconds; no saved dispatch/CPU-progress witness distinguishes delay from deadlock |
| Incorrect sequence | Replay publishes sequence 1 as expected; producer-exit case returns Terminal as expected. No incorrect notification sequence is observed |
| Wait-registration race | Replay proves both real actors, serialized registration and contention, then successful completion. The source has no unconditional Blocked write after the transaction; subsequent continuation state is the missing witness |
| Closure/notification race | Producer exit records the expected terminal wake, not an incorrectly admitted notification. Its later fetch fault remains unattributed |
| Deferred IPI delivery | Historical ICR observations are absent. Fresh extracted dispatch/busy-ICR tests PASS; they cannot establish physical delivery in an earlier guest |
| Test-only coordination | Both replay race actors returned; there is no debug rendezvous deadline. A later coordination stall remains possible, but no test-only root cause is demonstrated |
| Resource cleanup | First producer physically reaped; consumer completion/whole-cycle census absent. Replay had not reached closure. Neither log proves a cleanup leak |
| Kernel continuation interrupt state | Demonstrated by paired native control and binary audit; repaired by preserving saved IF. Exact contribution to each historical failure remains unproved |

Trace identities are scoped by attempt/boot before joining generation,
PID/incarnation/VM and sequence. Repeated numeric identities in different guests
are not the same process/channel and cannot identify an earlier faulting actor.
Trace `a`/`b` scalars decode as follows:

| Event | a | b |
|---|---|---|
| operation | admitted model opcode | requested observed sequence |
| publish-remove | published sequence, MAX on exhaustion | waiter detached |
| wait-registered | matched observed sequence | prior saved context RSP |
| wake-ready | saved context RSP | owner CPU in low byte, queued CPU in next byte |
| save | outgoing context RSP | process state |
| dispatch / dispatch-idle | selected context RSP | destination CPU |
| irq-frame | saved CS | saved RFLAGS before epilogue |
| resume | actual resumed RFLAGS | original syscall return PC |
| close | terminal reason | model had waiter |
| icr-sent | destination CPU | failed attempts before acceptance |
| icr-busy | destination CPU | first busy observation (0) |

## Final native validation matrix

**Native restoration: PASS. M5R: PARTIAL. Production FastLane: DISABLED.**
All accepted current guests use the final executable source map above.
There are 48 accepted native guests: nine M4 (five repeated 4-CPU, three
acceptance topologies and one diagnostic), three M5, 33 broader regressions,
and three M3.1B guests. The M3 compatibility wrapper and its nested mapping
receipt describe the same guest and are counted once. The separate pre-repair
IF control is a retained expected FAIL, not an accepted guest.

| Gate | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Ordinary M4 authenticated Doorbell acceptance | PASS | PASS | PASS |
| Additional targeted ordinary M4 repetitions | NOT_RUN | PASS (5/5) | NOT_RUN |
| M5 authenticated mediated transfer | PASS | PASS | PASS |
| M1 authority / M2 mapping-retirement / M3 native word probes | PASS | PASS | PASS |
| IPC deferred/reply / VM / SHM retirement | PASS | PASS | PASS |
| Authenticated Account S1/S2 contention | PASS | PASS | PASS |
| Account successful/failed exec, three cycles | PASS | PASS | PASS |
| Actual exit mode 11, three cycles | PASS | PASS | PASS |
| Actual exit mode 12, three cycles | PASS | PASS | PASS |
| Actual exit mode 13, three cycles | PASS | PASS | PASS |
| Scheduler lifecycle | PASS | PASS | PASS |
| R2D remote wake / physical handoff | PASS | PASS | PASS |
| Existing native kernel Doorbell mechanism fixture | PASS | PASS | PASS |
| M3.1B reclamation and teardown, eight boundary crossings | PASS | PASS | PASS |

Each M4 accepted guest has 30 authenticated pairs, 33 registrations and
32 completions/collections, plus one deliberately killed blocked consumer.
Each M5 guest has 20 pairs, 18 publications, 10 acknowledgements, 11 private-copy
witnesses, 17 remote wakes, 60 backing objects and 120 role views. The M5 backend
remains mediated, one publication per channel. No multi-message or performance
work was implemented. M3.1B proves the repaired reclamation path; its old
pre-repair retention control was NOT_RUN in M5R.

The machine aggregate is
`/home/ehsantor/.local/state/sunlightos/m5r/validation-summary.json`.
It joins `matrix.json`, `legacy-fill.json`, `audit.json` and original per-attempt
manifests/host ACKs. New artifact hashes are indexed by `artifact-index.json`.
Evidence is retained in two locations outside Cargo targets:

- `/home/ehsantor/.local/state/sunlightos/m5r/`: M4, M5, broad compatibility,
  scheduler/remote-wake, M3.1B, controls, host checks and production receipts.
- `/home/ehsantor/Projects/sunlightos-kernel/.fastlane-evidence/m5r-legacy/`:
  Account authority and exec/exit reruns. `.fastlane-evidence/` is explicitly
  ignored by Git and survives `cargo clean`.

The initial broad orchestration remains **FAIL** because five older Account
harnesses raised `ValueError` while printing `out.relative_to(ROOT)` for an
external evidence path, before any build/native attempt in those gates. Their
original commands/manifests are retained. The independent legacy reruns use an
ignored in-repository folder outside `target/`, with **no executable source or
acceptance-check changes**. All 18 native guests there independently satisfy
FORCED_AND_PASSED, terminal protocol PASS and original host ACK requirements.
This explains the infrastructure failures and supplies coverage; it does not
rewrite the failed orchestration. Clean-bootstrap failures (missing staging
parent, missing embedded binaries, existing Rappid DOM recipe inconsistency and
an interrupted stalled kernel.org dependency clone) also remain separate FAIL
commands. Existing recipes and the identical pinned DTC commit restored the
build, without desktop/audio source edits.

## Production and release outcome

| Check | Result |
|---|---|
| Ordinary debug production feature list empty | PASS |
| Exact normal embedded services in ELF and ISO-extracted ELF | PASS |
| Experimental authority/mapping/Doorbell/transfer/trace markers absent | PASS |
| Experimental adapter/trace/transfer symbols absent in ordinary debug ELF | PASS |
| Full supported ordinary release build | PASS |
| Retained release ELF has exact final normal service bytes and no experimental markers | PASS |
| Experimental control release feature | PASS rejection; original command FAIL / exit 101 with debug-only compile guard |
| Symbol inspection of fully stripped release ELF | NOT_RUN; independent debug symbol check above PASS |
| Production FastLane enablement | BLOCKED / disabled |

`audit.json`, `ordinary-release-audit.json` and
`release-feature-rejection.json` retain exact inputs and outputs. Final ordinary
services were restored after Account fixture runs; production images remain
separate immutable artifacts. No commit or push was made.

## Remaining blockers and next executable milestone

| Item | Classification |
|---|---|
| Both historical M4 failures | FAIL observations; exact cause remains unproved |
| Re-audit original failed images/serials/manifests | BLOCKED; artifacts deleted, no available backup |
| Historical global SHM census drift | UNRESOLVED; fresh scoped retirement passes are not causal reconciliation |
| M1 debug rendezvous timeout | UNRESOLVED; fresh compatibility passes do not explain the old timeout |
| Miri execution | NOT_RUN; cargo-miri unavailable |
| Complete fine-grained diagnostic capture | PARTIAL; explicit overflow, four registration trace records missing |
| New native scheduler / IPC / VM / lifecycle regressions | None observed in the matrix above |
| M5.1 / performance optimization / production enablement | NOT_PERFORMED |

The confirmed IF-state defect is repaired and current native validation is
reliable across the tested repetitions. **Neither historical failure is declared
causally closed. M5R is PARTIAL even though every fresh acceptance guest passes.**
Existing independent production security debts (including earlier D3/N3M and
service/global reconciliation gates) are not reclassified by this milestone.

The next executable milestone is targeted **M1 debug rendezvous progress
closure**, preserving its real contention witnesses and authority checks. Keep
M5.1 and production enablement deferred. A subsequent M5R diagnostic investigation
should reserve trace budget for channel/continuation events before collecting
transport chatter, and capture exact outgoing/resume frames around the unsafe
pre-repair window; it must retain the two historical failures as unresolved
unless stronger causal evidence becomes available.
