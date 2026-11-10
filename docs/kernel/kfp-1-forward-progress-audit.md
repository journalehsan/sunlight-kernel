# KFP-1 — Kernel forward progress and blocking audit

## 1. Executive summary

**Verdict: PARTIAL.** Inspected 34 execution-path families. No critical progress violation was reproduced or conclusively established. Five suspected defects/conditional hazards remain; fourteen mandatory properties below remain unproven. These counts do not imply that every kernel lock site or service protocol has been exhaustively verified.

Kernel syscall execution is nonpreemptible on its CPU: FMASK clears IF at entry and SYSRET restores it. Blocking generally means publishing a blocked state, releasing transaction guards, returning to Ring 3, and letting a pending scheduling interrupt select another task. It does not mean suspending an arbitrary Rust syscall stack in the middle of a wait. Futex explicitly replays WAIT until a serialized completion detaches it. Normal single-CPU blocking therefore has a credible CPU-release mechanism; a kernel spin or device poll still monopolizes that CPU.

SMP can mask stalls: another CPU can execute userspace while CPU0 is unable to advance the global clock, or while a remote CPU cannot receive a maskable dispatch IPI. A global scheduler-lock holder can turn a local delay into cross-CPU contention. Aging cannot help when scheduling cannot execute.

The shootdown implementation deliberately uses both fixed IPIs and NMI mailbox consumption. Its normal reclamation order is fail-stop and conservative, not timeout-and-free. However, its finite acknowledgment budget does not include unbounded ICR delivery polling. No production fixes, ABI changes, policy changes, enablement changes, commit, or push occurred.

## 2. Scope and source revision

Audited working-tree source at HEAD `9ec5a16484fe5d7024898737ca6c896f3345d6cf`, on 2026-11-10. HEAD alone does **not** identify this source: pre-existing modifications include syscall entry/dispatch, main, memory, workspace manifests, VFS and tmpfs. `.kfp-1-evidence/current/status-before.txt`, `preexisting.patch`, and `source-sha256.json` identify the inspected state. Findings describe this dirty working tree, not an assumed clean release.

Included native and Linux-personality dispatch, scheduler queues/aging/affinity, futex/lifecycle, synchronous IPC, feature-gated FastLane adapters, SHM/TLB, reclaim, device-backed file operations, timers and notification entry points. Helios Ring 3 implementation is excluded; user lock/service dependencies are conditional edges, not kernel execution stalls by themselves. Feature-gated paths are identified separately from production entry points. Existing security/lifecycle fixtures were inspected, not reconfigured or rerun.

This is a broad source audit with focused existing host tests, not a complete concurrency proof. No guest binary was built or booted in this milestone. Old guest receipts were not promoted to evidence for the current dirty source. See sections 12–14 for the remaining work.

## 3. Blocking-path inventory

The full required fields (entry, condition, mechanism, completion owner, scheduler/interrupt/preemption state, retained resources, bound, failure and evidence) are in [blocking-paths.json](../../.kfp-1-evidence/current/blocking-paths.json). One row denotes a call-chain family; repeated mutex sites are grouped. Caller-dependent states mean UNKNOWN outside the stated syscall/scheduler context. Spin mutex fairness and eventual owner completion are not assumed.

