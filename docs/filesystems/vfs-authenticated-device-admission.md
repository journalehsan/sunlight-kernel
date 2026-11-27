# VFS-N1F: Authenticated device admission and extent fencing

N1F qualifies the host admission contract on the existing N1E fake device.
The N1E lock remains the sole owner of the lease, medium, queue, barriers,
and fence. The N1D shared VFS remains the mount authority. Neither the
fixture nor its registration bindings are linked into the freestanding service.

## Authority and extent contract

| Identity or permission | Host contract |
| --- | --- |
| Device | One canonical fake device slot per authority, identified by opaque scope. Handles are invalid across slots, even if their serials match. No physical alias discovery is modeled. |
| Registration | The trusted host coordinator binds the existing VFS RPC registration: endpoint, PID, process incarnation, address space generation, and registration epoch. Binding issues a scoped session with a checked serial. Only that exact live session can use its lease. Withdrawal closes admission under the lease lock and keeps accepted I/O available for settlement. |
| Lease | Acquisition and transfer issue an opaque lease generation. Transfer checks both registrations at reservation and again at commit. A withdrawn incumbent must be drained or retired by the coordinator; a withdrawn successor cannot commit, even if the barrier succeeded. |
| Media | The device slot has zero or one registered medium. Registration assigns a fresh checked media generation and immutable capacity and logical block size. Planned replacement requires release or retirement, removal of the old medium, and fresh registration. |
| Extent | A grant covers one nonempty, aligned byte range on the canonical slot. Offsets are absolute; checked addition and capacity checks reject overflow and out of range requests. One data credit covers exactly one logical block; shorter blocks use a fixed size host buffer with zero padding on reads. |
| Rights | Read and write are checked independently. Client flush needs an explicit right and a grant over the entire device. The trusted coordinator can issue the N1E durability barrier for handoff or retirement, even when the client only has a partial extent or no flush right. |

Binding models authenticated kernel context; it does not authenticate raw
user input or independently look up kernel registration liveness. The
coordinator supplies current registrations and delivers exact withdrawal
events. The opaque session cannot be reconstructed from a registration
payload. The fixture holds at most 16 concurrent bindings per device; it
reclaims each binding at withdrawal without keeping tombstones. Production
must reuse the kernel-issued credentials and capability checks and resolve
registration lifetime at the authoritative kernel boundary.

One lease reserves the entire device. A second overlapping, adjacent, or
disjoint extent cannot acquire a concurrent lease. Extents restrict access
within that reservation. Granting concurrent leases would require canonical
physical alias resolution, transport enforcement of the actual affected
range, cache and writeback isolation, and verified ordering or coordination
for every device-wide operation. For now, a device-wide flush closes data
admission, waits for all accepted data credits, then holds one barrier credit
until terminal settlement.

## Media loss and accounting

Data and flush tickets carry the device scope, lease and media generations,
and a unique operation serial. Settlement checks the retained lease identity
and consumes only the exact pending credit. Removal during ownership fences
the device and leaves the original fake image and queue available for
terminal settlement. Even after all credits return, the fence stays latched
and replacement is refused. There is no in-place recovery from uncertainty.
After orderly replacement, a delayed duplicate from the previous medium is
stale and cannot alter a new medium, consume its credit, or fence it.

Rejections precede operation ID allocation and queue insertion. N1E retains
four data credits and one barrier credit; a barrier cannot be admitted while
any data credit remains charged. Terminal settlement removes the command and
its buffer once. The host churn test fills and reclaims all four data credits
and the barrier credit over 64 registration, lease, and media lifecycles.

## Requirements before production integration

1. Authenticate every admission through the existing kernel capability and
   registration checks. Bind it to the canonical physical device, aliases,
   geometry, media incarnation, extent and rights. Serialize withdrawal,
   replacement notification and transport submission with that authority.
2. Prove terminal settlement for each command and durability barrier on the
   real controller, including late replies, resets and uncertain writes.
   Keep DMA mappings, buffers and transport resources until device access
   has ceased and completion ownership is resolved. Caller return,
   cancellation and device disappearance alone do not prove quiescence.
3. After predecessor writeback, drain and durability, probe the successor
   read-only with a fresh backend and cache. Bind readiness to the N1D
   reservation, exact registration, media and lease generations, geometry,
   extent, rights and root identity.
4. Hold a bounded readiness pin through N1D publication. Revalidate the
   target, slot, owner and topology revision at insertion. Preserve
   caller-view and snapshot installation fencing; the kernel FD install
   remains a separate requirement.
5. On failed publication, keep the target fenced. Dispose of an untouched
   reservation directly. After transfer, revoke and drain the unpublished
   backend, prove unchanged media and no staged writes before regrant, and
   reprobe with fresh generations and cache if the predecessor was destroyed.
6. Withdraw by exact mount incarnation. Freeze new access while allowing
   close, writeback and settlement. Retained roots and file descriptions
   continue to pin the attachment; preserve description identity, shared
   offsets and explicit PREAD offsets. Reclaim ownership only after mount
   withdrawal, reference retirement, durability and DMA settlement. Fence
   and require explicit recovery if any of these cannot be proven.

## Qualification

Eight N1F tests pass, alongside all 14 N1E lease tests and all seven N1D
attachment regressions. Filesystem, IPC, VFS service and registration suites
pass 477 tests in total; all 469 N1E baseline test names are present. The
production kernel and release VFS service compile.
[Machine-readable results](vfs-authenticated-device-admission-results.json)
record the commands and evidence.

The unchanged ownership guard still fails with 71 references and the same
four unmatched findings: statfs, ProcFS mountinfo, private mmap and runtime
FAT mount. All remain open. No real storage ownership transfer, QEMU
qualification, production routing, or `/vfs-scratch` activation took place.
L1C-3 stays blocked, VFS-D1 deferred, and demand-backed main stacks opt-in.
