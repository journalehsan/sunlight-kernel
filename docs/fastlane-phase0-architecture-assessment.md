# Sunlight FastLane IPC: Phase 0 architecture assessment

Date: 2026-10-18. Audited revision: `d91ddb65a5223c11045a5ec85bf2206f0e889e68`
(`fix(sched): remove tick dependency from priority wake dispatch`), including the
pre-existing working-tree manifest/version changes. This phase adds this
assessment only; it changes no runtime code or syscall ABI.

## Decision

Build FastLane as an optional userspace transport over the existing shared-memory
and capability infrastructure. Preserve synchronous kernel IPC for channel
negotiation, authorization, capability transfer, service lifecycle, and privileged
operations. Reuse the existing frame allocator, mapping ledger, TLB shootdown,
endpoint identity, and scheduler ownership rules.

The repository has usable shared-memory backing and persistent mappings, but
does **not** yet provide a secure, revocable, lost-wakeup-safe cross-process
channel primitive. A userspace ring alone would not satisfy the requirements.
The necessary prerequisites are recipient/permission enforcement on lane
mappings, explicit channel lifecycle, and a working authenticated notification
contract. These should extend existing kernel facilities rather than create a
second authorization system.

Start with a bounded single-producer/single-consumer (SPSC) transport and a
synthetic two-process workload. Audio PCM submission is the best initial service
candidate, subject to measurement. Do not migrate services during this audit.

## Existing IPC and authority

The relevant source is [kernel IPC](../kernel/src/ipc/mod.rs),
[register messages](../kernel/src/ipc/message.rs),
[capability broker](../kernel/src/capability/mod.rs),
[syscall dispatch](../kernel/src/arch/x86_64/syscall.rs), and
[userspace IPC](../ipc/src/lib.rs).

* `ipc_call` takes `SCHEDULER`, then `CAP_BROKER`, then the destination IPC shard.
  `handle_ipc_call` checks SEND authority and the endpoint owner, enqueues a
  request, records a generation-tagged pending call, and blocks the caller.
  Queue capacity is 64. There are 16 shard locks; the global scheduler lock
  still serializes the call path.
* `enqueue_call` overwrites the badge with the caller PID. Service authorization
  must continue to use this kernel identity, plus service-specific policy.
  Shared-memory fields claiming a PID, UID, endpoint, or generation are not
  credentials.
* Receive and reply-wait require receive rights and actual endpoint ownership.
  An ordinary server has one current reply target; up to 64 deferred replies
  provide explicit tokens for delayed completion. A lane should not consume
  synchronous reply slots for each data item.
* Replies, deadline expiration, cancellation, and peer closure arbitrate under
  `SCHEDULER`. Call generations reject stale replies; a terminal result is
  consumed once. A completed call can now be collected under the scheduler lock
  without taking the broker/shard locks.
* The register ABI transports **four u64 words and two token words**, although
  `IpcMsg` has eight logical word slots. Register decoding clamps counts. A new
  negotiation must fit the actual wire budget or use a checked SHM descriptor;
  changing count validation is separate compatibility work.
* The broker resolves opaque bearer tokens. `derive` checks rights attenuation
  and preserves endpoint identity; public lookup uses send authority. Passing
  `msg.caps` transports token values, not recipient-bound SHM grants or an
  automatic ownership handoff. `can_grant` exists, but the ordinary IPC message
  path does not turn payload tokens into a new grant protocol.
* Endpoint IDs use checked monotonic allocation and are never reused. Destroying
  an endpoint checks owner/receive authority and revokes all its endpoint tokens.
  `revoke(token)` removes that token only; `derive` does not store a parent-child
  revocation tree. FastLane must explicitly define its dependency on endpoint
  destruction, individual grant revocation, and setup-token revocation.

Channel setup can therefore be an ordinary authorized service request, with the
service deciding whether the caller may open the requested data path. Kernel
binding must validate the live participants and their authority from kernel
state. A client must not gain mapping, wake, or management authority merely by
writing a lane identifier into shared memory.

