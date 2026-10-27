# FastLane F1.1B-0G: global authority and production token lifetime

Date: 2026-10-26 (Asia/Tehran). Evidence: `target/fastlane-f11b0g-evidence/`.
No commit or push. Restricted SHM, public Doorbell syscalls and production
FastLane remain disabled. This is a partial grant-prerequisite closure.

## 1. Executive decision

**F1.1B-0 remains BLOCKED.** This phase adds executable evidence rather than
another accounting framework: the existing F census now accompanies real
UAC/VFS service-loop acknowledgments and three repeated quiescent grant cycles;
a real pending grant reply is held over timeout and recipient exec; two
separate native images exercise global production token-history exhaustion,
concurrent final-slot issuance, and UAC error transport with batch rollback.
Both images pass the incremental 1/2/4/12-CPU matrix.

**Token-History is VERIFIED BOUNDED for one kernel boot.** Production retains
65,536 prepared identities, requiring up to 512 KiB of token words. Exhaustion
fails closed; unrestricted long-uptime availability is not accepted. The final
ordinary build excludes the test hooks and reduced-capacity configuration.

Global Authority and Grant Lifecycle remain PARTIAL. The fixture boundary covers
completed password-proof/VFS operations, not an authenticated Account session's
sessiond records and Account staging. Issuer exec versus VFS commit and legitimate
VFS replacement versus old-generation delivery still lack complete native proof.
No new production grant-authority defect was demonstrated. The token security
and availability decisions are separate; final decisions and compatibility
results are recorded below and in `validation-summary.json`.

## 2. Baseline and independent findings

Audited the current F report and machine summary, E3T, E3S, E2, E, D, C, R4 and
R4.1 reports at their actual `docs/ipc/fastlane-*` paths. F's native PID allocator
contract, 133 kernel tests, 258 service/library tests and 37 parser tests were
historical entry evidence, not substitutes for this phase's final-source tests.
The prior retired-token counterexample was repaired before this phase; no token
history is cleared or weakened here.

Preserved independently: original N3M post-race endpoint lookup failure;
historical and later D3 failures; N4 timeout before successful isolated replays;
Account Session Authority PARTIAL; native Doorbell B PARTIAL. New passes do not
explain these failures. Historical preservation hashes include 978 files from
F and E3T evidence, including their prior artifact inventories. No old evidence
file is intentionally rewritten or deleted.

## 3. Existing census coverage audit

Paths below are relative to this repository. `cap/` means
`kernel/src/capability/`; `ipc/` in kernel rows means `kernel/src/ipc/`.
Counts are inventories unless explicitly called events.

