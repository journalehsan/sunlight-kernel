# VFS-C1E3: authorized attachment publication

**PARTIAL implementation; coordinated publication BLOCKED.** The isolated
C1E2 build now accepts a cookie-only StorageCtl publication request. A valid
preparation receives EOPNOTSUPP and remains available for retry or abort.
There is no ownership transfer or namespace insertion in this candidate.

Publication uses the existing copied file continuation. Syscall 168 requires
the live caller's StorageAdmin grant. The service matches the exact caller
and cookie against the one retained preparation, then checks the reservation
against the same namespace owner, topology revision and target object.
The kernel correlates the response with the pending request and rechecks
the caller incarnation, address-space generation, authorization and service
epoch. It accepts the retryable refusal only for a received, ready and
uncancelled receipt with the current registered extent, media, active lease
and idle command and mapping state. The codec rejects a success reply while
commit is disabled. Wrong cookies cannot change the preparation. Existing
abort, caller retirement, service retirement and lease settlement remain
responsible for cleanup; the refusal creates no additional cleanup obligation.

The source-level blocker is the split between
`directory/attachment/native.rs::run_retained`, which owns the retained
`DirectoryNamespace<Device>`, and the ordinary `State` initialized in
`services/vfs_server/src/main.rs`, which owns a separate default-backend
namespace. The retained loop does not dispatch ordinary file or retention
requests. Calling the existing `Vfs::publish_prepared_device` there would
install into a graph ordinary callers cannot use. The native fixture uses
that primitive elsewhere, but its mount metadata still uses source zero.
The production publication path must carry the registered source and
protection into the common graph, keep the backend and lease there, and
consume the receipt once in the same commit. It must also serialize
successful commit with cancellation, retirement and withdrawal.

| Ownership finding | Disposition |
| --- | --- |
| Runtime FAT construction | **BLOCKED**: `kernel/src/storage.rs` still constructs the live backend. The ordinary storage inventory and unmount still consult the kernel mount table. |
| ProcFS topology | **BLOCKED**: `linux_procfs::mountinfo_snapshot` still reads kernel topology. The service snapshot has no ordinary ProcFS install path. |
| Linux statfs | **BLOCKED**: `sys_linux_statfs` still reads kernel capacity. The service capacity lookup has no ordinary Linux continuation. |
| Private mmap | **BLOCKED**: `process::mmap::sys_mmap` still reads the kernel backend. The retained service mmap path is limited to its native fixture admission. |

The unchanged guard fails with 71 references and four unmatched sites.
Guarded ordinary boot was not run. No production defaults, allocator
registration, memory policies or stack defaults changed.

The [results](vfs-storagectl-coordinated-publication-results.json) separate
fresh native refusal evidence and host checks from earlier component
qualification. Ordinary publication, authenticated file access to a
published mount, retained rename identity, private mmap, inventory
agreement, ordinary withdrawal and write persistence remain unqualified.
