# KFP-2A.1 — bounded xAPIC delivery and safe failure boundary

## Verdict and scope

**PARTIAL. Production runtime behavior changed.** The two blocking xAPIC send
APIs now have finite software polling and explicit `Result` outcomes. Their TLB
callers fail stop on delivery error rather than returning to reclamation.
This is a bounded delivery repair, not a completed guest fault-injection proof
or a resumable quarantine implementation. No commit or push. Production FastLane
remains disabled.

The mission explicitly requires stopping at the smallest safe boundary when
safe propagation needs a larger architectural change. That boundary here is
synchronous `invalidate`: its callers can already have removed PTEs and hold
physical ownership only in stack-local records. Returning an ordinary error
without a persistent retirement owner would abandon obligations and let later
exit/reap/cleanup reclaim unsafe backing. A new retirement journal across VM,
SHM, capabilities, quota, process exit and rollback is beyond this repair.

## 1. Source baseline

HEAD at capture: `882b40a65f55951b5cd8b9978c05580e71202488`.
Evidence: `.kfp-2a1-evidence/source-before.json` records exact SHA-256 hashes of
tracked files before edits; `status-before.txt` and `preexisting.patch` preserve
working-tree state. The existing three-line `security.rs` MM-2D trusted fixture
policy adjustment was preserved. Existing untracked KFP-2A evidence/report were
also preserved. `source-build.json` identifies the guest build inputs.

Reviewed KFP-1 report/current findings, run results and dependency graph, KFP-2A
report/results and commands. Prior passing guests are historical evidence only.
KFP-2A's initial MM-2D SHM prerequisite panic, missing guest fault controls,
unproved activation overlap/paging-cache lifetime, incomplete global lock proof,
and unmeasured latency remain separate from this repair.

## 2. Confirmed root cause and architectural boundary

`wait_for_icr_idle` previously spun indefinitely on volatile xAPIC ICR LOW
bit 12. Both `send_fixed_ipi` and `send_nmi` waited before and after ICR writes;
TLB invalidation called both before entering its bounded ACK scan. Thus the
100,000,000-scan ACK limit did not bound delivery. This establishes a software
liveness defect; it does not establish a stuck physical APIC in ordinary use.

