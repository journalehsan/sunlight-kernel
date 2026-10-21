# FastLane F1.3N: native Doorbell scheduler boundary audit (partial)

**F1.3N is not implemented or accepted.** This change completes a source audit
and adds executable host counterexamples for the existing dispatch-delivery
boundary. No kernel Doorbell, new scheduler primitive, syscall, native fixture,
or native channel is installed. N2–N8 remain outstanding. Existing F1.2/F1.3
uncommitted sources are preserved. Restricted SHM is not needed for a trusted
kernel fixture; its incomplete authority is not the reason that fixture is
absent here.

Reproduce with `python3 tools/fastlane_f13n_validate.py`. Results, raw command
logs, extracted Rust test source, toolchain identity, tracked patch, source
hashes, and explicit skipped native assertions are under
`target/fastlane-f13n-evidence/validation-summary.json`. A passing validator
means the listed host checks passed, **not** that F1.3N passed.

## N1: existing primitives and locks

The audited baseline comprises the F1.3 specification/evidence summary, F1.2
specification/ring implementation, and C1.2F tick-tail report, plus these sources:

| Source / primitive | Contract and reuse constraint |
| --- | --- |
| `arch/x86_64/syscall.rs::ipc_recv` | Takes SCHEDULER, then capability broker, then endpoint shard; empty check and blocking registration share those guards. Returns WouldBlock for retry. |
| `ipc/mod.rs::handle_ipc_recv`, `IpcBus::block_on_recv` | Queue check precedes BlockedOnIpc publication without releasing SCHEDULER/shard. Removes runnable membership and records block tick. Not a standalone generic wait object. |
| `sys_native_futex`, `sched::wake_native_futex` | Atomic value check, wait address/deadline and BlockedOnIo publication under SCHEDULER; wake scans exact address-space identity/address. Useful atomic conditional-wait pattern, private address-space semantics. |
| `ipc::finish_call`, `wake_terminal` | Existing generation-qualified call arbitration, then Ready publication. Does not establish a new Doorbell's lifetime or identity. |
| `sched::wake_pid`, `wake_io_pid` | State-gated Ready transition, priority wake flag, queue removal, live-owner handling or enqueue. PID-only lookup is insufficient for new object effects. |
| `queue_process_to_core` / `enqueue_ready` | Only Ready tasks without live/queue ownership enter queues. External enqueue requests dispatch; deferred IPI publication is limited to remote idle targets. A busy target can receive only a request bit. |
| `live_owner_core`, `schedule_tick` | Blocked bookkeeping can coexist with physical execution. Wake marks Ready without remote insertion for a live owner. Dispatch observes Ready and requeues it after saving context; it does not unconditionally rewrite Ready to Blocked. |
| `sched/handoff.rs`, interrupt epilogues | Outgoing stack pin is acquired before ownership is detached; naked assembly releases after stack pivot. Selection/reclamation reject a remotely pinned context. Preserve this protocol. |
| `request_reschedule_on`, `sched/requests.rs` | Release-published per-CPU request, AcqRel consumption by that CPU. Requests arriving after consumption remain pending. This is a scheduling request, not an interrupt. |
| `flush_dispatch_notifications` | AcqRel swaps deferred mask; at most one nonblocking IPI attempt per target; failed attempts are restored. Called after scheduler guards are dropped, with interrupts disabled. |
| `lapic::try_send_fixed_ipi` | Returns false if ICR is busy; true means writes accepted, not remote handler entry or task execution. No retry source is installed here. |
| `lapic::send_fixed_ipi`, `request_reschedule_after_sysret` | Existing synchronous ICR-idle waits and self-vector handoff. Hardware send completion is not remote task acknowledgement. No bounded busy-ICR proof for a new adapter follows from this helper. |
| `terminate_process_by_pid`, `apply_pending_fatal_signal`, self-exit, borrower teardown, reap | Publish terminal process state, remove queues, retain live ownership/pins until physical handoff, reclaim later. No Doorbell registry is visited. Every path would need exact-identity withdrawal. |
| `process/foreground.rs`, `Scheduler::add_process` | Checked incarnation allocation; allocation failure forces exit. Reaped slots can be reused. Object generation and task incarnation must both be checked under SCHEDULER. VM generation and IPC-call generation serve different purposes. |

Lock hierarchy on the audited receive path is
`SCHEDULER -> CAP_BROKER -> one IPC shard`. Scheduler queues use
`SCHEDULER -> one CORE_STATES lock`. Work stealing uses victim `try_lock`,
drops it before taking the thief lock, and never holds two core locks. This is
an audit of these paths, not a claim that every subsystem lock has been checked
for all possible inversions. New Doorbell state should be owned by Scheduler,
with no additional object spinlock, callback that reacquires SCHEDULER, or guard
held across physical context switching. LAPIC delivery belongs after guard
release. PMM/VM/reap paths retain their established locks and ordering.