## Kernel entries and costs today

| Operation | Current mechanism and cost boundary |
| --- | --- |
| Endpoint creation, bind, discovery, management | Syscalls/control-plane IPC; broker and service checks remain necessary. |
| Synchronous call | Syscall 1; message decoding/copying, capability lookup, scheduler/shard locking, queueing and caller blocking. |
| Receive / try-receive | Syscalls 4 / 18; owner checks and queue access. Empty blocking receive changes scheduler state. |
| Reply / reply-wait | Syscalls 2 / 3; target validation, reply storage and caller wake; reply-wait can combine a reply and next receive. |
| Timed IPC | Clock/deadline setup and call/receive syscalls; the deadline uses kernel monotonic time and wake transitions. |
| Userspace IPC retry | `ipc_call`, receive and notification wrappers yield and retry on WouldBlock. Submission, yield and terminal collection can each require kernel entry. Exact counts depend on dispatch timing. |
| SHM allocation | Syscall 92; frame allocation and zeroing, ledger/page-table work, broker publication and reference tracking under scheduler/PMM/broker locks. |
| SHM mapping / release | Syscalls 93 / 94; page-table/ledger operations and reference tracking. Release invalidates translations and frees backing only at the final mapping. |
| Access to an existing SHM mapping | Ordinary userspace loads/stores; no per-access syscall. This is the reusable FastLane foundation. |
| General notification | Syscalls 5 / 6 exist, but their handlers do not implement a usable send/wait protocol. |
| Private futex wait/wake | Syscall 155; compare/register is serialized, but waiters are scoped to one address-space identity. |

The small IPC payload is transferred through registers and copied into kernel
queue/reply structures; it is not a bulk byte-copy path. Bulk services separately
copy to/from SHM and private buffers. Context switching and Ready-queue delay are
distinct from syscall instruction cost, and neither is removed simply by
eliminating a payload copy. Shared cache-line traffic, fences, polling CPU time,
and buffer occupancy replace some costs in a userspace ring.

There is no general lock-free kernel IPC fastpath to extend. The obsolete
eligibility stub described in older reports has already been removed. The spawn
special case called a fast path in `ipc_call` is unrelated to data transport.
No measured percentage of current IPC time can be assigned to copying, mapping,
locking, or scheduling from the available evidence alone.

## Shared memory, mapping protection, and frame lifetime

Source: [shared-memory implementation](../kernel/src/memory/shared.rs),
[address spaces](../kernel/src/process/address_space.rs),
[region ledger](../kernel/src/process/region.rs),
[TLB invalidation](../kernel/src/memory/tlb.rs), and broker `ShmEntry` methods.

`alloc_shared_region` already handles multiple pages, checked rounding, frame
sanitization, rollback, and publication after the owner mapping succeeds. Shared
frames receive the `SharedMemory` accounting class. `map_shared_page` maps the
entire object despite its historical name. Both paths record mappings and
increment broker map counts. Reuse these mechanisms.

`shm_map_sized` already returns the **kernel-verified object capacity** through
the current map syscall. This improves on the older IPC performance report's
length-checking gap; a second size-query facility is unnecessary. Protocol
offsets, sizes, versions and arithmetic still require checking against that
capacity. `shm_create` returns a requested-size mapping rounded to pages.

Current limitations are concrete:

1. `sys_shm_map` resolves a token without a recipient check. Every SHM mapping
   is RW/NX; its reserved protection argument is not enforced. Allocation flags
   do not establish a rights policy either. Possession of a legacy SHM token
   therefore permits mapping the whole object RW.
2. A caller cannot repair that policy using ordinary `mprotect`: the ledger's
   `preflight_protect` currently permits protection changes only for eligible
   anonymous mappings. Parameterizing the existing SHM mapping path and checking
   broker grants is the relevant boundary.
