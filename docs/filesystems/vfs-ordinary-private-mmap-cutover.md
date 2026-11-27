# VFS-C1E7 ordinary private mmap

**PARTIAL.** Ordinary native and Linux mmap dispatch can populate private
pages from service-owned file descriptions in the opt-in candidate. Linux
descriptors still owned by the kernel use the historical eager reader. The
unchanged ownership guard remains red; production cutover is blocked.

The syscall adapter selects the D2A/D2B continuation using the descriptor
owner or an exact caller's pending transaction, under scheduler
serialization. Neither a caller name nor a native-only fixture identity is
needed. Linux flag translation leaves the original registers available for
replay and returns Linux errno on failure. Legacy handles retain their
existing route. A service-owned handle cannot enter the kernel VFS reader.

The continuation checks READ and the live epoch, pins the description,
stages at most 1 MiB via bounded positional IPC without moving the shared
cursor, and releases scheduler and broker locks before parking. The installer
rechecks caller, epoch and placement before committing eager private frames
with final protections. Rollback, owner retirement, and final close use the
existing mechanisms. The single executor, zero fill past EOF, protection
rules, and existing ZRAM and main-stack policies remain in force.

## Fresh qualification

- **PASS**, Linux syscall 9, one vCPU and 2 GiB with a disposable, unpublished
  FAT preparation. Four two-page maps copied 4,105 bytes each; a fifth map
  copied nine bytes at offset 4,096. Ring 3 checked EOF padding, private writes,
  unchanged backing bytes and cursor, protection transitions, unmap and access
  after source close. The test passed an unaligned offset through the ordinary
  Linux errno adapter. PID 39002 reaped 22,300 user frames and 50 page tables,
  with zero swap blocks or queued IPC entries. StorageCtl progressed after
  reap; the closed FAT partition was byte-identical and passed read-only fsck.
- **PASS**, diskless native syscall 50 at one, two and four vCPUs, 2 GiB.
  D1B/D2A/D2B checks covered descriptor saturation, exec, final close,
  permissions, six rollback cases, parked caller retirement and late reply,
  normal replay, and endpoint withdrawal followed by anonymous mapping.
  Parked samples held 538 frames, four regions and one description with two
  references. Terminal samples released the pin; settlement had zero owned
  frames, service rows and references. The service reaped 22,264 frames and
  53 tables before init completed. On four vCPUs, the service's post-destroy
  receipt logged after the caller observed withdrawal; both precede service
  exit. The local evidence verifier allows either cross-CPU log order.
- **PASS**, authenticated ordinary namespace publication on one vCPU, including
  both launch adapters and service-backed native mmap.
- **PASS**, C1E6B exercised registration and the descriptor retirement
  regression. Candidate disassembly shows inline array initialization in the
  registration frame with no separate out-of-line constructor frame. The
  registration frame still contains construction storage; this is not a
  claim of stack-free initialization. The guest emitted no stale-alias
  fallback. Stack capacity is unchanged. Production release ELF is stripped;
  production constructor execution was **NOT_RUN**.

The three requested builds passed: production release kernel, release VFS
service and opt-in candidate. The focused registration harness passed 112
tests, positional reply assembly passed three, and the FD harness passed 20
when run serially. Its initial parallel run failed in the synthetic stale
alias test; the run does not establish the cause. N1P populated and unknown
FSInfo closed images passed fresh read-only verification; fresh N1P guest
mutation cycles were **NOT_RUN**. The C1E2–C1E6 chain ran in the Linux guest;
standalone C1E1 was **NOT_RUN**.

Prior validation limits remain separate: direct kernel host tests were blocked
by private `ProcessState` imports and duplicate language items; workspace
library tests failed on the duplicate `panic_impl` in the Wayland crate; the
prior timed `secret_store` run did not finish. These were not rerun here.

## Ownership disposition

| Historical finding | Disposition |
| --- | --- |
| Private mmap | **PARTIAL** integration; historical finding open. Linux kernel-owned descriptors still use `KERNEL_VFS` for FSTAT and positional reads. |
| ProcFS mountinfo | **OPEN**. Non-migrated callers still snapshot kernel topology. |
| Linux statfs | **OPEN**. The historical capacity route still samples kernel FAT state. |
| Runtime FAT | **OPEN**. The kernel still constructs and publishes runtime FAT. Service FAT publication remains disabled. |

The unchanged no-growth guard **FAILS** with 71 references and the four
historical unmatched sites. Ordinary production boot is **BLOCKED / NOT_RUN**.
Exec while mmap is parked is **BLOCKED** by the single executor contract.
Linux parked withdrawal and caller retirement, Linux SMP, and D3 reclaim and
fault restoration were **NOT_RUN**. The native lifecycle evidence does not
qualify those Linux cases or complete Linux mmap compatibility.

See the [results index](vfs-ordinary-private-mmap-cutover-results.json) for
serial logs and gate status. For the Linux probe, build the service with
`vfs-c1e7-test`, embed it via `SUNLIGHT_VFS_N1J_ELF` in a debug kernel
with `vfs_c1e7_test`, and run `tools/test_vfs_n1h.py --c1e7 --kernel <elf>`.
Use the existing diskless `vfs_d2b_test` and `vfs_c1a_test` fixtures for
native lifecycle and authenticated launch, respectively.