| Resource class | Owning subsystem | Creation point | Retirement point | Authoritative state source | Existing census coverage | Missing instrumentation / G addition |
|---|---|---|---|---|---|---|
| Preparing tokens / grants | Broker exclusive transaction | `cap/mod.rs::issue_token`, `prepare_shared_region`, `cap/vfs_grants.rs::delegate_vfs_grants` | Prepared drop or transaction rollback; identity remains burned | Exclusive broker borrow and prepared object | No in-progress row while that borrow is held; history includes prepared identity | Native real partial-batch failure added; no independent pending-preparation registry exists |
| Committed, delivery-pending Auth/VFS/Account grants | Broker | `cap/lifecycle.rs` production commit under SCHEDULER -> CAP_BROKER | Reply settlement, timeout/cancel, expiry, exit/exec/session retirement | `auth_session_grants`, `vfs_caps`, `account_grants`, `GrantDelivery.delivered=false` | Full vector inventory, recipient/issuer/request identities | Whole-phase per-token terminal event ledger remains incomplete |
| Delivered grants | Broker, recipient owns usable authority | `cap/mod.rs::settle_grant_delivery` exact listed reply tokens | One-use consume, explicit cancel, expiry, process/VM/session retirement | Same rows with `delivered=true` | Full row inventory and bound identities | Service drain now identifies completed fixture operations |
| Consumed one-use grants | Terminal event, no live owner | `cap/vfs_grants.rs::consume_vfs_grant`, `consume_live_*` | Row removed atomically by consumption itself | Absence plus history exclusion and native replay assertions | Events/counters, not an additional inventory category | Native consumed-then-cancel regression; no persistent consumed-row table |
| Cancelled / revoked / retired grants | Terminal outcome, no live grant row | Owner cancel, IPC `finish_call`, session transition, reaper/exec | Removal is retirement | Retain/swap-remove result under broker lock | Remaining inventory; selected fixture outcome events | G records exact deadline removal of two pending grants before exec; no comprehensive terminal ledger |
| Reusable grant table storage | Broker | Fallible vector reservations | Retained until broker lifetime ends; reused on new insertions | Vector capacities and record sizes | `[GRANT-D] tables`; separate from lengths | No total allocator leak inference |
| Delivery references and process/VM bindings | Broker + process IPC state | `GrantDelivery`, exact `IpcReplyTarget` / `PendingIpcCall` installation | Reply completion, cancellation, endpoint close, process retirement | Scalar PID/incarnation/VM/request values, not owning Process/VM pointers | Broker identities, process fields, IPC shards | No scheduler/VM reference pin is hidden in these scalar records |
| Production VFS delegation registry | Broker, not VFS service heap | Real UAC exchange; VFS presents request token to syscall 164 | Kernel consume/cancel/retire | Broker VFS rows; service `handle_request` stack | Kernel rows only in F | Source audit finds no second VFS grant registry; G observes handler boundary, actual legacy handle counts and bump high-water bytes |
| VFS pending request / recipient token copies | VFS synchronous loop | `services/vfs_server/src/main.rs::handle_request` | Handler returns; reply-and-wait settles prior target | Current message/stack plus kernel reply target | Kernel target only in F | G handler acknowledgment after prior operation; bytes on dead stack are not claimed erased or authoritative |
| VFS legacy file/open/boot handles and SHM reads | VFS / kernel SHM | VFS OPEN/read handlers, `open_meta`, BootFs | CLOSE and SHM lifetime operations | VFS State / kernel SHM rows | SHM count, not full service handle store | G reports open_meta and BootFs handle counts; these are persistent file resources, not delegation grants |
| Underlying RAMFS/FAT open handles | VFS filesystem drivers | `sunlight-fs/src/{ramfs,vfs}.rs::open` | Driver `close` | Driver-owned fixed handle arrays | Not independently counted; `open_meta` is path metadata, not this authoritative handle inventory | Driver arrays remain outside G's service census; no total legacy file-handle cleanup claim |
| UAC password proof / issuer-owned delegation / reply copies | Broker + UAC synchronous stack | `services/sunlight-uac/src/bin/uac_service.rs::handle_auth_password`, `delegation.rs::handle` | Proof exchanged, omitted reply retired, handler returns; broker retirement | Broker Auth/VFS rows; stack copies are nonauthoritative | Kernel rows and selected C events | G acknowledges prior handler completion and heap used bytes; no independent grant map exists in this service |
| UAC elevation cache | UAC | `services/sunlight-uac/src/session.rs::SessionStore::ensure_session` | Overwrite/invalidate; expired rows may remain | Fixed FnvIndexMap<UID,UserSession,16>, nonauthoritative for kernel grant minting | Outside F snapshot | G reads actual cached and currently active counts; zero is an observation, not inferred absence of instrumentation |
| Account authorization and storage staging | Trusted UAC Account handler | `services/sunlight-uac/src/bin/account_service/mod.rs::{dispatch_group,publish}` | Synchronous result/drop, close/unlink/rename paths | Local request, parsed snapshot, temporary fd/SHM; broker Account grant | Broker grants only | Common fixture set does not execute active Account staging. Unsupported mutation publication remains fail-closed. Full cross-service acceptance gap |
| Kernel session references / receipts / service incarnation | Broker session table | `cap/sessions.rs::{register_account_session,bind_account_grant}` | Session transition, expiry, owner/issuer retirement | `SessionTable.rows`, bounded requests, boot-global epoch | Session length, receipt length, lifecycle counts; capacities in E3 observation | F/G VFS fixture creates no registered Account sessions; zero is not session reconciliation proof |
| Sessiond active and retired session records | Sessiond | `services/sunlight-sessiond/src/main.rs`, `ServiceState.active` | Stop/cleanup moves metadata to bounded `last_closed` | Active SessionRecord, last_closed, catalog / component state | Outside F atomic snapshot | No G epoch-qualified sessiond acknowledgment. Smallest missing service observation |
| In-flight IPC replies / endpoint generations | Kernel IPC + server process | `ipc/mod.rs`, `process/mod.rs` immediate/deferred targets | Exact completion/cancel/endpoint close/reap | Pending call, request generation, endpoint ID, reply outcome | F counts immediate/deferred/live/tombstone subsets, stored replies | Application purpose of unrelated resident replies is not attributed |
| Queued calls / waiters / cancelled/deferred cleanup | IPC shard | Queue enqueue / waiter insert / deferred handle allocation | `finish_call`, `remove_pending_call`, reply settlement, endpoint cleanup | Queue/waiter records checked against live process incarnation and pending generation | Exact ordinary-call validity, retained capacity bytes | Notification streams are outside this snapshot. Bounded rejectable tombstones are not live authority |
| Token history | Broker for entire kernel boot | Every successful preparation in `issue_token` | Never evicted, including failed preparations | `cap/token_history.rs::words` | Length/capacity bytes and membership | G adds global real-entropy exhaustion and native SMP participants; normal 65,536 limit also replayed separately |

The census does not count cumulative commits/consumes/retirements as disjoint
current states. There is no separate Cancelled/Consumed/Retired grant table to
count. These are removal outcomes. Capacity is explicitly displayed separately.

## 4. Ownership and correlation

The broker is the sole authority for Auth/VFS/Account grant eligibility. UAC
owns policy work until broker commit; Pending delivery belongs to its exact
issuer/recipient/request tuple. Delivered VFS authority belongs to the recipient
and can survive issuer exit by policy. Account grants additionally require their
live kernel session. VFS request tokens and stale service reply bytes confer no
independent authority.

F's existing boot-local fingerprint registry is reused, never bearer values in
logs. Census rows include class, fingerprint, PID/incarnation/VM, issuer
PID/incarnation/VM, request generation and Pending/Delivered state. G service
acks add epoch, role-checked service incarnation/VM, exact controller call and
actual service inventory. Selected commit/cancel/exec/settlement events record
previous/current and terminal ownership. **This is not yet a complete per-token
event ledger:** C logs the first batch token and some terminal counters rather
than every member of every aborted batch. No absent row or zero counter is used
to claim that missing event instrumentation exists.

## 5. Consistent reconciliation boundary

The ordinary controller waits, with a fixed 20-second failure bound, for every
other fixture process to die. The kernel independently requires actual Reaped
markers for the eight spawned children and all standard/race clients. Four
service epochs surround three additional real authentication/delegation,
consume, cancellation and replay cycles.