| ID | Path family | Source entry | Mechanism / completion dependency | Bound |
|---|---|---|---|---|
| B01 | syscall entry/return | `kernel/src/arch/x86_64/syscall.rs:204` init, syscall_entry, syscall_dispatch | nonpreemptible execution; current CPU | none for whole syscall |
| B02 | scheduler lock | `kernel/src/sched/mod.rs:4415` schedule_tick entry/callers | spin mutex; remote lock holder | none |
| B03 | runqueue/core locks | `kernel/src/sched/mod.rs:1600` pick_next_round_robin | spin mutex; lock holder | none |
| B04 | native futex WAIT | `kernel/src/arch/x86_64/syscall.rs:3014` sys_native_futex | BlockedOnIo + SYSRET replay; peer WAKE, timer, signal, VM retirement | absolute tick-derived deadline or none |
| B05 | Linux futex WAIT | `kernel/src/arch/x86_64/syscall.rs:3354` sys_linux_futex | BlockedOnIo + SYSRET replay; peer WAKE, signal, VM retirement | none; nonzero timeout rejected ENOSYS |
| B06 | futex completion scan | `kernel/src/sched/mod.rs:2104` wake_native_futex, wake_linux_futex, expire_native_futex_timeouts | bounded process scan; current CPU under scheduler lock | process count |
| B07 | synchronous IPC call | `kernel/src/ipc/mod.rs:1170` handle_ipc_call via ipc_call | BlockedOnIpc; userspace retry; endpoint service/reply, close, cancel, deadline | optional deadline; otherwise none |
| B08 | IPC receive/reply-wait | `kernel/src/ipc/mod.rs:1523` handle_ipc_recv, handle_ipc_reply_wait | BlockedOnIpc; sender, close, optional deadline | optional deadline |
| B09 | IPC cancellation/closure | `kernel/src/ipc/mod.rs:1928` handle_ipc_cancel; remove_endpoint at 570 | serialized queue drain; current CPU | queue/process population |
| B10 | waitpid | `kernel/src/arch/x86_64/syscall.rs:3753` sys_waitpid | BlockedOnIpc + EAGAIN; child termination | none; explicit NOHANG extension |
| B11 | thread join/reaped probe | `kernel/src/arch/x86_64/syscall.rs:3110` sys_native_thread_reaped | nonblocking probe; userspace retry; owner CPU epilogue/reaper | none across user retries |
| B12 | termination/reaping | `kernel/src/sched/mod.rs:2769` reap_process_resources | defer reclamation; no in-place wait; owning CPU context-switch epilogue | no wall bound |
| B13 | stack handoff pin | `kernel/src/sched/mod.rs:960` wait_for_pin_release | atomic waiter registration; skip candidate; remote epilogue | no wall bound |
| B14 | native pipe read/write | `kernel/src/arch/x86_64/syscall.rs:6170` native pipe registration helpers | BlockedOnTimer + park after SYSRET; peer read/write or endpoint closure | none |
| B15 | native poll | `kernel/src/sched/mod.rs:2408` block_current_native_poll | BlockedOnTimer; pipe/TTY producer or timer | optional ticks |
| B16 | Linux poll/epoll | `kernel/src/arch/x86_64/syscall.rs:7148` sys_linux_poll and readiness helpers | BlockedOnTimer + replay/retry; event producer or timer | optional ticks |
| B17 | TTY read/write | `kernel/src/sched/mod.rs:2422` block_current_tty_read/write | BlockedOnTimer + park after SYSRET; tty service/input IRQ/drain/hangup | none |
| B18 | Linux nanosleep | `kernel/src/arch/x86_64/syscall.rs:7974` sys_linux_clock_nanosleep, sys_linux_nanosleep | timer block; BSP global clock + scheduler | requested duration in ticks |
| B19 | remote wake notification | `kernel/src/sched/mod.rs:4313` flush_dispatch_notifications | finite hardware retries; pending IPI; APIC then target CPU | 4096 attempts per target; target execution unbounded |
| B20 | TLB shootdown | `kernel/src/memory/tlb.rs:199` invalidate | busy poll; target fixed IRQ or NMI | 100000000 ack spins AFTER delivery |
| B21 | APIC delivery | `kernel/src/arch/x86_64/lapic.rs:145` wait_for_icr_idle, send_fixed_ipi, send_nmi | unbounded spin; local APIC hardware | none |
| B22 | TLB activation snapshot | `kernel/src/memory/tlb.rs:119` activate; ActiveCpuSlot::current | atomic retry; mutator/slot writer | none under continual mutation |
| B23 | SHM view retirement | `kernel/src/memory/shared.rs:284` free_shared_page | synchronous unmap/shootdown; remote CPU NMI/IRQ | per-leaf TLB bound excludes ICR |
| B24 | FastLane restricted retirement | `kernel/src/capability/channel_mapping.rs:32` fastlane_mapping_sweep/drain_mappings | population-bounded drain plus TLB waits; current CPU/remote NMI | 8 rows/3 backings/two authorized VMs in adapter; TLB caveat |
| B25 | Doorbell conditional park | `kernel/src/sched/doorbell_native.rs:32` doorbell_wait_if_unchanged | BlockedOnIo + token/outcome; producer notify/close/cancel/exit | none |
| B26 | FastLane channel Doorbell | `kernel/src/capability/channel_doorbell_native.rs:121` fastlane_bell | conditional park; authorized producer/retirement | lifecycle/deadline dependent |
| B27 | anonymous allocation reclaim | `kernel/src/memory/swap.rs:501` maybe_reclaim_for_anonymous_allocation | synchronous bounded reclaim/compression; current CPU; may require TLB | shared page budget and local TSC deadline |
| B28 | user copy/demand backing | `kernel/src/memory/user.rs:146` validate_pages, back_demand_page | page walk and synchronous allocation; current CPU/PMM | range length; nested paths have own bounds |
| B29 | VFS/device IO | `kernel/src/main.rs:1845` KernelBlkDev::read_block/write_block | device lock + polling; virtio device/xHCI DMA | per-device bound; whole file size dependent |
| B30 | virtio block completion | `sunlight-virtio/src/blk.rs:143` VirtioBlk::submit | busy poll with fences; device backend | 50000000 iterations |
| B31 | xHCI transfer | `kernel/src/usb_xhci.rs:541` command, wait_transfer | event-ring poll; controller DMA | 2-second TSC budget; zero Hz expires immediately |
| B32 | serial output | `kernel/src/arch/x86_64/serial.rs:61` _print | spin lock + uart send; lock holder/UART | no local bound; uart dependency implementation needs verification |
| B33 | realtime clock seqlock | `kernel/src/arch/x86_64/rtc.rs:137` begin_realtime_write, realtime_snapshot | CAS/seqlock retry; writer CPU | none |
| B34 | idle/timer accounting | `kernel/src/sched/accounting.rs:64` snapshot, idle_loop | 8 retries or STI;HLT; writer CPU/interrupt | 8 retries for snapshot; idle intentionally indefinite |

