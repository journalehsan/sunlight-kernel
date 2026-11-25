# VFS-L1C-2B-R1B — VFS retirement before directory acquisition

**Native R1B passed on 2, 1, and 4 vCPUs**, in that order. Each run started a fresh QEMU TCG guest with 2 GiB RAM in the existing isolated candidate. Together with the [post-acquisition witness](vfs-l1c-2b-r1.md), this covers both R1 orderings. VFS-L1C-2B remains partial and VFS-L1C-3 remains blocked.

Init used the existing publication adapter to construct a private child, stage its normal namespace retain, and park. The debug fixture held only that request at the transport's single-send boundary. Once the independent `/bin/true` observer had dispatched while init was parked and exited with status zero, the scheduler poll matched the child, retain ID, and registration epoch. It verified that the continuation was still parked, the retain frame was still in `unsent`, the request had no delivery record, and its kernel preparation entry and caller reservation were still present. No copied reply was held.

The transport reserves delivery and removes the frame from `unsent` before copying it to the service. The derivation authorization also requires the exact delivery record. The gate stayed closed until cancellation, so the boot VFS service could never receive the request or derive the child's directory references. The preparation entry pinned the parent's existing binding locally. The child acquired zero service-side directory references.

The fixture released the VFS runtime lock and called `terminate_process_by_pid`. The production withdrawal path cancelled the unsent retain. The normal reaper reclaimed the service image and revoked its endpoints, then delivered the matching KLC proof. The exact retired registration row was gone when the fixture observed the proof. Init then consumed one `AuthorityRetired` terminal, reclaimed the private child, and disposed of the adapter once. The subsequent publication attempt and second cancellation were rejected. The checker found zero child binding installs, process insertions, enqueues, and first dispatches. Init collected the independent observer's zero status after local cancellation.

The service was PID 3, incarnation 50, VM generation 25, epoch 1 in all three guests. The child was PID 5, retain request 2, and the observer was PID 6. Numbers below are serial line numbers.

| vCPUs | Parked | Observer exit | Unsent checkpoint | Termination | KLC proof and row collected | Terminal consumed | Child teardown | Adapter disposed | PID settled | Observer wait |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1891 | 1896 | 1901 | 1902 | 1907 | 1911 | 1912 | 1915 | 1916 | 1917 |
| 1 | 1887 | 1892 | 1894 | 1895 | 1900 | 1904 | 1905 | 1908 | 1909 | 1910 |
| 4 | 1905 | 1910 | 1912 | 1913 | 1918 | 1922 | 1923 | 1929 | 1930 | 1931 |

| Resource | Disposition |
| --- | --- |
| Private process and PID | The pending-child table owned the private payload and reserved identity. Adapter cancellation removed the payload. The normal settlement poll removed the PID claim after local reclamation and registration cleanup. |
| Adapter and continuation | Init owned one adapter and the runtime owned one parked continuation. Init consumed one terminal after KLC proof delivery. The continuation was removed and the adapter was disposed of once. |
| Request, caller reservation and source binding pin | Production unsent cancellation discarded the request from the ownership ledger, dropped its preparation entry and parent-binding pin, and cleared the reserved caller slot. The matching image proof allowed collection of the retired registration row. |
| Directory references | The service never received the child retain. No child directory lease, acknowledgement, release obligation, or installed binding arose. |
| Local descriptors and heap | Rollback released three initial stream descriptions. The fixture inherited zero stream references. The process and descriptor-table boxes were dropped. |
| VM resources | Child rollback reclaimed 551 user frames and eight page tables. Service retirement reclaimed 22,264 user frames and 53 page tables. KLC also required reclamation of the service root. Both populated swap counts were zero. |

The fixture uses production staging, withdrawal, reaping, and cancellation. R1B reproduced no production defect. The post-acquisition R1 preparation-reference cleanup remains in place. The R1B checkpoint is enabled only in the debug fixture build. Both ordinary spawn entry points continue to use their existing paths.

All 105 registration host tests passed, including the R1 preparation-reference test and the 112-cycle ownership regression. The rebuilt post-acquisition R1 and R4 fixtures passed on 2, 1, and 4 vCPUs. R4 completed eight delayed-reply transactions per guest. Transaction/session and mmap compile checks passed. The candidate ownership guard reported zero new references. The unchanged SWAP-1 gate passed on four vCPUs, and ordinary Phase 3.0 boot passed on two vCPUs. The [machine-readable results](vfs-l1c-2b-r1b-results.json) index the serial logs, line checks, and image hashes under `target/vfs-l1c-2b-r1b/`.

R2, R3, native R5, and the full transaction resource census remain open. The runs established ordered lifecycle checkpoints on UP and SMP. The resource counts cover the identified heap owners and unpopulated swap teardown. Populated swap teardown, fault-driven swap-in, and interactive Btop and Qalculate checks remain unrun. Runtime service replacement is outside the scope of this witness. VFS-D1 remains deferred.
