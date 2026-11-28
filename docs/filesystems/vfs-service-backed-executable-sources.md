# VFS-C2G authenticated service-backed executable sources

**PARTIAL.** The opt-in `vfs_c2e_candidate` supplies executable bytes to
ordinary native spawn and native or Helios exec from the owning VFS service.
The copied host integration qualifies source selection, authentication,
positional reads, ELF input and source settlement. Ordinary boot is
**BLOCKED** by the unchanged ownership guard. Native and Helios guest
execution and end-to-end image rollback are **NOT_RUN**.

## Executable source boundary

| Entry | Source and loader |
| --- | --- |
| Bootstrap, internal service launch and legacy manager spawn IPC | The existing embedded-byte lookup and bootstrap image construction remain available independently of the VFS service. |
| Ordinary native spawn (39) and native exec (31) | The shared selector supplies bytes to `exec_into_process`. Downloads paths and relative paths under an authenticated Downloads cwd take the candidate route. Other mutable paths still use the legacy whole-file reader. |
| Helios Linux execve (59) | The Linux adapter translates to native exec (31). The existing ELF personality selector chooses the Linux ABI and stack setup from the ELF bytes. The native spawn entry can also load a Linux-personality ELF. |
| Interpreters and dynamic libraries | Ordinary static loading retains its interpreter and dynamic-image checks. The separate Helios debug handoff selects trusted embedded interpreters and fixture libraries using its existing rules. Runtime dynamic linker opens outside migrated authority still follow their existing filesystem routes. |

The two ordinary syscall sites previously called the same legacy executable
reader after embedded lookup. That reader holds the kernel namespace lock
through pathname stat, open, whole-file read and close. The candidate
intercepts migrated paths ahead of that reader, including errors and
service loss. One legacy reader invocation remains in the shared selector
for mutable paths outside candidate authority. Privileged service identity
and embedded tool grants still rely on their existing embedded provenance.

## Authorization and consistency

The candidate accepts an absolute Downloads pathname, a relative path
from the authenticated Downloads cwd, or a Downloads path joined against
the legacy cwd. It requires an existing live caller admission with a
retained namespace receipt. A caller can acquire that admission through
the existing Downloads cwd transition. A rebased root is refused, and a
worker sharing another process's cwd cannot use that owner's relative
service authority. Production bootstrap and ordinary production builds
keep their existing source selection.

The new copied OPEN option is kernel-origin. The VFS service checks the
authenticated registration and namespace receipt, searches from the
retained anchors with the captured uid and gid, and checks that the
resolved object lies in the mounted Downloads subtree. The protected
Downloads mount has no mount-wide execution prohibition in the existing
code. The service requires a readable regular file and a matching execute
mode bit. Root also requires at least one execute bit. The existing root
and directory search permissions apply, and file mode supplies no set-ID
transition. Ordinary user OPEN flags confer no executable-source authority.

The service uses one mounted VFS lock scope for pathname authorization,
open, handle metadata and byte capture. The retained backend handle
validates its mount slot and incarnation and pins its exact inode across
rename or unlink. The service stores the captured image in the existing
file description and reports its authoritative length in the OPEN reply.
The source is limited to 1 MiB. PREAD uses the authenticated registration,
caller, token, offset and bounded length. Reads come from the one
captured byte image, so a later rename, path replacement, write or
truncation cannot change the loader input. The capture and assembled
loader buffer are transient allocations. Neither is a filesystem copy.

The kernel requires a full response for every requested range and
rechecks the exact caller image, credentials, retention receipt and
registration before using the image. Service loss or stale authority
fails the launch. The existing ELF loading path still validates the
machine, personality, program headers, segment bounds and permissions
and constructs the isolated user address space with its existing W^X
rules.

## Source lifetime and process transactions

| Stage | Ownership and failure handling |
| --- | --- |
| Pending OPEN | The existing copied publication owns the request. Rejected or abandoned successful replies use the existing compensating CLOSE exchange. |
| Accepted OPEN | The same publication slot and frame allocation reserve an exact CLOSE before exposing the token to the internal reader. The source consumes no user fd. |
| PREAD | One candidate acquisition can be parked at a time. It holds a bounded loader buffer and exact caller binding. The scheduler, broker and registration guards are released before the syscall parks. |
| Terminal | Success, bad reads and cancellation activate the reserved CLOSE. A canceled positional publication has no cursor effects. Caller and registration retirement retain the existing file-owner and epoch cleanup. |
| Spawn | The source settles before child allocation and ELF construction. Existing private-image rollback and C1A authenticated namespace publication gate the child before dispatch. |
| Exec | Source failure precedes image replacement. ELF construction uses the existing rollback on failed replacement. Successful exec retains existing descriptor, old-image retirement and namespace-inheritance ordering. |

## Qualification

The copied host matrix uses the candidate route and source reader, the
kernel file-publication runtime, the real service dispatcher and the
same ELF personality, header and segment planning functions the kernel
loader uses. It covers native and Linux-personality ELF inputs, a
truncated ELF, denial of a readable file without execute bits, rename
with old-path replacement, in-place mutation and truncation between
PREADs, successful cleanup, abandoned OPEN, cancellation, short and
failed PREADs, and service withdrawal. The legacy namespace fixture
opens no executable handle. Host registration tests also exercise
existing private spawn publication failure and image-retirement
transactions. Fault injection through guest spawn and exec, page-table
construction, final dispatch, and native C2E through C2F2 and
C1E6A through C1E12 replays remain unqualified.

The unchanged audit produces 71 references before and after. The
multiset of file, category and source text has zero additions, deletions or
relocations. Inventory is **PASS**, no-growth is **FAIL**, final removal
is **FAIL**, and the ordinary boot command stops before QEMU.

| Historical finding | Disposition |
| --- | --- |
| `storage::mount` (dd00fa775) | Remains. The legacy FAT constructor still exists for the surviving kernel storage path. |
| `linux_procfs::mountinfo_snapshot` (dd00fa775) | Remains. Legacy topology still serves the non-service path. |
| `sys_linux_statfs` (dd00fa775) | Remains. Legacy capacity still serves the non-service path. |
| `process::mmap::sys_mmap` (5b8579f1d) | Remains. Legacy private file mmap still consumes surviving kernel-backed descriptions. |

Generic native open and Linux open/openat still produce legacy file
descriptions through `open_resolved_path` outside the migrated route.
The private secret staging syscall and the opt-in native C fixture also
produce legacy descriptions. Service-backed executable acquisition
produces none. Bootstrap embedded sources and debug dynamic dependencies
do not use the mutable executable source reader.

The production release and candidate debug and release builds passed.
The C2C GPT fixture matched its prepared SHA-256 checksum. The parallel
descriptor run failed its stale-alias cleanup assertion once. All 20
descriptor tests passed on the serial rerun.

Build, test, audit and fixture outcomes are recorded in the
[machine-readable results](vfs-service-backed-executable-sources-results.json).
