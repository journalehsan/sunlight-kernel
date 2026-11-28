# VFS-C2H legacy descriptor and mmap retirement

**PARTIAL.** Exit cleanup now validates the incarnation of an open
description before dropping a reference. A failed SSH staging descriptor
installation now closes the unpublished handle and unlinks the temporary
file. Zero legacy descriptor producers were migrated. The legacy Linux
private mmap path remains reachable from production OPEN and openat on the
kernel root and legacy FAT mounts. Both ownership gates still fail with 71
inventoried references and four historical findings. Ordinary boot is
**BLOCKED** and fresh guest execution is **NOT_RUN**.

## Descriptor regression

C2G's parallel run failed at fd 24 in
`exit_cleanup_consumes_stale_descriptor_alias_without_double_release` with
`Some(FileHandle(0))` where it expected `None`. The C2G results and failed
log remain in place. The original harness passed its 20-test rerun here. The shared description pool still makes the failure
possible. The fixture deliberately inserts an alias without retaining a
reference. Cleanup consumes fd 0 and frees the row. Another table can reuse
that slot before cleanup reads fd 24. The old presence check accepts the
replacement, and the separate release lock can also race with slot reuse
between those operations.

The focused regression models the resulting state deterministically with a
retired descriptor identity and another table's live row. Disabling only
the incarnation check reproduced the original assertion values. With the
check enabled, exit reports no backend to release for the stale alias, the
replacement still sees its cursor at 37, and its owner can close it as the
final reference. This is a real weakness in defensive retirement, exposed by
the fixture's synthetic alias and shared pool. The test does not establish
that normal descriptor creation produces unretained aliases in production.

[`FdTable`](../../kernel/src/process/fd_table.rs) stores a slot and a
nonwrapping incarnation on each open description identity. Exit checks and
decrements under one lock. Normal ownership release still requires a live
reference. Duplication and inheritance preserve identity and continue to
share the authoritative cursor. The incarnation replaces the redundant fd
offset copy with a four-byte-aligned identifier, keeping the descriptor table
layout within its previous size. Memfd mapping views retain their existing
slot representation and hold a live reference while using it. The C1E6A
initialization and C1E6B cleanup assertions remain in place.

## Reachable producers and readers

| Boundary | Source and remaining consumer |
| --- | --- |
| Generic native OPEN and Helios Linux open/openat | Linux open translates to the native OPEN number. Both it and native OPEN use `sys_open`; Linux openat resolves through `linux_at_path` or uses a service directory token with `service_open_at`. On the legacy branch, `open_resolved_path` uses `Vfs::open`, `create_file` or `create_file_exclusive`, applies truncation, and installs `FileHandle::vfs`. Linux private mmap accepts the readable regular descriptions this branch installs. Native spawn can inherit a legacy description outside the file-routed cohort. |
| Legacy mutable root | `init_kernel_vfs` constructs a writable root tmpfs over boot seeds. Ordinary files in that root remain kernel-owned outside the existing candidate routing, including shared temporary files, mutable configuration and application data. Metadata and pathname mutators, legacy directory-relative lookup, read/write/seek/stat/close, and the non-embedded `vfs_read_file` executable reader still depend on that ownership. |
| Legacy FAT | `init_kernel_vfs` attaches early persistent storage, and `storage::mount` attaches ordinary runtime FAT via StorageCtl where C2B has not reserved the parent. Generic opens on those mounts still install readable kernel descriptions. C1E10–C1E12 cover the existing committed service-owned FAT route. |
| Protected SSH staging and published files | `sys_secret_create` creates a mode-0600 exclusive temporary file and grants WRITE and FSTAT with CLOEXEC. That staging descriptor itself cannot pass mmap's READ check. The protected reopen of the published file through generic OPEN can supply a readable descriptor. `sunlight-libc/src/secret_store.rs` validates metadata, writes, closes, calls `sys_secret_publish` and reopens the result. `sys_secret_remove_temp` and rollback also use the same owner. The failed-FD-installation branch now closes its unpublished handle before unlinking. No secret bytes were used or recorded for qualification. |
| Native C guest | The opt-in `native_c_guest::launch` prepares an ELF through the writable fixture root and installs three legacy VFS capture streams before dispatch. Readable stdin can reach the legacy branch for a Linux-personality fixture; stdout and stderr are write-only. The capture collector and frame, name and handle baselines rely on that same filesystem. The fixture's image transfer itself does not install a guest descriptor. |
| Immutable bootstrap sources | Boot seeds, indexed compressed resources, embedded executables and the boot VFS server image retain their provenance. Direct startup or embedded-executable loads do not install user file descriptors. Generic OPEN of a seeded file through writable root tmpfs still installs an ordinary VFS descriptor, even when that particular payload originated in immutable storage. Early persistent disk state is mutable. |
| Other descriptors | Serial, TTY, Linux null device, pipes, sockets, eventfd, epoll and synthetic proc records use other handle types. Kernel directory descriptors may use `FileHandle::vfs` but fail the legacy mapping type check. Shared-page and memfd mappings have separate shared-memory handling. |