The following searches were discovery aids, followed by call-chain inspection: `spin_loop`, `loop`, `while`, `.lock()`, `block_current`, `request_reschedule`, `futex`, `invalidate`, and lifecycle/reaping helpers. Allocation/copy/file loops are size-bounded work, not automatically participant waits. Boot calibration, terminal fault halt loops and diagnostic feature barriers are not classified as normal blocking syscall defects. UART implementation details, peripheral initialization and all network-driver loops remain outside complete call-chain closure.

## 4. Scheduler progress analysis

* **Can a syscall release the CPU?** Yes, by publishing a blocked state and returning. `sys_native_futex` (syscall.rs:3014), `sys_linux_futex` (:3354), pipe/TTY registration (sched/mod.rs:2391), and `sys_waitpid` (syscall.rs:3753) implement this pattern. Native/translated futex rewinds RCX by two bytes and schedules after SYSRET (syscall.rs:3003). This is not an in-kernel sleep primitive. `process_exit` is a separate nonreturning handoff path.
* **When is kernel execution preemptible?** Normal syscall execution is not timer-preemptible: FMASK `0x44700` (syscall.rs:218), trusted entry, no STI in the inspected normal dispatch, and SYSRET IF restore (:285). NMI shootdown can interrupt it but does not schedule userspace. Internal/boot contexts need caller-specific state; no universal kernel-preemption guarantee was found.
* **Can busy-wait prevent its owner running?** A completion owner needing the same CPU or retained scheduler lock cannot run until the waiter returns/releases it. Hardware/DMA completion and lock-free NMI acknowledgment are different: they need not schedule a userspace owner. No actual production wait cycle was established merely from finding a spin.
* **Priority, nice, aging and bursts:** production mode is RoundRobin (sched/mod.rs:141), with nice debt/credit, residual slice handling, bounded queue visits and starvation rescue before low-priority skip (:1600–1710). Affinity, live ownership, stack pins and context dispatchability are checked first. Aging can rescue eligible queued tasks only when global time advances and scheduler selection executes. It cannot preempt an IF=0 lock holder. Burst feedback changes quantum/admission; it is not priority inheritance.
* **Pending wake delivery:** `request_reschedule_on` (:4227) publishes per-CPU intent; `flush_dispatch_notifications` (:4313) runs outside scheduler/core/IPC guards. It attempts up to 4096 hardware sends per target and panics on exhaustion. Acceptance into ICR does not prove target execution. A target in IF=0 remains pending until return. Normal syscall epilogue flushes (syscall.rs:1324); all early-exit/feature combinations have not been mechanically closed.
* **Blocked but consuming CPU:** yes, transiently between blocked-state publication and actual dispatch; futex can reenter/replay before delayed IPI handling. T4B host controls exercise eight repeated replays without re-registration or early success. There is no proven wall-time bound on this interval.
* **Ready but not dispatched:** yes, deliberately while still owned, remotely stack-pinned, affinity-ineligible, queued behind other work or awaiting IPI delivery. `wait_for_pin_release` (:960) registers an atomic release obligation and skips the candidate rather than spinning while holding SCHEDULER.
* **Nested interactions:** normal masked syscall entry prevents maskable IRQ recursion in scheduler critical sections. NMI uses its own IST and the TLB handler avoids scheduler/PMM/broker mutexes. This does not prove every NMI diagnostic feature or fault/allocator reentry safe. No lockdep mechanism was demonstrated.

## 5. Lock and interrupt safety

Observed order for ordinary IPC is scheduler → capability broker → endpoint shard (syscall.rs:1473–1500). SHM/restricted retirement uses scheduler → PMM → broker (channel_mapping.rs:32). Scheduler cleanup also enters VFS (sched/mod.rs:2917); VFS declares that it must not acquire scheduler or PMM while held (main.rs:1984). These are observed/local contracts, not a complete global ordering proof. Pre-existing tmpfs changes make allocator/VFS ordering particularly important to revisit; no such changes were edited.

