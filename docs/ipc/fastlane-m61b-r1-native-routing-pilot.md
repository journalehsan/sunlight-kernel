# M6.1B-R1 authenticated routing pilot: partial

Latest continuation: kernel policy catalog implemented and host-validated; native pilot remains **PARTIAL**. The original audit below is retained as historical evidence. See the continuation section for current implementation and results.

Status: **PARTIAL**. No native routing pilot ran and none of the acceptance conditions requiring native operation, cleanup, or physical retirement has been established. Production FastLane remains disabled. This document does not qualify the existing draft for production.

## Source and audit

The starting HEAD was `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`, with extensive pre-existing tracked and untracked changes. `.fastlane-evidence/m61b-r1/initial-status.txt` and `initial.patch` record the initial status and tracked patch. `initial-source.json` hashes 5,627 tracked and nonignored untracked files, including the existing untracked routing draft; its SHA-256 is `c7f60e63ad2355d6ab704c12c7393b3639d8e656124c999ab9de061f95d244ea`. This manifest identifies file contents, not a build environment or a clean commit. Existing M6.1A, M6.0R and M6.1B evidence was preserved.

M1's Account-mediated protocol binds independent caller consent to kernel `RecipientIdentity` values (PID, incarnation, VM generation), a session, and a generation-checked channel. M2 withdraws restricted mappings and performs TLB shootdown before releasing backing; M4 supplies authenticated Doorbell operations; M5.2 supplies the native ring and retirement tracing. These historical results do not establish behavior for this routing draft. The draft invokes its selection only after a channel is paired, takes the service role from the kernel-assigned embedded path and authenticates the actual channel participants. It does not issue or validate an independent policy identity, and no fixture implements the requested end-to-end operation. Its channel generation is not a policy generation. Experimental modules are guarded by `debug_assertions` and `fastlane_contract_test`; the latter implies the debug-only channel feature.

A viable integration must bind a kernel-owned, independently generated policy record to the live service's full identity, resolve it using that identity on establishment, and retire it on service replacement or revocation. Existing authenticated channel establishment, mapping and ring code should then be used without weakening their checks. Conventional and ring implementations need a shared operation contract, with cleanup complete before any new fallback attempt.

## Host-only reference model

`tools/fastlane_m61b_r1_model.rs` is a standalone host reference model, **not kernel code and not an authorization API**. It uses a four-slot catalog, slot ID plus a separately incremented generation, exact service PID/incarnation/VM matching, explicit retirement and checked generation exhaustion. Reuse changes the generation; an identity alone does not pass service binding. The model does not demonstrate native issuance, lifecycle hooks, concurrent serialization, or stale-policy enforcement in the kernel.

Its proposed fixture accepts one nonzero request ID and 0–32 bytes and returns the same ID and reversed bytes. Both modeled transports apply the same validation, result, and duplicate rules. Invalid requests leave the committed connection available; a valid request publishes once and completes synchronously; duplicate IDs are rejected, as are later distinct requests. Cancellation succeeds only before commit. The model's commit is `Preparing -> Committed`; transport is chosen before that transition and stays fixed. A failed or cancelled attempt has no modeled resource allocation. These are proposed semantics only: no conventional IPC or ring implementation executes them, and the host test compares two invocations of the same reference implementation. It is not proof of cross-transport equivalence, cancellation races, or resource cleanup.

## Validation and control matrix

| Control | Evidence and limit |
|---|---|
| Existing policy state machine | Four host tests pass; this is the old draft, without independent policy identity. |
| Reference policy and operation | Four standalone host tests pass, including stale/foreign identity, bounded exhaustion, separate operation/transport checks, modeled duplicate and cancellation behavior. No native enforcement. |
| Negative parser | Old draft host tests reject several malformed encoded offers and policy values. No R1 native receipt parser exists; the requested negative parser suite was not completed. |
| Experimental kernel | `cargo check -p sunlight-kernel --features fastlane_contract_test` passed. |
| 2-CPU native pilot | **Not run**: no integrated operation, policy lifecycle, native fixture or R1 receipt parser exists. Starting QEMU could not establish the requested proof. |
| Release exclusion | Default release build passed; its ELF contains none of the M6.1B receipt or fixture-path markers. Release check with `fastlane_contract_test` failed as expected and includes the explicit debug-only compile error, alongside other errors. This is targeted evidence, not an exhaustive production proof. |

