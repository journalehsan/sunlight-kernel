# VFS-L1C-2B-R1: Service retirement during pending creation

**Native post-acquisition R1: PASS on 1, 2 and 4 vCPUs.** Each run booted a fresh 2 GiB QEMU TCG guest using the same R1 image. VFS-L1C-2B remains partial and VFS-L1C-3 remains blocked.

Init staged a private child through the existing publication adapter and parked. The boot VFS service acquired the requested directory lease through normal namespace registration. Its real retained receipt passed the kernel's copied reply authentication and was held before completion. The next scheduler poll verified the continuation was still parked and the registration still owned the delivered retain. With the runtime lock released, it called the existing scheduler termination path. The existing reaper withdrew service authority, reclaimed the private service image and revoked its owned endpoints, then delivered the exact KLC exit proof. The fixture gated the caller wake on observing that proof; it did not construct a proof or replay the held reply.

The service was PID 3, incarnation 50, VM generation 25, registration epoch 1 in all three guests. The child was PID 5 and its retain was request 2. The caller consumed one `AuthorityRetired` result and discarded the held copy. The following table gives the corresponding serial line numbers.

| vCPUs | Parked | Receipt held | Termination | KLC proof / row collected | Terminal consumed | Local image teardown | Adapter disposed | PID settled | Observer wait |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 1890 | 1897 | 1898 | 1903 | 1907 | 1908 | 1911 | 1912 | 1913 |
| 2 | 1894 | 1901 | 1902 | 1907 | 1911 | 1912 | 1915 | 1920 | 1916 |
| 4 | 1908 | 1915 | 1916 | 1921 | 1925 | 1926 | 1929 | 1934 | 1930 |

The checker found no child binding, process table insertion, enqueue or first dispatch. The adapter went through disposal once, and an attempted publication after cancellation failed. An independent `/bin/true` observer ran while init was parked, exited with status zero, and init collected that status after local cancellation. This verifies progress across the checkpoint; ordinary boot is a separate check using an image without the destructive fixture.

| Resource | Ownership and disposition |
| --- | --- |
| Private reservation and process identity | Pending child table owned the reserved PID and private payload. Cancellation removed the payload. The claim retained the PID until the normal pending-child poll observed remote settlement and removed the slot. |
| Adapter and continuation | Init's fixture owned one adapter record and the runtime owned one parked continuation. The retirement result was consumed once; the continuation and held reply copy were removed, then the adapter completed cancellation once. |
| Registration request and directory reference | The exact registration owned the delivered retain and caller barrier; the service's private namespace held the directory lease. Withdrawal ended admission. Last-owner image reclamation destroyed the private backing; the matching proof allowed collection of the retired row, including the outstanding request and caller bookkeeping. The child never received an installed binding. |
| Source binding | The preparing request owned a strong reference to the parent's binding. The fix drops obsolete preparation entries on exact image completion, allowing the otherwise drained source binding and row to be collected. A real local binding user may still pin the retired row until it drops its reference and collection runs. |
| Descriptors and heap | Rollback removed three initial stream descriptions and their references; no stream was inherited. The process and descriptor table boxes and their owned fields were dropped. This identifies heap owners rather than counting all allocations or bytes. |
| Physical frames and address space | Private child rollback reclaimed 551 user frames and eight page tables per guest, with zero populated swap blocks. The service reaper reported 22,264 user frames and 53 page tables reclaimed with zero populated swap blocks; the proof also requires the root to be reclaimed. Shared resources with other owners remain governed by their existing lifetimes. |

The first 2-vCPU run reached the KLC proof and reclaimed the child locally, but the retired registration and PID claim remained charged. The delivered derivation held the source binding through a preparation entry in the same row that required that binding to drain. The exact-image completion path now drops those obsolete entries before testing for final binding drain. Withdrawal without matching completion keeps its existing ownership rules; real local users still defer collection. The focused host regression holds an independent binding throughout proof delivery, drops it, then checks row collection, single terminal consumption and PID release.

The R4 checkpoint and accounting are reused only in debug fixture builds; both ordinary spawn entry points and the boot VFS service are unchanged. The original R4 report and evidence are preserved. The rebuilt R4 fixture passed all 24 transactions across 1, 2, and 4 vCPUs. The 104 existing registration tests, the new focused test, the 112-cycle ownership check, transaction/session and mmap compile checks, four-vCPU SWAP-1, and two-vCPU ordinary Phase 3.0 boot passed. The candidate ownership guard reported zero new references. The [machine results](vfs-l1c-2b-r1-results.json) index the regression logs and the original failing pilot under `target/vfs-l1c-2b-r1/`.

This witness covers retirement after acquisition and before acknowledgement. The separate retirement-before-acquisition case listed in the phase plan remains unrun, as do R2, R3 and native R5. No service replacement, populated swap teardown, fault-driven swap-in, or new interactive Btop or Qalculate checks were run. VFS-D1 remains deferred.