Scheduler transaction paths perform synchronous work while holding spinlocks: SHM unmap calls remote TLB synchronization, direct reclaim runs compression/eviction under scheduler and PMM, and diagnostics may send serial bytes while holding scheduler. These are not sleeping waits but can delay all contending CPUs with IRQs disabled. Local lock hold durations are UNKNOWN.

The shootdown NMI consumes an atomic mailbox and flushes without taking scheduler, PMM, broker or shootdown-serializer locks (tlb.rs:415; interrupts.rs:252). Thus the obvious scheduler-holder → remote scheduler-waiter → acknowledgment cycle is specifically avoided in the normal handler. `ActiveCpuSlot::current` returns None for generation zero (mm2b_state.rs:65); it does not spin waiting for an interrupted local transition to finish. Epoch reload/retry covers activation overlap, but a universal starvation bound is not established.

The ordinary serial printer masks interrupts and takes SERIAL, then calls synchronous UART send for each byte (serial.rs:61). Panic `_try_print` avoids mutex recursion but does not prove transmitter completion bounded. Exact UART backend behavior and a complete reverse-order search across every caller remain unproven. Interrupt handlers waiting for a userspace reply were not found in the inspected call chains; this is not an exhaustive absence proof.

## 6. Thread and futex lifecycle assessment

Native WAIT and Linux WAIT both hold SCHEDULER across valid aligned atomic word read, VM identity capture, waiter publication and BlockedOnIo transition. WAKE holds the same scheduler mutex. A waker before acquisition makes the atomic condition mismatch; a waker after registration sees the blocked waiter. A wake after state publication but before actual descheduling makes the task Ready and leaves ownership with its current CPU until dispatch. This closes the inspected compare/register lost-wakeup window, subject to userspace publishing its word before WAKE.

Futex keys include address-space identity (physical root and generation) and VA. Physical shared backing across distinct VMs/VAs is unsupported. Native absolute timeout uses the tick-derived monotonic clock; Linux nonzero timeout returns ENOSYS rather than pretending to time out. The replay receipt prevents rereading/reblocking after completion. Native RESULT is consumed once.

`complete_futex_wait` (process/mod.rs:414) takes the registration once, clears deadline and Linux registration. `record_wakeup` (sched/mod.rs:2239) invokes it before generic wake. Expiry records timeout 2 before wake bookkeeping; a later wake cannot replace that detached result. Wake count is bounded by the caller limit; zero selects none. Generic signal/VM-range cancellation is a spurious wake, not a demonstrated EINTR or pthread-cancel contract. Host output explicitly marks EINTR/pthread_cancel UNSUPPORTED.

Termination clears registrations/receipts and defers reclamation while still owned, queued or stack-pinned (sched/mod.rs:2710, 2769). TIDs/incarnations and VM generations reduce stale identity confusion; complete exhaustion and every reuse interleaving are not proved. An ordinary futex does not identify a mutex owner: owner death cannot automatically repair an arbitrary userspace lock. A waiter can legitimately remain asleep forever without a wake/deadline; this alone is not a kernel deadlock.

Existing host evidence covers both wake/expiry winner orders, barrier-started contention serialized by a host mutex, signal-like wake/expiry, exit clearing and reuse rejection, wake limits and VM/address isolation, repeated timeout receipt consumption, native admission, Linux admission and delayed-IPI replay. Hardware memory translation, actual IRQ frame handling, forced migration and all exit races require guest proof. Passing helper tests do not cover every interleaving.

## 7. IPC and FastLane assessment

`handle_ipc_call` (:1170) checks terminal caller state, consumes terminal outcomes, validates capability and pending-call identity, enqueues once, records optional deadline, then parks. Reply identity includes PID/incarnation/call generation. `ipc_call` wakes a matching receiver and requests rescheduling on WouldBlock (syscall.rs:1496). This creates a scheduling dependency, not an in-kernel spin awaiting service work.

Receive/reply-wait registers under transaction locks, and a server with an unresolved reply target cannot receive another ambiguous call (ipc/mod.rs:1523). Cancellation removes queued and reply-waiter entries and preserves a tombstone for a late reply (:1928). Endpoint closure drains calls and publishes peer-closed outcomes through scheduler cleanup (sched/mod.rs:2746). Caller death is rejected before enqueue. No observed generic wait-graph check prevents a service-level A→B→A protocol cycle; no concrete deployed cycle is claimed.

A high-priority client parking releases the CPU, but the inspected paths do not demonstrate priority donation to its service. RR/aging offers conditional admission, not a service-specific latency guarantee. Service startup/readiness polling and synchronous UAC/supervisor/name-service dependencies need a concrete userspace call graph; they are not declared kernel defects from documentation.

