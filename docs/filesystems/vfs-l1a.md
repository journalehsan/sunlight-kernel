# VFS-L1A — retirement proof audit

**BLOCKED for native qualification.** KLC-1 implements last-owner exit settlement for the embedded VFS service. The unchanged host fixture reproduced four-slot exhaustion. The corrected host path recovered at capacity and passed 100 replacement cycles. See [KLC-1](../kernel/klc-1.md) and [current gate results](vfs-l1a-results.json).

| Resource and owner | What can discharge it |
|---|---|
| Runtime registration slot | Under the runtime lock, `collect_remote` takes a retired row only when its ownership table is empty and its caller rows are empty. Four slots are capacity, not four live services. |
| Kernel retain record | `Ownership::prepare` reserves one of 64 entries per registration before send. Registration, nonwrapping request ID and exact scope identify the request. The authenticated service refusal or acknowledged release clears it. An uncertain send stays owned. |
| Service lease | The service retention table holds an Arc to the exact directory pair. A success receipt identifies a particular lease. An unanswered retain can have created that lease without ever delivering its identity to the kernel. |
| Directory handles | Policy and other leases may share the pair. Dropping one reference need not close either handle. The final pair destructor closes both against the retained backend. Retain itself performs no persistent filesystem write. A process killed by the kernel does not run Rust destructors. |
| Kernel namespace and transport | Preparing rows hold snapshots; bound rows retain a namespace binding. Pending requests can hold resources independently. Registry withdrawal retires admission and marks pending requests; reconciliation waits for binding users to drain. A caller-delivery row also covers file-owner retirement and possible exec transfers. Neither is a disposable receipt. |
| Process and endpoint identities | The registration identifies endpoint owner, PID, incarnation, VM generation and service epoch. Scope binds the caller identity and namespace generation. They are identifiers, not PID reservations. The production spawn adapters still have no private pending-child reservation. |

Under runtime serialization, withdrawal abandons pending installation. A later authenticated success can only own compensating release. If success wins first, the installed binding retains the lease until local users drain. The ordinary terminal path remains an authenticated refusal or release acknowledgement. The new exit path takes the exact retired epoch row once the scheduler certifies private image retirement and all installed binding users have drained. Caller and file-owner barriers in that row refer to state in the destroyed service. Kernel continuations and descriptors retain their existing ownership.

The reaper withdraws registration before removing IPC references, revoking endpoints and reclaiming the VM. Its proof issues after last-owner exit cleanup. [KLC-1B](../kernel/klc-1b.md) adds a separate proof for embedded service backing after successful live exec has withdrawn exact admission and reclaimed the old VM. Borrower cleanup, failed exec, endpoint-only withdrawal and timeout provide no such proof. Copies delivered to other images and shared mappings retain their independent lifetime contracts. The current trusted service's mounted directory leases use private image memory. No successor authority participates in settlement.

Host validation exercises the production proof gate and settlement logic with simulated scheduling and resource reclamation. The ordinary guard still reports 71 references and four outside baseline. Boot stops before QEMU, so UP/SMP retirement, VM accounting and peer progress are NOT_RUN. Full qualification remains blocked, as do pending-child publication and VFS-D1.

**Descriptor harness.** The host crate compiles the production descriptor module directly. Its missing ABI and paging dependencies and omitted procfs, networking and PMM interfaces were test-integration drift. The harness now links the real ABI and paging types; unavailable PMM/procfs types cannot be instantiated, and unsupported network operations fail loudly. The 18 existing library tests pass; they do not validate guest memfd or networking behavior.

## VFS-G0 follow-up

The ordinary boot gate still fails at the unchanged ownership guard. Prior native qualification of Btop storage telemetry blocks the proposed deferral. The existing host evidence retains its historical scope. Pending-child integration remains BLOCKED. [VFS-G0 report](vfs-g0.md)
