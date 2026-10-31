# Authenticated Account cancellation and original-caller exit

Continuation: [authenticated completion/cancellation overlap](fastlane-authenticated-completion-cancellation-race.md)
adds a separate two-owner syscall contention fixture. This report retains the
original ordered-operation evidence and its NOT_RUN overlap classification.

2026-10-31. **PARTIAL.** Production FastLane, Restricted SHM and public
Doorbell remain disabled. No commit or push is authorized or performed.

Evidence is retained under `target/fastlane-cancel-exit-evidence/`. The final
`validation-summary.json` indexes every build/native attempt, the exact source
map, image identity, original serial hashes and independent host receipts.
The preceding [Account exec and SHM retirement milestone](fastlane-account-exec-shm-retirement.md)
is a separate regression baseline; its untested properties are not promoted.

## Scope and production repair

An authenticated root shell starts an ordinary supervisor. Its spawned original
caller obtains its own real Account/session snapshot and sends AUTHORIZE for
GroupCreate 61006. Account performs the existing password, policy, session,
recipient identity and pending-call checks and actually commits an operation
grant before entering the test fixture. No database mutation is published.

The first native attempt reaching cancellation observed grant=0 and pending
delivery=0 but receipt=1 after deadline settlement. The receipt disappeared only
when the caller subsequently exited. Production settlement now removes a retry
receipt only when its PID, incarnation and call generation match the settled
call and the associated grant is absent. Delivered grants and their receipts
survive a late cancellation. Other identities/generations are untouched. This
runs under the existing lifecycle/broker serialization, with bounded existing
session/receipt inventories; it adds no lock and changes no mapping, TLB or
allocator decision. The old failed-delivery host assertion is strengthened from
one retained receipt to zero. Exact mismatch and delivered-grant host controls
cover the new behavior.

`session-account-cancel-test` and `authenticated_cancel_test` are independent,
opt-in service/kernel features. The latter includes the established debug-only
per-object observer. Normal builds contain neither feature nor their markers.
The fixture uses ordinary IPC, endpoint, SHM, thread, signal, wait and service
restart syscalls. A read-only debug-image query waits for the exact selected
caller PID/incarnation, parent and Reaped state; it neither drives nor bypasses
cleanup. No production execution depends on a rendezvous or test hook.

## Exercised boundaries

| Mode | Actual boundary and terminal behavior |
|---|---|
| 1 | Original ordinary reply target retained before publication; real deadline wins; ordinary late reply is safely dropped |
| 2 | Original reply deferred with committed grant pending delivery; real deadline wins; late and duplicate deferred completions return InvalidArgument |
| 3 | Account commits the actual grant-bearing reply; original caller invokes IpcCancel before collection; the exact ReplyDelivered result and grant remain collectable |
| 4 | Supervisor sends authorized SIGTERM to its actual spawned original caller while grant, deferred completion and both request mappings remain outstanding; caller death/reap wins; late completion is rejected |
| 5 | Account commits ReplyDelivered; original caller exits without collecting its reply or releasing its SHM view; delivered grant and receipt retire through actual process lifecycle |
| 6 | Deadline wins before Account destroys its own endpoint; no second terminal; late completion rejected |
| 7 | Account destroys its own endpoint while original call remains pending; PeerClosed wins before the deadline; no later terminal publication |

Private IPC carries live completion/grant capabilities without printing them.
A different process attempts the actual live completion handle before allocating
any local deferred handle: handles are process-local and numerical values can
alias another process's own handle. The supervisor also attempts actual grant
consumption without the Account role and SEND-only/wrong-owner endpoint
management; both fail through existing authority checks.

After the original terminal or death, the still-privileged Account attempts
mint and consume for the original caller; both return Denied. In modes with a
remaining Account endpoint, the supervisor sends a fresh live call through that
same endpoint. The old completion remains invalid and actual old-grant adoption
for this different caller returns CapabilityRejected. Kernel observations bind
PID, incarnation, VM, endpoint lifetime and independent call generation. Endpoint
IDs are never-reused lifetime identities, not reusable slot generations.

The original caller also owns a real private endpoint. After its actual reap,
the supervisor's SEND capability to that endpoint is rejected. This exercises
owner-exit endpoint retirement as well as Account endpoint closure.

## Original caller and physical ownership

The Account operation grant and request SHM capability are distinct objects.
The observer correlates their actual original pending delivery and two mapping
references; it does not validate recipient-bound Restricted SHM policy.

A native thread in the original caller VM touches only the unused final byte
of the live request page. For explicit release, the established Release/Acquire
handshake finishes all pointer access before unmapping. For caller exit, the
actual lifecycle terminates that borrower; the assessor requires its actual
reap before the owner's reap-ready observation. The owner must have no active
CPU, owning core or queued core before its mapping is reclaimed. The trace saves
the retiring VM generation before address-space reclamation resets the live VM
identity; a zeroed post-reclaim identity is not substituted for the old VM.