FastLane diagnostics and native Doorbell entry points are feature-gated (syscall.rs:971). Doorbell registration couples identity/token and blocked state under scheduler, and completion validates incarnation/token/outcome (sched/doorbell_native.rs:32). Close, cancel and producer exit complete the same registration. Restricted mapping retirement drains views and keeps rows unrecyclable until complete (channel_mapping.rs:32). Lease/session authority, terminal reply and quota cleanup are security-sensitive; this audit did not alter enablement or claim full lease race closure. Broad existing FastLane security/lifecycle test results were not rerun or replaced.

## 8. Memory and TLB acknowledgment assessment

`invalidate` (tlb.rs:199) serializes requests, advances mutation epoch, snapshots active CPUs for the exact VM identity, flushes locally, publishes per-target mailbox requests and sends fixed IPIs **and NMIs**. Non-active CPUs are excluded because PCID is disabled and CR3 reload discards non-global translations. Activation retries if its epoch overlaps mutation (tlb.rs:119).

The target handler claims the mailbox once, reads the current identity, flushes if matching, then publishes acknowledgment (tlb.rs:415–469). A nonmatching current root needs no retained tagged translation. Fixed-vector handler sends EOI; NMI does not use the maskable interrupt EOI contract. IRQ masking on a scheduler lock waiter therefore does not alone block acknowledgment. NMI nesting/masking/hardware delivery and all activation interleavings remain unproven on hardware.

The acknowledgment loop is finite at 100,000,000 spins, **after** delivery. `send_fixed_ipi` and `send_nmi` call unbounded `wait_for_icr_idle`; the whole shootdown has no established finite bound. Ack expiry calls invariant panic (tlb.rs:470), not successful return. Replacement frees the old frame only after invalidation returns (address_space.rs:921); unmap clears the leaf then invalidates before returning physical ownership (:1441); SHM releases mapping references afterward (shared.rs:374). Thus no timeout-driven premature reclamation was found in these chains. Fail-stop preserves safety at the cost of availability; panic/NMI diagnostics are not a recovery proof.

Direct reclaim uses an allocation-shared page/time budget and local TSC instead of the frozen BSP tick (swap.rs:501). This bounds attempted reclaim work, not arbitrary nested device/TLB operations. It skips opportunistic reclaim if TSC calibration is absent. Quota reservation/rollback is ordinary serialized bookkeeping, with synchronous retirement possibly retaining charges while remote invalidation is pending. Full rollback/forced-revoke interleavings remain unproven.

Device-backed VFS calls retain VFS and a device lock. Virtio polls at most 50,000,000 iterations per submit; xHCI command/transfer polls use a local TSC two-second deadline, mark the controller dead on timeout, and expire immediately if Hz is zero (usb_xhci.rs:541,724). These completion producers are hardware/DMA, so IF=0 alone does not make the poll deadlock. Whole-file work can contain many individually bounded operations. Virtio late completion and buffer reuse deserve separate investigation (KFP-05).

## 9. Single-CPU versus SMP behavior

| CPUs | Source-supported behavior | Guest evidence this milestone |
|---|---|---|
| 1 | Proper blocked publication can release the only CPU after SYSRET. Kernel polling monopolizes all execution; no remote TLB target for sole active CPU. A client holding a userspace lock required by its service can still cycle. | NOT_TESTED |
| 2 | Remote wake is deferred by target IF=0. Scheduler contention can couple both CPUs. NMI acknowledgment avoids needing target scheduler acquisition. | NOT_TESTED |
| 4 | More contenders/activation overlap increase possible retry and tail delay; do not prove boundedness. | NOT_TESTED |
| 12 | More active TLB targets and global-lock contenders; useful AP work can conceal a stalled BSP clock. Existing MM-2B regression gate explicitly requires 12 online CPUs (tlb.rs:616). | NOT_TESTED |

No p50/p95/p99/max/sample-count kernel timing distribution is available: sample count **0** for syscall wall/CPU time, blocked time, lock wait, dispatch latency, wake-to-run, IRQ-off duration, remote ack latency and per-CPU progress in this milestone. Host tests are correctness controls, not latency benchmarks. Existing scheduler perf/timer/FastLane diagnostics are reusable, but different scopes must not be combined into a fictitious distribution. Host load, instrumentation overhead and QEMU vCPU scheduling would affect future measurements.

## 10. Dependency graph and suspicious cycles

Machine-readable graph: [JSON](../../.kfp-1-evidence/current/dependency-graph.json), [DOT](../../.kfp-1-evidence/current/dependency-graph.dot).

