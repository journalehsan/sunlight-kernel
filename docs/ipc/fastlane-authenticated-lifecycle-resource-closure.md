# Authenticated reply lifecycle and resource closure

2026-10-31 continuation: [authenticated completion versus actual caller exit](fastlane-authenticated-completion-caller-exit-race.md)
tracks the separate real ProcessExit syscall contest and per-object retirement.
Its final validation and preserved regressions are scoped to that milestone;
production FastLane and the independent security gates remain disabled/unchanged.

Continuation: [authenticated Account exec and per-object SHM retirement](fastlane-account-exec-shm-retirement.md)
adds actual privileged Account exec and correlated physical-release observations.
The results below remain the original milestone's evidence and classifications.

2026-10-31. **PARTIAL.** Production FastLane, Restricted SHM and public Doorbell
remain disabled. No commit or push was performed. This milestone extends the
pending-reply method proof into actual authenticated userspace operations; it
does not close all Phase A/B/C acceptance criteria.

Evidence root: `target/fastlane-authenticated-lifecycle-evidence/`.
The machine-readable entry point is `validation-summary.json`. Previous audit:
[`fastlane-pending-reply-race.md`](fastlane-pending-reply-race.md). Historical
failures and independent security gates retain their previous classifications.

## Root causes, contracts and changes

The preceding milestone repaired late deferral after closure, checked completion
handle exhaustion, and withdrew old-image incoming/outgoing reply authority on
successful exec. Its native drivers supplied server identity from kernel fixture
code. That left an authentication and lifecycle evidence gap. No additional
production race defect is established by this continuation.

The new `session-reply-test` UAC feature includes the existing N6 userspace reaper
mode. After normal root authentication, session checks and a real Account grant
commit, the original AUTHORIZE request remains pending. Account calls the ordinary
controller rendezvous. That controller defers the call, spawns an ordinary reply
peer, verifies the child's kernel badge, and delegates a SEND endpoint to Account.
No trusted role or manager capability is assigned to the reply peer.

The new module in `services/sunlight-uac/src/bin/authenticated_reply/mod.rs` runs
15 operations: three repetitions of the four owner-ordered reply/closure cases,
two actual successful exec replacements with outstanding deferred Account calls,
and one real peer exit with an outstanding deferred Account call. Each exec also
attempts a nonexistent executable and verifies the old completion remains live
after failure. Handles cross private test IPC/argv only; they are never logged.

The controller first exposes its still-live completion handle to the child. The
child's completion/liveness syscalls reject it; the controller independently
checks that its original target is still live before completing it. For every
operation Account tries management using both a SEND token and the actual owner
token held by the wrong process. Receive and destruction are rejected. The owner
then receives the real Account call, tests old handles against newer pending calls,
and executes ordinary defer/complete/destroy/exec/exit syscalls.

The kernel feature `authenticated_reply_test` adds observations only. A bounded
15-entry atomic endpoint list links actual received calls to accepted terminal
transitions in `finish_call`. Kernel snapshots supply real PID/incarnation/VM,
endpoint lifetime ID, call generation, Account role, CPU, deferred inventory and
consumed terminal state. No production algorithm consults this list. Reaper
instrumentation additionally reports the reply peer's ordinary IPC cleanup.
There is no new lock, transport, syscall, capability policy or identity injection.
The feature is off by default; kernel observations require debug assertions.

`--authenticated-reply` requires H4/N6 userspace-reaper mode. The runner refuses a
different image mode and requires both the existing H4/N6 assessment and the new
fail-closed reply assessment before writing a separate host ACK. Parser controls
remove every required record individually and reject duplicates, incorrect
identity/authority/generation/CPU, missing exec, stale terminal state and faults.
They never count as native evidence.

The first full build failed because Cargo auto-discovered a shared file directly
under `src/bin` as a standalone executable. Moving it into a module directory
fixed packaging. The failed build and source hashes remain in
`build-20261031T111351.989200Z/`; it produced no native acceptance or host ACK.

## Authenticated operation and lifecycle proof

Account is the **caller** of the new reverse-direction reply cases. The parent
fixture's original AUTHORIZE request is simultaneously outstanding against the
actual Account server. These two call directions are not conflated.

