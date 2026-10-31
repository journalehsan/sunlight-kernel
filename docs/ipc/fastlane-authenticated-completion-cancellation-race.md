# Authenticated Account completion/cancellation overlap

2026-10-31. **COMPLETE for this scoped milestone.** Production FastLane,
Restricted SHM and public Doorbell remain disabled. No commit or push.

Evidence: `target/fastlane-completion-race-evidence/validation-summary.json`.
The [cancellation/caller-exit milestone](fastlane-authenticated-cancellation-caller-exit.md)
remains the historical baseline. Its ordered winner observations do not become
concurrent evidence retrospectively.

## Root cause and implementation

The missing proof was execution overlap. The existing caller blocks in its own
pending IPC call, Account owns the process-local deferred completion, and no
foreign process can cancel that caller's call or adopt Account's completion
handle. Simply placing two ordered commands around the old rendezvous cannot
force both syscall invocations to execute together. No new production terminal
transition defect has been established in this milestone.

`authenticated_completion_race_test` is a separate opt-in, debug-only kernel
feature; `session-account-completion-race-test` is the corresponding service and
probe feature. Both depend on the earlier cancellation fixture. Ordinary images
exclude the coordination and marker strings. No new syscall, public capability,
global hot-path lock, unsafe kernel code, or production transport is introduced.

The actual root-authenticated caller submits AUTHORIZE for GroupCreate 61006.
Account performs the established password, policy, session and recipient checks,
commits the real grant, and defers its original reply. The private supervisor
checks live foreign completion/grant and endpoint-management denials before
releasing Account. Account alone may arm coordination, and only for its one
existing live deferred call matching the per-object observer's committed grant,
caller incarnation, endpoint lifetime and call generation.

Arming wakes that exact blocked caller through the existing scheduler wake path.
It leaves the pending call, its deadline, reply and grant state unchanged. This
is an explicit test-only scheduling action, not a production wake policy or a
read-only observation. The caller then invokes its ordinary self-only IpcCancel;
Account invokes its ordinary process-local deferred completion syscall.

Each actual syscall validates its actor under the existing scheduler lock,
records entry and releases the lock before rendezvous. Both entrants must be
present on different CPUs. The selected winner acquires the existing scheduler
lock; the loser independently performs a real failed `try_lock` while that winner
holds it. Only after that witness does the winner execute the original terminal
transition code. The loser subsequently executes the original syscall logic.
No test hook writes an outcome, settles a grant, removes a mapping, sends an IPI,
produces a TLB acknowledgement, or frees a frame.

All waits have a fixed iteration budget and a five-second guest-clock bound.
The iteration budget remains a bound when holding the scheduler lock prevents
the BSP clock from advancing. Missing entry, same-CPU execution, missing actual
contention, timeout, deadlock, assertion failure or driver failure cannot pass.
The normal native driver timeout is unchanged at 120 seconds per guest.

| Mode | Actual concurrent terminal contest | Further invocations |
|---|---|---|
| 8 | Completion wins while cancellation is active on another CPU | Later cancellation preserves the uncollected reply; duplicate completion rejected |
| 9 | Cancellation wins while Account completion is active on another CPU | Active and late completion rejected; repeated cancellation preserves the terminal |
| 10 | Both enter before an atomic test-coordination winner selection | Same independent lock contention, collection, late checks and retirement |

All three modes contend on the production terminal-state serialization lock.
The atomic selection in mode 10 chooses who first acquires that lock; it never
selects or publishes the IPC outcome itself. These are deterministic interleaving
fixtures, not uninstrumented scheduler fairness or latency measurements.

## Terminal outcomes and independent acknowledgement

The trace requires one exact `AE-TERMINAL` for the selected original call,
preceded by both syscall entries and the independent lock-contention witness.
Each actor has its own return and post-operation `RC-OUTCOME` snapshot, before
the caller collects its terminal. PID, incarnation, endpoint lifetime, original
call generation, CPU and mode must match the selected grant. Endpoint IDs are
never-reused lifetime identities; the IPC call generation is a separate value.

Cancellation wins with `ExplicitlyCancelled(generation)`. Account completion
returns `InvalidArgument` (4) after its original deferred entry has been
withdrawn. No reply payload survives. Completion wins with
`ReplyDelivered(generation)`, and cancellation cannot replace it. Cancellation
is idempotent by the existing ABI and returns zero in this losing case; it is a
rejected state change, not a newly invented cancellation error.

