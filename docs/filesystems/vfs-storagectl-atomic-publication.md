# VFS-C1E10: atomic StorageCtl publication

**PARTIAL.** Ordinary StorageCtl committed a service-owned FAT32 mount on
the disposable, single-vCPU, 2 GiB fixture. The readback run committed two
successive incarnations, at most one at a time, and withdrew both. This
does not complete the write or cross-ABI qualification matrix.

The candidate uses `vfs_c1e10_test` and `vfs-c1e10-test`, which include the
existing C1E7 service routes and C1E9 mutations. The runtime storage
dispatcher is the sole puller for the copied file channel in this candidate.
Previously the retention poller could pull a storage frame first and fail
to answer it; that is unsafe for a request dispatched once.

The service matches the exact caller and cookie, checks lease cancellation,
and locks the existing N1D reservation to revalidate the owner, topology
revision, and target object. N1D takes the same FAT instance only after
fallible insertion checks, with the registered source and ordinary
unprotected mount metadata. A failed insertion keeps the original preparation.
The service loop does not dispatch another request between insertion and
the authenticated reply, or between a rejected reply and compensation.

The kernel orders reply acceptance under its existing scheduler and runtime
locks. It checks dispatch, exact request and caller, active registration,
unrebased view, credentials and StorageAdmin. The device binding rechecks
the exact received receipt, media and device extent, active exclusive
lease generation, and zero outstanding commands, mapping users, retained
command data and pending flush. Success clears the receipt and installs the
reply in the existing buffer without allocation. The committed backend
retains the lease. A consumed cookie cannot abort the mount. A rejected
success reply triggers existing withdrawal and settlement; a failed
compensation fences the lease where possible and retires the runtime owner.
No VFS lock spans the publication reply syscall. The native suite did not
inject loss between insertion and acknowledgement.

The readback sequence verified one service row in both StorageCtl list
interfaces, source and incarnation, committed stat and chdir, relative
open, seek and shared cursor through dup, repeated publish and abort
rejection, withdrawal, and reattachment with the original seed payload.
New-write persistence remains unqualified. A broader attempt failed at
mkdir; ordinary service mutation inherits the existing immutable-root
policy for the fixture path, and copied open still rejects creation flags.
Native FAT private mmap also failed in a separate attempt. Committed
Linux mountinfo/statfs agreement, retained handles through FAT rename and
pinned FAT unlink or replacement remain unqualified. Relative explicit
dirfd operations remain unsupported.

The unchanged ownership guard **FAILS** with 71 references and four findings:

| Finding | Remaining legacy ownership |
| --- | --- |
| `storage::mount` | Constructs FAT for non-fixture registry devices. |
| `linux_procfs::mountinfo_snapshot` | Reads kernel topology for legacy views. |
| `sys_linux_statfs` | Reads kernel capacity for legacy paths. |
| `process::mmap::sys_mmap` | Reads through kernel-owned descriptions. |

Guarded ordinary boot was **NOT_RUN**. The separate disposable probe uses
the existing isolated harness. See the [results](vfs-storagectl-atomic-publication-results.json)
for the evidence boundaries and prior failed attempts.
