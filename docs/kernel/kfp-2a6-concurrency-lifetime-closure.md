# KFP-2A.6 — Contention investigation and lifetime evidence boundary

**Overall: PARTIAL. Gate A: PARTIAL. Gate B: PARTIAL. Gate C: NOT_PROVEN.** Production behavior did not change. Production FastLane remains disabled. No commit or push.

This milestone does not establish deterministic native R1–R6 closure. A passing natural-contention run must not be treated as that proof. CPU expansion and expanded VM race work remain stopped at Gate A. The current evidence adds fresh source-bound security pilots, source-extracted host ordering controls, and a recheck of historical identity-qualified retirement receipts. It does not provide the complete requested concurrency implementation.

## Baseline and integrity

Baseline HEAD: `127aee1d938781877264f6d0e13f67c3fcd89327`. The incoming tracked working tree was clean. `.kfp-2a6-evidence/baseline.json`, `status-before.txt`, and `preexisting.patch` preserve the revision, relevant source/build input hashes and working-tree state. The KFP-2A.4 and KFP-2A.5 reports and KFP-2A.5 results were reviewed. Historical evidence remains untouched. KFP-2A.5's reported source revision differs from current HEAD; its results are historical and are never relabeled as fresh guest execution.

The native runner hashes tracked/nonignored Rust, build configuration and tool inputs, including `.cargo/`, excluding generated target and dot-evidence snapshots. It checks that map after builds and guest execution; each guest retains its own service/probe/shell payload copies and verifies them byte-for-byte inside its ELF, extracts its ISO kernel, and compares ELF hashes. Each attempt's manifest records revision, source map, commands, environment overrides, features, CPU count, payload hashes, ELF/ISO hashes, serial hash, watchdog stop reason and original assessor output. These manifests are the authoritative per-guest build identities. A passing guest from another suite is never substituted.

The first original pilot uses the unchanged runner. Subsequent configuration changes explicitly run package-specific `cargo clean -p sunlight-kernel --target x86_64-unknown-none` before rebuilding. Commands and exit statuses are in `pilot-driver.json`. This avoids the demonstrated stale embedded-service cache path while retaining the runner's mandatory embedding checks. It is build control, not a security correction.

## Exact M1 identity and lifecycle contract

Authority resides in `capability/channel.rs`; native admission holds SCHEDULER then CAP_BROKER across coherent identity validation and mutation. A handle identifies a row but does not authorize a caller. Immutable participant identities are `(pid, incarnation, vm_generation)` from `RecipientIdentity::of`; session authority additionally uses `(session.id, session.epoch)`, the recorded issuer and Account authorization. Endpoint identities and IPC call generations belong to their own validation paths; they are not interchangeable with participant VM generations.

| Boundary | Authoritative transition and exclusion |
|---|---|
| Exit begins | `sched::exit` sets Finished and `exit_cleanup_pending` under SCHEDULER. External termination can set pending while an execution handoff remains outstanding. `is_exiting` includes pending, Finished and Reaped. |
| Early sweep | Exit calls `fastlane_mapping_sweep` before closing owned endpoints. Doorbell builds call `fastlane_bell_sweep` before mapping's ACTIVE early return. Both execute with the scheduler borrowed. |
| Exact identity | `channel_participant_terminal` searches the complete participant tuple before inspecting lifecycle/privilege. |
| Classification | Exact exiting participant → ProcessRetired. Exact live authorized participant → no retirement. Missing/different tuple or lost required privilege → IdentityChanged. |
| Retirement publication | `Channel::retire` commits only when live, clears the one-use consumer receipt, preserves pending wake obligations, stores the terminal reason and sets Retired under CAP_BROKER. |
| Endpoint closure | `close_owned_ipc_endpoints` → `revoke_endpoints_owned_by` → `retire_recipient_grants` → `channels.retire_process`. Removal of endpoint capabilities precedes finishing detached IPC calls as peer-closed. |
| Arbitration | First committed terminal outcome wins; later process/session retirement, revocation and expiry cannot overwrite it. This is serialization precedence, not numerical reason priority. |
| Cleanup | Mapping backing drains and Doorbell pending wake collection follow their own paths. Physical process cleanup is pin/handoff gated; logical Finished is not permission to immediately free an executing stack or VM. |

