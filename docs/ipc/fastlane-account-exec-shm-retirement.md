# Authenticated Account exec and per-object SHM retirement

2026-10-31. **PARTIAL.** Real privileged Account exec and physical reclamation of
its selected request frame are demonstrated on 2, 4 and 12 CPUs. The complete
exit/cancellation/stale-instruction memory-safety matrix is not demonstrated.
Production FastLane, Restricted SHM and public Doorbell remain disabled.
No commit or push was performed.

Evidence: `target/fastlane-account-exec-evidence/validation-summary.json`.
`per-object/index.json` links nine object traces to their original serial hashes.
The preceding [authenticated lifecycle audit](fastlane-authenticated-lifecycle-resource-closure.md)
remains the historical baseline; its failures and independent gate classifications
are not superseded by these scoped passes.

## Actual authenticated scenario

The separate `session-account-exec-test` service feature uses the existing H4/N6
native login and controller. Root authenticates through the real TTY, session
service and Account password verification. The original authorized requester
sends AUTHORIZE for GroupCreate, target 61006. Normal session validation and the
real broker commit occur before the test hook. There is one committed operation
grant, receipt and pending delivery; there has been no consume or Account database
mutation. This is an outstanding authenticated authorization operation, not a
completed group creation.

Account defers that original AUTHORIZE with the ordinary syscall and rendezvous
with the prestarted controller. It first actually attempts
`exec("/missing-account-exec")`. The failure preserves the privileged image,
old VM and live deferred completion. It then actually executes
`/bin/session-racer account-exec ...` through the native exec syscall. Private
argv carries the inherited endpoint, controller SEND token, old completion,
committed grant and exact original session/epoch/revision/scope. These values are
not printed. The runner requires both actual syscall logs inside the correlated
snapshot windows; method calls or image-change snapshots alone cannot pass.

The replacement preserves PID, process incarnation, UID/GID and process-owned
endpoint authority, and has a distinct VM generation. The image-specific Account
role is zero. Sessions, receipts, operation grants and pending deliveries are
retired. Old incoming completion authority is withdrawn; the original caller
receives exactly one PeerClosed for its exact call generation. The result is
consumed and the caller's pending/outcome/reply inventory becomes zero.

The replacement attempts the old completion twice, both before receiving a fresh
request and while each of two new requests is live. Liveness is false and every
completion returns InvalidArgument. With the actual original grant and full
scope/revision, real consume and mint syscalls both return Denied. It cannot
adopt the original pending operation. A fresh controller call succeeds through
the inherited endpoint, and the original caller's fresh request with the old
session is denied with SessionChanged. This preserves the existing distinction
between process-owned endpoint authority and image-specific broker authority.

The controller independently rejects SEND-only/wrong-process endpoint management.
The replacement then exits normally. An ordinary authorized sunlightctl restart
restores Account; the driver performs real login again and repeats the whole
cycle. Each topology completes three cycles in the same guest with distinct
sessions and Account incarnations. The controller, replacement, original caller
and caller's native borrower thread are actually reaped before host acceptance.
The borrower belongs to the caller VM; Account has no borrower that could violate
exec's quiescence restrictions. No PID allocation, endpoint ownership, affinity,
active-CPU mask or exec permission is bypassed.

Endpoint IDs are never-reused endpoint lifetime identities in the current
implementation. There is no separate reusable endpoint-slot generation. They
are reported together with the original caller's independent call generation.
Numeric PID reuse is not introduced: allocation still uses checked
max-retained-PID plus one.

## Observation boundary and physical ownership

The Account operation grant and the ordinary bearer SHM request object are
different capabilities. The test correlates their actual pending delivery,
caller, issuer and request mapping. It does not turn the Account grant into a
recipient-bound mapping grant or validate Restricted SHM policy.

