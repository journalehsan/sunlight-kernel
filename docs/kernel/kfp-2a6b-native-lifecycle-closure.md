# KFP-2A.6B — Native lifecycle evidence audit and release observations

**Verdict: PARTIAL. Production runtime behavior did not change. Production FastLane remains disabled. No commit or push.**

The milestone adds debug-fixture guard-drop observations and reruns the existing authenticated guest security suites. It does **not** close native R1–R6, exactly-once terminal publication, nested exit-hook continuity, or the CPU matrix. An immediate post-drop observation is deliberately not accepted as instrumentation of the mutex's internal unlock instruction. Successful legacy security assessors do not satisfy the stronger lifecycle contract.

Machine-readable results, individual runner manifests, serial traces, host controls, source hashes and invocation records are in `.kfp-2a6b-evidence/`. `results.json` is authoritative for fresh execution outcomes. `event-traces.json` contains parsed native contention and release observations; original serial logs remain alongside each image. `receipt-schema.json` is explicitly a proposed schema, not an implemented witness.

## Baseline and preservation

Revision: `127aee1d938781877264f6d0e13f67c3fcd89327`. The incoming tree was dirty: manifests, Cargo.lock, version state, the previous channel contention fixture and untracked milestone reports were present. `status-before.txt`, `preexisting.patch`, `channel_race.before.rs` and `baseline.json` preserve the incoming state. The required KFP-2A.5, 2A.6, 2A.6A reports and 2A.6A results were read. Historical failures and previous source-integrity attempts remain in their original evidence directories, unchanged; `source-integrity.json` verifies the captured historical inputs.

The only new runtime source change is in `kernel/src/capability/channel_race.rs`, already gated by explicit debug fixture configuration. The before-file distinguishes this change from the incoming 2A.6A modifications. No capability authority, terminal precedence, process exit semantics, scheduler policy or production lock implementation changed.

## Release instrumentation and limitations

`ObservedGuard` owns the actual `spin::MutexGuard<'static, Scheduler>` returned by the existing acquisition fixture. Dereference preserves the ordinary scheduler operations. Its destructor first drops that real guard, then captures a sequence, armed channel generation, exact `addr_of!(SCHEDULER)` lock address, owning CPU and actor into a fixed atomic slot. Capture does not invoke `finished()`, log, allocate, block, or wait for another CPU. Actor zero is inert. The actor binds to the existing exact PID/incarnation/VM participant saved by authenticated arm; no new authority identity is created.

Metadata stores precede Release publication of the sequence. The reader uses Acquire before reading metadata. Slots reset at authenticated arm after the preceding fixture has completed; these observations are scoped to that protocol, not a general lock telemetry facility. Receipts are printed later by the exact-creator observation path. Printing order is not sequence order.

This establishes an independently captured **post-guard-drop observation**. It does not capture the mutex's internal atomic unlock instruction. Another CPU can run between the internal unlock and receipt sequence allocation. The held/probe lock binding is established by source inspection, whereas the release observation includes its address. The strict requested exact-boundary proof therefore remains **NOT_PROVEN**. `unlock_validator.py` rejects the stronger claim even when its scoped ordering passes. A return marker is never substituted for release.

The inherited bounded holder/probe acknowledgement remains unchanged. No new holder handshake was added. It still waits for a remote atomic acknowledgement with IF clear and SCHEDULER held; this milestone does not broaden that mechanism into lifecycle coordination or claim universal progress. A lifecycle fixture must satisfy the stricter coordination constraints before CPU expansion.

## Lifecycle audit and receipt requirements

M1 uses existing `RecipientIdentity` PID/incarnation/VM tuples, channel handle generation, and `SessionRef` id/epoch. These distinguish exact participants and sessions; endpoint lifecycle correlation still requires an audited native endpoint identity/reuse contract. Merely adding a fixture label cannot establish endpoint generation or successor identity.

`Scheduler::channel_participant_terminal` searches the full tuple before checking `is_exiting()`. An exact exiting participant classifies as ProcessRetired; a missing or different tuple classifies as IdentityChanged. The prior production R2 correction is unchanged. Existing guest exit modes ending ProcessRetired are useful outcomes, but cannot establish the missing early-sweep ordering.

`Channel::retire` tests `live()`, invokes gated retirement/Doorbell work, stores the terminal and then marks the channel Retired. Broker and scheduler serialization protect normal mutation. First committed terminal wins; later calls are idempotent. Status snapshots cannot establish the number of terminal stores or inventory losing candidates. New receipts at actual candidate, terminal-store, cleanup-claim and final-resource boundaries remain necessary.

Native endpoint closure is `Scheduler::close_owned_ipc_endpoints`: gather owned endpoints and revoke broker endpoints, retire recipient grants, remove each endpoint from the IPC shard, and finish peer-closed calls. Broker revocation calls `retire_recipient_grants`, which retires channel participants as ProcessRetired. IPC PeerClosed is a separate result. There is no EndpointClosed channel terminal. A Revoked adapter is not accepted as this native path; precedence must be measured at the actual commits.

Physical reaping checks stack pins, owning CPU, ready-queue state and `exit_cleanup_pending`. It tears down resources before clearing pending and exposing the reusable lifecycle state. An entry marker before teardown does not prove completion. `receipt-schema.json` records the needed boundaries and rejection rules without manufacturing observations.