Required native negatives—identity forgery, operation denial, stale/foreign policy, duplicate attachment, forbidden downgrade, peer closure, cancellation, stale channel, post-publication fallback and mapping allocation failure—were **not exercised**. Model checks are not native receipts. No R1 attempt has a native terminal-outcome or inventory witness. No scoped grant/receipt cleanup, view removal, remote TLB ACK ordering, frame retirement, or stale adoption after closure has been proven for this source. In particular, a zero-target shootdown cannot prove a remote acknowledgement. No failed or aborted native runs exist for this milestone; no image was rebuilt for a pilot and no 2/4/12 matrix was started.

Logs and release marker/hash results are in `.fastlane-evidence/m61b-r1/`. The reference-model file SHA-256 at validation was `874833ea355e1136ef33e74fdc331f5855a0ef1974a2f4aef03886c74946d485`.

## Smallest executable next proof and R2

Implement the policy catalog inside the existing serialized kernel authority and connect its issuance and revocation to the trusted fixture service's exact lifecycle. Add a real conventional fixture operation and an independently implemented ring equivalent, authenticated caller setup, cleanup and bounded receipts. Add parser rejection tests and run host controls before constructing one source-matched 2-CPU image. Exercise success and all required failures with scoped inventory and physical release traces, explicitly distinguishing a real remote target and ACK from a zero-target release. Until then, all native acceptance criteria remain open. If a safely exercised remote target cannot be produced in that pilot, retain it as an explicit R2 gap. Full CPU matrix, broader service policy lifecycle, and production routing remain future work.

## Continuation 01: kernel policy identity, still PARTIAL

The continuation starts from the same HEAD, `eeba257e4c548af61a2b2c6aeeeb734ab8b07cc6`, including the existing R1 report and reference model. New evidence is isolated under `.fastlane-evidence/m61b-r1/continuation-01/`; previous evidence was not replaced. `source-before.json` hashes tracked and nonignored untracked files outside the evidence tree; its SHA-256 is `6845752d9bf7d868d8e49f3dd4650e15dd1df5c1d7bd5e435418db90f9570ef0`. `status.txt`, `head.txt`, and `initial.patch` retain the starting tracked-tree state. This source manifest is not a clean-commit or build-environment attestation.

### Implemented authority model

`channel_contract.rs` now contains the actual kernel `PolicyIdentity { policy_id, generation }` and an eight-slot `PolicyCatalog`. `ChannelTable` owns it under the existing broker serialization; no additional lock or userspace issuance API was added. Issuance is lazy at experimental contract begin, after checking that the actual caller is the channel creator and that the authenticated channel is paired. The peer's trusted embedded-image role and full kernel-observed `RecipientIdentity` determine the record. Unspecified services cannot obtain records. Existing operation checks remain independent and run in `Contract::begin`; a policy reference does not grant an operation or channel authority.

Slot IDs are reused only with a fresh generation from the catalog's own checked monotonic allocator. No channel allocator value feeds this allocator, even if numeric values happen to coincide. A full catalog returns `Capacity`. Generation overflow returns `Exhausted` before changing any slot and remains exhausted. Repeated lookup for the same exact service identity/role returns its existing reference; a conflicting role is denied. Resolution checks slot, generation, full process identity and trusted role. Exit and successful exec use the existing `ChannelTable::retire_process` hook to revoke catalog entries as well as channel records. There is no standalone revocation syscall; this pilot catalog's explicit revocation mechanism is process/image retirement. It does not claim a general service policy update protocol.

The native adapter records this independent identity in the connection draft and revalidates it against the live peer on subsequent contract operations. Receipts now distinguish `policy_id`, `policy_generation` and `channel_generation`. These are code and host-test results, not observed native receipts. The adapter still starts after existing authenticated pairing; it does not implement the requested complete request-to-close routing flow. Failures before `Contract::begin` still lack R1 terminal-outcome and cleanup proof. Catalog records are service-lifetime records, not per-attempt grants; an operation rejection does not revoke a live service's policy.

