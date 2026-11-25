# SMP scheduler work stealing

Source review of the current working tree on 2026-11-25. The findings below
describe code paths and possible outcomes. This review did not run QEMU or
measure scheduler latency or lock contention.

## Active Round Robin path

1. **Place runnable work.** Normal enqueue selects the affinity-eligible logical
   CPU with the lowest load, counting its FIFO entries and one current task if
   present. Ties favor lower logical CPU IDs. The enqueue path publishes the
   entry before requesting dispatch on the destination. Remote dispatch
   requests use a coalesced LAPIC IPI flushed after releasing guards. Ordinary
   preemption requeues runnable work on the same CPU.
   [Placement](../../kernel/src/sched/mod.rs#L558) ·
   [Enqueue](../../kernel/src/sched/mod.rs#L830) ·
   [IPI flush](../../kernel/src/sched/mod.rs#L4496) ·
   [Local requeue](../../kernel/src/sched/mod.rs#L2046)
2. **Find work.** Round Robin scans the local FIFO from the front for a task it
   can admit. If the local scan yields no task, it tries peers in ascending
   logical CPU ID order. It skips its own CPU and victims whose queue
   `try_lock()` fails. For each victim with at least two FIFO entries it pops
   up to half the queue from the tail, capped at 32 entries. The first eligible
   task is returned for dispatch. Additional eligible entries are appended to
   the thief's FIFO in tail-pop order. An idle CPU's local timer requests
   another scheduling pass.
   [Active policy](../../kernel/src/sched/mod.rs#L144) ·
   [Local selection](../../kernel/src/sched/mod.rs#L1703) ·
   [Victim scan](../../kernel/src/sched/mod.rs#L1506) ·
   [Idle timer](../../kernel/src/sched/mod.rs#L1350)
3. **Validate and hand off.** A stolen task must be Ready, allowed by its
   `cpu_mask`, free of live ownership on another CPU, and have a dispatchable
   saved context. If the outgoing kernel stack is still pinned on another CPU,
   the thief leaves the task on the victim queue and registers for a retry
   notification after release. Timer and reschedule IPI dispatch hold the
   global `SCHEDULER` mutex across selection. The victim's `try_lock()` applies
   only to the per-CPU queue lock.
   [Eligibility](../../kernel/src/sched/mod.rs#L1549) ·
   [Stack release request](../../kernel/src/sched/mod.rs#L1058) ·
   [Timer dispatch](../../kernel/src/arch/x86_64/interrupts.rs#L1306) ·
   [IPI dispatch](../../kernel/src/arch/x86_64/interrupts.rs#L1015)

## Open findings

| Path | Finding | Effect |
| --- | --- | --- |
| RR victim scan | The thief fixes the batch size before checking affinity. With a victim FIFO ordered `[migratable, victim-only]` from head to tail, it inspects one entry and restores the victim-only entry to the tail. | While the FIFO stays in that order, retries can repeatedly miss the migratable task. [Batch selection and reinsertion](../../kernel/src/sched/mod.rs#L1515) |
| RR steal threshold | The threshold examines FIFO length alone. Placement also counts the current task. | A CPU running one task with one eligible task queued can keep the queued task waiting while a peer is idle. [Steal threshold](../../kernel/src/sched/mod.rs#L1517) · [Placement load](../../kernel/src/sched/mod.rs#L571) |
| RR immediate dispatch | The first eligible stolen task returns directly to dispatch. The local FIFO path applies nice-counter accumulation, skipping, promotion, and aging to a fresh turn. | Fresh turns can receive different admission treatment depending on how they were selected. [Immediate selection](../../kernel/src/sched/mod.rs#L1573) · [Local admission](../../kernel/src/sched/mod.rs#L1758) |
| Inactive BORE fallback | The last-resort process-table scan checks Ready state and remote ownership. It omits affinity, remote stack pin, and saved-context checks. | Review the fallback before enabling BORE. [Fallback scan](../../kernel/src/sched/mod.rs#L1671) |

The affinity-tail case is the first regression target. A two-CPU case should
cover a victim FIFO with an eligible head and a victim-only tail, and verify
that the idle peer can find the eligible task while preserving the pinned task's
queue membership and affinity.
