# FastLane F1.3N-R2C: native ordering and exit closure evidence

## Executive decision

**Gate A: PARTIALLY VERIFIED. Gate B: NOT READY. Gate C: BLOCKED.**
R2C acceptance remains incomplete. This phase extends the existing R2V native
fixture rather than changing scheduler policy or the production release algorithm.
It adds forced release-wins and publication/release/recheck boundaries, real
exception termination, terminal-stack retention audits and quiescent process-table
reuse. It does not establish every required exit/registration or delayed-dispatch
reuse overlap. No native Doorbell adapter, Restricted SHM, notification syscall,
or production FastLane channel is enabled.

The authoritative machine-readable index is
`target/fastlane-f13nr2c-evidence/validation-summary.json`. Raw serial, QEMU
commands, CPU identities, ELF/ISO hashes and build-source hashes are in
`native/results.json` and `native/*cpu.serial.log`. The final fixture ELF SHA-256 is
`cfb7a86e596daa07231d22dfb9589d57f53056417516c9622c5dde8488237fa9`.
Failed/superseded fixture
attempts are retained separately and never substituted for the final matrix.

## Remaining gaps inherited from R2V

R2V forced registration-wins across 60 remote rejection obligations on 2/4/12
CPUs, but release-wins and concurrent boundaries were host-only. Its migrated
tasks used ProcessExit; exception termination, overlapping terminal races,
process-table generation reuse, pinned stealing and delayed notification overlap
were absent. Matched performance and complete strict compatibility success were
also absent. This report distinguishes newly demonstrated boundaries from those
still missing; boot success alone cannot close an ordering gap.

## Lifetime and ordering contract

A slot belongs to a physical CPU. Its pin is a process-table index, not a task
pointer, PID, generation token or nesting count. Publication happens under
SCHEDULER before live ownership is detached; `stack_handoff_core` preserves the
physical owner after migration. A second publication on an occupied slot panics.
Selectors and reapers access process records under SCHEDULER. Slot reuse and
reaping reject a physically pinned index, so the index cannot be replaced while
it names an active kernel stack. The static aligned slot itself never moves.

For one attempted rejection, name these operations:

* O: selector observes the index in the owner's pin (Acquire).
* W: waiter publishes its CPU bit (`fetch_or`, SeqCst).
* Q: waiter rechecks the pin (SeqCst load).
* X: owner, after pivoting RSP, exchanges EMPTY into the pin (x86 locked xchg).
* L: owner tests the waiter mask after X.
* D: if L was nonzero, `take_waiters` exchanges zero into the mask (SeqCst).
* U: if Q differs, waiter removes only its bit (`fetch_and`, Relaxed).

O alone is not a scheduling obligation. If Q still reads this index, W precedes
X in the relevant global order, and a registration not withdrawn by that waiter
must be visible to L/D. The CPU restores the rejected queue entry and may idle;
the releasing CPU owes a scheduling event. D transfers the claimed CPU bits to
`pin_release_wake`, which publishes coalesced requests and flushes the existing
bounded dispatcher after dropping all scheduler guards.

If Q observes EMPTY (or a different index), the caller continues the **same
selection pass**. Both RR selection and RR stealing subsequently check state,
physical/live ownership, affinity and context validity before selecting. A false
return is therefore not a promise that some later timer will rescue the task.
It removes the specific physical eligibility barrier within an already active
scheduling opportunity. The fixture requires that pass to select the target.
An exit or affinity restriction may instead make it correctly ineligible.

Registration-wins: W, Q-held precede X; L/D claim the bit, a remote handler
re-enters the scheduler, and the target actually executes. Release-wins: X
precedes W/Q; Q-released and U allow the current pass to execute eligible work.
At the publication boundary, W precedes X but Q follows X. D may claim the bit
before U, or U may remove it before D; either is safe because this CPU already
continues its active selection pass. An extra CPU scheduling notification is
bounded and does not identify any task. SeqCst establishes the ordering argument;
actual selection/delivery/execution supply the separate progress evidence.

The exact linearization points are the pin CAS publication, W RMW, Q load, X
exchange, D exchange, and per-CPU reschedule request/consumption atomics. Trace
markers bound these operations; they are not timestamps of individual assembly
instructions. In particular, EMPTY observation proves X has occurred but does
not timestamp X relative to every other CPU's instrumentation write.

## Native fixture and ordering traces