For each epoch: UAC's real single-threaded loop acknowledges after its previous
handler/reply path completed; VFS then acknowledges its own handler boundary;
the controller makes an ordinary invalid-token AUTHORIZE_EXEC request. The
existing kernel consume observer checks zero fixture grants and runs the same
SCHEDULER -> CAP_BROKER -> one-shard-at-a-time census. Each ack is observed at
actual reply delivery and verified against the current trusted UAC or VFS role,
service PID/incarnation/VM and controller request. No roles are installed by the
fixture. The test opcode exists only in feature-enabled service builds.

This is a consistent **completed-fixture-operation-set** boundary. Service
heaps are sampled at their separate acknowledgments, not frozen simultaneously.
A single service handler cannot retain previous synchronous grant/staging work
across its return; this is source-audited, with the subsequent loop ack and real
service progress as native evidence. Unrelated persistent services continue.
No spinlock spans service IPC or a rendezvous. No tables/counters are reset.

**Visibility limit:** this fixture does not create/drain an authenticated Account
session and obtain a corresponding sessiond last_closed/staging observation.
Resident ordinary IPC may remain; notifications and global allocation events are
not frozen. Therefore this boundary cannot certify whole-system/session global
reconciliation or total allocator leak freedom.

## 6. Conservation invariants

`broker_live = ipc_tokens + shm + pending_bound + delivered_bound + legacy`.
Retiring, deferred and tombstone counts are subsets, not extra summands.
`history_used <= history_capacity <= configured_history_limit`.

Each live bound row has one exact recipient incarnation/VM; each Pending row
has a matching issuer/request unless retirement is already in progress. Queued
ordinary calls/waiters resolve an exact live pending generation. Delivered
one-use authority cannot be consumed twice. Cancellation/retirement removes
eligibility and history prevents byte reassignment. Exec withdraws old roles
and VM-bound authority before replacement VM activation. Account replacement
cannot inherit old incarnation-bound private authority. Native assertions cover
these within the tested operation set; the incomplete session and event-ledger
scope above prevents a larger claim.

## 7. Real scenario coverage

| Scenario | Executed production operation and terminal boundary | Acceptance scope |
|---|---|---|
| A1 normal issuance/consume | Real root password verification -> syscall 164 exchange -> real VFS consume | Four original cycles/client plus three drained controller cycles; one-use replay denied |
| A2 cancel before delivery | Existing real pending UAC reply cancellation | Two rows removed before publication; unchanged C/D conservation assertions |
| A3 cancel after possible publication | Delivered owner cancellation plus replay; ambiguous reply/partial listing production-method tests | Native delivered ordering and host ambiguous-settlement tests; not all ambiguous transport orderings forced |
| A4 recipient exit | Existing pre/postcommit SIGKILL and held VFS consumption | Real exit/reaper, no live fixture grant/target after final boundary |
| A5 recipient exec | Delivered-grant exec and held VFS request; new held UAC grant reply | Deadline removes two Pending grants before exec; new VM and late reply rejection observed |
| A6 issuer retirement | E3 Account stop/replacement regression; production source ordering | Account evidence separate; VFS issuer-exec-before-commit ordering still NOT_FORCED |
| A7 delivery failure | Failed late publication after timeout/exec and recipient exit | No Pending authority reappears; subsequent service progress and final census |
| A8 one-use replay | Real VFS replay and cancellation after successful consumption | Both deny/redundant cancel outcomes preserve one owner; simultaneous inside-lock race not claimed |
| A9 replacement service | Actual Account replacement/relogin in existing E3 fixture | Separate current replay; VFS replacement role admission remains BLOCKED |
| A10 repeated cycles | Four acknowledged epochs, three full controller cycles | Broker fixture inventory and observed UAC/VFS storage reconciled at each boundary |

## 8. Retained storage and allocator limits

Broker vector capacities, SHM frame-vector capacities, history capacity, IPC
queue/waiter capacity, heap used/free and free PMM frames remain in raw D/F
samples. G adds actual UAC heap used, elevation-cache counts, VFS open metadata,
boot handles and bump high-water bytes. VFS's 64 KiB bump allocator still has
no-op deallocation; reporting its high-water mark does not repair that separate
availability limit. Kernel heap/PMM samples are outside the broker snapshot to
preserve lock ordering; they are not atomic allocation/free transactions.
The VFS `open_meta` count covers remembered paths, not all underlying RAMFS/FAT
driver handle arrays. Those ordinary file resources remain outside this grant
fixture's independent inventory; no total file-handle leak conclusion follows.

Boot-start -> final growth includes resident service startup and cannot be
labelled leakage. Repeated epoch deltas narrow attribution to the completed
fixture workload, but unrelated activity and allocator fragmentation remain
possible. No global allocation/free attribution exists. An empty grant vector
therefore proves neither total memory leak freedom nor zero physical drift.

## 9. Actual production history implementation

`kernel/src/capability/mod.rs::issue_token` reserves history storage before
entropy or publication. A 64-bit conditioned word becomes a 62-bit random
payload plus a two-bit IPC/VFS tag. Zero/reserved payloads, every historical full
token value and active collisions are rejected, with 16 bounded candidate
attempts. `kernel/src/entropy.rs` seeds its existing ChaCha20 conditioner from
40 collected bytes using virtio-rng, RDSEED, or fallback RDRAND. Timing is not
entropy. The entropy source and conditioner assumptions remain those of the
existing implementation; 40 collected bytes are not a claim of measured entropy.

`token_history.rs` keeps sorted u64 words, reserves in chunks of 256, uses binary
search and inserts only under the exclusive broker borrow. It never evicts.
Successful unpublished preparations and rolled-back VFS batch identities remain
burned. Reusable grant slots do not refund history. Full history returns
`CapError::TokenHistoryFull` before drawing a new candidate; allocation failure
is separately `CapabilityStoreFull`. Authenticated VFS issuance transports
`IssuanceError::TokenHistoryFull = 2`, distinct from Denied = 1. Legacy endpoint
creation still exposes its established invalid-token failure interface rather
than a new typed userspace exhaustion API.

