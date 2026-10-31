# Secure native integration boundary

Continuation on 2026-10-31: see
[`fastlane-pending-reply-race.md`](fastlane-pending-reply-race.md). The late
deferral-after-closure defect is now reproduced and fixed, with overlapping
native kernel-method races on 2/4/12 CPUs. Authenticated Account peers, real exec
with outstanding deferred authority and global grant/mapping reconciliation
remain unproved; the requested security milestone remains PARTIAL.

Audit date: 2026-10-31. This report covers the current dirty worktree; historical
results are not final-source acceptance. Production FastLane, Restricted SHM and
public Doorbell remain disabled. No commit or push is authorized.

## Preimplementation security and lifecycle contract

The current executable prerequisite is generation-safe deferred IPC completion.
`defer_current_reply` uses `wrapping_add(1).max(1) | (1 << 63)` and can reuse a
previous completion handle in the same server. This is a source-proven exhaustion
defect, not evidence of a practical 2^63-operation exploit. Correct it before
using deferred control-plane operations for a native channel.

Existing IPC SEND authorizes requests and endpoint-owner RECV authorizes receipt.
Deferral only transfers the calling server's already-owned reply target into its
bounded, process-local completion table. A completion token is not transferable
authority: lookup is scoped to the current kernel-authenticated server process.
The target binds caller PID, process incarnation, endpoint ID and IPC call
generation. Call generations and endpoint IDs already use checked allocation.
VM-qualified capability grants additionally bind the recipient's VM generation;
the ordinary reply token alone is not a mapping grant or a VM identity.

Required deferral transaction: validate current target; check bounded capacity;
compute a nonreused handle; reserve metadata fallibly; then transfer ownership.
Failure preserves the ordinary reply target, old handles, counter and pending
caller. Exhaustion permanently refuses further deferrals for that process;
ordinary reply remains available. Only final process reaping resets the counter.
Cancellation, deadlines and endpoint closure remove deferred ownership. Delivery
consumes it once. A late completion must neither overwrite an earlier terminal
result nor adopt a newer call or replacement process incarnation.

| Event | Required terminal behavior |
|---|---|
| Reply before cancellation/closure | Exactly one ReplyDelivered; later cancellation/closure cannot replace it |
| Cancellation/deadline/closure before reply | One Cancelled/DeadlineExpired/PeerClosed; deferred handle becomes invalid |
| Caller exit | No delivery to an exiting caller; existing teardown withdraws pending authority |
| Numeric PID replacement | Old caller incarnation cannot complete the replacement's pending call |
| Server exit | Existing endpoint and target teardown applies; no handle inheritance |
| Deferred handle exhaustion | QueueFull, no wrap, no target loss, ordinary reply possible |
| Unresponsive peer | Bounded metadata; caller cancellation/deadline remains usable |

These rules use existing SCHEDULER and IPC shard ordering. No new lock or payload
hot-path work is permitted. Native boot-method tests are not a forced concurrent
race between running independent userspace processes.

## Channel authority and memory contract (not implemented)

No FastLane create/attach capability class currently exists. SEND, RECV, legacy
SHM bearer possession, service names and userspace PIDs cannot substitute for
one. Future kernel issuance must explicitly authorize creation, bind exactly one
producer and consumer using PID/incarnation plus VM root/generation, and issue
nondelegable role grants for a checked channel slot/generation. Creation and
attachment must remain unavailable until that policy and its independent gates
are implemented. A concrete create/attach ABI is intentionally not published.

Backing must have kernel-owned Restricted provenance and sanitized pages, with
producer cursor RW/RO, consumer cursor RO/RW, and payload RW/RO for producer and
consumer respectively; all views NX. Legacy mapping aliases must be denied.
The cooperative Rust lease ring cannot be directly attached to hostile writable
memory: malicious mutation can violate borrowed non-atomic payload assumptions.
That unsafe boundary needs a separate representation/protocol review.

Close/exit/exec must fence admission and publication, wake registered waiters,
and retain backing until exact mappings are removed and synchronous TLB
acknowledgements complete. An outstanding userspace lease is not permission to
reuse frames. PID/channel/VM generation exhaustion must fail closed. The existing
Doorbell sequence/check/register ordering and bounded remote wake path must be
retained. No production transport or performance comparison is authorized by
this prerequisite's success.

## Independent gates at audit entry

Scheduler A and token history have historical scoped proofs. Current reports
classify Reaper/Endpoint Ordering, Grant Lifecycle, Global Authority, Session
Authority and Doorbell B PARTIAL; F1.1B-0 and production Gate C BLOCKED. Missing
proofs include actual late publication versus endpoint closure, stale endpoint
adoption, VFS grant races, common-epoch service inventories, causal closure of
N3M/D3 failures, recipient-bound mapping rights and active mapping revocation.
They remain independent of deferred-handle exhaustion.