Build with debug feature `pin_release_ordering_test`, which depends on the
existing `pin_release_test`. Normal R2V keeps its four rounds and parser. R2C
keeps those four rounds unchanged in intent and adds two rounds. All hooks are
absent from production release builds. The only added hooks outside the fixture
are before W, after W/before Q, and on exception termination.

Four registration-wins rounds retain the proven sequence: CPU0 executes and
yields a real ring-3 ELF; the real scheduler pins its outgoing stack; CPU0 drops
SCHEDULER and pauses before the production epilogue. CPUs 1..N-1 sequentially
reject and register, each consuming its initial request and returning no task.
The last queue entry remains eligible. CPU0 requires all acknowledgements and
the exact waiter mask before pivoting. The actual epilogue releases the pin,
claims waiters, requests and dispatches, and the saved task resumes on CPU N-1.
Every intended CPU must report rejection, waiter-bit publication, accepted send,
handler entry and subsequent scheduler pass. Missing registration fails.

Round 4 (`boundary=1`) pauses CPU1 inside the actual rejection path **before W**.
O has already observed CPU0's held pin. CPU0 waits without locks for that marker,
then returns through its ordinary RSP pivot and X. CPU1's bounded checkpoint
waits until its real slot load sees release, then performs W/Q/U. The callback
requires Q-released, and `after_schedule` requires the target selected on CPU1
in that very pass. No release-wake IPI is needed: the current pass provides
progress. Execution is checked through the existing user cookie and exit path.

Round 5 (`boundary=2`) pauses CPU1 **after W but before Q**. The checkpoint
asserts its bit is published while the pin is still held. CPU0 permits X only
after this marker. CPU1 observes release and continues Q/U and selection. The
releaser may already have claimed bit 1. The final claimed mask is recorded as
0 or 2; execution must happen on CPU1 regardless. This forces the narrow
publication/release/recheck boundary. It does not claim exhaustive instruction
races or simultaneous selector registrations: those serialize under SCHEDULER.
Both boundaries are exercised separately on 2/4/12 CPUs. All three observed
post-publication cases claim mask 2; withdrawal-before-drain is not separately
forced natively. The accepted alternative is source/host evidence, not an
additional observed native ordering.

Checkpoints are calibrated-TSC bounded watchdogs, not scheduling retries. The
owner takes no scheduler lock to pivot/release. Waiting under S for an operation
that needed S would deadlock; no such dependency is installed. There are no
allocations or serial writes inside the ordering checkpoints.

## Timer-assistance exclusions

APs mask only their local periodic timer in this isolated fixture; CPU0 retains
its timer for initial dispatch, terminal audit and cleanup. Any AP timer entry
during the critical phase panics. The target's queue affinity excludes CPU0;
stealing skips a sole victim queue entry. The controller publishes no replacement
wake before the terminal checkpoint. Registration-wins execution requires the
release request/send/handler/pass chain. Both new boundaries require the already
entered CPU1 selection pass to return the target, so an incidental later timer
cannot be counted as their success. A host timeout only kills the failed guest.
These exclusions apply to the critical remote execution, not timer-free boot,
UP scheduling or cleanup.

## Task exit interleavings

Round 3 uses probe role 4: yield once, validate its saved integer/SSE cookie,
then execute UD2. This reaches the real `terminate_current_user_process` with
code 132. The other five rounds use the real ProcessExit with code zero. Role
0/1/2/3 behavior is retained. Both terminal paths call the same fixture audit
only after `finish_current_process` has committed terminal state and dropped S,
before their existing assembly RSP pivot and X.

On SMP, the terminating CPU waits boundedly with interrupts disabled while
CPU0's ordinary controller tick acquires S. On UP the audit runs locally with S.
It asserts Finished, the correct physical pin, retained kernel stack, zero
queue occurrences and unchanged live generation. Real `wake_pid` and
`wake_io_pid` calls cannot requeue the terminal task; real reaping cannot free
its pinned stack; duplicate `terminate_process_by_pid` returns false. An audit
acknowledgement allows the owner to finish its normal release. Final cleanup
requires Reaped, no kernel stack, empty pins and waiters, and no remote delivery
obligation. The terminal event records actual owner and auditor CPU identities.