The kernel's new `authenticated_exec_test` feature includes the existing H4
observations. A bounded atomic observer selects at most three objects per boot
under the existing scheduler/broker lock order. Boot-local IDs 1..3 identify the
actual selected grant and its associated request object without exposing raw
capability tokens. Each record carries the relevant physical frame, VM, mapping
VA/owner/incarnation, endpoint/call generation and monotone observation sequence.
The full serial supplies the actual syscall and reaping evidence. IDs are scoped
by boot, object and original serial hash; frame addresses alone are not object
generations.

The selected request is one 4 KiB frame with two mapping references: the original
caller and Account. A native caller thread shares the caller VM and touches only
the unused final byte of the still-valid page. After PeerClosed it touches that
byte again and publishes a Release handshake; the parent waits with Acquire
before unmapping. The worker makes no further pointer access. Its bounded spin
provides an ordinary active borrower during the real unmap. It does not inject
an IPI, forge an active-CPU bit or simulate an acknowledgement.

Hooks observe the existing operations in this order:

1. Removal of the actual committed Account grant and scoped broker inventory.
2. SHM unmap request after existing PTE/ledger preflight.
3. Actual PTE removal before the existing synchronous TLB invalidation.
4. Real shootdown sequence and target mask, followed by actual ACK completion.
5. SHM unmap completion, reference decrement and retention or transfer to PMM.
6. Actual PMM bitmap clear, Free ownership assignment and accounting update.

The remote interrupt handler records only shadow sequence/flush atomics before
the ordinary mailbox acknowledgement. The initiator emits a witness only after
its existing Acquire acknowledgement wait completes. Thus `produced=request`
and `flushed=1` come from the independent real handler, not from a target mask or
an initiator-side assumption. There is no ISR serial logging or added lock. The
sequence numbers order observations and real acknowledgement boundaries; they
are not cross-vCPU timestamps or latency measurements.

All policy, mapping, reference-count, quiescence, mailbox and allocator decisions
remain independent of telemetry. Hooks are absent without the test feature; the
observer refuses non-debug kernel builds. Kernel observation code contains no
new unsafe block. Instrumentation uses fixed atomics and the already-held locks;
there is no new global hot-path lock or unbounded observer allocation.

## Per-object native results

Final source build: `build-20261031T125053.166247Z/`.
Final native attempts: `native-20261031T125115.241029Z/` (2 CPUs) and
`native-20261031T125333.152576Z/` (4/12 CPUs).
Each row below is an independently assessed actual native cycle.

| CPUs / cycle | Account PID/incarnation | Old -> new VM | Caller PID/incarnation/VM | Endpoint / call | Frame address | Remote flushed ACKs | PMM free sequence |
|---|---|---|---|---|---|---|---|
| 2 / 1 | 18/30 | 47 -> 90 | 39/51/87 | 14 / 4 | 531341312 | 1 | 25 |
| 2 / 2 | 43/55 | 94 -> 105 | 48/60/102 | 37 / 4 | 588333056 | 0 | 56 |
| 2 / 3 | 52/64 | 109 -> 120 | 57/69/117 | 44 / 4 | 645312512 | 1 | 88 |
| 4 / 1 | 24/36 | 59 -> 90 | 39/51/87 | 20 / 4 | 531406848 | 1 | 25 |
| 4 / 2 | 43/55 | 94 -> 105 | 48/60/102 | 37 / 4 | 588398592 | 1 | 57 |
| 4 / 3 | 52/64 | 109 -> 120 | 57/69/117 | 44 / 4 | 645378048 | 1 | 89 |
| 12 / 1 | 24/36 | 59 -> 90 | 39/51/87 | 21 / 4 | 531374080 | 1 | 25 |
| 12 / 2 | 43/55 | 94 -> 105 | 48/60/102 | 37 / 4 | 588365824 | 1 | 57 |
| 12 / 3 | 52/64 | 109 -> 120 | 57/69/117 | 44 / 4 | 645345280 | 1 | 89 |

Sessions are 1/2/3 respectively in each boot, each at epoch 1. Correlation IDs
are 1/2/3 within each boot. There are nine logical grant retirements and nine
actual final frame frees. Each selected lifetime has one terminal and one final
ownership transfer; the traces reject missing, duplicate or reordered records.

