# VFS-C1E5: authoritative StorageCtl inventory and withdrawal

The opt-in C1E2 kernel and service now route ordinary StorageCtl
`LIST_MOUNTS`, `INVENTORY_MOUNTS` and service-owned `UNMOUNT` over the
authenticated copied file channel. Production defaults are unchanged.

Inventory samples the C1E4 runtime owner's caller-visible attachment
snapshot. It checks the snapshot revision and visibility before encoding the
existing StorageCtl record. The service root is excluded because the
candidate has not migrated the ordinary kernel root. Service attachments
precede the legacy rows; only an authenticated end-of-service-rows result
permits indexing the legacy table. Service failure is returned to the
caller. The indexed ABI still samples independently on each syscall.
Prepared FAT adapters are private and have no snapshot row. The source,
target and protection fields come from the attachment; the returned ID
encodes the service registration epoch and the full local mount ID (slot
and incarnation). Unrepresentable identities fail closed.

The kernel preserves the public inventory and explicit root plus
StorageAdmin split. It declines rebased views in this candidate and
revalidates the caller, credentials and global view before dispatch and
reply. The service checks the copied registration and scopes the snapshot
to those credentials. Epoch or incarnation mismatch cannot redirect
unmount to the kernel or to a newer attachment.

Withdrawal closes VFS admission for the exact device attachment. Busy
means existing retained roots or descriptions can continue using their
handles and the caller can retry. Once the final pin is gone, the VFS hands
the transaction its FAT backend; the lease comes directly from that
backend. FAT synchronization and transport settlement happen outside the
topology lock. The VFS then finalizes the exact transaction before the
transport releases its lease. A settlement error fences the lease; a
settling or retired mount cannot start a second withdrawal. No mutable
mount index was added.

The publication gate still returns EOPNOTSUPP after exact cookie and
reservation validation. Its remaining prerequisites are:

| Integration point | Required before commit |
| --- | --- |
| `kernel/src/linux_procfs.rs::mountinfo_snapshot` and its ProcFS open path | Install a coherent caller-visible service topology sample, revalidating the exact caller view before installing the description. |
| `kernel/src/arch/x86_64/syscall.rs::sys_linux_statfs` | Route ordinary capacity reads to the service with a caller view and mount incarnation, preserving error and retirement behavior. |
| `kernel/src/process/mmap.rs::sys_mmap` and `process/mmap/service.rs` | Admit ordinary service descriptions to private mmap and retain them for positional reads with existing mapping and teardown guarantees. |
| `kernel/src/storage.rs::mount` and the retained preparation commit | Remove competing runtime FAT construction for the migrated extent; atomically publish registered source and protection and consume the preparation receipt under the lease, ordered against cancellation and retirement. |

The earlier committed-attachment control publishes inside its standalone
fixture. It does not install into the ordinary runtime or authorize an
ordinary StorageCtl withdrawal. The C1E5 native test keeps the gate closed.
Successful ordinary published inventory, actual ordinary withdrawal,
retained FAT descriptor and mmap lifetime, and write persistence through
the new route depend on publication readiness.

Fresh one-vCPU, 2 GiB QEMU execution passed ordinary inventory during
three live preparations, admin authorization checks, rejection of stale
service IDs, and failure without inventory fallback after service
retirement. Five explicit preparation releases and one exact service
retirement settled with zero commands, mapping users, retained command
data or pending flush. The queue and bounce allocation remained 20 KiB
at fixed addresses. The closed disposable image was byte-identical
to its input, passed read-only FAT fsck, and yielded the original
`KEEP.TXT` payload. This verifies preparation settlement; committed
withdrawal settlement is still blocked.

The [results](vfs-storagectl-authoritative-inventory-withdrawal-results.json)
record fresh execution and distinguish it from the historical C1E1–C1E4
evidence.