1. Futex waiter → peer WAKE → scheduler → target pending IPI → target SYSRET/IF. No lock is retained across the intended park. A user lock held by waiter can add a reverse edge; no concrete instance proved.
2. SHM retire → scheduler/PMM/broker retained → TLB serializer → APIC ICR → target NMI → atomic mailbox ack → reclamation. Normal NMI does not need retained mutexes; ICR progress and delivery are remaining external dependencies.
3. Timed waiter on AP → global clock → BSP timer → BSP exits IF=0 section. BSP kernel delay can suspend remote deadline progress while other CPUs appear healthy.
4. Service A → synchronous B → callback to A (or A-held user lock) is a conditional cycle. It is not a reproduced or conclusively established deployed cycle.
5. Ready task → remote outgoing-stack release → reschedule request. Handoff waiter registration avoids holding scheduler while spinning for the epilogue.

## 11. Findings and verified properties

No confirmed/reproduced defect is counted. SOURCE_SUPPORTED observations below are distinct from a confirmed reachable permanent stall. Severity describes potential impact; likelihood describes trigger evidence, independently.

### KFP-01 — TLB/APIC

**BUSY_WAIT / SOURCE_SUPPORTED**. Severity: high; likelihood: unknown; requires abnormal/stuck ICR.

* **Source:** `kernel/src/arch/x86_64/lapic.rs:145`, `kernel/src/memory/tlb.rs:316`.
* **Trigger:** ICR delivery bit never clears during shootdown delivery.
* **Required interleaving:** initiator acquires serializer and often scheduler/PMM; waits inside send_fixed_ipi before NMI and ack budget.
* **Affected CPUs:** 2/4/12 remote targets; 1 CPU internal IPI callers require separate reachability.
* **Expected behavior:** bounded delivery failure or acknowledgment completion.
* **Source-supported behavior:** ack budget cannot bound preceding ICR loops.
* **Progress impact:** initiator CPU stalls; scheduler holders can stall all CPUs.
* **Security/memory safety:** availability; no premature free established.
* **Existing coverage:** source only; existing MM-2B fixture does not inject stuck ICR.
* **Missing evidence:** fault injection or hardware noncompletion evidence.
* **Next investigation:** inspect/reuse bounded APIC fixture and authorize stuck-ICR diagnostic separately.
* **Confidence:** high on missing bound; low on occurrence.

### KFP-02 — scheduler/timekeeping

**STARVATION / SUSPECTED**. Severity: high; likelihood: conditional on long BSP kernel section.

* **Source:** `kernel/src/timekeeping.rs:1`, `kernel/src/arch/x86_64/syscall.rs:218`, `kernel/src/sched/mod.rs:2169`.
* **Trigger:** BSP remains in IF=0 kernel execution while another CPU has timed waiters.
* **Required interleaving:** BSP timer pending; AP continues; native deadline/global_tick cannot advance until BSP returns.
* **Affected CPUs:** 1/2/4/12; SMP can conceal BSP clock stall.
* **Expected behavior:** deadline and aging eventually advance.
* **Source-supported behavior:** expiration is conditional on BSP clock and scheduler availability; tick catches up after return.
* **Progress impact:** timer waits, IPC deadlines and aging delayed; indefinite only if underlying kernel stall indefinite.
* **Security/memory safety:** availability; no corruption established.
* **Existing coverage:** host timekeeping/helper controls; no delayed BSP guest run.
* **Missing evidence:** maximum IRQ-off interval and guest deadline latency.
* **Next investigation:** run existing timer/T4B guests with per-CPU evidence.
* **Confidence:** high on dependency; occurrence/upper bound NOT_PROVEN.

### KFP-03 — IPC/service lifecycle

**DEADLOCK / SUSPECTED**. Severity: high; likelihood: protocol-dependent UNKNOWN.

* **Source:** `kernel/src/ipc/mod.rs:1170`, `kernel/src/ipc/mod.rs:1324`, `kernel/src/arch/x86_64/syscall.rs:1496`.
* **Trigger:** authorized service A calls B synchronously while B needs A or a userspace lock held by A.
* **Required interleaving:** A parks awaiting B; B parks awaiting A; neither arms deadline or external cancellation.
* **Affected CPUs:** all; extra CPUs do not dissolve logical cycle.
* **Expected behavior:** protocol avoids cycle or deadline/cancel breaks it.
* **Source-supported behavior:** call path records pending generation and blocks; inspected path has no wait-graph cycle check.
* **Progress impact:** potential permanent service wait; no concrete deployed service cycle established.
* **Security/memory safety:** availability; authority rules unchanged.
* **Existing coverage:** cancel/peer-close source; no concrete cycle fixture run.
* **Missing evidence:** production service call graph and user-lock ownership.
* **Next investigation:** trace nameserver/supervisor/UAC/VFS nested calls using existing lifecycle receipts.
* **Confidence:** medium conditional model; low on actual occurrence.

