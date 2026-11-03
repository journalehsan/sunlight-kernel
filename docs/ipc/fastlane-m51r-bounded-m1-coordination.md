# M5.1R bounded M1 coordination

Successor: [M5.1R2 final bounded evidence](fastlane-m51r2-final-bounded-coordination.md)
completes the remaining controls and current regression gates. It classifies the
mechanism and repair COMPLETE, preserves historical attribution as UNRESOLVED,
and recommends gated experimental M5.2. The PARTIAL report below and its original
machine aggregate remain the M5.1R snapshot; their failures are not relabeled.

2026-11-02. **PARTIAL — functional coordination and final regression matrix
complete; historical causal attribution remains unresolved.** Production
FastLane remains disabled. No transfer functionality, shared ring, performance
work, production scheduler policy change, commit or push.

Evidence root: `.fastlane-evidence/m51r/`, outside Cargo target. Existing M5.1
evidence and all new failed attempts remain intact. The final run is
`final-20261102T193952.461482Z/` below that root. Native images are archived once
per build and referenced by subsequent native receipts.

## Failure reconstruction and attribution

The historical documentary witness is:

```text
entered=1 cpus=(1,18446744073709551615) mode=2 held=0 contended=0 finished=0
```

In mode 2, actor 1 is the authenticated consumer invoking attach; actor 2 is
the authenticated producer invoking revoke. Only the consumer entered, on
CPU 1. MAX means no producer entry was observed; it is not the producer's CPU.
No selected winner had acquired the lifecycle lock yet. The retained source
drops its scheduler guard before the rendezvous and holds no broker guard
there. This excludes a lifecycle lock held by that waiting invocation; it does
not reconstruct the missing producer's or unrelated actors' lock ownership in
the historical guest.

`docs/ipc/fastlane-m31-census-attribution.md` records this witness and the old
five-second panic, while later M3.2/M5R/M5.1 reports keep the debt open. The
documented original `target/fastlane-m31b-evidence/m1-loop/` and `m1-diag/`
artifacts are unavailable in this checkout. Searches of the retained evidence
and local M5R state found no original M1 failed serial/image pair. The available
M5.1 archive contains newer validation. `baseline/historical-witness.json`
preserves the documentary observation and explicitly lists its missing fields.
The earlier report's causal-completion wording is not adopted here.

The same M3.1 report also documents an earlier, untraced two-CPU panic at
`channel_race.rs:43` in
`repeat-attempts-b/native-20261101T154928.904344Z`. It provides no actor,
placement or IF witness. Its separate initial `repeat-attempts` orchestration
was ABORTED without a guest result. Neither observation is silently merged
with the later traced witness or attributed to the controlled placement.

| Question | Historical traced witness only | New source-bound evidence |
|---|---|---|
| Arrived actor | Consumer attach, actor 1, CPU 1 | Same actor and CPU in the negative control |
| Missing actor | Producer revoke has no entry | Producer is Ready, queued on CPU 1, owner 255 |
| Interrupts | Not logged; syscall entry source clears IF | Entry observation records IF=0 |
| Locks | `held=0`; source releases scheduler before waiting | Same control path owns no scheduler/broker guard during the wait |
| Affinity | Producer mask/queue not recorded | Both actors have mask 2, so only CPU 1 is eligible |
| Scheduler progress | Not captured | Consumer never yields and cannot be timer-preempted; producer cannot run on another CPU |
| M5R correction | Contribution to an unobserved producer continuation unknown | Negative control reproduces with the M5R repair present |

Eleven new ordinary diagnostic two-CPU guests passed without a timeout. Those
passes establish current observations, not the cause of the historical stall.
Their snapshots normally show an unrestricted producer running on CPU 0 and a
consumer restricted to CPU 1.

A separate development negative control deliberately restricts the mode-2
producer to CPU 1 at arm time, using the same lifecycle implementation and
M5R-corrected interrupt entry. Its retained serial records:

```text
[FL-COORD-CONTROL] mode=2 producer_mask=2 expected=deadline
[FL-COORD] event=entry generation=6 mode=2 actor=1 pid=45 inc=57 vm=100 cpu=1 if=0 sched_lock=1 broker_lock=0 peer=39 peer_inc=51 peer_vm=87 peer_state=Ready peer_mask=2 peer_owner=255 peer_queue=1 deadline_ms=5000
[FL-RACE-DEADLINE] entered=1 cpus=(1,18446744073709551615) current_cpu=1 mode=2 held=0 contended=0 finished=0
```

The `sched_lock=1` entry snapshot is taken before dropping the guard. The
following rendezvous owns no guard. The control reproduces an interrupt-disabled
run-slot stall with an otherwise eligible queued producer. It demonstrates a
fixture defect and the exact witness shape. The artificial mask is documented;
it is not evidence that the historical producer had that mask. Historical
producer placement and its preceding IPC/continuation remain unobserved, so
strict historical causal closure remains **UNRESOLVED**.

Original control:
`same-cpu-control/native-20261102T185935.399433Z/`;
build/source map: `same-cpu-control/build-20261102T185844.285128Z/`.
Both original commands and the guest remain FAIL. Their expected-control
interpretation does not overwrite those results.

The corrected bad-placement development control reaches the direct
`channel race placement impossible: both actors require one CPU` assertion
before entering the rendezvous wait:
`repaired-impossible-control/native-20261102T190330.660293Z/`, paired with
`build-20261102T190318.945168Z/`. It is also a retained FAIL guest, not accepted
race coverage. Its early mode-1 observation additionally missed the old strict
contention ordering requirement; that limitation is retained.

## Correction and real contention

All synchronization changes are in explicitly enabled debug fixture code.

- The M1 controller uses the existing fixture affinity selector for CPU 0.
  Its independently authenticated session-racer remains on CPU 1. The selector
  keeps its existing root/name restrictions and is now gated by the M1 feature.
- Every unsatisfied coordination predicate yields through the existing F3
  scheduler vector. Only the yield executes on the process's private kernel
  stack. No scheduler, broker or core guard crosses a yield. There is no busy
  wait, sleep, new timer policy or enlarged five-second deadline.
- The private-stack helper saves the original stack and RBX, checks that it
  cannot overwrite an existing private-stack continuation, and restores them
  only after the same process resumes. Identity is rechecked after yielding.
  Every resume asserts IF remains clear, preserving the M5R contract.
- The selected winner waits for contender readiness **before** acquiring the
  existing scheduler/lifecycle lock. It then runs the ordinary operation and
  never waits for another actor while holding that lock.
- The contender makes a real failed `try_lock` observation. Only an observation
  ordered before the winner's return marker qualifies. A late or successful
  probe is explicitly a missed window, completes normal lifecycle checks and
  supplies no concurrent-coverage credit.
- Each of four required orders has at most sixteen fresh, independently
  authorized attempts. Exhaustion fails deterministically. The source-bound
  assessor checks every attempt's exact generation/process bindings, both
  results, terminal state, and reported miss. Exactly one qualifying attempt
  must finish each order; an all-missed fixture cannot pass.
- Opcode 23 is only an exact-creator diagnostic receipt for the currently armed,
  finished channel. It compares the complete handle and process identity and
  cannot authorize an operation. Dedicated userspace command 18 invokes kernel
  opcode 23, leaving mapping command 23 (COLLIDE) intact. Native checks reject a foreign process and a
  changed token. Existing channel operations still decide every authority result.

The five-second bound applies independently to each existing rendezvous phase;
100,000 scheduler yields also bound iteration count. The retry count is fixed
at sixteen per order. No unqualified attempt is silently substituted for an
actual concurrent race.

## Bounded observations

`[FL-COORD]` entry snapshots capture both identities, CPU, IF and coherent peer
state/mask/queue/owner under the scheduler guard. Eight fixed actor/phase slots
hold ten scalar observations each (640 bytes), with no heap allocation or
retained peer pointer. Slots record start/deadline/end, entry/exit/first-resume
CPU, IF, yield count and completion state. Wait phases are 0=entry, 1=winner
readiness, 2=contender awaiting lock acquisition, 3=contender acquiring after
the probe. Phase 3 ends owning the scheduler guard; all yields own no guard.

