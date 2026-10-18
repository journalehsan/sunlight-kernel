# SunlightOS scheduler audit — Phase A

This is the historical Phase A baseline. Phase B fixes, current source
references, contracts and validation are recorded in
[scheduler-hardening.md](scheduler-hardening.md). The source line numbers below
describe the original baseline and can differ from the hardened checkout;
use `git show d2faa88e6:PATH` to inspect that version.

Audit date: 2026-10-18. Source baseline:
`d2faa88e6aee8974e131a19db4cc808af6d8b866`. Line references below refer to that
baseline. The pre-existing local `.version_state.json` change was excluded.
This phase changes no kernel scheduling, IRQ, locking, or lifecycle behavior.
The accompanying [host policy probe](../../tools/scheduler_policy_probe.py)
extracts production selector functions; it adds no guest instrumentation.

## Assessment and evidence limits

**The active scheduler already has one RR FIFO per CPU and batch work stealing,
but scheduling remains serialized by a global process-table/scheduler mutex.**
Adding per-CPU queues is therefore not a new optimization. Separating their
synchronization from the process table would be a substantial concurrency change.
Additional priority queues or scheduling classes are not justified in Phase A.

Correctness work comes first: source inspection exposes an outgoing-stack
reclamation hazard, premature publication of outgoing contexts, an indefinitely
skipping nice policy, and a reschedule flag that is rearmed by dispatch itself.
These findings must be resolved or experimentally ruled out before interpreting
policy or SMP performance comparisons.

Evidence labels used here:

- **Source fact:** directly follows from the referenced implementation.
- **Host reproduction:** production Rust policy functions executed in an explicit
  fixture; not a full kernel execution or a latency measurement.
- **Source-derived hazard:** a reachable-looking interleaving with an identified
  missing invariant; no guest fault reproduced in this audit.
- **Hypothesis:** needs timing, a guest test, or broader subsystem validation.

There are **no new guest latency, lock-time, context-switch-cycle, or SMP speedup
measurements** in this audit. No percentile numbers are inferred from tick rate,
older logs, comments, or a successful build. The host has QEMU 11.1.1 but no
`/dev/kvm`; TCG remains available for functional testing. The existing ISO/ELF
were not treated as source-matched benchmark evidence. Guest matrix commands
below are a reproducible test plan, not claimed completed runs.

## Verified architecture

Reference shorthand: **S** = `kernel/src/sched/mod.rs`, **I** =
`kernel/src/arch/x86_64/interrupts.rs`, **Y** =
`kernel/src/arch/x86_64/syscall.rs`, **B** = `kernel/src/ipc/mod.rs`.
Links retain the complete file and starting line; ranges in text delimit review.

