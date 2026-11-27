# VFS-N1E exclusive device lease and drain qualification

The host lease authority in `sunlight-vfs-server` owns one whole fake
device. It uses the existing VFS RPC registration identity and nonwrapping ID
allocator. The N1D `DirectoryNamespace` and shared VFS remain the mount
authority. The two are independent in this milestone. The lease module is
excluded from the freestanding target.

## Lease and operation contract

| State | Admission and ownership |
| --- | --- |
| Vacant | One trusted acquisition can establish a registration and lease generation. Competing acquisitions fail while any lease or fence occupies the device. |
| Active | Only the exact authority, registration and lease generation can admit reads and writes. A request names one in-range block and consumes one queue credit with a unique completion ID. Both reads and writes close at quiesce. |
| Quiescing | The predecessor remains recorded and the prospective successor has no access. Accepted requests retain their queue entries and can complete normally. Starting the durability barrier returns busy until the last accepted request settles. |
| Flushing | The fake transport owns one barrier credit. It copies the visible image to the durable image on confirmed success. A successful data write alone only changes the visible image. |
| Synchronized | Confirmed drain and durability let the coordinator commit the transition. Transfer replaces the incumbent and activates the successor under one lock. Release drops the incumbent and returns to vacant. |
| Cancelling | A cancelled transition with an outstanding flush keeps admission closed until definitive settlement. Cancellation before submission or after confirmed synchronization resumes the incumbent immediately. |
| Retiring | Retirement permanently closes the incumbent generation and discards its successor reservation. Accepted operations settle normally. A confirmed barrier already completed or in flight can satisfy retirement. A definite flush failure keeps retirement closed for retry. Confirmed synchronization releases the device. |
| Fenced | Ambiguous I/O, ambiguous flush or transport loss latches a reason and preserves the last lease identity for diagnosis. No generation can admit new operations and no acquisition or handoff can succeed. Outstanding credits remain charged until terminal settlement. |

A normal definitive flush failure returns the same owner and generation to
active and invalidates the transition. This outcome represents a transport
that is still usable with well-defined ownership. A retry needs a new
transition and successful barrier. Cancellation during a flush uses the same
rule when the completion is definitive. Ambiguity stays fenced even after
every credit is reclaimed. The fake accepts an explicit ambiguity outcome
only at terminal settlement. An earlier transport-loss notification leaves
the affected queue entries charged. A later successful completion can finish
accepted old work while the fence remains in force. The fake controller can
apply a write or barrier and then report ambiguity, exercising the lost-reply
case. Successful reads sample the visible block when the controller completes
them.

Each device instance contains its own transport and images. Whole-device
reservation covers every block in that instance even when contenders target
different offsets. Production must canonicalize physical aliases and
overlapping extents before admission. The host coordinator supplies trusted
registration identities for acquisition and transfer, controls transport
completion, and reports retirement against the exact incumbent lease. In
production, the kernel must verify incumbent and successor registration
liveness, including successor retirement before commit, and bind the lease
to the media incarnation and negotiated rights. The host fixture has fixed
media and read/write rights. Flush is a coordinator action.

Tokens carry opaque device scope and checked serials. The transport only
applies a completion while its exact token is pending. A token consumed
during drain cannot consume a newer credit, modify media bytes, or fence the
successor. A retirement or transport-loss event carrying an old lease fails
the same identity check. At serial exhaustion, operations requiring a fresh
serial fail closed. Credits track up to four retained data commands and one
flush, with no data commands pending when a flush starts. Removing a queue
entry reclaims its buffer and credit exactly once. A fence reserves the device
after queue reclamation. The module has no in-place recovery from ambiguity.

All transitions and fake media effects are bounded critical sections on the
single device lock. The coordinator observes busy and returns to drive drain
and barrier completion from outside that lock. The implementation acquires
no namespace lock and waits for no completion while holding the device lock.
Backend cache writeback is outside the fake. In the production sequence, first
freeze caller admissions and enqueue filesystem writeback under the
predecessor's authority, then close device admissions, settle accepted I/O,
and confirm a transport durability barrier.

## N1D attachment integration contract

1. **Stage and establish readiness.** Reserve the target against the existing
   mount authority and keep the target fenced during handoff. Check the
   predecessor's retained root and cwd pairs, dirfds, file descriptions,
   cursors, mapping population and pending calls. Complete necessary close and
   writeback before revoking those paths. After exclusive transfer, probe
   read-only using a fresh backend/cache and verify geometry, media,
   root object, and capacity. Bind the readiness receipt to the attachment
   transaction, authenticated registration, normalized extent, media and lease
   generations, and rights. Keep the receipt valid through publication.
2. **Publish under the topology fence.** Revalidate the N1D owner, target
   identity, mount slot, and base topology revision at the insertion point.
   Use a bounded lease/readiness pin covering the actual insertion so
   retirement or media loss cannot race the check. Commit the complete
   backend, crossing, source, and mount incarnation in the same revision.
   Existing caller-view and snapshot installation fences still apply.
   A copied ProcFS snapshot owns immutable rows. Eligibility for its
   installation and later use follows the proposed N1A contract.
3. **Compensate for failed publication.** A stale revision, target, or
   readiness receipt leaves no new crossing or revision. Dispose of an
   untouched reservation directly. After transfer, keep the target fenced,
   revoke and drain the unpublished backend, settle its transport, and prove
   unchanged media and absence of staged writes before regrant. If the
   predecessor backend was destroyed, reconstruct and reprobe it with a
   fresh generation and cache before making it visible again.
4. **Withdraw before device reuse.** Freeze new crossings and grants for the
   exact mount incarnation and keep close and settlement available. Normal
   withdrawal returns busy while retained roots or descriptions still pin the
   attachment. Preserve description identity and shared offsets and let
   PREAD use its explicit offset. Drain accepted work, perform writeback and
   confirm the durability barrier. Withdraw the row and crossing and advance
   the topology revision under the mount owner fence. Reclaim the lease and
   backend only after mount/reference retirement, verified transport and DMA
   settlement, and durability. A normal unmount may reveal the underlying
   directory. Loss and handoff keep it fenced.
5. **Recover explicitly.** If drain, revocation, media identity or write
   outcome cannot be proven, keep the extent and target fenced. Settle or
   reset transport and require media validation and any repair before an
   explicit remount with fresh registration, lease, backend and mount
   identities. Namespace withdrawal and queue reclamation each leave the
   device fence in force until such recovery.

The coordinated production candidate still needs kernel capability checks,
transport/DMA and buffer quotas, alias and media fencing, device-backed FAT
and retained-object adapters, capacity and topology publication, authenticated
caller binding before dispatch, the kernel ProcFS FD-install fence, descriptor
epoch and compensating Close repairs, and loader/mmap coverage. Production
routing, real FAT transfer and `/vfs-scratch` stay inactive. L1C-3 remains
blocked, VFS-D1 remains deferred, and demand-backed main stacks remain opt-in.

## Qualification

[Machine-readable results](vfs-exclusive-device-lease-results.json) record
the focused host checks, regression counts and build commands. The completion
tests drive queue and barrier settlement explicitly. Thread barriers coordinate
competing callers. The repeat test runs 64 acquire-transfer-release cycles
and verifies zero outstanding data and flush credits on each release.

The unchanged ownership guard still reports 71 references and four unmatched
live findings in Linux statfs, ProcFS mountinfo, private mmap, and runtime
FAT mount. All four remain open. The guard blocks ordinary native validation.
Fake durability and transport settlement are host evidence. Real media,
production cache and DMA behavior require separate qualification.
