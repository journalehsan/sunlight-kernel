# F1.1B-0H1: reaper and endpoint authority investigation

Date: 2026-10-27. **INCOMPLETE: ordering, global authority and grant lifecycle remain PARTIAL; F1.1B-0 and Gate C remain BLOCKED.** This is an investigation and a single native replay, not completion of the requested phase. No production, fixture or parser code was changed. Raw new evidence and a machine index are in `target/fastlane-f11b0h1-evidence/`. The original failed H trace is preserved.

## 1. Executive summary and starting evidence

The original N6 12-CPU failure is reproduced as an existing parser failure: `lifecycle order unproven`. Its first divergent predicate requires broker session retirement before IPC peer-close. The original log instead records peer-close 14 then retirement 15 on the same CPU. Two production cleanup paths perform these actions in opposite orders. This source finding explains a plausible mechanism, but does not prove the exact path for the original attempt or establish the full security invariant. A new isolated N6 12-CPU replay on the archived H image passed with the opposite order, retirement 14 then peer-close 15; it does not cancel the original failure or prove the required race.

H's N1 1-CPU, N2 and N7C 2-CPU, and N6 4-CPU results are inherited from inspected H artifacts. H's G evidence directory is absent; a search under `target` found no alternative `*f11b0g*` path. The G report is historical report-derived evidence, not independently inspected raw results. Gate A and bounded token history remain inherited, not reverified here. 65,536 token identities per boot remain an explicit availability bound.

## 2. Original attempt and provenance

Original: `target/fastlane-f11b0h-evidence/native-20261027T192731.843270Z/N6-12cpu.serial.log`, SHA-256 `ae3ec55d9c4f2a4f089b65995e5abc1d38ede6a09222274cd99f7b11d3e70349`. ISO SHA-256 `5329c11d3e3b227ac455ee0aa3ea2205bdeca6848137b366a8eb67a338840e80`; archived kernel ELF `25cdab45703db3e63f2722cffba067a8df54cabf60718a09398c9c29c0cf913f`; manifest also records `kernel_sha256` `5f5592da35ec11e64fb0e83fe23cfd2ffcf5863621b5ea26ad1862e66e08866e` for its separately described kernel image. Features `native_session_race_test,key_inject`; environment `SUNLIGHT_INJECT_PHASE=desktop_login`, `SUNLIGHT_SESSION_N4_AP=0`. Exact QEMU argv, embedded executable hashes, and full source snapshot are retained in the H build/native manifests and copied by reference into this phase's JSON. The 1,287 files recorded in the image source map match current files by SHA-256; this is a file-map identity, not proof that the historical image was built from the current Git commit. Current HEAD is `5b745d5b803c4445458e76f5002071931d1cb6bc`. The H untracked report is preserved.

Original old Account A: PID 24, incarnation 36, VM generation 59; sessiond PID 28, incarnation 40, VM 67; session 1 epoch 1; requester PID 39 incarnation 51 VM 87; endpoint ID 21 and **request** generation 4. Endpoint generation was not logged and cannot be inferred from the request generation. Replacement B is PID 42 incarnation 54 VM 93. No raw capability or session token is reproduced here.

## 3. Original ordered timeline and interpretation

Sequence numbers come from a relaxed atomic counter (`kernel/src/capability/session_race.rs:39`), not synchronized cross-CPU timestamps. They establish counter modification order, not by themselves a global happens-before relation. The relevant 14/15 markers occur on CPU 1 and correspond to completion markers; the source order within an actual cleanup path would establish program order, but the original trace lacks a path marker.

| Seq / CPU | Event and identity | Meaning / source |
|---|---|---|
| 1 / 2 | A trusted Account activated | Role observation; not a session grant (`session_race.rs`). |
| 4 / 1 | sessiond registered 1/1 | Broker session registration completion. |
| 8–10 / 0 | A mint entry, authenticated requester and exact request binding | Request/identity observations; not yet commit. |
| 11–12 / 0 | A mint committed; boot-local fingerprint | Broker grant commit and test observation; snapshot: one session, one receipt, one pending Account delivery. |
| 13 / 0 | Manager PID 11 signal 15 | Exit request, not exit completion. |
| unsequenced | `process_mark_finished` PID 24, fatal signal 143 | Process state transition; serial position alone is not a cross-CPU clock. |
| 14 / 1 | endpoint 21, requester 39/51, request 4 peer-closed | `finish_peer_closed_calls` has called `finish_call`: this call is terminal and its pending delivery settled. This is not a broker session retirement marker. |
| 15 / 1 | session 1/1, A 24/36/59 retired, grants 0 | `retire_sessions_for` logged after removing the row and its session grants under broker guard. |
| 16 / 1 | A service IPC fields cleared | Reaper observation of its process fields; it is not a global IPC census or full reaper completion. |
| unsequenced | `process_reaped` PID 24 | Reap completion observation; no service-manager cleanup acknowledgment is emitted here. |
| 17 / 0 | B 42/54/93 activated | Trusted-role observation for a different incarnation. |
| 19–22 / 0 | B attempts old session; broker result 11 | Old session denied as `SessionAuthorityUnavailable` in this fixture. |
| 32–34 / 0,1 | B proof, new session 2/1 registered | Fresh login registration, new session ID. |
| 41–52 / 0 | New grant minted and consumed; second consumption rejected | Separate fresh-session operation, not a proof of the old reply/closure overlap. |

