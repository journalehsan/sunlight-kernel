# VFS-L1C production launch attribution

The opt-in VFS launch transaction now captures scheduler attribution when it
stages the retain. It pairs the existing parent PID, process incarnation and
address-space generation with the session broker, session generation,
foreground execution scope and VFS routing flags. The scheduler's private
ticket still identifies the child, and the existing registration and request
identify the remote obligation. Neither ordinary spawn entry point changed.

Attached publication rechecks the exact parent and the captured attribution
under the scheduler guard before installing the receipt. A cancelled
foreground, forced exit, or changed attribution prevents publication.
Scheduler insertion uses the captured values directly, without looking up a
parent by number or copying ownership of a session or foreground scope. The
child keeps its original incarnation and address-space generation.

Parent retirement still detaches the reply waiter while retaining the source
pin. The attribution stays with the continuation and does not grant launch
authority. Detached publication remains disabled. Rollback still takes the
private child once, and a pending remote obligation can retain the PID claim
after local teardown.

The snapshot is stored inline in the existing eight-slot continuation array
and requires no heap allocation or additional owner. On publication the
continuation and PID reservation are released; the scheduler takes the
process and the installed namespace binding lasts until child cleanup. On
rollback the local resources are dropped and the continuation record is
discarded at settlement. Scheduler and queue backing can remain allocated
for reuse. This accounting does not attribute all kernel or service heap use.

The native fixture passed in fresh 2 GiB QEMU TCG guests, in order **2, 1,
then 4 vCPUs**. Each run exercised parent exit with an unsent retain,
parent exit with an accepted receipt, and attached publication through
the owned transaction. The attached child ran and its parent collected
status 0. Init collected each fixture parent's status 37 once. Native
publication used the default foreground scope; the host mixed regression
also exercises a nonzero scope and rejects a changed scope before commit.

Each native child returned 550 user frames and eight page tables, with
three standard descriptors and zero populated swap blocks. The settled
census had no live child frames, private PID claims, continuations, VFS
claims, IPC references or stat charges. Rollback dropped both boxes;
publication transferred the 10,240-byte descriptor table to the
scheduler and the reaped slot was available for reuse. The snapshot
measured 64 bytes per slot. Two earlier 2-vCPU fixture attempts are
preserved: an observer gate withheld dispatch, then a fixture request lock
stalled after enqueue.

All 107 registration tests and the host regressions passed. Production
release compiled. The primary ownership guard reproduced its four prior
findings with no new findings; the isolated candidate passed the guard,
ordinary Phase 3.0 boot at 2, 1 and 4 vCPUs, and standard two-pool SWAP-1
at 4 vCPUs. ZRAM and swap sources were unchanged; the separate host ZRAM
tests and native pressure gate passed. See
[machine results](vfs-l1c-production-launch-attribution-results.json)
for commands, exact serial evidence and image hashes.

L1C-3 remains blocked. Detached publication still needs the orphan handling
decision, independent session/capability authority and revocation checks,
and actual production adapter reservation accounting. Remaining native
lifecycle cases include acknowledgement in flight at parent exit, parent
exit racing service retirement, remote kill, and exec. Trusted service
registration reuse, heavier file and IPC resource cases, transaction
swap-in, full heap attribution, and interactive Btop and Qalculate remain
open. Runtime replacement is disabled and VFS-D1 stays deferred.