### KFP-04 — locks/priority

**PRIORITY_INVERSION / SUSPECTED**. Severity: medium; likelihood: contention plausible; duration UNKNOWN.

* **Source:** `kernel/src/sched/mod.rs:1600`, `kernel/src/arch/x86_64/serial.rs:61`, `kernel/src/sched/mod.rs:4415`.
* **Trigger:** kernel waiter contends with remote holder, or low-priority userspace completion owner competes with other tasks.
* **Required interleaving:** waiter spins IF=0 or sleeps awaiting service; holder must complete; no owner priority donation identified.
* **Affected CPUs:** 1 CPU user dependency; SMP remote lock contention.
* **Expected behavior:** completion owner receives eventual execution.
* **Source-supported behavior:** RR aging admits eligible queued tasks when scheduler runs; cannot interrupt IF=0 waiter/holder.
* **Progress impact:** unbounded latency not excluded; no proven inversion deadlock.
* **Security/memory safety:** availability; lock-order proof incomplete.
* **Existing coverage:** scheduler host policy PASS; priority guest scenario NOT_TESTED.
* **Missing evidence:** lock wait/hold histograms and owner/affinity experiment.
* **Next investigation:** existing scheduler guest benchmark with high/low nice and fixed affinity.
* **Confidence:** medium; no universal starvation conclusion.

### KFP-05 — virtio block timeout

**BOUNDED_WAIT / SUSPECTED**. Severity: high; likelihood: requires delayed completion beyond spin budget.

* **Source:** `sunlight-virtio/src/blk.rs:143`, `sunlight-virtio/src/blk.rs:194`, `kernel/src/main.rs:1845`.
* **Trigger:** device completes after submit returns false, followed by another request.
* **Required interleaving:** timeout leaves last_used_idx unchanged; next submit overwrites fixed descriptor/request storage and republishes; delayed used entry may satisfy next poll.
* **Affected CPUs:** all; device lock serializes CPUs but not late DMA.
* **Expected behavior:** timeout recovery establishes old request quiescence before reuse.
* **Source-supported behavior:** submit timeout returns false without marking device dead or resetting queue in inspected caller.
* **Progress impact:** IO failure/stall amplification and possibly misattributed completion.
* **Security/memory safety:** possible DMA/data integrity issue; NOT_PROVEN, requires separate memory-safety audit.
* **Existing coverage:** source only; no delayed-device fixture executed.
* **Missing evidence:** backend cancellation contract and delayed completion test.
* **Next investigation:** inspect existing storage fault fixtures before authorizing delayed-device diagnostic.
* **Confidence:** high on source sequence; consequences NOT_PROVEN.

Verified within their limits: serialized native/Linux futex admission/completion helpers; one-shot native receipts; stale registration clearing; bounded wake selection; RR policy/state controls; atomic handoff/reschedule state tests. These are host/source findings, not deterministic guest-forward-progress guarantees.

## 12. Unproven mandatory properties

The count of fourteen refers to this explicit list, not every unknown field in the inventory:

1. Single-CPU real guest A blocks, B runs, A resumes on current source (scenario A).
2. High-priority waiter/lower-priority completion owner under competing work and affinity (B).
3. Hardware wake-before-block delivery across SYSRET and replay (C).
4. Guest exit/wake/cancel/timeout races for current dirty source (D).
5. Remote acknowledgment and safe reclamation under delayed/masked target delivery on 2/4/12 CPUs (E).
6. A complete deployed synchronous service/user-lock dependency graph (F).
7. Whole-shootdown finite delivery bound, including ICR noncompletion.
8. Maximum BSP IRQ-off interval and remote deadline accuracy during it.
9. Global lock order/reentrancy proof across faults, VFS/allocator, NMI and optional diagnostics.
10. Eventual dispatch for every eligible Ready task under all affinity/pin/nice states.
11. Complete production FastLane lease/session/forced-revoke/quota rollback interleavings.
12. Prompt generic signal interruption, EINTR and pthread-cancellation semantics; unsupported controls are not PASS.
13. UART/device timeout recovery and late DMA/request-buffer reuse safety.
14. Per-path guest latency/progress distributions and instrumentation overhead across 1/2/4/12 CPUs.

## 13. Tests actually executed and follow-up experiments