## 10. Lifetime, restart and entropy contract

Supported identity exclusion is **one kernel boot / the single global
CAP_BROKER lifetime**. Exact full-token non-reassignment within that domain is
deterministic, not a probabilistic uniqueness claim. Initial token
unpredictability remains probabilistic and source-dependent (62 payload bits).
Existing typed-token domains, checked process incarnations, VM generations,
request generations and monotonically allocated endpoint IDs remain intact.

Services cannot reconstruct or reset the kernel broker. Production code has
one static CAP_BROKER initializer and no supported in-boot replacement/reset.
Isolated CapabilityBroker objects in host/native unit fixtures never replace
that global. Account/UAC/sessiond restart remains in the same history domain;
old private authority still requires its original binding/session. No automatic
restart is used to recover capacity.

A full reboot destroys the broker, process/VM/IPC domains and history. There is
no persisted history or boot epoch in a bearer word. An externally saved word
could coincide with a word in a later boot; exclusion across reboot is **not
provided**. Retaining/importing raw bearer authority across reboot is not a
supported contract. Likewise arbitrary in-boot broker reinitialization is
unsupported, not tested as safe recovery. Extending the security claim to either
would require explicit persisted/qualified identity policy, outside this phase.

## 11. Capacity, memory and operational budget

Normal configuration is **65,536 successfully prepared identities**, shared by
IPC, SHM, Auth, Account and VFS issuance. Each retained word is 8 bytes; maximum
payload storage is 524,288 bytes (512 KiB), plus a Vec header (24 bytes on x86_64)
and allocator metadata/alignment. This is 3.125% of the current 16 MiB kernel
heap for token words. Live authority table memory is additional. Sorted insert
may move 65,535 words (524,280 bytes) under serialization; contention latency at
the production maximum is not benchmarked here.

The reduced native configurations use 1,024 identities / 8,192 token bytes.
Only the limit changes: real global history reservation, insertion, token
identity, entropy, lookup, rollback, authorization and failure policies execute.
No entropy override, table clearing, retired-word eviction or role bypass is
introduced. Normal production builds exclude every G hook and retain 65,536.

## 12. Exhaustion and replay tests

Two images isolate otherwise irreversible exhaustion cases; neither resets
history. `grant_history_test` runs after real clients/reapers complete. It makes
and retires one real global endpoint, retains another as a usable sentinel,
and burns production prepared SHM identities until 1,023. Timer entries from
every selected CPU rendezvous through atomic ready bits **outside** scheduler
and broker locks. After all arrivals each participant invokes the real global
broker create method. Exactly one allocates the final identity; the others
return explicit TokenHistoryFull. The winner retires its new endpoint and
checks replay; every participant checks permanent failure, no live/endpoint
row growth, oldest replay denial and sentinel usability. Final sentinel
retirement restores the measured pre-test live inventory.

This is real global-kernel-broker SMP execution, not parallel userspace endpoint
syscalls and not measured simultaneous lock contention. One CPU is sequential;
2/4/12 have independently recorded CPU arrivals and outcomes. Serialization is
the production broker guard. No barrier waits with that guard held.

`grant_history_transport_test` uses the same real controller's root password
proof after epoch four. Under the existing broker transaction, only test setup
burns preparations to 1,023. Actual UAC BASE_CAPS delegation prepares/inserts its
first VFS row at 1,024, then fails preparing the second. The actual production
rollback truncates the unpublished batch, retains the delivered proof and
transports explicit error 2 / zero caps. The controller repeats the same real
request at full capacity: the same error, no new VFS row, proof still retained.
Ordinary exec then retires that proof and the actual reaper completes. This
separately verifies service-level exhaustion rather than inferring it from the
kernel-only contention image.

| Test | Evidence and limit |
|---|---|
| H1 below capacity | Real production UAC/VFS operations before prefill |
| H2 near capacity | Real preparation through 1,023; existing sentinel/proof remains usable |
| H3 exact boundary | One final endpoint succeeds; separate image inserts then rolls back one VFS batch member |
| H4 beyond capacity | Explicit broker failure and real client IssuanceError 2, repeated permanently |
| H5 concurrent final slot | Native global broker methods after all 2/4/12 CPU ready checkpoints; one winner |
| H6 rollback | Real two-member VFS delegation fails on member two; zero partial rows, original proof retained |
| H7 oldest retired replay | Global retired endpoint denied after exhaustion; isolated repeated-candidate exclusion regression |
| H8 newest retired replay | Final winning global endpoint retired and denied; history retained |
| H9 slot recycling | Actual removal/new endpoint insertion plus full-capacity/repeated-candidate production-method regressions; history never refunded |
| H10 lifecycle | Existing service replacement regression, same global boot history; no in-boot broker reset or across-boot exclusion claimed |

The global contention probe's `oldest` means its first retired endpoint after
the client drain; it is not a claim to have captured the earliest retirement
of the whole boot. The isolated normal-capacity boundary starts from a fresh
unpublished production broker and tests its first and last retired identities.
Source/history membership and repeated-candidate regressions supply the shared
exact-exclusion invariant across both scopes.

Normal-capacity host and isolated native production-broker boundary tests are
replayed separately. That existing isolated boundary supplies deterministic
test candidates to the real broker; it never replaces CAP_BROKER or publishes
those candidates. The new reduced-capacity global images use real production
entropy. Reduced-capacity success does not measure long-running production
availability or maximum-capacity contention performance.