| Operation | Independent terminal receipt | Further check |
|---|---|---|
| Defer, then owner closes endpoint | One PeerClosed | Old deferred completion denied |
| Owner closes, then attempts deferral | One PeerClosed | Deferral denied; stale ordinary target discarded |
| Deferred completion, then owner closes | One ReplyDelivered | Duplicate/late completion denied |
| Owner closes, then deferred completion | One PeerClosed | Completion denied |
| Failed exec with deferred reply outstanding | Existing handle remains live | Successful exec follows separately |
| Successful exec with deferred reply outstanding | One PeerClosed | Old handle denied in replacement image |
| Reply peer exits with deferred reply outstanding | One PeerClosed | Real exit/reap and both endpoint handles invalid |
| Account terminated by ordinary authorized manager | One original AUTHORIZE PeerClosed | Committed grant/receipt/delivery retired; replacement denies old session |

For each of the 15 reverse calls, exactly one accepted kernel terminal record
matches the captured caller, incarnation, VM, endpoint and call generation.
Account independently returns from the call with that result. Its subsequent
kernel snapshot has no pending call, terminal outcome or reply payload. Stale
completion retries cannot overwrite it. New calls succeed while old completion
attempts are rejected. Retired endpoint calls must return InvalidCapability or
EndpointNotFound; a timeout does not pass this assertion.

Successful exec preserves the reply peer's PID, process incarnation, UID/GID and
existing process-owned endpoint authority, while installing a distinct VM. Its
deferred table goes from one entry to zero. The replacement rejects the old
handle and replies through the inherited endpoint to a fresh pending call. This
is actual native exec, not a direct call to an exec cleanup method. The peer has
no privileged image role; this does not prove Account/session-owner role
withdrawal with an outstanding call.

After the reverse calls and peer reap, Account returns to the existing bounded
userspace readiness loop. The prestarted authorized sunlightctl performs its
normal restart; the real timer fatal-SIGTERM path terminates Account. Endpoint
removal and the matching original PeerClosed precede broker retirement in the
resource-reap path. The replacement has a different incarnation/VM and denies
the old session. Fixture, controller, sunlightctl and reply peer cleanup precede
the host ACK.

### Prevented and missing interleavings

These four userspace cases are **owner-ordered syscall transitions**, with an
independently blocked authenticated caller. They do not establish two concurrent
mutation entrants on separate CPUs. Only the endpoint owner can destroy/receive;
completion handles are server-local; reaping waits for execution quiescence.
Holding that owner inside a deferral syscall while asking for its own reap cannot
force an independent close-first mutation. The prior concurrent kernel-method
fixture remains separate. No ownership bypass or impersonated closer was added.

Actual numeric PID reuse is **BLOCKED by the current allocation contract**:
`process::pid::next_pid` uses checked max-existing-PID plus one. Reaped task slots
retain their PID until replacement with a larger allocated PID; exhaustion fails
closed. Slot reuse is not numeric PID reuse. Native stale-handle adoption by a
different process incarnation remains **NOT_RUN** in this authenticated fixture.

## Grant and mapping reclamation boundary

The original Account grant is real, committed and pending delivery. Each accepted
N6 run observes one mint, zero consumes, one retirement, and final zero sessions,
receipts, Account grants and pending Account deliveries. The fixture's explicit
SHM request/readiness mappings use existing ordinary SHM. Account's mappings are
cleaned through real process termination; the controller explicitly frees its
readiness view and checks success. The existing requester helper attempts to free
its request view but discards that syscall's result; its later real reap is
observed. That unchecked call is not a separate mapping-release receipt. This is
bounded grant/IPC metadata cleanup and exercised ordinary SHM teardown paths.

It is **not Phase C backing-frame reclamation acceptance**. No new correlated
trace records every view's PTE removal, remote TLB acknowledgement, backing-frame
retirement/reuse, allocator ownership or service heap inventory for these
authenticated operations. Repeating the IPC cases does not prove repeated SHM
frame/metadata reconciliation. Retained broker anti-reuse history and container
capacity are not live grants, and their retention is not called a leak or zero
resource drift. Restricted SHM is still disabled.

The committed grant authorizes an Account operation; it is not a recipient-bound
SHM mapping grant. Ordinary SHM retains its existing bearer policy. No channel
create/attach permission, active view revocation or hostile-mapping guarantee is
inferred from these results.

Source audit confirms the existing SHM release contract: preflight exact PTE and
ledger identity; `unmap_page` clears the PTE and synchronously invalidates TLBs;
then remove mapping/ledger ownership and decrement the broker reference count;
only the last view returns backing frames to PMM. Failure pins backing or fails
stop. The independent existing VM/SHM gate must be reported separately from
authenticated mapping-race proof.

## Validation and provenance

