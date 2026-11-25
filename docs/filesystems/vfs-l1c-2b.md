# VFS-L1C-2B native transaction lifecycle

**Verdict: PARTIAL. VFS-L1C-3 remains BLOCKED.** The sustained native fixture passes. Native R1–R5 and the full transaction resource census remain open. Keep transactional publication disconnected from both ordinary spawn entry points.

The changes are confined to the existing debug fixture: `services/init/src/main.rs` runs 34 repetitions of unsent cancellation, cancellation after authenticated retain, and successful publication, waiting for the existing observer or child to exit before the next operation. `kernel/src/ipc/vfs_registration/spawn.rs` logs adapter acquisition and disposal on the exercised paths and failed publication branch. There are no production behavior changes, new cleanup authorities, capacity increases, or boot policy changes. The existing separation of ordinary and fixture init remains in place. The historical artifact mismatch is documented in L1C-2A; the staging failure has not been reproduced or conclusively explained.

| Native case, 2 GiB | 1 vCPU | 2 vCPUs | 4 vCPUs |
| --- | --- | --- | --- |
| Dedicated L1C-2 fixture | PASS | PASS | PASS |
| Mixed transactions | 102 | 102 | 102 |
| Adapter acquisition / disposal | 102 / 102 | 102 / 102 | 102 / 102 |
| Cancelled claims settled | 68 | 68 | 68 |
| Published children dispatched, exited and reaped | 34 | 34 | 34 |
| Separate L1P construction/cancellation | 100 | 100 | 100 |
| Destructive retirement guests | 0 | 0 | 0 |

The mixed count excludes the 112 host cycles and the standalone L1P cycles. Each guest used a single VFS admitted by the existing boot policy. The observer must dispatch with the caller parked. In mode 1 the transport holds the queued retain for the observer, and the cancellation hook completes the request before service delivery. In mode 2 the service replies through the authenticated path before the adapter cancels and consumes the cancellation terminal; the release can settle after the next transaction begins. Success gives the scheduler the child and records binding → insertion → enqueue → first dispatch → authenticated `stat /tmp` completion → exit → reap. These are deterministic order witnesses on UP and SMP; they do not measure simultaneous execution in critical sections.

The per-request check correlates acquisition, terminal owner, and cancellation settlement by child and retain request, and matches scheduler events by child PID. Each cancelled child has one disposal and settlement record and no publication or first dispatch marker. The cancellation path checks a second cancel and a second publication attempt fail. Success checks that the consumed child cannot be cancelled, and each published child reaps once with 551 user frames, 8 page tables and zero swap blocks. This covers duplicate local cancellation; transport duplication and stale replies remain separate tests.

| Adapter branch | Native result | Ownership and coverage boundary |
| --- | --- | --- |
| Success | PASS | Takes the adapter slot; scheduler owns the child and registration keeps the bound lease. |
| Unsent cancellation | PASS | Takes the slot and cancels the private child. The queued obligation and PID claim settle through registration. |
| Cancellation after authenticated retain | PASS | Takes the slot, cancels the private child and drains the continuation. Registration owns release until settlement. |
| Failed publication preparation | NOT_RUN | Existing branch takes the slot, cancels the child and drains the terminal if present. Fixture did not reach a preparation rejection. |
| Service refusal | NOT_RUN | Live caller can consume refusal and dispose of the still-private child. No authentic refusal was arranged for this valid retain. |
| Parent termination | NOT_RUN | Existing syscall-return hook can take the orphan adapter and drop it outside the guards; registration polling drops the dead caller's continuation and retains outstanding obligations. PID 1 stays alive in the exercised fixture. Cleanup without another qualifying syscall return remains unproven. |
| Service retirement | NOT_RUN | Registration can return authority retired; no transaction guest retired the boot-trusted service. |
| Late acknowledgement | NOT_RUN | Existing mode 1 cancels before delivery. It cannot serve as the authenticated delayed-reply witness. |
| Duplicate terminal processing | PASS for local cancel; NOT_RUN for transport | Fixture checks that the adapter cannot extract the payload twice. No replay was delivered by the service. |

| Retirement witness | Native status | Missing boundary |
| --- | --- | --- |
| R1 | NOT_RUN | Separate fresh-guest cases with retirement before service acquisition and after acquisition but before acknowledgement. |
| R2 | NOT_RUN | Authenticated result consumed, then retirement at a synchronization boundary before the normal final authority check. |
| R3 | NOT_RUN | Publication and first dispatch pass in the sustained fixture; retirement afterward and subsequent runtime authority loss were not exercised there. |
| R4 | NOT_RUN | Delayed authenticated reply from the same still-authorized service after cancellation and local cleanup; observe compensation and absence of a second wake. |
| R5 | NOT_RUN | Production duplicate, stale request identity, stale creation identity, incorrect epoch, and post-reuse transport rejection under legitimate fixture identities. |

KLC-Q1 evidence under `/tmp/klc-q1/evidence/native/` covers exact-image exit and exec, endpoint withdrawal, a refused late reply after withdrawal, and binding drain. Those are separate witnesses, not R1–R5 passes. An endpoint-only retirement can retain a charged obligation until valid evidence arrives. A valid late reply for R4 requires the original service to remain authorized. An R5 subcase that depends on a new trusted VFS occupying a retired slot is **BLOCKED** on successor provenance approval; eligible slot reuse within the existing registration contract still requires its own native witness. R1–R4 do not require a replacement service.

| Boundary | Transaction resource ledger | Evidence and limit |
| --- | --- | --- |
| A: before acquisition | Adapter slot available; private reservations, continuation and request not yet assigned to this transaction. | Existing adapter slot check; no per-transaction allocator baseline. |
| B: construction | Child owns its private payload, reservations and VM; scheduler indexes its reserved identity. | Acquisition trace is emitted after staging, so construction and staging lack separate snapshots. |
| C: request staged | Registration owns continuation, request and remote retain; scheduler still holds reserved PID. | Exact request and epoch logged; private child absent from process table when parked. No complete registration-slot census. |
| D: outcome wins | Successful binding user and lease belong to registration; scheduler receives payload/PID and enqueues once. Cancellation leaves the child unpublished and hands eligible release to registration. | Per-request transitions and negative second terminal operations; local binding user counts were not measured. |
| E: transfer or local cleanup | Success consumes the continuation and transfers the child, its descriptor table and VM to scheduler ownership. Cancellation removes the private payload and relinquishes its locally reclaimable resources through the existing path. | Exact disposal and private slot checks; no native per-child heap, inherited descriptor/reference or cancelled VM census. |
| F: settlement and reap | Registration settles all 68 cancelled remote claims per guest. Reaper owns published child cleanup. Retired but unproven obligations must remain registered. | One matched cancelled-PID settlement and one reaper record per published child; no direct per-request census of released publication leases, descriptor references, heap, or cancelled frame/table counts. |

The ledger establishes ownership transitions for exercised paths but does not close all requested resource classes. Zero swap blocks on the published child reaps does not exercise reclaim with populated swap metadata. No swap architecture, mmap, PMM, or fault path changed. Fault-driven swap-in remains NOT_RUN.

The machine report lists gate results and exact paths. Native serial logs, QEMU launch records, build and ISO logs, and the correlated request check live under `target/vfs-l1c-2b/native/`. The existing candidate reduction overlays were preserved; the source comparison covered 12,346 baseline paths with zero unexpected drift. It establishes equivalence for the compared production paths subject to the documented candidate reductions, not equivalence of the full primary and candidate images.