The original caller collects the expected terminal result. A delivered reply
must have the Account reply label and exactly one grant. It sends that result
through private IPC to the separate supervisor, which compares the actual
returned grant with Account's original committed grant without logging either
capability. Cancellation must return Cancelled (9) and carry no grant. The
supervisor acknowledges the result and waits for the actual caller reap.
Account remains alive with its request mapping until this completes.

Later cancellation snapshots must preserve the exact outcome, reply and grant
inventories. Account retries its old completion twice and checks liveness;
all must reject. After original-caller reaping, the still-privileged Account
must fail mint and consume for the old caller. A fresh call by the unrelated
supervisor uses the same live Account endpoint; old-grant adoption returns
CapabilityRejected and the old completion remains invalid. SEND-only and
foreign-owner endpoint management remain rejected. The original caller's own
endpoint must retire on its actual exit.

The assessor requires final zero grants, retry receipts, pending deliveries,
request mappings and deferred targets. It also requires actual borrower,
original caller and Account reaping. A separate host receipt binds the serial,
ELF, ISO, archived embedded services and current source hashes. Parser controls
are explicitly separate from these native results.

## Per-object SHM, TLB and physical retirement

The operation grant and request SHM capability are separate real objects. The
selected grant links the two original mapping owners, their incarnations and
VM generations to the actual request backing frame. No Restricted SHM policy
claim is made from the legacy request page.

The caller's existing native borrower touches the unused final byte of that
page on its shared VM. The established release/acquire handshake finishes all
pointer access before explicit unmap. Account retains the other view while
the caller collects, releases its own view and exits. Reaping is observed
independently of waitpid, which alone may report Finished before reclamation.

For each of the two original views, acceptance requires exactly one unmap
request, actual PTE removal, matching TLB request, every required mailbox-handler
acknowledgement, TLB completion, unmap completion and reference release. The
first release retains the frame at one view; only the final release transfers
ownership to PMM at zero views. Exactly one actual PMM bitmap/Free ownership
update must follow both TLB completions. Duplicate terminal, retirement, unmap,
acknowledgement or physical-free observations fail acceptance.

A nonzero target mask requires the complete exact acknowledgement set and
handler-produced request sequence. An acknowledgement reports whether the remote
handler actually flushed. Zero-target unmaps are counted separately and never
used as evidence of remote execution. Trace sequence numbers order observations;
no cross-CPU clock synchronization or performance claim is inferred.

`physical-objects.json` retains each selected operation, mapping ownership,
unmap/PTE/TLB/ack records, terminal winner, logical retirement, physical free and
final inventories, linked to its original serial log and host acknowledgement.
Later reuse of the same selected frame, hostile stale-address instructions and
a common-epoch whole-system allocator/service census remain NOT_RUN.

## Validation and security audit

The final matrix and exact image/source identities are recorded in the summary.

| Final-source regression | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Legacy cancellation/caller-exit (7 modes) | 7/7 PASS | 7/7 PASS | 7/7 PASS |
| Privileged Account exec and physical retirement | 3 cycles PASS | 3 cycles PASS | 3 cycles PASS |
| Account authority contention S1/S2 | 2/2 PASS | 2/2 PASS | 2/2 PASS |
| Authenticated reverse replies and real peer exec | 15 operations PASS | 15 operations PASS | 15 operations PASS |
| IPC/VM/SHM plus kernel-method reply race | PASS | PASS | PASS |
| Native Doorbell adapter | 42 rounds PASS | 42 rounds PASS | 42 rounds PASS |
| Native scheduler lifecycle gate | PASS | PASS | PASS |

The regression manifests retain the exact commands, original serial logs,
source/image hashes, scope classifications and host receipts where supported.
The reverse-reply base classifier remains
PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT; its independent authenticated reply
assessment passes, and is not relabelled as concurrent endpoint destruction.
The nine new race guests and 42 regression guests total 51 accepted native
guests on this source. Production services, kernel and ISO build with no test
features or fixture markers; the kernel extracted from the production ISO
matches its archived ELF. An additional ordinary release kernel check passes.

Final host validation passes 141 kernel tests, 241 service/library tests,
19 manager runtime tests, 147 tests with the Doorbell adapter enabled, focused
scheduler checks and 132 FastLane parser methods. Miri is unavailable because
cargo-miri is not installed; execution is NOT_RUN.