3. `mapped_shared` and `owned_shared` are task records even when threads share
   an address space. The address-space ledger/cursor is shared, but mapping
   release ownership is not a general process-wide safe handle abstraction.
   Preserve task affinity initially; do not add blanket `Send`/`Sync` wrappers.
4. `shm_free(token)` releases the newest local view. This deliberately protects
   older persistent mappings from temporary map/copy/free operations, but a
   lane must track which view it owns and avoid accidental alias release.
5. The SHM address cursor is monotonic in a 4 GiB virtual range. Successful
   frees do not reuse holes. Persistent lanes reduce churn, but repeated
   reconnects can still exhaust virtual space. Bounded object/mapping quotas
   and reconnect testing are needed; a general virtual allocator rewrite is
   not a prerequisite for the first bounded experiment.
6. Reference counting preserves peer mappings when the owner frees or exits.
   This prevents premature frame reclamation; it does not implement channel
   revocation or prove the peer has finished accessing data.
7. `revoke_all_for(pid)` marks owned SHM entries revoked, but its only current
   call is the manual security self-test. Production exit/reap uses mapping
   cleanup, not this revocation operation. An object kept alive by a peer can
   remain resolvable after its owner exits.
8. `revoke_shared` removes broker metadata without unmapping existing views or
   reclaiming their frames. It is used with explicit cleanup in the self-test;
   it cannot serve as live lane revocation. Marking a token revoked likewise
   does not remove existing PTEs, and normal `free_shared_page` resolves only
   non-revoked objects. A live-revocation implementation must retain accounting
   metadata until every affected view is safely released.

SMP-safe synchronous TLB shootdown already exists. `unmap_page` calls
`tlb::invalidate_page` using the address-space identity before backing can be
reused. Extend this path for revocation; do not implement a second shootdown
scheme or free frames while another CPU can still access a mapping.

## Notifications: current guarantee and required contract

`ipc_notify_send` returns success without validating or recording anything.
`ipc_notify_wait` ignores its endpoint token and marks the caller BlockedOnIpc
without testing pending notification state. These handlers cannot guarantee
delivery or absence of lost wakeups. Userspace `notify_send` also discards errors;
`notify_wait` retries. Their public names are not evidence of working semantics.

Working notifications exist for **kernel timer and input producers**.
`send_timer_tick` and `send_input_notification` maintain queued/deferred
coalescing state in the existing endpoint queue. If its 64 slots are full, the
event remains deferred and is materialized later. Timer notifications carry an
accumulated tick sequence. Producers publish under `SCHEDULER` plus the endpoint
shard and wake the receiver; receive checks the queue and blocks under those
same locks. That serialized check/block boundary closes the receive race.
It is useful implementation precedent, not a userspace-callable lane doorbell.

Other mechanisms were checked to avoid duplication:

* Native and Linux futex wakeups match `(address_space_identity, virtual_address)`.
  Native compare/register is atomic relative to kernel wake/VM mutation, but
  mappings of the same SHM frames in different address spaces do not share this
  key. Neither futex ABI currently provides cross-process SHM-object waiting.
* [Linux eventfd](../compat-linux/src/eventfd.rs) has real counted-event semantics
  and descriptor lifetime tracking. Native pipe/TTY paths also have functioning
  blocking and dispatch support. There is no existing native endpoint-capability
  operation that hands an eventfd/pipe to an independently established FastLane
  peer. Exporting one would require additional authorization/transfer work and
  would not by itself solve the userspace arm/publication race.

The preferred next primitive is to complete the existing authenticated
notification send/wait facility, using broker endpoint authority and scheduler
wait/wake ownership. Confirm its caller compatibility before finalizing semantics.
Use a dedicated endpoint for the first lane direction, with coalesced pending
state separate from ordinary RPC reply targets and bounded independently of a
full ordinary-message queue. An endpoint/channel sequence or pending predicate
must be kernel-owned; a userspace flag alone is not a pending-event record.
Do not reuse timer/input event labels as arbitrary client notifications.

