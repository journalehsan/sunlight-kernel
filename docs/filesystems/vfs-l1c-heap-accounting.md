# VFS-L1C heap accounting

The process vector, round-robin queues, and two empty descriptor tables explain the post-R5 kernel heap increase on 1 and 2 vCPUs and all but 160 B on 4 vCPUs. Serial ordinary spawn, exit, wait and cleanup reused those records through 1,000 cycles. Global samples occasionally captured live VFS buffers or timer telemetry scratch. The fixture adds measurement only. VFS-L1C-3 remains blocked.

The machine results include raw counts, checkpoint PIDs, one-based serial line numbers, image hashes and regression outcomes. [Heap accounting results](vfs-l1c-heap-accounting-results.json)

The earlier qualification remains available in the original deliverables. [Resource accounting report](vfs-l1c-resource-accounting.md), [original results](vfs-l1c-resource-accounting-results.json)

## Retained heap after the R5 fixtures

The first ordinary observer exercises `Scheduler::reserve_task_storage`. In this build a `Process` occupies 4,032 B. The live vector allocation grows from 16,128 B for four slots to 2,064,384 B for 512 slots. The layout probe captured both allocations and the release of the old one. After both R5 cases settle, the vector holds three live boot records and two reaped records. Those five records occupy 20,160 B of the allocation, including 8,064 B for the reaped records. Its 507 unused slots occupy the other 2,044,224 B.

The same reservation grows the round-robin queues to 512 indices each across all 64 scheduler core entries, including offline entries. Each index is 8 B. The 64 buffers total 262,144 B. Some queues already reserve 1,024 B per online CPU at boot, so their **net** increase depends on the CPU count. Tiered queues and the reaped records' ordinary IPC and deferred-reply queues held zero backing bytes in this fixture.

Each reaped record owns a 10,240 B boxed descriptor table with no live descriptors. The two boxes add 20,480 B. Slot reuse overwrites the old process and releases its box. The scheduler retains the exit record for its existing wait and reuse semantics. The kernel stack, arguments, environment and current-directory string are released by reap. The private cancellation path drops its process and descriptor boxes. No child-owned VFS continuation, registration claim, or ordinary IPC state remained after the corresponding settlement.

| vCPUs | Heap before R5 | Heap at ordinary cycle 0 | Process vector increase | Run queue increase | Empty descriptor boxes | Unexplained increase |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 278,920 | 2,607,752 | 2,048,256 | 260,096 | 20,480 | 0 |
| 1 | 277,800 | 2,607,656 | 2,048,256 | 261,120 | 20,480 | 0 |
| 4 | 280,808 | 2,607,752 | 2,048,256 | 258,048 | 20,480 | 160 |

All heap amounts are bytes. The 16 MiB mapped kernel heap is counted separately from allocator use. The linked-list allocator charges each live layout at a minimum of 16 B, rounded up to 8 B. Its free-list metadata lives inside free blocks. Rounding adds zero bytes to the process-vector, queue and descriptor allocations above. Temporary layout traces on 2 and 4 vCPUs measured 72 B of rounding in the initial and warmed whole-heap samples. For example, a 1 B VFS inheritance path string is charged as 16 B until the retain settles. Boot-lived allocations such as live service stacks and namespace bindings, the IPC shards, and the pre-existing swap-candidate queue are included in the initial sample. The VFS service's own userspace heap is outside the kernel allocator counter. The traced 4-vCPU boot had a 280,872 B starting sample and a 2,607,656 B warmed sample, which reconcile exactly. The trace identified a 96 B IPC reply-waiter buffer that was present in one boot profile and absent in another. The precise owners of the 160 B net variation in the primary 4-vCPU run remain unverified.

## Ordinary spawn checkpoints

