# KFP-2A.3 — Retirement lifecycle closure

**Verdict: PARTIAL. Production runtime behavior changed.** Durable retirement ownership now covers release-old-frame replacement, final SHM backing handoff, and partial page-table creation rollback within the scopes below. Full rollback transactions, pending SHM views, retired executable images, concurrent failed cleanup, and complete independent resource accounting remain open. FI-6/FI-7 and the full CPU/security matrices do not satisfy acceptance. Production FastLane remains disabled. No automatic APIC recovery, background reclamation, commit, or push was implemented.

## 1. Baseline and source integrity

HEAD remains `882b40a65f55951b5cd8b9978c05580e71202488`. The incoming tree already contained uncommitted KFP-2A/2A.1/2A.2 changes and an unfinished KFP-2A.3 implementation, report, and evidence. The original milestone baseline is in `.kfp-2a3-evidence/status-before.txt`, `preexisting.patch`, and `source-before.json`. Its initial host suite passed 194 tests; the incoming KFP-2A.3 host suite passed 201 tests again in this continuation. These are different source snapshots.

`continuation/status-before.txt`, `incoming.patch`, and `source-before.json` capture the continuation baseline. Only three production source files were modified during continuation: `process/address_space.rs`, `memory/tlb.rs`, and diagnostic blocks in `memory/security.rs`. Other incoming production changes were preserved. Final pilot source-manifest SHA-256: `cee109b4668a479acd7d6f9f155326dd27428ba0150445fb2ded2fdf0062906f`. `source-integrity.json` records final source comparisons; `continuation/source-build.json` binds the six fresh pilot builds, with ELF/ISO/serial hashes in `continuation/guest-results.json`. Source manifests distinguish production/build inputs from generated evidence snapshots. Native wrappers exclude evidence directories from source discovery while retaining the original assessors, expected results, timeouts, and security checks.

KFP-1 through KFP-2A.2 reports, the prior results/fault-results JSON, and relevant source transitions were reviewed. Prior reports/results remain historical inputs, including the nine KFP-2A.2 terminal-fault guests; they do not validate this final source. The earlier KFP-2A.3 guests are also earlier-source evidence after the continuation changes.

Preserved attempts:

* Original `attempt-1/`: delayed-ACK guest lacked actual remote targets. Its normal gate pass did not satisfy the remote evidence requirement.
* Original native attempt 1: source discovery accidentally included newly generated evidence snapshots; failed before guest execution.
* Original native retry: QMP socket bind denied by sandbox; no guest security result.
* `continuation/attempt-1/`: new guest stopped on an incorrect fixture expectation (`FrameAllocationFailed` instead of the actual `PageTableAllocationFailed`). The complete failed source manifest, image, log, and diagnosis remain. The corrected test requires the exact table-allocation error; no production error or security assertion was weakened. This attempt used a range flush; final table rollback uses the full-flush adapter.
* `continuation/audit-results-attempt-1.log`: reconciliation initially compared a broad baseline capture against a narrower source filter, incorrectly classifying omitted non-source resources as changes. The corrected checker hashes all captured baseline files directly and requires exactly the three actual changed sources. No guest/test expectation changed.
* Fresh native guest attempts and all diagnostic output remain under `target/kfp-2a3-continuation-{m2,m4,m5}/`; their results are indexed in `continuation/results.json`.

## 2. Retirement ownership inventory

`ownership-inventory.json` describes 16 identified transition families with original/new owner, exact transfer point, capacity, frame/backing identity, VM/resource generation, required targets, cleanup, failure/retry, and unwind behavior. `withdrawal-sites.txt` retains the initial discovery list. This is not an exhaustive whole-kernel reachability proof.

