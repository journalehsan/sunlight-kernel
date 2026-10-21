# FastLane F1.3N-R: remote scheduler delivery

This is a delivery repair, not a native Doorbell adapter. Acceptance remains
**partial**: functional scheduler/IPI evidence is available, but a post-pin-release
eligibility rearm remains unproven. Matched remote
latency, exhaustive native race coverage, and production-hardware retry-budget
qualification are not established. The machine-readable result is
`target/fastlane-f13nr-evidence/validation-summary.json`. Raw failed experiments
are retained alongside successful runs. No production Doorbell, shared-memory
transport, userspace IPI interface, or capability change is installed.

## Baseline and reproduced boundaries

Baseline: `f7a285955cef31578983e0163ecfca558affc261`, with the developer's existing
F1.2/F1.3 host foundations left in place. The F1.3N audit, its evidence summary,
F1.2/F1.3 contracts, C1.2F tick-tail report, scheduler, LAPIC, IRQ/syscall tails,
IPC terminal wake, foreground lifecycle and native scheduler gate were reviewed.

The historical extracted-production regressions reproduce two **software
boundary limitations**, not measured production task failures:

* `request_reschedule_on` sets intent but does not publish delivery. Live-owner
  wakes and remote enqueue onto a busy CPU can therefore have no interrupt.
* The old flush attempts a source ICR once, restores Busy targets to a global
  mask, and returns. Hardware becoming available performs no software action.
  A later timer, IRQ or syscall tail is required to retry.

Run `python3 tools/fastlane_f13nr_validate.py` to re-extract the historical
functions from the baseline commit and run both old counterexamples and repaired
regressions. `baseline-boundary.log` records the old assertions; the repaired
extraction is `delivery-boundary.rs`. Only the MMIO send boundary is mocked.
The old F1.3N validator intentionally asserts the old limitation and is not a
validator for repaired delivery.

The first native experiment disabled target periodic timers and found another
concrete dependency: on 4/12 CPUs, the first awakened task executed and exited,
then other queued tasks remained unexecuted until the fixture timed out. The
exit path set local intent and halted without delivery. This is a **native
fixture reproduction**, retained in `native/`, not a report of a production
workload failure. `native-repaired/` passes after the exit handoff correction.

## Caller inventory and execution contexts

`request-inventory.json` records exact source lines and request contexts.
Locations below name functions as stable source anchors; the source manifest
and patch in the evidence directory identify exact content. `S` means the global
scheduler guard. Production wake mutations take S; their producers are ordinary
syscalls, timer bookkeeping, input IRQs, or boot initialization. IF is clear on
those entry paths. IPC producers can additionally hold the capability broker
and one endpoint shard. None may wait for hardware while holding those guards.