The contract must establish all of these properties before service use:

* Only authorized senders may signal; only the owner/authorized waiter may wait.
  Invalid tokens return errors, rather than a successful no-op or blind block.
* A signal arriving before wait registration remains observable. The kernel
  atomically checks pending state and either consumes/observes it or registers
  the waiter while serialized with signal and closure.
* The userspace empty-to-sleep handshake also closes the gap between observing
  an empty queue and entering that kernel wait. Arm, recheck, publication, and
  signal suppression need an explicit memory-order proof/model. Independent
  release/acquire stores to a cursor and an armed flag are not sufficient to
  rule out both sides missing each other's update.
* An active receiver and available capacity require no syscall per transfer.
  A producer only needs the doorbell on the proved sleeping/arming slow path;
  a sleeping/full producer needs a symmetric capacity-release wake protocol.
  Ring cursors remain authoritative after any coalesced or spurious wake.
* Close, revoke, timeout and cancellation compete with wait registration under
  the same lifecycle synchronization. A stale wake cannot target a new lane,
  new address space, or reused task slot. Sequence exhaustion must be defined.

Do not commit a new syscall number or claim the above contract already exists.
Completing the reserved notification operations is preferable to inventing an
unrelated FastLane-only sleep syscall unless implementation evidence requires it.

## Scheduler and C1.2F implications

Reviewed [C1.2F report](kernel/scheduler-phase-c12f-tick-tail.md),
[trial CSV](kernel/scheduler-c12f-trials.csv), and the available
[retained evidence index](../target/scheduler-c12f/evidence.md). The timings below
are historical reported measurements, not new FastLane results.

C1.2F shows a Ready timer could wait for additional periodic interrupts because
a retained reschedule bit did not itself deliver an interrupt. The fix publishes
the existing coalesced dispatch-notification mask for an unsatisfied external
priority wake. Timer/reschedule handlers flush after dropping locks, with
interrupts disabled. The LAPIC try-send is bounded; busy delivery retains the
mask. `priority_wake_pending` distinguishes real external wakes from internal
rotation, preserving residual quantum and positive-nice progress.

Reported steady timer wake-to-selection p50 changed from 20.396 ms to 0.198 ms,
with final p99 2.404 ms and maximum 54.861 ms. This measures wake-to-selected
dispatch, not physical IRET, end-to-end RPC, or an SMP latency guarantee.
One mixed trace commits a reply promptly but the equal-nice caller collects it
about 1,400 ms later behind CPU workers retaining their turns. Queue transport
alone does not change that scheduling policy.

C1.2F also gated an unconditional 4,194,304-entry PMM diagnostic scan on the
cleanup/dispatch path. The reported long interval included substantial host
vCPU runnable wait as well as costly guest IRQ-off work. It must not be presented
as a byte-copy or IPC transport cost. All timing traces were one-vCPU QEMU TCG;
the 1/2/4/8/12-vCPU lifecycle gates were functional validation.

In current source, `wake_pid` and native futex wakes mark Ready and set
`priority_wake_pending`. A live-owner wake can only set that owner's reschedule
bit; enqueue explicitly adds an interrupt for a remote **idle** CPU. These paths
do not establish prompt remote **busy** CPU wake delivery. Completing a lane
doorbell must deliberately audit that path and flush existing notifications
after dropping locks. A flag's publication is not a dispatch-latency guarantee.
Preserve affinity, live ownership, stack pins, terminal-state rejection, nice
policy, and residual credit. No scheduler class, priority donation, or default
priority boost is justified by this audit.

## Termination, exec, and revocation boundaries

Source: [process state](../kernel/src/process/mod.rs),
[scheduler lifecycle](../kernel/src/sched/mod.rs),
[exec/spawn](../kernel/src/process/spawn.rs), and
[address-space generations](../kernel/src/process/mm2b_state.rs).

