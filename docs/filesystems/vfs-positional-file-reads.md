# VFS-N1B: retained file description positional reads

2026-11-27, primary working tree based on `631e547e4`. **Service primitive
qualified on the host. Production mmap integration NOT_RUN.** The retained
service-owned description now answers a bounded read at an explicit offset
without touching its shared cursor. The private file-backed mapping path still
reads through the kernel VFS under the scheduler and PMM locks, and its
historical ownership finding remains open.

## Phase 1: authority and lifetime audit

### What the private mapping path owns today

[sys_mmap](../../kernel/src/process/mmap.rs#L230) accepts a file-backed private
mapping only for a Linux-compat caller. It checks `MAP_PRIVATE`, rejects every
other flag, requires a page-aligned `offset` whose span fits below `i64::MAX`,
looks the descriptor up in the shared process table, requires the handle to be
a kernel VFS handle and requires the READ right. It then takes
[`KERNEL_VFS`](../../kernel/src/process/mmap.rs#L272), samples
`fstat_handle` for the regular-file check and size, and passes a populate
closure to `map_private_kind`. That closure issues offset reads for each frame
through `vfs.read(handle, file_offset + done, ..)`, writing straight into a
freshly allocated, unpublished HHDM frame.

| Requirement | Current authority |
| --- | --- |
| File description | Kernel shared-process FD slot: rights, flags and a tagged `FileHandle` naming a kernel `Vfs` object. No service token participates. |
| Backing object | Kernel VFS mount table and its inode, kept alive by the open kernel handle; the pathname is never consulted during population. |
| Offset | The caller's `mmap` offset plus the frame index. No cursor exists in this path and none is advanced. |
| Lifetime | The FD lookup keeps the kernel handle live for the caller; completed private pages are independent of the filesystem afterwards, including under ZRAM reclaim and restore. |

The lock shape is the part that cannot be transplanted. `sys_mmap` is entered
with the [scheduler and PMM locks
held](../../kernel/src/arch/x86_64/syscall.rs#L12849), and the kernel VFS guard
lives across frame allocation and mapping. Any service-backed source would have
to park for an IPC reply while holding all three.

### What the service description owns

The service table keeps rows of `{owner, token, Arc<Mutex<Description>>}`, and a
[`Description`](../../services/vfs_server/src/open.rs#L48) is
`{File { filesystem: Arc<Mutex<Vfs>>, handle }, access, offset}`.

| Property | Contract in the service |
| --- | --- |
| Duplication | [duplicate](../../services/vfs_server/src/open.rs#L128) clones the `Arc` into a new row with a new token: one description, one cursor, one backend pin, separate ownership rows. |
| Cursor | Single `offset` field shared by every duplicate of that description; ordinary [read](../../services/vfs_server/src/open.rs#L165) and [seek](../../services/vfs_server/src/open.rs#L246) update it. |
| Rights | `access` is checked per operation; the token alone grants nothing because `Table::index` matches owner and token together. |
| Identity | The pin is the open handle, not the pathname. Rename and unlink leave the object reachable; a new file at the old path is a different object. |
| Release | A row drop frees one reference; the [last `File` drop](../../services/vfs_server/src/open.rs#L39) closes the backend handle, ending the namespace open credit. |
| Transfer | [transfer](../../services/vfs_server/src/open.rs#L142) re-keys the owner and preserves token, description, cursor and pin. |

This model already supports an independent positional read. The filesystem
interface is offset-addressed (`Vfs::read(handle, offset, buffer)`), so a
positional read is a first-class call: the description supplies identity and
rights, the request supplies the offset, and the shared cursor is simply not
part of the operation. Nothing new has to own the file, and no seek is needed.

### Dependencies that stay open

The service primitive is a prerequisite, not the prerequisite set. Removing the
historical dependency also needs:

1. **Descriptor epoch validation on lookup** (VFS-D1 repair one, deferred).
   The kernel row stores `{token, epoch, refs}` and `service_fd` discards the
   stored epoch (see `helios-vfs-authority-audit.md`, "VFS-D1 production
   readiness"). This stage did not exercise that path. The service checks the
   frame's registration epoch per request, but a kernel client must still
   carry and validate its installed epoch.
2. **Compensating Close on failed installation** (VFS-D1 repair two, deferred).
   A description retained for a mapping must be released when the kernel cannot
   install or complete the mapping.
3. **Parking outside incompatible locks.** The reply wait must not hold
   `SCHEDULER`, `PMM` or `KERNEL_VFS`, which today's call chain holds.
4. **Staged private frames.** Population must read into unshared, unpublished
   frames, validate, then map, and release frames, address-space reservations
   and the description on every failure and on cancellation.
5. **Namespace ownership prerequisites.** N1 leaves four historical owners
   open and N1A leaves the ProcFS topology snapshot unimplemented; neither
   gates this primitive, but both still stop ordinary boot before QEMU.

Because of items 1 to 4, production routing is not enabled here. It also would
be wrong to describe the mmap finding as resolved: `mmap.rs:272` still takes
`KERNEL_VFS` and still reads through the kernel filesystem.

## Phase 2: positional read contract

The operation is codec opcode `PREAD = 9` on the existing file-transport frame.
It shares the frame layout, the authenticated registration and the caller
identity with every other file operation; it adds no capability.

| Field | Meaning |
| --- | --- |
| `registration` | Must equal the service's authenticated registration; otherwise the frame is dropped without a reply. |
| `caller` | Selects the description row. PID, incarnation or address-space generation mismatch is `BadHandle`, reported as EBADF (9). |
| `token` | The retained description token. Zero is an encoding error. |
| `arg0` | Explicit absolute file offset. Any `u64` is representable on the wire; the service decides if it fits. |
| `arg1` | Requested length, 1 to `IO_BYTES` (4096). Zero is an encoding error, exactly as for ordinary `READ`. |
| `length`, payload | Must be zero. Namespace, receipt and UID/GID words must be zero. |

Service behavior, in order:

1. Resolve owner and token, then take the description lock. No rights are
   inferred from a token that the caller does not own.
2. Require `Access::Read` or `Access::ReadWrite`; otherwise EACCES (13).
3. Convert the offset to `usize` and check `offset + length`; a value that
   cannot address the service buffer space is EINVAL (22) before any backend
   call.
4. Call the backend at that offset. Report the returned count unchanged:
   a short read stays short, and a read at or past EOF returns 0 bytes with
   errno 0.
5. Never read, write or clamp the shared cursor. A positional read is not
   observable through `SEEK_CURRENT`.

Preserved properties: shared cursor semantics, descriptor rights, identity
across rename and unlink, bounded length, explicit offset validation, EOF and
short reads, existing private mapping semantics, registration-epoch and
description lifetime validation, and cleanup on close, owner retirement, epoch
retirement and service teardown. Positional reads are not implemented as seek
plus read: there is no seek in the request, no cursor restore, and no window in
which another duplicate could observe a moved cursor. The description lock does
serialize a positional read against a cursor read on the same description while
the backend call runs, which is the same serialization ordinary reads already
have.

## Phase 3: implementation

Only the service primitive and its wire contract changed.

| File | Change |
| --- | --- |
| [ipc/src/vfs_rpc/file_transport.rs](../../ipc/src/vfs_rpc/file_transport.rs#L26) | `PREAD = 9`, a shared operation predicate used by both decoders, request validation, reply validation shared with `READ`, and a codec regression. |
| [services/vfs_server/src/open.rs](../../services/vfs_server/src/open.rs#L194) | `Table::read_at`: owner and token lookup, rights, offset and range validation, backend offset read, no cursor update. |
| [services/vfs_server/src/runtime.rs](../../services/vfs_server/src/runtime.rs#L296) | `read_at_file` and request dispatch into the bounded 4096-byte reply buffer. |
| [services/vfs_server/src/runtime/tests.rs](../../services/vfs_server/src/runtime/tests.rs#L623) | Registers the new test module. |
| [services/vfs_server/src/runtime/tests/positional_reads.rs](../../services/vfs_server/src/runtime/tests/positional_reads.rs) | Seven end-to-end service tests over encoded frames. |

No mutable file ownership authority was added: `read_at` takes `&self`, reuses
the existing description, and holds no state of its own. Kernel code, the
private mapping path, the FD table, capability and credential checks, FAT and
statfs ownership, `/vfs-scratch`, the ProcFS snapshot design, demand-stack
policy, the kernel heap size and ZRAM policy, and the ownership inventory are
untouched. L1C-3 stays blocked and VFS-D1 stays deferred.

## Qualification

Host and service evidence only. Nothing in this stage reached a guest.

| Gate | Result | Evidence |
| --- | --- | --- |
| Service library tests | PASS, 54 (7 new) | `target/vfs-n1b/service-tests.log` |
| IPC library tests | PASS, 169, including the new codec case | `target/vfs-n1b/ipc-tests.log` |
| Registration regressions | PASS, 108, unchanged | `target/vfs-n1b/registration.log` |
| Kernel build | PASS | `target/vfs-n1b/kernel.log` |
| VFS service production build | PASS, linker script and `x86-64-v2` flags | `target/vfs-n1b/service.log` |
| Ownership guard | FAIL, unchanged: 71 references, 4 outside the inventory | `target/vfs-n1b/guard.log` |
| Ordinary boot | BLOCKED at the guard; QEMU not started | `target/vfs-n1b/ordinary.log` |
| Native validation | NOT_RUN | blocked by ordinary boot |

The service tests cover: explicit offsets at the start, middle and end of the
file; cursor preservation across reads, seeks and `SEEK_CURRENT`; independent
positional reads on two duplicates with one cursor; EOF at and past the end;
short reads; three unrepresentable offsets returning EINVAL without disturbing
the description; read-rights denial; foreign caller, incarnation and address
space rejection; a stale registration epoch dropped before dispatch; a closed
token; rename and unlink with a replacement file at the old path; retention
across `retire_caller`; release under `retire_file_owner`; epoch withdrawal
with EIO; and the namespace open-handle count returning to baseline.

The first kernel build attempt was terminated by the harness and left an
incremental object that failed to link with an undefined anonymous symbol.
Rebuilding with `CARGO_INCREMENTAL=0` succeeded; no source change was needed.

## Service primitive versus production integration

Qualified: a bounded positional read on an authenticated, retained service
description, with cursor, rights, identity, lifetime and error behavior as
described above.

Not qualified, and not attempted: a kernel client that sends `PREAD` for a
mapping FD, parking for its reply outside the scheduler, PMM and kernel VFS
locks, staged frame population with compensation, epoch-checked temporary
references, and any routing switch in `sys_mmap`. The live kernel backend
dependency at [mmap.rs:272](../../kernel/src/process/mmap.rs#L272) therefore
remains **open**, and ordinary boot remains blocked before QEMU on the four
historical owners from [N1](vfs-ownership-closure.md).

Machine-readable results: [vfs-positional-file-reads-results.json](vfs-positional-file-reads-results.json).