| Caller/source | Target and old contract | Classification and repaired contract |
| --- | --- | --- |
| `sched/mod.rs::queue_process_to_core` (RR and BORE branches) | Ready task, selected affinity/load target; only remote idle targets got a mask bit. S and briefly one core lock; core lock is released before publication. | Remote busy targets lacked independent delivery. Every external remote publication claims delivery, irrespective of an idle hint. `notify=false` internal rotation still creates neither a request nor a kick. Local publication remains local intent. |
| `tick` (idle/no-current, non-Running, fatal signal, expired slice branches) | Current CPU, S, IF=0, followed by `schedule_tick` in the same timer entry. | Correct local-only request; no redundant IPI. Remote wakes caused by deadline expiry are separate obligations, flushed even when the source has no local request. |
| `retain_priority_request` | Current CPU, S, already inside scheduler selection. The current pass cannot satisfy an additional retained priority wake. | Existing explicit local dispatch bit remains necessary. Flush after handler guard release. No priority/admission change. |
| `wake_native_futex_index`, `wake_linux_futex` | Exact VM/address matching precedes Ready; S; live owner may be remote and still physically executing. | Remote owner needs a kick. Detached task uses ordinary enqueue, preserving affinity and physical stack-pin checks. |
| `wake_pid`, `wake_timer_pid`, `wake_io_pid`, `wake_linux_poll_index`, `wake_for_signal` | State-gated Ready transition, priority wake flag, queue withdrawal; live owner or enqueue; S. | Remote owner no longer gets intent alone. Input IRQ, timer and syscall tails guarantee the source flush. PID lookup semantics are unchanged; no new authority is exposed. |
| `ipc/mod.rs::wake_terminal` | Exact call arbitration precedes Ready; S + existing IPC transaction guards. | Same live-owner/enqueue contract. Delivery happens after all guards are released. No IPC payload or terminal-arbitration change. |
| `terminate_address_space_borrowers`, `terminate_process_by_pid`, `apply_pending_fatal_signal` | Terminal state, queue withdrawal, owner retained until physical handoff; S. Target may be remote. Parent wake uses ordinary wake path. | Remote owner kick; no new task pointer in interrupt metadata. Existing generation, borrower and reclamation checks remain authoritative. Local fatal/tick handling has an imminent scheduler pass. |
| `process/foreground.rs::cancel_foreground`, `terminate_session`; `syscall.rs::send_signal_generation`, `sys_foreground` | Authenticated lifecycle/state changes under S; request each live owner. | Remote kicks through the same primitive. Foreground/session authorization and RecoveryBlocked policy unchanged. |
| `sched/regression.rs::on_tick` forced-exit branch | Debug native lifecycle fixture, S, timer CPU can differ from owner. | Same remote kick; no separate test delivery implementation. |
| `request_park_after_sysret` | Current CPU; atomic-only while S may be held; explicit local dispatch mask. Used for native pipe/TTY parking. | Existing necessary local kick retained. Also used after exit finalization, with flush before the stack pivot/idle transition. |
| `request_reschedule_after_sysret` | Current CPU; IF=0; caller has dropped S. Yield, native/Linux futex wait, poll/epoll and timeout handoffs. Previously used an unbounded synchronous ICR helper. | Publish the existing local park notification and use the bounded common flush. No scheduler lock spans MMIO retry. |
| `request_reschedule` wrapper and syscall request-only sites | Local only: `ipc_call`, `ipc_reply_wait`, `ipc_recv`, `ipc_notify_wait`, `sys_waitpid`, `wait_linux_eventfd`, `wait_pipe_io`, `sys_linux_nanosleep`, TTY read/write block branches. | Correct local-only signaling for this remote-delivery contract. IPC clients explicitly yield after WouldBlock; native pipe/TTY block helpers arm park notifications. Other legacy local waits can still depend on timer/client yield. This repair does not claim timer-independent local parking for every existing syscall. |
| `process_exit`, `interrupts.rs::terminate_current_user_process` | Local intent followed by static-stack idle; no guaranteed immediate pass. Finishing can wake remote parents/peers. | Required local handoff notification and common flush, after finalization drops S and before STI/HLT. IF stays clear until the existing physical stack-pin release. No allocation/reclaim policy changes. |
| `mark_scheduler_ready`, `enter_first_process` | Pre-ready enqueue requests intentionally suppressed AP dispatch during bootstrap. Initial work otherwise waited for AP timers. | Queue release converts existing remote intent to delivery obligations. First-process entry flushes after S release. AP IRQs still cannot select unseeded queues. |

`process/spawn.rs` and native thread/clone syscalls enqueue through the same
external queue function. Keyboard/mouse notifications call `wake_pid`. Timer
IPC expiration calls ordinary terminal wakes. Boot-only scheduler safety tests
operate before scheduler release; their pre-ready intent is handled at release.
There is no supported CPU hot-unplug or logical-to-APIC remapping operation.

All remote paths can target a CPU that is idle, preparing to idle, or returning
from idle: none suppresses the kick based on that state. A request may already
own delivery, in which case no second mask publication is required. Target-task
migration is serialized under S; a late old-CPU interrupt is a scheduling hint,
not authority to touch the moved task. Stack handoff, incarnation and VM identity
checks are unchanged. A future kernel context that publishes remote intent must
supply a guard-free IF=0 flush before returning/halting; the primitive does not
make arbitrary future callers safe automatically.

## xAPIC and bounded source progress

