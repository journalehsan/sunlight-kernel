# KFP-2A.4 — MAP_FIXED transaction closure and native runner recovery

**Overall: PARTIAL.** Gate verdicts and final run counts are recorded below and in `.kfp-2a4-evidence/results.json`. Production behavior changed only in anonymous-mapping rollback failure containment. Whole-transaction persistent ownership is **not closed**. Production FastLane remains disabled. No commit or push.

## Baseline and evidence boundary

Baseline HEAD: `882b40a65f55951b5cd8b9978c05580e71202488`, November 10, 2026. The incoming tree had 15 modified tracked files, the untracked retirement journal, KFP-2A through KFP-2A.3 reports and evidence. `status-before.txt`, `preexisting.patch`, and `baseline.json` preserve that state. KFP-1 through KFP-2A.3 and `.kfp-2a3-evidence/continuation/results.json` were reviewed. Their tests are historical, not final-source evidence.

The initial broad hash capture includes prior untracked evidence snapshots. The current runner instead hashes all tracked/nonignored source/build inputs outside dot evidence and `target/`, retaining `.cargo/`. This is the same source-domain correction documented in KFP-2A.3; it excludes generated snapshots, not production code. Each native attempt checks that map before/after build and execution, binds ELF/ISO hashes, extracts the ISO kernel for byte comparison, and verifies each selected service, probe and shell is actually embedded. Commands, cwd, environment overrides, separate stdout/stderr, exit statuses and source hashes are retained per attempt. No secret tokens are added to receipts.

## Gate A: shell prerequisite diagnosis

The historical failures were guest launch prerequisites, **INFRASTRUCTURE_FAILURE**, with security cases **NOT_RUN**. Host Python, executable permissions, interpreter, Rust toolchain, working directory, QMP and the original source/image checks had already passed in the recorded KFP-2A.3 attempts. The original serial evidence reports `[SPAWN] Request ... path=/bin/sshl0 ... caps=0x40000`, followed immediately by `[TTY] Spawning /bin/sshl FAILED`, without an ELF `Loading` message.

`services/tty_server/src/main.rs::spawn_tab` uses `SPAWN_AUTHENTICATED`. `handle_spawn_call` consumes its live login proof but later requires either a trusted session-service `SPAWN_SESSION_INITIAL/BOUND` attachment or inherited `account_membership`. Its final branch explicitly rejects `authenticated_session` without either attachment. The boot TTY lacks Account membership. Thus proof consumption succeeds but publication is denied **before** `spawn_with_session_attachment` and ELF loading. This is a source-established protocol mismatch, not a project build failure or evidence that retirement changes caused the historical failure. Repairing the production TTY session protocol is a separate change.

### Test infrastructure correction

An explicit `kfp-security-runner` UAC fixture reuses the existing `m53r41` session-launch fixture. Real login/session registration, bootstrap authorization, stale/replay denials, four launch rollback probes and component HELLO/READY remain on their ordinary paths. The resulting ordinary Account member spawns the real `/bin/sh -c` shell with `touch /tmp/session-race-probe && /tmp/session-race-probe CASE`. The shell runs and collects the unchanged original native clients. The fixture collects the actual shell exit and emits a structured `shell-collected` receipt and the existing exit-marker format. It never infers success from a security terminal marker. This replaces interactive TTY launch/keyboard driving only; it is explicitly not evidence of repaired interactive login.

`tools/kfp2a4_native.py` uses the original M1/M2/M3/M4/M5 assessors. No security case or assertion is deleted or relaxed. The Account bootstrap feature is added to the original per-suite debug configuration; production admission is unchanged. No client receives trusted service roles, forged membership, an unrestricted lookup profile, or raw broker access. Source-integrity failures remain fatal. Builds and guests run sequentially because embedded-service target outputs are shared.

Preserved attempts include the first IPC-driven shell controller: Account launch succeeded, but looking up `sshl` was correctly denied to the ordinary client. The correction uses supported noninteractive shell execution without adding lookup authority. A subsequent combined-feature attempt enabled M5 features for M1/M2; M2/M3 passed, but M1 observed IdentityChanged terminal reasons in exit races. That broader configuration is retained separately; it cannot qualify the original per-suite configuration. The final driver restores the original per-suite features.

## Gate B: actual transaction and intended ABI

