# FastLane F1.3: Doorbell specification and host foundation (partial)

F1.3 adds `sunlight-fastlane-doorbell`, a dependency-free `no_std` serialized
state model, and an isolated host adapter combining it with the F1.2 ring.
**No kernel Doorbell adapter or syscall is implemented. No native FastLane
channel exists.** This phase is partial: host correctness evidence is useful,
but acceptance requiring native scheduler blocking/waking is unmet.

The existing uncommitted F1.2 work was preserved. Sources and commands for this
phase are recorded under `target/fastlane-f13-evidence/validation-summary.json`.
Run `python3 tools/fastlane_f13_validate.py` to reproduce bounded host checks.
The validator never enables native data transport. Raw artifacts are untracked.

## Existing foundation audit

Audited sources include `kernel/src/arch/x86_64/syscall.rs`, `ipc/mod.rs`,
`sched/mod.rs`, `process/mod.rs`, capability lifecycle code, F1/F1.1/R4.1 and
F1.2 documents, scheduler hardening and C1.2F documentation. The baseline F1.2
summary records five passing test groups and 1,130,000 concurrent messages;
this phase separately reruns the existing ring tests.

| Mechanism | Implemented behavior and limits | Reuse assessment |
| --- | --- | --- |
| Notify syscalls 5/6 | `ipc_notify_send` returns success without sending; `ipc_notify_wait` marks BlockedOnIpc without a matching notification object/sequence/authority check | Unsuitable; syscall numbers are not a foundation |
| Blocking IPC receive | `handle_ipc_recv` checks queue and calls `block_on_recv` with mutable Scheduler and bus; syscall holds scheduler and IPC shard through both | Existing atomic check/register/state pattern is suitable; not a standalone Doorbell |
| IPC completion | `finish_call`/`wake_terminal` arbitrate generation-tagged outcomes, remove wait state and publish Ready; live owner gets reschedule request, otherwise enqueue | Preserve terminal arbitration and C1.2F priority wake behavior |
| Private native futex 155 | `sys_native_futex` reads atomic u32 and registers BlockedOnIo under SCHEDULER, keyed by address-space identity and address; deadline/range cancellation supported | Atomic registration pattern useful; cannot join independent address spaces |
| Linux futex | Address-space identity/address wait key, separate thread wait metadata | Also not a cross-process shared-object Doorbell |
| Runnable publication | `wake_pid`, `wake_io_pid`, native futex wake clear wait metadata, set Ready and `priority_wake_pending`; remove old queue entries; live owner handled separately | Existing primitives must be used under scheduler lock, with exact incarnation checked before PID-only helpers |
| Remote dispatch | `request_reschedule_on` publishes per-CPU bit only; enqueue interrupts remote idle CPUs via deferred notification mask; live-owner wake can request without publishing an IPI | Cross-CPU runnable transition exists; prompt busy-CPU execution is not established for a new Doorbell |
| Deferred interrupts | `flush_dispatch_notifications` consumes bounded CPU mask after locks, tries IPI once and restores failed bits; comment explicitly retains periodic-timer fallback | No strict prompt-delivery proof; do not interpret a bit as a delivered interrupt |
| Timer sleep | BlockedOnTimer plus deadline scanning; private futex deadlines use monotonic ns | Real implementation, unsuitable as notification polling workaround |
| Cancellation/closure | `handle_ipc_cancel` removes exact call; `finish_peer_closed_calls` matches PID, incarnation, endpoint and generation before terminal wake | Good existing IPC lifecycle discipline; does not automatically cover new Doorbell objects |
| Process termination | Finished state/removal from queues under scheduler; remote live owner requested to reschedule; reclamation deferred until physical handoff | Must integrate Doorbell exit cleanup before any new native wait; not done here |
| Authority/incarnation | Capability broker checks rights; IPC calls carry foreground incarnation and checked call generation; process/VM identities are distinct | No PID-only authorization; model identity comparisons are not capability checks |

Scheduler state changes and existing wait scans use the global SCHEDULER lock.
IPC syscall paths also acquire broker/shard locks in their established orders;
no Doorbell callback may reacquire SCHEDULER while it is held. Scheduler/core
order remains S -> one core. Timer interrupt and syscall tails may observe a
logically blocked task still physically executing: `live_owner_core` handling
and physical handoff are essential, not optional queue details.

C1.2F explicitly limits its timing evidence to single-vCPU and documents
request-only remote wake paths. The audit establishes that an atomic wait can
be expressed under the existing lock; it does **not** prove that a native
Doorbell is impossible or requires scheduler redesign. What remains unverified
is the complete generation-qualified park/wake/termination adapter with prompt
remote interrupt delivery, including a busy ICR and a still-executing waiter.
That prerequisite is left unresolved here. A kernel-owned fixture could test
it without restricted SHM and should be the next scoped step. Existing private
futex tests and prior native IPC gates cannot substitute for those assertions.

## State and authority specification