Wait observations are formatted only after both operations finish. This avoids
serial-lock delay before the contention probe. Only the first resumed CPU/IF
is sampled; every return still executes the IF assertion. `untraced_yields`
explicitly counts the remaining unsampled yield pairs. Zero-yield slots use MAX
for absent resume observations. These are aggregate progress observations, not
a complete scheduler-dispatch or interrupt history.

Each yield publishes Running -> Ready under the existing scheduler guard,
drops that guard, and invokes F3. Ordinary dispatch restores Running. Entry,
readiness, lock publication and completion are the peer progress transitions;
coordination allocates no blocked waiter or separate wake object.

There is a 24-record per-generation formatting cap with an explicit overflow
marker and final `records/overflow/missing` counts. Final acceptance requires
complete slots, consistent deadlines, no overflow/missing bounded records, and
exact sampled CPU/IF states. Ordinary `[FL-RACE]` and `[FL-OUTCOME]` markers
retain the real operation/lock/terminal order.

## Final validation

Final revision is `04ac8dfac5b675d64c9066be39dffe8b1f6b7e2b` plus the six-file
correction. The 1,369-file executable/configuration map has canonical SHA-256
`1488f0bfba10bf53488916c156f4180c7ffae800a9b4bcd649683c2cde9d7f84`.
`source-audit.json` records the exact differences and unchanged boundaries.
Each build/native manifest carries its own full map, image/service hashes,
original serial and host assessment. The final run's `matrix-driver.py` records
the orchestration; it aligns legacy direct builders with their actual Cargo
output directory to avoid the M5.1 stale-image error.

The repeated native M1 matrix is **PASS**. All thirty accepted guests use
`m1/build-20261102T193952.581514Z/validation-summary.json` and the same archived
image. ELF SHA-256 is
`bd5bd4bd12e46cbded4069cd533ba54d9d680f0b4ca5ed65cf4b6e7b5c73eeae`;
ISO SHA-256 is
`5044679f1c77676ed7828a132f5d75f7b139db06de726c6596985f561e246792`.

| CPUs | Guests passed | Fresh pair attempts | Qualifying races | Missed windows | Qualified actor 1 / 2 wins | Maximum attempts for one order | Rendezvous timeouts |
|---|---:|---:|---:|---:|---:|---:|---:|
| 2 | 10/10 | 68 | 40 | 28 | 20 / 20 | 6 | 0 |
| 4 | 10/10 | 68 | 40 | 28 | 20 / 20 | 4 | 0 |
| 12 | 10/10 | 79 | 40 | 39 | 20 / 20 | 8 | 0 |

Each topology has exactly ten qualified observations of each of modes 1–4:
consumer attach before producer revoke; producer revoke before consumer attach;
producer attach before actual consumer exit; actual consumer exit before
producer attach. The 95 misses complete the same exact-identity, terminal,
stale-attachment and zero-authority checks but count as no race coverage.
All 215 attempts are retained, including their 430 independent operation
results. There is no timeout retry or failed-guest substitution in this matrix.

`coordination-assessment.json` gives the exact thirty manifest paths/hashes and
independent bounded-observation assessment. Per-guest receipts are under
`m1-{2,4,12}-{1..10}/native-*/`. There are 527/553/847 actual scheduler yields
on 2/4/12 CPUs. The explicitly unsampled additional yield pairs are 316/335/616;
all 1,927 resumed continuations execute the IF assertion. Bounded record
overflow/missing counts are zero on every attempt. Complete phase slots and
five-second deadlines independently pass the supplemental assessor.

`review/precorrection-control.trace.txt` and `review/final-example.trace.txt`
are small extracts; the immutable original serials remain in their native
directories. The final example joins channel generation 10 to the exact
producer PID/incarnation/VM 39/51/87 and consumer 49/61/108. Both enter with IF=0,
on CPUs 0/1; lock-held sequence 49, real contention 50, and returns 51/53 prove
the qualified order. The final complete record has `finished=3`, `records=7`,
`overflow=0`, `missing=0`, `qualified=1`.