Exit marking, early sweep classification, endpoint revocation and terminal publication cannot execute simultaneously inside this scheduler critical section. Their order within self-exit is fixed. Other authenticated operations contend before acquiring that guard. Remote user memory accesses, in-flight execution/handoff and asynchronous invalidation acknowledgments are separate concurrency domains.

An exact exiting participant is **not** a new identity. An exact Reaped object still present in the process vector is also exiting and classifies ProcessRetired. Once the object is absent, this classifier returns IdentityChanged; an earlier committed ProcessRetired remains unchanged. PID reuse with a new incarnation, or the same PID/incarnation with a new VM generation, never matches the predecessor. A session epoch change is handled through session retirement/authorization, not by silently replacing the row's participant. An endpoint generation change must be rejected by endpoint/call validation; no complete new endpoint-generation race witness was collected here.

`retire_process` accepts a PID and relies on lifecycle serialization/nonreuse at its call sites. Its signature alone is not independent proof against delayed successor-PID cleanup. Full successor-generation completion remains NOT_PROVEN in this milestone.

There is no `EndpointClosed` channel terminal enum. Owned-endpoint teardown publishes ProcessRetired through recipient retirement. Explicit channel revocation publishes Revoked; IPC peer-close is a distinct call outcome. Thus R4 must name which closure path it exercises. Treating a synthetic Revoked result as an actual endpoint-closure witness would be invalid.

## Contention root cause and repair disposition

The sixteen historical terminal assertion failures share the established KFP-2A.5 defect: Doorbell early sweep used `channel_process`, which excludes exiting processes, converting an exact exiting participant into IdentityChanged=5. First-terminal arbitration then prevented endpoint retirement from publishing ProcessRetired=2. The complete-tuple-first classifier corrects that defect. This milestone makes no further production correction.

The original-configuration coverage failure is different. `channel_race::acquire` records both actors, releases SCHEDULER and uses private-stack yielding waits. The contender publishes READY, then waits for HELD using a scheduling yield. The winner observes READY, acquires SCHEDULER, publishes HELD and executes its ordinary operation without waiting. The contender may resume only after winner completion. Its real `try_lock` then succeeds and emits `lock-missed`; the window is gone. A short original mapping operation makes this especially visible. There is no evidence here of a lost production retirement event or observation of a successor generation causing this coverage failure.

The required existing witness is a failed scheduler `try_lock` whose probe sequence precedes the winner's returned sequence, for the exact armed channel generation and both authenticated actors on distinct CPUs. `observed` is exact-creator authorized. A HELD flag alone, two entries alone, or an eventual correct terminal result cannot satisfy it. Sixteen existing bounded attempts remain unchanged; misses remain misses.

The fresh original pilot passes naturally but still requires multiple attempts. This is evidence of scheduling dependence, not a deterministic fix. No sleeps, retry increases, fabricated contention, weakened assertions or terminal precedence changes were introduced. No guest fixture correction was implemented. In particular, making the winner wait while holding SCHEDULER with IF clear for a contender whose continuation needs SCHEDULER would create the dependency this investigation must avoid. A safe deterministic native coordination design remains an explicit blocker, not a claimed impossibility.

## Required race disposition

`host_controls.py` extracts the exact classifier, `retire_unavailable` and `retire` bodies into a standalone Rust harness. Process identity storage, lifecycle state and channel structure are adapters. Two host actors rendezvous outside a mutex and execute explicitly ordered operations; no rendezvous survives into a guarded operation. The controls test actual method bodies but not native authentication, actual scheduler lock contention, hardware execution or physical cleanup.