Timer IRQ and reschedule-vector dispatch both acquire SCHEDULER and invoke
`schedule_tick`; request retention and C1.2F priority admission must remain
unchanged. A request does not promise immediate execution of an equal/lower
priority task. Affinity, queue ownership and physical stack pins constrain
migration and stealing even after Ready publication.

## The exact unverified delivery boundary

`tools/fastlane_f13n_boundary.py` extracts the actual bodies of
`reschedule_bit`, `request_reschedule_on` and `flush_dispatch_notifications`;
it imports the production request-mask module. Only LAPIC access is simulated.
The tests execute serially against controlled busy/available hardware state.

1. Publish a CPU-1 request and flush: request remains pending, zero IPI attempts.
   This reproduces the primitive used by live-owner wake paths. No task is
   created by this host test.
2. Publish a CPU-11 request and deferred bit with busy ICR, then flush: one
   attempt, deferred bit retained, zero sends. Make ICR available without
   invoking another kernel tail: no autonomous attempt occurs. An explicitly
   invoked later flush accepts the send and leaves the request pending.
3. Repeated publications for CPUs 1 and 11 coalesce into two attempts;
   consumption of CPU 1's request preserves CPU 11's request.

These are positive assertions of a **limitation**, not native failing-wakeup
observations. Numeric target IDs in the mock are not executing CPU identities.
They establish that these primitives alone do not provide timer-independent
prompt delivery. They do not prove that a safe adapter is impossible or that
a large scheduler redesign is necessary. A minimal event-driven delivery and
retry contract, plus native validation, remains work to do. Simply setting the
deferred mask for busy targets fixes only the request-only case. Replacing the
bounded flush with an unbounded ICR spin does not establish bounded behavior.
No periodic fallback, synchronous remote task acknowledgement, or altered
scheduler policy was added to bypass this boundary.

## Proposed adapter contract, not implemented

Use fixed-capacity Scheduler-owned slots, each containing the existing host
Doorbell model and a checked, nonrepeating channel generation. Derive task
`{pid, foreground.incarnation}` from live Scheduler state, never caller-supplied
untrusted identities. One exact consumer, sole producer, and trusted owner;
one waiter per object; reject duplicate registration. Fail slot reservation
before modifying process wait state. Exhausted generations retire permanently.

The model's sticky terminal reason implements a direct Open-to-Closed
transition. A native Closing state is needed only if storage retirement must
wait for references. Proposed operations borrow slots under SCHEDULER and
consume effects before unlocking, retaining no raw task pointers or asynchronous
borrowed object references. Retirement must resolve waiter registration, process
wait token and any outstanding effect before slot reuse. A fixed inline value
does not by itself prove scheduler queue allocation or PMM behavior is bounded.

Proposed internal operations are snapshot, notify, conditional wait, close and
cancel. No public syscall or native authority contract is proposed here. A
process wait token must identify object slot/generation and registration epoch;
PID/incarnation alone cannot distinguish cancellation followed by a new wait
on another object within the same task lifetime.

## Wait/notify linearization and handoff proof obligations

```text
consumer CPU                          SCHEDULER                     producer CPU
snapshot sequence S                   lock/read/unlock
Acquire-drain Ring; observe empty
                                      lock
validate task incarnation + open + S
install object waiter + process token
publish Blocked, remove queue         W: registration/block commit
request local scheduling opportunity
                                      unlock
                        <-- wake may arrive before physical switch -->
                                                                   lock
                                                    validate producer + object
                                                    increment sequence       N
                                                    detach exact registration
                                                    validate process token
                                                    publish Ready            R
                                                    retain owner OR enqueue
                                                    request dispatch
                                                                   unlock
                                                    deliver interrupt        I
save outgoing context, honor Ready/Blocked, pin/pivot/release
selected / resumed                    E (distinct from R and I)
Acquire-drain Ring again
```

W and N must be serialized by the same lock. Notify before W changes sequence,
so wait returns Changed without blocking. Notify after W detaches the waiter
and publishes Ready. Between Blocked publication and physical switch, live
ownership prevents duplicate insertion; `schedule_tick` handles the raced
Ready state. After ownership detachment, ordinary enqueue plus stack-pin checks
protect physical context lifetime. No caller may later write an unconditional
Blocked state. Notification during the locked transaction must wait for its
completion; deterministic hooks inside that transaction cannot wait for a
notifier that needs the same lock.

