# F1.1B-0H: cross-service lifecycle investigation and partial replay

Date: 2026-10-27. **INCOMPLETE: Global Authority and Grant Lifecycle remain PARTIAL. F1.1B-0 and Gate C remain BLOCKED.** This is a limited current-source replay, not closure of the requested H phase. No authorization, production, or test-hook code was changed, and no FastLane functionality was enabled. Evidence is in `target/fastlane-f11b0h-evidence/validation-summary.json` and its linked raw attempts.

## Baseline and evidence boundary

The F1.1B-0G report records Gate A VERIFIED NATIVE, Token-History VERIFIED BOUNDED (65,536 prepared identities per boot, fail-closed exhaustion), Global Authority and Grant Lifecycle PARTIAL, Session Authority and Gate B PARTIAL, and F1.1B-0 and Gate C BLOCKED. It reports eight native cells, 133 kernel, 258 service/library and 41 parser tests and a passing final D3/RecoveryBlocked replay; earlier D3 failures remain unexplained. The original N3M post-race lookup failure and prior N4 timeout remain unresolved historical observations. Genuine numeric PID reuse was not forced. The `target/fastlane-f11b0g-evidence/` directory and its machine summary are absent in this checkout; these G results are report-derived and its raw evidence cannot be independently rechecked here. No missing artifacts were recreated or represented as current runs.

G explicitly left an authenticated Account/sessiond staging and metadata observation outside its completed UAC/VFS operation set, incomplete per-token terminal correlation, issuer exec versus VFS commit, endpoint closure versus reply, competing consumption/cancellation, stale slot consumption and legitimate VFS replacement admission. G's delayed UAC reply cancellation-before-exec and exit/reap results do not by themselves prove all these orderings.

## Ownership graph and release points

```text
TTY password proof <-- UAC authentication --> broker proof
TTY -> sessiond REGISTER/ACTIVATE -> broker session (sessiond owner, UAC issuer)
client -> Account handler in UAC -> broker Account grant -> exact IPC reply -> client
client -> UAC proof/exchange -> broker VFS delegation -> VFS consumes for client
sessiond logout / owner or issuer retirement -> broker session and Account grant removal
SunlightD manages service lifetime; IPC and scheduler perform endpoint/process cleanup
```

| Resource | Creator | Authority owner | Reference holder | Retirement point | Census source |
|---|---|---|---|---|---|
| Authentication proof | UAC via `Scheduler::issue_auth_session_grant` | Broker | Exact TTY recipient and issuer/request identities | Registration consumption, cancellation, expiry or process retirement | Broker grant rows in `capability/reconciliation.rs` |
| Account session | `register_account_session` after trusted proof | Broker `sessions.rs` | sessiond active metadata, broker owner/issuer identities and request receipts | `transition_account_session` REVOKE, expiry, owner/issuer retirement | `session_accounting`; E3 session subset inventory |
| Account grant | Account handler through `Scheduler::account_grant` | Broker | Exact client incarnation/VM, UAC incarnation and request; session binding | One-use consumption, revoke/suspend, expiry, failed delivery or exit/exec | Broker Account rows, E3 mint/consume and inventory |
| Session metadata and children | sessiond `create_session` | sessiond for metadata; broker independently owns authorization | `ServiceState.active`, components and `last_closed` | `begin_stop`, supervision, `finalize_stop`; `last_closed` is historical metadata | No epoch-qualified service acknowledgment in current fixture |
| Account staging | UAC Account `publish`/dispatch | UAC for transient file/stack resources; broker for grants | Handler, temporary fd/path, synchronous reply | Close/rename/unlink, handler return or failure path | Not independently measured by current fixture |
| VFS delegation | UAC exchange, broker `delegate_vfs_grants` | Broker; delivered authority belongs to bound recipient | VFS synchronous request and broker rows | `consume_vfs_grant`, cancellation, expiry, exit/exec; delivered VFS grants can outlive issuer by policy | Broker census; G reports service-boundary acknowledgments |
| IPC calls/replies | Client and endpoint service | Kernel IPC and process lifecycle | `pending_call`, `ipc_reply_target`, deferred targets and endpoint queues | Exact call settlement, `finish_call`, endpoint removal, reap | E3 fixture cleanup and F scheduler/broker/shard census |
| Service lifecycle | SunlightD and kernel spawn | SunlightD manages units; kernel installs/withdraws trusted roles | Service process identity and activation records | Normal exit, stop or crash; reaper/exec retirement | E3 role-qualified lifecycle events; no complete manager inventory here |

