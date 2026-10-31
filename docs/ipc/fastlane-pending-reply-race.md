# Pending reply versus endpoint closure

Continuation: [authenticated lifecycle and resource closure](fastlane-authenticated-lifecycle-resource-closure.md)
adds actual Account-caller userspace ordering, deferred-reply exec and peer exit
evidence. Its explicit PARTIAL classification and independent blockers apply;
the kernel-method matrix below remains scoped to its original source snapshot.

2026-10-31. **PARTIAL**: the late-deferral defect is reproduced and fixed, and
both orders pass a genuinely concurrent native **kernel-method** fixture on
2/4/12 CPUs. This does **not** close the requested authenticated Account milestone.
Production FastLane, Restricted SHM and public Doorbell remain disabled.
No commit or push was performed.

## Source and preserved counterexample

Base revision: `02955ae7eee07f5fa453a18e920dd248eb35d488`, with an existing dirty
worktree. The revision alone does not identify these executables. Each attempt
records the complete source hash map, actual commands, ELF/ISO hashes, ordinary
embedded executable hashes, serial hashes and host acknowledgement where valid.
Entry state and patch are retained under
`target/fastlane-reply-race-evidence/entry/`.

The final canonical source-map SHA-256 is
`66526f812f3e24b7523e2013a4421bc0778e533e5085377a0b53bf37f5dab9b8`.
`final-source.json` identifies 1,308 implementation/build input files;
`final-source.tar.gz` preserves their bytes, including new untracked modules.

The pre-fix native attempt is
`reproduction/native-20261031T104840.247321Z/validation-summary.json`.
It is **FAIL**, without host ACK: case 2 entered publication on CPU 0 and closure
on CPU 1; closure committed at sequence 7; deferral then returned success.
The assertion rejected that stale deferred record. The failing execution used
`reproduction/build-20261031T104815.834935Z/validation-summary.json` and its source
archive. It proves a production-method defect, not a demonstrated ring-3 exploit.
The 4/12-CPU pre-fix reproductions are **NOT_RUN** after that failure.
No failed attempt has been reparsed into acceptance.

## Production change and exec policy

Deferral now checks the server's live state and the caller's exact pending
PID/incarnation, endpoint lifetime ID and call generation, with no terminal
outcome already installed. This happens under the existing scheduler guard,
before reservation, counter advancement or ownership transfer. Rejection
preserves the ordinary reply target. Capacity, allocation failure and handle
exhaustion continue to preserve it too. Stale ordinary reply attempts remain
usable to discard that target and cannot replace an earlier terminal outcome.

Deferred liveness and completion use the same exact pending-call predicate.
An exiting server cannot complete. A stale caller target is consumed and its
delivery metadata abandoned, but completion returns `InvalidArgument` instead
of silently succeeding. This rejection is an intentional behavior change.

Successful exec withdraws the old image's incoming ordinary/deferred server
reply authority and completes still-live callers once with `PeerClosed`.
It also withdraws an outgoing old-image call from queues, waiters and all
ordinary/deferred targets, and clears old caller result/deadline state before
return to the replacement image. Failed image preparation preserves authority.
The completion counter is never reset by exec. Endpoints retain their existing
process-lifetime policy; this is not a new VM-qualified channel grant or endpoint
inheritance policy. Ordinary IPC keys do not acquire a VM identity in this change.

No production code consults the test rendezvous. There is no new production
lock, syscall, capability, unsafe block, mapping format or transport. Existing
SCHEDULER/broker/shard ordering, grant settlement, SHM retirement, TLB ACKs,
checked handle allocation and remote-wake algorithms remain in place.

## Concurrent native method fixture

`ipc_reply_race_test` is opt-in, disabled by default and debug-only. The BSP and
AP1 enter independent operation drivers; both readiness events must precede
either operation's mutation. All waits are bounded and occur outside guards.
The requested winner performs its ordinary serialized mutation before the other
entrant proceeds. No timestamp or cross-CPU clock comparison determines order.
Sequence events and actual CPU identities establish the overlapping intervals.

The drivers use real Process/address-space records, ordinary issued endpoint
capabilities, production call/receive/defer/complete methods, checked endpoint
owner destruction, normal peer-closure completion and real process-resource
reaping. Wrong-owner and SEND-only destruction are rejected. Endpoint IDs are
checked, never-reused lifetime identities; there is no separate reusable endpoint
slot generation in this implementation. Call generations are checked separately.