| Transition | Authoritative owner and boundary | Final status |
|---|---|---|
| Anonymous munmap | Expected leaf/swap marker → static journal before clear → PMM/ZRAM after exact synchronization | Retained KFP-2A.2 implementation; detached empty-table pruning remains separate |
| CoW/swap-out `ReleaseOldFrame` | One old physical frame staged before replacement PTE store; journal persists on failed synchronization | Implemented in initial KFP-2A.3 |
| MAP_FIXED `RetainOld`, commit/rollback | Old leaves and installed replacement obligations remain in transaction vectors/snapshots | OPEN |
| Swap-in retained compressed backing | Generation-qualified ZRAM arena owns payload; stack locator links the pending operation | OPEN operation identity |
| Installed-leaf rollback | Caller owns expected backing; detached leaf/table obligations still have local intervals | OPEN |
| Partial-created-table rollback | Parent graph before unlink; journal owns all created tables after unlink; exhaustion preserves parent graph | Implemented in continuation; local/inactive guest proof |
| Empty-table pruning | Parent graph until unlink; detached table/cache lifetime lacks a complete independent proof | OPEN |
| SHM pending views | Broker owns frames/quota; process retains mapped-view metadata; pending invalidation itself is not journaled | OPEN |
| Forced SHM revocation | Broker fences admission; actual views must drain before final backing release | PARTIAL; pending-view gap remains |
| Final `ReclaimedShared` handoff | Existing retired broker row retains exact vector and quota; returned token is only a locator | Implemented in initial KFP-2A.3 |
| Quota reserve/commit/refund | Move-only identity-qualified reservation moves into broker; refund follows final physical release | Scoped final handoff covered; failed mapped rollback OPEN |
| Process-exit SHM cleanup | Views remain tracked until verified free completes; reaper does not drain tracking first | Pending-operation and post-failure lifecycle proof OPEN |
| Retired executable image | `mem::replace` leaves old AddressSpace on exec stack before activation/reclaim | OPEN |
| Inactive address-space destruction | Exclusive root graph, zero active CPU mask, no outstanding same-VM journal record | PARTIAL; complete activation exclusion and detached intervals not proved |
| Permission withdrawal | Physical owner remains attached; permission-operation metadata remains local | OPEN operation; frame-release adapter unsuitable |
| Kernel VMM unmap/flags | Caller/kernel mapping ownership, local-only invalidation helper | Not generalized; production reachability of unused unmap helper unproved |

Source anchors: `process/address_space.rs::{replace_mapping,rollback_mapped_page,rollback_created_tables,reclaim_empty_tables_for_page,reclaim_user_space}`, `process/mmap.rs::{rollback_anonymous,sys_munmap}`, `process/spawn.rs::exec_into_process`, `memory/shared.rs::{free_shared_page,cleanup_shared_pages}`, `capability/mod.rs::ReclaimedShared`, and `memory/shared_quota.rs::Reservation`.

## 3. Shared architecture and implemented changes

The existing allocation-free `OWNERS` journal remains the common synchronization/physical-retirement engine. A non-Clone generation-qualified token is a locator, not the authoritative owner. Dropping it does not erase records. Root, VM generation, slot generation, shootdown sequence, target mask, state, exact backing entries, and per-entry release receipts gate reclamation. Failed synchronization quarantines before terminal failure. There is no resumable replay after a panic between a PMM call and its journal receipt.

Changed production implementation/caller files across the complete KFP-2A.3 baseline: `kernel/src/process/address_space.rs`, `kernel/src/memory/tlb.rs`, `kernel/src/capability/mod.rs`, `kernel/src/memory/shared.rs`, and the feature-gated `kernel/src/capability/channel_mapping.rs` adapter. Other milestone changes in `capability/tests.rs`, `main.rs`, `memory/retirement.rs`, `memory/security.rs`, `memory/shared_quota.rs`, and `process/mmap.rs` are tests, observation availability, or diagnostic invocation/receipt hooks. The exact eleven-path delta from the initial KFP-2A.3 snapshot is retained in `changed-paths.json`; the continuation-only three-file patch is `continuation/continuation.patch`.

### Old-frame replacement