Source: `services/sunlight-sessiond/src/main.rs`, `services/sunlight-uac/src/bin/account_service/mod.rs`, `services/sunlightd/src/main.rs`, `kernel/src/capability/{lifecycle,sessions,mod,vfs_grants,reconciliation}.rs`, `kernel/src/ipc/mod.rs` and `kernel/src/process/{spawn,exec_authority}.rs`. Sessiond's configured service specifies restart on failure; that configuration alone does not establish a successful recovery. Logout's request path checks synchronous broker REVOKE before acknowledging. Other sync/stop paths ignore transition return values; broker authorization remains fail closed, but an independently bounded cross-service convergence or explicit failure observation has not been established here.

## Snapshot and accounting limits

F's census serializes scheduler, broker, then IPC shards; it separates current live rows, cumulative outcomes, and reusable capacity. G reports separate ordered UAC/VFS service acknowledgments and a fixture-local quiescent operation set, not a system-wide atomic snapshot. This H run uses the existing E3 session-only inventory and fixture-process IPC cleanup. It has no matching sessiond `active`/`last_closed` acknowledgment, Account staging drain, VFS acknowledgment, global census, or fixture operation generation connecting these observations. It therefore cannot assert cross-service quiescence or conservation. No lock was held across service IPC by changes in this phase; there were no code changes. Sessiond metadata is not a broker grant. Terminal counters are not current inventory; capacity and history are distinct from live grants and allocation leak accounting.

## Current-source native evidence

The linked build manifest records the source map, kernel/ISO and embedded service hashes and test features. The existing parser checks identities, actual role-qualified registration, ordering, terminal subset inventory, fixture process IPC cleanup, source/image matches and timeout failures. These are E3 fixture results, not H cross-service acceptance.

| Scenario | Current result | Scope and remaining gap |
|---|---|---|
| N1, 1 CPU | FORCED_AND_PASSED | Logout before Account mint; broker denied old authority. Session subset and fixture IPC cleaned. No sessiond metadata acknowledgment. |
| N2, 2 CPUs | FORCED_AND_PASSED | Account commit before logout; pending grant retired and stale use denied. No global cross-service drain. |
| N7C, 2 CPUs | FORCED_AND_PASSED | Consumption before revoke and second consumption denied. Does not force VFS consume/cancel competing entry. |
| N6, 4 CPUs | FORCED_AND_PASSED | Actual Account replacement and fresh TTY login; E3 session-subset scope only. |
| N6, 12 CPUs | FORCED_AND_FAILED | Existing parser required retirement before peer-close; peer-close sequence 14 preceded retirement sequence 15. The fresh-login portion executed, but the combined attempt failed and is not acceptance evidence. |

For N2 the observed inventory changed from one session, one receipt, one pending Account grant to zero of each after revoke. N7C recorded one mint and one consume and zero terminal Account grant/session/receipt entries. Capacity remained at four session slots and 576 grant metadata bytes; allocator attribution is UNMEASURED. These are separate boot-local fixture results, not a global inventory equation.

## Current 12-CPU failure

The preserved N6 attempt had commit sequence 11, manager signal 13, peer-close 14, session retirement 15, old IPC cleanup 16 and replacement activation 17. The existing parser explicitly requires retirement before peer-close, so the combined result is FORCED_AND_FAILED even though replacement rejected the old session and the fresh-login subfixture ran. Source inspection of `Scheduler` reaping shows endpoint removal and `finish_peer_closed_calls` before `revoke_endpoints_owned_by`, which calls `retire_recipient_grants`; both are called through the scheduler lifecycle context. This explains a plausible source-level sequence, but has not established all execution/visibility boundaries or whether this ordering meets the intended security contract. Do not weaken the parser or call the failure resolved without a dedicated proof. No retry was made.

