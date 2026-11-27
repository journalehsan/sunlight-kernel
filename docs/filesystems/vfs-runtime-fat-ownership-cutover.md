# VFS-C1E8: runtime FAT ownership

**PARTIAL implementation. BLOCKED ordinary publication.** No StorageCtl
request committed a service FAT mount. The authorized publication gate
still returns EOPNOTSUPP for a valid preparation.

The opt-in kernel now fences legacy runtime FAT construction by physical
parent ID for the disposable disk. The fence includes partition aliases
and remains in place between leases; the fixture already transfers the
whole device queue to the authenticated lease transport before the boot
FAT path. Other disks retain their existing routing. Pulling a copied
publication request now uses the existing topology invalidation for
mountinfo and statfs. The same scheduler checks on dispatch, withdrawal
of a pending request, and reply reject rebased callers and recheck
credentials and StorageAdmin for publication.

N1D now has an insertion path that takes the existing backend only after
the path, capacity, slot allocation, and topology checks succeed. The
result uses the installed mount's tag, and the mount keeps the caller's
source and protection metadata and the backend's existing object IDs.
The focused host case refuses an occupied target, retries with the same
prepared backend, checks metadata and case-insensitive object identity,
then exercises retained roots and handles through rename and remount.

The missing commit is at
`services/vfs_server/src/directory/attachment/native.rs::RetainedStorage::dispatch`
and `kernel/src/ipc/vfs_registration/file.rs::reply_file`. The kernel
can validate a refusal against the received preparation but does not
consume the receipt on successful commit; the copied reply codec still
rejects success. The readiness check at storage control 0x109 also ends
before namespace insertion. Simply calling N1D before replying would
leave a visible attachment when the kernel rejects a stale reply.
A future commit must order the exact reservation and backend transfer
against caller cancellation, service retirement, and the active lease,
and compensate an unacknowledged insertion before dispatching any further
namespace requests. The existing lease and withdrawal lifecycle must
continue owning settlement.

There is also a concrete routing gap. Native `sys_mkdir`, `sys_unlink`
and `sys_rename` in `kernel/src/arch/x86_64/syscall.rs` use the kernel
namespace. Service IPC mutation helpers in `services/vfs_server/src/main.rs`
use the runtime owner, but they are a separate entry path. Route ordinary
mutations for selected callers through the authenticated retained view,
preserving credentials and rename-no-replace semantics. The existing
`mkdir_beneath` is available; unlink and rename still need anchored
operations. Ordinary service descriptors and mmap, inventory/withdrawal,
and routed Linux mountinfo/statfs are present from C1E5–C1E7 but were
not demonstrated on a committed FAT mount.

| Unchanged ownership guard finding | Legacy authority and remaining transition |
| --- | --- |
| Runtime FAT construction | `storage::mount` constructs FAT for non-fixture registry devices. Move each admitted extent to exclusive service attachment before removing legacy construction. |
| ProcFS kernel topology | `linux_procfs::mountinfo_snapshot` reads kernel-owned mounts for legacy callers. Move their caller views and mounts to the service topology. |
| Linux statfs kernel backend access | `sys_linux_statfs` reads kernel FAT capacity for legacy callers. Migrate those path views and read capacity from the resolved service incarnation. |
| Private mmap kernel FSTAT and positional reads | `process::mmap::sys_mmap` still populates mappings for kernel-owned descriptors. Move those descriptions to the existing service mmap continuation. |

All three requested builds passed, as did the embedded fixture service
build, 12 focused storage tests, and 112 host registration tests.
The fresh isolated single-vCPU, 2 GiB C1E7 regression exercised ordinary
StorageCtl preparation, refusal and abort. Five releases and exact service
image retirement kept queue and bounce fixed at 20 KiB. The settlement
assertions checked zero commands, mapping users and retained command data
and no pending flush. PID 39002 reaped 22,300 user frames and 50 page
tables with zero swap or IPC entries. Read-only fsck and exact
`KEEP.TXT` recovery passed; the closed FAT image matched the input.

The unchanged no-growth guard **FAILS** with 71 references and all four
unmatched sites. Guarded ordinary boot was **NOT_RUN**. Committed lookup,
inventory and mountinfo agreement, capacity, file operations, both mmap
ABIs, retained-reference withdrawal, committed settlement and reattach,
and closed-image payload acceptance were **NOT_RUN**. Prior preparation
release and 20 KiB DMA evidence cannot qualify a committed withdrawal.
See the [results](vfs-runtime-fat-ownership-cutover-results.json) for
fresh checks and prior evidence kept separately.