`replace_mapping` validates expected state and replacement fields before reserving one slot/one entry for `ReleaseOldFrame`. It rejects same-frame replacement, which otherwise could free a still-mapped frame. Journal exhaustion returns `FrameAllocationFailed` before the PTE changes; CoW/swap callers retain their existing unused-allocation rollback. The replacement PTE owns the new backing, and the journal owns the detached old frame. Exact synchronization precedes PMM release, receipt, and recycling. `RetainOld` remains caller-owned: treating rollback snapshots or compressed backing as ordinary releasable old frames would be incorrect.

### Final SHM backing and quota

Zero-view decrement/revocation returns a locator while the existing retired broker entry keeps its frame vector and quota receipt. Dropping that locator preserves both. Finalization finds the same permanently non-reused token, verifies retired/zero views, releases the last frame, then pops its descriptor. It refunds only after all backing releases and removes the empty row. Repeated finalization after row removal is a no-op. The loop avoids `Vec::drain` erasing unprocessed identities during unwinding.

This needs zero new journal slots because final backing already has a persistent allocated owner and its real views must already have synchronized. Duplicating physical ownership into each view record would be incorrect. This broker mechanism does not close pending-view invalidation. The global broker survives stack unwinding, not reboot.

### Partial-created-table rollback

`rollback_created_tables` stages every created table before clearing any parent. It reserves one slot with at most three exact physical frame entries for the entire rollback; real allocation-failure tests create one/two tables before the next allocation fails. Table frames share the unique-PMM-owner release action with private backing frames, so the same journal is suitable. Their release condition is distinct: the new `tlb::invalidate_retirement_all` adapter requests a full address-space flush after parent withdrawal, using the existing sequence, active-target, ACK, quarantine, and bounded APIC paths.

Only successful completion permits each table's PMM release and journal receipt. The journal remains durable if synchronization fails or the locator disappears. The stack array is only a snapshot once its parents are unlinked. No extra journal engine or global wait lock was introduced.

If capacity is unavailable, including a same-VM reservation conflict, rollback returns without unlinking or freeing any created table. The attached root graph remains authoritative and the original mapping error is returned. Later mapping can reuse the empty hierarchy; inactive-root destruction can reclaim it. Repeated faults at different locations can retain more attached empty tables until reuse/teardown or memory exhaustion; no background reclaim was added. This closes that detached-table ownership gap, not installed-leaf rollback, general empty-table pruning, or old exec-root ownership.

## 4. Journal capacity and exhaustion

Capacity is unchanged: eight records, 32 exact entries each, 8,776 bytes for the table and 1,088 bytes per record (mutex overhead excluded). Reservation scans eight slots, staging is bounded by 32, and neither allocates. No capacity increase was used to pass tests. Eight is the retained bounded admission limit, not a demonstrated workload-sizing optimum.

One anonymous request retains one slot across all maximum-32-entry chunks. Old-frame replacement needs one slot/one entry. Partial-created-table rollback needs one slot/up to three entries. Final SHM backing needs no slot. Same-VM overlapping reservations are refused; at most eight distinct VM reservations can coexist. Entry-cell count is not permission to admit arbitrary view/table/image transactions. General MAP_FIXED/view/exit transactions have no proven complete reservation bound yet.

Prepared reservations recycle only when empty. Synchronized chunks recycle only after every release receipt. Withdrawn, pending, quarantined, or staged-but-unfinished records cannot silently disappear. Checked generation/sequence exhaustion rejects or fail-stops rather than wrapping. Nested same-VM table rollback retains its original graph; unrelated-VM nesting is bounded by remaining capacity.

