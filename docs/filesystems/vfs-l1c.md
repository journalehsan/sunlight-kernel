# VFS-L1C authenticated publication

**PARTIAL.** Namespace staging now hands its exact request ID to the code that records the continuation or provisioning state. Both ordinary spawn adapters still publish before binding commitment. The ordinary-child invariant and VFS-L1Q readiness remain unmet. VFS-L1, primary platform qualification, and VFS-D1 remain blocked.

## Implemented changes and adapter status

- `Runtime::queue_namespace_inner` returns the ID embedded in the retain frame that it placed in the unsent queue. Boot-root staging and derived inheritance use that ID to select the exact preparing row. Chdir uses the ID directly to record its continuation. These paths previously reconstructed the ID by finding the highest-numbered row. The ID is scoped to the authenticated registration. This change preserves the wire format, replay ordering, capacities, and service authentication.
- The registration test for queued delivery, early reply rejection and slot reuse now checks that the returned ID matches the request on the wire. A kernel host test exercises all eight private reservations, keeps the reclaiming PID charged until local teardown, and checks rejection of its old ticket after PID reuse.
- The direct `sys_spawn` path stages a local process, revalidates an inherited session before wiring stdio, inserts the child into the scheduler table, provisions inheritance using the ordinary table index, and enqueues the child. It still returns the existing PID result on success.
- `handle_spawn_call` uses `spawn_with_session_attachment` for the embedded IPC path. The helper consumes an optional session attachment before inserting, provisioning, and enqueueing. The IPC handler applies supervision, trust and reap-watcher bookkeeping under the still-held scheduler guard and returns PID and VM generation in the reply. Both adapters retain their existing semantics and neither uses the pending-child constructor in production.

## Ordering required for activation

The existing scheduler-owned `Pending` table reserves a PID and creation serial. The private loader records the incarnation and final VM generation in `Identity`. Ordinary lookup, enumeration and queueing operate on the published table. Local cancellation takes the payload under the scheduler lock and destroys the private image outside that guard. The slot remains charged until destruction completes. The local constructor currently supports canonical stdio and lacks the broader cleanup and session ownership needed by the production adapters.

| Step | Required transaction behavior |
| --- | --- |
| Construct and reserve | Use the existing private reservation and retain a scheduler-authenticated creation identity. Capture the parent identity and image, retained namespace authority, credentials and grants, and the exact active registration. Secure scheduler insertion, cleanup, and session handoff capacity before exposing a retain request. |
| Authenticate and request | Associate the creation ticket, PID, incarnation and final VM generation with the request ID and full registration, including its endpoint, owner image and epoch. Service derivation must resolve this private target through a trusted transaction record. The current `authorize_derivation` requires a matching ordinary process table row. |
| Wait | Suspend the spawning caller through a deadline-aware continuation. Existing copied StatPath admission demonstrates parking under scheduler serialization and a waiter consuming a terminal result. Both spawn adapters need their own single-consumption continuation and must release scheduler, PMM, broker and runtime guards before userspace runs. |
| Validate acknowledgement | The existing control reply authenticates the registration, correlates the delivered ID and receipt scope, and checks credential and cwd state. A pending-child adapter must also revalidate the exact private creation and cancellation state, parent identity or retained authority, grants and any deferred session or supervision permit. A duplicate, invalid or replacement-epoch reply cannot authorize publication. |
| Commit, publish and enqueue | Revalidate authority under scheduler → broker → runtime serialization shared with service retirement. Binding installation must precede the atomic move from the private slot to the ordinary process table. Ordinary table insertion is the proposed publication linearization point. The scheduler must enqueue exactly once before releasing the guard. Capacity and session handoff failure must be resolved before insertion. First ring-3 dispatch follows enqueue. |
| Cancel | The proposed cancellation linearization point is the scheduler-serialized take of the private payload, paired with fencing the exact VFS preparation under the runtime lock. Cancellation or retirement winning first prevents transfer. A reply after cancellation may settle or create exactly correlated compensation. Production arbitration across the two owners remains open. |

## Local resources, remote obligations and capacity