Source chain: native syscall 50 → syscall `sys_mmap` → `process/mmap.rs::sys_mmap` → `map_anonymous_kind`. Helios syscall 9 → `translate_mmap_flags` → the same syscall wrapper and mapper. `sys_mmap` holds SCHEDULER then PMM throughout. Scheduler exclusion serializes mapping/exit mutations; it does not prevent userspace on another CPU from observing individual PTE publications.

| Step | Actual behavior | Reversibility and ownership |
|---|---|---|
| Input/protection | Private anonymous only; fd=-1, offset=0; unsupported flags rejected; W^X and representability checked | No mapping mutation |
| Range | Checked page layout, alignment, arithmetic, canonical user span; fixed/noreplace mutually exclusive | No mapping mutation |
| Ledger discovery/policy | `preflight_replace` stages a complete bounded ledger image and rejects protected overlap | Ledger plan is heap-backed and caller-owned; not a retirement reservation |
| Per-leaf validation | `expected_replaceable_leaf` checks anonymous/brk MAY_REPLACE, recorded owner, PMM owner, PTE permissions, and swap existence | Performed during installation, not a complete pre-withdrawal backing snapshot |
| Reservation | Reserve installed-vector and swap-candidate capacity | No MAP_FIXED journal reservation, no all-frame/table reservation |
| New frame | Bounded direct reclaim, one owned allocation, sanitize | Caller local owns unpublished frame; PMM metadata records PID/class |
| Existing PTE | `replace_mapping(...RetainOld)` matches exact leaf and installs new PTE | Old frame/swap identity is initially on call stack; installed vector is updated only after synchronous return |
| Synchronization | `invalidate_page`, existing exact-sequence remote protocol and fail-stop | Successful return precedes old backing release; no MAP_FIXED transaction token is bound |
| Hole/table installation | `map_page` may allocate up to three tables; partial creation rollback uses existing journal | Capacity failure of table rollback keeps tables attached; installed-leaf rollback is a different path |
| Ledger commit | `commit_replace(plan)` only after every installation | Logical ledger commit point; **not** an atomic multi-PTE publication point |
| Old reclamation | After ledger commit, loop frees displaced frames/discards old swap backing | Old identities reside in local installed vector; no durable operation record |
| Rollback | Reverse installed leaves: exact new leaf → old mapping, synchronize, free new frame; holes use `rollback_mapped_page` | Can itself fail; successful restoration invalidates again instead of assuming earlier TLB generations suffice |
| Accounting | PMM owner/class/free counters; swap candidate untrack/track/discard | No shared-quota reservation/commit/refund occurs for private anonymous MAP_FIXED |

The code explicitly promises normal ENOMEM rollback preserving the original region and publishes its ledger only after complete installation. Therefore an implementation must preserve whole-operation rollback intent; it cannot silently stage partial committed chunks. This is an implementation-derived contract, not an imported Linux/POSIX atomicity guarantee. Remote observers can see intermediate PTEs today; simultaneous visibility atomicity is **not provided/proven**. Caller serialization and remote translation synchronization are separate properties.

Native errors return `u64::MAX`. Helios validates the full flag word, removes only advisory MAP_STACK, preserves fixed/noreplace, rejects the native-thread flag, and reaches the same ownership/authorization checks. Linux hintless non-PROT_NONE anonymous mappings are demand-backed; fixed mappings remain eager. Helios returns ENOMEM=12, EACCES=13, EEXIST=17, EPERM=1 for Protected, otherwise EINVAL=22. Translation does not grant permission to replace SHM, framebuffer, telemetry or other protected records. Brk uses the mapper but deliberately retains collision semantics rather than replacement.

## Narrow production correction

Only `kernel/src/process/mmap.rs::rollback_anonymous` changes production execution in this milestone.

