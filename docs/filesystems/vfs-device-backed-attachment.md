# VFS-N1L: Authenticated FAT32 attachment

**PARTIAL (November 27, 2026).** The host N1D transaction now publishes a
working FAT32 backend over the existing authenticated lease authority.
It uses one disposable RAM-resident sector image and one trusted fixture
registration. The FAT driver reads and writes sectors; the RAM filesystem
only provides the parent namespace. The host transport models barrier
completion with a separate durable image in RAM. It does not qualify
file-backed or physical storage.

## Readiness and publication

The existing N1D reservation captures the exact VFS owner, topology revision
and empty target directory identity. Staging then acquires a whole-device
read/write/flush lease for the supplied authenticated registration and medium.
Outside the topology lock, it creates a fresh, uncached FAT32 driver, checks
the BPB and sector extent, verifies the bounded root allocation chain and
reads the root directory. This is a readiness check, not a filesystem repair
or full volume integrity scan.

The private stage owns both the initialized backend and the reservation. The
backend holds the same lease binding that publication revalidates. Under the
VFS lock, publication checks the reservation again, then holds the existing
lease state lock across the in-memory insertion. That pin checks the exact
lease, media generation, active phase and registration session, including
service epoch. No device I/O or IPC runs in the publication pin. The backend,
crossing, mount incarnation and topology revision become visible together.
Invalid initialization, cancellation and failed publication release the
unpublished lease after drain and a confirmed host barrier. If that fails,
the lease stays fenced. Failed publication consumes no slot or revision.

## Withdrawal

1. Match the existing VFS owner and mount incarnation. Advance the revision
   and close new path admission. New snapshots omit the closing mount; path
   resolution keeps it in place and returns Busy instead of falling through
   to the underlying directory.
2. Return Busy while handles remain. Existing handles keep their backend and
   can read, write and close. Existing caller views and pending snapshot
   installations reject the old revision.
3. At zero pins, sync FAT, then close lease data admission, confirm drain and
   complete the covering host transport barrier. The lease remains occupied.
4. Under both owners' locks, verify the synchronized release transition,
   remove the row, advance the revision and release the lease. A failed
   settlement keeps the mount closing and the lease fenced. Dropping the
   installed control handle without withdrawal fences the lease as well.

Host sector copies and barriers are synchronous memory operations. The host
test path has no blocking device call under the topology lock. Native use
needs a separate preparation and settlement path that does not hold topology
or scheduler locks across blocking I/O or IPC.

## Qualification

| Gate | Result | Evidence |
| --- | --- | --- |
| Host FAT32 lifecycle | PASS | Four publish/withdraw cycles on one sector image. Verified file creation, write, reopen, stat size, positional read, untouched existing file, Busy on a live handle, ordinary close, fresh mount IDs, scoped rows and stale snapshot rejection. |
| Failure and accounting | PASS | Malformed boot sector, stale revision and revoked registration reject publication. Successful compensation and withdrawal end vacant with zero caller credits, retained operations and flushes. Injected outstanding work keeps the row closing and lease fenced; terminal completion drops the retained command without reopening admission. |
| Persistent backing image integrity | NOT_RUN | Host remount and unchanged-file checks cover logical image contents; no host file or QEMU backing image was changed or compared for N1L. |
| Native FAT mount and FD installation | BLOCKED | N1J's authenticated VirtIO coordinator can drive fixture requests, but the ring-3 filesystem service has no authenticated block submission/completion interface. FAT root visibility from a tmpfs view is qualified on the host; FAT retained service objects and native snapshot/FD installation are open. |
| N1L QEMU, DMA and physical storage | NOT_RUN | No N1L guest mount cycles, DMA allocations or physical barriers were exercised. |
| N1J historical evidence | PASS | Preserved authenticated registration, exact image retirement, and 64 lease cycles on the disposable device. The one retained allocation was four queue pages and one bounce page (20,480 bytes). The run matched the full image and finished with zero retained commands and mapping users. This did not return DMA frames to the allocator. |
| Ownership guard | FAIL | Unchanged guard reports 71 references and the same four unmatched sites: statfs, ProcFS mountinfo, private mmap and runtime FAT mounting. |

Physical DMA reclamation and coordinated cutover remain **BLOCKED**.
The ordinary boot disk, runtime FAT owner, production routing and
`/vfs-scratch` are unchanged. L1C-3 remains blocked, VFS-D1 deferred,
and demand-backed main stacks opt-in.

See [machine-readable results](vfs-device-backed-attachment-results.json).