| Required case | Evidence and limit |
| --- | --- |
| D1 exit before rejection | Terminal late-wake and no-queue audit, plus old-PID wake after reuse. No artificial stale queue entry is inserted. |
| D2 exit after rejection before original release | **NOT FORCED**. Native task exits after its migration/resumption; this is a later pin lifetime. |
| D3 exit during registration | **NOT FORCED**. S serializes lifecycle mutation with selection; the terminal audit is a safe boundary, not an overlapping W/Q proof. |
| D4 exit concurrent with release | Actual terminal commit/pin/reaper boundary and later real release/reap; simultaneous remote terminal mutation not forced. |
| D5 exit after wake dispatch | Actual migrated self-exit/UD2 and late-wake state audit. Delayed hardware interrupt after index replacement is not forced. |

RecoveryBlocked rules and production exit ordering are unchanged. The fixture
never frees a stack while the terminating CPU uses it, never resumes a terminal
context and never treats faulting as expected task continuation.

## Slot reuse and ABA

The six ELF processes are fully allocated before arming, held in a bounded
fixture-owned prepared vector, then admitted one at a time using the existing
`add_process_after_reaping`. Every assignment asserts index 0 and a distinct
nonzero live VM generation. Before the next admission, the prior task is reaped,
all physical pins/waiter masks and remote request/dispatch obligations are empty.
Wakes for the previous PID leave the new generation BlockedOnIo and unqueued.
The captured generation is from before dispatch, not from the cleared reaped VM.
PIDs are distinct across these rounds; reuse of the numeric PID itself is not
covered. VM identity generation is recorded explicitly and is not conflated
with the separate foreground/session incarnation counters.
This is actual process-table reuse, beyond R2V's four preallocated table slots.

The per-CPU physical slot is also reused by successive tasks. Its owner cannot
start another pin before finishing its release helper: IF remains clear through
the epilogue and nested pinning panics. Therefore an old **owner drain** cannot
run after that same CPU has published a newer pin. A late waiter can register
after D; it rechecks under S. While that CPU holds S the task index cannot be
reaped/replaced and no other selection on the same waiter CPU can publish a
newer same-bit registration. U only clears that CPU's bit. An unrelated CPU's
bit remains intact. No additional generation counter is justified by this audit.

A delayed reschedule notification contains only a CPU destination. It neither
references the old task nor clears any pin. Request consumption and fresh
publication remain independently atomic, so an earlier notification cannot
erase a request published after acknowledgement. The next pass re-evaluates
current task state. Redundant notification is harmless and coalesced. A stale
waiter bit after D is transient and withdrawn by Q/U; a persistent unmatched
bit at quiescence is a failure. **Native overlap of delayed A dispatch with C's
new pin is not forced.** Quiescent reuse plus source reasoning is not advertised
as completing that acceptance requirement.

## Migration, masks and dispatch

Actual execution moves from CPU0 to CPU N-1 in the first four SMP rounds and
from CPU0 to CPU1 in the two new boundaries. Requested affinity is not used as
proof: the exit/fault hook reads the real current CPU and PID. Each rejected
queue state has exactly one occurrence and no current owner on the rejecting
CPU. S serializes competing selectors and ownership transfer. The fixture does
not force pinned `steal_work`, simultaneous competing eligibility, alternate
work while waiting, or a newer runnable task overlapping wake dispatch.

The 4-CPU registration-wins cycles have three real waiters; 12-CPU cycles have
eleven. Registration is sequential under S, with one mask bit per actual CPU.
The representation is AtomicU64, supporting indices 0..63. Tested QEMU topology
uses contiguous IDs, and each CPU asserts local APIC ID equals scheduler ID.
Existing clamping/contiguous-APIC assumptions do not establish support for sparse
or APIC IDs >=64. No CPU hot-unplug behavior is claimed.

`request_reschedule_on` and `flush_dispatch_notifications` remain unchanged.
Per-CPU intent/delivery ownership coalesces; bounded Busy-ICR retry preserves
unattempted work. The existing request-only/Busy/idle fixture and host boundary
suite are replayed separately. R2V-scoped request/send counters remain bounded
1..4 per waiter per registration-wins epoch. They include a normal exit self-IPI
and must not be divided by boot time to claim IPIs/sec. New ordering rounds can
have a redundant release kick; same-pass selection is their progress guarantee.

## Resource accounting and native CPU matrix

Each topology runs six lifecycle generations. UP is a local control and makes
no remote-rejection claim. The machine index records actual IDs, registrations,
claimed masks, ordering boundaries, terminal events and final frames.

