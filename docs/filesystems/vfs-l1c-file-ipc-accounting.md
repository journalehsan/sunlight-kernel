# VFS-L1C file and IPC resource accounting

The ordinary native spawn, exit, wait and reap path completed ten file and IPC
workloads per guest on fresh 2 GiB QEMU TCG boots, in 2, 1, 4 vCPU order.
Init passed its three standard descriptors explicitly, waited for status 37
and checked that it could not collect the same status twice. Each child waited
for its own namespace admission under the active VFS registration, opened
two regular files, checked shared read position through the duplicate,
created two ordinary IPC endpoints and touched two SHM allocations before
exiting with all resources still open.

| Resource | Live checkpoint | After reap and VFS settlement |
| --- | --- | --- |
| Frames and page tables | 537 mapped user pages and 537 PID-owned frames | Reap returned 534 private user frames and eight page tables; the three SHM pages were accounted through the separate shared lifetime. Zero PID-owned frames and zero swap blocks. |
| Descriptors | Six entries: three inherited, two opens and one duplicate. Global kernel description and reference counts rose from 72/72 to 74/78. | No live entries; kernel counts returned to 72/72. The reaped scheduler row kept its empty 10,240-byte table until reuse. |
| Ordinary IPC | Two child-owned endpoints, zero queued messages and zero measured endpoint queue backing. | No child-owned endpoints, matching endpoint queues, local IPC state or shard message/reply references. |
| Shared memory | Two objects with one mapping reference each; 12,288 bytes charged to the child incarnation. | No owner or mapping entries, both broker tokens absent, zero child quota bytes. Broker SHM object count returned to one boot-owned object. |
| VFS | The child had a namespace binding in the live registration. | The original registration stayed active. The settlement check waited for matching caller, lease and release records to clear, then checked absence of the namespace binding, child VFS publication and spawn rows, and quota charges. This workload did not create a private publication adapter. |

| vCPUs | Free frames, before and after | User-private frames, before and after | Page-table frames, before and after | Kernel heap before → after |
| --- | ---: | ---: | ---: | ---: |
| 2 | 261,754 | 28,198 | 84 | 278,920 → 2,597,768 B |
| 1 | 262,010 | 28,198 | 86 | 277,800 → 2,597,672 B |
| 4 | 261,242 | 28,198 | 84 | 280,808 → 2,597,768 B |

The first cycle grew the reusable process vector from 4 to 512 slots
(2,048,256 B net). Round-robin queue backing grew to 262,144 B from
2,048, 1,024, and 4,096 B respectively. One reusable record held the empty
descriptor box. Those three allocations leave 256, 256, and 416 B of the
net heap changes unattributed. The settled heap returned to the same
warmed total after cycle ten; individual checkpoints on SMP saw temporary
elevations. Global heap samples include background allocations and do not
measure the VFS server's userspace heap.

The only changes are an opt-in fixture, its live and settlement probes, and
build wiring. The first 2-vCPU attempt completed four cycles, then failed
the fixture's live namespace assertion in cycle five. The final fixture
explicitly waits for admission before opening files and passes all three
standard descriptors. No production resource defect was established.
Build with `CARGO_NET_OFFLINE=true cargo build -p sunlight-kernel --features vfs_l1c_file_ipc_test`,
then use `tools/make_hybrid_iso.sh` on the debug ELF. The [machine results](vfs-l1c-file-ipc-accounting-results.json)
include artifact hashes, samples, and serial line numbers. Local logs,
build output and QEMU arguments are under `target/vfs-l1c-file-ipc-accounting/`.

Production release compilation and the host registration (107 tests), IPC,
VFS service, descriptor, scheduler, kernel, retirement, ZRAM and swap-slot
regressions passed. The primary ownership guard reproduced four pre-existing
findings; the isolated candidate reported zero new findings and passed
ordinary Phase 3.0 boot on all three CPU counts.
The same candidate passed standard two-pool SWAP-1 on four vCPUs. The
fixture changes do not touch swap or ZRAM, and the pressure run remains
independent of the VFS workload.

This run leaves queued payload and reply-waiter teardown, peer-held SHM
mappings, service-side file object and heap census, populated swap in this
workload, cross-lifetime registration reuse, and concurrent workloads open.
The earlier resource closure and production exec reports remain the evidence
for their respective adapter and exec paths. Both ordinary spawn entry
points are preserved. Detached publication stays disabled, L1C-3 stays
blocked with its existing prerequisites, and VFS-D1 stays deferred.