## Implemented source and unsafe-code audit

`kernel/src/ipc/deferred_token.rs` now allocates the original high-bit wire
encoding with checked addition. `u64::MAX` is issued once; every later attempt
returns QueueFull. `defer_current_reply` checks capacity and handle exhaustion
and uses `VecDeque::try_reserve(1)` before clearing the ordinary reply target or
advancing the counter. There is no new lock, syscall, capability, mapping,
dependency or unsafe block. Successful ordinary/deferred delivery and the
existing grant-settlement paths remain unchanged. QueueFull at exhaustion is a
deliberate fail-closed behavior change; services can still reply ordinarily.

The opt-in `ipc_deferred_test` feature is off by default and additionally gated
by debug assertions. Its native fixture uses real Process/address-space records,
the production call/receive/defer/complete/cancel/terminal methods and a private
ordinary capability broker. It does not run independent ring-3 participants or
the authenticated Account service. Capacity, counter exhaustion, incarnation
replacement and Finished state are explicitly injected. No lock-held rendezvous,
timing inference, fabricated grant, secret-token logging or transport enablement
is introduced. Its method calls execute sequentially on the bootstrap CPU;
configuring more CPUs does not turn them into concurrent peer races. The
fixture's final inventory and host acknowledgement are scoped
to its private IPC state, not global service/allocator reconciliation.

The production methods remain serialized by the existing scheduler guard.
Copying the target before reservation does not duplicate published authority:
the old field is cleared before inserting the single deferred record, all under
that guard. Failed reservation mutates neither ownership nor counter. Previously
returned handles are not recycled while the process is live, including after
cancellation. Reaping's reset occurs only after clearing all reply metadata for
the finished process. Native allocator-failure injection was NOT_RUN; the error
path is a source-audited propagation of try_reserve failure.

Additional source boundary: ordinary IpcCallId does not carry a VM identity.
`spawn::exec_into_process` withdraws grant/image roles and clears ordinary reply
state, but does not explicitly clear the deferred completion table. This report
does not classify intended process-lifetime IPC inheritance as an exploit.
Before reusing it for VM-scoped channel authority, specify the exec policy and
test withdrawal/rejection against the old VM. No such proof is claimed here.

## Final-source validation

Evidence root: `target/fastlane-native-boundary-evidence/`. Every build/native
attempt has its own manifest. The harness records commands, exits, full source
hashes, ELF/ISO hashes, seven normal embedded service identities, serial hashes,
and separately written/read-back host acknowledgements. ISO extraction verifies
the actual boot ELF. The same frozen image is used for 2/4/12 CPUs. Host parser
negatives check omitted/duplicated cases, missing boot regressions, incorrect
CPU/identity/inventory, faults, feature contamination and scope promotion.

The original counter expression was extracted unchanged for a host reproducer:
1 test passed, 2 failed (exit 101), demonstrating stale-handle reuse at the
boundary. Its source, binary and output remain in `reproduction/`; these are
expected pre-fix FAIL results, not native exploit evidence. An initial standalone
compile used pub(super) at crate root and failed; visibility was corrected before
running the captured reproducer. No native attempt failed in the new fixture.

| Final-source check | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Nine opt-in deferred IPC boot-method cases | PASS | PASS | PASS |
| Invalid ordinary request capability and server-local completion lookup | PASS | PASS | PASS |
| Stale handle, cancel-first, reply-first and close-first terminal outcomes | PASS | PASS | PASS |
| Injected stale caller incarnation and exiting-caller rejection | PASS | PASS | PASS |
| Injected capacity rollback; handle/call generation exhaustion | PASS | PASS | PASS |
| Existing IPC security/queue/deadline/deferred lifecycle gates | PASS | PASS | PASS |
| Ring-3 conventional IPC 1,000-call regression | PASS | PASS | PASS |
| Existing VM/SHM retirement, sanitization and failed-unmap gate | PASS | PASS | PASS |
| Remote SHM unmap acknowledgements | PASS (1) | PASS (3) | PASS (11) |
| Native fixture cleanup and independent host ACK | PASS | PASS | PASS |
| Forced concurrent reply/endpoint-closure race | NOT_RUN | NOT_RUN | NOT_RUN |
| Actual numeric PID reuse / real peer-exit interleaving | NOT_RUN | NOT_RUN | NOT_RUN |
| Native FastLane creation/attachment, channel rollover and lease-death cases | BLOCKED | BLOCKED | BLOCKED |
| Hostile channel mapping, active channel revocation and channel frame reuse | BLOCKED | BLOCKED | BLOCKED |
| Native FastLane/conventional IPC performance comparison | BLOCKED | BLOCKED | BLOCKED |

