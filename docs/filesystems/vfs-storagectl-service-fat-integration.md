# VFS-C1B StorageCtl service FAT integration

**PARTIAL. Ordinary StorageCtl transfer and production acceptance are BLOCKED.**
The existing service runtime, policy bindings and file descriptions can now
use the same block-capable namespace as an N1D attachment. A host regression
instantiates that combination. Ordinary boot still constructs the existing
device-free service namespace, and no StorageCtl request was routed to it.

## Authority and transaction boundary

Native syscall 168 reads credentials from the current process. Except for
public device and mount inventory, StorageCtl requires UID zero and the
explicit StorageAdmin capability. The target is copied and decoded before
the VFS lock. Device enumeration describes registry geometry and operations.
Mount resolves the selected source ID to its parent transport and extent,
requires FAT32 and WRITE, validates the target, constructs a bounded
partition view (USB or VirtIO) and probes FAT in the kernel. VirtIO samples
capacity with a 1024 FAT-sector budget; USB does not publish capacity.
Mount insertion publishes source, protection and backend together.
Both mount inventory operations read indexed rows from KERNEL_VFS.
Unmount resolves the mount ID there and checks protection, busy handles and
sync before detach. Record copyout happens after the guard is released.

The kernel must retain caller admission, registry identity and geometry,
overlapping extent exclusion, authenticated sector access, transport and DMA
settlement. The registered service must own FAT probing, retained objects,
path crossing, allocation, capacity and the attachment graph. The current
native block channel authenticates the service but is compiled as a fixture
and admits a fixed disposable extent. Registry IDs are not leases. A live
handoff needs a trusted binding from source ID to canonical media and exact
extent, with whole-device, partition and internal kernel writer aliases fenced.

Reserve the exact service target and topology revision before probing. Bind
the staged backend and its probe to the reservation, the authorized caller
view and the active lease, then install identity, source, protection and
capacity together. The current native fixture has source zero. Carry real
source metadata from the authenticated handoff before enabling public
inventory. Reject stale reservations, caller or epoch changes and lost
continuations; release unpublished leases after confirmed settlement, or keep
uncertain transport fenced. For withdrawal, close crossings before flushing,
honor retained descriptions, finish removal under the existing N1D split
transaction and return the lease only after confirmed transport settlement.

Move StorageCtl mount, unmount and mount inventory together with every
supported path and descriptor operation on the extent: open/create, stat,
directory traversal, reads, writes, seek, positional I/O, mutation, sync,
exec and private mmap. Keep ProcFS mountinfo and Linux statfs on that same
graph. No catalog formed by joining independent snapshots is authoritative.
Boot-selected FAT remains a kernel attachment and requires its own transition;
the read-only boot share can remain a bootstrap input.

## Integrated prerequisite and limits

The service's existing copied runtime now carries one device type through
control dispatch, policy pins and file descriptions. The default type stays
NullDevice. The host test binds a caller to the same namespace in which an
authenticated disposable lease publishes FAT. Capacity resolved through a
retained caller view identifies that attachment. The runtime opens an
ordinary tmpfs file in that shared graph, and its description stays usable
after FAT withdrawal and lease settlement. Anchored FAT lookup below the
mount root still returns Unsupported. The test verifies that boundary
through the same authenticated open path instead of using an unanchored
fallback. Durable retained FAT directory and file identity remains required
before ordinary FAT open, mutation or private mmap can be enabled.

The N1D host attachment suite separately verifies scoped snapshot visibility,
revision invalidation, mount incarnation changes, capacity updates, retained
file reads during withdrawal and compensation. The ProcFS FD install fence
and Linux statfs copyout still need authenticated kernel adapters. The
existing synchronous snapshot install callback cannot fence a round trip.
Retain immutable mountinfo bytes per description and preserve the bounded
Linux format, visibility, copyout errors and the existing 120-byte statfs ABI.

## Qualification and ownership

| Gate | Status | Evidence boundary |
| --- | --- | --- |
| Ordinary StorageCtl and ordinary FAT file operations | **BLOCKED** | Missing live registry lease handoff and anchored FAT identity. No competing mount was added. |
| Integrated native 1/2/4-vCPU, 2 GiB matrix | **NOT_RUN** | No ordinary StorageCtl attachment can be published by this candidate. |
| Shared runtime and caller view | **PASS** | Host component test. Ordinary ProcFS/statfs and FAT descriptor/mmap acceptance remain **NOT_RUN**. |
| N1P native FAT fixture | **PASS** | Archived N1P image, fresh disposable disks and independent closed-image checks: four cycles each with populated and unknown FSInfo, free clusters 127004 → 127003, 20 KiB stable retained DMA, no remaining commands or mapping users. |
| Host and release builds | **PASS** | See the [results index](vfs-storagectl-service-fat-integration-results.json). |
| Unchanged ownership guard | **FAIL** | 71 ring-0 references; no-growth rejects the same four historical findings. Ordinary acceptance remains blocked. |

| Historical finding | Disposition |
| --- | --- |
| Runtime FAT construction | **BLOCKED.** StorageCtl still calls kernel storage mount. Fixture lease and block I/O do not authorize a real StorageCtl source. |
| ProcFS topology | **BLOCKED.** ProcFS still reads the kernel VFS and registry. No copied, caller-scoped snapshot install transaction exists. |
| Linux statfs | **PARTIAL.** The service capacity primitive resolves against the retained caller view and mount incarnation. Linux still resolves and samples the kernel mount. |
| Private mmap | **PARTIAL.** The isolated D2 path uses retained service descriptions. Ordinary Linux kernel VFS mmap remains live, and anchored FAT descriptions are unavailable. |

Replays of the archived C1A candidate used the unchanged ordinary, D1B and D2
verifiers. One vCPU passed all three. At two vCPUs the guest timed out after an
L1C retain timeout; it did not reproduce the historical D2A seek result. At
four vCPUs the guest completed and C1A/D1B passed, but the D2B verifier rejected
the order again: the exact caller terminal event for request 156 preceded
the service's 1→0 handle and rejected late reply record. The terminal record
still held one service row, and a later checkpoint reported zero. These are
distinct events; this run remains failed under the original required ordering.
The historical two-vCPU seek failure has a successful write for token 20 and
the caller's Err(Failed), without a matching seek service completion in that
interval. This trace cannot distinguish an admission failure from a failed
or lost completion. No seek behavior or verifier criterion changed.