The current driver uses 32-bit xAPIC MMIO: ICR high at `0x310`, low at `0x300`.
Low bit 12 is delivery status. A previous command can keep the **source CPU's**
ICR busy. Each CPU owns its own LAPIC ICR; IF=0 prevents ordinary local IRQs
interleaving high/low writes. The existing TLB NMI handler does not send another
IPI. This is not an x2APIC MSR protocol and makes no claim about x2APIC Busy.
CPU indices equal initial APIC IDs on supported dense-ID platforms, capped at
64. Native fixture markers verify CPUID index against actual LAPIC ID; sparse
APIC IDs and hot-unplug remain unsupported architecture constraints.

`send_fixed_ipi_bounded` tries at most **4096** times, with PAUSE after Busy.
It returns immediately when a low-register write is accepted. The flush handles
at most 64 target bits, outside scheduler, IPC and core locks with IF clear.
Temporary Busy therefore progresses in this same execution, without arming a
periodic timer or requiring a later external flush. No dynamic queue, worker,
retry interrupt, recursive handler, or second wakeup system is introduced.

On exhaustion, the flush restores the failed bit **and all unattempted bits**,
then panics with source and target identity. An out-of-online-range deferred
target similarly restores work and fails before MMIO. This is terminal bounded
failure, not successful recovery under permanent hardware failure. A finite
instruction/attempt budget is not a wall-clock deadline, and a budget exhausted
under virtualization does not prove broken hardware. Production-hardware budget
suitability is unqualified. Other subsystems' existing synchronous LAPIC helpers
are not redesigned by this scheduler repair.

## Request, dispatch and acknowledgement protocol

Each CPU has one atomic byte: `REQUESTED=1`, `DELIVERY_OWNED=2`. This replaces the
old eight-byte global request mask with fixed 64-byte request state; the existing
64-bit dispatch-notification mask is retained. There is no allocation or wrap.

1. Under S, a producer validates its existing task identity and eligibility,
   publishes Ready and queue/live-owner state.
2. A local intent-only request Release-ORs REQUESTED. Remote requests and
   explicit local handoffs AcqRel-OR both bits.
   Only a producer seeing delivery ownership previously clear publishes a bit
   into the existing dispatch mask with Release ordering.
3. After guards drop, a flush AcqRel-swaps that mask to zero. Each claimed bit is
   DispatchPending, then DispatchAttempted during the bounded LAPIC operation.
   Other flushers can claim other newly published bits; no target obligation
   can be taken twice from the same mask publication.
4. Successful ICR writes establish **InterruptAccepted**, not InterruptHandled,
   scheduler selection, or task execution. Neither request nor ownership is
   cleared by the send operation.
5. The reschedule handler records InterruptHandled in the debug fixture, then
   acquires S. `schedule_tick` calls `take`: one AcqRel exchange clears both
   request and ownership at the scheduling opportunity. It observes preceding
   request publication and protected scheduler state. Existing selection rules
   decide which eligible task, if any, can run.
6. User-task ProcessExit on the validated target CPU is a separate execution
   marker. Cleanup is checked after the physical pivot and subsequent reap.

The ownership flag is distinct from request intent: an ordinary local request
cannot suppress the first remote kick. An explicit local handoff owns delivery
and a later remote request shares that already-pending self-IPI. The two bits are acknowledged atomically;
there is no interval in which a newer remote request is cleared independently
from its delivery ownership. A request before the exchange belongs to the pass
that follows it. A request after the exchange retains both intent and a fresh
kick obligation. Real task producers hold S while publishing and consuming
state, so Ready publication cannot race past the same pass's protected decision.
Acquire/Release on the mask transfers the delivery obligation to another source
flusher if it wins the swap. The owner publishes the mask before leaving its
bounded protected operation; process cancellation cannot abandon that obligation.

If a scheduler opportunity consumes a request before its old mask is dispatched,
a stale IPI can still arrive. It is harmless: no task pointer travels in the
mask, and the handler finds no request or handles newer intent. A fresh request
can then produce another mask bit; any duplicate is bounded by publications
across scheduling opportunities. Repeated requests between opportunities share
one delivery claim. Internal queue rotation still uses `notify=false`; it
cannot create the C1.2F unbounded priority-continuation problem.

