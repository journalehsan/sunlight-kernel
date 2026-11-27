# VFS-C1C live storage registry integration

**PARTIAL.** The isolated candidate binds the disposable VirtIO queue to the live registry and the existing authenticated lease authority. Ordinary StorageCtl attachment and production cutover remain **BLOCKED**.

## Registry and access paths

| Path | Ownership and lifetime |
| --- | --- |
| VirtIO boot | PCI discovers and initializes one queue. Production registers the physical capacity, assigns a disk ID, discovers partitions through the boot queue, and publishes a legacy transport handle. Boot share reads and boot FAT construction occur before normal runtime. The share's copied bytes do not carry transport access. |
| VirtIO runtime | Boot-selected FAT and StorageCtl FAT instances both use `KernelBlkDev` views of the same `VIRTIO_BLK` queue. File, exec and private-mapping readers, directory mutations, sync and capacity sampling can reach that transport. Unmount checks retained handles and flushes. Serialization alone does not withdraw legacy access. |
| USB | Kernel boot registers the physical device and discovered children; `usb_storage::DISK` owns the mass-storage transport. The USB reservation also keeps generic xHCI delegation from claiming that controller. USB FAT goes through a bounded view and is outside this candidate. |
| Registry aliases | A physical row has parent zero; partition rows name that parent, and a superfloppy uses the physical row. Registry IDs are monotonic inventory identities. UUIDs, discovery ordinals and mount paths confer no authority. Source ranges are absolute, checked sector intervals within the physical disk. |
| Removal | The live registry has no removal or replacement protocol. The lease authority has its own opaque media incarnation and fences transport loss. In this candidate the private queue and registered rows remain fixed for the boot; duplicate parent rows, USB media changes and live hot replacement have not been qualified. |

The C1C boot branch checks the disposable marker, discovers the actual MBR,
registers the physical disk and its FAT source in `storage::DEVICES`, and moves
the queue into the existing N1J/N1P owner before either legacy VirtIO slot or
boot FAT is populated. Neither physical nor partition inventory IDs can reach
that transport through StorageCtl, and there are no preexisting legacy
descriptions to drain for this disk. This is an exclusive first handoff; it
does not qualify transfer of an already mounted production disk.

The authority registers the queue's physical capacity as one medium. On each
authenticated acquisition the kernel re-resolves the configured source and
requires the exact physical and source records before acquiring against the
same opaque media handle. The whole-device lease prevents concurrent grants;
native reads and writes are separately restricted to the registered FAT
partition. Rejected acquisition installs no active generation. The existing
authority still controls admission, data completion, release barriers and
uncertain transport fencing. The runtime does not add a parallel owner table.

The ordinary syscall still authenticates StorageAdmin and dispatches to the
kernel mount path. The native block adapter authenticates the registered
service image but does not consume a StorageCtl continuation or caller-scoped
reservation. The disposable FAT lifecycle only publishes fixture topology.
Ordinary publication requires one transaction that binds caller, target
reservation, source, medium, lease generation and service mount incarnation;
then crosses that topology for path and descriptor operations and reads it
for both inventory ABIs, ProcFS and Linux statfs. Boot-selected FAT migration
also requires explicit withdrawal, confirmed sync and a transport handoff.

## Stable FAT identity implementation plan

FAT directory entries carry a short-entry slot, LFN slots, first cluster and
size. Slots move or are reused; a first cluster can be zero, change on writes
or truncate, or be reused after deletion. The current `FatOpen` retains a
historical path and anchored service lookup stops at the FAT mount root.

1. Add a mount-scoped retained object table, with a root object and checked
   monotonic IDs. Intern lookups by the current short-entry locator under the
   FAT lock. Retained file and directory descriptions reference the object
   ID and mount incarnation; an I/O operation resolves its current live
   locator under that same lock. Repeated lookup and open share identity.
2. Update the locator in the same serialized rename transaction, including
   cross-directory moves and replacement. Keep source identity and retire
   the overwritten locator so slot reuse cannot alias a retained object.
   Keep descendants anchored to their own objects when a parent moves.
   On partial FAT write or sync failure invalidate uncertain lookups and
   require recovery; preserve existing destination-first durability and
   FAT/FSInfo accounting.
3. Select the supported unlink and replace lifetime semantics explicitly.
   Initially reject destructive mutations of pinned objects unless a
   detached-chain lifetime is implemented and qualified. If supported,
   withdraw lookup immediately, defer freeing until the last description or
   mapping pin settles, and ensure detached objects cannot be reopened by
   slot or cluster reuse. Test truncation through a second description.
4. Close new crossings at withdrawal, keep retained IDs and lease access
   until drain and sync, then invalidate the incarnation. Remount assigns a
   fresh incarnation; persistent cross-remount inode numbers are not needed
   for this contract. Qualify rename, directory move, empty files, reused
   slots, pinned unlink, failed media writes and stale handles before wiring
   ordinary routing.

## Qualification boundary

See the [results index](vfs-live-storage-registry-integration-results.json).
On one vCPU and 2 GiB, the C1C kernel with the qualified N1P service ELF
completed four authenticated FAT cycles per disposable disk, with both
populated and unknown FSInfo. The verifier matched four lease generations
to physical ID 1 and source ID 2, covering all 131072 sectors for ownership
and sectors [2048, 131072) for data. Closed-image checks preserved the prefix
and unrelated data, and counted 127004 → 127003 free clusters. Each release
settled with zero outstanding commands, zero mapping users and the same
20 KiB retained DMA allocation. Additional C1C CPU configurations were
**NOT_RUN**; this fixture requires one CPU.

Registry and host regressions, the registration harness, production kernel
and release service builds, and the isolated candidate build passed. Fresh
replays of the archived C1A image passed C1A, D1B and D2B at 1/2/4 vCPUs.
The qualified D2A logs passed unchanged reverification at all three CPU
counts; the older 2/4-vCPU PREAD Busy failures still reject.

The unchanged ownership guard still reports 71 ring-0 references and four
unmatched historical findings: runtime FAT and ProcFS topology are **BLOCKED**;
Linux statfs and private mmap remain **PARTIAL**. The C1B two-vCPU retain timeout
and four-vCPU withdrawal-order rejection remain separate failures: the former
records a timeout for child 20, retain 26 before its settlement marker.
Request 156 in the latter reaches caller terminal with one service row
before the service withdrawal marker; the fresh four-vCPU replay shows the
required reverse order. Neither failure reproduced in the fresh replays.
The logs do not establish a shared cause or resolve the earlier D2A seek
failure. No acceptance criterion changed.
