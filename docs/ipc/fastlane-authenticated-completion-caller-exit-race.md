# Authenticated Account completion versus actual caller exit

2026-10-31. **COMPLETE for this scoped milestone.** Production
FastLane, Restricted SHM and public Doorbell remain disabled. No commit or push.

Evidence root: `target/fastlane-completion-exit-evidence/`. The completed
[completion/cancellation milestone](fastlane-authenticated-completion-cancellation-race.md)
is the task-entry baseline. The historical baseline remains intact; this
new source independently passes the preserved regression matrix.

The separate debug-only `authenticated_completion_exit_test` kernel feature and
`session-account-completion-exit-test` service feature select this fixture.
The original root-authenticated caller submits the existing GroupCreate 61006
operation. Account uses its ordinary password, policy, session and recipient
validation, commits the real grant and defers the original reply. A separate
supervisor verifies foreign completion, endpoint-management and grant-authority
denials before releasing Account. Account alone can arm the selected live call,
bound to its committed grant, caller incarnation, endpoint lifetime and call
generation. No bearer capability, password or completion handle is logged.

Arming wakes that exact pending caller through the existing wake path without
changing its pending call, deadline or outcome. The caller returns to ring 3,
waits for the bounded shared-page handshake and calls `sunlight_libc::exit(0)`.
This executes native ProcessExit syscall 20. It does not call IpcCancel, send a
signal, explicitly release its mapping or stop its actual VM borrower.
The ordinary address-space-owner exit terminates that borrower. The parent
independently observes wait status and actual quiescent reaping.

The hook at the real ProcessExit entry binds a CPU-qualified entry witness to
the selected original caller. Other finish/kill paths cannot supply that entry.
Both actual syscall invocations record entry before contending for the existing
scheduler lifecycle lock. The selected winner holds that lock until the other
CPU independently witnesses a failed `try_lock`. Only then does the winner run
the ordinary lifecycle implementation. The loser subsequently runs its own
ordinary implementation. Synchronization has both a five-second guest-clock
bound and a fixed iteration budget, including while the scheduler lock is held.
No new production lock, scheduling policy, syscall, authority bypass, unsafe
kernel code, terminal write or resource-retirement simulation is introduced.

| Mode | Contested order |
|---|---|
| 11 | Account completion commits before actual caller exit |
| 12 | Actual caller exit commits before Account completion |
| 13 | Both enter before atomic test-coordination winner selection |

The mode-13 selection controls lock acquisition, not an IPC result. These are
bounded controlled interleavings; they do not measure scheduler fairness or
cross-CPU latency.

`EX-RACE` records arming, real syscall entry, both actors' lifecycle entries,
lock ownership, independent contention, the real Finished state publication,
exit cleanup commitment and Account's actual syscall result. `EX-OUTCOME`
snapshots observe the original caller and grant/receipt/delivery inventories
under the existing serialization. Trace sequence numbers order events without
assuming synchronized CPU clocks.

If completion wins, exactly one `ReplyDelivered` terminal exists. Its grant and
receipt remain live after completion; exit then retires them, preserving the
committed reply until normal reaping clears the process IPC state. This also
exercises exit after successful completion without collecting its bearer grant.
If exit wins, the operation's lifecycle outcome is `CallerExited`: no IPC reply
terminal may be published into the retired caller. The active Account completion
returns InvalidArgument (4). Its pending-call metadata may remain in the Finished
process until reaping, where it must be cleared. The assessor does not invent an
IPC cancellation terminal for this exit outcome.

Account retains its own request mapping while the parent observes caller and
borrower reaping. Account retries the old completion twice, checks liveness and
must fail old-caller mint/consume. A fresh unrelated supervisor call through the
same Account endpoint cannot adopt the old grant or completion. The original
caller's endpoint must be retired. The Account daemon is then restarted through
the ordinary authorized manager path and the session logs out. Final receipt,
grant, pending-delivery, request-mapping and deferred-target inventories must be
zero. Duplicate terminal, logical retirement or physical-free records fail.

The shared per-object observer links the authenticated grant to its original
caller/issuer incarnations, VMs and request backing frame. Both original mapping
views require exactly one unmap request, actual PTE removal, matching shootdown
request, all required handler-produced remote acknowledgements, TLB completion,
unmap completion and reference release. Caller view removal must be inside
quiescent exit reaping. The first release retains one view; the final release
transfers the frame to PMM only at zero views. Exactly one physical PMM ownership
update must follow both TLB completions. The Account view cannot be removed by
caller-exit cleanup. Nonzero remote masks require the exact independent handler
acknowledgements; zero-target unmaps are counted separately.

The operation grant and legacy request SHM are separate real objects. This is
not a Restricted SHM policy claim. Forced numeric PID reuse, hostile stale-address
execution, forced same-frame reuse and a common-epoch whole-system census remain
outside this scoped race proof.

## Final-source exit matrix

All nine new guests passed with independently executing native syscall actors
and independent host acknowledgements. The same unchanged build-input hashes
bind every accepted run; complete manifests and physical records are in
`validation-summary.json` and `physical-objects.json` under the evidence root.