| Required case | Host observation | Native requirement still missing |
|---|---|---|
| R1 retirement wins | Exit → ProcessRetired publication → sweep; ProcessRetired retained | Boundary-qualified sweep after retirement publication |
| R2 exact exiting | Exit → exact sweep → retirement hook; sweep publishes ProcessRetired | Native observation receipt before the later retirement hook |
| R3 sweep before exit | Live sweep leaves terminal empty → exit → retirement → later sweep | Both authenticated actors coordinated at those boundaries |
| R4 closure wins | **Revocation adapter only:** Revoked → retirement; Revoked retained | Actual endpoint teardown and IPC/channel outcome correlation |
| R5 generation change | Incarnation successor → stale exact-tuple lookup → IdentityChanged; VM successor and absent controls also pass | Native PID/session/endpoint successor permutations |
| R6 cleanup overlap | Ordered repeated sweep/retire/revoke retains first result | **No overlap or physical cleanup proof**; exactly-once drain/release not measured |

Every host sequence is retained in `host-controls-1.stdout`; fixture identities are PID 6/10/11, incarnation PID+100, VM PID+200. HOST-R labels identify scoped controls only and are not assertions of completed guest R cases. The first host harness compilation failed because placeholder substitution entered a diagnostic string. Its source, compiler output and FAIL receipt remain in `host-attempt-1/`; the corrected attempt passes.

No new native R1–R6 deterministic witness was established. Existing guest witnesses demonstrate natural real contention and correct outcomes in their own recorded attempts. They do not measure terminal store count independently or establish all cleanup/resource invariants.

## Concurrent MAP_FIXED review

Gate A's investigation was recorded in `gate-a-investigation.json` before this secondary review. Expanded Gate B guests were not started. VM-R1 through VM-R7 are **NOT_RUN** here.

The whole-request journal owner reserves before page-table preparation, frame allocation or leaf replacement. Each record is root/VM-qualified, with non-Clone locator, ordered preparation/install cursors, original/replacement identities, disposition and synchronization obligations. Preparing → Installing → Committed, or Cancelling → RolledBack, governs which backings release and which transfer to the live graph. Restoration mismatch and unresolved shootdown quarantine ownership and fail-stop. Teardown rejects a VM with outstanding journal ownership.

Mapping syscalls, munmap, fault handlers and process-exit mutation use scheduler exclusion. MAP_FIXED additionally holds PMM while its bounded journal sections run; journal guards do not span frame free, ZRAM discard or TLB wait. This source order is not evidence that two native requests actually arrived concurrently or that an exit/fault contender reached the intended boundary. Journal unit tests establish admission/generation rules, not VM-R1/R2/R3/R4/R6 native arrival coverage. Historical shootdown injections do not establish cleanup overlap or late ACK after cancellation with a successor VM.

Visibility is per-PTE publication before ledger commit. Prepared empty tables remain attached to the canonical VM graph. Remote user threads can observe intermediate mappings. Whole-range atomic visibility is neither promised by this evidence nor proven. No physical-frame reuse is authorized while synchronization remains unresolved; this milestone adds no recovery or reclamation engine.

## Lifetime accounting boundary

`historical-accounting-recheck.json` reruns the inherited checker against the original raw KFP-2A.5 logs without modifying them. It retains exact `(slot, transaction generation, root, VM, index, VA, backing)` tuples, prerelease observations, synchronized owner/required/received masks, final PMM allocation/release/reuse observations, retained resources and attached-table receipts. This is a reproducibility check of an existing algorithm, not a new independent instrument or fresh guest accounting pilot.

The final-source normal historical dedicated transaction fixture has 16 exact frame releases, 21 backing transfers and one swap disappearance. Shootdown busy/NotSent/missing-ACK cases retain eight exact old/new frames; restoration failure retains eight exact backing resources. Their identity sets and source manifest references are in the recheck file. Aggregate counts alone are not accepted as identities. The initial historical normal attempt is preserved with its earlier-source qualification.

