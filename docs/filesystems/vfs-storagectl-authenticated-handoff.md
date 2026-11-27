# VFS-C1E authenticated StorageCtl handoff

**PARTIAL. Ordinary handoff and coordinated routing are BLOCKED.** This
pass verified the current build and ownership boundary. It did not wire
the ordinary syscall to the service or publish a migrated mount.

Syscall 168 checks StorageAdmin for mutation, copies the target, then
enters the kernel VFS. Its mount arm calls `storage::mount`; the unmount
and mount inventory arms also operate directly on the kernel graph.
The C1C/C1D service acquires its lease through the separate native block
fixture (syscall 180). Resolving a registered extent in that fixture does
not link the lease to an ordinary caller, target reservation or StorageCtl
continuation. The archived C1D service exercises retained FAT handles,
but supplies no evidence of service ownership from syscall 168.

| Historical finding | Disposition | Smallest coherent replacement |
| --- | --- | --- |
| Runtime FAT construction | **BLOCKED**: `kernel/src/storage.rs:153` constructs kernel FAT. | Bind authenticated StorageCtl admission to a parked continuation, the exact registered source and physical lease, and the registered service epoch. Pass the partition span to the existing block executor. Stage FAT in the reserved service target and publish only after checking readiness and routing. Settle the lease on cancellation or completed withdrawal; keep uncertain transports fenced. |
| ProcFS topology | **BLOCKED**: `kernel/src/linux_procfs.rs:173` reads the kernel graph. | Install an immutable mountinfo snapshot for the retained caller view after authenticating its revision. Use that graph for StorageCtl mount inventory as well. |
| Linux statfs | **PARTIAL**: service capacity exists; `kernel/src/arch/x86_64/syscall.rs:6450` still reads kernel capacity. | Resolve capacity from the service mount selected by the caller's retained path view, and copy it out using the existing Linux ABI only after the continuation validates. |
| Private mmap | **PARTIAL**: candidate service transactions exist; `kernel/src/process/mmap.rs:277` still reads ordinary kernel VFS files. | Admit ordinary opens as service descriptions and route private mapping of those descriptions through the retained service transaction and positional reads. |

The publication gate must also cover native and Linux traversal, retained
root and cwd, creation, mutation, sync, open and description operations
including seek, PREAD, duplication and final close. Each route exercising
the attachment must select the same mount incarnation. No isolated metadata
adapter can make publication safe while file operations still use another
graph.

Production kernel, release service, and the existing isolated C1C kernel
build passed. The latter uses the archived C1D fixture ELF; it is not a
new C1E image. Host library tests passed (296) and the registration harness
passed (112). The unchanged guard rejected both enforcement modes with 71
references and the same four unmatched findings, so guarded ordinary boot
was **NOT_RUN**.

Reverification of the archived populated and unknown FSInfo C1D images
passed independent closed-image `fsck.fat -n`, exact payload and fixture
accounting checks. Each variant has four archived cycles at one vCPU and
2 GiB, with 20 KiB retained DMA, zero outstanding commands and zero mapping
users at release. These are archived component results. Fresh native
StorageCtl, reattachment, retained directory and descriptor operations,
private mmap, mountinfo/statfs, cleanup and image validation are all
**NOT_RUN**. The single-CPU restriction remains; no SMP FAT qualification
is claimed. The C1D results and their links to earlier accepted and rejected
evidence remain unchanged.