| Case | Result and precise limit |
|---|---|
| JE-1 zero capacity | Host PASS; guest full eight-slot journal rejects old-frame replacement without PTE/free-count change. New partial-table failure keeps the actual created table linked and allocated. |
| JE-2 last slot | Guest PASS for exact old-frame replacement reservation/transfer/release; table-specific last-slot scenario NOT_RUN. |
| JE-3 concurrent reservations | Host PASS: 12 threads compete for eight unique slot/generation owners. Concurrent guest retirement NOT_RUN. |
| JE-4 multi-resource transaction | Host two-entry frame/swap chunk PASS. Guest one/two-table rollback PASS, with all table identities staged before first unlink. General leaf/view/image transaction NOT_PROVEN. |
| JE-5 reservation rollback | Host stale-generation/slot-reuse PASS; guest unused reservations finish once and reject repeat finish. |
| JE-6 exit exhaustion | New guest full-journal inactive-root teardown and repeated teardown PASS for an attached empty table, independently observed as one physical release/zero reuse. Actual concurrent process-exit/SHM cleanup under exhaustion NOT_PROVEN. |

Exit cannot return an ordinary error after committed teardown. Current policy is terminal containment: root reclaim rejects any outstanding same-VM journal record before root mutation; SHM cleanup panics rather than discarding uncertain views. This is not a deferred exit protocol for unjournaled obligations and does not establish post-failure forward progress. Reaper IPC cleanup may precede these guards; no claim that the entire exit operation is unchanged on failure is made.

## 5. Host and fault-injection results

Fresh final-source host results: kernel 201 PASS; journal 14 PASS directly and included in the kernel tests; ring 9, Doorbell 42, IPC 94, UAC 18 PASS. The five library suites total 364 tests, including kernel/journal coverage. Scheduler production state/lifecycle/accounting/policy host controls PASS. Default production kernel check and whitespace check PASS; retained dependency warnings were not suppressed. Logs live in `continuation/`.

All active FI requires debug assertions and explicit `kfp2a2_fault_test`; compile-time mode and features are bound per build. Production ICR polling remains limited to 4,096 observations. Production ACK budget is unchanged. No ACK is synthesized for a single-CPU execution.

| Scenario | Fresh 2-CPU result | Evidence boundary |
|---|---|---|
| FI-1 busy ICR | PASS scoped | Actual polling exhausts 4,096 reads; anonymous backing remains journal-owned/allocated; expected terminal panic. |
| FI-2 NotSent | PASS scoped | Real anonymous withdrawal/publication precedes injection; exact backing retained; no release/reuse; terminal panic. |
| FI-3 missing ACK | PASS scoped | Handler actually claims/flushes; ACK suppressed; bounded diagnostic timeout; quarantined exact backing. |
| FI-4 delayed ACK | PASS scoped; full requested late-completion property NOT_PROVEN | Handler delays 4,096 spin hints after flush and before current ACK. Exact operation sequence and real remote target observed. Completion after a terminal timeout is not executed. |
| FI-5 stale/duplicate ACK | PASS scoped; full VM-successor property NOT_PROVEN | Previous sequence written twice cannot complete current request; duplicate claim rejected; real current ACK then completes. Successor VM/process/session adoption interleaving not forced. |
| FI-6 concurrent retirement | NOT_RUN in guest | Host arbitration/target controls are not overlapping guest retirement proof. |
| FI-7 cleanup after failed retirement | NOT_PROVEN | No equivalent nonterminal containment mechanism was established. Normal/repeated cleanup and zero-view finalization are separate passing checks. |

Fail-stop is retained. A watchdog or host termination is never counted as successful cleanup. No safety condition was bypassed to continue after failed synchronization. Table-specific remote failure injection remains NOT_RUN; its shared protocol reuse is source analysis, not independent remote table-fault proof.

## 6. Identity-based guest accounting

`continuation/guest-results.json` contains exact root/VM/VA/frame receipt tuples plus image/log hashes. Each boot is a separate allocation domain, even when physical addresses repeat. `continuation/accounting.json` independently checks receipt identities, pairing and order against raw logs, rather than relying on aggregate zero drift or journal release bits alone.