| Interleaving | Outcome |
| --- | --- |
| First remote request | Claims delivery and publishes one mask bit. |
| Second/multiple sources while owned | Intent retained; delivery coalesces even after acceptance. |
| New request during send | Coalesces if the target has not consumed; claims fresh delivery if it has. |
| New request immediately before acknowledgement | Included in the following protected scheduling decision. |
| New request immediately after acknowledgement | Survives as new request/ownership and mask publication. |
| Busy initially or on consecutive attempts | Same bounded source execution retries. Exhaustion preserves remaining work and fails. |
| Busy plus acknowledgement/new work | Old send may cover new work; any fresh mask remains independently claimable. Controlled host hooks exercise this. |
| Idle entry/exit while delivery pending | Idle hints do not affect kick decisions; fixed vector remains pending across IF=0. |
| Task migration/state change | S serializes queue/owner changes; interrupt rechecks scheduler state. |
| One source, multiple targets | Independent atomic bytes and mask bits; bounded 64-target capacity. |
| Unsupported offline transition | Publication rejects out-of-range targets; flush revalidates before MMIO. No silent redirect/clamping at publication. |

## Remaining critical eligibility boundary

The delivery protocol reaches scheduling opportunities, but that is insufficient
for a task that is not yet physically dispatchable. Source audit finds a possible
interleaving that the native fixture does not force:

1. CPU A pins an outgoing stack, saves its context, drops live ownership and
   releases S, but has not executed the assembly pivot/release store yet.
2. A wake publishes Ready. Load-based target selection may choose CPU B.
3. B handles the repaired IPI, consumes the request, and RR selection correctly
   rejects the context because `stack_is_pinned_remotely` is true. It leaves the
   task in B's queue and can return to idle.
4. A clears its handoff slot after pivot. That aligned Release store publishes
   physical eligibility, but does not itself request another scheduling event
   on B. The existing periodic timer could repair it; no independent rearm is
   established here.

`known_limit_pin_release_does_not_rearm_consumed_request` imports production
handoff and request state and demonstrates the remaining software boundary:
after simulated rejection, consumption and physical release, the task can be
eligible with neither request nor dispatch pending. This is a **host lifecycle
counterexample**, not a reproduced native production failure. General queue
selection is not executed by that small model. Native forced before/after-pivot
coverage remains necessary to establish actual reachability and repair it.

No stack-pin safety check is removed. Repeated self-IPIs while waiting for a
remote pin could create an interrupt storm; a callback inserted into a naked
post-pivot epilogue requires a separate stack/alignment and lock-order proof.
Simply publishing a continuation bit also has a race with physical release
unless registration/recheck and post-release consumption are coordinated.
A source-CPU reschedule continuation could avoid a naked-epilogue callback,
but still needs retained registration and post-release consumption to be proved
against migration, cancellation and acknowledgement. It is not implemented or
assumed here. Routing every pinned task to its old CPU is not generally correct after an
affinity change. These are not installed as unverified workarounds.

Consequently the complete Ready-to-execution liveness guarantee remains
unproven, despite repaired request and Busy dispatch and successful native idle
wakes. The existing handoff behavior is retained, acceptance stays partial, and
the F1.3N adapter is not ready. This is a concrete lifecycle blocker, not merely
a missing performance measurement.

## Idle transition

Common BSP/AP idle runs on established static stacks: CLI, account idle,
`sti; hlt`, then CLI/account interrupt completion. Wake publication no longer
consults whether `core_current_task` currently looks idle. Before CLI, an IPI
handler can run and select work under S. After CLI, an accepted fixed vector
stays pending; STI's interrupt shadow covers the immediately following HLT,
so pending delivery wakes/interrupts the halt. After HLT, the fixed vector
interrupts it normally. A stale no-work interrupt followed by a new request
cannot erase the new request: consumption precedes the protected selection,
and a later request obtains fresh ownership. The exit path delivers while IF
is clear and only enables interrupts after moving off the outgoing task stack
and releasing its existing handoff pin.

The fixture publishes an idle-entry marker in this exact CLI-to-STI window.
Repeated source events race with idle preparation, but the runs do not force
both orders of every physical instruction interleaving. Host tests establish
that idle hints cannot suppress delivery; native exhaustive idle-race coverage
is explicitly unverified. No claim that an accounting marker alone proves a
physical HLT instruction is made.