## Native R1–R6 matrix

| Case | Fresh native disposition | Missing witness |
|---|---|---|
| R1 retirement wins | NOT_PROVEN | Retirement commit before competing classification; one terminal store and all candidates |
| R2 exact exiting participant | NOT_PROVEN | Exact tuple observed exiting before retirement publication, with cleanup completion |
| R3 sweep before exit | NOT_PROVEN | Live native sweep before actual exit commit |
| R4 endpoint closure wins | NOT_PROVEN | Native endpoint closure completion, endpoint identity and competing retirement commits |
| R5 successor mismatch | NOT_PROVEN | Real native stale identity compared with actual successor generation; no adoption |
| R6 cleanup overlap | NOT_PROVEN | Independent overlapping cleanup intervals, cleanup owner, all candidates and single store |

No new authenticated R1–R6 guest payload was implemented. The unchanged M1 clients exercise attach/revoke and attach/exit contention; they cannot be relabeled R1–R6. Missing witnesses are NOT_PROVEN, not a successful guest execution or an inferred PASS. No confirmed security violation was observed in the executed controls; the unexecuted properties have no security verdict.

## Nested exit-hook continuity

`finish_current_process` uses the existing Finished/Reaped check to preserve a terminal process result. Reaping uses pending and pin/ownership checks. Channel terminal arbitration uses `state != Retired`. These are distinct guards; none is a general recursion-depth guard.

In combined/Doorbell builds, `finish_current_process` explicitly drops the channel fixture guard before calling `bell_race::acquire`. The new guard observer captures that drop independently; the later channel `finished()` marker cannot prove continuous exclusion through it. No change hides or closes this gap.

Direct recursive acquisition of the non-reentrant scheduler mutex would not constitute a safe test. The audit did not establish a contract proving all nested entries impossible or a bounded safe-failure path. Cleanup retains pending state until its end; pending alone does not prove protection against an arbitrary internal recursive callback. Normal Rust mutable borrowing and lock serialization constrain existing paths, but are not a native reentrant-callback witness.

Direct nested invocation, exit with pending Doorbell observation, repeated native cleanup, reentrant callback, and real process generation turnover remain NOT_PROVEN as focused native scenarios. Existing host repeated cleanup/retirement and generation controls pass with their adapter scope; they do not close these gaps. No production exit behavior was changed to facilitate tests.

## Validation and image integrity

Stage A reruns kernel library (208), IPC (94), Doorbell (9), production kernel check, scheduler policy controls, source-extracted classifier/arbitration controls, inherited bounded-contention controls and fourteen synthetic receipt controls. The new source-extracted destructor controls verify drop-before-observation, generation/address/CPU fields, inactive behavior, and independence from `finished()`. Ten additional validator controls reject stale generation, wrong lock/owner/CPU, late failure, missing evidence and a return marker offered as unlock. These are host controls, not native lifecycle proof.

Stage B reruns the original, combined and Doorbell-only authenticated M1/M2/M3 suites on two CPUs. Separate M4/M5 runs retain their own payloads and assessors. All five fresh guest attempts PASS their unchanged assessors; all five source maps match the final tested source. Twelve native held < failed probe < post-drop sequences pass scoped observation validation, while all twelve remain NOT_PROVEN for the exact internal-unlock contract. Each run cleans the kernel package before building to avoid the historical embedded-service cache mismatch. Individual manifests record requested features, environment, every build command, QEMU invocation, source map, image hashes, archived payload hashes and offsets, CPU count, watchdog and serial hash. `results.json` records final fresh statuses; failed attempts are retained rather than replaced.

The runner checks intended archived payload bytes inside the ELF, extracts the kernel from the ISO and compares it to the ELF, and rejects source mutation during the attempt. No prior guest image is executed. Source revision alone does not identify the dirty tree; the source manifest hash does. Build IDs are timestamped attempt identities, not embedded per-attempt payload identities. The M1 configurations retain the same intended M1 suite contents; this is not a dedicated lifecycle suite. Full requested embedded lifecycle-suite/build-generation identity is unfulfilled.

| CPUs | Qualification |
|---|---|
| 1 | NOT_OBSERVED: native sequential lifecycle fixture absent; simultaneous IF-clear contention not applicable |
| 2 | Fresh legacy security suites; full lifecycle pilot NOT_PROVEN |
| 4 | NOT_OBSERVED: required native lifecycle pilot incomplete |
| 12 | NOT_OBSERVED: same gate |

## Remaining work and verdict

**PARTIAL** is required. Post-drop observations, source audit and passing legacy regressions improve the evidence but do not meet acceptance. Exact unlock instrumentation, actual native lifecycle receipts, all outcome candidates/store counts, native R4 closure, real successor generation, R6 cleanup overlap, nested continuity and dedicated embedded payload identity remain open. The CPU matrix must follow a valid lifecycle pilot.

Recommended KFP-2A.6C: instrument the internal unlock boundary with a debug-only fixed atomic observer; add actual lifecycle decision/completion receipts; implement authenticated native R1–R6 and nested controls using bounded coordination outside SCHEDULER; verify payload/build generation inside each image; then expand CPU coverage and rerun affected security cases. Production FastLane remains disabled.