## Requested reconciliation matrix and missing races

| H scenario | Status in this phase |
|---|---|
| R1 successful session/grant and teardown | PARTIAL: N7C covers real mint/consume/revoke and fixture cleanup; no correlated service inventories. |
| R2 logout with pending grant | PARTIAL: N1/N2 force Account ordering; missing H cross-service reconciliation. |
| R3 recipient termination | NOT_RUN in H; G report describes earlier exit cases. |
| R4/R5 Account retirement/replacement and fresh login | E3 N6 replay as recorded in machine summary; H-wide inventory NOT_RUN. |
| R6 sessiond managed termination | PARTIAL: real logout path reached; active/last_closed and deferred cleanup not acknowledged. |
| R7 endpoint closure with pending reply | NOT_FORCED. |
| R8 repeated cycles | NOT_RUN with H correlated inventories. |
| R9 reachable failure rollback | PARTIAL: stale authorization denied in N1/N2; H storage/delivery rollback census NOT_RUN. |
| R10 concurrent independent sessions | NOT_RUN. |

Remaining grant-specific orderings: VFS consume versus cancellation competing entries, reply publication versus endpoint closure, UAC issuer exec/retirement versus VFS commit, late consume versus actual replacement slot, and legitimate VFS replacement versus old delivery are **NOT_FORCED**, with replacement admission **BLOCKED** under the currently documented contract. G's timeout-before-recipient-exec ordering is historical production-contract prevention for its synchronous path, not a newly forced live-pending-across-exec race. N4 cooperative-stop prevention is a separate historical result, not a forced retirement-before-commit. No safety or role checks were disabled.

## Validation, provenance and decisions

The machine summary records exact commands, each attempt's serial and QEMU logs, image/source identities and check statuses. Host kernel, service/library, SunlightD runtime, scheduler and parser checks passed. Strict Clippy failed on existing workspace warnings, media failed on a missing WAV fixture, workspace formatting differs, and Miri is unavailable; these are recorded failures, consistent in kind with G's baseline, not passing checks. The H native runs are not the required 1/2/4/12 H reconciliation matrix, and no H compatibility, D1/D2/D3, RecoveryBlocked or verified TCP/TLS replay is claimed. Production build and other results are recorded explicitly as run or NOT_RUN in the machine summary. No new parser negative tests, reconciliation instrumentation, grant race hooks, or production changes were made.

| Gate | Decision |
|---|---|
| Scheduler A | VERIFIED NATIVE, inherited G report; no contrary relevant result here |
| Token-History | VERIFIED BOUNDED, inherited G report, one boot and 65,536 identities; not revalidated in H |
| Global Authority | PARTIAL: no Account/sessiond/UAC/VFS correlated quiescent census |
| Grant Lifecycle | PARTIAL: essential grant-specific orderings remain NOT_FORCED/BLOCKED |
| Session Authority | PARTIAL: original N3M lookup failure remains unexplained |
| Doorbell B | PARTIAL: earlier D3 failures remain unexplained despite reported later replay |
| F1.1B-0 | BLOCKED; independent grant prerequisites are not verified |
| Gate C | BLOCKED |

## Handoff

Completed prerequisites are the G report's bounded token history and its documented subset, plus the limited current authenticated E3 replay above. Next, add a test-only, epoch-correlated sessiond active/closed acknowledgment and Account staging drain to the existing G operation-set census, then force the missing grant-specific races with actual authorized participants and verify all inventories on the same final image. Independently investigate the original N3M lookup failure and retain D3's first failures and RecoveryBlocked execution status. Existing broker session, issuer/recipient/VM binding and bounded history are design inputs for Restricted SHM, but incomplete reconciliation and race acceptance do not authorize its implementation or activation.