* FI-1–FI-3: actual withdrawn anonymous frames match persistent owners; independent PMM bitmap/owner/event observer reports allocated, zero releases, zero reuse.
* Old-frame replacement: before-release PMM receipts report allocated/zero releases/zero reuse; corresponding post-release receipts report unallocated/one release/zero reuse. FI-4/FI-5 remote munmap receipts include sequence, required/received masks, and handler observation of the exact sequence.
* Partial tables: three exact table lifetimes per normal/delayed/stale pilot (one plus two tables) have paired synchronized/released receipts and independent PMM observations. These VMs are inactive; this does not prove remote table-cache behavior. Per-table shootdown sequence and journal slot-generation receipts are not independently emitted.
* Full-journal retention: exact root/VM/table identity is observed attached, then after two inactive teardown calls reports unallocated/one release/zero reuse; PMM baseline agrees. It is not a concurrent active process-exit test.
* Final SHM handoff: exact physical frame and original PID/incarnation accompany a dropped locator and repeated finalizers. The identity-qualified ledger is 4,096 bytes before and zero after; independent PMM observer reports one release/zero reuse. This fixture is a never-mapped zero-view object, not pending peer cleanup.

No complete independent census of swap-slot retirement, all table/view generations, every quota reservation/refund, successor adoption, or all retired records exists. Production aggregate counters agree within these fixtures, but full requested no-loss/no-duplicate/no-early-reuse/no-stale-generation accounting is NOT_PROVEN. PMM release followed by receipt update is not crash-atomic and cannot be safely replayed as recovery.

## 7. Controlled guest stages and CPU matrix

Stage A passes within host scope. Fresh Stage B includes six 2-CPU guests (normal and FI-1–FI-5), all passing their explicit scoped validators. Normal guests also execute allocation-failure tests through `mm2a_test_injection`. FI-6/FI-7 and the complete accounting requirements remain open, so the stipulated complete-pilot prerequisite for Stage C is not met.

| CPUs | KFP-2A.3 final-source lifecycle execution |
|---:|---|
| 1 | NOT_RUN; no remote ACK fabricated |
| 2 | Normal MM-2D, scoped JE-1/2/4/5/6, final backing and FI-1–FI-5 scoped PASS; FI-6/FI-7 incomplete |
| 4 | NOT_RUN; incomplete Stage B gate |
| 12 | NOT_RUN; incomplete Stage B gate |

Historical 1/2/4/12-CPU results are not promoted. The source-bound native security pilots below are independent regressions and do not complete Stage B/C.

## 8. Native security regressions

`continuation/results.json` indexes original native build/run manifests and their exact failures. Fresh default-check and native builds bind source, ELF/ISO, and embedded service identities. QMP-dependent guests were retried with authorized socket access after preserving the original sandbox denial. Three fresh 2-CPU native regressions were attempted with unchanged assessors and expected markers:

| Native fixture | Build | Runner | Security-case execution |
|---|---|---|---|
| M1/M2 restricted mapping | PASS | FAIL | NOT_RUN: shell launch prerequisite |
| M4 notification | PASS | FAIL | NOT_RUN: shell launch prerequisite |
| M5 transfer | PASS | FAIL | NOT_RUN: shell launch prerequisite |

All three timed out waiting for `sunshell registered as 'sshl'` after the guest reported that `/bin/sshl0` failed to spawn. Source/image/service bindings passed; QMP socket access succeeded in these fresh attempts. The original parser FAIL results and every missing-case diagnostic are preserved. These watchdog/timeout stops are not cleanup success and do not prove security-case failure or success. The authenticated spawn failure's cause was not established.

M3 ring representation/host tests pass (9 tests); a dedicated native M3 compatibility replay is NOT_RUN. Native M1/M2/M4/M5 acceptance remains unestablished if their launch prerequisite fails. The native parser's FAIL result and the distinction between prerequisite failure and execution of a security case are both retained. No evidence establishes a causal connection between a fixture launch failure and retirement changes.

Additional Stage D disposition:

