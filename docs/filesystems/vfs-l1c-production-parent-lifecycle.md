# VFS-L1C production parent lifecycle

The Stage 1 plan distinguishes the captured transaction from the original
reply ticket. Exit and exec retire the exact ticket; they do not revoke the
captured source. Live profile withdrawal remains a separate revocation edge.
The existing debug adapter's cancellation on exit is a control for rollback
and cannot serve as the production contract.

There is a separate publication decision: the current scheduler insertion
copies session and foreground state by numeric PPID. A detached transaction
cannot use that insertion contract as-is. Its publication must use captured
generation-aware attribution and respect launch authority revocation. This
change does not decide that policy or activate ordinary spawn.

## Production ownership

The opt-in transaction entry point transfers the private child rollback token
into the existing bounded continuation slot after staging succeeds. The
scheduler still owns the private PID reservation and arbitrates transfer and
rollback. Normal exit, remote termination, and exec already retire the exact
VFS process identity. That shared edge detaches an owned transaction's waiter
and keeps its source binding pinned; legacy caller-scoped requests still
cancel, as does a live profile withdrawal before detachment.

An already delivered retain may complete against the captured source after
the old reply ticket is gone. The pending caller-retirement barrier still
applies to new derivation authorization. Reply validation still requires the
exact active registration, request, private child, and captured source.
The original waiter cannot consume or publish a detached transaction.

Scheduler polling marks terminal work; the BSP timer takes at most one
detached terminal rollback token and destroys the private child after dropping
the runtime and scheduler guards. This does not need the parent to return
from a syscall. Local rollback can finish before remote settlement. The PID
claim and the continuation's source pin survive until the exact remote
obligation settles or the existing service-image proof establishes that the
old authority is gone. A retired registration alone cannot recycle them.

## Validation scope

The native fixture uses the production ownership entry point. Its two cases
exit with an unsent retain or an accepted receipt. The fixture waits for exact
ticket detachment and validates the source pin before it explicitly cancels
the child. Its callback after disposal records the census; production timer
drain performs the disposal. The old debug normal-exit disposal hook has been
removed from the exit path. Earlier adapter results remain separate controls.

Both cases passed in fresh **2 GiB** QEMU TCG guests in order **2, 1, then
4 vCPUs**. Each child stayed private and returned 550 user frames and eight
page tables, closed three descriptors, and had zero populated swap blocks.
Each settled census showed zero live frames, private PID reservations,
continuations, VFS claims, IPC references and stat charges for that child.
Init collected each parent's status 37 once. The
[machine results](vfs-l1c-production-parent-lifecycle-results.json) include
image hashes, QEMU arguments and exact serial lines.

The host suite covers exact identity mismatch, an in-flight acknowledgement
after simulated exit or image replacement, rejection of the retired reply
ticket, and remote claim settlement with exact service-image proof. Those
cases do not provide native remote-kill, exec, or service-retirement race
evidence. The initial host attempt exposed a retirement barrier on an
already delivered reply; that failure is preserved in the run directory.
The production release build and all host regressions passed; the registration
suite ran 107 tests, including the existing 112-cycle mixed regression. The
primary ownership guard still reports the same four pre-existing findings.
The isolated candidate passed the ownership guard and ordinary Phase 3.0
boot on 2, 1 and 4 vCPUs. Standard SWAP-1 passed on 4 vCPUs with two pools.
These runs do not clear the primary guard or qualify swap in a VFS transaction.

## Readiness

**VFS-L1C-3 remains blocked.** The production ownership and rollback foundation
now exists. The PARENT requirement remains partial: detached publication
needs generation-aware scheduler insertion, the captured launch reservations,
and their revocation checks. Native acknowledgement-in-flight, parent/service
retirement ordering, remote kill, and exec remain unqualified. The existing
trusted service reuse, heavier resource census, transaction swap-in, heap
attribution, ordinary adapter, primary ownership, and interactive application
gates remain in force. VFS-D1 stays deferred.
