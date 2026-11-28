# VFS-C2I3B protected SSH file ownership

**PARTIAL.** The opt-in candidate routes the protected staging producer and
published reopen to the authenticated Ring-3 VFS. Host tests pass for the
copied service and kernel transaction paths. The unchanged ownership guard
blocks native boot before QEMU, so the native producer-to-consumer run is
**NOT_RUN**. Coordinated runtime root activation remains **BLOCKED**.

## Producer and consumers

The production producer is `sys_secret_create` in
`kernel/src/arch/x86_64/syscall.rs`. It exclusively creates the staged file
in kernel tmpfs and installs `FileHandle::vfs` with WRITE, FSTAT, and
CLOEXEC. Generic `open_resolved_path` installs the readable description
for the published reopen. The stage cannot pass mmap's READ check. The
legacy description from a published reopen can reach legacy regular-file
mmap if used by an eligible Linux caller.

`sunlight-libc/src/secret_store.rs` is the consumer of these syscalls.
`create_if_absent` and `replace` validate the existing destination as
applicable, generate an exclusive staging name, verify staging FSTAT,
write the full payload, check the size and validator, publish, and close.
The create-if-absent loser checks the winning published file.
`load`, `existing_status`, and `validate_existing` reopen through ordinary
OPEN and validate the owner, mode, type, size, and content as applicable.
The native StatPath probe distinguishes a missing destination from a
failed reopen. `cleanup_stale_temps` is the legacy directory sweep, and
`secret_remove_temp` is the legacy rollback for a failed stage.

The checked-in ordinary executable using this lifecycle is the disabled
by default `secret_store_test.service` managed by SunlightD. It carries
the existing HostKeyAdmin grant and uses synthetic payloads. The source
tree also declares and validates the expected SSH host-key path. The SSH configuration code consumes path
metadata only. Candidate builds embed a
fresh copy of the fixture linked against the descriptor-bound libc ABI.

## Candidate ownership and settlement

- The existing privileged native create syscall admits an authenticated
  copied OPEN with WRITE, FSTAT, and CLOEXEC. The kernel attests
  HostKeyAdmin and snapshots the exact caller identity and credentials.
  The service requires a live retained root receipt, the unrebased service
  root, and the root-owned `/etc/sunlight` directory on that same tmpfs
  mount. The parent must have no group or other write bits. Staging is
  exclusive, with the caller's UID and GID and regular-file mode
  `0100600` (permissions `0600`).
- The existing service descriptor handles write and FSTAT. The staging
  descriptor remains open through publication, and the copied publication
  names its exact epoch, token, owner, source, and destination. The service
  checks staging mount and inode identity against the named source under
  the same VFS lock. Publication reuses the private-file metadata
  validation for create-if-absent and replace-existing. The protected
  namespace contains no seeded private destination of the required mode.
  Replacement in the candidate is for dynamic files in that tmpfs.
- The service prepares the rename and allocates its pathname before
  acknowledgement. The kernel rechecks the caller incarnation, authority,
  credentials, and registration at the copied acknowledgement. If the
  acknowledgement is refused, the old publication stays in place. On
  acceptance the service changes the path without further fallible work,
  seals stage writes, and returns to the serialized dispatch loop.
  Readers that already held the old file keep its inode through close.
  The existing contract is atomic visibility on a live system.
  `RequireDurability` still fails closed, since this tmpfs supplies no
  stable-storage file or directory sync.
- Failed provisional OPENs, rejected replies, and failed fd installation
  use the existing descriptor drop and compensating CLOSE. A successfully
  installed protected OPEN reserves its existing frame for final CLOSE, so
  normal close needs neither a new slot nor a frame allocation. Protected
  opens can occupy at most half the publication table. Failed write or
  publish closes the stage through that path. The service conditionally
  unlinks only the original staged inode and closes the retained backend
  handle under one lock. Stale or reused pathnames do not identify the
  cleanup target. File-owner retirement settles the exact caller, and
  registration retirement drops outstanding file descriptions.
- The existing protected OPEN authorization runs before generic routing in
  the candidate. Authorized published reopen and its native StatPath probe
  use the same service. Unprivileged service OPEN, relative aliases, and
  legacy daemon handles have no route to protected contents or mutation.
  The candidate does not sweep staging names with legacy ReadDir or
  unlink. Descriptor ownership removes abandoned stages, and the legacy
  remove syscall refuses pathname deletion in this routing mode.
  Service failure returns an error without selecting kernel tmpfs.