| Mode | CPUs | Authenticated cycles | Actual winners |
|---|---:|---:|---|
| 11 | 2 | 3 PASS | completion, completion, completion |
| 11 | 4 | 3 PASS | completion, completion, completion |
| 11 | 12 | 3 PASS | completion, completion, completion |
| 12 | 2 | 3 PASS | caller-exit, caller-exit, caller-exit |
| 12 | 4 | 3 PASS | caller-exit, caller-exit, caller-exit |
| 12 | 12 | 3 PASS | caller-exit, caller-exit, caller-exit |
| 13 | 2 | 3 PASS | completion, caller-exit, completion |
| 13 | 4 | 3 PASS | caller-exit, caller-exit, caller-exit |
| 13 | 12 | 3 PASS | caller-exit, caller-exit, caller-exit |

These runs demonstrate 27 real original-caller exits, 27 selected operation
grants, 54 mapping removals and 27 actual physical frees. Every selected object
has zero final receipts, grants and pending deliveries. All 54 unmaps have zero
remote targets: caller cleanup occurs after complete VM quiescence, and Account
releases its own remaining view after caller reaping. No remote acknowledgement
or remote flush is claimed from these exit-race unmaps. The separately replayed
completion/cancellation and exec regressions retain their remote-target checks.

Host validation passes 141 kernel tests, 241 service/library tests, 19 manager
runtime tests, 147 tests with the Doorbell adapter enabled, scheduler checks and
143 FastLane parser methods. Miri is unavailable; its execution is NOT_RUN.

The first strict primary comparison found one new manual range-expression lint
in the shared fixture guard. It was repaired without changing lint levels.
The repeated comparison has kernel 133/133, Account daemon 5/5 and probe 1/1
diagnostics against the isolated task-entry archive, with zero new diagnostics
and no non-lint compiler errors. The separate strict Cargo commands still FAIL
on unchanged dependencies/build scripts. All 14 diagnostic source files match
task entry; these failures are preserved and are not relabelled as passes.

Both initial 2-CPU pilot orders and their exact source/image identities remain
preserved as preparatory evidence. The first final matrix attempt failed before
boot because the sandbox denied the local QMP socket. Its empty serial log,
QEMU error, failed assessment and withheld host acknowledgement remain indexed.
The approved rerun uses unchanged source, guest timeouts and lifecycle protocol.
The production-contract audit verifies 15 unchanged IPC, capability, VM, SHM,
TLB, PMM, foreground and Doorbell source files. Only opt-in observer/coordination
hooks wrap the existing scheduler and syscall lifecycle implementations.

## Preserved regressions and production image

| Final-source regression | 2 CPUs | 4 CPUs | 12 CPUs |
|---|---|---|---|
| Concurrent authenticated completion/cancellation | 3 modes x 3 cycles PASS | 3 modes x 3 cycles PASS | 3 modes x 3 cycles PASS |
| Legacy cancellation/caller-exit and endpoint lifetimes | 7/7 PASS | 7/7 PASS | 7/7 PASS |
| Privileged Account exec and physical retirement | 3 cycles PASS | 3 cycles PASS | 3 cycles PASS |
| Account authority contention S1/S2 | 2/2 PASS | 2/2 PASS | 2/2 PASS |
| Authenticated reverse replies and real peer exec | 15 operations PASS | 15 operations PASS | 15 operations PASS |
| IPC/VM/SHM plus kernel-method reply race | PASS | PASS | PASS |
| Native Doorbell adapter | 42 rounds PASS | 42 rounds PASS | 42 rounds PASS |
| Native scheduler lifecycle | PASS | PASS | PASS |
| Ordinary production startup, IPC, VM and SHM | PASS | PASS | PASS |

The 60 fixture/regression guests and three additional ordinary-image guests
passed on the final source. Ordinary production services, kernel and ISO build
with no fixture features or markers; the kernel extracted from the ISO matches
its archived ELF, with all archived embedded services present. The additional
ordinary release kernel check passes. The ordinary runtime excludes the absent
mm2a_test_injection-only marker by inspecting the actual ELF and records that
feature distinction. Instrumented VM regressions keep their injection checks.
The authenticated reverse-reply base classifier retains its original
PREVENTED_BY_PROVEN_PRODUCTION_CONTRACT classification; its separate authenticated
reply assessment passes and is not relabelled as endpoint-destruction contention.

The exit matrix has 11 completion winners and 16 caller-exit winners: 27 unique terminal lifecycle outcomes. Only the completion winners publish an IPC reply terminal; the exit winners publish none. No retired caller receives a new reply.

Completion/cancellation physical regressions independently validate 27 selected objects, 25 remote acknowledgements, 25 remote flushes and 29 separately counted zero-target unmaps.

Account exec physical regressions independently validate 9 selected objects, 9 remote acknowledgements, 9 remote flushes and 9 separately counted zero-target unmaps.

Endpoint IDs are never-reused lifetime identities; call generation is a
separate exact IPC identity. Independent host receipts bind each supported native
protocol to its original serial and images. The evidence index preserves failed
and superseded attempts; no successful reparse changes their original result.
Forced numeric PID reuse, a hostile stale-address instruction, forced same-frame
reuse, common-epoch whole-system reconciliation and production transport readiness
remain outside this scoped proof. All broader security gates retain their prior
PARTIAL/BLOCKED classifications. The next independent lifecycle milestone is
actual endpoint-owner destruction contending with authenticated completion.