For example, 2 CPUs, cycle 1, grant/object 1:

| Sequence | Observed event |
|---|---|
| 3..5 | Actual committed grant, frame 531341312, both original mappings, views=2 |
| 8 | Logical grant retirement; sessions/receipts/grants/pending deliveries=0 |
| 9..14 | Account PTE cleared, invalidation complete, backing retained with views=1 |
| 15..16 | Original PeerClosed(4); replacement has same incarnation and new VM, auth=0 |
| 17 | Independent caller VM borrower has touched the still-valid mapping |
| 18..20 | Caller unmap requested, PTE cleared; real TLB request 163, target mask 2 |
| 21 | CPU 1 witness: produced=163, flushed=1; initiator has observed its real ACK |
| 22..23 | Real TLB completion, then SHM unmap completion |
| 24 | Last reference removed, views=0, ownership transferred to PMM |
| 25 | Actual physical frame free, ownership class Free=0 |
| 27,30..31 | Two fresh inherited-endpoint calls; old completion and broker operations denied |

Account's mapping removal has zero remote targets in these runs because its old
VM has no remote borrower. The caller's final unmap has eight observed remote
flush/ACKs across nine cycles. One 2-CPU cycle had a legitimate zero-target
release. That cycle passes its zero-target release checks but is **BLOCKED** for
a forced remote-ACK interleaving. The all-cycles remote assessor therefore stays
BLOCKED at 2 CPUs, and PASS at 4/12 CPUs. At every topology the selected trace
actually uses CPUs 0 and 1; 12 configured CPUs does not mean 12 participants in
this object operation. The separate legacy SHM regression exercises 1/3/11 remote
acknowledgements on its own objects and is not substituted for this object's
trace.

## Repetition and drift limits

At each of three complete cycle boundaries in each guest, the selected grant,
receipt, pending delivery, mapping references and backing-frame ownership are
zero. Account exec retains the caller view (views=1); only its later checked
release transfers the frame (views=0). This demonstrates scoped reclamation and
absence of accumulating live ownership for these nine selected lifetimes.

The broker retains container capacity: post-retirement session metadata is 704
bytes and grant metadata capacity is 576 bytes after each cycle. Identity bindings
return to zero. Anti-reuse token history legitimately grows as services restart
and sessions/capabilities are issued; final counts are 186/183/185 at 2/4/12 CPUs.
Those counts and capacities are not live grants. They do not establish zero
global heap drift. Service allocator inventories and a common-epoch whole-system
frame census remain **NOT_RUN**.

For the selected backing frame, the actual PMM Free ownership and free bitmap
update are observed, rather than an allocator-cache retention claim. The PMM's
forward scan can allocate later request objects at higher addresses; the three
different frame addresses do not establish lost ownership. Subsequent reuse of
these same selected physical frames is **NOT_RUN**: no first-reuse claim was
captured before the observer moved to the next object. The optional claim hook
covers the single-frame claim path, not every contiguous allocation path; its
silence is not a global no-reuse census. No allocator was modified to force reuse.

## Acceptance classification

PASS below means the specific final-source native scenario above. It does not
generalize to unexecuted cancellation, exit or hostile mapping interleavings.