### Validation from this continuation

| Stage | Result | Evidence / limitation |
|---|---|---|
| Actual kernel contract/policy host tests | PASS, 6 tests | `host-contract.log`: old four contract tests plus actual catalog binding/revocation/reuse and deterministic exhaustion tests. |
| Reference model | PASS, 4 tests | `host-model.log`; remains a standalone model, not cross-transport semantics proof. |
| Experimental kernel type-check | PASS | `experimental-check.log`. |
| Negative native receipt parser controls | NOT IMPLEMENTED | No R1 native receipt grammar/parser or native operation yet. Existing malformed offer tests are not substituted for this. |
| 2-CPU routing pilot | NOT RUN | No source-matched end-to-end fixture exists. No native image was built or rerun. |
| Ordinary release build | PASS | `release-build.log`; targeted marker scan in `release-markers.json`. |
| Release with experimental feature | EXPECTED REJECTION | `release-feature-denial.log` includes the explicit debug-only compile error and additional feature-exclusion errors. |
| Patch whitespace | PASS | `diff-check.log`. |

Default release ELF SHA-256: `a7bc7e0c2fcdd903eb09046f9dcf6f071b962e4b5e0ce23a67413f94eb6a926e`. The byte scan found no `[FL-M61B-K]`, fixture service path or `policy_generation=` marker. This is targeted production exclusion evidence; production FastLane remains disabled. No full CPU matrix, commit or push occurred.

### Required native controls and retirement status

| Required control | Current proof |
|---|---|
| Forged caller | Existing authenticated channel checks are reused; no R1 native attempt. |
| Unauthorized operation on permitted transport | Actual contract host rejection; no R1 native attempt. |
| Stale policy generation | Actual kernel catalog host rejection after retirement/reuse; no R1 native attempt. |
| Foreign policy identity | Actual catalog host rejection for foreign PID/incarnation/VM/role; no R1 native attempt. |
| Duplicate attachment | Contract host duplicate rejection is narrower; attachment itself untested for R1. |
| Required downgrade | Actual contract host rejection; no R1 native attempt. |
| KernelOnly FastLane | Actual contract host rejection; no R1 native attempt. |
| Peer closure during establishment | Unproved for R1. |
| Cancellation before commit | Reference model only; unproved for R1. |
| Stale channel generation | Existing channel mechanism only; no R1 native attempt. |
| Fallback after publication | No native operation publication exists; unproved. |
| Mapping/grant allocation failure | No R1 injection or native cleanup witness. |

There are no new native positive, negative, failed or aborted attempt receipts. No R1 evidence proves grant/receipt withdrawal, mapping/view removal, remote TLB target/ACK ordering, exactly-once backing-frame retirement, zero final scoped inventories, or stale adoption after close. Metadata counts and zero-target shootdowns are not physical-retirement proof. Historical M2/M5.2 evidence was not reused as proof for this changed source.

The draft's establishment commit remains the successful `Contract::finish` transition from Preparing to Ready in operation 32, after transport-specific setup checks. Transport cannot be selected again by this contract afterward. This is not yet an operation-publication boundary with proven cleanup and cancellation ordering. FastLaneAllowed's existing initial offer selects conventional transport before setup; recovery from partially allocated FastLane setup is not implemented or proven. An implementation must retire a failed attempt's resources before starting a fresh conventional attempt.

### Smallest missing executable proof and R2

The next executable step is a single source-matched fixture pair using existing authenticated channel establishment, with an actual conventional IPC bounded operation and an independent ring implementation of the same request, completion, error, cancellation and duplicate contract. The current reverse-bytes host model is only a proposed contract. Integrate independent policy references into that complete establishment protocol and add bounded native receipts plus parser negative controls before building the 2-CPU image. Then exercise the twelve controls and capture physical retirement witnesses. A safe nonzero remote TLB target plus ordered ACK must be demonstrated or explicitly retained for R2; it is currently entirely unproved for R1.

R2 also retains the broader policy update/replacement lifecycle, full 2/4/12 validation and any production routing review. This continuation does not complete R1 or establish production readiness.