Base revision: `02955ae7eee07f5fa453a18e920dd248eb35d488`, with an existing dirty
tree. `entry/` retains the entry status, tracked/staged patches, full source map
and source archive. The implementation source map contains 1,312 files; canonical
SHA-256 is `3ca541e6a4b77d24d80024e1d2cb6a81b10baa95125dfce9f59a6a5e68441086`.
`final-source.json` and `final-source.tar.gz` retain the final inputs, including
untracked new modules. Each attempt records source hashes before/after, actual
commands, exits, archived embedded executables, ELF/ISO and serial hashes.
The final ISO-extracted ELF is compared with the executed archived ELF.

Final authenticated build: `build-20261031T111829.020791Z/`.
Final 2-CPU attempt: `native-20261031T111857.033981Z/`.
Final 4/12-CPU attempt: `native-20261031T112016.131621Z/`.
Full compatibility results are indexed by the root `validation-summary.json`.
Preliminary 2/4/12 passes used a different source
snapshot without the outstanding-peer-shutdown case; their manifests remain
unchanged and are not final-source acceptance.

Final ELF SHA-256:
`45edabbe310fd2aa6665073eac7874db2f360532fc6b02761b51e8df254e62b5`.
Final ISO SHA-256:
`ff6cb1b7eec4f9bf648d3d11d203686eb0e0a61af4641110f019ebee74f9b835`.
All eight embedded executables are archived and identity checked. Fixture IDs
are 261471919 / 165697979 / 882996978 for 2 / 4 / 12 CPUs respectively.

| Final authenticated gate | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Three repetitions of all four owner-ordered reply/close cases | PASS | PASS | PASS |
| Exact authenticated Account, endpoint lifetime and call identities | PASS | PASS | PASS |
| Wrong owner, SEND-only management, live foreign completion rejection | PASS | PASS | PASS |
| One accepted terminal and consumed result for each of 15 calls | PASS | PASS | PASS |
| Late/duplicate and stale completion against a fresh pending call | PASS | PASS | PASS |
| Two failed-exec preservation checks, two real pending-reply execs | PASS | PASS | PASS |
| Real peer exit with deferred Account reply pending | PASS | PASS | PASS |
| Original Account pending call, authorized termination and replacement denial | PASS | PASS | PASS |
| One committed grant retired; scoped grant/receipt/delivery inventory zero | PASS | PASS | PASS |
| Fixture/peer reaping and scoped IPC metadata cleanup | PASS | PASS | PASS |
| Source/image identity and independent host ACK | PASS | PASS | PASS |
| Concurrent independent authenticated closer | BLOCKED | BLOCKED | BLOCKED |
| Privileged Account exec with its original request outstanding | NOT_RUN | NOT_RUN | NOT_RUN |
| Actual numeric PID reuse | BLOCKED | BLOCKED | BLOCKED |
| Authenticated different-process-incarnation adoption attempt | NOT_RUN | NOT_RUN | NOT_RUN |
| Correlated mapping/remote ACK/backing retirement and repeated resource drift | BLOCKED | BLOCKED | BLOCKED |

The new reply observations actually execute on CPUs 0 and 1 at every topology;
the configured CPU count is not claimed as the number of mutation participants.
Account is PID/incarnation/VM 18/30/47 on 2 CPUs and 24/36/59 on 4/12 CPUs. The
reply peer is 42/54, with observed VM generations 93 -> 94 -> 95 in all three
runs. Endpoint IDs are checked, never-reused lifetime identities; this code has
no separate reusable endpoint-slot generation to report.

Final-source host checks passed 140 kernel tests, 266 service/library tests,
three ring documentation tests, 19 manager runtime tests, focused scheduler
checks, the existing boundary/race parser suites and seven new authenticated
parser test methods. New Rust module formatting and diff whitespace passed.

Independent native R2D, Doorbell and scheduler lifecycle images passed on
2/4/12 CPUs. R2D completed eight real lifetimes and restored frame baselines
72,710 / 72,198 / 70,150, with 4 / 12 / 44 forced remote rejections. Doorbell
completed 42 transitions and 43 create/retire lifetimes with zero live reference,
waiter, obligation, pending, queued, deferred or pin inventory and restored frame
baselines 72,677 / 72,165 / 70,117. Scheduler observed one external-exit race at
each CPU count, 6 / 8 / 13 deferred reaps and 2 / 4 / 12 reused process slots.