| Existing command | Result | What it establishes |
|---|---|---|
| `python3 tools/threading_t4b_host.py .kfp-1-evidence/current/t4b-host` | exit 0 | Production helper extraction/admission controls pass; placement/VM/trap adapters stubbed. EINTR/pthread_cancel UNSUPPORTED. |
| `python3 -m unittest discover -s tools -p test_threading_t4b.py -v` | 4 tests, PASS | Synthetic receipt rejection/acceptance; no guest proof. |
| `python3 -m unittest discover -s tools -p test_threading_t4a.py -v` | 2 tests, PASS | Synthetic parser controls; no guest proof. |
| `python3 tools/test_scheduler_host.py` | exit 0, PASS | Existing handoff/request/quantum/timekeeping/accounting/policy/service/queue host controls. |

Logs, extracted Rust/binaries, source hashes and matrices are under `.kfp-1-evidence/current/`. The scheduler runner has a hardcoded output path `target/scheduler-b-evidence/focused-host.log`; its log was copied to the KFP directory. This was an artifact-isolation exception: the existing runner also uses disposable `/tmp/scheduler-host-*` compilation files. It did not build or replace a guest kernel. Existing security/lifecycle evidence directories were not modified. Python imports may generate ignored `tools/__pycache__` cache files. No source instrumentation was added.

Reusable guest infrastructure was inspected: `tools/threading_t4b_run.py` supports 1/2/4/12 CPUs with a required 2-CPU race pilot, source identity capture, frozen ELF/ISO, 300-second guest timeout, and stop-on-pilot-failure. However it builds into shared production target locations and writes `.threading-evidence/t4b`, rather than a KFP-only directory. No guest runner was executed here: reproducible isolated current-source guest preparation remains outstanding, so A–F are NOT_PROVEN. The MM-2B feature fixture has a 12-CPU topology assertion; arbitrary CPU counts cannot simply be substituted. This limitation prevents PASS, not further read-only investigation.

Recommended bounded experiments using existing infrastructure, before any new instrumentation:

1. Prepare an isolated source-bound T4B build/output configuration without editing production source; run 2-CPU pilot then 1/4/12 guests. Preserve kernel/ISO hashes, full receipts, per-case completion and per-CPU dispatch evidence. Stop on pilot failure.
2. Run existing scheduler guest/perf fixtures with fixed affinity and high/low nice, checking completion-owner progress rather than aggregate throughput. If they cannot express scenario B, mark NOT_PROVEN and scope a diagnostic-only follow-up.
3. Reuse MM-2B 12-CPU acknowledgment and existing restricted-retirement fixtures. Verify PTE clear → mailbox request → ack → final frame/quota release; do not infer ordering from a PASS marker alone.
4. Inspect concrete nameserver/supervisor/UAC/VFS service call sites and user-lock lifetimes; select an existing terminal-reply/peer-exit fixture to test a real dependency chain.
5. Inspect existing APIC/device fault fixtures for ICR noncompletion and delayed virtio completion. If absent, propose bounded diagnostic instrumentation as a separate milestone. No fault injection or source changes were authorized or performed here.

For every guest run record bounded workload, timeout, CPU topology, binary/source hashes, p50/p95/p99/max/count where the fixture actually exposes samples, and host variability. Wall-clock timeout alone does not distinguish guest CPU execution from host descheduling.

## 14. Risk-prioritized remediation candidates (no implementation)

| Rank | Candidate for a separately authorized milestone | Prerequisite evidence |
|---|---|---|
| 1 | Bound the complete APIC/shootdown delivery transaction with a safety-preserving fail-stop policy | KFP-01 fault reachability; no timeout-and-free recovery |
| 2 | Define virtio timeout quiescence/reset and descriptor reuse contract | KFP-05 backend and late completion evidence |
| 3 | Establish kernel critical-section budgets and BSP deadline-progress contract | KFP-02 guest IRQ-off/per-CPU clock evidence |
| 4 | Close global lock ordering and concrete service callback cycles | KFP-03/04 call graph, lock ownership, lifecycle evidence |
| 5 | Specify and verify priority/affinity, cancellation and wake-to-run guarantees | scenarios A–D and unsupported semantic decisions |

These are investigation/remediation candidates, not approved changes. Keep current capability checks, VM identity, FastLane production exclusions, lease authority and deferred reclamation intact.

## 15. Final audit verdict

**PARTIAL** — normal blocking has source-supported CPU-release and race-serialization mechanisms, with passing focused host controls. Whole-kernel bounded progress, current-source single-CPU guest behavior, service dependency closure and complete SMP acknowledgment/delivery guarantees remain unproven. No FAIL is asserted without a conclusive critical violation; no PASS is inferred from host controls or prior C1.2F latency improvements.

Only this audit document and dedicated KFP evidence were intentionally added. Production source, pre-existing working-tree changes, syscall ABI, scheduling/security policy and FastLane enablement were not modified. No commit or push occurred.