Task-vector slots are reused. Spawn paths normally choose a PID above existing
PIDs; PID allocation conventions alone are insufficient as a future lane
identity contract. Address-space identities already include a checked generation;
threads can share that identity, while exec replaces it. IPC call generations
identify calls, not process incarnations or shared-memory channels.

The reaper removes endpoint queues and caller references, revokes owned endpoint
tokens, finishes peer-closed calls, and releases SHM mappings. Reclamation waits
for live-core/stack ownership and active-address-space safety; address-space
owners also account for borrower threads. The self-exit path closes endpoints
before final reaping; externally terminated tasks may need deferred reaping.
Successful exec cleans the old address space's SHM mappings. Reuse these hooks,
but explicitly attach lane state: endpoint cleanup currently has no lane to close.

Future kernel channel records must bind participating tasks and address-space
generations, their mapping views, controlling endpoint, rights, and a distinct
channel incarnation. Channel IDs are references into broker-validated state,
not independent bearer authority defined by the userspace ring library.
Close/revoke must stop new attachments/waits, resolve both data and space waiters,
and prevent stale service restarts from inheriting the old stream. Define which
participant owns control authority and what each grant revocation invalidates.

Keep mapping/frame accounting until outstanding views are gone. Graceful detach
can quiesce userspace references before unmapping. Forced revocation must define
the fault/termination behavior for an actively accessing peer; a safe Rust API
must not promise that a borrowed buffer survives asynchronous unmapping.
A timeout or peer death is never a consumed-buffer acknowledgement.

## Workload suitability

| Workload | Observed path | Assessment |
| --- | --- | --- |
| Audio PCM submission | `submit_pcm_chunk` retains one acknowledged 4 KiB client page. Server `submit_pcm` maps it, validates, copies into its private PCM queue, releases its mapping, then replies. Errors retire the client page. | Strong first candidate for fixed bounded slots and persistent server mappings. Preserve private queue/DMA ownership initially; measure map/unmap and synchronous submission overhead, pump deadlines, underruns and CPU. A lane is not a DMA buffer. |
| Audio capture | Audiod owns a bounded capture queue and copies periods to a caller SHM page, with capture owner checks. | Separate direction and protocol. Preserve device-source controls, period loss accounting, stop/drain semantics, and capture authorization; do not infer suitability solely from playback. |
| Compositor frame publication | UI maps a surface once. `commit_with_desktop_overlay` may copy a hidden draw half to the visible half, then calls `COMMIT_FRAME`. Display acknowledges before marking damage/redrawing and checks the kernel badge against the window owner. | Existing pixels already avoid per-frame mapping. A lane could batch damage/frame descriptors, but cannot claim to remove rendering or the UI copy. Define surface-generation changes, resize/close, frame ownership, pacing, overlay authority and completion before changing acknowledgement behavior. |
| Network socket bulk transfer | `SEND_SHM` maps/copies to a Vec/unmaps; `RECV_SHM` maps, obtains owned data, copies and unmaps. TCP manager checks caller badge plus socket identity. NIC frame proxy syscalls 90/91 remain a separate kernel path. | Potentially useful for a bounded per-socket buffer pool, after measuring packet sizes and batching. Preserve connect/bind/close and socket ownership in the control plane. A userspace lane does not remove stack/NIC copies or authorize DMA. |
| VFS data transfer | Native VFS server uses inline replies up to 16 bytes; larger reads copy `big_buf` into newly allocated SHM and return `DATA_SHARED`. Its source comment calls this zero copy despite that copy. | Candidate for sustained bulk streams, not metadata or small reads. Define handle/access authority, offsets, EOF/errors, cancellation and returned-buffer ownership. Audit both client and server release paths; this server is not every kernel/libc file-I/O path. |
| Small control requests and privileged operations | Compact register IPC already carries commands and checked identities. | Retain kernel IPC. Setup/pinning/cache traffic would often outweigh a lane's benefits, and privileged operations still require kernel enforcement. |

