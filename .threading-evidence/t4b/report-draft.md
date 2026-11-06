# T4B — Futex contention, timeout and cancellation lifecycle

Status pending final receipts. Interrupted/EINTR/pthread-cancel scope PARTIAL:
these interfaces remain unsupported; scheduler signal/I/O spurious completion is
supported and tested separately. No T5/T6/T7 or shared-futex expansion.

Initial HEAD `73cb0b81dff5bd7d6e10d36d82122686cfaf5fa3`, no dirty source files.
Initial source identity (11,106 files), empty dirty-file inventory and pre-change
state machine are under `.threading-evidence/t4b/initial/`. Historical inventory
hashes 4,965 existing evidence files, including artifact contents, before changes.
The T4A final manifest remains preserved at its original path and source identity.

## Ownership contract

A waiter is an admitted Process slot, not an allocated queue node. Registration,
wake selection, expiry, generic completion, mapping retirement and termination
serialize on SCHEDULER. One active registration is tied to exact PID, foreground
incarnation, VM `(pml4_phys,generation)`, ABI and aligned address. CPU affinity is
placement, not identity. There is no per-wait generation counter: one thread
cannot execute another WAIT while blocked, completed metadata is detached before
runnable publication, and a new WAIT invalidates any previous native receipt.

| Before | Serialized event | After |
| --- | --- | --- |
| Running, no registration | supported matching WAIT | address/deadline published, BlockedOnIo |
| Running | mismatch/invalid word/immediate expiry | immediate return, no registration |
| Registered, BlockedOnIo | matching WAKE | detach; native receipt 0; Ready |
| Registered, BlockedOnIo | processed native expiry | detach; native receipt 2; Ready |
| Registered, BlockedOnIo | signal/I/O or committed VM cancellation | detach; spurious receipt 0; Ready |
| Any | thread/process termination, successful exec, reap | detach, discard receipt, no terminal task enqueue |
| Completed native wait | matching RESULT | consume exactly once |
| Ready/running/retired | late WAKE/expiry/cancellation | no second waiter completion |

`Option::take` in `Process::complete_futex_wait` owns the result. The expiry path
records 2 before normal wake bookkeeping attempts completion with 0, which cannot
overwrite the detached registration. Wall time alone does not choose the winner:
the first serialized terminal action does. A native success can be spurious; it
is not proof that a FUTEX_WAKE selected the thread. Reported wake counts measure
selection, independently of userspace resumption. Result receipts hold scalars,
not mappings or thread references. Teardown and constructors prevent incarnation
reuse. Wake placement removes obsolete queue entries, notifies a live owner or
enqueues a quiescent task; physical pins defer reclamation and migration safely.

The ownership audit retained the existing terminal transitions. Native std on
12 CPUs subsequently exposed a separate return-path defect: the requested
self-IPI can arrive after userspace issues another syscall. A WAIT returned 0
while still registered and BlockedOnIo; libc immediately requested RESULT and
received EINVAL because no event had completed the waiter. Two frozen diagnostic
retries captured this exact state on CPUs 3 and 10 with exact TID/incarnation,
address, active deadline and an empty receipt. This is a demonstrated defect,
not a timeout-result guess.

Both native and Linux registered WAIT now replay the original SYSCALL instruction
until a terminal event detaches the registration. The replay helper rewinds RCX
by two bytes and returns the original syscall number (155 or 202); saved arguments
and the original native deadline remain unchanged. A bounded task-local
`futex_return_pending: Option<(address, Linux)>` marks only the logical syscall
return continuation. It owns no waiter, result, mapping or thread reference and
is never used by wake selection. Active replays neither re-register nor validate
or reset the original word/deadline. Once detached, replay clears this marker and
returns the existing raw WAIT result 0. Native RESULT still consumes the winning
receipt separately. Completed waits must return even after unmap/replacement, so
the completed replay precedes memory validation. New logical native WAIT continues
to invalidate a previous unconsumed receipt. Exit/exec/reap discard the continuation
marker with the other waiter state; constructors initialize it empty.

This closes early return without holding a scheduler lock while waiting or
reintroducing blocking state after a wake. It uses the existing syscall replay
and scheduler handoff mechanism and the same waiter lifecycle. Host controls
withhold delivery for eight consecutive replays, then complete through wake or
expiry and require one registration, preserved winner, one runnable publication
and one-shot receipt. Linux wake replay is checked too. Compiling the same control
against the initial production syscall bodies fails deterministically because
initial WAIT returns 0 where a replay is required. Evidence is retained under
`host-final-replay/`.

The new debug fixture uses existing T4A hook locations; completion observations
receive the already-chosen numeric result. `futex_lifecycle_test` selects the new
fixture only in debug builds. Existing T4A observations retain their behavior.

## Control scopes

