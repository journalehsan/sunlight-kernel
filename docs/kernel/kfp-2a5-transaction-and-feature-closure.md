# KFP-2A.5 — Transaction ownership and feature interaction

## Verdict and evidence boundary

**Overall: PARTIAL. Production runtime behavior changed. Production FastLane remains disabled. No commit or push.**

The historical M1 terminal-reason defect is reproduced, isolated and corrected. A separate final-source original-configuration contention-coverage failure remains: mode 3 exhausted the existing sixteen attempts without observing a failed scheduler `try_lock`. That attempt remains failed; later configurations do not cancel it. Gate A is therefore **PARTIAL**, not COMPLETE.

MAP_FIXED now uses a durable whole-request owner in the existing retirement journal, with admission before preparation or withdrawal, distinct transfer/release receipts, full-flush commit/rollback barriers, and quarantine on failed restoration. Gate B is **PARTIAL**: actual overlapping syscalls, concurrent process exit, complete independent table-free/swap-release accounting, and the complete CPU matrix are not demonstrated.

Machine-readable entry points are `.kfp-2a5-evidence/results.json`, `m1-analysis.json`, `accounting.json`, and `validation-results.json`. Per-attempt `results.json` files retain exact commands, source hashes, features, environments, CPU counts, image hashes, serial hashes, classifications and original assessor results. Historical failures were not edited or deleted.

## 1. Source baseline

HEAD is `882b40a65f55951b5cd8b9978c05580e71202488`. `baseline.json`, `status-before.txt`, `preexisting.patch`, and `baseline-source/` capture the incoming uncommitted KFP-2A through KFP-2A.4 work. The required KFP-2A.3 and KFP-2A.4 reports and KFP-2A.4 results were read. Existing source changes are retained; `source-integrity.json` identifies this milestone's delta separately.

Final source-manifest SHA-256: `9eaaedb4695359dcc3159916d79f87000409b462d3e176513f91432de79c7fc8`. Exactly fourteen source/build/tool paths differ from the incoming snapshot. All other captured source inputs and the historical results remain unchanged. `milestone-only.patch` separates these changes from the preexisting uncommitted work.

KFP-2A.4's original two-CPU M1–M5 passes, normal MM-2D pass, four exact rollback-adapter host tests, and incomplete whole-transaction gate are historical evidence. Its combined-feature M1 failure is the preserved input, not superseded by those original-configuration passes. This milestone's before/after and final-validation source manifests distinguish every subsequent source snapshot.

## 2. Gate A reproduction and feature isolation

The authenticated SessionD fixture starts an Account-inherited real shell and the original native clients. No Account, capability, process or session identity is forged. The original assessors and terminal expectations remain unchanged.

Commands use `python3 tools/kfp2a4_native.py --suite m2 --kernel-fixture FIXTURE --evidence-root DIRECTORY`; the runner records all service builds and the QEMU invocation. Base kernel features are `key_inject,session_launch_test`. Defaults are empty.

| Fixture | Additional effective kernel features | Before correction, two CPUs |
|---|---|---|
| mapping, original M1/M2/M3 configuration | `fastlane_mapping_test,fastlane_channel_test,native_session_race_test` | PASS, 10 race attempts, no terminal mismatches |
| doorbell | original features plus `fastlane_doorbell_test` and doorbell dependency fixture | FAIL, 17 terminal mismatches; additional contention coverage exhaustion retained |
| transfer, historical combined configuration | doorbell features plus `fastlane_transfer_test` | FAIL, six terminal mismatches in this reproduction |

The historical failure had 19 attempts and 16 terminal mismatches: one mode-3 attempt and fifteen mode-4 attempts. The new combined run had one mode-3 and five mode-4 terminal mismatches. These are repetitions of **one classification defect**, not sixteen independent bugs. `m1-analysis.json` joins each attempt to its immutable producer, consumer, issuer, session/epoch, PID/incarnation/VM, operation, CPU, lock-order and outcome receipts. No secret capability token is introduced into the new evidence.

### Causal ordering