The September [IPC performance audit](ipc-performance-audit.md) establishes
operation-count reductions, not latency gains. For N acknowledged audio chunks,
client alloc/free falls from 2N to 2 over its lifetime, while server map/free
remains 2N. Its host mock tests and QEMU boots do not establish playback quality
or hardware SMP speedup. The [responsiveness audit](2026-09-16_IPC_RESPONSIVENESS_AUDIT.md)
also documents SHM leaks and alias-release fixes that persistent views must retain.

No existing general userspace SPSC transport with the needed rights/wait/lifecycle
contract was found in the audited IPC and service paths. Device DMA rings,
kernel pipe rings, audiod's private queue and same-process worker mailboxes serve
different ownership domains. Solar's `shm_pool.rs` is a proof of concept: acquire
returns a dummy token and release is unimplemented. It is not a reusable channel.

## Proposed implementation boundaries and sequence

These are proposed increments, not APIs already present or implementation
performed by this phase.

1. **Freeze a measured baseline and wire/lifetime contract.** Use two independent
   native processes and existing IPC/SHM. Specify one producer and one consumer
   per direction, fixed capacities and quotas, version negotiation, mapping
   ownership, close behavior and fallback. Keep setup within four wire words
   plus two tokens, or validate a sized SHM descriptor. Separate data-available
   from capacity-available events.
2. **Add the pure no_std SPSC layout and validation logic to the shared IPC
   crate.** Use fixed-width offsets/lengths, never process pointers; checked
   layout arithmetic; bounded counters with specified wrap behavior; separated
   writer-owned cursors; release publication and acquire consumption. Slot
   release acknowledges completed reading/copying. Return Full/Empty explicitly;
   do not overwrite live slots or busy-spin indefinitely. This stage can be
   host-tested without exposing an operational native lane.
3. **Extend broker/mapping enforcement for restricted lane views.** Keep existing
   SHM object allocation/refcounts. Bind grants to live recipients and generations;
   check read/write/management rights at map time and enforce them in PTEs. If
   cursors and payload have different writers, place them on separable pages.
   Prevent the legacy unrestricted RW map path from remapping a restricted lane
   object with greater rights. Preserve legacy SHM ABI behavior for existing
   objects through an explicit compatibility distinction. Roll back setup failures
   and bound objects, mapped bytes, and view counts before publishing a lane.
4. **Complete notification and lifecycle integration.** Use existing notification
   entry points, endpoint authority, scheduler transitions and dispatch mask.
   Prove kernel pending-check/register and userspace arm/recheck/signal ordering,
   including space wakes. Add close/revoke/exit/exec hooks and generation checks;
   retain backing until safe unmap/shootdown and reference release complete.
   Test actual cross-core races, not only serialized state-machine permutations.
5. **Enable a synthetic native channel, then one opt-in audio experiment.** Attach
   through kernel IPC and retain the existing PCM-copy/DMA boundary. During
   active, non-full transfer, verify zero transport syscalls per item. Validate
   the blocked/full path independently. Keep legacy negotiation/fallback available.
   Do not automatically replay an uncertain in-flight submission through fallback;
   completion or a new stream generation must resolve duplication/ownership.

Peer memory is untrusted even when attached through an authorized channel.
Read-only receiver mappings do not stop the producer changing its own writable
bytes. Validate copied descriptor snapshots against immutable local capacity and
generation state; reject corrupt cursor distances, slots, lengths and offsets.
Do not return ordinary Rust references with unproven concurrent-mutation or
revocation lifetimes. A reviewed shared-memory access representation and explicit
trust/immutability rules are prerequisites to claiming safe zero-copy payload
borrowing; volatile accesses alone do not provide synchronization. Retaining a
bounded copy into server-owned memory is appropriate where those rules cannot
yet be enforced. Limit each service's drain work to preserve control-plane and
other-client progress.

No IPC rewrite, global scheduler-lock redesign, new capability namespace,
general MPMC queue, NIC/audio DMA handoff, new scheduler class, or service-wide
migration is required for this first implementation.