| Property | Evidence and disposition |
|---|---|
| One prior owner per observed frame release | Historical scoped join and independent PMM observer pass; universal/concurrent-exit proof NOT_PROVEN |
| Valid transfer source/destination | Journal disposition implies old/new live graph ownership; no independent destination/exit handoff observer: NOT_PROVEN |
| Required synchronization before release | Historical owner masks/sequence and per-resource observations pass scoped checks; complete accepted/rejected ACK event proof NOT_PROVEN |
| No reuse with outstanding obligation | Historical retained PMM windows show zero reuse; global allocator lifetime proof NOT_PROVEN |
| Page-table exactly-once release | Attached table identities exist; independent table-free ordering missing: NOT_PROVEN |
| Swap exactly-once discard | One exact generation-encoded slot disappears; Boolean absence cannot count duplicate discards: NOT_PROVEN |
| Quota reservation/refund | Private anonymous MAP_FIXED has no shared-quota operation; concurrent shared-quota accounting NOT_PROVEN |
| Journal reservation/descriptor | Host state tests pass; complete native concurrent reserve/cancel/exit receipts NOT_PROVEN |
| Process-exit ownership transfer | Source serialization/teardown guard only; actual race/handoff NOT_PROVEN |
| No stale-generation completion | Existing journal controls pass; cancelled-transaction/successor-VM guest proof NOT_PROVEN |

## Progress and locking review

No production or fixture execution path changed, so this milestone adds no kernel wait, lock ordering, recursion, journal contention or IRQ/preemption-disabled block. Host rendezvous waits have five-second receive bounds and occur outside their mutex; host thread scheduling is not a kernel forward-progress proof. Native watchdog limits remain unchanged. Existing APIC delivery remains bounded, and unresolved TLB synchronization remains fail-stop.

The existing scheduler-wide exclusion, PMM ownership and synchronous remote ACK dependency are retained. Fairness, priority inversion bounds, global lock-order completeness and maximum interrupt-disabled duration are not newly established. A passing short guest does not establish those properties.

## Validation results

The generated machine-readable results and per-attempt manifests are authoritative. Fresh guest results, exact naturally qualified contention sequences and the CPU matrix are appended below after pilot collection. CPU 1 requires a separate scheduling-interleaving fixture because the current original assessor requires distinct CPUs. It must not be silently relaxed. CPUs 4 and 12 remain NOT_RUN until the deterministic two-CPU pilot satisfies the new contract.

| Fresh two-CPU guest | Result | Security boundary |
|---|---|---|
| Original M1/M2/M3, mapping kernel | PASS | 19 natural attempts; one qualified attempt per existing mode |
| Combined M1/M2/M3, transfer kernel | PASS | 13 natural attempts; one qualified attempt per existing mode |
| Doorbell-only kernel, original M1/M2/M3 clients | **FAIL** | Mode 1: all 16 attempts miss contention; userspace assertion line 460. M2/M3 workloads NOT_RUN |
| M4 Doorbell clients/kernel | PASS | Original native assessor |
| M5 transfer clients/kernel | PASS | Original native assessor and independent M5 payload |

All five guest source manifests match the unchanged baseline source domain. No guest build/image/source-integrity failure occurred in these five attempts. The Doorbell coverage assertion is a TEST_FAILURE, not an infrastructure failure and not a reproduced wrong terminal reason. Its missing downstream cases are not passing security evidence.

Exact requested original kernel features: `key_inject,session_launch_test,fastlane_mapping_test`. Effective kernel closure adds `fastlane_channel_test,native_session_race_test`. Combined requested features replace mapping with `fastlane_transfer_test`; its closure additionally includes `fastlane_doorbell_test,fastlane_mapping_test,fastlane_channel_test,native_session_race_test`, `dep:sunlight-fastlane-doorbell` and `sunlight-fastlane-doorbell/fixture`. Doorbell-only omits transfer. `feature-sets.json` records these closures. The M1/M2/M3 UAC payload requests `kfp-security-runner,fastlane-mapping-test`; M4 and M5 use their own corresponding service features. Full service environments remain in each guest manifest.

The following are **observed natural witnesses**, not deterministic reproduction claims. Both actors' exact participant identities, CPU, bindings, complete event lists and terminal observations are in `race-witnesses.json`. Numbers below are boot-local sequence values `lock-held < lock-contended < winner-returned`:

| Existing mode | Original generation / sequence | Combined generation / sequence | Final terminal |
|---|---|---|---|
| 1 attach before revoke | 6 / 13 < 14 < 15 | 7 / 22 < 23 < 24 | Revoked=1 |
| 2 revoke before attach | 7 / 22 < 23 < 24 | 9 / 40 < 41 < 42 | Revoked=1 |
| 3 attach before exit | 17 / 112 < 113 < 114 | 10 / 49 < 50 < 51 | ProcessRetired=2 |
| 4 exit before attach | 23 / 166 < 167 < 168 | 17 / 112 < 113 < 114 | ProcessRetired=2 |

The contender's failed try_lock is real, and both CPUs 0 and 1 participate. These four modes do not provide the six new lifecycle boundary witnesses. The Doorbell failed guest has no qualifying natural attempt; its outcomes remain preserved.

Default-production kernel check and `git diff --check` pass. The production check emits four existing dependency warnings from sunlight-net.

Fresh host regressions: kernel library **208 PASS** (capability authorization, journal/ledger, VM/SHM/quota helpers included), IPC **94 PASS**, Doorbell **9 PASS**, Helios compatibility **29 PASS**, scheduler controls **PASS**. Six source-extracted ordering controls and live/exiting/successor/absent classifier controls pass within their adapter scope. Host library totals are 340; overlapping subsets are not added again. No new tests were added to production source. Fresh MM-2D, hardware PMM/VMM, MAP_FIXED fault guests, full SHM lifetime guest matrix, ring/UAC library reruns and parser controls are NOT_RUN here; earlier passes remain historical.

| CPUs | Gate A | Gate B | Gate C |
|---|---|---|---|
| 1 | NOT_RUN; controlled single-CPU fixture missing | NOT_RUN | NOT_RUN |
| 2 | PARTIAL; original/combined/M4/M5 pass, Doorbell coverage fails | No fresh guest pilot; historical scoped proof only | No fresh guest pilot; historical accounting recheck only |
| 4 | NOT_RUN; deterministic pilot incomplete | NOT_RUN | NOT_RUN |
| 12 | NOT_RUN; deterministic pilot incomplete | NOT_RUN | NOT_RUN |

## Independent verdicts and next work

**Gate A: PARTIAL.** The exact historical classifier defect remains corrected, but the original coverage issue is **not resolved deterministically**. The unchanged fixture still depends on natural timing; a new Doorbell-only coverage failure is preserved. No fresh evidence establishes all R1–R6, independent terminal-store count, successor endpoint/session authority or exactly-once physical cleanup.

**Gate B: PARTIAL.** The existing durable MAP_FIXED owner and source-level serialization remain intact. VM-R1–R7 native overlap evidence is absent; no Gate B CPU expansion is authorized by these results.

**Gate C: NOT_PROVEN.** Identity-qualified historical release/retention observations reproduce, but full transfer-destination, table-free, swap-discard-count, quota and exit ownership proof is incomplete. This is not downgraded because M4/M5 pass.

**Overall: PARTIAL.** Production files changed: none. Production behavior changed: no. Production FastLane remains disabled. No commit or push.

Recommended next milestone is a **KFP-2A.6 continuation**, not a closure declaration: implement safe test-only native coordination and receipts at exit/classification/publication/cleanup boundaries, with explicit one-CPU scheduling controls and no scheduler-dependent peer wait under the lifecycle lock. Resolve the coverage fixture before expanding concurrent MAP_FIXED or accounting work. Then add independent table release/reuse, swap discard counters, destination ownership and cancelled/successor-generation ACK observations; run each gate's own two-CPU pilot before its matrix.

Evidence entry points: `.kfp-2a6-evidence/results.json`, `race-witnesses.json`, `host-controls.json`, `historical-accounting-recheck.json`, `baseline.json`, `feature-sets.json`, and per-guest `results.json`. Failed guest and host attempts are retained. `artifact-index.json` binds retained evidence; `deliverables.json` binds this report and machine-readable results.