* Original defect: `installed.pop()` removed the authoritative local descriptor before restoration. A missing process or failed exact restoration returned false, skipped the frame release, then silently continued; the entire vector could empty and an ordinary mmap error return with a changed PTE and no displaced-backing locator.
* Call chain: eager native/Helios anonymous-map allocation, sanitize, validation, install or ledger-commit failure → rollback. This also covers nonfixed anonymous rollback through the shared helper.
* New order: copy the last descriptor, attempt exact restore/unmap and its synchronization, assert successful restoration, release the new frame, then pop. A failure enters the existing nonreturning kernel panic while the descriptor and remaining prefix still exist and outer scheduler/PMM exclusion remains held.
* Closed gap: silent normal return and descriptor discard after a reported rollback failure. Duplicate cleanup after complete rollback remains empty/no-op. Successful behavior and permission checks are unchanged.
* Boundary: this is **fail-stop containment**, not a persistent transaction owner, recovery protocol, or permission to reclaim after a stop. Kernel panic does not unwind. The host catch_unwind probe only observes the pre-stop control flow with adapters; it does not model resumable kernel execution.
* Regression: exact function extraction against controlled scheduler/PTE/PMM adapters, two failing safety cases before the change, all four after. Real hardware restoration failure remains unforced in guests.

No APIC retry/recovery, background reclaim, journal capacity increase, additional global wait lock, or scheduler algorithm change is introduced.

## TX invariant disposition

| Invariant | Disposition |
|---|---|
| TX-1 pre-reserved ownership | OPEN: MAP_FIXED does not reserve a durable whole-operation owner |
| TX-2 no ownership loss | PARTIAL: reported rollback failures retain descriptors and stop; ordinary snapshots/new frames still have local-only intervals |
| TX-3 no premature reclamation | Source order requires successful per-page synchronization before old/free-new release; independent full transaction/table fault proof OPEN |
| TX-4 safe rollback | PARTIAL: exact-match restoration with another invalidation; restoration failure now stops instead of returning. Whole-range/durable rollback OPEN |
| TX-5 exactly-once release | Scoped host rollback and journal guards PASS; all frame/table/swap guest identities OPEN |
| TX-6 exactly-once accounting | Scoped release-once host control PASS. No private-map SHM quota refund exists; full PMM/swap/table transaction receipts OPEN |
| TX-7 no stale completion | Existing mailbox/journal identities reject stale sequences; MAP_FIXED operation generation/successor proof OPEN |
| TX-8 capacity failure | OPEN for MAP_FIXED: journal exhaustion does not gate its RetainOld path |
| TX-9 process exit | Current SCHEDULER exclusion serializes synchronous mutation; same-VM journal reclaim guard cannot see an unjournaled MAP_FIXED vector. Concurrent transaction/exit handoff OPEN |
| TX-10 isolation | Shared native/Helios policy path unchanged; executed security and translator controls have their stated scopes |

## Journal capacity and safe redesign boundary

`OWNERS` is `Table<8,32>`. `reserve(root,vm)` refuses another record for the same pair, including Prepared owners, before finding an empty slot. Thus it admits up to eight **distinct VM operations**, each with a 32-entry chunk; it does not offer 256 independently reservable obligations to one transaction. Generations are checked and nonwrapping. Staging 33 entries fails without partial publication. A staged Prepared record cannot be cancelled through `finish`; it must be empty. `next_chunk` requires Synchronized and every entry released. That is suitable for irreversible munmap chunks, not retention of all old backing until a replacement transaction commits.

| Required boundary | Current MAP_FIXED result / evidence |
|---|---|
| Zero capacity | Not an admission check for RetainOld; existing journal adapter tests do not prove unchanged MAP_FIXED on exhaustion |
| Last slot | Actual journal reservation tested; whole MAP_FIXED reservation absent |
| Multiple page tables | Existing `map_page` and attached hierarchy ownership; no transaction-wide table receipts |
| Partial table creation | Existing adapter reserves up to three entries; on capacity conflict retains attached parents. It cannot own installed leaves |
| Partial installed-leaf rollback | Exact function controls cover success and reported-restore failure containment; hardware/table-cache proof remains open |
| Range > one reservation | Mapper has no journal-entry bound and continues per-page; no whole-range durable admission guarantee |
| Concurrent replacements | Scheduler serializes requests, including remote waits; not a proof of independently prepared competing transactions |
| Exit during preparation | Scheduler prevents concurrent root mutation while held; no durable handoff for a discarded/failed operation |

A complete repair must define commit versus rollback dispositions for old/new backing, full-transaction admission and cancellation before withdrawal, operation/VM generation binding, and attached versus detached table ownership. Existing release-only chunk receipts cannot express “retain this old mapping for rollback” by marking it released. Reserving another same-VM slot later fails by design. Cycling chunks would free old pages before the remaining request can fail, violating the implementation's rollback intent. Arbitrarily limiting all fixed mappings to 16/32 pages or enlarging the journal would change accepted ABI behavior without resolving the ownership protocol.