## Validation and evidence

Existing instrumentation can be reused: IPC `diagnostic_snapshot` counts depth,
queue full, coalescing, deadlines, cancellation and late/peer-closed results;
`sched/perf.rs` has bounded per-CPU counters/histograms for scheduling, switches,
queueing and lock timing. Current lock histograms observe selected scheduler
paths, not all IPC syscall/broker/shard costs. Add bounded per-CPU observations
at missing boundaries; avoid per-message serial logging or a new global metrics
lock. Retain calibrated-clock validity checks.

Existing functional gates include `phase_shm`, `phase_sec`, phase 2.6 and scheduler
lifecycle gates. The kernel host library is `mm2a_host.rs`: it does **not** compile
and execute `kernel/src/ipc/mod.rs` as a host IPC test suite. The timer marker
`round-trip test: 1000 calls OK` is printed after 100 ticks, not 1,000 measured
calls. The boot `IPC SMP terminal-state stress` executes transitions under the
scheduler lock; booting with multiple CPUs does not make it a concurrent race test.

Acceptance for subsequent increments must include:

* Host tests/model exploration for wraparound, Full/Empty transitions, slot
  reuse, malformed layouts/cursors/descriptors, and both wake handshakes. Include
  deliberately weakened ordering variants to show the tests detect a missed wake.
* Native authority tests for forged/wrong-class tokens, unauthorized attachments,
  read-only write faults, legacy-map escalation attempts, recipient/generation
  mismatch, quota exhaustion, and complete allocation/map rollback.
* Native lifecycle/race tests for signal-before-wait, signal-during-registration,
  last-item drain versus sleep, full-ring release versus producer sleep,
  coalescing under queue pressure, timeout versus close, live revocation,
  process death/exec, thread borrower exit, endpoint recreation and task-slot
  reuse. Test active peer access and delayed cleanup without premature frame reuse.
* Repeat functional tests at 1/2/4 CPUs, then the supported scheduler lifecycle
  matrix. Measure same-core, remote idle and remote busy receiver placement;
  exercise both peers under CPU saturation and mixed nice values. Check bounded
  drain fairness, producer backpressure, shutdown and control-plane responsiveness.
* Compare identical workloads using legacy IPC, persistent SHM plus existing
  control IPC, and FastLane. Report throughput, bytes/messages, syscall count,
  copies/mappings, context switches, CPU time including polling, buffer occupancy,
  and p50/p95/p99/max data and wake latency. Separate setup cost and amortization
  break-even from steady state; report idle CPU cost and right-censored waits.
  Trace publication, Ready, selected dispatch and user receive separately.
* Preserve source/image hashes, feature flags, raw logs, workload sizes, CPU
  placement, repetitions and host conditions. Report QEMU TCG limitations and
  measure hardware separately before asserting production latency or audio gains.

Phase 0 checks run against the existing working tree:

| Check | Result |
| --- | --- |
| `cargo test --locked --offline -p sunlight-ipc -p sunlight-audiod --lib --target x86_64-unknown-linux-gnu` | PASS: IPC 42 tests; audiod 25 tests. |
| `python3 tools/test_scheduler_host.py` | PASS: focused production state/lifecycle/accounting/policy tests. |
| `cargo check --locked --offline -p sunlight-kernel --bin sunlight-kernel` | PASS; dependency warnings remain. |

Local logs: `/tmp/fastlane-phase0-host.log`,
`/tmp/fastlane-phase0-scheduler-host.log`, and
`/tmp/fastlane-phase0-kernel-check.log`. Scheduler host evidence is also written
by its existing runner beneath `target/scheduler-b-evidence/`. These are local
generated evidence, not committed artifacts. No new QEMU boot or performance
trial was run for this documentation-only phase, and no FastLane speedup is claimed.

Phase 0 is complete. Runtime implementation starts with the isolated transport
contract and host model, followed by the existing broker/mapping/notification
extensions above; secure native service rollout depends on those prerequisites.