| Acceptance item | Classification | Evidence or limit |
|---|---|---|
| Real authenticated Account, authorized original caller, committed outstanding grant | PASS | Nine postcommit AUTHORIZE operations following real authentication |
| Actual privileged Account exec; failed exec preserves old authority | PASS | Nine real failed/successful syscall pairs and exact snapshot windows |
| Exact incarnation, old/new VM, endpoint lifetime and call generation | PASS | Per-object records and fresh-call receipts |
| Image privileges withdrawn; process-scoped endpoint/identity preserved | PASS | auth=1 -> 0, real mint/consume Denied, inherited endpoint serves new calls |
| No replacement adoption of old pending operation | PASS | Old completion invalid before/during fresh calls, old grant denied |
| Exactly one original terminal, late/duplicate rejection | PASS | PeerClosed once per exact original call; consumed state clear |
| No leaked receipt or pending-delivery metadata | PASS | Actual retirement and repeated scoped zero inventory |
| Opt-in grant/object/frame/mapping/TLB/PMM correlation | PASS | Three bounded boot-local objects, nine linked JSONL traces |
| No selected frame release while a recorded valid mapping remains | PASS | Retain at views=1, final transfer at views=0 after both unmaps |
| Selected final frame free follows every required remote ACK | PASS | Eight real handler flush witnesses; one correctly zero-target release |
| Forced remote interleaving in every selected cycle | BLOCKED | 2-CPU cycle 2 has no remote target; 4/12 CPU cycles all observed |
| Account exec cleanup and caller exit after checked SHM release | PASS | Real exec unmap, checked caller release, actual participant reaps |
| Pending caller exit with committed grant still outstanding | NOT_RUN | Exit here occurs after result consumption and checked release |
| Explicit/deadline cancellation of this committed authenticated operation | NOT_RUN | Separate native scenario still required |
| Exactly-once selected grant retirement and physical frame free | PASS | One logical retirement, final transfer and PMM free per selected lifetime |
| Stale completion/grant/bearer remap and duplicate free rejected | PASS | Real syscalls and fixture assertions |
| Hostile stale virtual-address read/write after unmap | NOT_RUN | No faulting instruction probe; PTE removal is not substituted for it |
| No persistent selected-object live-resource drift across complete cycles | PASS | Three same-guest cycles/topology, nine reconciled lifetimes |
| Same selected physical frame later safely reused | NOT_RUN | Actual PMM free observed; later reuse not observed |
| Common-epoch global allocator/service-heap drift | NOT_RUN | Scoped broker capacity and ownership are insufficient |
| Different-incarnation adoption attempt | NOT_RUN | Replacement here retains its incarnation; different-process test absent |
| Actual numeric PID reuse | BLOCKED | Current allocator does not reuse numeric PIDs |
| Production instrumentation exclusion and FastLane disabled | PASS | Ordinary image feature/marker/embedded-service audit and native boot gates |
| Strict kernel and service Clippy | FAIL | Existing dependency diagnostics stop strict checking |
| Miri availability | BLOCKED | Active nightly lacks cargo-miri |
| Miri execution | NOT_RUN | Availability prerequisite blocked |

## Validation and provenance

Entry was clean at `1e6a22e2401cc28c17cd61f8e715742505cc4587`; that HEAD is
unchanged. `entry/` preserves the entry source archive and worktree state.
The final source map contains 1,316 files, including the four new Rust/Python
module/test files. Its canonical JSON SHA-256 is
`6aa6c3895f647e9252adefd9a4a92ae19efc8abade55633a0723cfdec7e1db51`.
`final-source.json`, `final-source.tar.gz`, `final-diff.patch` and
`final-status.txt` preserve the inputs and complete local changes. Markdown
reports are separately hashed and included in the final archive; the established
build guards hash executable/configuration sources rather than Markdown.

Final authenticated ELF SHA-256:
`828c055873174d56026e9564420cc348979ce49d802a21d0f6c236c1085bc1aa`.
Final authenticated ISO SHA-256:
`5c45c334cd5c675935922e8ea45437b76ec1513ca43f8bc9166d4c58ed5ced01`.
The actual ISO-extracted kernel matches the archived ELF. All eight embedded
executables are archived and byte-identity checked. Each final native guest has
a separate host receipt covering its entire original serial, all three cycles,
source/image identity, security assertions and cleanup. Parser-only fixtures
are excluded from these execution counts. Final serials are reassessed against
the unchanged current source during evidence finalization.