`sched::exit` marks the exact process Finished, then calls `fastlane_mapping_sweep`, then closes owned IPC endpoints. With the doorbell feature enabled, `fastlane_mapping_sweep` calls `fastlane_bell_sweep` before its mapping-ACTIVE early return. That sweep called `channel_process`, which deliberately hides exiting processes, and classified the resulting absence as `IdentityChanged` (5). `Channel::retire` publishes only the first terminal outcome. Subsequent endpoint closure invokes `retire_recipient_grants` / `retire_process`, but cannot replace 5 with `ProcessRetired` (2).

In the original M1 configuration, there are no mapping backings yet and no doorbell sweep. The ACTIVE early return lets endpoint closure publish `ProcessRetired` first. The triggering feature is therefore doorbell's additional sweep, not transfer payload handling, initialization order, credential loss, generation reuse, or sixteen different ownership failures. Scheduling affects the number of coverage attempts, not this terminal-reason ordering.

### Repair and contract

`channel_participant_terminal` searches by the complete `RecipientIdentity` before classifying state: an exact exiting participant yields `ProcessRetired`; missing/mismatched identity or lost required privilege yields `IdentityChanged`; an exact live authorized participant yields no retirement. `Channel::retire_unavailable` is shared by admission, mapping and doorbell sweeps. Original role/session checks, first-outcome semantics, receipt destruction, backing drains and wake obligations remain intact. These adapters are debug-feature gated; they do not enable production FastLane.

| Failure family | Participants and operation | Expected / historical actual | Ownership and completion |
|---|---|---|---|
| Mode 3 | exact producer attach wins before exact consumer exit | final `ProcessRetired` / `IdentityChanged` | scheduler serializes attach, logical exit, sweep and endpoint close; one immutable terminal result |
| Mode 4 | exact consumer exit wins before producer attach | producer gets Terminal; reason `ProcessRetired` / `IdentityChanged` | exit fences admission; loser cannot attach; early sweep incorrectly won reason publication |

No evidence established duplicate terminal publication, lost termination, unauthorized attachment or generation adoption in those sixteen failures. The mismatch was a published wrong reason, not absence of a terminal state. The expectation matches the explicit process-retirement hook and is preserved.

### Separate final-source coverage failure

The final original mapping configuration reached mode 3 but all sixteen attempts reported `lock-missed`; the userspace fixture failed at its unchanged `covered` assertion. Its yielding wait for HELD can resume after a short winner operation has finished. This is an unfulfilled actual-contention requirement, not a renewed terminal-reason mismatch. No retry, longer timeout, forged contention, assertion relaxation or wait under the scheduler lock was added. Combined and doorbell-only final configurations pass separately. The original final failure prevents Gate A acceptance and CPU expansion.

The first final M5 attempt failed the ELF embedded-payload check for `uac_service` before QEMU launch. It is a build/image-integrity failure, not a security-test failure. Its artifacts and classification are retained separately.

`embedded-cache-diagnosis.json` proves that the failed M5 stripped ELF is byte-identical to the prior M2 combined ELF (`b9dc23c7a61b90298b11f3aacb29a8692b434daca7c87eeaa5f1bbe810e75909`), contains the old UAC payload, and lacks the expected M5 UAC payload. A recorded package-specific `cargo clean -p sunlight-kernel --target x86_64-unknown-none` followed by a fresh authenticated M5 build passes both payload verification and the native M5 assessor on the unchanged final source. This is a corrected build-control attempt, not a rerun hiding a security failure. Automatic cache invalidation across service-feature changes remains a runner/build-system limitation; the embedding check continues to reject mismatches.

| Final-source two-CPU configuration | Result | Boundary |
|---|---|---|
| Original mapping M1/M2/M3 runner | FAIL, coverage | no terminal mismatch; M1 mode-3 contention exhausted; M2/M3 not reached |
| Doorbell-only kernel with original M1/M2/M3 clients | PASS | original assertions retained |
| Combined transfer kernel with original M1/M2/M3 clients | PASS | original assertions retained |
| Original M4 runner | PASS | original doorbell controls |
| Original M5, first build | SOURCE_INTEGRITY_FAILURE | no guest launched |
| Original M5, fresh verified build | PASS | original transfer controls |