| Case | Winner | Independently checked terminal result |
|---|---|---|
| 1 | Deferral before closure | One `PeerClosed`; deferred entry retired |
| 2 | Closure before deferral | One `PeerClosed`; deferral rejected without target transfer |
| 3 | Deferred completion before closure | One `ReplyDelivered`; closure cannot replace it |
| 4 | Closure before deferred completion | One `PeerClosed`; completion rejected |

After both entrants finish, a third phase consumes the terminal result once,
checks a second read is empty, retries stale completions and ordinary replies,
retries endpoint retirement, and checks no queued calls, waiters, pending calls,
deferred entries or endpoint capabilities remain. Both Process records are
actually reaped; kernel stacks, mappings and ready-queue references are absent,
and physical frames return to baseline. No Account grant is minted by this
fixture. Broker anti-reuse token history and retained container capacities are
not claimed to be globally reclaimed or reconciled by a frame count.

Additional sequential native method checks exercise ordinary/deferred exec
withdrawal, idempotent retirement, unchanged handle counters, outgoing-call
withdrawal and rejection of an old completion while a newer call succeeds.
These checks are not actual exec syscalls or additional concurrent races.
The prior nine boot-method cases retain their explicit injected-identity,
injected-Finished-state and injected-exhaustion scope.

## Final-source validation

Final build: `build-20261031T105327.842895Z/validation-summary.json`.
Final native matrix: `native-20261031T105400.494251Z/validation-summary.json`.
Final host checks: `checks-20261031T105400.494009Z/validation-summary.json`.
All paths in this section are relative to `target/fastlane-reply-race-evidence/`.

ELF SHA-256:
`d3823554f6268300371190dcb73eceb9521805b58e74eb5fad9f5be261460c8c`.
ISO SHA-256:
`3f7f46a94e9334eecd657f63b2b7b76818ae30eba6bec732b8027d443bf84431`.
The ISO-extracted ELF matches. The same archived image ran all CPU counts.

| Check | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Overlapping deferral/closure, both orders | PASS | PASS | PASS |
| Overlapping deferred completion/closure, both orders | PASS | PASS | PASS |
| Single terminal result, rejected late/duplicate attempts | PASS | PASS | PASS |
| Failed-deferral ordinary target preservation | PASS | PASS | PASS |
| Actual fixture reaping and frame baseline restoration | PASS: 72,681 | PASS: 72,169 | PASS: 70,121 |
| Independent host receipt, exact source/image match | PASS | PASS | PASS |
| Existing IPC security, queue, deadline and nine deferred cases | PASS | PASS | PASS |
| Conventional ring-3 IPC 1,000-call regression | PASS | PASS | PASS |
| Existing VM/SHM retirement and sanitization gate | PASS | PASS | PASS |
| Remote SHM unmap ACKs | PASS: 1 | PASS: 3 | PASS: 11 |
| R2D remote wake, eight real lifecycle rounds | PASS | PASS | PASS |
| Doorbell, 42 transitions / 43 lifetimes, zero reference/frame drift | PASS | PASS | PASS |
| Scheduler lifecycle and external-exit race | PASS | PASS | PASS |
| Existing authority/lifecycle and eight real exec replacement entries | PASS | PASS | PASS |
| Exec withdrawal methods; injected stale-incarnation/exiting-caller checks | PASS | PASS | PASS |
| Authenticated Account peers racing actual closure | NOT_RUN | NOT_RUN | NOT_RUN |
| Real exec with outstanding reply, real exit/PID-reuse adoption attempt | NOT_RUN | NOT_RUN | NOT_RUN |
| Recipient-bound grant/mapping reclamation in this race | BLOCKED | BLOCKED | BLOCKED |

Host checks passed: 140 kernel tests, 266 service/library tests, 3 ring doc
tests, 19 manager runtime tests, focused scheduler state/lifecycle/accounting
checks and 10 parser test methods with omission, duplicate, wrong-CPU,
nonoverlap, identity, order, inventory and scope-promotion negatives. Four new
kernel host tests exercise the production identity predicate and the test gate.
Host tests do not establish native interleavings.

