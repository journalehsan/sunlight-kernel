# VFS-C2I1: Persistent mounts before ordinary admission

**PARTIAL.** The opt-in candidate connects immutable fstab, early reservation,
standalone storagectl and committed publication to ordinary spawn. The
unchanged ownership guard blocks ordinary boot; guest cases are **NOT_RUN**.

The candidate uses kernel `vfs_c2i1_candidate` and service
`vfs-c2i1-candidate`. The compiled C2C fstab is selected with
`sunlight-fs/vfs-c2c-fixture`; compressed boot uses Limine's inventory.
Production defaults are unchanged.

Before probing FAT, the kernel reads the boot inventory, discovers USB and
the boot VirtIO partitions, and discovers the designated non-boot C2B PCI
function. It rejects the boot function, I/O BAR conflicts, cloned PARTUUIDs,
unsupported geometry and media without the disposable marker. The existing
authenticated owner receives the queue for exactly one selected FAT extent.
Exclusion spans the registered physical parent and all its aliases; I/O is
bounded to the partition. Multiple resolved entries are refused. The
candidate does not census unselected VirtIO functions or lease USB.

Configured parents and the claimed /mnt subtree cannot reach legacy runtime
attachment. The early partitioned boot /mnt producer is disabled in this
case; raw boot media retain their protected boot path. Invalid or unreadable
fstab closes admission and fences GPT attachment. Existing nofail semantics
allow an absent source; ambiguous or present but unmountable entries fail.

Init waits for authenticated retained root metadata while serving the
nameserver. Only the exact embedded /bin/storagectl fstab launch by the
registered bootstrap init can pass the gate early. It still receives its
namespace receipt before dispatch and its existing StorageAdmin grant.
Independent TTY startup is unchanged. The candidate omits C1E fixture callers
and their test directories under /mnt.

StorageCtl restricts preparation to the reserved source and configured
target. The existing receipt, N1D publication and compensation paths handle
the transaction. Only the accepted publication acknowledgement records a
committed lease generation. The current lease must remain active for
ordinary spawn and publication. Withdrawal or settlement closes readiness.
The service table is authoritative for the attachment. A rejected
publication acknowledgement invokes existing insertion rollback; storagectl
aborts the exact preparation token on error. There is no new guest evidence
for lease, attachment, DMA or frame accounting. Refused transport setup
does not demonstrate DMA reclamation.

| Historical finding | Remaining owner |
| --- | --- |
| `storage::mount` | Unreserved devices still construct kernel FAT at unclaimed targets. |
| `linux_procfs::mountinfo_snapshot` | Legacy callers still read kernel mount topology. |
| `sys_linux_statfs` | Unrouted Linux callers still read capacity from the kernel owner. |
| `process::mmap::sys_mmap` | Legacy readable root and FAT descriptions still reach kernel mmap. |

No legacy constructor was removed. Candidate fencing does not retire all
legacy FAT ownership or switch the ordinary runtime root. See the
[results](vfs-pre-admission-persistent-mounts-results.json).
