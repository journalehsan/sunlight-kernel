# VFS-L1C production exec lifecycle

The opt-in owned launch transaction passed the two scoped exec cases in fresh
2 GiB QEMU TCG guests, in order **2, 1, then 4 vCPUs**. Init used an ordinary
disposable parent; its private child was staged through the production owned
transaction. The fixture exercised the normal native exec syscall. All changes
are confined to the existing debug fixture and its user programs. No production
lifecycle defect was demonstrated.

| Case | Native observation |
| --- | --- |
| Invalid ELF with an accepted retain | Exec rejected the non-ELF `/etc/motd`. The same caller and captured launch still matched, and the owned retained transaction could publish. The child ran and its parent collected status 0. |
| Successful replacement with an accepted retain | The replacement executed with the same PID and incarnation and a different VM generation. The continuation had detached, was unparked, still held the private child, and passed the captured source check. The fixture requested cancellation after verifying detachment; the production rollback drain disposed of the child, and the live VFS service completed release. |

The contract fences the old caller and reply ticket only after replacement
image preparation succeeds. Attached publication requires both the exact
caller and the full launch snapshot to match. A detached continuation keeps its
source pin and remote obligation; the new VM does not inherit publication
authority. Teardown releases local resources before recycling any claim still
needed for remote settlement. The successful case verifies that ordering
through the existing production drain and the fixture settlement census.

The launch attribution remains 64 bytes inline per continuation slot. It adds
no heap allocation or independent owner. In the rejection case, publication
consumed the continuation and private PID reservation, installed the namespace
binding, and transferred the process and its 10,240-byte descriptor table to
the scheduler. Normal exit removed the binding and made the reaped slot
reusable. In the replacement case, rollback destroyed both boxes and the
snapshot stayed with the continuation until settlement. Each private child
started with three descriptors and 550 user pages; cleanup returned 550 user
frames and eight page tables with zero populated swap blocks. Both cases
settled with zero live child frames, private PID claims, adapter records,
continuations, VFS claims, IPC references, or stat charges. Init collected the
parent's status 37 exactly once per case. These counts do not attribute all
heap backing or independently census the retired parent VM at the commit boundary.

All 107 registration tests, existing host regressions and production release
compilation passed. The primary ownership guard reproduced its four prior
findings. The isolated candidate passed the guard, ordinary Phase 3.0 boot on
2, 1, and 4 vCPUs, and standard two-pool SWAP-1 on 4 vCPUs. The swap and ZRAM
sources were unchanged; the pressure gate and host ZRAM tests are independent
of the VFS transaction. The [machine results](vfs-l1c-production-exec-lifecycle-results.json)
include commands, image hashes, and serial evidence, including the first
two-vCPU attempt that stopped at an unrelated fixture replay assertion before
the successful exec case. The fixture now stops at receipt observation in that
case, as it does in the accepted-retain exit control.

This qualifies early exec rejection and replacement with an already accepted
retain. It does not exercise a failure after VM construction begins, an
acknowledgement in flight across retirement, a service-retirement race, remote
kill, trusted VFS registration reuse, or heavier file, IPC and swap-in cases.
Full heap attribution and interactive Btop and Qalculate remain open. Detached
publication still requires an orphan policy, independent launch authority and
revocation checks, and production adapter reservation accounting. Both ordinary
spawn entry points and runtime replacement remain as before. **L1C-3 remains
blocked and VFS-D1 remains deferred.**