| CPUs | First four execution CPUs | New boundary execution | Registered held rejections | Frames before / after | Result |
| ---: | --- | --- | ---: | --- | --- |
| 1 | 0 | local control | 0 | 68,226 / 68,226 | PASS local |
| 2 | 1 | 1 | 4 | 67,968 / 67,968 | PASS |
| 4 | 3 | 1 | 12 | 67,456 / 67,456 | PASS |
| 12 | 11 | 1 | 44 | 65,428 / 65,428 | PASS |

Quiescent assertions independently check zero active pins, waiter bits, target
queue occurrences, remote request bits and deferred dispatch. Six test
generations retire; one reusable Reaped scheduler record remains. The prepared
vector retains its bounded capacity; its empty length and this record are valid
fixture metadata, not active tasks. CPU0's ordinary local control request is
excluded, rather than falsely reporting globally zero intent. Physical-frame
restoration is required but is not substituted for scheduler-state assertions.
Heap bytes, complete scheduler allocation bytes and independent reference-count
instrumentation are **unmeasured**. There is no blanket zero-metadata-drift claim.

## Performance and C1.2F

No credible matched pre-repair source/binary/workload baseline was established.
No stash, reset, checkout, clean or Git metadata modification is used. No timing
improvement percentage, release overhead, remote latency distribution, lock hold
time, IPI rate or SMP-tail regression result is inferred from the functional TCG
fixture. Those measurements remain **NOT RUN / MATCHED BASELINE UNAVAILABLE**,
which is an evidence gap rather than an observed scheduler correctness failure.

Current-only C1.2F normal and mixed/audio captures are indexed under
`c12f/summary.json`, with reproduction commands, per-boot traces, p50/p95/p99/max,
startup/steady partitions and validators. They are single-vCPU captures and may
share host resources with compatibility replay; they cannot establish a matched
performance effect or SMP tail bound. The two normal captures each passed 22 trials. Their steady external
Ready-to-dispatch distributions are:

| Boot | p50 ms | p95 ms | p99 ms | max ms |
| --- | ---: | ---: | ---: | ---: |
| 1 | 0.412272 | 2.798810 | 10.027047 | 112.820627 |
| 2 | 0.170780 | 0.705417 | 2.333469 | 6.860332 |

Mixed/audio capture 1 passed all eight trials. Capture 2 is **FAIL**, not a
pre-existing failure classification: startup trial 1 recorded `tty_server`
`ready_count=0` and an empty wait list, violating the required service sample.
Per-trial diagnosis passes trials 2..8, and all four workers made progress in
all eight trials. This does not turn the overall failed capture into PASS.
`c12f/mixed-capture2-diagnosis.json` preserves that distinction. A separate
same-image retry after our other QEMU gates finished passed all eight trials.
It is indexed in `c12f/mixed-isolated-retry.json` with ELF/ISO hashes and its own
raw serial. It cannot erase the original failure or prove why startup differed.

These are current-only observations; the first boot also contains an internal
Ready-to-dispatch tail of 1571.963645 ms. Long tails remain visible and their
regression attribution is unverified. Structural validation and positive-nice
progress are distinct from comparative performance or audible sound.

## Compatibility replay and diagnostic classification

Reproduce with:

```sh
python3 tools/fastlane_f13nr2c_validate.py --checks --native --compatibility
python3 target/fastlane-f13nr2c-evidence/c12f/reproduce.py
```

The collector reuses existing host and compatibility commands. `checks.json`
records exact commands, exits and logs; `compatibility/validation-summary.json`
indexes lifecycle, exec, resident/SHM retirement and VM/TLB replay. Remote smoke
and forced pin evidence remain separate. D1/D2/D3, RecoveryBlocked and real
certificate-verifying TCP/TLS results must be read from their own current logs.
No prior build's success substitutes for a current failed prerequisite.

Kernel host, extracted scheduler, boundary, Ring, Doorbell, IPC/service, Python
validators, native parser negative controls, bare-metal default/fixture/release
compilation, strict Clippy, changed-file formatting, workspace formatting,
media/WAV and diff checks are recorded independently. Complete R2C acceptance
is not claimed when any required check is missing or failed.

