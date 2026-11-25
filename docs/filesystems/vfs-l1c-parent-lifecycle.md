# VFS-L1C parent lifecycle qualification

Two normal parent-exit orderings passed in fresh 2 GiB QEMU TCG guests, in the order **2, 1, then 4 vCPUs**. A disposable process exited with an unfinished private child while the VFS registration remained live. The child never entered the scheduler. The [machine results](vfs-l1c-parent-lifecycle-results.json) record image hashes, exact QEMU arguments, transaction identities, allocator samples, and one-based serial line numbers. The earlier branch matrix remains in the [adapter accounting report](vfs-l1c-adapter-accounting.md).

## Normal exit and cleanup progress

The debug fixture `vfs_l1c_parent_lifecycle_test` reuses the native publication and accounting infrastructure. Init spawns the disposable parent through ordinary spawn. The parent uses inherited VFS authority, constructs a private child, stages one retain and exits through the ordinary exit syscall with status 37. Init verifies that it can collect that status exactly once. The second case starts after the first has settled.

| Parent exit checkpoint | 2 vCPUs | 1 vCPU | 4 vCPUs | VFS outcome |
| --- | --- | --- | --- | --- |
| Retain still unsent | Pass | Pass | Pass | The exact queued retain and its caller-delivery reservation cleared. No child receipt or release acknowledgement was observed. |
| Authenticated retain reply held before publication | Pass | Pass | Pass | The continuation carried the retained receipt at exit. The live service acknowledged the matching release. |

The first 2-vCPU guest ran the unsent case with the existing syscall-return orphan hook bypassed for the fixture transaction. The parent exited and reaped. During five host seconds after init collected status 37, the serial showed neither private teardown nor transaction settlement. Normal exit makes a one-way stack handoff and bypasses ordinary syscall return, so the return hook alone gives the adapter no guaranteed disposal edge.

The repair is scoped to the debug adapter. After normal exit commits and releases the scheduler guard, the exit edge matches the retiring caller's exact identity to the staged transaction and takes the adapter. The adapter guard is released before the child owner drops and reclaims the private image. All six passing transactions still bypassed the syscall-return orphan hook for the fixture. Scheduler polling and the authenticated VFS protocol continued to handle reaping and registration settlement.

## Resource accounting

| Resource | Disposition in each passing transaction |
| --- | --- |
| Parent process and descriptors | Normal reaping cleared three live descriptors, the kernel stack, address-space regions, and pending IPC state. The scheduler kept a reusable process slot and its empty 10,240 B descriptor table. The next parent reused that slot. |
| Parent physical memory | The reaper returned 534 user frames and eight page tables. The exact parent owned zero PMM frames at settlement. |
| Private child process and descriptors | Rollback closed three initial descriptors and dropped the private process box and descriptor table box once. The census found neither a published process nor a reaped scheduler slot for the child. |
| Child physical memory | Rollback returned 550 user frames and eight page tables. The exact child owned zero PMM frames at settlement. Parent and child both had zero populated swap blocks. |
| Pending child, adapter, and continuation | The exit edge emptied the adapter slot and rollback freed the private image. Polling cleared the dead parent's continuation and released the pending-child PID and remote claim after VFS settlement. The inline adapter and continuation backing remains available for reuse. |
| Registration and other transaction state | The service registration stayed active in the same epoch. The parent namespace binding, caller row, and associated lease or release obligations drained. The child had no namespace binding, claim, caller-delivery state, or exact local, delivered, or unsent retain. Parent and child had zero pending IPC references and stat charges, with no outstanding caller publications or chdir operations. |

Child construction reduced the global free-frame sample by 558 pages in all six cases. Settlement increased free frames by 542 relative to the live-parent baseline, matching the parent's 534 user frames and eight page tables. Global heap changes from the live-parent baseline were −37,352 B and −33,080 B at 2 vCPUs, −33,080 B in both cases at 1 vCPU, and −32,920 B and −33,080 B at 4 vCPUs, in case order. These samples include parent and service activity. The disposition verdict uses the owner census and teardown records. The retained process-vector capacity was 512 records, and the reaped slot retained an empty descriptor table.

## Regressions and CPU-specific diagnostics

- Production release compilation passed. The host registration suite passed 105 tests, including the 112-cycle mixed ownership regression. IPC, VFS service, descriptor, kernel, scheduler, retirement, ZRAM, and swap-slot host checks also passed.
- The isolated candidate passed the ownership guard. The primary workspace reproduced the same four pre-existing ownership findings as adapter accounting, with zero new findings.
- Ordinary Phase 3.0 boot passed in the isolated candidate at 2, 1, and 4 vCPUs. Standard SWAP-1 passed at its harness-selected 4 vCPUs with two pools.
- Exact 2- and 1-vCPU SWAP-1 diagnostics reached the gate's two-pool prerequisite. The current CPU-based policy permits only one pool at those counts. Both runs are recorded as an unmet diagnostic prerequisite. The parent lifecycle cases have no such prerequisite.

Both ordinary spawn entry points retain their existing behavior. The debug build adds an embedded disposable parent for the native test. ZRAM remains independent of VFS.

## Remaining qualification gaps

This pass covers normal parent exit while a live registration has an unsent retain or an accepted retain reply awaiting publication. It leaves parent exit with an acknowledgement still in flight, parent exit concurrent with registration retirement, remote termination, and parent image replacement outside the observed matrix.

VFS-L1C-3 remains blocked under the existing requirements. The inherited gaps include trusted cross-lifetime registration row reuse and a transaction census with regular-file descriptions, child-owned IPC endpoints, queued payloads, populated swap, and fault-driven swap-in. Full heap attribution still includes the original unattributed 160 B and the userspace VFS heap. Interactive Btop and Qalculate remain prerequisites. VFS-D1 stays deferred.