`Channel { slot, generation }` identifies a model object; generation must be
nonzero. Owner, sole producer and sole consumer are exact `{pid, incarnation}`
pairs with nonzero incarnation. Sequence starts at zero. There is one optional
waiter and one sticky terminal reason. All transitions require exclusive access.
The type contains no pointers, queues, heap owners or callbacks. Size on the
validated x86_64 host is 104 bytes. Creation is one inline value; a false
reservation flag fails Capacity without changing state. That flag models a
trusted bounded allocator decision; it is not an implemented allocator or quota.

`snapshot` and `wait` require the consumer; `notify` requires the producer;
`close` and `revoke` require the owner. Every operation checks exact channel
identity before role. `peer_exit` is a trusted lifecycle input, never an
untrusted operation. The model's public identity values are **not security
credentials**. It is not linked into the kernel. A native adapter would derive
identities from authenticated scheduler state, hold validated object authority,
reject withdrawn capabilities, and allocate nonrepeating checked generations.
This library does not issue generations or prevent a trusted owner reusing one.

A mismatched slot/generation returns Stale; wrong role/incarnation returns
Unauthorized. Terminal operations reject new waits/notifications with the
sticky reason. Repeated owner close returns the existing terminal reason and
no additional wake. There is no adoption or replacement of either peer.
RecoveryBlocked policy and production process behavior are unchanged.

Sequence uses checked addition. A notification at MAX transitions to Exhausted,
detaches and returns the waiter as a terminal wake obligation. It returns an
Effect with `terminal=Exhausted`, never a successful sequence increment. All
later waits/notifies fail terminal. No modular ABA is allowed. The owner must
retire this generation; it cannot reset the sequence in place.

## Conditional wait and linearization

The library returns wake obligations; it does not execute scheduler operations.
A kernel adapter MUST consume those obligations before releasing serialization.
Returning an Effect then asynchronously calling a PID-only wake is insufficient.

```text
Consumer                         Shared serialization             Producer
snapshot sequence S             lock / unlock
Acquire ring head; empty
                                 lock
check live + sequence == S        |
register exact waiter            |     notify cannot enter here
commit Blocked state             |
                                 unlock + atomic park
                                                                 lock
                                                        increment sequence
                                                        detach exact waiter
                                                        publish Runnable
                                                                 unlock
                                                        deliver pending IPI
resume; Acquire ring head again
```

Model notify linearizes at sequence increment, terminal notify/close at first
terminal assignment, and wait at the final sequence comparison and waiter
assignment within one exclusive transaction. Kernel blocking state MUST be
committed in that same transaction. There must be no later unconditional
Blocked write. A notify arriving before physical park publishes Ready; handoff
must honor that state. Host `Mutex<Doorbell>` plus `Condvar::wait` implements
atomic unlock-and-park; it loops on registered state to tolerate OS spurious
wakes. No new timeout or periodic polling implements consumer sleep.

If notify precedes snapshot, a completed message is observed by the subsequent
ring Acquire under the host synchronization contract. If notify follows
snapshot and precedes wait, sequence differs and wait returns immediately.
If wait wins serialization, the notifier detaches the registered waiter and
must make it runnable. Preemption during either locked transaction merely
delays the other operation; it cannot expose half-registration. Multiple
notifications advance sequence but wake at most one registered waiter; further
notifications need no redundant runnable publication. Cancellation removes the
registration under the same lock. A spurious wake requires ring/lifecycle
recheck and a fresh wait transaction.

The deterministic test enumerates positions before snapshot, between snapshot
and ring check, between ring check and wait, and after registration. “During
registration” is serialization exclusion, not a simulated concurrent mutation
inside the critical section. Before-park, blocked and remote positions are
**abstract model orders only**. They do not validate actual kernel preemption,
remote CPUs, scheduler queues or physical context switches.

## Ring memory ordering and future integration

Producer: obtain authorized channel; write bounded payload into free slot;
Release-publish head; then notify. Payload writes need no individual kernel
transition. Initially notify every successful publication; suppressing wakes
based only on a stale empty/nonempty guess is not proven safe. Consumer:
snapshot sequence; Acquire-drain ring; if empty conditionally wait against that
snapshot; after every return recheck terminal state and Acquire-drain again.
F1.2 Acquire of head establishes payload happens-before from producer Release;
Release tail and producer Acquire tail establish safe slot reuse. Notification
sequence is a separate synchronization domain and never substitutes for head
Acquire. The host mutex additionally orders producer notification with later
snapshot/wait; a future kernel ABI must explicitly supply compiler and CPU
ordering across publication/syscall/snapshot and mapped atomic accesses. Merely
invoking a wake syscall is not a Rust shared-memory proof. Current supported
ring architecture remains little-endian x86_64 with lock-free 64-bit atomics.

Full returns backpressure without overwriting. The host producer yields on
Full solely to run a bounded synthetic fixture. A blocking production producer
would require a separate reverse-direction space-available Doorbell/protocol;
data-available notification does not implement it.

Native integration still requires restricted backing provenance, per-peer
RO/RW/NX mappings, prevention of legacy writable aliases, active revocation,
verified exit/exec/PID-reuse and VM lifecycle, safe close and acknowledged TLB
invalidation. R4.1 production VFS/UAC delegation and F1.1B-0 remain blocked.
No shared ring header authorizes a kernel operation. No legacy SHM substitute,
production service migration, priority change or new futex architecture occurs.

