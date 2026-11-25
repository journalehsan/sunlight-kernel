# VFS-L1C adapter resource accounting

**Scoped qualification. VFS-L1C-3 remains blocked.** This pass uses the branch inventory and accounting criteria from the [resource closure](vfs-l1c-resource-closure.md). The [machine results](vfs-l1c-adapter-accounting-results.json) give the per-branch identities, allocator samples, disposition counts, image hashes, and one-based serial line numbers.

## Native execution

The debug adapter accounting fixture extends the existing publication census. Each profile starts in a fresh 2 GiB QEMU TCG guest with 2 vCPUs, followed by fresh 1- and 4-vCPU guests for successful scenarios. The live-registration profile runs the two R5 controls first. It then runs unsent cancellation, cancellation after retain consumption, and preparation rejection. A second build of this profile adds service refusal. The accounting feature is `vfs_l1c_adapter_accounting_test`. The retirement images also enable the corresponding R-series feature. The retirement cases each boot a separate guest because they terminate the registered VFS service. PID 1 collects the observer or published child and waits for the relevant audit slot to drain.

Preparation rejection is injected at the existing publication callback after the kernel has validated live authority and the retained receipt. Refusal is triggered by withholding the derivation source for the exact fixture request. The ordinary userspace VFS service generates and sends the refusal through its authenticated control path. The retirement profiles use the existing R1B, R1, R2, and R3 checkpoints and service-image retirement proof.

| Disposal route | 2 vCPUs | 1 vCPU | 4 vCPUs | Observed disposition |
| --- | --- | --- | --- | --- |
| Publication and normal reap (R5 control) | PASS | PASS | PASS | Scheduler owns the published child. The reaper consumes the execution resources. |
| Held retain reply, cancellation, and compensation (R5 control) | PASS | PASS | PASS | Private image drops, late acknowledgement queues release, and stale completion is rejected. |
| Unsent cancellation | PASS | PASS | PASS | The queued retain is removed before delivery. No retain receipt or release acknowledgement is present. |
| Cancellation after retain consumption | PASS | PASS | PASS | An authenticated receipt is held before cancellation. The matching release acknowledgement settles it. |
| Preparation rejection with live authority | PASS | PASS | PASS | Publication returns the injected preparation error. The failed arm drops the private image and the release settles. |
| Authenticated service refusal | PASS | PASS | PASS | The service returns `Refused(Resolve)`. Private cleanup settles without acquiring or releasing a receipt. |
| Service retirement before acquisition (R1B) | PASS | PASS | PASS | The retain is still queued. Matching service-image proof collects the old registration. |
| Service retirement after acquisition (R1) | PASS | PASS | PASS | The held authenticated reply is discarded. Matching service-image proof collects the old registration. |
| Service retirement after retain consumption (R2) | PASS | PASS | PASS | Publication rejects the withdrawn authority. Image proof settles the release obligation. |
| Service retirement after publication (R3) | PASS | PASS | PASS | The child remains scheduler-owned through service retirement. It exits and reaps before the adapter drops. |
| Parent loss | Unqualified | Unqualified | Unqualified | Native disposal and autonomous progress remain open. |

Each passing branch returned **550 child user frames and eight page tables**. Each child had zero populated swap blocks.

## Ownership and allocation disposition

The staged census keys each transaction by child ticket, retain request, and service registration. It witnesses one private process box, one descriptor table with three live entries, one reserved PID, one adapter, one continuation, and one VFS claim. The private rollback reports closed descriptors, reclaimed user frames and page tables, released swap blocks, and both heap boxes dropped. A published child transfers its process to the scheduler and closes descriptors and reclaims its address space and stack at normal reap. The R3 checkpoint confirms the child continues under scheduler ownership after the service image retires and before the child is reaped.

Settlement requires zero child-owned PMM frames, private PID reservation, continuation, namespace binding, associated VFS claim and caller-delivery state, pending IPC references, and stat charge. For live registrations the census also checks the exact request has cleared local, delivered, and unsent slots. For retirement it requires the old registration row to have been collected. R5 cancellation, consumed-retain cancellation, and preparation rejection correlate the release acknowledgement with the original retain. The two R5 controls also retain the prior publication, stale-completion, and compensation checks.