The current kernel strict-Clippy replay has 24 distinct diagnostic/path pairs,
all also present in retained R2V evidence (`clippy-comparison.json`, zero
current-only pairs). Dependency/build-script failures prevent complete fixture
Clippy coverage. Media again has 36 passing tests and the same missing repository
WAV failure in `mp3_converted_repository_wav_is_valid_but_exceeds_loader_limit`.
Workspace-format failure remains separate from formatting of the files changed
here; unrelated files are not rewritten. All current workspace-format hunks also
appear in retained R2V output (`format-comparison.json`, zero current-only hunks).
Miri is BLOCKED because cargo-miri is not installed for the current toolchain.
Failures are classified by matching evidence, not by assuming every failure was
pre-existing. Source hashes are checked against each native build manifest;
test-fixture edits require a new native image and matrix.

Current replay outcomes (detailed commands remain in the JSON index):

| Gate | Outcome |
| --- | --- |
| Kernel host | PASS, 91 tests |
| Extracted production scheduler host | PASS, 63 tests |
| Delivery/pin boundary | PASS, 29 tests |
| Ring / Doorbell host | PASS, 5 / 8 tests |
| IPC/service host | PASS, 115 tests |
| Old and new evidence-parser negative controls | PASS |
| Default, pin fixture, ordering fixture and release target compilation | PASS |
| Forced ordering fixture / remote request-only Busy smoke | PASS, 1/2/4/12 CPUs |
| Existing scheduler lifecycle | PASS, 1/2/4/12 CPUs |
| Exec/authority and resident/SHM retirement | PASS, 1/2/4/12 CPUs |
| RecoveryBlocked | PASS, 1/2/4 CPUs |
| Standalone D1/D2/D3 | PASS, 1/2/4 CPUs |
| Native TCP/TLS | PASS, 12 CPUs, certificate verification plus unknown-issuer/wrong-host rejection |
| VM/TLB, unmap, protect | PASS, 12 / 4 / 4 CPUs |
| Strict Ring/Doorbell Clippy | PASS |
| Strict kernel / ordering fixture Clippy | FAIL at matched dependency/build-script diagnostics; complete fixture lint coverage BLOCKED |
| Changed-file formatting / diff check | PASS |
| Workspace formatting | FAIL, all hunks match retained R2V output |
| Media/WAV | FAIL, 36 pass plus the matched missing WAV failure |
| Normal C1.2F boots 1 / 2 | PASS, 22 / 22 trials |
| Mixed/audio C1.2F boots 1 / 2 | PASS / FAIL (startup service sample absent) |
| Separate same-image mixed/audio retry | PASS, all eight trials; original failure retained |
| Miri | BLOCKED, cargo-miri unavailable |

The combined lifecycle/resident/VM runner exits nonzero only for workspace
formatting; its native assertions passed and its own source hash verification
passed. This aggregate exit is retained, not relabelled as an overall PASS.
The initial TCP/TLS attempt failed before exercising TLS because the sandbox
rejected local socket creation. The approved retry outside that restriction
passed, including verified direct/proxy HTTPS, unknown-issuer rejection and
wrong-host rejection. Both attempts are retained; the denied attempt is BLOCKED
environment evidence, not a TLS correctness failure. The separate mixed retry
is indexed independently.

Two earlier fixture mistakes are retained: a selection assertion was applied to
a later exit IPI, and a generation was read after reaping cleared its identity.
The final fixture scopes the selection assertion to the first boundary pass and
captures the live identity before dispatch. Neither retained attempt establishes
a production lost wakeup. `native-before-table-reuse/` is superseded, not final.

## Remaining limitations and Native Doorbell prerequisite

Critical remaining coverage is D2/D3 initial-pin exit overlap, delayed dispatch
across newer pin publication, pinned stealing/alternate-work/new-work overlap,
forced timer-epilogue waiters, simultaneous multi-waiter arrival contention,
complete metadata accounting and matched performance. The mixed/audio replay
startup service-sample failure also remains visible. Safe source arguments do not replace the requested native evidence.
Consequently Gate A stays **PARTIALLY VERIFIED**, even when the final new matrix
passes. This report is a concrete partial closure, not completed R2C acceptance.

Gate B stays **NOT READY** independently: exact-identity conditional block/wake
handoff, cancellation, registration withdrawal and adapter lifecycle integration
remain unverified. Passing pin-release tests alone cannot authorize adapter
readiness. Gate C stays **BLOCKED** until Restricted SHM authorization,
per-recipient rights, active revocation, delegation wiring and production
lifecycle races pass. Scheduler delivery and secure transport are separate gates.