| Suite | Final-source evidence and limitation |
|---|---|
| MM-2D | Fresh normal 2-CPU boot gate PASS; unchanged expected markers |
| VMM/PMM | Scoped boot/rollback/replacement/bitmap accounting PASS; independent full standalone suite NOT_RUN |
| SHM lifecycle | Normal peer lifecycle and final zero-view handoff fixtures PASS; forced-revocation/concurrent pending-view matrix NOT_RUN |
| IPC mapping closure | IPC host 94 PASS; native mapping cases NOT_RUN after prerequisite failure |
| Scheduler controls | Production host state/lifecycle/accounting/policy PASS; full guest controls NOT_RUN |
| Capability/quota | Kernel host suite and scoped guest final handoff PASS; full guest concurrency/refund matrix NOT_RUN |
| 1/4/12 native regressions | NOT_RUN; Stage B lifecycle gate incomplete |

Applicable host suites and normal MM-2D are not substitutes. Production FastLane and production restricted entry points retain their existing debug-feature gates.

## 9. Lock order and forward progress

Existing order is SCHEDULER → PMM → optional CAP_BROKER → SHOOTDOWN_SERIALIZER. Short OWNERS sections are bounded metadata operations; existing additional edges are PMM → OWNERS and SERIALIZER → OWNERS. Replacement and partial-table rollback release OWNERS before PTE writes, TLB waits, PMM calls, or diagnostic output. No OWNERS callback acquires an outer lock. Remote handlers take no OWNERS/broker/PMM lock.

The broker finalizer uses the existing PMM/broker/quota exclusion, does not reacquire PMM, allocate, or wait remotely. The table adapter stages a fixed three-entry snapshot and allocates nothing. Capacity conflict retains attached ownership instead of nesting another wait or journal engine. No new global lock spans shootdown waits; the pre-existing serializer still spans them, and existing outer spinlocks/IRQ-off contexts still constrain progress.

No new critical inversion was found in these changed paths. This is source review plus scoped tests, not a whole-kernel proof. Serializer fairness, priority-inversion bounds, full activation exclusion, elapsed MMIO bounds, and every IRQ-off/remote wait dependency remain unproved. Terminal failures retain outer guards. No unbounded polling or APIC recovery was added.

## 10. Performance and retention

The journal remains 8,776 bytes; capacity/record size did not change. Successful ordinary `map_page` has no added journal operation; only partial-created-table rollback uses the new adapter. That error path adds reserve/stage/state/receipt work and a full flush instead of its former page flush. Full-journal failures can retain one/two linked empty tables until reuse or teardown. Before/after production latency and worst-case retained footprint were not measured.

The final optimized host observation executed 100,000 reserve-through-finish iterations in 10,720,065 ns (about 107 ns per iteration), excluding production spinlocks, VMM/PMM, APIC and guest scheduling. It is not production retirement latency. Old-frame replacement adds metadata work for one frame but no additional shootdown; final SHM backing reuses its broker vector without another allocation. Guest wall time is only fixture duration, not a flush/latency metric.

Contention distributions, normal-retirement latency distributions, repeated nonterminal fault retention, priority inversion and reclamation throughput remain unmeasured. Terminal fault guests cannot demonstrate repeated continuing-fault behavior.

## 11. Final verdict and next milestone

**PARTIAL.** The added ownership paths have bounded reservation or retained-original-owner policies within their documented scopes. Acceptance is not met: transaction/view/image gaps, full late/stale completion semantics, concurrent guest cleanup, complete identity accounting, full CPU matrix and native security acceptance remain open.

Recommended KFP-2A.4: first establish durable MAP_FIXED/installed-leaf, pending-view and retired-image ownership with whole-transaction admission bounds; resolve the authenticated native fixture launch prerequisite without weakening grants; then add per-operation/table/view/quota identity receipts and a separately justified test-only containment mechanism. Prove actual concurrent FI-6/FI-7 on two CPUs before expanding to 1/4/12 and the complete native security matrix. Automatic recovery remains a separate, unimplemented milestone.