The legacy Linux branch in
[`process::mmap::sys_mmap`](../../kernel/src/process/mmap.rs) takes the
scheduler-owned descriptor, checks READ, uses `KERNEL_VFS` to fstat it and
populates eager private pages by positional reads under the VFS lock. The
C1E7 syscall adapter already passes service-owned descriptions to D2A/D2B.
Removing the legacy branch today would remove mappings for readable ordinary
files on the kernel root and legacy FAT mounts.

## Coordinated activation

Before removing that branch, the same publication boundary must bring
ordinary native and Linux pathname operations and their descriptor semantics
onto the authenticated service namespace. It must cover root startup and
inheritance, retained cwd and directory-relative operations, creation and
truncation, read/write and cursor operations, mutations, and executable
source access. Remaining Linux compatibility work includes append and ordinary positional I/O. Vectored I/O uses the
ordinary read/write adapters and still needs continuation qualification. Preserve existing
pathname policies such as the trusted KV store and HostKeyAdmin checks. The
protected staging, publish, cleanup and reopened read must use one owner
with an authenticated capability check and atomic owner/mode validation.
Opt-in capture fixtures need a retained service description installed before
dispatch and collection through the same owner, preserving capture and
cleanup assertions. Retain immutable boot provenance and make its ordinary
filesystem access available through the published namespace.

For each writable mount, select the service owner before the first ordinary
creation, or freeze admission and settle open descriptions, directory/cwd
pins, executable sources and in-flight writes before handoff. Publish the
retained root, mount crossings, pathname routes and caller scopes through
the existing C1A and attachment mechanisms. For FAT, settle writes and I/O
and transfer exclusive device ownership before admitting the successor.
Enable ordinary routing for all participating callers together, with service
loss returning an error. Once all readable legacy producers and inherited
descriptions are gone, remove the legacy mmap copy path and qualify ordinary
dispatch using C1E7. No production routing changes were made in C2H.

## Ownership guard

| Historical finding | Source-level disposition |
| --- | --- |
| `storage::mount` | Remains. `Fat32::mount(KernelDisk::new(view, usb))` still constructs the legacy backend for ordinary StorageCtl attach, including legacy fstab consumers. Needs service attachment authority for remaining devices and coordinated removal of the legacy constructor. |
| `linux_procfs::mountinfo_snapshot` | Remains. `KERNEL_VFS.lock()` still supplies legacy caller-visible topology. Needs authenticated graph observation for remaining callers and mounts. |
| `sys_linux_statfs` | Remains. `KERNEL_VFS` still supplies capacity by legacy mount identity for unrouted Linux callers. Needs service capacity on the same published graph. The finding moved down two source lines due to staging cleanup above it. |
| `process::mmap::sys_mmap` | Remains. `KERNEL_VFS.lock()` still supplies the eager positional copy described above. Needs all readable legacy producers and inherited handles retired. |

Inventory enumeration **PASS**, 71 to 71, with zero added, removed or
relocated ownership sites. No-growth **FAIL**, four findings before and
after. Final removal **FAIL**, 71 references before and after. The unchanged
guard stopped `tools/test.sh` before QEMU.

## Qualification

Host descriptor, filesystem, service FAT, authenticated registration and
copied transport results are indexed in the
[results file](vfs-legacy-descriptor-mmap-retirement-results.json). The host
coverage exercises directory anchors, create/truncate, positional reads and
cursor preservation, close and retirement, exact failed-publication
compensation and service withdrawal. The host harnesses do not invoke
ordinary native or Linux OPEN, openat or mmap inside a guest. Native and
Linux syscall execution, eager page installation and the SSH staging
full-fd-table syscall case are **NOT_RUN**. No new C1E10–C1E12, C2B, C2E,
C2F, C2F1, C2F2, C2G, C1A or C1E6A/B guest evidence is claimed. The prior
[committed FAT mmap evidence](vfs-committed-fat-private-mmap.md) and C2G
failed descriptor evidence retain their original scope. The C2C GPT fixture
passed its existing SHA-256 check.