## 3. Gate B transaction reconstruction

Production path: `process/mmap.rs::map_anonymous_kind` validates ABI flags, range, W^X and replacement policy, stages the existing ledger plan, and dispatches MAP_FIXED to `map_fixed_transaction`. Ordinary nonfixed mapping and the previous rollback helper retain their existing path.

1. Validate every target leaf against the existing ledger, owner, present/swap identity and protection. Shared/system/protected ranges are rejected.
2. Compute the complete backing-entry requirement: one entry per replacement frame plus one per present/swapped original. Absent/demand leaves require no original-backing entry.
3. Reserve one operation record for the exact root/VM generation and full range. Persist every original descriptor and its flags before preparing replacements.
4. Reserve swap-candidate metadata, prepare required page tables, allocate and sanitize every replacement frame. Record each allocation in the authoritative record before further fallible work.
5. Enter Installing only when all replacement frames and all admitted backing entries are ready. Write individual leaves through `write_transaction_page`, which validates journal generation, VM identity, expected backing, flags and ordered install/restore position.
6. Perform the existing full-address-space owned TLB flush. Exact-sequence target completion precedes old-backing release.
7. Publish the staged ledger, mark Committed, track new anonymous candidates, release originals, transfer replacements to the live mapping graph, then recycle the operation record.
8. On preparation/installation error, enter Cancelling, restore installed leaves in reverse order, synchronize the full VM, mark RolledBack, transfer originals back to the live graph and release all prepared replacements exactly once.
9. On restoration mismatch or failed synchronization, retain the record and both backing sets, quarantine and fail-stop. No continuation/retry/reaper drain is authorized.

Ledger publication and physical release are irreversible transitions. A replacement leaf becomes visible to hardware at its individual PTE store, **before** ledger commit. Table preparation also publishes empty hierarchy nodes. Whole-range atomic hardware visibility is not provided. A concurrent user thread may observe a mixture of old/new pages or fault on an absent page during rollback; fault handling and competing kernel VM operations serialize on SCHEDULER. The old `MAY_REPLACE` comment's word “atomically” must not be read as a proven multi-page hardware guarantee.

## 4. Authoritative ownership and admission

`memory/retirement/transaction.rs` extends the existing `Table`, rather than adding another retirement engine. The non-Clone token is only a slot/generation locator. The static record retains root/VM, range, complete original page descriptors, original flags, new flags, prepared replacement identities, preparation/install cursors, capacity, disposition, exact shootdown sequence/masks, and distinct release/transfer completion bits. Dropping a token never clears an unfinished record. Existing inactive-root teardown refuses outstanding same-VM records.

Capacity remains **eight operation records, each with 32 backing entries**. A VM cannot combine the eight records into 256 slots. A wholly absent 32-page range fits; a wholly backed 16-page replacement fits; a wholly backed 17-page replacement does not. Mixed requests use `pages + original_backings`. Requests larger than 32 pages or lacking one free same-VM-compatible record return NoMemory before page-table preparation, frame allocation or old-leaf withdrawal. Capacity was not enlarged to admit these cases.

Persistent transaction metadata increases journal storage from 8,776 to **19,656 bytes**, and each record from 1,088 to **2,448 bytes**; it does not increase the number of admitted backing obligations. The standalone host size result is in `retirement-host-final.stdout`.

Prepared page tables stay attached to the VM's canonical root graph, including on preparation failure and rollback. They are initialized before parent publication. This path never detaches them or recursively reserves a second journal record. Their root/VM-qualified graph remains the owner until normal inactive teardown. Retained empty tables can consume memory until reuse/teardown; aggregate final-PMM restoration is tested, but independent per-table free receipts are still missing. This is a deliberate ownership policy, not a claim that every failed request restores the identical table topology.