Independent regression images are archived in `regressions/{r2d,doorbell,scheduler}/`.
Each `prebuild.json` records the actual isolated Cargo build, current full source
map, seven embedded service identities and matching ISO extraction. The existing
validators then use `--skip-build` with those archived bytes. Their default build
commands are not claimed as executed prebuilds. R2D forced 4/12/44 remote
rejections across eight lifetimes, with frame baselines restored at
72,710/72,198/70,150. Doorbell restored 72,677/72,165/70,117 frames and had no
active objects, waiters, task references, obligations, deferred work, pending
requests, queued entries or pins. Scheduler observed 6/7/12 deferred reaps,
2/4/12 slot reuses and one actual external-exit race at each CPU count.

Authority regression evidence is in
`authority/authority-20261031T105914.555047Z/`: the existing secure-entropy
refusal control and 1/2/4/12-CPU lifecycle gates passed. Each runtime repeated
eight successful replacement entries and eight failed-exec preservation checks,
then entered the original `/bin/true`. This exercises the new successful-exec
cleanup path with no outstanding deferred reply; it cannot substitute for the
missing outstanding-reply exec test. The tested ISO is preserved losslessly,
its extracted ELF matches, and seven exact embedded payloads are archived.

The ordinary production build passed in
`production-20261031T110144.865589Z/validation-summary.json`, with an empty
feature list and all race/deferred/session/reaper markers excluded. Actual ISO
extraction and seven embedded service identity checks passed. This is one
production build; a separate ordinary-image runtime matrix is NOT_RUN.

Intermediate passing builds/checks and a 2-CPU native run precede the final
outgoing-exec withdrawal addition. They remain archived and are not final-source
acceptance. Regression and production exclusion results are indexed by the root
`validation-summary.json`.

## Remaining acceptance blockers and next executable milestone

Kernel drivers possess fixture capabilities and supply recorded server identity
to production methods. They are not independently running authenticated Account
peers. The tests therefore cannot certify the requested authenticated race,
grant issuance/delivery retirement, session ownership, or a common-epoch global
resource census. Recorded VM generations are observations; ordinary reply keys
are not VM-qualified mapping authority. Real exec/PID reuse is not inferred from
method calls or injected state.

In the current ABI, RECV/endpoint destruction require the endpoint owner,
completion handles are server-local, and reaping waits for execution quiescence.
Holding the server inside a syscall while waiting for its own reap cannot force
the close-first order. A new fixture must preserve that contract rather than
impersonating a second owner or bypassing the reaper. Numeric PID allocation is
monotonic and fails closed on exhaustion; forcing live PID reuse would require
test-only identity substitution and must remain separately classified.

Next executable milestone: extend the authenticated Account/controller harness
with pending-reply observations before lifecycle serialization, drive shutdown
through the existing authorized manager, and record both entry/terminal/cleanup
receipts. First classify which close-first interleavings the execution-ownership
contract prevents. Add real successful/failed exec with outstanding deferred
reply and exact recipient/issuer VM identities; reject late replies into a fresh
call and reconcile the actual grant and mapping inventories. If the required
independent closer is unavailable, report BLOCKED at that boundary rather than
relaxing ownership or adding a production bypass.

Global Authority, Grant Lifecycle, Session Authority and Doorbell B remain
PARTIAL; production Gate C remains BLOCKED. Historical N3M/D3 and changed-service
Clippy failures are not resolved by these results. Strict Clippy, Miri, broad
D1/D2/D3 and network/UI replay are NOT_RUN in this milestone. There are no
FastLane performance samples. Native allocator-failure injection is NOT_RUN;
fallible reservation propagation is source-audited, with capacity/exhaustion
rollback exercised separately.

Reproduce the scoped concurrent method proof with fresh attempt directories:

```sh
python3 tools/fastlane_native_boundary_validate.py --section checks --reply-race --evidence-root target/fastlane-reply-race-evidence
python3 tools/fastlane_native_boundary_validate.py --section build --reply-race --evidence-root target/fastlane-reply-race-evidence
python3 tools/fastlane_native_boundary_validate.py --section native --reply-race --cpus 2 4 12 --image-manifest <fresh-build>/validation-summary.json --evidence-root target/fastlane-reply-race-evidence
```
