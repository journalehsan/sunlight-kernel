# VFS-N1: ownership closure and native baseline

2026-11-27, primary working tree based on `631e547e4`. **Native baseline
BLOCKED.** One inventory mismatch is resolved. Four historical references still
fail the no-growth check; removing them requires contracts that are not ready.
No kernel, service, launch profile or memory policy change was needed for N1.

## Five matches on entry

The [guard](../../tools/audit_vfs_cutover.py) compares a multiset of path,
category and exact source text. Line movement is immaterial; a total of 71
against 68 inventory rows does **not** mean only three unmatched sites.
The [historical four-site review](helios-vfs-authority-audit.md) describes
the same operations as rows 1–4 below.

| Source in this tree | Origin and classification | Disposition |
| --- | --- | --- |
| `kernel/src/arch/x86_64/syscall.rs:6309`, sys_linux_statfs | `ca178af0c`; transitional consumer. Historically line 6259. Queries live FAT capacity for the kernel-resolved mount. | **Open.** Bind the authorized path lookup and capacity to one mount generation, preserving cwd, traversal, capacity units, errno and kernel copyout. A service stat record cannot supply this information. |
| `kernel/src/linux_procfs.rs:173`, mountinfo_snapshot | `ca178af0c`; transitional consumer. Kernel procfs formats a kernel topology snapshot using the device registry. | **Open.** Keep procfs presentation and caller visibility checks; obtain a bounded, generation-consistent topology from the caller's namespace authority and revalidate before FD installation. Device discovery alone cannot determine visibility. |
| `kernel/src/process/mmap.rs:272`, sys_mmap | Now committed in `b79f7e4d8` (uncommitted in historical review); live file ownership dependency. READ-checked kernel descriptor, fstat and offset-based reads populate private frames. | **Open.** Retain the exact service description and rights across bounded positional reads; release scheduler/PMM locks before parking, stage and revalidate before mapping, compensate on failure. Cursor-based READ or pathname reopen cannot preserve dup/unlink semantics. Completed private pages must remain independent of VFS, including during ZRAM reclaim and restore. |
| `kernel/src/storage.rs:123`, mount | Original constructor `6c77f3cc2`, changed by `ca178af0c`; live ownership dependency. The mutable binding is not a second constructor, but enables new capacity sampling on the real runtime FAT backend. | **Open.** Keep admin-checked mount, partition range, write rights, bounded non-USB sampling, busy checks, sync and detach together. Stage the owner and cache before publishing one generation, unwind failed probes. No equivalent service FAT contract; no inventory edit for this row. |
| `kernel/src/main.rs:2567`, init_kernel_vfs | **Fifth**, `611ff358a`; bootstrap requirement, demonstrated text-match artifact **as an addition**. The parent called the same constructor once on the same root with BOOT_INITRAMFS. The child commit passes the boot-resource entries when enabled and BOOT_INITRAMFS otherwise (main.rs:767–778). | **Inventory resolved; root remains kernel-owned.** Replaced the obsolete exact expression with its successor in the same baseline row. No second allowance or change to compressed resource selection. Retire the constructor only with a full root handoff. |

## Executed checks and native stop

Fresh `python3 tools/audit_vfs_cutover.py --no-growth` exited 1 both
before (71 references, five unmatched) and after inventory review (71, four).
An in-memory check with the guard's own additions function verified that either
a duplicate new root expression or the obsolete expression adds a fifth
violation. The inventory still has 68 rows; the script is unchanged.
`./tools/test.sh` exited 1 at the same four findings before building or
launching QEMU. Logs: `target/vfs-n1/{guard-before,guard-after,ordinary}.log`.
There is no fresh guest, service receipt, or native baseline for this tree.