## 13. Security and availability decisions

Keep the existing bounded fail-closed design (Option 1); do not increase capacity
or redesign identity construction. Security acceptance is limited to the boot
lifetime described above. No production traffic rate has been measured.

Illustrative initially-unused budget ceilings, not workload measurements:
1 preparation/second exhausts in 18.20 hours; 10/second in 1.82 hours;
1/minute in 45.51 days; 100/day in 655.36 days. Boot and failed work reduce all
ceilings. An initially-unused budget allows at most 65,536 preparations, not
65,536 application launches or VFS grants. One base exchange produces two VFS
identities and requires its separate password proof/SHM transport; other IPC
and service setup can consume more.

Low-frequency service setup may fit a finite boot budget. Repeated app launches,
Channel create/teardown or long uptime have no demonstrated admission budget.
Invalid proofs/path requests rejected before preparation do not spend history;
valid preparations abandoned or failed later do. Authorized endpoint/SHM churn
can spend the shared budget despite recycling live records. There is no
per-issuer lifetime budget or supported in-boot renewal mechanism. Thus
availability is **finite and not accepted for unrestricted long-running
production authorization workloads**, independently of replay safety.

## 14. Remaining lifecycle race inventory

| Invariant / production entry | Competing operation / intended ordering | Current evidence and classification | Smallest missing work |
|---|---|---|---|
| UAC reply publication cannot authorize exited recipient | SIGKILL/reap vs pending reply | Existing real pre/postcommit paths replayed: FORCED_AND_PASSED | Preserve exact request/reap assertions |
| Old VM cannot receive a Pending grant reply | Real call deadline -> exec -> late UAC reply | G native cancellation-before-exec observed. Live-Pending-across-exec: PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT for this synchronous caller path; late reply exclusion FORCED_AND_PASSED | Do not defeat cancellation to manufacture a live pending ordering |
| Old VFS request cannot consume after recipient exit/exec | Held real VFS consume vs retirement | F held exit/exec paths replayed: FORCED_AND_PASSED | No claim of interleaving within serialized broker operation |
| Consume/cancel has one terminal owner | Real VFS consume then owner cancel; cancel then VFS replay | Both ordinary terminal orders asserted; inside-lock competing-entry race NOT_FORCED | Add explicit competing-operation ready/finish evidence if complete native overlap acceptance is required |
| Retirement versus late completion cannot resurrect | Pending reply cleanup and later service progress | Exit and new timeout/exec subset FORCED_AND_PASSED; global general case PARTIAL | Exact endpoint-close/retirement operation-set fixture |
| Pending reply versus endpoint closure | Endpoint destroy/peer-close vs completion | Existing IPC/authority regressions; new dedicated native ordering NOT_FORCED | Reuse rendezvous around ordinary endpoint destruction |
| Issuer retirement/exec cannot commit private old authority | `Scheduler::try_delegate_vfs_grants` vs UAC lifecycle | Shared scheduler/broker serialization source; VFS-specific native issuer exec ordering NOT_FORCED | Reuse authenticated root manager, not the UID-1000 grant controller, to retire actual issuer at existing service boundary |
| Recycled grant slot cannot revive old bearer | Remove/reinsert vs late consume | History/replay native tests and production methods pass; simultaneous late-consume slot race NOT_FORCED | Bind old-request late consumption to a recorded replacement insertion |
| Replacement service cannot adopt old private state | Real Account replacement and pending old-generation delivery | Existing N5/N6 current replay recorded separately; VFS replacement BLOCKED | Legitimate VFS replacement-role admission contract, not forged trusted flag |

N4 remains the existing cooperative-stop **prevention** proof. It is never
labelled forced retirement-before-commit. Session Authority's original N3M
endpoint failure is not reclassified as a grant defect without causal evidence.
Essential NOT_FORCED cases keep complete Grant Lifecycle acceptance PARTIAL.

## 15. PID and VM regression contract

No allocator change. Ordinary dynamic PID selection is checked max-retained+1;
reaped rows preserve the maximum through replacement, no runtime numeric reuse
is introduced. Native fixture spawns/reaps eight real children per image/CPU,
with increasing PIDs and distinct process/VM identities. Checked isolated native
allocator MAX boundary still refuses rollover/exhaustion; live full-width table
exhaustion is not fabricated. Real exec changes VM generation, retires old
bindings and withdraws image roles. Genuine numeric PID reuse remains NOT_FORCED;
ordinary runtime reuse is prevented by the established native-tested contract.

## 16. Native matrix and measured inventories

The generated results appendix below identifies every accepted run and contains
real inventories, storage samples and image identities. Final F census occurs
before kernel-contention exhaustion in one image and after transported
exhaustion/proof-exec in the other. These are different checkpoints, explicitly
labelled; their history lengths are not interchangeable. Source identities are
verified before/after each run and across the final sweep.

## 17. Compatibility and tooling

Final-source compatibility uses the existing E3/E2 builders, parsers and gates;
source/image/embedded hashes and all commands are retained per attempt. Builds
are serialized to avoid overlapping embedded userspace staging. Gate A stays
VERIFIED NATIVE. Doorbell B stays PARTIAL regardless of new smoke passes.
The lifecycle collector now records RecoveryBlocked NOT_RUN if its D3
prerequisite fails; it does not reuse a historical RecoveryBlocked result.

Strict Clippy and workspace formatting failures remain actual failed checks;
unchanged diagnostic-source comparisons and changed-line formatting checks are
separate evidence. Missing WAV fixture and unavailable Miri remain failures /
unavailable, not waived successes. Current replay details follow in the appendix.

