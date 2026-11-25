# VFS-L1C-2B-R5: Repeated completion handling

**Partial native qualification.** All three cases passed in fresh QEMU TCG guests with 2 GiB RAM on 2 and 1 vCPUs, in that order. The 4-vCPU run was stopped when testing shifted to the existing regression gates. Four-vCPU R5 remains unqualified.

| Case | 2 vCPUs | 1 vCPU | 4 vCPUs |
| --- | --- | --- | --- |
| Published child | PASS | PASS | Unqualified |
| Cancelled and reclaimed child | PASS | PASS | Unqualified |
| Completed registration operation after continuation slot reuse | PASS | PASS | Unqualified |
| Reuse of the service registration row across service lifetimes | Not run | Not run | Not run |

The debug fixture captured the original authenticated VFS retain replies and passed the copied bytes to the production completion handler. It preserved the full registration, request, and child identities. It used the existing boot service, publication adapter, and private child teardown. The ordinary spawn entry points, VFS permissions, boot gate, ownership guard, and ZRAM paths were unchanged. No production defect was observed and no production repair was made.

For the published child, the original retain completed the parked continuation. Publication installed the binding, removed the continuation, transferred sole ownership to the scheduler, and inserted and enqueued the child once. A repeated completion was rejected with the exact binding and process identity still intact. Repeated wait-for-publication and publication found no eligible transaction. The adapter was disposed once. The child dispatched, completed its metadata operation, exited, and yielded one wait status. The original receipt's release was acknowledged once; the scheduler reaped 551 user frames and eight page tables, with zero swap blocks.

For the cancelled child, the legitimate service reply was held after service acquisition. The parked continuation completed with cancellation; the adapter reclaimed the unpublished child once. Teardown removed three descriptor entries, 551 user frames, eight page tables, and the process and descriptor table boxes. No swap slots were populated. The original late completion queued compensation after local reclamation. The repeated completion was rejected, with no continuation, binding or scheduler child created. One correlated release acknowledgement preceded final settlement and removal of the pending PID claim.

For the reuse case, the second transaction occupied the first transaction's continuation slot under the same live service registration. While the second continuation was parked and its own reply had arrived, the completion handler rejected the first transaction's original bytes. The fixture checked that the new claim was still live, parked and without a result; its own reply then followed the cancellation path. This qualifies reuse of the per-operation continuation slot. Reuse of the service registration row by a different service lifetime remains unqualified.

The [machine results](vfs-l1c-2b-r5-results.json) index serial logs and event line numbers, including both transaction identities and the resource counts. The initial 2-vCPU pilot is preserved separately: its checker conflated an earlier observer PID with the later private child. Final checks scope the no-publication and no-reap assertions to the later transaction.

VFS-L1C-2B remains partial. VFS-L1C-3 remains blocked pending remaining resource and lifecycle qualification. A full resource census, populated swap teardown in a creation transaction, fault-driven swap-in, and interactive Btop and Qalculate were not covered. VFS-D1 remains deferred.