The before-construction and constructed samples differ by 558 free frames. The observed construction heap increase is 47,072 B, or 47,080 B for the R3 image. The scalar heap and free-frame samples include unrelated activity. The resource verdict comes from the exact owner census, branch-specific transfer or drop, and teardown counts. The scheduler process vector has space for 512 records after warmup. Each reusable reaped slot may retain a 10,240 B empty descriptor table. The machine results record process vector, round-robin queue, tier queue, and reaped IPC backing separately from live state. Boot-lived inline adapter support tables can retain storage while the transaction's entry is empty. The 16 MiB mapped heap is an allocator capacity figure.

The table reports raw changes in the global free-frame and heap-used samples for the newly covered branches, ordered by 2, 1, and 4 vCPUs. The retirement profiles include the service reaper and scheduler warmup.

| Branch | Free-frame change | Heap-used change (B) | Measured reusable scheduler and descriptor capacity change (B) |
| --- | --- | --- | --- |
| Unsent cancellation | +0 / +0 / +0 | +0 / +0 / +0 | +0 / +0 / +0 |
| Cancellation after retain consumption | +0 / +0 / +0 | +0 / +0 / +0 | +0 / +0 / +0 |
| Preparation rejection with live authority | +0 / +0 / +0 | +0 / +0 / +0 | +0 / +0 / +0 |
| Authenticated service refusal | +0 / +0 / +0 | +0 / +0 / +0 | +0 / +0 / +0 |
| Service retirement before acquisition (R1B) | +22,314 / +22,314 / +22,314 | +2,285,544 / +2,286,568 / +2,283,496 | +2,328,832 / +2,329,856 / +2,326,784 |
| Service retirement after acquisition (R1) | +22,314 / +22,314 / +21,792 | +2,285,544 / +2,286,568 / +2,316,888 | +2,328,832 / +2,329,856 / +2,316,544 |
| Service retirement after retain consumption (R2) | +22,314 / +22,314 / +21,792 | +2,285,544 / +2,286,568 / +2,316,888 | +2,328,832 / +2,329,856 / +2,316,544 |
| Service retirement after publication (R3) | +22,314 / +22,314 / +22,314 | +2,295,784 / +2,296,808 / +2,293,736 | +2,339,072 / +2,340,096 / +2,337,024 |

In the 4-vCPU R1 and R2 samples, the independent observer was still awaiting reap when the child census passed. The observer subsequently returned 514 user frames and eight page tables. These records are distinct from the private child's already-cleared ownership.

The original primary 4-vCPU run's **160 B residual remains unattributed**. The observed deltas in this matrix are from different samples and provide no identity match to those original allocations. Full heap attribution, including the userspace VFS heap, remains open.

## Regressions and migration prerequisites

Production release compilation and the host registration, IPC, VFS service, descriptor, kernel, scheduler, retirement, ZRAM, and swap-slot suites passed. The registration suite passed 105 tests, including the 112-cycle mixed ownership test. The isolated candidate passed the ownership guard. The primary workspace guard reproduced all four pre-existing findings with zero accounting-change findings.

Ordinary candidate Phase 3.0 boot passed at 2, 1, and 4 vCPUs with an extended timeout. The initial attempts timed out before emitting serial output. Candidate SWAP-1 passed in its standard 4-vCPU configuration with two pools. The harness forces four vCPUs even when an invocation requests fewer. Diagnostic reruns with the exact 2- and 1-vCPU topologies reached the gate’s two-pool precondition. The existing policy allows one pool at those CPU counts, so those diagnostic runs are outside the multi-pool gate’s supported scope. The earlier SWAP-1 serials labeled 1, 2, and 4 vCPUs all report four vCPUs. Candidate checks ran in the existing isolated source tree. Earlier transaction/session and mmap feature build evidence remains part of the closure record.

The changes are scoped to debug instrumentation and fixture control. Both ordinary spawn entry points continue using the existing service contracts and ownership rules. ZRAM remains independent of VFS. The scheduler uses its existing allocation and reuse policy.

Parent loss remains unqualified. The existing adapter fixture authenticates PID 1 as its parent and these scenarios keep it alive. The orphan path still needs a native transaction census, including cleanup progress without a later qualifying syscall return. VFS-L1C-3 also retains the trusted cross-lifetime registration row reuse prerequisite and the transaction census with regular-file descriptions, child-owned IPC endpoints, queued payloads, populated swap, and fault-driven swap-in. SWAP-1 independently covers its ZRAM-backed workload. Interactive Btop and Qalculate remain prerequisites. VFS-D1 stays deferred.