Sessiond's `LOGOUT PASS` exists in the trace but is not an epoch-qualified cross-service cleanup acknowledgment. Actual process exit request, `Finished`, peer-close, broker retirement, local sessiond cleanup, reaper completion, activation and fresh login are separate transitions. Broker role withdrawal is not independently logged, nor are a service-incarnation number distinct from the process tuple, endpoint generation, or all IPC delivery references.

## 4. Reaper, endpoint ownership and lock graph

`kernel/src/sched/mod.rs:2631` `close_owned_ipc_endpoints` gathers endpoint IDs and invokes broker `revoke_endpoints_owned_by` before removing endpoint queues and finishing calls. The normal self-exit path marks `Finished` then invokes it at approximately line 4319. `reap_process_resources` at lines 2787–2813 instead removes endpoint queues, completes peer-close calls, then invokes broker endpoint/recipient retirement. Both operate through scheduler lifecycle state, with broker and shard acquisitions at the shown scopes; `finish_peer_closed_calls` holds broker while settling each exact call. Reap can defer for pinned stacks, running CPUs, borrowed or active address spaces; no bounded eventual-reap proof was acquired.

`kernel/src/ipc/mod.rs:656` removes the queue and returns waiters; `finish_peer_closed_calls` at 1904 checks requester PID/incarnation and pending endpoint/request generation, then `finish_call` at 993 settles undelivered grants, clears pending call and records `PeerClosed`. Reply delivery at 860 validates the requester, endpoint, pending generation and terminal state and settles the grant before assigning the reply. These observations do not independently prove every publication/retirement interleaving or endpoint ID reuse behavior.

`kernel/src/capability/mod.rs:1021` removes broker endpoint capabilities and calls `retire_recipient_grants`; at 1053 it retires sessions, pending issuer-bound and recipient-bound grants. Delivered VFS authority has a distinct policy and is not generically revoked solely for issuer death. `sessions.rs:251` removes sessions and session grants before its marker. `lifecycle.rs:380` verifies requester and current reply binding, then holds broker through session participant liveness and account commit; `sessions.rs:273` validates exact issuer, active session and UID in that broker transaction. Scheduler-to-broker ordering and exact recipient/VM/call identity are relevant; broker locks are not held over service IPC by this investigation. `process/spawn.rs:179` performs recipient retirement for successful VM replacement; `process/exec_authority.rs` clears trusted roles. SunlightD `stop_service` (`services/sunlightd/src/main.rs:1290`) requests termination and waits for observed exit/gone before restart, with timeout/failure paths. Sessiond `begin_stop` and `finalize_stop` (`services/sunlight-sessiond/src/main.rs:586`) retain local state and make revoke calls; those paths ignore transition errors, so independent local convergence is not demonstrated.

## 5. Safety invariant and unresolved questions

An obsolete Account must not create, deliver, or revive authority after its relevant authoritative invalidation. A valid commit that wins must follow its documented subsequent grant lifecycle. A revocation that wins must prevent a later authorized commit. Endpoint closure by itself is transport cleanup and is not the session-row revocation operation. Existing code serializes account session checks and commits with broker retirement and checks process liveness, but a complete native proof of the interval, old-generation reply publication, VFS path and all required cleanup is absent. Peer-close may therefore be logged first, but safety of every permitted ordering remains unproven. Sessiond metadata can remain after broker retirement and is not itself a broker capability. The exact moment trusted issuer rights become unusable and whether another CPU can observe close while retirement remains pending require explicit boundary instrumentation. No bounded cross-service progress or endpoint-generation nonadoption proof was collected. The parser's ordering is an enforced fixture assumption; no documented security contract requiring that log order was identified, and its replacement has not been proven.

## 6. Native fixture, classification and correction

No new deterministic ordering fixture was implemented. New isolated replay: `native-20261027T195242.711044Z`, N6 12 CPUs, image and sources matched; parser `FORCED_AND_PASSED` for old and replacement session with fresh login. It observed retirement before close; this is the existing fixture's classification, not a forced peer-close-first security race. Original failed N6 remains `FORCED_AND_FAILED`. Root cause classification: **parser predicate failure explained; full production safety and exact original cleanup path insufficiently evidenced**. No production defect was established and no production or parser fix was made. Negative parser tests for a revised contract were not added because the contract is not yet proven.

## 7. N3M relationship