## 18. Failures, timeouts and retries

All failed builds and native attempts remain in new timestamp directories.
Initial diagnostic errors included kernel/userspace IpcMsg type mismatch and a
wrong enum spelling in the optional transport probe. The first sandboxed native
attempt timed out before any serial output. The direct diagnostic boot reached
the fixture and failed its incorrect `old_grants == 2` exec assertion: actual IPC
timeout had already retired those two pending grants. This was a fixture
assumption error, not evidence of stale authority. G now observes the actual
terminal IPC removal, checks zero grants at exec, and records both boundaries.

A subsequent guest run passed G/F/C but the old D parser rejected extra cycle
memory rows. G reuses D's parser on its original start/final scope while
preserving intermediate raw rows. The composite failed attempt remains failed;
final accepted matrices replay all relevant parsers. A mistyped manifest path
failed before QEMU launch and is retained as a driver attempt. The shared Cargo
cache briefly delayed the final sweep; unrelated builds were not killed or
cleaned. Final source scope and any later failures are stated in the appendix.

Final review also removed a misplaced `d3_ok` check accidentally duplicated into
the separate E2 production collector branch. It could have raised an undefined
variable error there; the exercised lifecycle branch was correctly scoped.
`collector-branch-check.json` executes the actual branch AST with stubbed build
and native runners, testing production and both D3 outcomes. This is host
collector logic evidence only. The correction changes no compiled kernel or
service code. Exact final-source fixtures and affected collectors are replayed;
earlier attempts retain their source mismatch flags.

## 19. Source/image and evidence integrity

Source hashes cover all tracked/untracked Rust, TOML, Python, shell, assembly,
linker and boot configuration inputs as in the existing collector. Each fixture
build archives stripped ELF, ISO and embedded init/SunlightD/TTY/UAC/sessiond/VFS/
grant-probe executables, verifies embedded byte offsets and records hashes.
Normal and compatibility images record separate feature configurations and
hashes. No evidence from incompatible source snapshots is used as a final pass.
Early diagnostic attempts are still indexed. Three formatting-only diagnostic
hunks and token-bearing failure-assertion formatting were corrected during the
first core replay. That attempt is retained with a false whole-source stability
flag; affected checks are replayed against the final frozen source. New
assertions use boolean predicates so an unexpected successful mint cannot print
a bearer value in panic diagnostics. Historical artifact preservation
is rechecked at consolidation. Disk was checked before work: 26 GiB available;
no broad cleanup or deletion of old evidence was performed.

## 20. Acceptance and smallest continuation

| Gate | Decision |
|---|---|
| A scheduler | VERIFIED NATIVE |
| B native Doorbell | PARTIAL; D3 findings remain unexplained |
| Session Authority | PARTIAL; original N3M finding remains unexplained |
| Global Authority | PARTIAL; real completed UAC/VFS operation set reconciled, active Account/sessiond/staging and complete terminal correlation still missing |
| Token-History | VERIFIED BOUNDED for one kernel boot; availability remains finite |
| Grant Lifecycle | PARTIAL; essential issuer/endpoint/replacement/competing-entry cases remain unforced |
| F1.1B-0 | BLOCKED; not INDEPENDENT_GRANT_PREREQUISITES_VERIFIED |
| C production FastLane | BLOCKED |

A bounded continuation should extend **this same epoch/ack census** to an
existing authenticated Account/sessiond fixture: observe active/last_closed
session metadata and Account operation/staging completion, correlate all batch
members and terminal outcomes, then run three quiescent cycles. Separately add
the missing exact endpoint-close/issuer and cancellation entry rendezvous using
existing authorized management paths. Do not open another accounting framework.
VFS replacement cannot be closed by assigning a trusted flag; it needs an
explicit supported admission/retirement contract.

## 21. Next-stage handoff

Track A, Session Authority: preserve and investigate original N3M's first
post-race endpoint lookup failure with exact caller/endpoint/request/activation
identities. A later isolated replay does not explain it. Reuse G's epoch
observations when adding Account/sessiond reconciliation, without upgrading
Session Authority from accounting alone.

Track B, native Doorbell: investigate historical and later D3 first-failure
checkpoints under their exact source/image identities. Preserve RecoveryBlocked
ordering and current NOT_RUN when its prerequisite fails. Grant/history passes
supply no explanation of D3 causality.

Track C, Restricted SHM preparation only: specify nontransferable issuer/
recipient/VM binding, explicit per-mapping read/write rights, partial-map
rollback, active PTE revocation and remote TLB completion, pin release before
frame reuse, Channel generation/peer closure and native cross-process teardown.
Do not implement or enable production mapping/transport while authority gates,
Doorbell acceptance and native Channel validation remain incomplete.

## 22. Final measured results

Validation spans October 26–27, 2026. Only exact final-source, unchanged-before/after
G/F/C/D parser passes enter this table. Superseded attempts remain indexed.