Accordingly this milestone stops at the expressly permitted safe boundary: the demonstrated silent-rollback defect is contained, and the complete persistent transaction redesign remains **PARTIAL**. No staging scheme or new partial-success ABI is introduced.

## Forward progress and locks

Changed rollback code acquires no new lock, allocates nothing and adds no wait. It uses the existing SCHEDULER → PMM → existing replacement/unmap/TLB path. Failed restore stops under the existing outer guards. The test fixture runs in ordinary userspace and calls existing spawn/wait/yield paths.

Existing OWNERS critical sections are short metadata sections and release before PMM or remote waits; the serializer acquires OWNERS for binding/completion, never the reverse. Existing SHOOTDOWN_SERIALIZER and outer spinlocks span remote waits with normal syscall IF=0. No new global serialization spans ACKs. Bounded ICR polling, exact ACK comparison, missing-ACK fail-stop and no implicit retry remain unchanged. Empty-table unlink/cache synchronization in `rollback_mapped_page`, activation exclusion, global lock-order completeness, fairness and IRQ-off duration remain unproven. A source search is not a timing or whole-kernel lock proof.

## Validation and independent receipts

Final execution results, guest boundaries, resource identities and remaining limitations follow in the generated disposition below. No unexecuted test is promoted from historical evidence or from a parser control. In particular the B1/B2 **complete transaction** pilot is a prerequisite for B3, and remains incomplete, so the 1/4/12 transaction matrix must not be expanded.

### Host controls and requested MF coverage

Commands are in `host-results.json` and each focused probe's evidence. Kernel host tests compile production VM/broker/SHM/quota helpers, not the hardware-dependent complete mapper. The rollback probe extracts the exact production `rollback_anonymous` and `replacement_for_expected` functions; its scheduler, page-table and PMM services are controlled adapters. Journal and ledger harnesses include the actual production modules by path.

| Executed host check | Result | Boundary |
|---|---:|---|
| Kernel library | 201 PASS | Existing capability, VM, SHM/quota and retirement state tests |
| IPC library | 94 PASS | IPC authority/control logic |
| UAC library | 18 PASS | Service policy helpers |
| FastLane ring | 42 PASS | Ring/representation library |
| Doorbell | 9 PASS | Notification state machine |
| Helios compatibility | 29 PASS | Includes mmap flag translation controls |
| Scheduler controls | PASS | Existing production helper controls; no new guest scheduler matrix |
| FastLane parser controls | 204 discovered; 137 PASS, 67 SKIPPED | Synthetic controls; skipped security prerequisites are NOT_RUN |
| Exact rollback, before fix | 2 PASS, **2 FAIL**, exit 101 | Failed restore and missing process discarded descriptors; preserved original failure |
| Exact rollback, after fix | 4 PASS | Successful reverse release, duplicate cleanup, failed restore and missing process |
| Retirement journal | 17 PASS (14 existing, 3 new) | Same-VM reservation exclusion, 33-entry rejection, zero/last slot, stale locator, release protocol |
| Replacement ledger | 5 PASS | Drop-before-commit, whole ledger commit/split, protected overlap, capacity rejection, pending insertion, large table-crossing range |

The 14 existing journal tests are also in the 201 kernel tests and must not be counted twice as independent coverage. No host adapter result is guest PMM/TLB evidence.