The fixed path does not perform per-page direct reclaim while holding its reservation: physical preparation failure cancels safely rather than attempting nested same-VM retirement. Prepared-only cancellation conservatively uses a full VM flush too; an acknowledgment failure can therefore fail-stop even when no old leaf was withdrawn. This is a remaining progress limitation, not permission to release uncertain mapped backing.

## 5. Dispositions and synchronization

| Disposition | Preconditions and owner | Allowed completion / failure |
|---|---|---|
| Preparing | full-request reservation; original graph still live; replacement allocations recorded | complete preparation or cancel; no backing release through generic journal APIs |
| Installing | complete frame preparation; exact expected leaves; SCHEDULER and PMM exclusion | ordered PTE stores; full-flush synchronization; commit or reverse restoration |
| Committed | all leaves installed, exact ACK set complete, ledger published | release originals; transfer replacements; no cancellation |
| Cancelling | preparation/install failure; journal remains authoritative | reverse exact restoration; full flush; no release before completion |
| RolledBack | all installed leaves restored and exact ACK set complete | release replacements, transfer originals, finish only after every receipt |
| Quarantined | restore mismatch or delivery/ACK failure | retain all unfinished ownership; deny release, recycling, late completion and teardown |

The old generic `stage`, `released`, `next_chunk` and `finish` paths cannot consume a transaction record. PMM/ZRAM operations happen outside the journal mutex; release and transfer are not represented as the same receipt. A panic between physical release and its receipt is terminal containment, not a supported replay point. No recovery engine or background reclamation was added.

## 6. Tests and accounting limits

Host: 208 kernel tests pass, including one new M1 first-outcome/participant test and six transaction tests. The standalone journal run passes 20 tests (a subset, not 20 additional independent kernel tests). Tests cover whole-request rejection, unused cancellation, exact commit/rollback release-versus-transfer permissions, locator loss, same-VM exclusion, stale/partial ACK refusal, quarantine and repeated-resolution refusal. Host models do not demonstrate concurrent native execution.

Additional host suites pass: IPC 94, UAC 18, ring 42, doorbell 9, and Helios 29: **400 library tests total**. Scheduler state/lifecycle/accounting/policy controls pass. Parser controls execute 137 passing cases and skip 67 (204 discovered). Default production kernel check and whitespace check pass. The first transaction host compilation failed module-path resolution; that build failure is preserved separately from the corrected passing host runs. The new guest image also passes the original MM-2D gate before entering the transaction fixture.

| ID | Implemented guest control / remaining boundary |
|---|---|
| MT-1 | real four-page replacement with local and remote full-flush completion |
| MT-2 | all eight records occupied; oversized request; existing leaves/data and free count unchanged |
| MT-3 | actual PMM failure after one prepared replacement; all original leaves/data restored unchanged |
| MT-4 | test-only installation failure after one real PTE replacement; reverse restoration and replacement release |
| MT-5 | test-only restoration failure; persistent exact old/new identities and terminal quarantine |
| MT-6 | existing 4,096-read busy-ICR / NotSent injection after real replacement and owned publication |
| MT-7 | actual remote handler with missing ACK; delayed/stale diagnostic modes use the existing protocol |
| MT-8 | same-VM host admission exclusion only; overlapping native MAP_FIXED syscalls NOT_RUN |
| MT-9 | source-level scheduler exclusion and outstanding-owner teardown guard; concurrent exit/handoff NOT_RUN |
| MT-10 | two real cancellation cycles, no outstanding owner on return, exact original PTE/data and allocator restoration |
| MT-11 | four-page range across a P1-table boundary; table graph retained until teardown |
| MT-12 | generation-encoded swapped original replaced and discarded; private anonymous mapping has no shared-quota charge/refund |

`accounting.py` independently joins slot/generation/root/VM/index/VA/backing tuples across synchronized decisions and resolution. PMM bitmap/owner/release/reuse observations are independent of the journal release bit. Released frames must be allocated with zero releases/reuses before release, and unallocated with exactly one release/zero reuse afterward. Transferred frames remain allocated with zero releases/reuses. Quarantined resources stay allocated with zero release/reuse. Observer windows are per transaction, not proof that a physical address is never reused in later transactions or boots.

