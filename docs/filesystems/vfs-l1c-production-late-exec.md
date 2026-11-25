# VFS-L1C late exec failure qualification

Late exec failure remains **NOT_RUN**. This follow-up uses the existing
early rejection and successful replacement controls after the scope was
narrowed to safer tests. Neither control triggers a failure after replacement
VM construction begins. No production lifecycle code was changed.

The native syscall resolves and checks the image before invoking
`exec_into_process`. During preparation the loader temporarily installs the
replacement address space in the process record. On a preparation error it
restores the original address space, reclaims the candidate user space, and
restores the saved personality, Linux thread group and semaphore state, and
service attributes. Source review establishes the ordering; it does not
establish resource closure for an actual late failure.

Successful preparation is the boundary for retiring the old caller and its
reply authority. The loader fences that authority before activating the new
VM, reclaims the retired VM, and the syscall then handles close-on-exec,
surviving service descriptor transfers, and old reply retirement. The launch
snapshot remains 64 bytes inline. A replacement image does not acquire the
detached transaction's publication authority.

The early rejection control checks that the original caller can still publish
its retained child and wait for status 0. The successful replacement control
checks that the same PID and incarnation run with a new VM generation, that
the detached continuation still pins the source, and that cancellation drains
the private child and remote retain. The fixture checks for zero remaining
child frames, private PID reservations, adapter records, continuations, VFS
claims, IPC references and stat charges. These checks concern the child
transaction; they do not measure the failed replacement VM, exhaustively
attribute heap allocations, or independently census the retired parent VM.

The [machine results](vfs-l1c-production-late-exec-results.json) distinguish
fresh control execution from unqualified cases and keep the primary ownership
guard separate from the isolated candidate gates.

Late preparation failure and its old-image and resource census, in-flight
acknowledgement and service retirement races, remote kill, trusted registration
reuse, heavier file and IPC state, populated swap teardown, fault-driven swap-in
within a transaction, full heap attribution, and interactive Btop and Qalculate
remain open. Standard two-pool SWAP-1 and host ZRAM evidence are independent of
VFS transaction coverage. The existing
[integration prerequisites](vfs-l1c-integration-readiness.md) still apply,
including an orphan policy, independent launch authority and revocation, and
adapter reservation accounting before detached publication. Detached publication
stays disabled, both ordinary spawn entry points remain unchanged, L1C-3
remains blocked, and VFS-D1 remains deferred.
