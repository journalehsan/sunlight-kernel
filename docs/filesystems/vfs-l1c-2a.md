# VFS-L1C-2A publication transaction closure

**Verdict: PARTIAL. VFS-L1C-3 activation remains blocked.** The ordinary boot gate is restored, and the dedicated publication fixture passes separately. Neither ordinary spawn entry point was changed. Native retirement, late replies, and complete transaction allocator accounting remain open.

## Ordinary candidate boot

The archived failing Phase 3.0 run at 2 GiB and 2 CPUs reached Foundation Complete, registered VFS, and completed init's root metadata request (`status=0x0`). Init then invoked syscall 61889 three times and logged `[VFS-L1C-1] init observed failure`. Syscall 61889 is the debug fixture entry; the ordinary kernel rejected it as unknown. The L1C-2 fixture init issues two cancellation calls before the success call, then asserts on the returned observer PIDs. An unknown syscall cannot stage a continuation, reserve a private child, or leave a fixture request outstanding. The trace ends before ordinary service startup and the later native stat probes. The assertion is the source-level explanation for the stop; the trace contains no separate assertion diagnostic.

| Exact missing marker | Emitter |
| --- | --- |
| `timezone: zone=` | `services/timezone_service/src/main.rs` |
| `[USB-MOUSE] boot mouse ready` | `services/sunlight-usb-mouse/src/main.rs` |
| `[VFS-RPC] native root stat caller resume/result copy OK transport=copy` | `services/init/src/main.rs`, later root stat check |
| `[VFS-RPC] native path stat traversal/provenance/errors OK transport=copy` | `services/init/src/main.rs`, later traversal check |
| `[VFS-RPC] native StatPath service routing/ABI/errors/copy recovery OK` | `services/init/src/main.rs`, native StatPath probe |
| `[VFS-RPC] Linux stat service routing/ABI/errors/copy recovery OK` | `services/init/src/main.rs`, Linux stat probe |

The historical run shows an artifact mismatch, not an ordering failure in the transaction. Its exact Cargo cache or staging transition was not recovered. The starting shared init artifact matched the earlier fixture SHA-256. The first unchanged gate replay rebuilt ordinary init and passed; it did not reproduce the historical failure. Compare `target/vfs-l1c-2/ordinary.log`, `target/vfs-l1c-1/ordinary-final.serial.log` (last passing baseline), and `target/vfs-l1c-2a/ordinary-replay.serial.log`.

The kernel now builds fixture init in a feature-specific Cargo target and embeds it from its own build output. Ordinary builds continue to use the ordinary init artifact. The unchanged gate passed at 1, 2 and 4 CPUs with 2 GiB after artifact separation. No expectations or timeouts changed.

## Adapter and transaction ownership

The debug adapter keeps one non-cloneable `Child` and an exact copied `RemoteClaim` across retry. The child owns the local private payload; the scheduler indexes the reservation and PID. Registration owns the continuation, provisional receipt, delivery records and outstanding remote work. The observer is an ordinary scheduler process, separate from the private child.

- Success removes the continuation and moves the payload and PID into the scheduler once. The bound lease remains with registration. A later local cancel or drop cannot retrieve the committed payload.
- Unsent cancellation discards the queued retain. Cancellation after receipt reclaims the child and hands the lease back to registration for release. Both fixture paths check that a second cancel and publication fail.
- Failed publication now reclaims the child and consumes its terminal continuation rather than panicking at the publication call. The existing syscall-return cleanup can take the exact fixture owner if its parent image is gone; it drops the owner outside adapter and scheduler guards. The runtime's existing poll separately drops the dead caller's continuation. The patch does not establish that the cleanup hook runs if no further syscall returns.
- Refusal and retirement use the existing registration terminal results. A live caller can consume the result; dead callers are removed by the existing poll. The fixture's expected-outcome assertions still constrain cancellation modes; arbitrary refusal or retirement in those modes has not been qualified.
- A late response can settle or compensate for a registered request. Local disposal leaves uncertain work and any associated PID claim charged. Duplicate local cancellation cannot extract the payload again; no extra cleanup authority was added.

The final fixture exercised ordinary success and both cancellation modes, but did not trigger the new orphan or failed preparation branch natively. Host regression exercises failed preparation, parent loss and retirement in the actual registration runtime with a modeled scheduler.

## Qualification

| 2 GiB | Ordinary Phase 3.0 | Dedicated publication fixture |
| --- | --- | --- |
| 1 CPU | PASS | PASS |
| 2 CPUs | PASS | PASS |
| 4 CPUs | PASS | PASS |

The fixture shows authenticated binding → scheduler insertion → enqueue → first dispatch → metadata completion for `stat /tmp` → normal exit → reap. The cancelled children never entered the process table or ready queue and their PID claims settled separately. Exact traces are `target/vfs-l1c-2a/native-final/continuation-{1,2,4}.serial.log`; ordinary traces are `target/vfs-l1c-2a/ordinary-{1,2,4}.serial.log`.

| Witness | Native | Host evidence |
| --- | --- | --- |
| R1 retirement before acknowledgement | NOT_RUN | Terminal retirement and retained remote ownership |
| R2 retirement after acknowledgement before publication | NOT_RUN | Final publication rejects retired authority |
| R3 publication before retirement | NOT_RUN | Scheduler-owned process survives registration retirement |
| R4 late acknowledgement after cancellation | NOT_RUN | Authenticated copied reply reaches compensation with no second wake |
| R5 duplicate and stale responses | NOT_RUN | Exact correlation, duplicate decisions and modeled slot/epoch reuse |

## Resource and regression evidence

The 112 mixed host publication cycles now explicitly check removal of exact continuation rows, private payload disposal, one modeled enqueue on success, and one release obligation plus a pinned PID on delivered cancellation. The per-cycle runtime is freshly constructed; this test does not measure allocator drift across repeated cycles against one live service. It does not require remote rows to reach zero while work is still owed. No complete native adapter, continuation, registration, binding user, descriptor reference, heap, frame, page table or swap census was taken at publication acquisition and cleanup.

The separate native L1P construction/cancellation regression passed 100 cycles per CPU count. Its pre/post allocator and descriptor baseline and zero swap candidates apply to that regression. The published fixture child's reaper reported 551 user frames, 8 page tables and 0 swap blocks; these are child teardown counts rather than transaction allocator deltas. No swap, mmap or PMM path was changed and no VFS call was introduced into swap-out or fault-driven swap-in. Host ZRAM and swap-slot regressions and the native SWAP-1 pressure gate at 2 CPUs and 2 GiB passed. Fault-driven swap-in remains NOT_RUN; the direct-call SWAP-1 evidence does not qualify it.

All 104 registration host tests passed, including the strengthened 112-cycle test, continuation regressions and the two KLC 100-cycle tests. IPC, VFS service, descriptor, scheduler, kernel host and resource retirement suites passed. Transaction/session and mmap checks passed. The unchanged guard reports 71 primary references and the same four findings, and 68 candidate references with zero findings. The candidate comparison covers 12,346 baseline paths with zero unexpected drift and preserves the existing reduction overlays. Exact log paths and status are in `vfs-l1c-2a-results.json`.

## Remaining decisions

Keep both ordinary spawn paths disabled. Native R1–R5, fault branches of the debug adapter, and a transaction-specific native resource census still need witnesses before promotion. Endpoint-only retirement can retain obligations indefinitely if the old service cannot send a proof; elapsed time does not authorize reclamation. Runtime VFS replacement remains unsupported and requires a separate project-owner decision about successor provenance and recovery. No replacement authority, forced retirement completion, or waiver of a native gate was introduced.