All seven final-source two-CPU modes pass their scoped controls: normal, restoration failure, busy ICR, NotSent, missing ACK, delayed ACK and stale/duplicate ACK. The normal boot has 68 joined resource resolutions, including its earlier MM-2D transactions. The dedicated transaction fixture contributes 38: **16 exact frame releases, 21 backing transfers, and one swap disappearance**. Each of the three shootdown-failure modes retains eight exact old/new frames; restoration failure retains eight exact old/new resources. All independent checker joins pass; unpaired observed backing obligations are rejected by the checker. These counts index identity sets in `accounting.json`; they do not replace those identities.

The transaction remote-success fixture reads zero before and after replacement. Its exact owned ACK evidence is valid, but that value comparison alone cannot distinguish a stale old translation. The preceding ordinary MM-2D test has distinct-value/reuse checks; it is not promoted into a transaction-specific remote-value witness.

Swap existence is observed independently, but a Boolean absence is not an independent exactly-once discard counter. Table publication records name the exact parent and frame, but final aggregate allocator recovery does not prove each table's synchronization/free/reuse order. Delayed/stale modes do not establish successor-VM adoption, late completion after terminal failure, or complete independent accepted/rejected ACK receipts. No shared-quota operation is reported as a measured zero when it is not applicable.

## 7. CPU matrix and progress audit

The final per-attempt outcomes are indexed in `results.json`. Both gates remain short of their independent pilot acceptance, so their 1/4/12-CPU matrices are NOT_RUN. The original M1 assessor additionally requires actors on distinct CPUs; its single-CPU contention requirement is unsupported, not silently relaxed. A separate single-CPU sequential security control is future work.

| CPUs | Gate A | Gate B |
|---|---|---|
| 1 | NOT_RUN; distinct-CPU race contract unsupported | NOT_RUN; pilot incomplete |
| 2 | PARTIAL; original coverage FAIL, combined/M4/fresh-M5 PASS | PARTIAL; seven scoped pilots PASS, MT-8/MT-9 NOT_RUN |
| 4 | NOT_RUN; pilot incomplete | NOT_RUN; pilot incomplete |
| 12 | NOT_RUN; pilot incomplete | NOT_RUN; pilot incomplete |

Production lock order remains SCHEDULER → PMM with short journal metadata sections. No journal guard survives a PTE operation, PMM release, ZRAM discard, serial receipt or TLB wait. The fixed path does not recurse through old-frame retirement or partial-table rollback. Its page/resource loops are bounded by 32; generic reservation scans eight records. Remote handlers do not need the scheduler/PMM locks held by the caller. Existing bounded APIC delivery and fail-stop behavior remain unchanged.

Overlapping mmap, munmap, kernel page-fault handling and process teardown serialize through the scheduler guard; ordinary remote user accesses do not. Bounded synchronous execution avoids a new asynchronous exit handoff protocol. A quarantined owner prevents teardown, so failed synchronization can stop process/VM progress indefinitely by design. No starvation or successful recovery claim is made for that state.

## 8. Changes and next milestone

Default production behavior changes in `process/mmap.rs`, `process/address_space.rs`, `memory/retirement.rs`, and new `memory/retirement/transaction.rs`. Debug-only FastLane changes are in `capability/channel.rs`, `channel_native.rs`, `channel_mapping.rs`, and `channel_doorbell_native.rs`. The transaction feature, boot invocation, host tests, `memory/transaction_gate.rs`, and native runner options are test infrastructure. Unrelated incoming changes are preserved.

Recommended KFP-2A.6: establish deterministic real M1 contention coverage without a scheduler-dependent peer wait under the lifecycle lock; resolve reproducible embedded-image cache provenance; add actual overlapping MAP_FIXED/munmap/fault/exit interleavings, independent table reclamation and swap-discard receipts, and an explicit partial-preparation table-accounting control. Only then expand each passing pilot independently to the CPU matrix. A passing combined M1 run or ordinary MM-2D boot must not substitute for those missing obligations.
