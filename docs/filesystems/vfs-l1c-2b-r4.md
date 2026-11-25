# VFS-L1C-2B-R4: Delayed completion after cancellation

**R4 native regression: PASS.** The final fixture completed eight transactions each on 1, 2, and 4 vCPUs at 2 GiB. An earlier two-vCPU build completed eight more transactions before teardown counters were added. The broader VFS-L1C-2B phase remains PARTIAL; VFS-L1C-3 remains blocked.

The previous fixture had two cancellation modes. Mode 1 held the request in `next_control` until an observer ran and cancelled it before service delivery. Mode 2 allowed normal delivery and consumed the successful retain before cancelling. Neither had a checkpoint between acquisition of the lease by the service and processing of the acknowledgement by the kernel.

Mode 4 adds that checkpoint in the debug-only fixture. In `transport.rs::control_runtime`, the ordinary copied reply passes the boot service identity, registration and endpoint checks before the fixture takes an immutable copy of the matching reply. The copy contains the actual retained receipt; no userspace service code changed. The service generates that receipt only after `policy.rs` acquires the directory pins through the normal retention path. The scheduler poll selects the existing cancellation result and wakes the parked caller. The adapter takes its record once and calls the existing private child cancellation path. After local teardown the fixture rechecks the same service and registration, then gives the original bytes to `Runtime::control_reply`. The registration owns compensation and settlement throughout. No lock holder is paused at the checkpoint.

The only kernel changes outside the adapter fixture are feature selection, the copied reply checkpoint and debug teardown diagnostics in the existing rollback and pending child paths. The build embeds separate fixture init, which selects only the R4 loop. The two ordinary spawn entry points are unchanged. No production lifecycle defect was reproduced or repaired.

| Point | Example: child 5, request 2, service 3, epoch 1 | Owner and disposition |
| --- | --- | --- |
| A | Before construction | Adapter and reply buffer empty; no resources assigned to this transaction. |
| B | Genuine retained reply held | Private reservation and PID charged; adapter owns the private image. Registration owns the delivered retain and continuation. The service holds the directory lease. The child has three local initial stream descriptions; this fixture requests no inherited descriptors. Staging borrows the parent's existing receipt for derivation; no child binding is installed. |
| C | Cancellation terminal consumed; adapter disposed; child torn down | Three descriptor records and their references removed through rollback. VM teardown reports 551 user frames and eight page tables reclaimed, with zero populated swap slots. The child process box and boxed descriptor table are dropped, along with the owned fields of the process. The private payload and continuation are gone. The pending PID claim, delivered retain, and service lease remain charged against the original registration. |
| D | Original reply reaches normal completion | Same registration row; the delivered marker and request preparation clear. The correlated receipt enters retryable release. No continuation remains to wake or return another result, and this route uses registration data after the private payload is gone. A publication attempt after cancellation is rejected. |
| E | Service release and caller barriers settle | The normal copied channel accepts the service's correlated release and lifecycle acknowledgements. The pending table removes the reclaimed child claim when the retain and caller records are absent. The live service registration remains assigned to the same service for unrelated work. |

The private child was absent from the process table before waiting and after cancellation. The checker found no insertion, enqueue or first dispatch for any of the cancelled child IDs, and exactly one adapter disposal and settlement per request. A separate observer dispatched while init was parked, exited, and was reaped with status zero. The serial teardown counters cover the identified child heap owners, not an allocator census of every nested allocation or byte. No populated swap teardown or active registration replacement was exercised.

| 2 GiB | Native R4 | Ordinary Phase 3.0 |
| --- | --- | --- |
| 1 vCPU | PASS (8) | PASS |
| 2 vCPUs | PASS (8; 8 earlier pilot cycles) | PASS |
| 4 vCPUs | PASS (8) | PASS |

The existing 104 registration tests passed, including the 112-cycle ownership test and the continuation cases. Transaction/session and mmap compile checks passed. SWAP-1 passed via the unchanged gate, which selects four vCPUs. The unchanged ownership guard reports zero candidate findings and the same four primary findings. Source equivalence covers 12,346 baseline paths, the eight R4 source files, and the previously documented candidate reductions; it reports no unexpected drift.

Evidence lives under `target/vfs-l1c-2b-r4/`. The final serial logs are `stripped-{1,2,4}.serial.log`, and `native-check.json` contains the per-transaction line numbers. The pilot is preserved in `initial-2.serial.log` and `initial-check.json`. Some short unstripped launches ended before producing serial output. The boot display showed Limine loading the ELF, and a longer diagnostic launch reached kernel initialization. The final matrix used an ELF with debug sections stripped and a longer launch budget. See the [machine results](vfs-l1c-2b-r4-results.json) for exact evidence paths.

R1, R2, R3 and native R5 remain unexecuted. No new Btop or Qalculate interaction tests were run. Fault-driven swap-in remains outside this witness.