| Feature / CPUs | Accepted native attempt | IPC / SHM | Pending / delivered | Pending / queued calls / waiters | History used / capacity |
|---|---|---|---|---|---|
| grant_history_test / 1 | `native-20261026T143721.785674Z` | 60 / 4 | 0 / 0 | 0 / 0 / 0 | 127 / 256 |
| grant_history_test / 2 | `native-20261027T074054.025010Z` | 60 / 4 | 0 / 0 | 0 / 0 / 0 | 150 / 256 |
| grant_history_test / 4 | `native-20261027T074112.034769Z` | 60 / 4 | 0 / 0 | 0 / 0 / 0 | 196 / 256 |
| grant_history_test / 12 | `native-20261027T105813.064580Z` | 60 / 4 | 0 / 0 | 1 / 0 / 1 | 380 / 512 |
| grant_history_transport_test / 1 | `native-20261027T105833.432128Z` | 60 / 4 | 0 / 0 | 0 / 0 / 0 | 1024 / 1024 |
| grant_history_transport_test / 2 | `native-20261027T105855.014293Z` | 60 / 4 | 0 / 0 | 0 / 0 / 0 | 1024 / 1024 |
| grant_history_transport_test / 4 | `native-20261027T105932.075006Z` | 60 / 4 | 0 / 0 | 0 / 0 / 0 | 1024 / 1024 |
| grant_history_transport_test / 12 | `native-20261027T110021.810809Z` | 60 / 4 | 0 / 0 | 1 / 1 / 1 | 1024 / 1024 |

Each accepted cell requires zero fixture grants at four acknowledged epochs,
actual child reaps and zero stale queued identities. Resident IPC is displayed
without resetting it. The driver diagnostic call itself is owned and counted.

| Feature / CPUs | Cycle heap-used bytes | Cycle free frames | Final VFS / Auth retained entries | IPC queue/waiter retained bytes |
|---|---|---|---|---|
| grant_history_test / 1 | 1743088,1743280,1743280,1743280 | 30591,30589,30589,30589 | 8 / 8 | 7616 |
| grant_history_test / 2 | 1746800,1744464,1744464,1744464 | 30313,30311,30311,30311 | 8 / 4 | 8160 |
| grant_history_test / 4 | 1768520,1768616,1768616,1768616 | 29821,29819,29819,29819 | 16 / 4 | 7616 |
| grant_history_test / 12 | 1925352,1925544,1892872,1892776 | 27231,27229,27751,27751 | 32 / 16 | 12608 |
| grant_history_transport_test / 1 | 1744584,1744776,1744776,1744776 | 30571,30569,30569,30569 | 8 / 8 | 8704 |
| grant_history_transport_test / 2 | 1776160,1776352,1776352,1776352 | 30313,30311,30311,30311 | 8 / 4 | 8704 |
| grant_history_transport_test / 4 | 1770368,1770464,1770464,1770464 | 29801,29799,29799,29799 | 16 / 4 | 8704 |
| grant_history_transport_test / 12 | 1900856,1901048,1901048,1901048 | 27753,27751,27751,27751 | 32 / 16 | 10432 |

Measured service tuples `(service, records, active-or-boot-handles, heap/bump-used)`:
`[('uac', 0, 0, 0), ('vfs', 0, 0, 350)]`. These stores stayed stable over the three additional
cycles. Zero session/receipt inventory here does not prove Account cleanup.
Kernel heap/frame deltas remain unattributed; allocation/free events and
fragmentation are not measured, so total allocator leak freedom is not claimed.
For the contention image, the D/F final census precedes prefill. H-G completion
separately records 1,024 retained words / 8,192 bytes and live-authority baseline
restoration, without a second complete allocator census. Transport samples follow
the failed batch, proof exec and reap.

## 23. Token decision and image integrity

**Token-History: VERIFIED BOUNDED.** Security acceptance combines real global
production-code exhaustion/rollback, native CPU rendezvous, exact history
retention, normal-capacity isolated production-method boundary tests, replay and
slot regressions, and same-kernel service replacement. It applies only to one
kernel boot. Across-reboot token exclusion and broker reset are unsupported.
Availability remains finite and is not accepted for unrestricted uptime.

- `grant_history_test`: ELF `4a2e26b68788011416f6c4a72f4c856a58be57dfe990bb8ef5d5fd5c79e51367`, ISO `c39cbc423bee9164ae4f9b0d2b5d5bfcbcdd5369418ee47091f693c0eefd00fa`.
- `grant_history_transport_test`: ELF `97a1caced7f905e17faaddde319056808975188f41fe34835fa299fbd6862e36`, ISO `9e7340df0a533d5a84d31926bd1fa623bcf7ebcdfecff3b41d2f5cc13a8496d0`.

Embedded UAC/Account, VFS, sessiond, init, SunlightD, TTY and probe hashes are
in the build manifests. `ordinary-instrumentation-scan.json` checks archived
ordinary ELF/embedded bytes and absence of G/E3 markers. `final-metrics.json`
contains all accepted source hashes, inventory rows, correlation identities,
service epochs, history outcomes and memory samples.

## 24. Final host and compatibility replay

| Check | Result | Qualification |
|---|---|---|
| kernel-host | PASS, exit 0 | 133 tests; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/kernel-host.log` |
| services-host | PASS, exit 0 | 258 service/library tests; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/services-host.log` |
| scheduler-host | PASS, exit 0 | focused scheduler host gate; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/scheduler-host.log` |
| parsers | PASS, exit 0 | 41 tests including negative G parser cases; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/parsers.log` |
| strict-clippy | FAIL, exit 101 | known failures in unchanged diagnostic sources; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/strict-clippy.log` |
| media-host | FAIL, exit 101 | 36 pass; missing WAV fixture fails one test; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/media-host.log` |
| workspace-format | FAIL, exit 1 | existing workspace differences; changed Rust additions separately clean; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/workspace-format.log` |
| diff-check | PASS, exit 0 | whitespace check; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/diff-check.log` |
| miri | UNAVAILABLE, exit 1 | component unavailable; `target/fastlane-f11b0g-evidence/checks-20261026T143706.443515Z/miri.log` |
| authority | PASS | `target/fastlane-f11b0g-evidence/compatibility/authority-20261027T110138.656147Z/validation-summary.json` |
| core | PASS | `target/fastlane-f11b0g-evidence/compatibility/core-20261027T110354.122233Z/validation-summary.json` |
| network | PASS | `target/fastlane-f11b0g-evidence/compatibility/network-20261026T142905.381927Z/validation-summary.json` |