N linearizes at checked increment; sequence MAX instead commits terminal
Exhausted and a wake obligation. R is scheduler-visible under the same lock.
Close linearizes at first terminal assignment, rejects future wait/notify and
detaches at most one waiter. Unlock publishes the completed transaction.
Delivery I, handler entry, selection E and consumer resume are separate
milestones. The state argument covers absence of a stale Blocked overwrite;
it does **not** prove I or E. The missing delivery contract prevents promoting
this proposed argument to a verified native implementation.

## Lifecycle rules still to implement

Close must consume its effect under SCHEDULER before retirement. Repeated close
preserves first terminal reason. Cancellation must detach the exact token and
either safely wake the live blocked caller or complete its termination; merely
removing the waiter strands it. Notify/cancel must arbitrate in one transaction.
Unrelated signal wakes must withdraw registration before a different wait can
be installed. Exit must withdraw before process-slot reuse on **all** terminal
paths, including forced fatal signals and address-space-owner borrower exit.
Sole producer exit publishes ProducerExit and wakes a live consumer; consumer
exit removes its registration without waking the dead consumer. Stale
incarnations do nothing. No ownership adoption or RecoveryBlocked policy change.

Generation exhaustion and sequence exhaustion must preserve the host model's
terminal behavior; neither counter may wrap. Kernel reservation failure and
rollback need native assertions. The host model's boolean reservation input
does not exercise native resource exhaustion.

## Evidence and unfinished validation

The machine-readable summary is authoritative for command exit codes. Logs
separate host Doorbell/ring suites, kernel host logic, IPC/services, scheduler
production-state tests/validators, target compilation, formatting, Clippy, media
fixture failures and Miri availability. Pre-existing workspace formatting and
missing WAV failures must not be repaired in unrelated modules for this phase.
No Rust production file changes, so there are no changed Rust files to format;
foundation formatting is an additional baseline check. Kernel strict Clippy is
reported independently, without suppressing existing warnings.

The recorded run passes five delivery-boundary tests (three counterexamples/
controls plus two production request tests), eight Doorbell tests, five Ring
tests, 91 kernel host tests, and 115 IPC/service host tests. Scheduler host
checks and 21 Python validator tests pass. Kernel target check, foundation
target build, strict foundation Clippy and foundation formatting pass.
Strict kernel Clippy fails on existing dependency/build-script lints, including
`sunlight-sensors`, `sunlight-tui`, and the kernel build script. Workspace
formatting fails. Media has 36 passes and one missing-file failure for
`docs/songs/onaldin_music-catch-the-sunlight-333176.wav`. `cargo-miri` is not
installed for the current nightly. Full logs preserve further diagnostics.

All 18 requested native transition assertions are **not reached**, recorded
individually as skipped. Native 1/2/4/12 CPU configurations were not run: there
is no Doorbell fixture to execute. No raw native Doorbell serial logs exist.
No actual distinct executing CPU pair, migration race, physical blocking,
remote execution, exit cleanup, idle wakeup count, teardown frame accounting or
native latency distribution is claimed. Host boundary tests cannot replace
these results. Native userspace and cross-process FastLane remain absent.

Resident, native C1.2F, VM/TLB/SHM retirement, exec, D1/D2/D3, RecoveryBlocked,
and network TCP/TLS gates were not rerun for this audit-only change and are
explicitly deferred in the summary. Passing source-only regressions does not
revalidate them. C1.2F's historical timing is single-vCPU and cannot supply
cross-CPU latency or clock comparability for this phase. No performance
threshold, maximum tail, CPU saving or power saving is asserted.

## Handoff prerequisites

First establish a bounded, retained event-driven remote dispatch delivery
contract, including busy ICR, busy target, IF-disabled target, local park and
post-handoff requests. Validate it independently with actual CPU placement and
physical execution markers before installing the adapter. Then implement the
fixed-capacity exact-registration adapter and complete lifecycle integration;
run deterministic native transitions and the full CPU/regression matrix.
Measure same-clock intervals first; cross-CPU timestamps require an independent
comparability check. Keep serial output outside timed paths.

Kernel-owned Ring integration can follow without unrestricted SHM: producer
Release-publishes head before notify; consumer snapshots, Acquire-drains, waits
conditionally, then Acquire-rechecks. Doorbell ordering never replaces cursor
Acquire/Release. Test FIFO/integrity, capacity/backpressure, burst coalescing,
drain/sleep, and close with queued data using owned backing whose lifetime
outlasts both peers. This native integration is deferred here.

Production channel prerequisites remain restricted backing provenance,
per-peer protections without writable aliases, capability issuance and native
lifecycle authority, active revocation, exec/exit/PID reuse, and acknowledged
TLB retirement. F1.1B-0 and R4.1 remain blocked. No unrestricted SHM substitute,
production transport, Audio migration or generalized MPMC/futex redesign.