| Requirement | Actual coverage | Full property |
|---|---|---|
| MF-1 normal replacement | Real ledger preflight/commit and extracted reverse rollback; existing guest regression separately recorded | Whole-transaction identities NOT_PROVEN |
| MF-2 reservation failure | Actual journal and ledger capacity controls | MAP_FIXED journal admission NOT_IMPLEMENTED |
| MF-3 installation failure | Extracted production rollback after partial installed suffix; controlled failed restore | Guest partial-installation transaction receipts NOT_RUN |
| MF-4 shootdown failure | Existing generic journal retention tests | MAP_FIXED fault/retained identities NOT_RUN |
| MF-5 stale ACK | Actual journal sequence/slot-generation tests and existing mailbox host controls | New MAP_FIXED/VM successor guest interleaving NOT_RUN |
| MF-6 concurrent replacement | Existing host journal contention plus scheduler serialization source review | Actual overlapping replacement requests NOT_RUN |
| MF-7 exit race | Missing-process defensive control, generic pending-VM journal exclusion | Concurrent guest transaction/exit handoff NOT_RUN |
| MF-8 large range | Real ledger accepts >32 pages across a table boundary; journal rejects 33-entry staging | Safe whole-operation capacity behavior NOT_IMPLEMENTED |
| MF-9 duplicate cleanup | Exact production rollback and journal repeated-release rejection | All physical/table/swap/refund identities NOT_PROVEN |
| MF-10 native/Helios | Actual translator and policy host controls; original native security cases separately assessed | Full Helios guest replacement/fault/teardown matrix NOT_RUN |

The failed-restore adapter exercises a reported helper failure, not a demonstrated hardware trigger. In normal synchronous syscalls a missing process is excluded by scheduler serialization; that control verifies defensive behavior if the invariant is violated, not an already reproduced production exit race.

### Original native suites and preserved failures

All final runs use the same final source map recorded in `source-final.json`, with independent embedded-payload and ISO/ELF checks. Commands, cwd, environment overrides, stdout/stderr, exit status, hashes and assessments are retained under each run directory. QEMU termination after the observed completion markers is host cleanup, not guest recovery or clean shutdown proof.

| Original suite | Final source-bound run | Result |
|---|---|---|
| M1/M2 | `native/m2-20261110T114300.901935Z/results.json` | PASS, including unchanged M1 security assertions and M2 hardware fault witnesses |
| M3 | Same M2 guest, original M3 word-compatibility assessor | PASS; aligned machine-word accesses/private copies only, not general payload transport authorization |
| M4 | `native/m4-20261110T114519.255756Z/results.json` | PASS |
| M5 | `native/m5-20261110T114832.466254Z/results.json` | PASS |

The first new attempt (`m2-20261110T113527.259675Z`) is INFRASTRUCTURE_FAILURE at the shell prerequisite; the security cases are NOT_RUN. The broader combined-feature attempt (`m2-20261110T113841.550007Z`) is TEST_FAILURE: 16 unchanged M1 terminal-outcome assertions fail despite zero client exit statuses and passing M2/M3 assessments. The observed exit-race terminal reason is IdentityChanged=5 rather than required ProcessRetired=2. Source review identifies the extra doorbell sweep before process retirement as a relevant ordering difference; this milestone does not repair or independently prove the full cause. That failed combined configuration remains a limitation and is not canceled by passing original configurations. The earlier passing M4 combined-feature run also remains an earlier-source attempt, not final-source validation.

Historical KFP-2A.3 shell failures are preserved as INFRASTRUCTURE_FAILURE, with build/source checks passing and downstream native security NOT_RUN. No BUILD_FAILURE or SOURCE_INTEGRITY_FAILURE is observed in final runs. Parser-only controls never substitute for the original suites. Gate A is COMPLETE for trustworthy execution of the original suites on two CPUs, not for the broader combined feature configuration or production interactive TTY launch.

### Guest pilot, faults and CPU matrix

`mm2d/results.json` records the fresh normal mode-0 two-CPU MM-2D run: PASS, approximately 44.12 seconds to required markers. Its source-bound build uses `kfp2a2_fault_test,mm2a_test_injection` with `KFP2A2_FAULT=0`; exact build and QEMU commands are retained. Existing zero-capacity/last-slot, attached-table retention, lower-level replacement, SHM and MM-2A controls execute. This is a regression pilot, not the requested complete MAP_FIXED transaction B1/B2 pilot. The inherited runner's overly broad scope text is retained as `runner_original_scope` and corrected in the current `scope` field.

| CPUs | Executed | MAP_FIXED transaction matrix |
|---|---|---|
| 1 | NOT_RUN | NOT_RUN: complete two-CPU transaction pilot missing |
| 2 | Original M1–M5 PASS; normal MM-2D regression PASS | B1/B2 incomplete |
| 4 | NOT_RUN | NOT_RUN: pilot incomplete |
| 12 | NOT_RUN | NOT_RUN: pilot incomplete |

