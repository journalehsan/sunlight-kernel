# SFS-5A native block contract

This is the admission contract for a future SunFS native adapter. It is not an
assertion that the current native `sunlight_block::BlockDevice` implements it.
The existing `sunlight_sunfs::block` types already encode most data-plane results;
no second production state machine or native SunFS mount is introduced.

## Operations and admission

`info()` returns checked byte capacity, logical sector size, optional physical
sector size, transfer alignment, maximum transfer, read-only/flush/FUA capabilities,
and a nonzero media incarnation. Identity is queried before admission and pinned
through each operation. An unavailable hardware identity must be reported as absent,
not fabricated from capacity, PCI position, a label or a content marker. A kernel
registry ID is boot-local routing identity, not a persistent hardware serial.
Writable admission requires exclusive device/partition authority, a pinned media
incarnation, supported geometry, truthful flush, and a qualified ownership/fence
policy. Native registry grants and VFS mount authorization remain mandatory.

`read(offset, output)` validates checked bounds/alignment and completes the entire
range before exposing bytes. Native single-sector errors become read errors and
leave caller output unchanged. `write(offset, input)` reports completion certainty
and any known completed prefix. `flush()` drains the admitted writer's prior writes
through every cache and transport. `write_fua()` is optional and explicitly
unsupported unless actually implemented. No implicit successful flush or FUA.

| State | Required representation / interpretation |
| --- | --- |
| Completed read | All requested bytes validated and copied; no durability claim |
| Completed write | `WriteCompletion::Completed`; DMA/transfer completed, durability unknown |
| Durable write | `Durable` only with a qualified stable-write/FUA contract |
| Durable cut | Successful flush after all dependent writes completed |
| Partial completion | `IoFailure.completed=Some(n)` only with authoritative prefix evidence; remainder certainty retained |
| Failed operation | `Certainty::Known` only when outcome and side effects are known |
| Uncertain completion | `Certainty::Uncertain`, usually `completed=None`; freeze mutation |
| Unsupported | `UnsupportedDurability` for flush/FUA; no submission, no success |
| Unavailable | `Offline` with retained incarnation; old uncertain writes remain uncertain |

The legacy native trait has only `Io`, `OutOfRange`, and `Unsupported`. Its VirtIO
boolean API loses timeout/device-status distinctions. A future adapter must map
every ambiguous `Io` write/flush to uncertain, never known-zero completion. A
successful cache `write_block` means software acceptance, not device completion.
Eight sequential 512-byte sectors do not become an atomic 4096-byte transfer.

## Ordering and visibility

1. Within a request, descriptor header/data/status follow transport layout; no
   atomic write assumption, including a single sector. A partial failure may have
   modified any subset unless the backend proves a completed prefix.
2. Under exclusive serialized submission, finish one transfer before publishing
   the next. Submission order, device execution, completion and durability are
   distinct. Serialization alone does not order persistence.
3. A flush is submitted after all covered device writes complete. Its acknowledged
   scope is all prior accepted writes on this device, including dirty cache slots
   written before the inner barrier. Partition flush forwards to the parent;
   unrelated writers must not race this scope.
4. Read-after-write may be served from a software or device cache. It proves
   visibility only. Dirty cache eviction also proves no stable-media cut.
5. Volatile device/host caches are allowed only with an honest flush chain. QEMU
   cache configuration is part of qualification; neither CPU fences nor queue
   completion substitute for a storage flush. Cache modes cannot be generalized.
6. FUA is unused. Even checkpoint FUA could not replace flushing dependencies.
7. A completion must identify the single published descriptor head and exactly
   one next used entry before status/data is consumed. A status error does not
   prove absence of write side effects. An impossible/unsolicited entry poisons
   the queue. No new request is allowed to reinterpret an uncertain old completion.
8. CPU release/acquire/full barriers govern coherent x86 DMA visibility. This
   does not qualify noncoherent platforms, an IOMMU teardown, or physical storage.

## D/P/R/R0

After any needed R0 (zero the oldest checkpoint and acknowledge its flush), write
the complete new dependency closure and wait for every transfer. Acknowledge flush
D before submitting the new checkpoint. Complete the checkpoint write, then
acknowledge flush P. Only then is the new checkpoint durable within the qualified
device model. Zero the oldest slot and acknowledge flush R before releasing its
closure; last runtime pin and request quiescence are additional prerequisites.
When recovering three checkpoints, R0 must finish before reusing the oldest slot.
On any failed/partial/uncertain write or flush, latch the volume error, stop all
mutations, preserve reservations/retained closures and report the cause. A failed
R can coexist with a durable P; never claim that the new checkpoint is absent.
No blind retry or writable remount while the old transport can still act.

## Ownership and authoritative fencing

Conceptual lifecycle: CREATED → PREPARED → SUBMITTED → COMPLETED → RETIRED.
Timeout instead enters TIMED_OUT_UNCERTAIN. Only an actually supported quiescence
operation can enter FENCING → FENCED → RETIRED. A timeout is not cancellation;
writing a reset register, dropping a handle, or observing a quiet interval is not
a fence. A fence receipt must prove every prior request completed or is incapable
of future DMA/access, bind the device/queue generation, and make all late entries
incapable of retiring new ownership. Media side effects can remain uncertain even
after DMA has been fenced; recovery is still required.

Current VirtIO policy after the repair is terminal quarantine until reboot, with
the queue and bounce page allocated for the kernel lifetime. No reset, drain,
generation reuse, DMA unmapping or retirement is authorized. Bounded failure and
retention preserve safety but do not establish a usable native fence. USB xHCI
also pins allocations and marks a timed-out transport dead; BOT reset is not a
general proof of DMA fencing. Hosted simulator session reset is a model operation
and cannot stand in for either native transport.

No I/O wait may hold a scheduler-critical lock. The future SunFS integration must
pin device/mount/object authority, release outer VFS/scheduler locks, perform the
bounded operation and revalidate identity. Existing synchronous VFS→device polling
does not establish that requirement. Poll iteration limits are finite work bounds,
not calibrated wall-time or scheduler-progress proofs.