The candidate no longer executes the legacy staging constructor or the
protected branch of generic legacy OPEN. Other native and Linux
OPEN/openat paths, production staging and reopen, legacy FAT, inherited
legacy descriptors, and native C capture can still create or carry kernel
regular-file descriptions. Kernel tmpfs remains the owner of the ordinary
legacy root. No legacy constructor was removed globally.

## Qualification

Host tests drive the actual copied service dispatcher through exclusive
creation, exact mode, credential rejection, write, FSTAT, conditional
publication, ordinary descriptor read and close, and replacement with an old
reader still pinned. They cover out-of-space write, refused provisional
OPEN, refused publication acknowledgement, create collision, stale token,
unrelated staging-name replacement, caller incarnation retirement, and
registration withdrawal. Repeated closes return to the service handle
baseline, counting retained directory pins separately. Kernel host tests
cover install failure, reserved CLOSE settlement, and rejection after
capability revocation, incarnation change, or deadline expiry. A filesystem regression
also confirms that exclusive create releases its inode if handle generations
are exhausted. The separate libc identity tests require the Sunlight syscall
ABI and timed out when tried on Linux. The candidate adapters reach
`open_service` for both protected opens. Native handle, frame, and inode
accounting through the ordinary syscall adapters is
**NOT_RUN**. The prior C2I3A worker evidence remains intact.

Both unchanged ownership gates **FAIL** before and after. The exact
inventory is **71 → 71**, with **0 additions and 0 removals** when compared
by file, category, and source text. The same four findings exceed the
reviewed no-growth baseline.

| Historical finding | Remaining live dependency |
| --- | --- |
| `storage::mount` | Unreserved StorageCtl devices still construct kernel FAT. |
| `linux_procfs::mountinfo_snapshot` | Legacy Linux callers still read kernel mount topology. |
| `sys_linux_statfs` | Legacy Linux callers still obtain capacity from the kernel owner. |
| `process::mmap::sys_mmap` | Remaining readable legacy regular-file descriptions still use kernel metadata and positional copy. |

## C2I4 readiness

The protected file source-routing prerequisite is implemented in the
candidate. Its native qualification remains open. Ordinary native and
Linux lookups outside staged routing still use the legacy root, even when
the process has a metadata receipt. Remaining callers include legacy
OPEN/openat and directory-relative lookup, pathname mutations and metadata,
non-embedded executable lookup, and legacy stream inheritance or native C
capture. Trusted KV, synthesized public account data, and compatibility
adapters still need coordinated admission and policy verification.

C2I4 requires the following work before coordinated activation.

1. Publish the authoritative root and retained cwd before dispatch and
   admit remaining ordinary native and Linux callers. Route pathname
   operations, executable admission, descriptor inheritance and stdio, and
   exec/exit through that authority. Preserve the C2I3A shared-worker
   publication and generation rules.
2. Preserve C2I1's committed reservation and readiness for the eligible
   extent, then fence unreserved devices and publish the remaining early
   `/state`, `/mnt`, `/boot` FAT crossings and runtime StorageCtl mounts
   through exclusive service ownership. Keep the immutable boot share and
   archive as bootstrap resources. Mountinfo and capacity must observe the
   same published graph.
3. Qualify ordinary native and Linux OPEN/openat, directories, cwd, append,
   positional I/O, private mmap, protected publication, and exact resource
   drains through guarded native boot. Include service withdrawal and
   replacement, caller exit and exec, and the existing storage and memory
   regressions. Retire the remaining legacy descriptions before removing
   the legacy mmap dependency.
4. Keep the 2 MiB executable source limit. Direct TTY bootstrap
   (3,314,704 bytes) and uncompressed VFS bootstrap exceed the limit and
   use direct embedded startup. The uncompressed candidate VFS is 89,100,848 bytes. The compressed
   candidate VFS is 524,848 bytes. The previously inventoried oversized ordinary or optional
   sources are Dict (14,058,032), Yazi (26,942,728, with the historical
   baseline at 24,379,624), Files (2,342,000 in the current local artifact),
   Rappid Rabbit (5,502,192), Vortex Shell (5,813,072), Control Panel
   (2,397,008), and optional DocServ (5,043,424). They need a supported
   source at activation without increasing the snapshot limit.
5. Resolve all four remaining no-growth findings and qualify the guarded
   runtime before changing defaults. Final removal requires clearing the
   full live ownership inventory. D3 fault restoration remains deferred.

See the [result matrix](vfs-protected-ssh-file-ownership-results.json),
[C2I3A report](vfs-worker-admission-continuity.md), and
[staged plan](userspace-vfs-migration.md).