| Scenario | CPUs | Authenticated cycles | Winners | Remote ACK / flush | Zero-target unmaps |
|---|---:|---:|---|---:|---:|
| Forced completion | 2 | 3 PASS | completion, completion, completion | 3 / 3 | 3 |
| Forced completion | 4 | 3 PASS | completion, completion, completion | 3 / 3 | 3 |
| Forced completion | 12 | 3 PASS | completion, completion, completion | 3 / 3 | 3 |
| Forced cancellation | 2 | 3 PASS | cancellation, cancellation, cancellation | 2 / 2 | 4 |
| Forced cancellation | 4 | 3 PASS | cancellation, cancellation, cancellation | 3 / 3 | 3 |
| Forced cancellation | 12 | 3 PASS | cancellation, cancellation, cancellation | 3 / 3 | 3 |
| Concurrent selection | 2 | 3 PASS | completion, cancellation, cancellation | 2 / 2 | 4 |
| Concurrent selection | 4 | 3 PASS | completion, cancellation, cancellation | 3 / 3 | 3 |
| Concurrent selection | 12 | 3 PASS | cancellation, cancellation, cancellation | 3 / 3 | 3 |

All nine race guests passed, with three complete authenticated cycles per guest:
27 selected grants, 54 verified mapping removals, 25 real remote acknowledgements
and flushes, 29 separately counted zero-target unmaps, and 27 physical frees.
Every selected object has zero final receipts, grants and pending deliveries.
There were no duplicate terminals or physical frees in these accepted runs.

Initial 2-CPU pilots observed both forced orders and the freely selected order.
Every failed or superseded attempt remains indexed with its original source
map and classification; a successful reparse cannot erase an earlier result.
One early build changed source during fixture development and is invalid for
identity acceptance. Preparatory native runs precede the final lint repair.
The first final-source matrix launch failed before boot because the sandbox
denied the local QMP socket; its empty serial, error log and withheld host ACK
are retained. The approved rerun uses the unchanged bounded guest protocol.
The first ISO inspection used an incorrect internal path; both that failed
inspection and the corrected extraction are retained separately from native
guest results.

The first primary strict-lint comparison found a new compile-time constant
assertion in the fixture. It was replaced with a conditional feature guard,
without any lint suppression. The repeated comparison reports kernel 133/133,
Account daemon 5/5 and probe 1/1 diagnostics versus the isolated task-entry
source archive, with zero new diagnostics and no non-lint compiler failures.
Strict Cargo Clippy remains FAIL on unchanged dependency/build-script
diagnostics and does not reach all primary crates. The native kernel command
reports 51 virtio, seven FAT, five compat-linux, three TUI build-script, two
sensors, two SMBIOS, one IPC and one kernel build-script errors. The service
command stops on the same FAT and IPC failures. All 14 diagnostic source files
match the task-entry hashes. Supplemental primary comparisons do not turn
these failed strict Cargo commands into passes.
No lint level or toolchain is weakened. Unavailable Miri is UNAVAILABLE and its
execution is NOT_RUN.

The authority boundary is unchanged: Account's role alone authorizes grant
mint/consume; a deferred token is local to its actual server; cancellation
acts only on the current caller. Test scheduling does not create an identity,
role, endpoint owner, grant, pending call or terminal result. No bearer token,
password, credential seed or completion handle is printed. The kernel hook has
no unsafe block. The additional service unsafe calls use the existing native
syscall ABI and aligned atomics inside the already-mapped validated request page.
Production IPC/VM/SHM, exec, caller-exit and scheduler algorithms are preserved.

The broader Global Authority, Grant Lifecycle, Session Authority, Reaper/Endpoint
and Doorbell security gates keep their independent classifications. A completion
versus cancellation proof does not close concurrent endpoint destruction, VFS
commit/delegation, recipient-bound Restricted SHM mapping, public Doorbell or
production FastLane integration.

## Reproduction

```sh
python3 tools/fastlane_f11b0e3_validate.py --section build --h4 --cases N6 --account-cancel 8 --evidence-root target/fastlane-completion-race-evidence
python3 tools/fastlane_f11b0e3_validate.py --section native --h4 --cases N6 --account-cancel 8 --account-exec-cycles 3 --cpus 2 4 12 --image-manifest <new-build>/validation-summary.json --evidence-root target/fastlane-completion-race-evidence/mode-8
```

Use the same race image with modes 9 and 10 in separate evidence directories.
Legacy modes 1-7 require their original separate cancellation image. A changed
source, embedded executable, ELF or ISO invalidates identity acceptance.

Next smallest executable milestone: actual authenticated completion versus the
original caller's ordinary exit syscall. Reuse Account and the original caller
as the two real syscall actors, with the parent independently observing and
reaping its child. Force both lifecycle winner orders with the same grant/frame
trace, actual borrower and owner quiescence, and no terminal publication into a
dead caller. The earlier ordered caller-exit modes remain the regression
baseline. Signal posting alone must not be counted as actual exit. Keep endpoint
destruction owner-only and production FastLane blocked pending its independent
gates.