Native manifests: `native-20261031T102715.212346Z/validation-summary.json` (2)
and `native-20261031T102812.088918Z/validation-summary.json` (4/12).
Build: `build-20261031T102631.723204Z/validation-summary.json`.
All recorded source maps remained unchanged throughout these executions.

Host checks in `checks-20261031T102716.387174Z/` passed: 136 kernel tests,
266 service/library tests (including 12 ring and 9 Doorbell tests), 3 ring
documentation tests, 19 manager runtime tests, focused scheduler production
state/lifecycle/accounting/policy checks, 5 parser methods with negative
subcases, and diff whitespace. New Rust module formatting and the feature-enabled
bare-metal kernel check passed separately. The two ring stress paths each retain
their 1,130,000-transfer assertions; no host benchmark was rerun or promoted to
native performance. Strict Clippy, complete workspace formatting, Miri, broad
D1/D2/D3 and network/UI regressions were NOT_RUN in this milestone. Earlier
independent failures remain unresolved.

The first Doorbell driver was interrupted (exit 130)
while Cargo waited for the shared build directory; that attempt is BLOCKED,
with native execution NOT_RUN, and its logs are preserved in `doorbell-final/`
and `doorbell-driver.log`. The retry uses the existing isolated build directory.

The isolated final-source Doorbell retry passed on 2/4/12 CPUs using the existing
native fixture and its unchanged validator: 42 transitions and 43 object
lifetimes per configuration, actual ring-3 workload execution on every CPU,
cross-CPU notification delivery, and zero active objects, waiters, task references,
obligations, deferred work, pending requests, queued entries and pins. Frames
before/after matched at 72,676 / 72,164 / 70,116 respectively. These are independent
kernel Doorbell regressions, not public API or cross-process transport evidence.
Doorbell B remains PARTIAL because the independent D3 causal/combined replay
requirements are not closed. Evidence: `doorbell-isolated/results.json`.

The ordinary production services, kernel and ISO also passed their fresh build
and ISO extraction checks. The production feature list is empty and the new
`[IPC-DEFER]` marker is absent; ordinary service payload inclusion and unrelated
fixture marker exclusion passed. Evidence:
`production-20261031T103106.835693Z/validation-summary.json`. This is build and
provenance evidence; a separate ordinary-image QEMU replay was NOT_RUN. The
instrumented-image IPC/VM native matrix above remains the runtime evidence.

| Additional check | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Existing cross-CPU Doorbell fixture, 42 transitions | PASS | PASS | PASS |
| Doorbell frame/reference cleanup | PASS | PASS | PASS |
| Ordinary production service/kernel/ISO build and marker exclusion | PASS (one build, no CPU-specific claim) | same build | same build |

Reproduce with fresh attempt directories:

```sh
python3 tools/fastlane_native_boundary_validate.py --section checks
python3 tools/fastlane_native_boundary_validate.py --section build
python3 tools/fastlane_native_boundary_validate.py --section native --cpus 2 --image-manifest <fresh-build>/validation-summary.json
python3 tools/fastlane_native_boundary_validate.py --section native --cpus 4 12 --image-manifest <fresh-build>/validation-summary.json
python3 tools/fastlane_native_boundary_validate.py --section production
```

The Doorbell retry's `prebuild.json` records the actual isolated Cargo command,
environment, source and image hashes; its driver used `--skip-build` with those
archived bytes. The original runner's default build command is not claimed as
the executed isolated prebuild. The root `validation-summary.json` indexes all
attempts and hash-verifies the source/image/serial/host receipts; `final-source/`
and `implementation.patch` retain the implementation. Preexisting dirty-worktree
changes are preserved; the patch includes the existing Cargo version change.
No commit or push was performed, and no unrelated desktop/audio source changed.

## Remaining blocker and next executable milestone

The generation-exhaustion prerequisite is complete within the tested scope.
No independent security gate is promoted: production Gate C stays BLOCKED and
native FastLane performance has zero samples and no numbers. There is no new
native channel, mapping API or production Doorbell entry point.

Next: force an actual pending reply versus endpoint closure using existing
production paths, exact authenticated caller/server incarnations, VM identities,
endpoint ID and call generation. Observe both winner orders on separate CPUs,
one terminal outcome, rejected late grant publication, rejected stale adoption,
and final cleanup/host acknowledgement, with coordination outside scheduler
guards. Define and validate the exec/deferred
authority policy before using that path for VM-qualified grants. Then close the
remaining independent authority/Doorbell gates before Restricted SHM attachment
and hostile-peer zero-copy work. No benchmark can precede those security proofs.