A preserved 12-CPU attempt exposed a fixture ordering error: native waitpid
accepts Finished as well as Reaped, so exit status alone did not establish
resource cleanup. Account attempted a stale-map rejection before the caller
view was reaped; that map was legitimately still possible and the assertion
failed. The fixture now waits on the read-only selected-caller query before
releasing Account. Independent actual reaper, PTE, TLB and physical-free records
remain mandatory; the query is coordination, never reclamation evidence.

Each selected lifetime requires the real mapping/PTE removal, TLB request,
complete target mask and acknowledgements, reference retention at views=1,
final transfer at views=0, and actual PMM bitmap/Free ownership update. Remote
witnesses originate in the real mailbox handler before its normal acknowledgement;
the initiator reports them only after its existing Acquire acknowledgement wait.
No IPI, active-CPU bit, acknowledgement or allocator reuse is injected. There is
no ISR logging or new global hot-path lock. A zero inventory alone cannot pass
physical reclamation. Trace sequence numbers order observations; they are not
cross-CPU timestamps or latency measurements.

On process exit, quiescence can legitimately give zero remote targets. That is
separate from checked unmap with a live remote borrower. The final matrix and
per-object index report actual targets, handler acknowledgements and flushes;
zero-target operations never count as remote execution. Later reuse of the same
selected physical frame, hostile stale-address instructions, and a common-epoch
whole-system allocator census remain NOT_RUN.

## Limits of race closure

Modes 1/2 and 3 establish cancellation-wins and completion-wins outcomes at their
respective controlled boundaries. Modes 4/5 establish actual original-caller
exit-wins and completion-wins lifetimes. Modes 6/7 establish both deadline/close
winner orders. These are ordered real operations. Overlap of the actual reply
publication syscall with cancellation or endpoint closure is **NOT_RUN**; no
sequential winner result is reported as a forced concurrent invocation.

The Account endpoint owner and completion authority are process-local. The
foreign-owner denials do not authorize a second process to close Account's
endpoint or complete its reply. A real overlap fixture must preserve that
contract. Pre-publication self-IpcCancel is also NOT_FORCED here: an attempted
fixture resumed the blocked caller at its actual deadline, so that failed
attempt remains a deadline result, not explicit-cancel proof. Explicit
IpcCancel after committed reply is a separate tested boundary.

Exactly one call terminal is required for each surviving caller. In exit-wins
mode 4, there is one actual process termination and no published IPC terminal
into the dead caller; an invented cancellation response is not required or
counted. Late/duplicate completions must remain rejected.

## Validation and remaining lint debt

The machine-readable summary is authoritative for the completed CPU matrix and
per-object totals. Each accepted native guest must complete three authenticated
cycles under the default bounded timeout and pass current source/image guards,
actual participant reaping, physical-release assessment and host acknowledgement.
Parser controls are excluded from native counts.

Strict Cargo Clippy remains FAIL; dependency/build-script diagnostics prevent
it from reaching all changed primary crates. The previous diagnostic source
files were compared with HEAD and are unchanged. A supplemental strict primary
crate comparison uses the actual compiler invocation and built dependencies,
with an isolated archive of this task's entry source. Missing binary assets in
an early baseline made that comparison invalid; those attempts are retained.
With the assets supplied unchanged, entry/current diagnostics match: 133 kernel,
five Account daemon and one probe. Two new fixture warnings (unnecessary unwrap
and excess function arguments) were fixed without lint suppression. This
supplement does not turn the failed strict Cargo command into PASS. Final exact
commands, diagnostics and source identity are retained with the comparison.

Final host checks passed 141 kernel tests, 62 IPC tests, 27 session-service
tests, 18 UAC tests and 123 FastLane parser tests. The exact source map was
unchanged during these checks. Strict dependency/build-script diagnostics are:

| Existing crate | Errors under `-D warnings` |
|---|---:|
| sunlight-virtio | 51 |
| sunlight-fat | 7 |
| sunlight-compat-linux | 5 |
| sunlight-tui build script | 3 |
| sunlight-sensors | 2 |
| sunlight-smbios | 2 |
| sunlight-ipc | 1 |
| sunlight-kernel build script | 1 |

The service strict invocation independently stops on the same FAT and IPC
failures. The pre-existing duplicate binary-target manifest warning also remains.
No lint level or toolchain was changed. Compiler and Cargo execution worked;
these are source lint failures, distinct from the QMP socket sandbox failures
preserved in native attempts.

Miri availability is UNAVAILABLE on the installed toolchain; execution is
NOT_RUN. Host/parser, native Account exec/retirement, authenticated reply,
IPC/VM/SHM and ordinary-image regressions are recorded independently. Broad
combined D1/D2/D3, desktop, audio, networking and whole-system heap/reuse probes
are outside this execution. Their prior failures are not superseded.