Each 2 GiB QEMU TCG guest ran the pre-scheduler private construction and rollback fixture and both R5 cases first. Then init called the ordinary native spawn syscall for `/bin/true` and collected exit status zero using `waitpid`. The debug checkpoint waited for the child to reach `Reaped` with consumed wait status and for its VFS caller records, local leases and release obligations to clear. The checkpoint logged the global kernel heap before the first cycle and after 1, 10, 100 and 1,000 cycles. The guests ran in 2, 1, 4 vCPU order.

| vCPUs | Before first cycle | After 1 | After 10 | After 100 | After 1,000 |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 2,607,752 | 2,612,024 | 2,607,752 | 2,612,024 | 2,607,752 |
| 1 | 2,607,656 | 2,607,656 | 2,607,656 | 2,607,656 | 2,607,656 |
| 4 | 2,607,752 | 2,607,752 | 2,607,752 | 2,607,752 | 2,607,752 |

All three cycle-1,000 samples match their warmed heap baselines.

The two reaped rows, the process-vector capacity, and the queue backing stayed constant across the ordinary checkpoints. Each of the 1,000 ordinary reaps per guest returned 514 user frames and eight page tables and left zero frames owned by the child. Free frames, user-private frames and page-table frames matched the respective warmed baselines at every checkpoint. Both R5 cleanup routes returned the previous 550 user frames and eight page tables per child. The R5 checker also retained the original publication, release acknowledgement, late-cancellation and stale-completion checks.

## Allocations visible during cleanup

- **Pending VFS inheritance.** A reaper-only sample can overlap the copied retain exchange. The allocation-site trace caught the 1 B path string charged as 16 B in `Runtime::stage_inheritance` after the ordinary child had reaped. The path stays live until the correlated request settles. This is distinct from the empty descriptor box in the process record.
- **VFS copied file control.** A 4,272 B excursion matches the single `sys_vfs_retention` copied-file frame, consisting of 4,096 payload bytes and a 176 B header. The VFS service can hold this local buffer during its next control poll even after the child's retirement acknowledgements have cleared. The buffer is dropped when that syscall returns.
- **Timer telemetry.** On SMP the timer commits telemetry outside the scheduler lock. The first reaper-only 4-vCPU run caught excesses of 1,766,424 B at cycle 10 and 1,600,480 B at cycle 1,000. A separate allocation trace reproduced the large transient and attributed its dominant bytes to the live `TelemetrySnapshot` vectors, frame lists, and B-tree nodes in `commit_telemetry_snapshot_inner`. At trace cycle 30 the 1,766,680 B increase comprised 1,766,392 B of telemetry scratch and 288 B of pending VFS path strings. These syscall and telemetry allocations vary with when a snapshot crosses an in-flight operation. Their lifetimes follow the transport call or telemetry snapshot.

No persistent growth was demonstrated in the serial workload. The scheduler increase is reusable capacity plus bounded intentional retention. The 160 B difference predates the ordinary loop. The evidence did not justify a production lifecycle change or shrinking. The probe allocator used for allocation-site attribution was temporary and is absent from the fixture implementation. Its source, symbols, and serial logs are available in the local evidence directory `target/vfs-l1c-heap-accounting/`.

## Regression gates and qualification boundary

Production release compilation, host registration, IPC, VFS service, descriptor, scheduler, retirement, ZRAM and swap-slot tests passed. The session/transaction and shared-descriptor mmap feature builds passed. The existing isolated candidate at `/tmp/klc-q1/candidate` passed the ownership guard, SWAP-1 and ordinary Phase 3.0 on all three CPU counts. The accounting ISO was built from the primary workspace and used the same isolated 2 GiB QEMU configuration. The primary ownership guard still reports the same four pre-existing findings, with zero additional findings from this fixture.

The ordinary spawn entry points and ZRAM remain outside the instrumentation. The result covers serial small children with the exercised descriptor and VFS profile. Transaction-specific populated swap, regular-file descriptions and child-owned IPC resources, service lifetime reuse, and parent-loss cases retain their existing qualification requirements. VFS-L1C-3 and later migration stages remain blocked.