| Area | Verified implementation and references |
| --- | --- |
| Active policy | `SCHEDULER_MODE = RoundRobin`, not Bore ([S:92–100](../../kernel/src/sched/mod.rs#L92)). The module's SunBrust/BORE introduction is not an accurate description of all active paths. `TIME_SLICE_TICKS = 10` is not used to determine the active quantum. |
| Scheduling unit | A `Process` entry represents a native process, native address-space borrower thread, or Linux thread. Native borrowers have independent scheduling state; Linux workers carry TGID/shared-owner metadata ([process/mod.rs:140](../../kernel/src/process/mod.rs#L140), [S:1642](../../kernel/src/sched/mod.rs#L1642), [Y:2063](../../kernel/src/arch/x86_64/syscall.rs#L2063), [Y:2288](../../kernel/src/arch/x86_64/syscall.rs#L2288)). |
| Global state | `spin::Mutex<Scheduler>` owns `Vec<Process>`, online count, global-tick mirror and legacy Bore core states ([S:348](../../kernel/src/sched/mod.rs#L348), [S:3738](../../kernel/src/sched/mod.rs#L3738)). Public fields/helpers rely on callers following the locking and IRQ contract. |
| Active per-CPU state | Separate, 64-byte-aligned `CORE_STATES[64]`, each with a spin mutex, RR `VecDeque<usize>`, current index/ticks, idle RSP and counters ([S:271–344](../../kernel/src/sched/mod.rs#L271)). Bore uses the duplicate `Scheduler::cores` instead. Alignment reduces adjacent-core false sharing; it does not remove global serialization. |
| Initialization | APs initialize features, TSS and syscall MSRs and arm periodic LAPIC timers, then halt ([smp.rs:314](../../kernel/src/arch/x86_64/smp.rs#L314)). `init_cores` clamps to 1..64, builds static idle frames and reserves 128 RR entries per CPU ([S:3846](../../kernel/src/sched/mod.rs#L3846)). First entry sets BSP current/owner, seeds other Ready entries and publishes `SCHEDULER_READY` with Release under S ([S:3750](../../kernel/src/sched/mod.rs#L3750)); AP timer reads use Acquire ([I:892](../../kernel/src/arch/x86_64/interrupts.rs#L892)). |
| Queue ownership | Queue entries are process **indices**, not generation-qualified handles. `queued_on_core` and `owning_core` use `u8::MAX` for absent. Duplicate suppression is O(1) metadata checking; targeted removal uses `retain` ([S:627–817](../../kernel/src/sched/mod.rs#L627)). |
| Placement | Detached Ready tasks choose the least-loaded eligible CPU, counting queued plus current tasks; ties prefer the lowest CPU. Every placement scans all online CPUs and locks each core sequentially ([S:454](../../kernel/src/sched/mod.rs#L454)). A preferred-first-CPU helper exists ([S:733](../../kernel/src/sched/mod.rs#L733)). Ordinary dispatch requeues locally. |
| Selection | FIFO front, state/owner/affinity/context validation, aging/nice policy, then stealing. One pass is bounded by initial local queue length ([S:1354](../../kernel/src/sched/mod.rs#L1354)). Context validation checks stack bounds and a nonzero saved RIP; it is not full frame validation ([S:819](../../kernel/src/sched/mod.rs#L819)). |
| Migration/balancing | Empty local selection scans victims in CPU order. It takes up to half a victim FIFO, capped at 32, from the back; queues of length <=1 are not stolen. Affinity is checked after removal; rejected Ready entries are pushed back. The batch uses a fixed stack array ([S:1175](../../kernel/src/sched/mod.rs#L1175)). No periodic equalization, topology awareness, last-CPU preference, or running-task migration was found. |
| Quantum policy | `q = 5 + floor(clamp(burst + 16*nice, 0, 1024)*45/1024)`, with a one-shot negative-nice promotion override. At 100 Hz, nominal quanta span 50–500 ms; default burst 256/nice 0 gives 16 ticks/160 ms. Nice changes counter admission as well as q ([S:190–242](../../kernel/src/sched/mod.rs#L190), [S:1392](../../kernel/src/sched/mod.rs#L1392)). These are nominal values, not observed slice lengths; see F4/F5. |
| Clock/tick | Per-CPU periodic LAPIC timers nominally 100 Hz ([lapic.rs:208](../../kernel/src/arch/x86_64/lapic.rs#L208), [lapic.rs:278](../../kernel/src/arch/x86_64/lapic.rs#L278)). CPU 0 alone advances global elapsed ticks; calibrated time catches up after delayed IRQs ([timekeeping.rs:71](../../kernel/src/timekeeping.rs#L71)). Per-core quantum accounting advances one per delivered timer callback, not elapsed time ([S:979](../../kernel/src/sched/mod.rs#L979)). |
| Dispatch | Timer Phase 1 always takes S after the AP boot gate; Phase 2 takes S again only if the atomic reschedule bit is set ([I:919](../../kernel/src/arch/x86_64/interrupts.rs#L919), [I:1013](../../kernel/src/arch/x86_64/interrupts.rs#L1013)). Reschedule IPI directly takes S ([I:854](../../kernel/src/arch/x86_64/interrupts.rs#L854)). Core locks are nested inside S. |
| Context switch | Save GPR frame in naked IRQ entry; save FXSAVE, RSP and FS_BASE in Rust; change owner/state, load address space, FXRSTOR and FS_BASE, update TSS RSP0; return next RSP. The actual `mov rsp` and `iretq` happen **after Rust releases S** ([S:1464–1620](../../kernel/src/sched/mod.rs#L1464), [I:744](../../kernel/src/arch/x86_64/interrupts.rs#L744), [I:806](../../kernel/src/arch/x86_64/interrupts.rs#L806)). FXSAVE covers the implemented x87/SSE state, not general XSAVE/YMM ([cpu.rs:55](../../kernel/src/arch/x86_64/cpu.rs#L55)); kernel target avoids SIMD ([config](../../.cargo/config.toml)). |
| CPU accounting | Task runtime uses an open interval closed at actual descheduling/exit bookkeeping. Idle time has separate per-CPU bounded seqlock snapshots and `sti; hlt` loops ([S:888–959](../../kernel/src/sched/mod.rs#L888), [accounting.rs:29](../../kernel/src/sched/accounting.rs#L29)). Bookkeeping dispatch time precedes physical user execution, so runtime counters do not directly measure wakeup-to-user-run latency. |

`Process::sched_type` and `weight` are present but do not implement active FIFO
real-time or fair scheduling ([process/mod.rs:242](../../kernel/src/process/mod.rs#L242)).
The active RR mode is FIFO ordering with adaptive/nice behavior, not strict
equal-quantum RR. Positive nice increases the formula's base q even though some
comments suggest shorter slices. Burst/churn scoring, aging and nice-counter
logic remain active in RR.

### Blocking and wakeup paths

| Path | Registration, wakeup and dispatch behavior |
| --- | --- |
| IPC call/receive/reply-wait | S encloses capability/shard operations and the empty-check-to-block transition ([Y:1270](../../kernel/src/arch/x86_64/syscall.rs#L1270), [Y:1738](../../kernel/src/arch/x86_64/syscall.rs#L1738), [Y:1771](../../kernel/src/arch/x86_64/syscall.rs#L1771)). B publishes the request/pending generation, then blocks; receive checks pending messages before registering a wait ([B:1008](../../kernel/src/ipc/mod.rs#L1008), [B:1283](../../kernel/src/ipc/mod.rs#L1283)). WouldBlock requests rescheduling and returns through SYSRET; userspace retry/yield is part of this ABI. |
| IPC terminal wake | Reply/cancel/deadline arbitration accepts only the matching in-flight generation and first terminal outcome. It marks BlockedOnIpc Ready; still-owned tasks get an owner reschedule request rather than another queue entry ([B:811](../../kernel/src/ipc/mod.rs#L811), [B:902–1005](../../kernel/src/ipc/mod.rs#L902)). This specifically closes the wake-between-block-and-deschedule duplicate-running window at the metadata level. |
| Timer wake | CPU 0 scans IPC call and receive deadlines, Linux/native poll timeouts and native futex deadlines under S ([S:983](../../kernel/src/sched/mod.rs#L983), [B:1597](../../kernel/src/ipc/mod.rs#L1597), [S:1716](../../kernel/src/sched/mod.rs#L1716), [S:2070](../../kernel/src/sched/mod.rs#L2070)). CPU 0 also finds timer_server by name, queues a coalesced tick and wakes it; TTY gets a periodic wake every third global tick ([I:941](../../kernel/src/arch/x86_64/interrupts.rs#L941)). timer_server reports accumulated ticks, not the kernel's complete sleep queue ([timer_server](../../services/timer_server/src/main.rs#L25)). Linux nanosleep registers a scheduler poll deadline ([Y:6186](../../kernel/src/arch/x86_64/syscall.rs#L6186)). |
| Native/Linux futex | Wait value check and waiter registration share S with wake; keys include exact address-space identity and address. Native wait uses an atomic word read and optional ns deadline; Linux WAIT uses a four-byte copy and rejects timed waits in this subset ([Y:2199](../../kernel/src/arch/x86_64/syscall.rs#L2199), [Y:2424](../../kernel/src/arch/x86_64/syscall.rs#L2424)). Wake scans the process table and respects a count limit ([S:1663](../../kernel/src/sched/mod.rs#L1663), [S:1857](../../kernel/src/sched/mod.rs#L1857)). |
| Pipe/socketpair/eventfd/poll | Readiness check, operation and wait registration share S; wake scans blocked tasks and checks readiness or FD membership ([Y:4757](../../kernel/src/arch/x86_64/syscall.rs#L4757), [Y:5530](../../kernel/src/arch/x86_64/syscall.rs#L5530), [Y:7453](../../kernel/src/arch/x86_64/syscall.rs#L7453), [S:2000–2107](../../kernel/src/sched/mod.rs#L2000)). Native retry paths rewind the syscall instruction and preserve its number. Several Linux retry paths request only the periodic reschedule fallback. |
| TTY rings | Failed read/write and registration are serialized with producer notification under S. Input push/output drain release ring locks before acquiring S to wake waiters; registration then either sees data/space or the later wake sees the waiter ([Y:1999](../../kernel/src/arch/x86_64/syscall.rs#L1999), [Y:2018](../../kernel/src/arch/x86_64/syscall.rs#L2018), [S:1941](../../kernel/src/sched/mod.rs#L1941), [Y:7633](../../kernel/src/arch/x86_64/syscall.rs#L7633), [Y:7985](../../kernel/src/arch/x86_64/syscall.rs#L7985)). `clear_tab` also releases rings before waking ([tty_io.rs:220](../../kernel/src/process/tty_io.rs#L220)). |
| Input IRQ | Keyboard/mouse push atomic rings but then take S, scan for their service by name, acquire an IPC shard and call wake_pid ([keyboard.rs:513](../../kernel/src/arch/x86_64/keyboard.rs#L513), [mouse.rs:431](../../kernel/src/arch/x86_64/mouse.rs#L431)). Thus input IRQ service can wait behind unrelated global scheduler work. The IRQ does not directly switch to the receiver. |
| Generic I/O | `wake_io_pid` requires BlockedOnIo and preserves owner-versus-queue handling ([S:2109](../../kernel/src/sched/mod.rs#L2109)). Native audio is primarily userspace DMA-progress polling and IPC timeout wakeups, not an assumed kernel audio-completion IRQ ([audiod:635](../../services/sunlight-audiod/src/main.rs#L635), [CPU accounting audit](../CPU_ACCOUNTING_AUDIT.md)). VFS reads can execute under S and KERNEL_VFS ([Y:7453](../../kernel/src/arch/x86_64/syscall.rs#L7453), [Y:7569](../../kernel/src/arch/x86_64/syscall.rs#L7569)). |
| Yield/waitpid | Yield changes Running to Ready while retaining ownership, then sends a self reschedule IPI after dropping S ([Y:1973](../../kernel/src/arch/x86_64/syscall.rs#L1973)). waitpid checks child state and registers `wait_child` under S, returning EAGAIN while blocked; exit wakes the parent ([Y:2700](../../kernel/src/arch/x86_64/syscall.rs#L2700), [S:3725](../../kernel/src/sched/mod.rs#L3725)). |
| Signals/sessions | Signals can wake parked/suspended tasks. Foreground cancellation and session termination set forced exits and request owner preemption; CPU-bound tasks are checked on timer ticks and syscalls check forced exits before dispatch ([S:2140](../../kernel/src/sched/mod.rs#L2140), [S:2674](../../kernel/src/sched/mod.rs#L2674), [Y:784](../../kernel/src/arch/x86_64/syscall.rs#L784), [foreground.rs:88](../../kernel/src/process/foreground.rs#L88)). |

Wakeup notification is deliberately distinct from publication. Per-CPU bits
prevent one CPU from consuming another's request ([S:3512–3536](../../kernel/src/sched/mod.rs#L3512)).
Enqueue requests an IPI only for a remote idle target; a busy target notices the
flag at its timer, which can cut short its nominal quantum. Native park paths
also queue a local notification. A syscall-tail flush attempts nonblocking ICR
writes after guards drop; failed sends remain pending ([S:3550–3598](../../kernel/src/sched/mod.rs#L3550),
[Y:1127](../../kernel/src/arch/x86_64/syscall.rs#L1127)). Timer/keyboard/mouse wake
paths do not themselves flush that mask: periodic timers preserve fallback
progress, but immediate wake-to-run is not guaranteed. A pending bit is not a
lost wakeup merely because execution awaits a tick.

### Exit, generations and session lifetime

Self-exit marks Finished, closes the runtime clock, clears current/owner and
returns a per-CPU static idle-stack top. Its syscall caller pivots before
enabling IRQs ([S:3646](../../kernel/src/sched/mod.rs#L3646),
[Y:1953](../../kernel/src/arch/x86_64/syscall.rs#L1953)). External termination
retains a live owner and asks that CPU to reschedule ([S:2609](../../kernel/src/sched/mod.rs#L2609)).
Reaping checks owner, queue membership, all CPUs' current entries, active VM
mask and address-space borrowers; borrowers do not reclaim their owner's user
pages ([S:2235–2448](../../kernel/src/sched/mod.rs#L2235)). Reaped slots may be
overwritten after stale queue removal ([S:483–595](../../kernel/src/sched/mod.rs#L483)).
These checks are necessary but do not cover physical outgoing-stack use (F1/F2).

Publication assigns a fresh foreground incarnation and inherits parent
incarnation, broker/session/generation, execution membership and cancellation
under S. Session teardown uses the full broker/session/generation tuple, even
for idle or reparented members; parent signal authority follows incarnation
links ([S:483](../../kernel/src/sched/mod.rs#L483),
[foreground.rs:21–175](../../kernel/src/process/foreground.rs#L21)). Address-space
identity, foreground incarnation, IPC request generation, receive generation
and terminal generation are different namespaces. An optimization must preserve
all of them, not substitute PID or queue index as a universal lifetime token.

Queue indices have no independent incarnation check. Their safety currently
depends on S serializing removal, terminal transition, reap and slot reuse.
Splitting locks without an explicit lifetime protocol would invalidate that
argument. Existing generic wake helpers reject terminal states; IPC rejects
stale generations and terminal resurrection. No normal duplicate-enqueue or
check/register lost-wakeup counterexample was found in those reviewed paths.
This conclusion does not cover every driver or user-space synchronization API.

## Hot-path cost inventory

Notation: N = process vector length, including retained dead slots; C = online
CPUs; Qc = entries in a CPU's FIFO; W = matching waiters; D = descriptors inspected
per task; E = endpoints/messages visited; R = resources/pages reclaimed.
All costs exclude time spinning for locks. `S` below means the global mutex.
No scheduler O(1) claim includes unbounded allocator, device or UART time.

| Path | Work, scans and allocations | Locks and contention |
| --- | --- | --- |
| Timer without dispatch | AP local tick O(1), several short core-lock acquisitions. BSP additionally at least four N-sized passes: two IPC deadline passes, poll expiry, futex expiry; name/PID searches add more. Expired calls may scan deferred targets and endpoint queues. | Every CPU takes S at nominal 100 Hz, including idle CPUs after boot. BSP may nest IPC/PMM/telemetry locks. Thus ~100*C mandatory timer acquisitions/sec (1200 at 12 CPUs), a **derived rate**, not a measured contention result. |
| Enqueue detached/wake | PID lookup O(N) where used; duplicate/owner check O(1); least-load placement O(C); deque push amortized O(1). Already queued enqueue_process also removes O(Qc), while enqueue_once avoids that case. | S then each core sequentially. Deque push can allocate above reserved capacity 128; there is no hard no-allocation guarantee. Frequent wakeups touch multiple CPUs' cache lines. |
| Owner still live at wake | O(N) lookup or waiter scan, then O(1) state and atomic flag; normally no queue scan or allocation. | S; no second queue publication. Dispatch is deferred to the owner. |
| Local task selection | Common first eligible candidate O(1); skips/stale entries O(Qc) per pass; invalid Ready candidates may invoke O(C) replacement, giving O(Qc*C). Nice skip pushes back, normally reusing capacity. | S plus repeated local core locks; policy and metadata are globally serialized. |
| Empty-queue steal | O(C*32) bound per call plus deque push growth; no N scan in RR steal. Pops fixed stack batch, checks each entry, requeues rejected affinity/context entries. | S then victim try_lock; release victim before thief lock. A rejected entry later uses blocking victim lock. Under the current S contract other scheduler CPUs cannot concurrently mutate victims, so try_lock is not parallel scheduling. |
| Queue removal/block | O(1) when not queued (usual owned block); otherwise O(Qc) retain, no allocation. | S then recorded core. No all-queue scan on the ordinary path. |
| Dispatch to task | O(1) save/restore/accounting and selection, **plus O(N+C) reap scan even with no Finished tasks**; when eligible dead tasks exist add full cleanup costs. CR3/FXSAVE/FXRSTOR/MSR/TSS work occurs even on same-task successful selection. | Second S acquisition in timer; S held through architectural operations and reaping. Outgoing runnable task's enqueue also sets the reschedule bit again (F4). |
| IPC call/reply | O(N) PID searches, endpoint map operations and linear reply-waiter membership/removal; successful completion avoids scanning all deferred targets. Endpoint message count is bounded; map/VecDeque creation/growth can allocate ([B:459](../../kernel/src/ipc/mod.rs#L459), [B:483](../../kernel/src/ipc/mod.rs#L483)). | S -> capability broker -> IPC shard for syscall paths. IPC sharding does not parallelize transitions still enclosed by S. |
| Futex wake/deadline | O(N) search, plus O(W*C) placement for detached wakes. No waiter index. Native deadline scan runs every BSP callback even with no deadlines. | S; placement/core locks. Queue growth may allocate. |
| Pipe/eventfd/TTY readiness | O(N*D) for poll FD membership, rising to O(N*N + N*D + W*C) with shared-owner PID searches and detached wake placement; direct-key waiters start at O(N). Shared-owner lookup can add N-sized searches for shared threads ([S:1642](../../kernel/src/sched/mod.rs#L1642)). Each wake may add O(C) placement. Buffers used in syscall paths are fixed-size, but that does not make the whole operation allocation-free. | S -> ring/eventfd/epoll locks; failed operation and registration remain atomic relative to wake. Producer ring -> **drop ring** -> S for TTY. |
| Input IRQ wake | O(N) name search and O(N) PID lookup, notification map/deque work, then O(C) placement if detached. | IRQ-disabled S wait, shard and core locks. Atomic raw input rings do not make the entire IRQ path lock-free. |
| Telemetry | O(N+C) traversal, copies at most 64 process snapshots; **allocates** process Vec and per-core Vec under S and PMM ([telemetry.rs:245–368](../../kernel/src/telemetry.rs#L245)). Expensive publication walks are outside S. | Roughly once/second on BSP via elapsed deadline. Holds S -> PMM and briefly core/other accounting locks. This contradicts an absolute allocation-free interpretation of the timer Phase 1 comment. |
| Reap/teardown | Scan O(N+C); each resource cleanup adds borrower/deferred/IPC/FD scans, VM/page-table walks, allocator frees, VFS close and wake fan-out. May allocate names and endpoint lists. No per-dispatch task/page budget in `reap_finished_processes`. | S nests PMM, broker, IPC shards, device registration, VFS, pipe/epoll and heap-related work; serial output in creation/exit/reap remains even without verbose_diag. |
| Session cancellation | Two outer N passes for session teardown; each owner cancellation may scan N and each member wake searches PID O(N), plus IPC cancellation scans and placement. Worst-case quadratic or worse composition with many owners/retained deferred entries; no new task objects needed. | S throughout. Logical membership storage is bounded per process, but work is not constant-time ([foreground.rs:88](../../kernel/src/process/foreground.rs#L88)). |
| Yield | Runtime checkpoint, state transition O(1), self IPI, then normal dispatch costs. | S followed by ICR send after dropping S. Blocking `send_fixed_ipi` polls ICR; coalesced notifications use nonblocking attempts. |

The global mutex is a **verified serialization point and plausible scaling
bottleneck**. Dominance, average contention, hold tails and scaling loss remain
unmeasured. Long teardown, VFS or memory transactions could matter more than
FIFO manipulation. Direct anonymous reclaim already has page/time budgets
([swap.rs:486](../../kernel/src/memory/swap.rs#L486)); this does not bound other
work enclosed by S, including the scheduler's own reap loop.

### Lock and interrupt safety

Reviewed acquisition edges are S -> one `CORE_STATES[c]`; S -> CAP_BROKER -> IPC
shard for IPC; S -> PMM -> CAP_BROKER for SHM/reap; and S -> VFS/ring/epoll or
swap-related locks. S precedes process-table/queue mutation. Core locks are not
held in pairs on the ordinary RR paths. Telemetry follows S -> PMM -> core.
Boot code explicitly releases PMM before acquiring S
([main.rs:776](../../kernel/src/main.rs#L776)); this is material while APs exist.

`spin::Mutex` does **not** mask IRQs. Syscall entry executes CLI for the entire
syscall; timer/reschedule and ordinary external IRQ entries run with maskable
IRQs off. Code must keep IF off through lock release and the final stack/IRET
handoff. `with_scheduler` and `current_process_rsp` do not enforce that contract
([S:3741](../../kernel/src/sched/mod.rs#L3741), [S:3807](../../kernel/src/sched/mod.rs#L3807)).
No active same-CPU timer/keyboard lock recursion was established for the reviewed
normal callers. These helpers are an interrupt-safety maintenance risk if called
from a new IF=1 kernel context. No complete absence-of-deadlocks proof is claimed.

TLB mutation may wait for remote acknowledgement with IF off. Its fixed-vector
plus NMI mailbox path deliberately avoids S in the remote handler, preventing
the simple “remote CPU spins on S, initiator waits for its maskable IPI” cycle
([tlb.rs:199–310](../../kernel/src/memory/tlb.rs#L199)). Preserve this property;
turning that handler into a scheduler-lock consumer would introduce a cycle.
Address-space activation uses an epoch retry and atomically tracks active CPUs
([tlb.rs:119](../../kernel/src/memory/tlb.rs#L119)). It does not track kernel-stack
occupancy, which is a separate invariant.

There is a latent core-lock scope trap in the empty-pop branch of pick_next:
`return self.steal_work(cpu_id)` is evaluated while its local core guard is
still in scope ([S:1357–1364](../../kernel/src/sched/mod.rs#L1357)). Stealing may
relock the thief. With S held, a queue whose initial length was positive cannot
normally become unexpectedly empty, so this is **not a demonstrated active
deadlock**. A future independent-queue implementation would make it reachable
unless the guard is explicitly released first.

## Ranked correctness and performance findings

### F1 — Critical: reclamation can free the stack running the dispatcher

**Source-derived memory-safety hazard, no guest fault reproduced.** External
termination or timer-applied fatal signal marks the current process Finished
while it still owns a CPU ([S:2633](../../kernel/src/sched/mod.rs#L2633),
[S:2699](../../kernel/src/sched/mod.rs#L2699)). A timer interrupt from ring 3
enters on that process's TSS kernel stack. `schedule_tick` clears its owner and
current entry, selects another process, activates the new VM, then calls
`reap_process_resources(prev)` before returning ([S:1538–1583](../../kernel/src/sched/mod.rs#L1538)).
The reaper can now pass all current/owner/active-VM checks and executes
`kernel_stack = None` ([S:2438–2442](../../kernel/src/sched/mod.rs#L2438)).
Rust and its caller are still executing on that allocation: the physical pivot
is later at [I:777](../../kernel/src/arch/x86_64/interrupts.rs#L777).

Use a victim with a private VM, no borrowers and another Ready task: activating
the next VM removes the active-mask barrier, so the checks do not establish
safety. A Finished borrower can lose its kernel stack even without a VM reclaim.
The reaper's comment that the dying context already pivoted is true for the
ordinary self-exit path, not for these interrupt-driven termination paths.
Allocator reuse is not required to violate the stack lifetime; it can make the
failure visible. Existing SIGKILL self-tests simulate clearing owning_core and
reaping; they do not execute a real outgoing-stack handoff
([main.rs:3121–3195](../../kernel/src/main.rs#L3121)).

Required acceptance test: kill a CPU-bound ring-3 victim while other Ready tasks
remain; record physical per-CPU stack use with fixed diagnostic fields; assert
no stack free until every CPU has physically left it. Repeat for native/Linux
borrowers, fatal timer signals, owner exit and slot reuse. A metadata-only test
is insufficient. Any fix belongs in a separate correctness change.

### F2 — High: an outgoing context becomes runnable before physical handoff

**Source-derived SMP race, no guest fault reproduced.** Runnable `prev` is
published into its CPU's FIFO with `owning_core = MAX` before the assembly
pivot. Once timer_rust drops S, another CPU can select/steal it while the old CPU
is still unwinding on its saved kernel stack ([S:1534–1537](../../kernel/src/sched/mod.rs#L1534),
[I:1030–1049](../../kernel/src/arch/x86_64/interrupts.rs#L1030)). For example,
leave at least two queued tasks after local selection; an idle thief may pop
`prev` from the tail. It can restore that context and later use the same kernel
stack for an IRQ while the old CPU still uses it. Ordinary owner/queue duplicate
checks see a valid detached Ready entry and cannot detect this physical overlap.

A blocked outgoing task has the analogous window: a remote wake after S drops
can enqueue it before the old epilogue completes. Reap/slot reuse after an
outgoing task-to-idle transition also needs scrutiny. Required test: a bounded
diagnostic pause just after S release and before RSP pivot, with another CPU
attempting wake/steal/reap. Do not implement such a pause in production or log
inside the window. A future synchronization split must first establish a
physical handoff/lifetime protocol, not merely stronger metadata atomics.

### F3 — High: positive nice can indefinitely skip a Ready task

**Host reproduction plus source arithmetic.** Local selection accumulates debt,
then skips when counter <= -3, recovering only +1. Aging supplies +2 on an aged
visit, but nice +8..+10 loses at least 3 on the same visit. At burst <=768 the
aged net change on a skip is `+2 -3 +1 = 0`; counter -7 is a fixed point. Larger
burst doubles the loss and cannot repair it ([S:209–225](../../kernel/src/sched/mod.rs#L209),
[S:1392–1429](../../kernel/src/sched/mod.rs#L1392)). Setting nice to +8..+10 is
allowed by the syscall ([Y:3829](../../kernel/src/arch/x86_64/syscall.rs#L3829)).

The host probe executes the actual selector with a neutral peer and 10,000
selection opportunities. The +8/+9/+10 task is never selected; its peer
continues running. An isolated current task can be restored by the dispatch
fallback, and a remote steal bypasses this nice admission logic. Therefore the
finding is indefinite **local contention starvation**, not a claim that every
positive-nice task always stops on every SMP workload. A pinned fixture removes
stealing as an accidental rescue. No finite worst-case wait can be promised for
the present policy. Required regression: eventual execution for every supported
nice value with at least one continuously runnable peer on one CPU.

### F4 — High: dispatch rearms its own reschedule request

**Source fact with source-derived timing consequence.** `schedule_tick` consumes
the local bit at entry. Its normal outgoing Ready/Running requeue calls
`enqueue_process_to_core`, which unconditionally sets the same bit again
([S:1477](../../kernel/src/sched/mod.rs#L1477), [S:1534](../../kernel/src/sched/mod.rs#L1534),
[S:642–648](../../kernel/src/sched/mod.rs#L642)). No subsequent clear follows.
Thus, after a dispatch requeues a runnable task, the next periodic timer will
again enter dispatch even if no task exhausted a quantum or requested a new
wake/yield. If the outgoing task remains runnable on every switch this repeats.

With eligible neutral peers and timely 100 Hz IRQs, this predicts rotation every
timer callback (~10 ms), rather than the 50–500 ms nominal quanta. A lone runnable
task can repeatedly select itself, still paying architectural save/restore and
the full reap scan without incrementing the context-transition counter. This
also makes apparent switch rate an unreliable proxy for dispatch overhead.
This is not a measured 10 ms slice result. Verify with dispatch-reason counters
and task IDs at selection under a no-wakeup/no-yield CPU-only load before any
quantum tuning. Distinguish internal requeue from an external dispatch request
in a separate fix, preserving wake publication.

### F5 — Medium: quantum ticks are per core and not reset at every dispatch

**Source fact.** Idle dispatch, voluntary rescheduling and successful task
selection do not reset `current_ticks`; `set_core_current_task` only changes the
index ([S:389](../../kernel/src/sched/mod.rs#L389), [S:1484](../../kernel/src/sched/mod.rs#L1484),
[S:1554](../../kernel/src/sched/mod.rs#L1554)). Reset occurs on the timer's
non-Running/quantum-expiry path and self-exit. An IPI-driven yield can transfer
residual ticks to the next task, and idle ticks can be inherited by a newly
dispatched task. Quantum expiry may attribute the aggregate to whichever task
is current at that time. `timeslice_used`, full-quantum burst scoring and
watchdog checks then need not describe that task's actual slice. Delayed IRQs
further separate ticks from elapsed CPU time. Required tests cover early yield,
IPI park, idle-to-task and different-quanta peers; define expected accounting
before optimizing it.

### F6 — High performance risk: global critical sections include scans and cleanup

**Serialization, scans, allocations and log sites are source facts; their
latency impact is unmeasured.** All CPUs' timer bookkeeping and dispatch compete
with process creation, IPC, input IRQ wakeups, readiness scans, VFS operations,
telemetry capture and full resource reclamation. `reap_finished_processes` scans
N even when empty and has no reclaim budget; it runs after successful dispatch,
periodically on eligible tick paths, and on add_process. The periodic modulo
condition can run on multiple CPUs at the same global tick, or miss a multiple
after catch-up ([S:1062](../../kernel/src/sched/mod.rs#L1062)).

Verbose diagnostics are default-off, but creation, exit and successful reap
still print under S ([S:536](../../kernel/src/sched/mod.rs#L536),
[S:2453](../../kernel/src/sched/mod.rs#L2453), [S:3601](../../kernel/src/sched/mod.rs#L3601)).
Telemetry still allocates under S/PMM. IRQ-disabled spinning on any of this can
increase input and audio-service scheduling latency on other CPUs. Measuring
only FIFO pop cost would miss these tails. Require separate lock-site and
reap/telemetry timing before attributing the bottleneck to the queue structure.

### F7 — Medium/high lifecycle hazard: waitpid status can disappear on slot reuse

**Source-derived race; no guest reproduction here.** Exit wakes a waiting parent
but does not copy an exit result to the parent. waitpid later searches the
process table for the child's PID/PPID and reads `exit_code`. Reaping may occur
first; a subsequent spawn overwrites any eligible Reaped slot, independent of
whether its parent consumed the exit status ([S:3725](../../kernel/src/sched/mod.rs#L3725),
[S:522–554](../../kernel/src/sched/mod.rs#L522), [Y:2704–2727](../../kernel/src/arch/x86_64/syscall.rs#L2704)).
The parent can then see “no such child.” Unique new PIDs do not preserve the old
result. Confirm with a deliberately delayed parent and concurrent spawn/reap
churn; distinguish logical completion, resource reclamation and wait-status
consumption. Preserve authenticated foreground/session status and first-exit
semantics when addressing it.

### F8 — Medium/conditional: affinity and CPU topology assumptions

**Source facts, mostly latent limitations in current supported configurations.**
All default masks allow every CPU; only clone inheritance was found as a mask
assignment beyond constructors. There is a Linux getaffinity implementation,
not a general setaffinity/migration service ([Y:4671](../../kernel/src/arch/x86_64/syscall.rs#L4671)).
Placement defaults to CPU 0 if no online CPU matches a mask. Local picking
rejects disallowed entries, but same-current fallback does not recheck affinity
([S:454–475](../../kernel/src/sched/mod.rs#L454), [S:1585](../../kernel/src/sched/mod.rs#L1585)).
Empty/offline-only masks therefore have no coherent park/reject behavior.
Do not expose writable affinity without handling this case.

`current_cpu_id` maps CPUID's initial 8-bit APIC ID directly to a scheduler index
and clamps at 63. AP TSS storage is assigned by bring-up order, while TSS updates
use CPU ID minus one ([S:3827](../../kernel/src/sched/mod.rs#L3827),
[smp.rs:179](../../kernel/src/arch/x86_64/smp.rs#L179),
[smp.rs:318](../../kernel/src/arch/x86_64/smp.rs#L318)). IPI destinations use the
same numeric ID. This assumes BSP ID 0 and dense, matching enumeration/APIC
indices; 12 CPUs is within the array bound but not proof of arbitrary hardware
topology support. CPU hotplug is not implemented here. Validate actual IDs and
TSS ownership on physical SMP systems before claiming general scaling.

### F9 — Medium performance/fairness limitation: balancing is conservative and policy differs on steal

An idle thief refuses a victim with exactly one queued Ready task, even if that
victim is running another task. Two tasks may consequently share a busy CPU
while an eligible CPU idles. Tail batching reverses order among stolen tasks;
victims are scanned from the same end and no residence-time metric is used.
Steal returns an eligible task without applying local aging/nice admission.
Nice service share can therefore depend on migration. These are verified policy
properties ([S:1175–1246](../../kernel/src/sched/mod.rs#L1175)), not measured
systemwide unfairness. Test one queued worker, skewed placement and affinity
rejection before changing batch size or victim selection.

### F10 — Unresolved risks and findings not established

- **Duplicate enqueue:** ordinary enqueue_once plus owner guard, targeted removal
  and S serialization provide a coherent defense. No ordinary duplicate producer
  was established. Private enqueue-to-core is not independently idempotent; its
  call-site contract matters. Diagnostics check metadata/queue consistency
  ([S:2891](../../kernel/src/sched/mod.rs#L2891)), but cannot detect F2's physical
  stack overlap. Generation-free indices make stale-entry recovery especially
  sensitive to slot reuse.
- **Lost wakeups:** reviewed IPC/pipe/TTY/futex check-and-register paths share S
  with wake, and live-owner wake requests go to that owner. No concrete lost
  wakeup was established in these paths. Spurious readiness wakes must retry the
  condition; TTY's periodic SMP wake may also wake BlockedOnIpc tasks with a
  pending call ([I:966](../../kernel/src/arch/x86_64/interrupts.rs#L966)). Audit
  consumers against exact pending request identity, not state alone.
- **Priority inversion:** no priority inheritance/donation was found in the
  active scheduler or IPC transition code. A latency-sensitive client can wait
  for a lower-service-share server, and all CPUs can wait behind an unrelated S
  holder. This is a supported inversion scenario, not a measured incident.
  Strict priorities would make it more important, not automatically solve it.
- **Watchdog:** checked at quantum expiry and compares that slice's tick count
  to the configured limit ([S:1041](../../kernel/src/sched/mod.rs#L1041)). It is
  not a hard elapsed-time execution watchdog and cannot preempt a syscall with
  IF off. F4/F5 further undermine interpreting the count as per-task runtime.
- **Interrupt latency:** whole syscalls have IF off; the scheduler cannot bound
  latency of an arbitrary long syscall/device transaction. Quantum changes alone
  do not establish a real-time audio guarantee.
- **Invalid saved context:** local selection may re-enqueue an invalid-context
  Ready entry indefinitely. That is defensive rejection, not recovery; the
  dispatch fallback's Ready check is weaker than candidate validation. No
  naturally generated invalid frame was reproduced. Add bounded rejection
  counters/tests rather than treating a rejected frame as a runnable success.
- **Deadlocks:** no normal S/core-lock cycle was proved in reviewed active
  paths. IRQ-off requirements, the latent empty-pop lock scope, nested subsystem
  locks and NMI shootdown behavior prevent claiming blanket deadlock freedom.

## Measurements: low-overhead design

Existing telemetry supplies per-CPU busy/idle time, timer callbacks, context
transitions and optional schedules/wake events. Wake events count the issuing
CPU, not the eventual target. Context transitions exclude same-task selection;
steal_count counts batches, not migrations. There are no wake latency, migration
or lock histograms. Task snapshots are capped at 64 and expose an address-space
generation truncated to 32 bits, not a full task incarnation
([telemetry.rs:288–299](../../kernel/src/telemetry.rs#L288),
[telemetry.rs:334–366](../../kernel/src/telemetry.rs#L334)). Use a dedicated
diagnostic identity for thread-level experiments.

The existing `sched_lock_hold_ns` print is **wait plus hold** for timer Phase 1:
the first timestamp is taken before acquisition, the second after guard release
([I:917–1004](../../kernel/src/arch/x86_64/interrupts.rs#L917)). It does not cover
Phase 2, IPC, reaping in other callers or every lock acquisition. It uses plain
RDTSC-derived now_ns; its name must not be taken as a validated lock-hold metric.

For a future minimal, default-off diagnostic feature, use preallocated aligned
per-CPU integer counters/logarithmic histograms and a bounded trace ring for
rare outliers. No UART, formatting, allocation, task-table walk, new shared
lock, or unbounded retry in a measured path. Export snapshots later via an
existing debug/QMP surface; mark overflows, rejected samples and trace drops.
Serialize timestamp reads where instruction ordering matters; calibrate empty
bracket overhead. Test cross-CPU TSC synchronization, monotonicity and migration
skew before subtracting clocks across CPUs. With only the 10 ms fallback clock,
report sub-tick measurements unavailable rather than zero.

| Metric | Minimal hooks and definition | Reporting/controls |
| --- | --- | --- |
| Wakeup-to-run | On a genuine blocked -> Ready transition store the **first** pending timestamp and full publication incarnation. At next dispatch consume it once. Separate still-owned wake (which may resume without a new user context) from detached wake. Optional userspace sequence acknowledgement measures first user execution; selection alone omits restore/IRET time. | p50/p95/p99 and max per wake source/CPU, sample count and censored tasks. Reset on block/exit/reuse; never reset timestamp on duplicate wake. Report approximate histogram quantiles with bucket bounds, not false ns precision. |
| Context switch | Per-CPU timestamps around selection, architectural save/restore, reap and whole IRQ separately; at a safe post-pivot point or userspace acknowledgement measure completed handoff. Same-task, task-task, idle-task and task-idle are separate. | Cycles/ns distribution, switch count, cycles per completed handoff. Pair same-VM thread ping-pong with different-VM IPC ping-pong. Half round-trip alone is **not** a pure switch cost; subtract/describe syscall, wake, IPC and cache costs. |
| Lock wait/hold | At each S acquisition: t0 before attempt, t1 after acquisition, t2 immediately before release. Record wait=t1-t0, hold=t2-t1 after release with fixed call-site tags. Do not replace blocking acquire with polling merely for instrumentation. | p50/p95/p99/max by timer Phase 1/2, IPC, input, spawn, reap, telemetry and memory/I/O site. Sum holds/wall time estimates serialized utilization; per-CPU waits show spin cost. Counters must include non-timer holders to attribute contention. |
| Fairness/worst wait | Track Ready-entry time, dispatch time and runtime delta per incarnation; also maintain runnable intervals and completed work in a fixture. Exclude blocked periods from service entitlement. | Equal-work neutral peers: Jain index `(sum x)^2/(n*sum x^2)`, min/max runtime and longest continuous Ready wait. Nice values must be reported separately because current weights are not defined proportional shares. Censored Ready tasks and F3 are failures, not omitted samples. |
| Migrations/scaling | Remember last-dispatch CPU per incarnation; increment when it changes. Count eligible/rejected steals, tasks moved, batch sizes and unsuccessful scans. | Migrations/task-second, steal success, per-CPU queue depths and idle-with-eligible-backlog. Throughput speedup T(C)/T(1), efficiency speedup/C for identical total work; separate fixed-total-work and fixed-work-per-CPU experiments. |
| UI/audio responsiveness | Userspace monotonic sequence IDs for injected input, receipt and first displayed acknowledgement; frame-present timestamps if available. audiod pump-gap, queue depth, completed frames and underrun deltas from existing status. | Input-to-visible p50/p95/p99/max, frame deadline misses, audio underruns/10 minutes and pump-gap max. Input-to-client receipt is a separate metric if display presentation cannot be observed. Correlate with lock tails and CPU busy; do not infer audible quality from DMA motion alone. |

Instrumented/uninstrumented paired runs must quantify observer overhead on both
idle and saturated workloads. A proposed acceptance budget is <=1% throughput
change for basic counters and <=2% for sampled timing, reported with run-to-run
variation; these are test goals, not current measured properties. Use every-event
lightweight counters, optionally sample detailed timing 1/64 events. Do not use
sampled data alone to claim absence of rare stalls. No new guest hooks are added
by this Phase A deliverable.

## Reproducible validation and benchmark matrix

### Completed host checks

| Command/check | Actual result on this baseline |
| --- | --- |
| `cargo check --package sunlight-kernel --bin sunlight-kernel --offline` | PASS, dev profile, 9.28 s reported by Cargo. Existing dependency cfg/style warnings; no kernel error. Build duration is not a scheduler benchmark. |
| `python3 tools/scheduler_policy_probe.py` | PASS as an **audit probe**: verified current starvation counterexample and neutral FIFO behavior. It exits zero when its expected observations hold, including the defect. |
| `rustc --edition=2021 --test tools/foreground-scope-proof.rs -o /tmp/scheduler-audit-foreground-tests` then run | 7/7 PASS, production scope/session logic with scheduler/wakeup fixture. No real context switching. |
| `rustc --edition=2021 --test ipc/src/cpu_accounting.rs -o /tmp/scheduler-audit-accounting-tests` then run | 4/4 PASS, runtime arithmetic including blocked syscall tails and migration accounting. |
| Production `kernel/src/timekeeping.rs` via a host wrapper defining the `serial_println!` macro, `rustc --test`, then `--test-threads=1` | 4/4 PASS. Direct standalone compilation first failed because the crate macro was absent; the wrapper supplies only that macro. Single-thread execution avoids the tests' shared reset state. |

Policy probe output (10,000 selection opportunities per row, one CPU, one test
task plus a nice-0 peer, initial burst=256, simulated ticks; not wall time):

| Test task nice | Test task selected | Peer selected | Test task final counter |
| --- | ---: | ---: | ---: |
| 0 | 5000 | 5000 | 0 |
| +7 | 95 | 9905 | -7 |
| +8 | 0 | 10000 | -7 |
| +9 | 0 | 10000 | -7 |
| +10 | 0 | 10000 | -7 |

The probe extracts `pick_next_round_robin`, scoring, counter, quantum functions
and constants from S. It stubs context validity, ownership and stealing and
requeues the selected fixture task; it does not execute schedule_tick, tick,
IRQs, burst growth on real quantum expiry or physical switching. The +8..+10
fixed-point argument is independent of the missing full dispatcher behavior;
the +7 count is only this fixture's observed count, not a promised service ratio.
It also produces neutral quanta 5/16/50 ticks for burst 0/256/1024.

To reproduce the timekeeper host wrapper from the repository root:

```sh
python3 - <<'PY'
from pathlib import Path
source = Path('kernel/src/timekeeping.rs').resolve()
Path('/tmp/scheduler-audit-timekeeping.rs').write_text(
    '#[macro_export]\nmacro_rules! serial_println { ($($a:tt)*) => '
    '{ let _ = format_args!($($a)*); }; }\n'
    f'#[path = "{source}"] mod timekeeping;\n')
PY
rustc --edition=2021 --test /tmp/scheduler-audit-timekeeping.rs \
  -o /tmp/scheduler-audit-timekeeping-tests
/tmp/scheduler-audit-timekeeping-tests --test-threads=1
```

### Guest functional gates available now

Run CPUs **1, 2, 4, 8, 12**; 1/2/4/12 are mandatory, 8 fills the scaling curve.
Run sequentially because tools/test.sh rebuilds and overwrites common artifacts.
Use a fixed host, QEMU version/CPU model, accelerator, RAM and device profile.
Archive kernel/ISO checksums, Git diff, feature/env settings, serial output and
host load. The script may delete temporary logs on exit: capture its console
and copy desired guest evidence before cleanup or use an isolated manual boot.

```sh
mkdir -p target/scheduler-audit
for cpus in 1 2 4 8 12; do
  for gate in phase_sec helios-thread-probe helios-io-probe shell-pty-liveness audio; do
    SUNLIGHT_TEST_CPUS="$cpus" SUNLIGHT_TEST_MEMORY_MB=4096 \
      ./tools/test.sh "$gate" \
      >"target/scheduler-audit/${gate}-${cpus}cpu.log" 2>&1 || exit 1
  done
done
```

These named gates exist in [tools/test.sh](../../tools/test.sh); CPU override is
read at [line 865](../../tools/test.sh#L865). `mm2b` forces 12 CPUs;
`mm2d`, `mm2e` and `swap1` force 4, so they are additional fixed-topology gates,
not adjustable matrix rows. `audio` opts into HDA with a no-output backend;
other gates do not implicitly exercise audio hardware. Phase_sec covers
borrower teardown, IPC bounded queues/deadlines/generation arbitration and 64
SMP terminal-state iterations ([expected messages](../../tools/tests/phase_sec.expected)).
It does not establish F1/F2 safety or latency bounds. PTY gates cover foreground
and session progress; existing scope host tests add stale-incarnation isolation.
No full workspace test command is claimed: the default bare-metal target is not
a host test runtime, and this audit did not change workspace implementation.

### Performance workloads and test fixtures

For each C, warm up 30 seconds, then measure 120 seconds; repeat five fresh
boots per workload. Run mixed interactive/audio soak for 10 minutes per boot.
Randomize performance workload order across boots. Keep total host vCPU demand
within physical capacity for KVM scaling; record SMT topology, host pinning,
frequency policy, other VMs and thermal state. Run TCG separately, labeled as
functional/emulated timing; do not combine it with KVM or physical results.

| Workload | Fixed, reproducible input and assertions |
| --- | --- |
| Idle/service baseline | Desktop and normal services, no user jobs. Capture busy/idle, timer/schedule rates, queue depth, same-task dispatch and lock time. Repeat a no-desktop profile separately if supported. |
| Pure CPU saturation | Fixed integer/hash work and deterministic seed; Nworkers = C, 2*C, 4*C. Use independent native worker threads without per-iteration syscalls. Start with a bounded yielding barrier; record work units/runtime per thread. Run fixed-total-work and fixed-work-per-CPU separately. |
| RR policy/fairness | Two then 4*C neutral workers; equal burst histories. Separate all-nice combinations -10, 0, +7, +8, +10. A fixture should constrain masks in test code because no public setaffinity service was found. No forced affinity ABI change is needed for Phase A. Check zero-progress workers explicitly. |
| Wake/block races | Deterministic seed; 100,000 IPC/futex/pipe handshakes, receiver Ready/owned/detached cases. Force events before registration, after registration before deschedule, and after deschedule. Deadline/reply/cancel simultaneous outcome: exactly one completion, no terminal resurrection, no queued owner. Run on one CPU and cross-CPU pairs. |
| Yield/quantum | Two CPU workers, one explicit yield at a known sub-quantum offset, idle-to-task wake, then continuous non-yielding runs. Count dispatch reasons, residual ticks and self-selection; test F4/F5 without inferring q from comments. |
| IPC throughput/latency | One client/server pair and C pairs on distinct endpoints, then C clients to one server. Fixed zero/small payload and one larger supported message shape; report calls/sec, round-trip and wake latency separately. Repeat with 4*C CPU workers. |
| I/O readiness | Bounded pipe/socketpair, eventfd and supported poll/epoll fixtures; alternate full/empty transitions, close/EOF, interrupted waits and timeouts. Check every sequence once, no EOF/output loss, bounded completion. Include TTY stdout backpressure and stdin arrival. |
| Lifecycle/stack safety | Repeated spawn/yield/block/exit/kill/reap/reuse with retained owner/borrower relationships; delay parent waitpid while other CPUs spawn. Session A cancellation concurrent with session B activity, stale generation messages, reconnect and repeated teardown. Assert physical stack quiescence (F1/F2), exact exit result and eventual reaping. |
| Skewed SMP load | C workers initially preferred to CPU 0, then C+1 and 4*C; single queued victim case; mixed eligible masks. Record idle CPUs with eligible backlog, per-task migrations, steal retries/batches and convergence. Requires a dedicated fixture using existing preferred-CPU helper. |
| Interactive mix | Continuous fixed 48 kHz stereo PCM stream through audiod, a documented asset/checksum and volume; animated UI plus injected key/cursor event every 100 ms with fixed seed; C IPC pairs at fixed offered rate; 4*C CPU workers. Measure audio underruns/pump gap, input-to-visible delay, frame gaps, IPC throughput and lock tails simultaneously. Repeat file reading/writing as a separate variant with identical disk image. |

The dedicated fairness/race/mixed-load driver and the proposed timing hooks are
**not implemented** by existing gates or the new host probe. The table specifies
their workload contract; it is not a claim that a generic benchmark command
already runs all of them. SunLight Bench supplies existing CPU work and worker
spawning, but its GUI run changes nice to -10 and its barrier yields; retain
those settings in reports rather than using its score as a neutral scheduler
microbenchmark ([bench main:995](../../sunlight-bench/src/main.rs#L995),
[thread barrier:122](../../sunlight-bench/src/thread.rs#L122)).

An isolated device-matched QEMU launch can use the freshly built, checksummed
ISO and a unique QMP socket. This no-disk command is a starting profile; attach
a private copy/snapshot of the same disk for fixtures requiring stored assets:

```sh
qemu-system-x86_64 -accel tcg -cpu max -smp 4 -m 4096 \
  -cdrom target/sunlightos.iso -display none \
  -serial file:target/scheduler-audit/mixed-4cpu.serial.log \
  -qmp unix:/tmp/scheduler-audit-4cpu.qmp,server=on,wait=off \
  -device virtio-rng-pci,disable-modern=on \
  -device qemu-xhci,id=xhci -device usb-mouse,bus=xhci.0 \
  -audiodev none,id=snd0 -device intel-hda -device hda-micro,audiodev=snd0 \
  -no-reboot -no-shutdown
```

For performance on a KVM host, use `-accel kvm -cpu host`, record that change
and keep it fixed across C. Use [tools/runs.sh](../../tools/runs.sh#L19) for a
supported graphical runner (`--cpus N --accel kvm --no-disk`); it does not itself
inject the mixed workload. Headless display-none runs can test render work but
need explicit presentation instrumentation or QMP image observations to claim
visible responsiveness. Keep audio backend, framebuffer resolution, disk/cache
and network configuration constant. Existing telemetry can be sampled with:

```sh
python3 tools/cpu_accounting_capture.py \
  --socket /tmp/scheduler-audit-4cpu.qmp \
  --kernel target/x86_64-unknown-none/debug/sunlight-kernel \
  --seconds 120 --period 5 --label mixed-4cpu \
  --output target/scheduler-audit/mixed-4cpu.jsonl
```

The ELF must be the exact one in that ISO. Optional `cpu_accounting_diag`
counters require an explicitly rebuilt kernel/ISO; enabling that feature alone
with cargo check does not change a boot image. Its bounded serial reports also
perturb timing; separate this baseline capture from the proposed silent timing
feature. The capture tool checks ABI v4/sequence and reads a bounded top-task
view; it cannot supply p99 wakeup latency or fairness of >64 tasks.

Acceptance checks: no panics/faults, no duplicate owners/queue entries, no
Finished/Reaped resurrection, no use-after-free or frame/TLS/FX corruption,
matching generations, no lost handshakes, eventual exit/reap, and no workload
left indefinitely Ready. Preserve exactly-once terminal IPC outcomes. Proposed
product targets should be set before comparing policies: e.g. zero additional
audio underruns over the idle control, no UI event loss and bounded p99 input
delay. Choose numeric latency budgets from supported devices/buffering; do not
declare a hard real-time SLA from the 100 Hz tick. Report failed/censored runs,
maxima and per-run percentiles, not only pooled averages.

## Alternatives evaluated separately

These are prospective tradeoffs, not implementations or measured winners.
Queue topology and scheduling policy are independent choices.

| Alternative | Expected benefit | Cost, risks and decision condition |
| --- | --- | --- |
| Optimized Round Robin | Keep current FIFO topology; fix forward progress, reschedule/quantum bookkeeping and physical handoff; reduce repeated scans/lock acquisitions and reserve storage. Lowest conceptual change and clearest baseline. | Still no workload-specific latency preference. Choose first; establish a defined quantum/fairness contract and measure before adding policy. Bounded deferred cleanup needs its own lifetime proof. |
| Priority Round Robin | FIFO within explicit priority levels can favor audio/UI service when CPU queues contend; a fixed small bitmap can make next-level selection cheap. | Starvation protection, aging, cross-priority fairness and IPC/server priority inversion require explicit rules. New queue membership/priority-change invariants. Current nice skip is not a sound substitute. Justified only if wake-ready wait remains the dominant interactive delay after correctness/critical-section fixes. |
| Per-CPU run queues with independent scheduling | Current queues already provide locality; removing S from local tick/select could reduce cache-line contention and serialized capacity. | Requires stable process/context lifetime, block/wake handoff, dual-queue migration protocol, affinity, cross-CPU IRQ-safe locking and generation-safe reuse. More physical queues alone changes none of the present serialization. Choose only after hold/wait measurements demonstrate S limits scaling and F1/F2 are resolved. |
| MLFQ | Use measured execution/block history to favor short interactive bursts without fixed application priorities. | Needs well-defined demotion, boosts, quantum accounting and anti-gaming/anti-starvation guarantees, including syscall-heavy IPC producers. Adds periodic work and membership transitions; present burst scores are not evidence of correct MLFQ. Evaluate only against measured interactive misses and a stable RR baseline. |
| Fair scheduling | Runtime-based service can target proportional shares and reduce opaque nice-counter behavior. | Needs documented weight semantics, runnable-runtime accounting, lag bounds, wakeup placement and group/thread fairness. A tree/heap typically introduces O(log Q) ordering work; alternatives must specify their actual data structure. Does not remove global lock or bound IRQ-off syscalls, and fair service alone is not an audio deadline guarantee. Appropriate if measured service-share error dominates after correctness fixes. |

## Risk-ranked incremental plan and queue recommendation

1. **P0 correctness, separate changes:** establish physical outgoing-stack
   ownership until completed handoff (F1/F2), then test kill/borrower/VM/slot
   reuse with forced interleavings. Preserve static exit/idle stacks, FXSAVE/FS
   state, TSS, active-VM tracking and shootdown forward progress. Do not remove
   global serialization as part of these fixes.
2. **P1 correctness/policy contract:** fix or explicitly constrain indefinite
   nice skips (F3), separate internal requeue from external reschedule request
   (F4), define/reset per-task quantum accounting (F5), preserve child completion
   until consumed (F7). Each gets focused production-path tests and existing
   IPC/session gates before performance tuning. Do not silently introduce
   priorities/classes while doing this work.
3. **P1 evidence, low implementation risk:** add silent default-off counters and
   timing histograms as specified, with bounded storage and host arithmetic
   tests. Measure the 1/2/4/8/12 CPU baseline and mixed load; collect actual
   wait/hold attribution, dispatch reasons, same-task cost and censored waits.
4. **P2 bounded hot-path work, moderate lifetime risk:** gate empty reap scans
   with pending-work metadata; bound cleanup per invocation and move heavy work
   only after proven stack/VM quiescence. Replace telemetry allocations with
   fixed scratch storage; remove/gate synchronous lifecycle UART from scheduler
   critical sections without hiding test outcomes. Avoid maintaining stale
   counters/indices when introducing cached PID/endpoint or deadline metadata.
5. **P2 queue/placement refinements, moderate policy risk:** reduce redundant
   core lock/CPUID accesses while preserving one-lock-at-a-time ordering;
   quantify/prevent queue growth allocations; evaluate the single-queued-victim
   steal case and locality-biased placement. Compare each change against neutral
   fairness, wake tails and audio/UI results, not just aggregate throughput.
6. **P3 concurrency split, high risk and conditional:** only if S wait remains
   substantial after the preceding work, design and review independent local
   scheduling with explicit migration/lifetime/lock-order invariants. Require
   SMP race gates and scaling evidence before adoption.
7. **P4 policy experiments, high risk and conditional:** compare priority RR,
   MLFQ and fair scheduling individually against the validated RR baseline with
   identical workload seeds, CPU counts, device profile and instrumentation.
   Reject an interactive improvement that starves IPC servers, harms lifecycle
   progress or loses audio/IRQ safety.

**Recommendation:** retain the existing per-CPU RR FIFOs for the next phase.
Multiple **priority** queues are not yet supported by evidence. Independently
locked per-CPU scheduling may eventually be justified by measured global-lock
wait and scaling loss, but the existing queues already implement the physical
topology and the critical missing work is safe handoff and shorter/bounded
critical sections. Treat a proposed decision threshold such as >5% saturated
capacity spent waiting on S, with worsening tails across C, as a trigger for
investigation—not a measured fact or sufficient proof by itself. Reassess after
the correctness fixes and source-matched measurements; Phase A performs no
scheduler rewrite or scheduling-class introduction.
