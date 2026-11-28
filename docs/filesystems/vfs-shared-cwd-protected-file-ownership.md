# VFS-C2I3 shared cwd and protected file ownership

**PARTIAL.** Native shared workers and Linux thread clones under the Downloads
candidate now stage separate retained namespace admission when created with an
authoritative Downloads cwd. Protected SSH file routing remains open. No
regular-file producer or kernel filesystem ownership site was retired.

The creation paths call the existing registration runtime under scheduler and
broker serialization. Admission binds the worker's PID, process incarnation,
address-space generation and credentials, with a separate receipt derived from
the owner's current retained directory pair. The derivation checks that the
shared owner remains in the same address space and still holds the source
binding. The worker's pathname field is not the anchor. The existing service
directory pair keeps object identity through rename.

The retained file channel uses the worker's own receipt. Directory transitions
serialize against copied pathname operations; on a successful transition,
workers' prior profiles are withdrawn without discarding the completed cwd
continuation. Existing workers are not readmitted to the new anchor, and a
worker that calls before admission completes can receive an error. Those gaps
must be closed before claiming coherent ordinary shared-cwd operation. Tests
cover the host retention exchange, an independent relative open, transition
out of Downloads, and final reference drain. Native clone execution, the
ordinary relative mkdir/stat matrix, and withdrawal on the actual syscall
path are **NOT_RUN**.

SSH create, publish, temp removal and published reopen still reach the legacy
owner. `sys_secret_create` still installs a kernel regular-file description
with WRITE and FSTAT rights. The path and HostKeyAdmin checks, root-owned
protected parent validation, mode 0600, exclusive creation, conditional
replacement and C2H close/unlink compensation remain intact. The filesystem
suite exercises synthetic private publication and replacement; that is not
qualification of a service-owned SSH producer. Migration must bind creation,
write/sync, publish, unlink and reopen to the same authenticated service
authority, including compensation if descriptor installation fails.

Coordinated root activation remains **BLOCKED**. Existing shared workers after
a transition and ordinary callers outside the staged root/Downloads admission
paths still lack the required namespace admission. Generic legacy open and
openat, SSH staging and reopen, and opt-in native C capture streams retain
kernel descriptions. Embedded bootstrap ELFs and the immutable boot archive
and share remain bootstrap sources; public account compatibility requests and
trusted KV still need coordinated routing and policy verification. C2I1 reserves
the eligible persistent extent; unreserved runtime StorageCtl devices and
remaining legacy FAT mounts on the `/state`, `/mnt` and `/boot` routes still
need exclusive service ownership and publication before ordinary dispatch.
Mount topology, capacity and legacy private mmap must switch at that boundary.

Previously measured ordinary ELF sources still over 2 MiB include Dict,
both Yazi variants, Files, Rappid Rabbit, Vortex Shell, Control Panel and the
optional DocServ. See the [runtime root inventory](vfs-runtime-root-authority-transition.md)
for measured sizes. The C2I2 limit remains 2 MiB.

Both ownership gates were run before and after. Inventory remains 71;
no-growth and final removal both **FAIL**. Guarded boot stops before QEMU:
guest-dependent worker, protected file and previous milestone replays are
**NOT_RUN**.

| Historical finding | Remaining dependency |
| --- | --- |
| `storage::mount` | Legacy FAT backend construction |
| `linux_procfs::mountinfo_snapshot` | Kernel mount topology |
| `sys_linux_statfs` | Kernel capacity lookup |
| `process::mmap::sys_mmap` | Legacy regular-file descriptions |

See the [result matrix](vfs-shared-cwd-protected-file-ownership-results.json).
