# FastLane security closure: peer-close-first native path

2026-10-28, Asia/Tehran. Security closure continues before production FastLane
implementation. **The committed-grant peer-close-first resource-reap path is now
VERIFIED NATIVE on 2, 4 and 12 CPUs.** This is a scoped proof, not completion of
all security gates. Production FastLane, Restricted SHM and public Doorbell APIs
remain disabled. No commit or push was performed.

Evidence: `target/fastlane-security-closure-evidence/validation-summary.json`.
H5's report, images, source identity and all earlier attempts remain preserved.

## Changes and actual production path

The first step used the existing H4R fixture on the exact H5 image. On 2 and 4
CPUs it observed peer-close before retirement and denied the finished issuer's
precommit grant. Those runs validate the earlier source snapshot and are retained
as preparatory evidence. They do not prove the historical committed-grant case.

The new opt-in UAC feature `session-reap-test` extends the existing N6 fixture.
After a genuine authenticated grant commits, Account passes an ordinary shared
page through the existing private rendezvous. The controller replies, allowing
Account to return to ring 3. Account publishes readiness from userspace, then
executes a bounded loop with no further syscall. Only after observing that
publication does the prepared, authorized `sunlightctl` submit its normal restart
request. The ordinary manager sends SIGTERM; the existing timer applies the fatal
signal and the normal resource reaper performs cleanup. Controller polling and
Account's loop are bounded; no kernel spinlock is held across coordination.

Account's original mapping is cleaned by process reaping. The controller releases
its mapping and records successful cleanup before exiting. Timeout/failure does
not manufacture completion. Ordinary builds exclude the feature and the new
marker. Manager permissions, process identities, scheduler locking and production
authorization are unchanged. The page uses existing shared-memory IPC, not
Restricted SHM or a new transport API.

`--reaper-userspace` selects this mode in the existing H4 runner. It requires
`--cases N6`, rejects an image with the wrong mode, and adds host predicates for
the real commit, pending delivery, ring-3 readiness, timer SIGTERM termination,
resource-reap path, exact peer-close/retirement order, mint/consume counts and
coordination-map cleanup. Existing general parser predicates remain unchanged.

## First failure, correction and final native results

The first modified N6 run, `native-20261027T221024.744887Z`, reached the intended
guest path and cleanup. Its host acknowledgement was correctly withheld because
the new predicate mistakenly rejected the original valid commitment. Controller
READY enables the trace before the postcommit N6 arm: commitment was sequence 6,
arm 8. The correction requires exactly one original commit before that arm and
rejects later old-issuer grant outcomes. A new unit regression covers both cases.
The failed attempt remains FORCED_AND_FAILED and has not been promoted by
reparsing. A new build and new native executions provide final acceptance.

Final mode image:
`build-20261027T221201.531440Z/validation-summary.json`, under the evidence root.
Kernel ELF SHA-256:
`98b924dfd6ca728257b86f7b7145e73f124ba3de03bfb518709148c1b85a1426`.
ISO SHA-256:
`a7d32c157f97725bc413fd583215dc3e0549402bdc30369d2db16d63b7e95eb2`.
All eight executable payloads match their archived bytes inside the ELF. The
kernel extracted from the ISO also matches the ELF. Full source hashes, exact
QEMU commands and service identities are retained per execution.

| Final mode execution | CPUs | Fixture | Result |
|---|---|---|---|
| `native-20261027T221421.089729Z` | 2 | 764316261 | Mode PASS, host ACK PASS |
| `native-20261027T221533.829155Z` | 4 | 372374421 | Mode PASS, host ACK PASS |
| `native-20261027T221708.187462Z` | 12 | 701925176 | Mode PASS, host ACK PASS |

All three recorded original commitment at sequence 6, postcommit arm 8, resource
reap 13, endpoint removal 14, matching pending-call PeerClosed 15 and broker
retirement 16. The final old grant was pending delivery, not a fabricated mint.
Each old issuer was Account PID/incarnation/VM `24/36/59`, recipient `39/51/87`,
session/epoch `1/1`, endpoint 21 and call generation 4. The endpoint ID and IPC
call generation are separate values; no endpoint-generation value is invented.

