# VFS-L1C-2B-R2 — Service retirement after successful registration

**Native R2 passed on 2, 1, and 4 vCPUs**, in that order. Each run used a fresh QEMU TCG guest with 2 GiB RAM and the candidate’s ordinary boot-time VFS service. VFS-L1C-2B remains partial. VFS-L1C-3 remains blocked pending R3, native R5, and remaining resource qualification.

## Native transaction

Init constructed a private child with the existing pending-child adapter, sent the normal namespace retain, and parked. An independent `/bin/true` dispatched while init was parked and exited with status zero. The boot VFS service derived the child’s directory pair and returned its authenticated receipt over the normal copied channel. Init consumed `Retained` through `wait_for_publication`, which leaves the receipt in the transaction for the later publication decision.

The checkpoint verified that the service registration and private child were live, the continuation was unparked, the exact receipt was held, the delivery and preparation were cleared, the caller slot remained reserved, and the child had no installed namespace binding. With the scheduler guard still held, the fixture invoked `terminate_process_by_pid`. Production withdrawal changed the terminal result to `AuthorityRetired` and left a release obligation for the child receipt in the retired registration row. The normal process reaper produced the matching KLC proof after reclaiming the service’s private image and completing endpoint cleanup. Proof delivery collected the retired row and settled the release obligation through service-image retirement.

On the next fixture entry after proof, the existing publication transaction returned `Error::Identity` while the child was still in the private table. Init consumed `AuthorityRetired` once, and the fixture verified that the continuation was absent on a second wait. The adapter cancelled the child, rejected repeated cancellation, and was disposed of once. Normal polling removed the PID claim after local reclamation and registration settlement. Init received the fixture creation-failure result and collected the observer’s zero exit status. No guest recorded a child binding install, process insertion, enqueue, or first dispatch.

All three guests used service PID 3, incarnation 50, VM generation 25, registration epoch 1, child PID 5, retain request 2, and observer PID 6. The table gives serial line numbers. The machine-readable results include staging, parking, the observer exit, the reaper summary, terminal consumption, and the heap drop.

| vCPUs | Authenticated result | Result consumed | Termination | Withdrawal | KLC proof and row collection | Publication rejected | Child teardown | Adapter disposed | PID settled | Observer wait |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1898 | 1902 | 1903 | 1904 | 1909 | 1913 | 1915 | 1917 | 1919 | 1918 |
| 1 | 1894 | 1895 | 1896 | 1897 | 1902 | 1906 | 1908 | 1910 | 1911 | 1912 |
| 4 | 1912 | 1913 | 1914 | 1915 | 1920 | 1924 | 1926 | 1928 | 1929 | 1930 |

## Resource disposition

| Resource | Observed disposition |
| --- | --- |
| Registration preparation and caller slot | Receipt processing cleared the delivery and preparation. Production withdrawal retired the caller slot, and matching KLC proof collected its registration row. |
| Directory pair | The authenticated receipt represented one retained root/cwd pair. Withdrawal queued one unbound receipt for release. KLC settled that obligation through reclamation of the service’s private backing. |
| Private process, PID and adapter | Local cancellation removed the private payload and dropped the process and descriptor-table boxes. The normal settlement poll removed the PID reservation. The fixture recorded one adapter disposal and confirmed repeated cancellation failed. |
| Child descriptors and VM | Rollback released three initial stream descriptions, reclaimed 551 user frames and eight page tables, and reported zero populated swap blocks. |
| Service VM | The reaper reported 22,264 user frames, 53 page tables and zero populated swap blocks. The KLC proof also required service address-space root reclamation. |

R2 is confined to debug-only fixture code and calls the existing lifecycle and publication functions. No production defect was reproduced. The first two-vCPU pilot placed its release assertion after the termination call. The service reaped synchronously, so the check ran after the row had already been collected. The final fixture checks the release in the withdrawal path itself and preserves the pilot trace.

## Qualification and limits

All 105 registration tests passed, including the 112-cycle ownership regression. Fresh R1, R1B, and R4 guests passed on 2, 1, and 4 vCPUs. R4 completed eight delayed-reply transactions per guest. The transaction/session and mmap compile checks passed, and the candidate ownership guard reported zero new references. SWAP-1 passed on four vCPUs, and the ordinary Phase 3.0 boot gate passed on two vCPUs, both with 2 GiB RAM.

A full transaction resource census, teardown with populated swap in this transaction, fault-driven swap-in, and interactive Btop and Qalculate checks remain open. The observer executed while init was parked and its exit status remained available after service retirement and adapter disposal. Runtime service replacement remains outside this witness. VFS-D1 stays deferred.

The [machine-readable results](vfs-l1c-2b-r2-results.json) index the raw serial logs, checker output, and image hashes under `target/vfs-l1c-2b-r2/`.