Host controls extract actual production completion, expiry, wake selection and
signal-wake function bodies into a retained Rust harness. Host adapters provide
queue placement, monotonic time and process fixtures, so these controls prove
state transitions and cardinality, not hardware context switching. Barrier-started
host threads contend on the serialization lock, with both forced orders and 64
unforced races each for wake/expiry and signal-cancel/expiry. Arc counts concern
the host harness only. C11 additionally compiles the real native syscall body with
VM/trap adapters; mutation before its atomic comparison must return mismatch with
no registration. Wrong-address and duplicate RESULT consumption are rejected.

Native evidence is QEMU multi-thread TCG ring-3 execution, not a performance claim.
One bounded owner uses static stacks/TLS and real native spawn, WAIT/WAKE/RESULT,
exit and reaped checks. Three actors run the lifecycle cases: waiter, competing
controller, independent progress worker. Native case 8 first registers 32 waiters;
the owner selects two, one becomes a waker on CPU 1, the other re-registers on an
unrelated word. The owner on CPU 0 and that worker compete over the remaining 30.
The two initial selections plus concurrent selections total 32 on the primary
word. No FIFO contract is assumed: resumed workers elect their roles atomically.
The unrelated registration remains active until explicitly woken. The per-VM
32-worker admission cap is preserved. On one CPU the wakers interleave; there is
no cross-CPU migration claim there.

| Control | Production host scope | Native scope |
| --- | --- | --- |
| C1 wake before expiry | PASS | case 0; consume wake receipt after deadline |
| C2 expiry before wake | PASS | case 1; late cancellation and wake cannot overwrite 2 |
| C3 wake/expiry contend | PASS, forced both orders + 64 races | case 2; either winner legal |
| C4 cancellation before wake | PASS, scheduler spurious completion | case 3, finite deadline |
| C5 wake before cancellation | PASS | case 4 |
| C6 cancellation/expiry contend | PASS, forced both orders + 64 races | case 5; either winner; cases 3/1 force both orders |
| C7 exit while registered | PASS for detach primitive; termination publication adapted | case 6 external termination, no resumption, exit 37 |
| C8 selected waiter migration | PASS for changed placement; hardware adapted | case 7 quiescent affinity change, wake, actual dispatch on destination |
| C9 concurrent wake(1) | PASS for both ABI queues | case 8, bounded competing calls |
| C10 repeated expiry/retry | PASS, 256 registrations | case 9, 16 registrations |
| C11 word mutation before registration | PASS for production syscall; mapping/trap adapted | inherited source-compatible T4A compare/enqueue controls |
| C12 spurious wake | PASS; subsequent FUTEX_WAKE selects 0 | case 10, scheduler wake, result 0, selection 0 |
| EINTR/signal restart/pthread cancel | UNSUPPORTED | NOT_RUN, affected scope PARTIAL |

Native C8 serializes affinity publication and wake with SCHEDULER; it proves
identity across selected-waiter migration, not unsupported live kernel-stack
migration. Native timeout granularity remains 10 ms. Linux timed waits are not
supported; Linux host wake cardinality and T4A compatibility cover the supported
indefinite Linux contract. Raw user VAs are never claimed to implement shared
backing futex keys.

## Accounting

After every native case, fixture-owned registrations, return continuations, deadlines, receipts, ready
queue occurrences, live owners, stack pins and handoff waiter bits must be zero.
Each exact PID/incarnation/VM is reaped, exit status checked, kernel stack and FS
context cleared. The owner and its VM deliberately remain alive until fixture
shutdown. Static user stacks/TLS are owner storage, not per-worker resources to
reclaim. There is no external waiter allocation or separately pending timeout
queue. FD description references return to the owner's three. Free frames return
to the post-owner baseline. Existing task-slot FD-table caches remain reusable:
heap accounting allows exactly `(maximum actors - 3) * size_of::<FdTable>()`
above the first warmed three-worker baseline, then requires stability. That
retained capacity is distinct from active allocations/references owned by threads.

The first pilot passed cases 0–7 and failed trying to admit a 33rd borrower in the
cardinality fixture. Its panic and source/ELF/ISO identity remain preserved; no
CPU expansion followed that failure. The revised fixture elects waker and unrelated
queue roles from selected waiters and passed a new 2-CPU pilot. Earlier host Rust
format-string compilation failure is also retained. Neither failed attempt counts
as accepted native evidence.


## Receipt validation correction

The first native std 12-CPU run printed `native futex wait failed`, then the
supervisor restarted docserv. Later success markers incorrectly satisfied the
inherited marker-only gate. Original receipt files remain untouched; the explicit
`std-12cpu-invalidated.json` supersedes their reported PASS with FAIL. The hardened
T4B gate rejects that failure and unexpected panics even if all success markers
follow. Only the explicit worker-panic containment subprocess is allowed. Synthetic
negative controls prove that a restarted service cannot erase a failed attempt.
The first diagnostic rerun passed, but was not treated as a fix; two further
bounded retries reproduced the empty-receipt/active-registration defect before
code changes. Corrected production behavior requires fresh native receipts.