Both close and retirement occur on the same CPU inside the production scheduler
guard. Other CPUs cannot perform scheduler-serialized grant/reply operations
between them. The old issuer was already Finished and did not resume after final
retirement. Replacement Account has a distinct incarnation/VM and rejects the
old session with error 11. Final inventories have zero sessions, receipts,
Account grants and pending Account deliveries; one mint and zero consumes were
recorded. Fixture/controller/sunlightctl cleanup and old issuer reaping precede
host acknowledgement. Retained history and metadata capacity are not live grants.
Global service heaps and allocator-wide reconciliation are still UNMEASURED.

The existing base N6 classifier still prints
PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT. Its separate `userspace_reaper=PASS`
and `peer_close_first=OBSERVED` fields certify this mode; no grant/reaper lock
contention is inferred from N6. H5's S1/S2 contention proof remains separate.

Reproduce the final mode with the current source:

```sh
python3 tools/fastlane_f11b0h4_validate.py --section build --cases N6 --reaper-userspace --evidence-root target/fastlane-security-closure-evidence
python3 tools/fastlane_f11b0h4_validate.py --section native --cases N6 --cpus 2 --reaper-userspace --image-manifest target/fastlane-security-closure-evidence/build-20261027T221201.531440Z/validation-summary.json --evidence-root target/fastlane-security-closure-evidence
```

A subsequent build gets a new manifest directory; use that manifest when source
changes. The 4-CPU and 12-CPU runs were expanded only after the preceding CPU gate
passed. Fresh login before each fixture passed; post-retirement fresh login was
NOT_RUN in this mode. The historical peer-close-first failure remains preserved;
it is not reclassified as a successful original attempt.

## Regression and remaining security work

Final-source ordinary H4 S1/S2 replayed on 2 CPUs with both winner orders, genuine
lock contention, cleanup and ACKs. Host checks passed: 133 kernel tests, 258
service/library tests, 19 manager runtime tests, scheduler checks and 52 parser
methods. Changed Rust formatting and diff whitespace passed. Final ordinary
services, kernel and ISO built with test features absent and marker exclusion
verified. Strict kernel and changed-service Clippy are FAIL_CURRENT (the latter
stops at seven existing UAC capability/session-library diagnostics); media/WAV
and workspace formatting are FAIL_CURRENT. Miri is UNAVAILABLE. No matched
baseline claim or unrelated remediation was made.

Remaining gates retain their prior classifications:

| Gate | State and remaining proof |
|---|---|
| Reaper/Endpoint Ordering | PARTIAL; the path is now observed, but late reply/publication and stale-endpoint adoption races remain separate |
| Grant Lifecycle | PARTIAL; issuer retirement/exec versus VFS commit and competing consume/cancel orders remain |
| Global Authority | PARTIAL; correlated Account/sessiond/UAC/VFS drain and service inventories remain |
| Session Authority | PARTIAL; original N3M lookup cause and wider recovery/isolation remain |
| Doorbell B | PARTIAL; original D3 failures still need causal closure and applicable combined replay |
| Scheduler A | VERIFIED NATIVE, preserved |
| Token-History | VERIFIED BOUNDED, 65,536 identities per boot, preserved |
| F1.1B-0 / production Gate C | BLOCKED |

The next smallest security proof is a real pending reply competing with endpoint
closure, with exact caller incarnation, VM, endpoint identity and call generation
and one terminal outcome. It must use existing production delivery/cancellation
paths. Global reconciliation, independent session/Doorbell failures and remaining
grant races must be closed before enabling production FastLane.

## 2026-10-31 prerequisite update

The subsequent source audit found and repaired deferred completion-handle wrap.
Checked exhaustion and fallible metadata reservation preserve the ordinary reply
target on failure. Fresh boot-method lifecycle/identity/exhaustion checks,
conventional IPC/VM/SHM gates and the separate native cross-CPU Doorbell fixture
passed on 2/4/12 CPUs. These scoped results do not close the pending-reply race
above or enable production transport. Contracts, unsafe audit, exact validation
matrix and retained evidence are in
[Secure native integration boundary](fastlane-native-integration-boundary.md).