The final-source matrix is **PASS** on the unchanged source map. In addition to
the 30 repeated M1 guests, it includes 9 M4/M5/M5.1 guests, 33 IPC, VM/SHM,
authority, lifecycle and scheduler guests, and 3 M3.1B retention/reclamation
guests. One additional M1/M2 combined preflight guest passed on the same source.
M1 authority acceptance, restricted mapping, repeated transfer, host parser
controls, and the ordinary production image checks passed. The experimental
release build was rejected with the required debug-only feature diagnostic.
Exact manifests and serials are under the final evidence root; aggregate counts
and gate results are in `.fastlane-evidence/m51r/validation-summary.json`.

## Security audit

| Boundary | Review |
|---|---|
| M1 admission | Complete handle, role, issuer/session, incarnation/VM, expiry and terminal checks unchanged; race assessor additionally joins actors to exact bindings |
| Lifecycle serialization | Existing scheduler and broker guards retained across actual authority mutations; no test peer wait under either guard |
| M2 mapping / M3 reclamation | Mapping rights, backing identity, NX, TLB retirement and physical free implementation byte-identical to baseline |
| Hostile integrity | Memory safety/private snapshot stability retained; coherent publication and application authenticity remain NOT PROVIDED |
| M4 / M5 / M5.1 | Doorbell and single/repeated transfer implementation byte-identical; no new data operation or shared ring |
| Scheduler / M5R | Scheduler, interrupt entry and shared Doorbell trampoline byte-identical; private fixture yield uses the existing saved-IF contract |
| Unsafe lifetime | One debug-only private-stack helper; internal scalar stack pointer only, live process-owned stack, bounded reservation, same-task restore and identity/IF assertions |
| Diagnostics | Constant scalar storage; no new hot-path lock, production trace sink, authority object or per-operation heap allocation |
| Production | All new selectors/helpers/receipts require the debug M1 feature; final ordinary-image and release exclusion audits passed |

## Preserved development failures

The evidence root retains the sandbox QMP denial, the expected pre-repair
deadline, a compile error from an ambiguous diagnostic loop integer type,
the initial synchronous-trace contention miss, the corrected impossible-placement
control, and the old parser's rejection of a complete bounded-retry development
guest. These original statuses are not overwritten. The latter guest completed
all four real orders with two explicit missed windows; it is development-source
evidence, not a final-source accepted guest.

The first frozen matrix, `final-20261102T191038.369433Z/`, remains FAIL and
its broad orchestration INTERRUPTED. It completed thirty M1 and nine M4/M5/M5.1
guests, then combined mapping-enabled M1/M2 guests failed on 2/4/12 CPUs because
new receipt command 23 selected the existing COLLIDE handler. The retained
source and serial identify that dispatch collision; no rendezvous timeout
recurred. The userspace protocol now has a dedicated command 18. A final-source
combined M1/M2 preflight PASS is retained at
`coord-command-repair/native-20261102T193739.006564Z/`. All final acceptance is
rerun on the corrected source; the old passes are not substituted. The prior
matrix's subsequent IPC passes and interrupted authority attempt also remain
in their original manifests. A separate failed audit postprocessor (a string
passed to the Path-based digest helper) is retained under that run's `review/`.

## Remaining debt and next executable milestone

Historical M4 failures: **UNRESOLVED**. Historical global SHM census drift:
**UNRESOLVED**. Miri: **UNAVAILABLE** (`cargo miri --version` reported that
the component is not installed); execution **NOT_RUN**. Production enablement
remains disabled and blocked by independent security debt.

Historical M1 attribution remains partial because the original missing actor's
placement and continuation are unavailable. A passing final matrix cannot fill
those missing historical observations. The next smallest executable milestone
is a bounded, unpinned pre-correction control that captures producer IPC-ready
and dispatch state, affinity, lock owner, run queue, and CPU 0 continuation/IF;
also recover the original failed M1 artifacts if they still exist. A new
matching control alone cannot establish the historical actor's exact state.

The aggregate validation summary reports functional coordination **COMPLETE**,
new regression matrix **PASS**, and milestone **PARTIAL** for this reason. This
classification preserves the distinction between reproducing and repairing a
demonstrated fixture defect and proving the cause of the original incident.