| Ordinary operation | Historical evidence only; K = kernel, S = service | Fresh |
| --- | --- | --- |
| Download and discover | Fetch created through K (no S delivery on create); selected path stat used S retained root/cwd and received validated ENOENT in both STACK-2A controls. No independent stored byte comparison or proof of a loader failure. | NOT_RUN |
| Git mkdir/chdir | Linux mkdirat used K cwd/*at resolution and kernel credentials; selected chdir reached S with authenticated anchors and received ENOENT. Separate run4 Prepare/EAGAIN calls never reached S; absent versus invalid binding unresolved. | NOT_RUN |
| PTY marker | Producer create and subsequent ordinary open/read used K. The fixture report attributes the metadata miss to routed stat; it establishes readable marker bytes, but supplies no per-request identity or validated reply trace. | NOT_RUN |
| Packaged executable stat | STACK-2B short eager: S validated success, then foreground execution succeeded. Short and isolated demand: utility exited 1 without observed publication. Long pair failed in both modes. Service delivery/error **unknown** for those failures. | NOT_RUN |

Historical evidence: [download and packaged runs](vfs-namespace-consistency-audit.md#failure-boundaries),
[Git run4 and admission](../Helios_LinuxCompat/filesystem-namespace.md),
[PTY fixture](../memory/tty-pty-functional-qualification.md). When the ordinary
guard passes, capture fresh boots with matched image inputs, RAM, CPU, resource
mode, service grants and commands; change only eager versus opt-in demand stacks.
Use identical packaged and localhost-downloaded ELF bytes, fresh directory and
marker names, the ordinary Git and PTY paths, and a second process for reopen,
enumeration and byte comparison. Correlate raw errno, operation, caller
incarnation/VM, credentials, retained root/cwd, epoch, publication, service
receipt and validated completion; distinguish admission from traversal. Record
shell discovery and actual loader entry/exit separately. Preserve failing logs.

## Proposed first mount: /vfs-scratch (design only)

Reserve a new empty, volatile, data-only mount in the initial namespace.
Do not replace /tmp: its existing executable downloads and IPC marker users
would require a broader transition. S owns all scratch inodes, entries,
quotas, descriptions and directory cursors. K owns authenticated crossing of
the mountpoint, caller identity and descriptor rights. Identify the mount by
namespace ID, mount ID, generation and service registration epoch. Resolve
component crossings from retained anchors, including dirfd, chdir, dot-dot
at the mount root and rebased process root. Parent enumeration and topology
must expose the same published boundary; never use a prefix-only dispatch.

| Contract before publication | Required behavior |
| --- | --- |
| Namespace and mutation | Agree across native and Helios on lookup, access, stat, mkdir/rmdir, create/exclusive, open, append/truncate, relative and dirfd operations, getcwd/chdir, directory enumeration, rename/replace and unlink. Stable open objects survive rename/unlink. Cross-mount rename fails EXDEV. Report volatile capacity from enforced byte/inode quotas; bound handles, cursors and requests too. |
| Other entry points | Bind read/write, positional I/O, seek, fstat, dup, close, sync and directory cursors to the original description. Inventory *all* syscall variants, legacy IPC, embedded/ordinary exec, interpreter and mapping paths. Explicitly reject execution, file mapping, special nodes, links and unsupported metadata operations consistently before any K lookup or side effect. No persistent writeback promise. |
| Admission | Both ordinary spawn adapters must install exact identity, root/cwd, credentials and operation grants before first execution. Keep bootstrap narrowly scoped. Check kernel credentials, search/write permissions, capability restrictions and FD rights; S independently validates registration, anchors, token and grant. Revalidate after credential, cwd, exec or epoch changes. |
| Lifetime | Pin the epoch-checked description over parked operations, protect against FD reuse, share offsets only when required. Qualify inheritance, CLOEXEC, displacement, last close, exit and exec transfer. Reserve cleanup before Open; authenticate compensating Close on failed installation or abandoned reply. Namespace withdrawal does not close a file. |
| Transition and loss | Start unpublished with a reserved empty target and no covering K objects, handles, cwd pins or in-flight crossings. Build bounded S state privately. Admit only when every adapter and parent view is ready; publish one generation atomically. Failed staging releases reservations and leaves the target absent. After publication fence new requests on loss, drain/settle old operations and retire all pins; never reveal a backing K directory. Restart cannot recover data or rebind old tokens. Explicit remount creates an empty new generation; whole-mount withdrawal only. |

Host gates: test crossing/root confinement, quota boundaries, permissions,
cross-mount rename, open/unlink lifetime, directory enumeration, positional
versus shared offsets, FD/epoch reuse, delayed replies, failed installation,
rollback and atomic publication against concurrent readers. Native gates:
first pass the unchanged ordinary guard/boot and record the four N1 cases;
then exercise ordinary native and Linux callers against each other's mutations,
stat/fstat, getdents, real dirfds, cwd and exact bytes. Test denials, noexec
and mapping refusals, service loss/restart with open FDs, retained cwd and
in-flight writes, no fallback and accounting after drain. Prove first-use
readiness and clean rollback on UP and SMP; record delivery for every routed
operation. All scratch acceptance gates are NOT_RUN. Neither L1C-3 nor
production D1 is enabled by this design.