## Deterministic regressions

`tools/fastlane_f13nr_boundary.py` extracts current production publication,
bootstrap, flush and bounded-send bodies and imports production request state.
Controlled MMIO outcomes cover local no-kick, remote kick, local pending intent
followed by a remote request, pending/in-flight coalescing (including remote sharing of an explicit local
self-IPI), bootstrap release,
1/3/4095 Busy outcomes, 4096-attempt exhaustion, preservation of unattempted
targets, stale/offline targets, multiple sources/targets, and request publication
inside dispatch before/after acknowledgement. The host catches panic only to
inspect retained state; production does not resume from that failure.

The existing scheduler service host fixture now imports production request
state, so C1.2F priority-continuation tests also exercise delivery ownership.
Its obsolete standalone request boolean and an unused queue-probe import were
removed; no alternate dispatcher was introduced.

Production request-module tests additionally exercise every supported target,
coalescing, post-ack retention and two threads separated by barriers. No random
sleep is used. The exact progress assertions are accepted target mask != 0 in
one repaired flush, and attempt count == Busy count + 1 without any second
external flush. The historical model instead asserts zero delivery for request
only and zero retry when hardware becomes available without a later flush.
Cancellation of a CPU delivery is not supported; task cancellation follows
existing lifecycle validation at scheduling time.

These tests prove software state transitions, not physical APIC delivery. They
also do not prove arbitrary callers eventually return from a stuck critical
section or that hardware delivers a command forever despite CPU failure.

## Native scheduler evidence

Build/run with `python3 tools/test_remote_wake_smp.py --cpus 1 2 4 12
--evidence-dir target/fastlane-f13nr-evidence/native-frozen`. The `remote_wake_test`
feature and all instrumentation/injection hooks require debug assertions and
are absent from release code. It reuses the existing isolated scheduler ELF
loader, task creation, affinity, wait/wake, ring-3 probe, exit and reap machinery.
Role 2 in the existing assembly probe exits immediately; roles 0/1 of the
lifecycle gate retain their previous spin/yield/cookie behavior. No syscall is
added.

The target is CPU 1 on SMP and CPU 0 on UP. Other CPUs are actual sources,
checked against their LAPIC IDs. Each source wakes its own preallocated blocked
task with affinity restricted to the target. The target enters the normal idle
loop and masks its periodic LAPIC timer. Sources mask theirs before the wake's
return-tail flush. Later rounds use explicit fixture control IPIs to establish
the next source event; they do not retry an abandoned Busy dispatch. The target
finishes four rounds, then a post-pivot control opportunity verifies every
fixture slot Reaped, no kernel stack/pin/queue membership, no deferred or
requested bits, and exact free-frame restoration. Test continuation events are
distinguished from the actual target wake event.

The source ICR mock branch arms three consecutive Busy outcomes per source
per round. Only sources that own a dispatch consume that injection; coalesced
requests need no attempt. The eventual accepted send is real xAPIC MMIO, the target handler
really runs, and the ring-3 task exits on the target. Hardware Busy is separately
counted; zero observations cannot establish physical hardware-busy coverage.
The source timers cannot serve as later retry flushes after publication.

The 1-CPU first wake is handled in its already-entered timer scheduling pass;
no remote wake is claimed. Subsequent local exit/fixture continuations need
self-IPIs. Native counters scope attempts/accepted to the bounded dispatcher
when its destination is the target; handled/opportunities are target-only.
Requests count **fixture task wake publications**, not every source-control or
exit request. Rates cannot be computed from these different milestone counts.

| CPUs | Task wake requests | Attempts / accepted | Handled / opportunities | Executed | Deferred / unresolved | Free frames before / after |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 (local) | 4 | 20 / 8 | 5 / 5 | 4 | 0 / 0 | 68241 / 68241 |
| 2 | 4 | 20 / 8 | 8 / 9 | 4 | 0 / 0 | 67983 / 67983 |
| 4 | 12 | 34 / 16 | 16 / 17 | 12 | 0 / 0 | 67471 / 67471 |
| 12 | 44 | 60 / 45 | 45 / 46 | 44 | 0 / 0 | 65423 / 65423 |

