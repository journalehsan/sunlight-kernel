# VFS-L1C final resource accounting review

**Scoped qualification only. VFS-L1C-3 remains blocked.** The measured scheduler capacity is intentional and reusable. The primary 4-vCPU run has a stable 160 B accounting residual whose allocation owner is unknown. The serial workload showed no cumulative growth at the measured checkpoints.

This review preserves the [resource report](vfs-l1c-resource-accounting.md) and [results](vfs-l1c-resource-accounting-results.json), the [heap report](vfs-l1c-heap-accounting.md) and [results](vfs-l1c-heap-accounting-results.json), and the historical R-series outcomes. The [closure results](vfs-l1c-resource-closure-results.json) index the reconciliation and remaining gates.

## The 160 B residual

The residual is the change between the pre-R5 and warmed samples after subtracting the known scheduler and descriptor allocations. It is not the difference between 4-vCPU and 2-vCPU warmed totals; those totals are equal.

| Sample | Pre-R5 heap | Warmed heap | Observed delta | Accounted delta | Residual |
| --- | ---: | ---: | ---: | ---: | ---: |
| Primary 1 vCPU | 277,800 | 2,607,656 | 2,329,856 | 2,329,856 | 0 |
| Primary 2 vCPUs | 278,920 | 2,607,752 | 2,328,832 | 2,328,832 | 0 |
| Primary 4 vCPUs | 280,808 | 2,607,752 | 2,326,944 | 2,326,784 | 160 |
| Traced 4 vCPUs | 280,872 | 2,607,656 | 2,326,784 | 2,326,784 | 0 |

The 4-vCPU accounted delta is 2,048,256 B for the process vector (4 to 512 slots), 258,048 B net for the 64 round-robin buffers, and 20,480 B for two empty descriptor tables held by reaped records. Slot reuse frees the old descriptor table. The mapped 16 MiB heap is separate from the allocator's used counter.

The primary serial log has 280,808 B at line 1888, then 2,607,752 B at lines 2008 and 2010. It holds that value after 1, 10, 100 and 1,000 ordinary cycles (lines 2026, 2166, 3569 and 17554). The separate trace has 280,872 B at line 1949 and 2,607,656 B at lines 2150 and 2153. Relative to the primary run, the trace starts 64 B higher and ends 96 B lower. Both offsets enter the residual; treating the warmed 96 B difference as an explanation for all 160 B would omit the baseline difference. The trace identified a 96 B IPC reply-waiter buffer that varied across boot profiles. That establishes a candidate class of background allocation, not an identity match to either primary checkpoint.

The known allocations incur zero layout rounding. In the separate trace the initial and warmed whole-heap rounding charges are both 72 B, so rounding contributes zero to that trace's net delta. The allocator's free-list metadata resides in free blocks. The trace independently attributes large in-flight excursions to timer telemetry scratch; it also identifies pending VFS paths, while separate samples match the 4,272 B copied file frame. None of this evidence identifies the primary 160 B as telemetry, rounding, or intentional retention. The original primary samples lack a simultaneous live-layout census. Exact attribution would require a matched allocator census at both boundaries of a run that reproduces the residual. The local source logs are under `target/vfs-l1c-heap-accounting/` (`settled/native-4.serial.log`, `trace-4.serial.log`, `layout-symbols.log`, and `trigger-symbols.log`).

Across 1,000 ordinary spawn/exit/wait/reap cycles per guest, the final heap matched the warmed sample on all CPU counts. The process slots and queue backing stayed stable. Each ordinary reap returned 514 user frames and eight page tables; the checkpoints matched the warmed free-frame and PMM class counts. The transient 4,272 B elevations on 2 vCPUs cleared by cycle 1,000. This supports reuse for this serial workload. It does not establish behavior for concurrent children, heavier resource profiles, or longer service lifetimes.

## Publication adapter coverage

The accounting matrix used fresh 2 GiB QEMU TCG guests at 1, 2, and 4 vCPUs. Each ran 100 private create/cancel cycles, 12 construction checkpoints, two ledger exhaustion cases and 523 PMM allocation failures with baseline recovery, followed by the two R5 transactions. Private construction rollback is a separate witness from disposal of an installed publication adapter.

| Disposal route | Evidence and remaining boundary |
| --- | --- |
| Successful transfer to scheduler | R5 accounting qualifies one adapter disposal, one publication and one reap; the child used standard descriptors and copied stat. |
| Cancellation with a held authenticated retain reply | R5 accounting qualifies one private disposal with no scheduler row; the late reply generates correlated compensation, the release settles, and stale completion is rejected after continuation slot reuse. |
| Unsent cancellation and cancellation after consumed retain | The [sustained fixture](vfs-l1c-2b.md) qualifies ownership transitions. The full R5 resource census was not run for either ordering. |
| Service retirement before or after acquisition; after retain consumption; after publication | [R1B](vfs-l1c-2b-r1b.md), [R1](vfs-l1c-2b-r1.md), [R2](vfs-l1c-2b-r2.md), and [R3](vfs-l1c-2b-r3.md) supply native lifecycle witnesses. R2 qualifies authority rejection, and R3 preserves scheduler ownership after transfer. The complete resource census across these authority retirement branches remains open. |
| Failed preparation with a live registration | The adapter's failed-publication arm cancels the private child and consumes the terminal if present. Native accounting for preparation rejection is not run. R2's authority rejection does not cover it. |
| Authentic service refusal | Native fixture disposal and resource accounting are not run. |
| Parent gone | The syscall-return orphan hook takes the adapter and drops it outside the guards. Native accounting and progress without a later qualifying syscall return remain unqualified. |

The two R5 accounting paths each returned 550 user frames and eight page tables, with zero populated swap slots. Settlement checked zero child-owned PMM frames, private PID reservation, adapter, continuation, VFS claim, ordinary IPC references and stat charges. The published process can retain an empty descriptor table. The accounting run supplies the scoped 4-vCPU R5 witness that the earlier [R5 report](vfs-l1c-2b-r5.md) lacked; neither run qualifies a registration row reused by a different service lifetime.

## Gates and migration decision

The recorded production release, host registration (including the separate 112-cycle mixed ownership regression), IPC, VFS service, descriptor, kernel/scheduler, retirement, ZRAM and swap slot checks passed within their recorded scopes. The transaction/session and mmap feature builds passed. The isolated candidate passed the ownership guard, SWAP-1 and ordinary Phase 3.0 on all three CPU counts. The primary guard still has four pre-existing findings; its ordinary gate stopped before boot. These are inherited results verified against the saved evidence, with no new guest or build run for this documentation review.

Keep VFS-L1C-3 blocked pending the remaining disposal and resource qualification above, trusted cross-lifetime registration reuse, and a transaction census with regular-file descriptions, child-owned IPC endpoints and queued payloads, populated swap, and fault-driven swap-in. SWAP-1 independently qualifies its own ZRAM-backed pressure workload. Full heap attribution, including the original residual and service heap outside the kernel counter, remains open. The existing interactive Btop and Qalculate prerequisites are not qualified by this review; VFS-D1 stays deferred.

Only this report and its machine-readable companion change. Scheduler retention policy, both ordinary spawn entry points, and ZRAM's independence from VFS are preserved.