Authority: entropy-unavailable fail-closed and normal 65,536 boundary on 1/2/4/12
CPUs. Core: pin release, Doorbell and scheduler on 1/2/4/12, VM/TLB on 12, unmap
and protect on 4. Network: resident liveness on 12, native TCP/TLS certificate
verification on 1/2/4/12. These are focused source scopes, not a workspace-wide
runtime claim. Detailed commands, results and images remain in each manifest.

- `target/fastlane-f11b0g-evidence/n4/native-20261027T112604.999198Z/validation-summary.json`: `[('N4', 2, 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT'), ('N4', 4, 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT'), ('N4', 12, 'PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT')]`.
- `target/fastlane-f11b0g-evidence/session/native-20261027T111955.423218Z/validation-summary.json`: `[('N1', 2, 'FORCED_AND_PASSED'), ('N2', 2, 'FORCED_AND_PASSED'), ('N3M', 2, 'FORCED_AND_PASSED'), ('N3R', 2, 'FORCED_AND_PASSED'), ('N5', 2, 'FORCED_AND_PASSED'), ('N7R', 2, 'FORCED_AND_PASSED'), ('N7C', 2, 'FORCED_AND_PASSED')]`.
- `target/fastlane-f11b0g-evidence/session/native-20261027T112450.017751Z/validation-summary.json`: `[('N6', 2, 'FORCED_AND_PASSED')]`.

N6 includes actual replacement and fresh authenticated login. N4 retains the
production-prevention classification; it does not force retirement before commit.
Current isolated N3M success does not explain its original lookup failure.

## 25. D3 failures and interrupted attempts

Final-source lifecycle `target/fastlane-f11b0g-evidence/compatibility/lifecycle-20261027T110944.420856Z/validation-summary.json`: **D3 PASS;
RecoveryBlocked PASS.**

Recovery-image per-CPU execution (separate from the dependency image):

- 1 CPUs: D3 PASS; RecoveryBlocked PASS. `target/fastlane-f11b0g-evidence/compatibility/lifecycle-20261027T110944.420856Z/recovery/serial-1cpu.log`.
- 2 CPUs: D3 PASS; RecoveryBlocked PASS. `target/fastlane-f11b0g-evidence/compatibility/lifecycle-20261027T110944.420856Z/recovery/serial-2cpu.log`.
- 4 CPUs: D3 PASS; RecoveryBlocked PASS. `target/fastlane-f11b0g-evidence/compatibility/lifecycle-20261027T110944.420856Z/recovery/serial-4cpu.log`.

`final-d3-results.json` retains exact final source, image/embedded hashes and
commands. Earlier G D3 also failed at `d3-leaf phase 0 actual 2 failure 0` on
2 CPUs after a 1-CPU pass; 4 CPUs and RecoveryBlocked were NOT_RUN. Its
`current-d3-failure.json` remains unchanged. The separate final source comparison
records three later collector/parser-only changes and does not claim the entire
old D3 runner file is byte-identical. No causal connection to G grant changes
is established. Historical/later D3 failures and original N3M remain unexplained.

Earlier failed D3 serial: `target/fastlane-f11b0g-evidence/compatibility/lifecycle-20261026T135043.937809Z/d1-d2-d3/serial-2cpu.log:5275`; serial SHA-256
`34f6724f62d4c244b3d1399ae73ee0a41f6bc69c402bb6f079e7f945df78ee0f`. Its ELF SHA-256 is
`22f8fe8556cfbe3764f9130c47f368502d091c316fa408080ed3a7d13ce77f06` and ISO SHA-256
`4046b828b4dede47479e885b9e156debdf6967c82d20208c88a1538f6a7003ea`. The full old source and embedded identities remain in
`current-d3-failure.json`; this is independent failed evidence.

The interrupted continuation completed grant CPUs 1/2/4. Its first 12-CPU
attempt produced no serial and QEMU terminated on signal 1; it is NOT_RUN,
not PASS. The new bounded retry is separately archived. No old failed or
source-unstable attempt is promoted by a later pass. The original N4 timeout
remains preserved.

## 26. Final conservation decision and handoff

Historical F/E3T inventory: 978 files,
0 changed/missing. Earlier 453-file inventory:
0 changed/missing. Requested baseline reports/summary:
10 checked, 0 changed/missing. Final source freeze matches:
True. No broad cleanup was performed.

The initial added-line formatting result was overwritten by its clean rerun;
`format-initial-result.json` explicitly transcribes that three-hunk failure and
its limitation. It is not a preserved raw checker output. Historical evidence
was unaffected. Collector-review repair and stubbed host branch checks are
separately recorded; those are not native evidence.

**Global Authority: PARTIAL. Grant Lifecycle: PARTIAL. F1.1B-0: BLOCKED.**
Gate A remains VERIFIED NATIVE, Gate B and Session Authority remain PARTIAL, and
Gate C remains BLOCKED. Token security acceptance does not accept unrestricted
availability or authorize production transport.

The smallest accounting continuation extends this same epoch/ack census to
Account/sessiond active/last_closed/staging ownership and correlates every batch
member through a terminal outcome. Missing exact endpoint-close, issuer-commit,
consume/cancel and late-slot races need existing authorized management paths.
VFS replacement requires a supported admission contract, never a forged role.
Track A investigates N3M, Track B investigates D3, Track C prepares mapping and
active revocation requirements only. Restricted SHM and production FastLane
remain disabled. No commit or push was performed.