| Final native validation | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Actual authenticated Account exec cycles | PASS: 3 | PASS: 3 | PASS: 3 |
| Correlated logical retirements and actual PMM frees | PASS: 3 | PASS: 3 | PASS: 3 |
| Actual selected-frame remote flush/ACK witnesses | PASS: 2 | PASS: 3 | PASS: 3 |
| Forced selected-object remote interleaving in every cycle | BLOCKED: one zero target | PASS | PASS |
| Scoped complete-cycle live-resource reconciliation | PASS | PASS | PASS |
| Prior concurrent IPC-method races and nine deferred cases | PASS | PASS | PASS |
| Conventional ring-3 1,000-call IPC and existing VM/SHM gates | PASS | PASS | PASS |
| Independent remote SHM release acknowledgements | PASS: 1 | PASS: 3 | PASS: 11 |
| R2D remote wake and real lifetime rounds | PASS | PASS | PASS |
| Doorbell transitions and backing restoration | PASS | PASS | PASS |
| Scheduler lifecycle, deferred reap and external exit race | PASS | PASS | PASS |
| Existing authority, lifecycle and ordinary real exec replacements | PASS | PASS | PASS |
| Prior authenticated reverse-reply/peer exec/peer exit/Account reap | PASS | PASS | PASS |
| Ordinary production startup, IPC, VM and SHM | PASS | PASS | PASS |
| Ordinary image test-marker exclusion and embedded service identity | PASS | PASS | PASS |

The IPC regression is in `regression-ipc/native-20261031T125739.448296Z/`.
Independent R2D/Doorbell/scheduler images and native results are in
`regressions/20261031T125739.434903Z/`, including ISO-extracted ELF and embedded
service checks. R2D restores each baseline over eight lifetimes; Doorbell
observes 42 transitions, 43 creations/retirements, zero live ownership inventory
and matching before/after frame totals 72,677 / 72,165 / 70,117. Scheduler's
external exit race and reused task slots are independent of numeric PID reuse.

Authority replay is in `compatibility/authority-20261031T130237.631974Z/`.
It also passes at 1 CPU, with eight actual replacement-entry/failed-exec receipts
per authority run, then enters the original program. The separate no-entropy
control refuses bootstrap authority. `image-audit-v2.json` verifies the extracted
ELF, seven actual embedded services and exact replacement-entry counts. The
earlier `image-audit.json` used a nonexistent marker for that count field; its
unchanged image hashes remain valid, while the zero count is not acceptance.

The preceding authenticated-reply fixture passes at
`regression-authenticated-reply/native-20261031T130612.776637Z/`. Its original
base classifier remains PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT; the independent
userspace-reaper and authenticated-replies assessments pass all 15 reverse calls,
two actual peer execs and real pending-reply peer exit, with host ACKs. That
fixture's own untested privileged-exec/reclamation fields are not rewritten.

Ordinary build: `normal-20261031T130613.958583Z/`. Production executions and
ISO-extracted ELF checks are in `continuation-20261031T130646.064236Z/`. Its
empty kernel feature set and actual ELF exclude all Account-exec/reply/session
test markers and the allocation-failure-injection marker. The latter is required
and passes in the separate instrumented IPC/VM image, and is explicitly excluded
from the ordinary marker list because `mm2a_test_injection` is absent. No timeout
was enlarged and no failure injection was enabled in the ordinary image.

Host checks pass 140 kernel tests, 266 service/library tests, three ring
documentation tests, 19 manager runtime tests, focused scheduler checks and
115 FastLane parser tests, including nine new parser mutation test methods.
New-module rustfmt and final diff whitespace pass. Strict kernel and service
Clippy failures and the unavailable Miri prerequisite remain in
`extra-checks-20261031T125622.188289Z/`. Broad combined D1/D2/D3, network, media,
desktop and audio replays and whole-workspace formatting are NOT_RUN here;
their earlier failures are not reclassified.

