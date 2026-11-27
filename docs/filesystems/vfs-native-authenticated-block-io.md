# VFS-N1M: Native authenticated block I/O

**PARTIAL (November 27, 2026).** Native block qualification passed on one
disposable QEMU VirtIO disk. Integrated native FAT32 attachment is blocked
at the N1L transaction boundary.

The isolated kernel feature `vfs-n1m-test` adds six commands to copied VFS
syscall 180. The ring-3 VFS image uses the existing nameserver registration.
The kernel binds its endpoint, PID, process incarnation, VM generation and
registration epoch from that registration, and compares the current caller
image to that binding. The caller supplies no owner, session, or device
selector. The existing authority binds the lease to the registered media
generation and checks each operation's rights and extent.

| Opcode | Request | Successful return |
| --- | --- | --- |
| 0x100 | Acquire the configured exclusive whole-device lease | Lease generation |
| 0x101 | Read one 512-byte sector at an absolute LBA | Zero after confirmed completion and copy-out |
| 0x102 | Write one 512-byte sector at an absolute LBA | Zero after confirmed completion |
| 0x103 | Flush | Zero after confirmed virtual barrier |
| 0x104 | Drain, flush and release | Zero after release |
| 0x105 | Submit the final retirement-test write at LBA 191 | Accepted request generation; settlement belongs to the kernel |

The opcode, user buffer pointer, LBA and lease generation occupy RDI, RSI,
RDX and R8. Unused fields are zero. Rejections return all ones; transport
uncertainty follows the existing probe's fail-stop fencing path. The whole
grant is needed for device-wide flush; the test interface further restricts
data access to LBAs 128–191. User copies use the existing address-space
checks, and DMA stays in the retained kernel queue and bounce allocation.
The synchronous call owns its exact lease ticket until the typed VirtIO
receipt is verified and consumed. Transport polling runs without scheduler,
IPC, topology or lease state guards, under the isolated single-CPU
coordinator's queue serialization.

The service observed successful readback and release across 64 cycles; each
cycle also checked stale generation and out-of-window rejection. The actual
VFS registration was endpoint 1, PID 3, incarnation 43, VM generation 18,
epoch 1. The service exited after accepting its last write. Registration
withdrawal reduced caller credit to zero and retained one command. After
exact image retirement the coordinator consumed the matching transport
receipt, drained the queue, confirmed a covering barrier and released the
lease. The run reported 129 data commands and 129 barriers, zero pending
commands and zero DMA mapping users. The harness compared all 4 MiB of the
backing image. The four queue pages and one bounce page (20,480 bytes) were
allocated once and remained owned at verified retained reset; physical DMA
reclamation is **BLOCKED** and was **NOT_RUN**.

The N1L attachment module compiles only on the host. Its device adapter
owns the authority locally, its publication pin calls that authority inside
the namespace transaction, and its withdrawal path performs writeback and
barriers with the namespace locked. The native block window also excludes
the FAT boot sector. The smallest next integration is to use a marked
disposable FAT medium with an explicit geometry and sector grant, and adapt
the existing N1D reservation to a copied native block device. Keep probe and
writeback outside the topology guard. Carry the exact live lease through
readiness and publication, then split withdrawal into close admission and
check pins, sync and settle outside the guard, and revalidate the same mount
incarnation before final removal and lease release. A direct swap of the
host adapter would not preserve those contracts. Native FAT readiness,
file operations, retained handles, withdrawal and repeated mount integrity
were **NOT_RUN**. Native snapshot and FD installation remain **BLOCKED**.

All 481 baseline, 59 N1H, seven N1D and three N1L test names passed. The
unchanged ownership guard **FAILS** at 71 references with the same four
unmatched sites: statfs, ProcFS mountinfo, private mmap and runtime FAT
mounting. Ordinary boot storage, runtime FAT and production routing were
untouched. L1C-3 stays blocked, VFS-D1 deferred and demand-backed main stacks
opt-in.

See [machine-readable results](vfs-native-authenticated-block-io-results.json).