Original N3M failed an endpoint lookup after a mint-first race, with the old helper collapsing multiple failure classes; see `docs/ipc/fastlane-f11b0e3t-n4-n3m-closure.md:157`. H N6 first failed a parser assertion on ordering of observed broker retirement and pending-call peer closure. IPC is a shared subsystem, but the source and recorded checkpoints do not establish a common cause or prove independence. **Shared subsystem; insufficient causal evidence.** N3M remains unexplained.

## 8. Authority census and snapshot boundary

The existing census is `kernel/src/capability/reconciliation.rs`: scheduler/broker/shard inventory is distinct from service metadata and cumulative event counters. Broker owns committed and pending grant rows, delivery bindings, session and issuer-bound authorization; consumed or retired grants are terminal transitions, not extra live rows. UAC holds handler/stack/file staging and authenticated role, while broker owns grants. Sessiond owns `active`, children and `last_closed`, not broker capability state. VFS delegation is a broker record created by UAC exchange (`vfs_grants.rs:6`); VFS consumes it rather than maintaining a second authoritative delegation registry. VFS request/stack and open handles are separate resources. IPC owns endpoint queues, pending calls, reply targets and deferred cancellations. These are source ownership observations, not a correlated global snapshot.

The original session-subset snapshot after seq 15 has zero sessions, receipts, Account grants and pending Account deliveries; four retained session slots and 576 retained grant-metadata bytes are capacity, not live authority. Cumulative minted/retired counts are not current inventory. No epoch-correlated sessiond/UAC/VFS acknowledgments, full IPC endpoint-generation inventory, allocation drift or shared quiescence boundary was added. **No resources are newly reconciled globally.** R1/R2/R3/R4 remain PARTIAL or NOT_RUN at the required correlated cross-service scope; concurrent sessions and repeated bounded cycles are NOT_RUN.

## 9. Grant races and native matrix

Endpoint close versus reply publication E1/E2/E3: **NOT_FORCED** on this phase's native evidence. N6 cancels one pending old call but does not establish both publication winners and a genuine overlap with actual endpoint-generation evidence. Issuer retirement versus VFS commit V1/V2/V3: **NOT_FORCED**. VFS commit is a broker-mediated insertion and proof consumption, not an independent VFS-service authorization commit. Other unforced cases include consume/cancel, issuer exec versus VFS commit, stale slot consumption and legitimate replacement/old delivery. No fixture claims a prevented race without exercising its real boundary.

| CPUs | H1 native scope | Decision |
|---|---|---|
| 1 | No new native run | NOT_RUN |
| 2 | No new native run | NOT_RUN |
| 4 | No new native run | NOT_RUN |
| 12 | One archived-image N6 replay plus fresh login | Existing fixture FORCED_AND_PASSED; required ordering/race matrix NOT_FORCED |

A 12-CPU configuration is not evidence of 12-way contention. Participating CPUs for each marked event are in raw serial and JSON, not inferred from configuration.

## 10. Validation, failures and provenance limitations

Corrected E3 parser discovery: 14 tests PASS; G parser discovery: 4 tests PASS. Initial `python3 -m unittest tools.test_fastlane_f11b0e3 tools.test_fastlane_f11b0g` failed with module import errors; retained in the ledger and not reclassified as a pass. `git diff --check` passed at inspection. No new final-source production build, host kernel/service sweep, compatibility, D1/D2/D3, RecoveryBlocked, TLS, clippy, media, formatting or Miri checks were run. Previous failures remain historical and are not classified as matched without a diagnostic/source comparison. The replay reused the H image, not a new H1 build; its manifest reports unchanged source hashes. `target/fastlane-f11b0h1-evidence/validation-summary.json` records the limited attempt, commands, image identities and missing fields explicitly. Resource drift and full cleanup remain UNMEASURED.

## 11. Gate decisions and next blocker

| Gate | Decision |
|---|---|
| Reaper/Endpoint Ordering | PARTIAL: original safety and endpoint-generation proof absent. |
| Global Authority | PARTIAL: no correlated service/IPC quiescence. |
| Grant Lifecycle | PARTIAL: E and V critical orderings NOT_FORCED. |
| Session Authority | PARTIAL: original N3M unexplained. |
| Gate B | PARTIAL: historical D3 unexplained. |
| Gate A | VERIFIED NATIVE, inherited report. |
| Token-History | VERIFIED BOUNDED, inherited report, 65,536 identities per boot. |
| F1.1B-0 and Gate C | BLOCKED. |

The smallest next step is a bounded test-only checkpoint identifying the actual exit/reap path, endpoint generation, and broker retirement and pending reply publication boundaries, plus correlated sessiond cleanup acknowledgment. Force the reachable close/retire order without holding a spinlock across a barrier, then use real Account/UAC and VFS operations to establish the competing boundaries and inventories before revising the parser. Restricted SHM design may reference the existing identity and broker-serialization facts only with these limits; implementation or activation is not authorized.
