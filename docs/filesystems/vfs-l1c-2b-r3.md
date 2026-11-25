# VFS-L1C-2B-R3 — Service retirement after process publication

**Native R3 passed on 2, 1, and 4 vCPUs**, in that order, in fresh isolated QEMU TCG guests with 2 GiB RAM. The already-published child remained scheduler-owned when the boot VFS service completed its lifetime. VFS-L1C-2B remains partial. VFS-L1C-3 stays blocked pending native R5 and remaining resource qualification.

## Native transaction

Init used the pending-child adapter and normal copied namespace retain. While init was parked, an independent `/bin/true` ran and exited successfully. Init consumed the authenticated `Retained` result and called the existing publication function. It installed the receipt's root/cwd binding, removed the creation continuation, transferred the private payload and PID to the scheduler, inserted the child, and enqueued it once.

The scheduler selected the child, which ran the native `stat /tmp` applet and received directory metadata. The child then entered a fixture syscall checkpoint, yielding in userspace until init released it. Init verified that the exact receipt was bound and the child was live in the scheduler, then called `terminate_process_by_pid` on the existing boot VFS service. Withdrawal drained the child's binding and left its release obligation with the retired registration. The production reaper delivered matching KLC image-exit proof after VM reclamation and endpoint cleanup. The proof collected the retired registration row, settling the release with the service's private backing.

Before releasing the checkpoint, init verified that the child's exact process identity and wait ownership survived, that the private payload, PID reservation and creation continuation were absent, and that repeated wait-for-publication and publication both returned `Error::Identity`. In userspace, fresh `stat /tmp` and `chdir /tmp` both returned `Errno::Again` (EAGAIN) under the sticky service route. `getcwd` still returned the same process-local path. The child then exited with status zero. Init consumed that status once, waited for reaper completion, verified local resource disposition, and disposed of the adapter. The independent observer's exit status remained available. [VFS namespace routing contract](userspace-vfs-migration.md#copied-retained-object-spawnexec-caller-provisioning-2026-11-17).

All guests used child PID 5, incarnation 683 and VM generation 1351, service PID 3, incarnation 50 and VM generation 25, registration epoch 1, and retain request 2. The table gives serial line numbers. The JSON records the complete checked sequence, including the binding install, checkpoint, withdrawal and both waits.

| vCPUs | Retained | Committed | Enqueue | First dispatch | Stat success | Terminate | KLC proof | Scheduler ownership rechecked | Namespace results | Child exit | Child reaped | Adapter disposed |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1898 | 1901 | 1902 | 1903 | 1931 | 1933 | 1939 | 1943 | 1945 | 1949 | 1952 | 1956 |
| 1 | 1897 | 1900 | 1901 | 1902 | 1927 | 1929 | 1935 | 1939 | 1941 | 1945 | 1948 | 1952 |
| 4 | 1915 | 1918 | 1919 | 1920 | 1948 | 1950 | 1956 | 1960 | 1962 | 1966 | 1969 | 1973 |

## Resource disposition

| Resource | Observed disposition |
| --- | --- |
| Creation continuation, payload and PID reservation | Publication consumed the continuation and transferred the payload to the scheduler. The private PID reservation was absent before and after service retirement. Init consumed the child’s wait record once before the reaper completed. |
| Child root/cwd pair | The installed receipt represented one pair. Withdrawal drained its binding and queued the matching release obligation in the retired registration. Matching KLC proof settled that row through private service-image reclamation. |
| Child descriptors and VM | Three descriptor slots remained occupied through retirement. After normal exit all descriptor slots were empty, the user VM root and kernel stack were gone, and the PMM reported zero frames owned by the child PID. The reaper reported 550 user frames, eight page tables and zero populated swap blocks. |
| Service VM and endpoint | Matching KLC proof followed endpoint cleanup and private root reclamation. The service reaper reported 22,264 user frames, 53 page tables and zero populated swap blocks. |
| Adapter | The fixture held the handle through normal child reaping. Identity lookup and cancellation returned `Stale`. It dropped the handle once, after verifying the scheduler reaper had already reclaimed the child’s execution resources. |

The first two-vCPU pilot reused a cached utility ELF that lacked the R3 applet. The corrected fixture builds and embeds its own utility image, keeping the regular utility artifact independent. The final witness required no production repair. Its kernel observations and entry points are gated on debug assertions and the R3 test feature.

## Qualification and limits

All 105 registration tests passed, including the 112-cycle ownership regression. Fresh R1, R1B, R2, and R4 guests passed on 2, 1, and 4 vCPUs. R4 completed eight delayed-reply transactions per guest. The transaction/session and mmap compile checks passed. The candidate ownership guard reported zero new references. SWAP-1 passed on four vCPUs and ordinary Phase 3.0 boot passed on two vCPUs, both with 2 GiB RAM.

The witness exercises metadata and directory routing, with a process-local `getcwd` control. Operation-wide file routing is outside this fixture. The reaped scheduler record and its empty descriptor table remain available for slot reuse. A full transaction resource census, retirement with populated swap in this transaction, fault-driven swap-in, and interactive Btop and Qalculate checks remain open. Runtime service replacement remains outside the witness. VFS-D1 stays deferred.

The [machine-readable results](vfs-l1c-2b-r3-results.json) index the raw serial logs, checker output, and image hashes under `target/vfs-l1c-2b-r3/`.