The independent preceding IPC-method race fixture, nine deferred cases,
conventional ring-3 1,000-call IPC test, and existing VM/SHM gates passed on
2/4/12 CPUs. Their separate remote-SHM release gate observed 1 / 3 / 11 remote
acknowledgements. These are retained compatibility results, not authenticated
grant/mapping retirement measurements.

| Independent final-source compatibility gate | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Prior concurrent kernel-method IPC races and nine deferred cases | PASS | PASS | PASS |
| Conventional IPC and existing VM/SHM gate | PASS | PASS | PASS |
| Remote SHM ACK count | PASS: 1 | PASS: 3 | PASS: 11 |
| R2D remote wake and real lifecycle rounds | PASS | PASS | PASS |
| Doorbell transitions and resource restoration | PASS | PASS | PASS |
| Scheduler lifecycle and external-exit race | PASS | PASS | PASS |
| Existing authority/lifecycle and real exec replacement entries | PASS | PASS | PASS |
| Ordinary production startup, IPC, VM and SHM markers | PASS | PASS | PASS |
| Ordinary image test-marker exclusion and embedded service identity | PASS | PASS | PASS |

Authority evidence is in `compatibility/authority-20261031T112551.676086Z/`.
The entropy-refusal control and 1-CPU authority gate also passed. Each authority
runtime observed eight replacement entries with failed-exec preservation checks,
then entered the original true program. Its archived ISO, extracted ELF and
seven actual embedded executables are independently checked in `image-audit.json`.
These regressions do not add an outstanding authenticated Account call to exec.

Ordinary build: `normal-20261031T112016.131637Z/`. Its empty feature set excludes
all new/previous race, deferred, session and reaper markers. The first ordinary
runtime driver incorrectly required the allocation-failure-injection marker,
which is compiled only with `mm2a_test_injection`. Its 2-CPU attempt timed out and
remains **FAIL** in `production-native/`; 4/12 CPUs were NOT_RUN in that attempt.
Source and actual ELF inspection established the missing marker was not compiled.
The corrected driver verifies that absence and records the feature exclusion;
fresh 2/4/12 executions passed in `production-native-final/`, with separate host
receipts. The instrumented VM regression still requires and passes that injection
marker. No failed attempt is promoted by reparsing, no production test feature is
enabled, and no runtime timeout is enlarged.

Reproduce with fresh attempt directories:

```sh
python3 tools/fastlane_f11b0h4_validate.py --section build --cases N6 --reaper-userspace --authenticated-reply --evidence-root target/fastlane-authenticated-lifecycle-evidence
python3 tools/fastlane_f11b0h4_validate.py --section native --cases N6 --cpus 2 --reaper-userspace --authenticated-reply --image-manifest <fresh-build>/validation-summary.json --evidence-root target/fastlane-authenticated-lifecycle-evidence
```

Expand to 4/12 CPUs only after the smallest run passes. Host regressions and
independent IPC/VM/SHM, scheduler, Doorbell and authority/exec images are retained
under this evidence root. PASS, FAIL, BLOCKED and NOT_RUN remain separate; a
passing isolated gate does not cancel historical combined-replay failures.

## Remaining blockers and next executable milestone

Phase A remains PARTIAL: both owner-ordered results now have authenticated
userspace evidence, but concurrent independent-closer adoption/closure races are
not forced under the existing owner/quiescence contract. Phase B remains PARTIAL:
real deferred-reply exec is observed for an ordinary peer, while privileged
Account image-role withdrawal with outstanding operations and different-process
incarnation adoption are untested; actual numeric PID reuse is unavailable.
Phase C remains BLOCKED for the requested correlated backing-frame, remote ACK,
exactly-once retirement and repeated inventory/drift proof.

The next smallest executable milestone is a separate authenticated **Account
exec** case with its original caller and committed grant outstanding: capture
image-role withdrawal and grant cancellation, verify one caller terminal result,
and have the replacement reject old completion/session authority. Keep its
process-lifetime endpoint contract explicit. Then add per-object SHM mapping and
TLB retirement observations to the authenticated shutdown path before repeated
resource cycles. Do not modify PID allocation or weaken endpoint ownership to
manufacture otherwise-prevented acceptance cases.

Global Authority, Grant Lifecycle, Session Authority, Doorbell B and production
Gate C retain their independent PARTIAL/BLOCKED status. Historical N3M/D3 and
Clippy issues are not closed here. Strict Clippy, Miri, broad combined D1/D2/D3,
network, desktop and audio replay are NOT_RUN in this continuation.