MAP_FIXED-specific journal exhaustion, allocation failure with transaction receipts, partial installation failure, APIC delivery failure, missing ACK, delayed/stale ACK, concurrent transaction and process-exit race are **NOT_RUN** in guests. Existing generic journal/mailbox tests and historical faults do not establish these interleavings. The matrix is deliberately not expanded following an incomplete safety pilot. Default production kernel `cargo check --locked --offline -p sunlight-kernel --bin sunlight-kernel` passes against the final source map. No separate release-binary exclusion check was executed; existing production feature guards remain unchanged.

### Identity-based accounting and its limits

`accounting.json` retains the original native assessors' identity-qualified backing, mapping, TLB and lifecycle receipts. These are FastLane resources, not invented MAP_FIXED transaction receipts:

* M2: 30 backing lifetimes, 58 views; 16 remote and 42 zero-target invalidations.
* M4: 30 channels, 33 registrations, 32 completions, 32 collected outcomes, 32 physical parks and 31 remote wakes.
* M5: 20 channels, 60 backings, 120 views; 9 remote and 111 zero-target invalidations; 18 publications, 10 acknowledgments, 11 private copies and 17 remote wakes.

The normal MM-2D raw output independently pairs three frame and three table lifetimes by kind, root, VM generation, VA and physical frame. Each synchronized observation reports allocated=true, releases=0, reuses=0; its matching released observation reports allocated=false, releases=1, reuses=0. The derived checker requires all six exact identity pairs, not aggregate zero drift. Observers reset per fixture window; repeated physical addresses in another VM/window are not a global non-reuse claim. The exhausted-journal table remains attached until later reclaimed once. The zero-view SHM fixture records final quota 4096→0 and repeated refund=0. Derived capability identifiers are fingerprinted; original logs and hashes remain preserved.

Missing MAP_FIXED receipts include a durable transaction/reservation identity, complete old/new/swap/table ownership across all phases, accepted/rejected transaction-generation ACKs, rollback/commit handoff, and exit ownership transfer. Private anonymous MAP_FIXED does not perform shared-quota reservation/refund; absent operations are not reported as measured zero. No fail-stop transaction fault was forced in the final guest, so there is no claimed retained-resource recovery from a terminated guest.

## Final disposition

**Gate A — COMPLETE**, scoped to the original M1–M5 suites on two CPUs. The historical root cause is the boot TTY's legacy authenticated spawn lacking the required Account/session attachment. The opt-in test runner uses the existing authenticated SessionD fixture and a real shell without weakening production authorization or security assertions. Production interactive TTY repair and the preserved broader-feature M1 failure remain separate work.

**Gate B — PARTIAL.** The exact rollback helper now stops before losing an unsuccessfully restored descriptor. Its successful reverse restore/free ordering and duplicate cleanup controls pass. Durable whole-request ownership reservation, safe journal exhaustion admission, page-table rollback lifetime closure, and concurrent exit handoff still require a transaction protocol beyond the existing release-only journal primitives. The current logical commit remains ledger publication after all PTE installations; intermediate PTE visibility is unchanged. Normal successful rollback restores exact leaves and synchronizes before freeing replacement frames; failed restoration now fail-stops. MAP_FIXED still has no complete journal-exhaustion guarantee before withdrawal.

**Overall — PARTIAL.** Production behavior changes only in anonymous rollback failure containment in `kernel/src/process/mmap.rs`. UAC opt-in fixture files and `tools/kfp2a4_native.py` change test infrastructure. Existing unrelated edits are preserved, source integrity checks pass, and no commit or push is performed. Production FastLane remains disabled. The combined-feature failure is preserved rather than characterized as a resolved or absent regression.

Recommended KFP-2A.5: extend the existing retirement owner with whole-request admission, cancellation and explicit commit/rollback dispositions for old/new backing, swap, table graphs and mapping records. Define the operation/VM generation handoff and exit ownership rules before implementing controlled two-CPU fault/concurrency receipts. Only after that complete pilot passes should the 1/4/12 CPU matrix run. Keep production TTY protocol repair and the combined-feature terminal-reason discrepancy independently tracked.

Machine-readable entry point: `.kfp-2a4-evidence/results.json`. Baseline, preserved attempts, command records, source snapshots, independent accounting and the task-only production patch are in the same evidence directory. `artifact-index.json` hashes the retained artifacts; `deliverables.json` binds this report and final results.
