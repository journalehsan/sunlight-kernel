# Helios VFS authority audit

`./tools/test.sh` reproduces all four findings from the unmodified no-growth
guard before building or starting QEMU. The evidence is
`target/helios-qalculate/vfs-authority/ordinary-boot.log`. The guard is a
source comparison: a new backend reference does not by itself establish
that a migrated mount was accessed or that a new owner was created.

| Site and origin | Classification and current call path | Required authority and available operations |
| --- | --- | --- |
| [mmap.rs:272](../../kernel/src/process/mmap.rs#L272), uncommitted private mmap population | Actual new ring-0 file consumer, currently for legacy file handles. Linux mmap → syscall adapter → private file copy; uses kernel VFS handle stat and positional reads. | Service open description for a migrated file. Copied STAT supplies descriptor metadata and READ supplies bounded bytes, but READ moves the shared cursor. Add bounded positional read on a retained description. |
| [syscall.rs:6259](../../kernel/src/arch/x86_64/syscall.rs#L6259), `ca178af0c` | Actual new legacy VFS metadata consumer. Linux statfs resolves a path through the kernel namespace and asks the kernel FAT backend for capacity by mount ID. | Resolve with authenticated root/cwd and credentials at the owning service. Copied path STAT can model admission, but has no capacity result; a mount-incarnation-bound statfs operation is needed for migrated FAT. |
| [linux_procfs.rs:173](../../kernel/src/linux_procfs.rs#L173), `ca178af0c` | Actual new legacy namespace observer. Procfs open on `/proc/self/mountinfo` snapshots the kernel mount table and checks FAT source IDs against the device registry. Exposure excludes file-routed and root-rebased callers. | Keep procfs synthesis in the kernel and source topology from the caller's authoritative namespace. No copied mount snapshot operation supplies this view; use bounded authenticated topology records or an authoritative publication with equivalent scope and generation checks. |
| [storage.rs:123](../../kernel/src/storage.rs#L123), `ca178af0c`; constructor originally `6c77f3cc2` | Guard classification artifact for the **constructor addition**. StorageCtl MOUNT_FAT32 still mounts through the same kernel owner. The binding became mutable to sample capacity, which **is** new backend work at a legacy owner. | Moving the constructor requires service ownership of this FAT mount and bounded partition I/O. No live service FAT mount operation replaces StorageCtl today. Keep this finding in the guard until the ownership path changes. |

The mmap caller holds scheduler and PMM locks across the current synchronous
operation, and the copy additionally holds the VFS lock. The descriptor must
carry READ rights; non-kernel handles are rejected. The descriptor is borrowed
under the scheduler lock; no independent file retain exists. Bytes go into
sanitized private frames before publication. Short EOF leaves zeroes where
initialized; premature zero read while bytes are expected fails the copy.
Allocation/population and fixed replacement use rollback paths in mmap; those
paths were inspected, not exercised in this pass.

For a service mapping, admit against the exact descriptor and rights under
the kernel lock, retain the service description explicitly, and use the
copied channel's bounded request, park and completion pattern. Use positional
chunks of at most a page; seek/read/restore is incorrect across dup and
concurrent reads. Release scheduler, address space, descriptor and PMM locks
before parking. Pin the original description through close, rename and unlink;
bind each reply to registration epoch, token, operation, caller incarnation
and VM generation. Stage unpublished frames with bounded admission, revalidate
mapping state at commit, and settle one release on success, failure, exit,
exec, timeout or retirement, including late acknowledgements. Committed
private frames belong to the VM independently of the file. There is no
path reopen or file reference after initialization.

Statfs currently obtains kernel-derived credentials and cwd and preserves
secret path restrictions during lookup. It releases the VFS lock before user
copy; errors yield errno and there is no lasting handle or mount mutation.
A service operation needs anchored lookup with the same policy and a
mount-generation check across delay, including unmount/replacement; keep
Linux record encoding and copyout in the kernel.

Mountinfo currently holds VFS then device registry during bounded formatting,
outside the scheduler lock. Procfs checks the node again before installing
the read-only descriptor; a failed snapshot or FD allocation leaves no
published descriptor. A replacement must bind the snapshot to the caller's
visible namespace and revalidate caller and scope before installing the
descriptor. No private metadata or global topology should be inferred from
device enumeration alone.

StorageCtl requires storage admin for MOUNT_FAT32. The mount path validates
the target and FAT32/write device flags under serialized kernel VFS access,
constructs a partition view, probes the backend and publishes the mount last.
Probe failure does not install a mount; bounded non-USB capacity sampling is
best effort. A service cutover must carry the checked admin grant and
partition capability, reserve and validate placement before publication, and
release unpublished backend/device ownership on failure. The existing mount
reference and busy/flush/detach contract must survive this move.

## Stop boundary

The copied regular-file path only has a dedicated native fixture opt-in
(`syscall.rs:4495`); it does not supply production Qalculate descriptors.
Its protocol has cursor-based read and exec transfer but no retained
positional mmap source. Runtime FAT publication remains in the kernel.
Enabling production file routing and moving FAT mounting with its dependent
consumers are substantial migration prerequisites. No site was repaired or
reclassified in the baseline. The guard and ordinary boot fail; native replay
and post-repair validation were not run.

Next, qualify a service-owned mount with production descriptor routing and
bounded retained positional reads, including delayed completion and teardown.
Separately establish authoritative FAT mount, statfs and topology interfaces
before moving the three storage telemetry sites. Rerun the ordinary gate
before binding or running Qalculate.

## VFS-D1 production readiness (2026-11-25)

**Stop before enabling production open.** An ordinary child can eventually
receive a derived binding, but neither ordinary spawn adapter guarantees it
before first dispatch. In `process/spawn.rs:1531` and
`arch/x86_64/syscall.rs:4637` the scheduler publishes and enqueues the child.
`vfs_registration/inheritance.rs::provision_inheritance` returns unit even
if capture returned None; successful queueing sets a profile flag, not a
binding. Only the authenticated retained reply installs the binding. The
source receipt is pinned and derivation checks registration, target identity,
credentials and cwd. Those checks cannot order first dispatch. Inheritance
copies the source grants; a metadata-only boot grant cannot by itself
authorize regular-file open. The native file fixture is not this launch path.

| Lifecycle | Existing contract and D1 qualification gap |
| --- | --- |
| FD and description | The scheduler protects the shared process FD table. A slot holds rights, flags and a tagged FileHandle. Legacy VFS handles name kernel objects. A service handle indexes the mutex-protected ServiceDescription pool (fd_table.rs:528); duplicates retain that row, which carries a token, epoch and reference count. The service owns the open description, cursor and backend object pin. Ownership must follow that object through rename/unlink, never a pathname reopened on demand. |
| Operation admission | file.rs publishes bounded copied requests and matches replies by registration, caller (PID, incarnation, VM generation), request ID and operation. A publication is a pending call, not a retained FD reference. syscall.rs::service_fd copies token and rights but discards the stored epoch. Require an epoch-checked temporary reference across park and completion, independently of slot reuse or final close. |
| Open and rollback | service_open calls install_service_fd after successful completion. Installation takes the current shared table; open_service removes an uninstalled local row on failure, but the install error branch sends no compensating service Close. Bind installation to the captured table/identity and reserve cleanup for every successful service acquisition that cannot be installed, including abandonment after dispatch. |
| Close and inheritance | Taking a slot withdraws it before release. Duplicate slots share the row; final release owns close. Exec filters CLOEXEC and publishes TRANSFER for surviving descriptions before allowing the old file-owner barrier to drain. Thread/fork transfer publication remains unfinished. Local reference counts alone do not grant the child service authority. |
| Exit and restart | Scheduler exit/reaper paths retire the caller; the reaper takes remaining handles. FileOwnerRetired and its acknowledgement are separate from namespace release. File publication expiry and caller/registration retirement exist; a cancelled waiter does not establish that a delivered open never ran. Old descriptions must fail closed after epoch retirement. |

First complete the [child publication contract](../Helios_LinuxCompat/filesystem-namespace.md#readiness-and-child-publication)
for both spawn adapters: reserve an unpublished child and bounded cleanup,
park the requester, then enqueue and report success only on authenticated
binding commit. Preserve the direct init/server bootstrap handshake. This
changes spawn error decoding, resource reservation and timeout/epoch fencing
across the scheduler, init and callers; it cannot be safely implemented as a
descriptor routing switch. Prove delayed acknowledgement, refusal, cancellation
and epoch replacement before first instruction on UP and SMP.

Then establish a single service-owned mount and grant, repair open rollback
and operation lifetimes, and qualify read-only open/read/fstat/dup/close with
independent close and a shared cursor. Reject unsupported operations at that
owner. Future positional reads can use the same retained description without
moving its cursor. No production descriptor lifecycle, rename/unlink or
concurrency result is claimed here. The ordinary gate rerun stopped before
QEMU at 71 references, four outside the baseline.