Every build/native attempt retains its commands, original status, source map,
logs and artifacts. The root summary indexes preliminary passes as older source
snapshots, not final acceptance. Preserved failures include the private-predicate
compile error (`build-20261031T123052.853749Z`), an observation format-argument
error (`build-20261031T124358.574280Z`), and an exact-text tooling replacement
that applied no edits. Preliminary zero-target native successes remain BLOCKED
for remote-interleaving proof. The first previous-reply replay failed before
guest boot because sandbox policy denied the local QMP socket; its empty serial
and QEMU diagnostic remain in `native-20261031T130519.241376Z/`. A retry with an
incorrect archived manifest path is NOT_RUN in `native-20261031T130601.397951Z/`.
The later reply replay uses the same archived image outside that socket
restriction, in a fresh attempt directory. Neither failed attempt is promoted.

Reproduce with fresh attempt directories and the default bounded timeouts:

```sh
python3 tools/fastlane_f11b0h4_validate.py --section build --cases N6 --account-exec --account-exec-cycles 3 --evidence-root target/fastlane-account-exec-evidence
python3 tools/fastlane_f11b0h4_validate.py --section native --cases N6 --account-exec --account-exec-cycles 3 --cpus 2 --image-manifest <fresh-build>/validation-summary.json --evidence-root target/fastlane-account-exec-evidence
```

Expand to 4/12 CPUs only after the smallest native run passes. Native runners
need permission to bind their local QMP socket. `drivers/` retains the auxiliary
regression, image-audit, production, host-check and finalization scripts; the root
summary links their raw manifests and distinguishes host controls from execution.

## Security and unsafe-code audit

No new syscall, privilege, capability policy, production transport or IPC
linearization rule is introduced. The only non-cfg production edits store an
existing reference-decrement result in a local before its unchanged branch, and
make an existing reply-liveness predicate crate-visible for observation. The
normal ownership and fail-stop SHM preflight remain intact. No frame release is
moved before PTE removal, real remote acknowledgements or final reference
decrement. There is no new global lock and no telemetry-controlled allocation,
retirement, completion or acknowledgement.

Kernel instrumentation has zero new unsafe blocks. The test userspace worker's
two volatile reads require its still-live request mapping; its Release/Acquire
handshake ends all pointer accesses before unmap. Native spawning uses the
existing unsafe thread API with that lifetime. Existing VM-bound
NativeThreadReaped (syscall 156), rather than Finished alone, is required before
ordinary stack/TLS unmapping. Its raw syscall invocation uses the existing
contract. The address-space PTE write and PMM unsafe ownership operations retain
their existing locks and safety preconditions; added hooks execute after those
mutations. No unsafe implementation or aliasing rule is relaxed.

Private capabilities and completion handles cross test IPC/argv only. Trace
output contains boot-local IDs, mapping/frame addresses and the participating
process identities, not payloads, passwords, credentials, secret seeds or raw
capability tokens. Existing session/reaper logs remain scoped to the test image.
Production exclusion is checked against the actual ordinary ELF and archived
embedded service contents, rather than Cargo arguments alone.

Strict kernel/service Clippy failed in existing dependency sources; no unrelated
lint fixes are included. Those failures remain visible and do not validate the
changed code under strict Clippy. Miri is not installed for the active nightly;
its availability prerequisite is BLOCKED and execution is NOT_RUN. Host parser
mutation controls are tooling checks only, never synthetic native measurements.

## Remaining work

The next smallest executable milestone is one real requester deadline or explicit
cancellation after Account's authenticated grant commit, while Account still
retains its actual SHM view. Correlate the original terminal and pending delivery
retirement with the caller's unmap/ACK, prove backing retention for Account's
remaining view, then complete Account's ordinary cleanup and final PMM release.
Keep pending-caller exit as a separate subsequent scenario. Add hostile stale-VA
instruction/fault evidence and later same-frame reuse only through existing safe
allocation/lifecycle paths. No numeric PID reuse, forged endpoint owner or
quiescence bypass is authorized to make these cases execute.

Global Authority, Grant Lifecycle, Session Authority and Doorbell Gate B retain
their independent partial classifications; F1.1B-0 and production Gate C remain
blocked. Scoped passes here do not cancel historical failed combined replays.