Intel SDM Vol. 3A section 11.6.1 describes ICR delivery status and the separate
xAPIC HIGH/LOW register interface. Idle is controller status, not evidence that
a target executed invalidation. x2APIC uses a different MSR interface and is
rejected by `cpu_identity::validate_controller`; this patch does not add support.
Architectural source: [Intel SDM](https://cdrdv2-public.intel.com/835754/253668-sdm-vol-3a.pdf).
No hardware delivery duration guarantee or exhaustive errata review is claimed.

## 3. Production changes and polling classification

| Function | Applicability | Bound / result | Caller expectations |
|---|---|---|---|
| `wait_for_icr_idle` | xAPIC only, fixed/NMI | 4096 volatile reads per invocation; bool | Internal idle observation |
| `send_icr` (new) | xAPIC only, fixed/NMI | At most 8192 reads, two writes; explicit error | No implicit retry |
| `send_fixed_ipi` | TLB; optional test broadcast | `Result<(), IcrDeliveryError>` | Controller idle after send; ACK still required |
| `send_nmi` | TLB | Same | NMI doorbell for IF=0 target; ACK still required |
| `try_send_fixed_ipi` | scheduler bounded sender | Existing one-read attempt, unchanged | Acceptance only |
| `send_fixed_ipi_bounded` | scheduler dispatch flush | Existing 4096 attempts, unchanged | No held scheduler/core/IPC guards; caller fail-stops on failure |

No other production ICR polling loop was found in kernel source. Limine handles
AP startup. LAPIC timer calibration is a different loop, outside this scope.
The scheduler primitive cannot replace blocking delivery semantics: it returns
on command acceptance and does not wait for post-write idle.

## 4. API and failure propagation

`NotSent` means this invocation made no ICR writes. Previous commands may still
be pending. `DeliveryUncertain` means LOW was written but idle was not observed
within budget; it does not mean the target failed to receive it. `Ok(())` means
controller idle was observed, never remote synchronization completed.

Every blocking send call site checks the result. Production TLB fixed and NMI
errors panic with target CPU, exact sequence and outcome. The optional MM-2B
broadcast uses its existing invariant failure path. No wrapper retries an error.
Missing ACK still uses the existing non-returning timeout. Only exact per-target
sequence equality permits successful return. No delivery outcome writes an ACK.

The kernel panic handler disables interrupts, does not unwind and never returns;
its diagnostic serial output uses try-lock. This is an initiator fail-stop, not
a claim that all other CPUs halt simultaneously. Outstanding locks remain held.
The failure state is terminal and unavailable for reuse, not operational recovery.

## 5. Lock, interrupt and clock analysis

The new helper allocates nothing, acquires no lock, sleeps nowhere and logs
nothing in the poll loop. It requires IF=0 across HIGH/LOW writes, as documented
on both unsafe send APIs. Normal syscall FMASK/fault gates and boot callers
provide masked contexts; no universal proof for arbitrary future callers is
claimed. The ordinary NMI handler performs mailbox operations without issuing
IPIs or taking scheduler/PMM/broker/serializer locks.

Retirement can hold SCHEDULER → PMM → CAP_BROKER, then SHOOTDOWN_SERIALIZER;
other VM callers inherit their own exclusion. These edges are unchanged. Waiting
cannot yield under these guards. Scheduler dispatch uses its separate existing
bounded path outside those guards.

The budget limits software reads, **not wall-clock time**: MMIO exits, host
scheduling or stalled hardware can delay a read. TSC calibration can be zero
(the LAPIC calibration path explicitly has a fallback); no universally available,
reliable deadline clock in all send contexts was proved. Therefore no time-based
deadline or timer-interrupt dependency was introduced. 4096 reuses the existing
scheduler attempt budget as a conservative policy, not a hardware timing proof.

## 6. Safety invariants and ownership

* `unmap_page` clears the leaf, synchronizes, then returns physical ownership.
  A failed send cannot return that ownership to the caller.
* `replace_mapping` synchronizes before `ReleaseOldFrame` frees old backing.
* `munmap` stores removed ownership in a bounded stack array, synchronizes the
  chunk, then frees backing/swap slots and reclaims empty tables.
* SHM `free_shared_page` preflights, synchronously unmaps each leaf, then removes
  the view/reference; `ReclaimedShared::reclaim` releases frames before quota.
* Restricted FastLane retires row authority, revokes and drains real process
  views, and clears backing tokens only after drainage. No-view objects have no
  translation obligation. This protocol is unchanged and not production enabled.
* Owner teardown requires inactive VM/lifecycle exclusion; it cannot substitute
  for completion of a failed active retirement.

On fail-stop the affected caller's ownership record, backing, remaining page
tables, broker references and quota remain where they were at failure. Earlier
successfully synchronized chunks/pages may already have been retired. No blanket
claim of zero changes to the entire operation is made. Cleared PTEs are absent,
but stale remote translations may still access retained physical backing.
Capabilities are not universally revoked by an APIC error. Continuing normal
execution or process exit is not supported after this terminal state. Retained
state is not a persistent quarantine surviving reboot or process teardown.
No retry is permitted from the abandoned stack; a late ACK cannot resume it.
Reclamation would require complete matching ACKs or a separately proved address
space quiescence protocol, neither synthesized from timeout.

Mailboxes remain serialized, single-outstanding and use checked nonzero sequence
numbers; publication requires prior ACK, payload precedes Release publication,
claims use CAS, completion is Release/Acquire equality. Duplicate/stale ACKs
cannot complete a new sequence. Arbitrary hostile ACK writers are not supported.
The full activation-overlap and paging-structure cache proof remains unresolved.

## 7. Fault-injection design and boundary

No production fault injection was added. `icr_controls.py` extracts the actual
production polling/helper text and replaces only MMIO with thread-local simulated
reads/writes. Five host tests cover permanently busy before write, permanently
busy after write, final permitted idle read, idle arriving too late, and normal
fast path. This is isolated host instrumentation; it cannot affect a kernel build.

The unchanged real mailbox source is exercised by the KFP-2A fault test wrapper
for missing/delayed completion, duplicate claims, stale sequence and concurrent
claims. These are protocol controls, not guest retirement tests.

| Required case | This milestone's proof scope |
|---|---|
| FI-1 stuck ICR | Host helper bounded-exit proof only; guest NOT_RUN |
| FI-2 TLB delivery failure | Source fail-stop barrier; guest/backing accounting NOT_RUN |
| FI-3 missing ACK | Host mailbox retention; guest timeout NOT_RUN |
| FI-4 delayed ACK | Host sequence tests; within/beyond guest budgets NOT_RUN |
| FI-5 duplicate/stale ACK | Host real-mailbox tests; guest NOT_RUN |
| FI-6 concurrent retirement | Existing serializer source; host concurrent claim only; guest NOT_RUN |
| FI-7 exit/repeated cleanup | Source terminal boundary; resumable guest cleanup NOT_RUN |

Guest FI would require a fixture that observes exact resource identities before
non-returning failure, independently of locks retained by the halted initiator.
An ordinary post-panic PMM query could itself deadlock on those locks. No guest
survival or watchdog termination is accepted as evidence of reclamation safety.

## 8. Guest validation

Results and complete commands are in `.kfp-2a1-evidence/guest-results.json`.
The runner freezes source/ELF/ISO hashes, starts at 2 CPUs, then 1/4/12, requires
every unchanged MM-2D marker and stops on failure. This is normal-operation
regression testing, not guest fault injection. Requested remote targets are
CPU count minus one. Individual ACK sequences, quota snapshots and exact backing
identities are not emitted by this fixture. Host termination after all markers
is not a guest shutdown; watchdog termination is failure.


| CPUs | Normal MM-2D | Watchdog | Fixture/boot seconds |
|---:|---|---|---:|
| 1 | PASS | No expiry | 28.03 |
| 2 | PASS | No expiry | 25.79 |
| 4 | PASS | No expiry | 26.03 |
| 12 | PASS | No expiry | 23.78 |

These durations include boot and fixture execution, not shootdown latency.

## 9. Independent lifetime accounting

FI allocations, installed views, retirements, pending obligations, per-CPU ACK
generations, released/retained frames and refunds are **NOT_MEASURED** in guests.
There is no fabricated zero or inference from survival/aggregate free-page drift.
Normal MM-2D asserts local/remote content, exact releases, old-frame guard reuse,
new backing content and PMM restoration; this is narrower than FI accounting.
No physical-frame reuse or refund claim under delivery failure is established.

## 10. Regression validation

Executed: kernel host 185/185; real mailbox 10/10; extracted production ICR
controls 5/5; ring/doorbell/IPC/UAC host libraries 163/163; scheduler host PASS;
FastLane parser controls 137 passed, 67 skipped (not executed security fixtures).
Normal source-bound MM-2D guests passed at 1/2/4/12 CPUs with no watchdog expiry.
See evidence logs/results for executed tests. Kernel host tests cover capability
and VM logic; IPC/UAC/FastLane ring/doorbell host libraries and receipt-parser
controls have distinct scopes. Parser controls are not native M1–M5 security
execution. Full SHM forced revoke, IPC mapping closure, NMI fault, native FastLane
M1–M5 and scheduler guest/performance matrices were not rerun. Expected results
and tests were not weakened or deleted. Existing KFP-2A failures remain preserved.

## 11. Performance

The normal idle helper performs one volatile read; a successful send performs
two reads and two writes, matching the old successful operation's MMIO count.
There is now a finite counter/branch on busy iterations. Host controls assert
these exact counts. They are not a latency distribution or hardware benchmark.
Kernel ICR/ACK/shootdown/reclamation latency samples: zero; p50/p95/p99/max and
failure wall duration are unavailable. No claim of measured performance parity.

## 12. Unresolved properties

Guest FI-1–FI-7, exact failed-retirement accounting, resumable cleanup, exhaustive
activation/cache lifetime proof, lock-order completeness, alternate feature
combinations, hardware/VMware validation and successful-path latency distributions
remain open. A finite software loop does not guarantee a responsive MMIO device
or responsive panic rendering hardware.

## 13. Next milestone

Define a persistent retirement owner and its VM/SHM/exit/quota protocol before
adding operation-level recovery. Separately add a source-bound diagnostic guest
fixture with independent pre-failure receipts and allocator/reuse observations
that do not require locks held by the fail-stopped CPU. Then run the required
2-CPU fault pilot and 1/4/12 expansion. COMPLETE is not justified here.