Caller completion, the local child image, retain request, remote lease, registration slot, and PID claim have distinct lifetimes. A future timeout should complete the caller and allow independent local destruction while preserving the exact remote obligation. Authenticated refusal or acknowledged release can settle transport state. KLC last-owner exit and qualifying successful exec settle only the retired service image's private backing and respect local binding drain. Endpoint withdrawal, epoch replacement and timeout provide no proof that delivered service state has gone away.

The established runtime owns four bounded registration rows with 64 leases per row. It allows one delivered retain per epoch because the service processes retain IDs in replay order. A stalled retain can delay later retains in the same epoch. Local construction has eight slots. The current components do not connect those slots to remote obligations. Before integration, the PID must stay reserved after local teardown for any surviving numeric-PID dependency. Recovery of a registration row by itself gives no PID release proof. A live, indefinitely unresponsive service may prevent settlement indefinitely. There is no production bound on spawn cancellation in this phase because neither adapter has a namespace wait.

Boot VFS provenance remains restricted to the existing trusted boot image policy. There is no new privilege exemption or runtime provenance grant. An integrated bootstrap policy must explicitly avoid circular dependency for trusted startup services while keeping ordinary spawn fail closed when namespace authority is absent.

## Validation scope

The registration harness runs production control and retention logic with modeled scheduler, VM and service lifecycle boundaries. Existing 100-cycle last-owner exit/replacement and mixed lifecycle regressions passed. The kernel host library builds the real pending-state source using counted resource payloads. Its cancellation/transfer race tests exercise the test-only transfer state. The new capacity test observes the local PID claim before and after teardown. These tests do not couple namespace ACKs to production scheduler insertion, dispatch or cancellation.

No memory-reclaim code changed. The local private loader creates no anonymous reclaim enrollment, and PID-based production reclaim requires an ordinary process row. VFS-L1P native accounting covers the local constructor and cancellation only. Its independent 1 GiB boot failure remains open. KLC-Q1 direct-call ZRAM restoration evidence does not cover fault-driven swap-in or exit/exec with swapped backing.

The isolated candidate passed the unchanged Phase 3.0 boot gate at 2 GiB and 2 vCPUs. The native VFS-L1P fixture rerun also passed at 2 GiB on each CPU count below. Each guest completed 100 local construction/cancellation cycles, 12 heap failures, two VM ledger failures, and 523 PMM failure points. Frame, heap, descriptor and reclaim-candidate counts returned to the guest's starting values. Init, VFS and Service Manager progressed after the fixture.

| vCPUs | Local fixture | Free frames | Heap bytes | Descriptions / references | Swap candidates |
| --- | --- | --- | --- | --- | --- |
| 1 | PASS | 266468 | 176832 | 72 / 72 | 0 |
| 2 | PASS | 266210 | 177856 | 72 / 72 | 0 |
| 4 | PASS | 265698 | 179904 | 72 / 72 | 0 |

Host regression totals were 94 registration, 168 IPC unit and nine IPC integration, 47 VFS service, 18 descriptor, and 217 kernel host tests. The focused scheduler gate passed, along with the retirement and ZRAM filters. The primary kernel compile and the candidate transaction/session and mmap feature compiles passed.

Native production delayed acknowledgement, exclusion during an actual wait, peer progress across that wait, first dispatch and first namespace-dependent metadata access remain **NOT_RUN** for both adapters. Transactional refusal, timeout, parent exit, service retirement races, and mixed transaction settlement are also **NOT_RUN**. Replacement remains **NOT_RUN** under the existing boot-only provenance policy.

The candidate retains the six reviewed KLC-Q1 source reductions. The equivalence check covered 12,346 baseline paths with no unexpected drift and matched all five VFS-L1C source files. The primary guard still reports 71 references and the same four findings. The candidate guard, baseline and ordinary boot script match the primary. Primary Btop and Qalculate source and assets were kept intact.

## Evidence

- [Machine-readable results and gate paths](vfs-l1c-results.json)
- Candidate source hashes and drift check in `target/vfs-l1c/source-equivalence.json`
- Host, guard, compile and candidate boot logs in `target/vfs-l1c/`
- Candidate local-construction matrix under `target/vfs-l1c/native/`
- Prior local and retirement evidence in [VFS-L1P](vfs-l1p.md), [KLC-1](../kernel/klc-1.md), [KLC-1B](../kernel/klc-1b.md), and `target/vfs-l1p/klc-q1/`