All rows use xAPIC. Native per-CPU IA32_APIC_BASE reads assert enable bit 11 set and x2APIC bit
10 clear; BSP reads `0xfee00900` and APs read `0xfee00800`. Actual sources are `{0}` on 1/2 CPUs, `{0,2,3}` on 4,
and `{0,2,3,4,5,6,7,8,9,10,11}` on 12. Executions are on CPU 0 for UP and
CPU 1 for SMP. Injected Busy counts are 12/12/18/15; observed hardware Busy is
zero in every row. Exact per-run injected counts are in the summary;
coalescing determines which sources actually consume an armed injection. Accepted and handled can differ because vectors coalesce;
these are different milestones, not task execution acknowledgements.

`native-frozen/{1,2,4,12}cpu.serial.log` and its hashed ELF/ISO metadata are the
final source-frozen matrix. `validation-summary.json` extracts actual source
IDs, task execution CPU IDs, request/send/handler/opportunity/execution counts,
pending/unresolved counts and before/after free frames per configuration. The
failed eight-round preallocation experiment in `native-repeated/12cpu.serial.log`
hit the existing 64-entry address-space ledger at task 65. Four rounds require
44 task address spaces on 12 CPUs; no resource limit was raised to fit the test.

## Compatibility, measurement and handoff

`compatibility-results.json` indexes raw authority/exec, resident security/SHM,
scheduler lifecycle, VM/TLB/unmap/protect, D1/D2/D3 and RecoveryBlocked results.
Unsupported CPU configurations are recorded, rather than extrapolated. These broader compatibility gates and C1.2F captures preceded the final
local-delivery ownership hardening. The final delta was checked by production
scheduler host tests, target/release checks and the 1/2/4/12 native wake matrix;
the entire broader matrix was not replayed after that delta. This limitation
also prevents final compatibility acceptance. Native
TCP/TLS logs are separate and must contain certificate-verification assertions;
a generic resident pass is not network evidence. DialogD/SessionD protocol and
blocking implementations are untouched; resident startup checks are not a new
dedicated blocked-wait latency experiment for those services.

C1.2F capture/analysis and any available before/after results are indexed by
`performance-results.json`. TCG functional fixture runs print transitions and
are unsuitable for hot-path timing. No remote p50/p95/p99/max, rates, clock
synchronization, lock-duration change, CPU saving or power saving is inferred
from them. Cross-CPU TSC comparability is not validated. The normal 22-trial capture passed its structural validator; steady external
wake p50/p95/p99/max were 0.145/1.038/2.378/131.038 ms. The eight-trial mixed
capture also passed, with 0.177/1.248/2.405/120.918 ms. These are single captures
under concurrent host test load, not a demonstrated before/after improvement;
extreme tails remain unresolved without a matched comparator. The C1.2F
observer uses same-CPU timestamps and its established validity checks; historical timings
alone are not a matched baseline for this repair.

Kernel/IPC/service, ring/Doorbell and scheduler host tests, target builds, strict
Clippy, changed-file formatting and diff checks are recorded separately.
Baseline archive checks reproduce unrelated strict Clippy, formatting and
missing-WAV failures without resetting the working tree. Large existing
formatting changes in `sched/mod.rs`, `main.rs` and `syscall.rs` are not folded
into this repair. New/changed fragments follow rustfmt; request, LAPIC, fixture,
accounting, IRQ and build files pass independent formatting checks. Miri remains
unavailable. Raw command exits take precedence over this narrative.

The implementation establishes a bounded software delivery continuation and
has real remote scheduler evidence. It is **not yet accepted as completing all
F1.3N-R requirements**, and the separate native Doorbell adapter should not be
enabled automatically. Outstanding prerequisites include a race-safe post-pin-release eligibility
continuation, forced native handoff coverage, qualified retry-budget
behavior on production hardware, stronger native idle/ack race instrumentation,
reliable matched performance/tail validation, and any failed/unrun compatibility
items identified in the summary. The adapter still independently requires exact
registration identity, cancellation/exit integration and its lifecycle tests;
restricted production SHM and grant prerequisites remain separate work.