## Closure, death and bounded lifetime

Close/revoke publish terminal then take the single waiter once. Producer exit
is terminal because it is the sole future source; it wakes the consumer with
ProducerExit. Consumer exit removes its waiter without waking a dead task and
terminates the object. An unrelated or later incarnation exit does nothing.
First terminal wins notify/close/exit races. Model cancellation is a detached
registration, not terminal channel cancellation; the owner can revoke to make
cancellation terminal. Close is terminal immediately: buffered ring data is not
implicitly promised drained. A higher layer may drain already published data
under still-valid backing lifetime, but must specify this policy explicitly.

Exclusive borrows protect inline model storage until operations finish; host
threads borrow the adapter through scoped joins. There are no scheduler-held
references in this implementation. A future kernel object cannot be reclaimed
until waiter detachment, pending wake consumption and all in-flight operations
are quiescent; exact incarnation checks must occur under scheduler serialization.
Real process cleanup, authority revocation and slot generation retirement must
use the same transaction as wait setup. This lifetime contract is specified,
not implemented natively. No survivor adoption is introduced.

## Validation and measurements

Eight host test groups assert sequence/result/waiter transitions, not just lack
of panic. They cover empty wait, multiple/coalesced notifications, duplicate
registration, cancellation, spurious retry, wrong identity/generation/role,
reservation failure, sequence exhaustion, all terminal reasons in both legal
notify orders, duplicate close and 10,000 creation/retirement cycles. One hundred
actual host notify/close races serialize through the mutex. Host registered
waiters are explicitly observed before terminal wake and before first ring
publication. External test timeout is the bounded failure detector.

The ring fixture exchanges 20,000 16-byte synthetic sequence/complement payloads
through capacity two; asserts FIFO, integrity, final publication receipt and no
remaining waiter; explicitly fills the ring and checks Full. It closes only
after final drain. Registered idle close and producer-death tests independently
assert terminal returns. No arbitrary sleep is used to establish interleavings.

Host measurements are in `host-tests.log`: 10,000 timestamped notify calls and
1,000 registered wait/wake samples. Thread-create-to-registration includes
thread creation and host scheduling, so is not isolated blocking setup cost.
Notify-to-wait-return includes mutex release, OS scheduling and return overhead;
it is not native wake-to-runnable latency. Instant is monotonic; actual clock
resolution is uncalibrated. No affinity/isolation, serial logging in timed paths,
priority changes or IPC comparison. Exact percentiles vary per validator run.

The inline model has zero metadata allocations by construction; each terminal
iteration asserts no waiter. That proves bounded model state, not whole-kernel
allocator drift. Mutex/Condvar/thread allocations are outside this claim.
Native context switches/notification, idle wakeups/s, spurious-wake counts,
coalescing rate, lock hold durations, native runnable/execution latency and
resource drift are unavailable. No CPU/power improvement is claimed. Consumer
waiting has no polling by design; host producer Full retry is fixture-only.

| Classification | Result |
| --- | --- |
| Implemented and host-model validated | Serialized no_std state/identity specification; eight host test groups; ring fixture |
| Native kernel-fixture validated | None |
| Native userspace validated | None |
| Implemented but insufficiently validated | No native adapter implemented; model is not a scheduler primitive |
| Blocked prerequisite | Verified native park/wake/termination handoff and prompt remote delivery; production authority/restricted mapping boundary |
| Deferred | Kernel fixture; production channel; full native compatibility matrix; native latency/resource instrumentation |

For each required native test (no waiter, unchanged wait, notify before/during/
after registration, remote notify, blocked close, task exit, stale generation,
repeated lifecycle), status is **host-model only** where represented above;
actual kernel test is unavailable because no adapter was implemented. Native
1/2/4/12 CPU runs are **not run**, never passing boots without assertions.

Fresh checks include kernel host tests, IPC/audiod/sunlightd host tests, scheduler
host suite, kernel target check, ring tests, Doorbell target build, strict Clippy,
changed Rust formatting and diff checks. Machine-readable results are
canonical. Native resident, scheduler SMP, VM/TLB, SHM retirement, D1/D2/D3,
RecoveryBlocked, process exit/exec and TCP/TLS are not rerun: production kernel,
IPC and service sources were not modified. Prior R4.1 runs are historical
baseline evidence, not F1.3 validation. Existing workspace-format differences
and missing WAV are separate baseline issues. Miri remains unavailable; no
model checker or dynamic memory-model validation is claimed.

The native foundation acceptance criterion is unmet. The next step is a
kernel-owned synthetic fixture using the existing scheduler lock and real
wait/Ready transitions, exact identities, deferred post-lock IPI delivery and
exit cleanup, with barriers proving physical handoff races on 1/2/4/12 CPUs.
This can proceed independently of restricted SHM but must be verified before
claiming a kernel Doorbell foundation or completed FastLane IPC.