## Final native results

All rows use the same archived cancellation image and final executable source
map. Each cell represents three complete authenticated operations in a fresh
QEMU guest with the stated CPU count; isolated parser controls are excluded.

| Mode | 2 CPUs | 4 CPUs | 12 CPUs | Remote ACK/flush witnesses, by topology |
|---|---|---|---|---|
| 1: before publication, deadline wins | 3/3 PASS | 3/3 PASS | 3/3 PASS | 3 / 3 / 3 |
| 2: pending deferred reply, deadline wins | 3/3 PASS | 3/3 PASS | 3/3 PASS | 3 / 3 / 3 |
| 3: cancel after commitment, before collection | 3/3 PASS | 3/3 PASS | 3/3 PASS | 2 / 3 / 3 |
| 4: actual original-caller death wins | 3/3 PASS | 3/3 PASS | 3/3 PASS | 0 / 0 / 0 |
| 5: commitment wins, actual uncollected caller exit | 3/3 PASS | 3/3 PASS | 3/3 PASS | 0 / 0 / 0 |
| 6: deadline then owner endpoint closure | 3/3 PASS | 3/3 PASS | 3/3 PASS | 3 / 3 / 3 |
| 7: owner endpoint closure wins | 3/3 PASS | 3/3 PASS | 3/3 PASS | 3 / 3 / 3 |

The 21 accepted guests contain 63 selected object lifetimes, each with exactly
one actual physical frame-release event. Their 126 view removals include 82
zero-remote-target and 44 nonzero-remote-target invalidations. All 44 remote
acknowledgements have independently produced mailbox-handler witnesses with
`flushed=1`. Both zero and nonzero target masks occur at every configured CPU
count. One 2-CPU mode-3 cycle has no remote target; all exit-mode retirements
have zero targets after verified quiescence. Configured CPU count does not imply
participation by every vCPU. No physical-release total is inferred from inventories.

Independent final-source regressions also passed on 2/4/12 CPUs: nine prior
Account exec/physical-retirement cycles with nine remote ACK/flush witnesses;
15 authenticated deferred-reply rounds and two real peer execs per topology;
the existing IPC/VM/SHM and concurrent kernel-method reply-race gate; and ordinary
production boot/security/SHM gates. Kernel-method races are separate evidence
and do not establish overlapping authenticated Account calls. Older independent
closer/PID-reuse and broader combined-gate limitations remain intact.

The final assessor re-reads the actual serial files and host receipts, verifies
their hashes and source/image identities, and emits 63 per-object JSONL traces
plus an index under `target/fastlane-cancel-exit-evidence/per-object/`. Removing
physical, TLB, reaper or terminal/grant records from each accepted guest yields
84 rejected parser controls; those mutations are never counted as native runs.

Final executable source map: 1,319 files, canonical SHA-256
`024a2e3cf1a7f465debb495a94da5a503daacb0f60fc63a60ee353240298e574`.
Git HEAD remains `1e6a22e2401cc28c17cd61f8e715742505cc4587`; the dirty entry
matches the preceding milestone's final source map, and its audit hash remains
unchanged. The final summary records full ISO, extracted ELF, embedded-service,
archive and documentation hashes, every attempt, and remaining property gaps.

## Preserved unsuccessful attempts

The summary indexes all timestamped manifests and their hashes, including
attempts from earlier source snapshots. No failed attempt is rewritten as PASS.
The retained failures include the initial receipt leak; a missing shared-worker
feature gate; a foreign process-local handle alias assumption; a wrong fresh-call
badge; a driver exit-marker mismatch; repeated-login fixtures missing ordinary
session logout; the 12-CPU Finished-versus-Reaped ordering error; and sandbox
QMP socket bind denials before guest execution. Parser mutation/control failures
and missing-asset strict-baseline attempts are preserved separately from native
acceptance. Later source/image success does not promote those attempts.

The current source is identified by the established executable-source/configuration
hash map, its canonical hash, a source archive, the unchanged Git HEAD and dirty
worktree status. ISO-extracted kernel and embedded-service bytes are checked
against their archived hashes. Documentation is hashed separately because
Markdown is outside the established executable source map. Commands, serial
hashes and independent host acknowledgements remain attached to every native
result. The entry snapshot and preceding milestone evidence are preserved.

## Next smallest executable security milestone

Force and observe overlapping invocation of real authenticated Account reply
publication and original-caller cancellation under the existing process/endpoint
authority model. Require independent entry/progress witnesses and both terminal
winner orders while retaining the exact selected grant/request frame trace.
Keep the production transport disabled and classify a missing overlap NOT_FORCED.
