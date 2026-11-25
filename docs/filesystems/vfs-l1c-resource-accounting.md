# VFS-L1C resource accounting

The existing private process creation transaction passed the scoped resource checks in 2 GiB QEMU TCG guests with 1, 2, and 4 vCPUs. Each guest ran the pre-scheduler construction and rollback fixture, then exercised publication with normal exit and cancellation with a late VFS retain reply. The accounting probes are debug-fixture code. Both ordinary spawn entry points and production ZRAM code are outside the change.

## Measured lifecycle

| Resource | Construction and admission | Normal completion and cancellation |
| --- | --- | --- |
| User frames and page tables | The staged address-space walk counts resident user mappings before dispatch. The pre-scheduler fixture also samples the PMM user-private and page-table class counters. | Native reap and private rollback report the returned frames and tables. After settlement there are zero PMM frames charged to the child PID. User-private and page-table class counts return exactly to their pre-scheduler baseline after each private cycle or injected failure. |
| Process and descriptor tables | The private transaction holds one boxed process and its boxed descriptor table with three live descriptor entries. | Publication moves the process into the scheduler vector and frees the private box. Normal reap closes the descriptor entries and drops the task stack. A consumed, reaped record can retain its empty 10,240-byte descriptor table until slot reuse. Cancellation destroys both boxes and installs no scheduler row. |
| Pending-child storage and PID | The pending table holds an inline entry keyed by a generation-bearing ticket. The reserved PID is invisible to ordinary process lookup. | Publication removes the pending entry when the scheduler takes ownership. Cancellation drops the process before releasing the PID reservation. A late remote claim keeps the reservation until compensation and caller retirement settle. The published PID can still occur in a reusable scheduler record. |
| Adapter and continuation | The debug adapter has one local owner. The boot-lived VFS runtime uses an inline continuation slot, with a provisional VFS claim correlated to the child and registration. | The adapter is disposed once on either route. The continuation slot is released and reused, and the previous completion is rejected. Settlement checks the absence of the exact continuation, namespace binding, local and remote lease, caller-delivery row and queued or delivered retain. The boot service registration remains active. |
| IPC and VFS request resources | The published child performs a copied metadata operation. This request uses a quota charge plus request buffers and namespace references. | The child has zero charged stat bytes and requests, zero matching stat, file and chdir publication entries, and zero ordinary queued IPC and reply-wait references across the shards at settlement. The reaped process has empty local IPC and reply state. The release acknowledgement is correlated with the original receipt. |

Each staged child mapped 550 user pages. Both native cleanup paths returned 550 user frames and eight page tables per child and reported zero populated swap slots on every CPU count. Each vCPU configuration ran 100 private create/cancel cycles, 12 injected construction checkpoints, two VM-ledger exhaustion cases, and 523 PMM allocation failures. The warmed private baseline included 72 descriptor descriptions, 72 references, 24,100 user-private frames, 71 page-table frames, and zero swap candidates. The fixture asserted equality of its PMM, heap, descriptor, swap-candidate and scheduler baselines after the corresponding cleanup. The R5 checker independently verified one publication and wait status, one cancelled private teardown, correlated release acknowledgements and rejection of a repeated completion after continuation slot reuse.

## Allocator capacity and live resources

The kernel heap has 16 MiB of mapped capacity. The heap-used counter also includes backing storage reserved for later reuse. During the native run the ordinary observer spawn’s task-storage reservation grows the scheduler process vector from 4 to 512 slots. The reaper leaves reusable process records, which can retain an empty descriptor table and collection backing even after all live entries are gone. The fixed pending-child, continuation and registration arrays have boot-lived storage, and an empty slot frees its live contents. These allocations and the service’s own baseline explain why global heap equality is only asserted by the isolated, warmed fixture.

After both native transactions settled, global free frames returned to the initial sample on all three guests. The heap-used counter stayed above the initial sample as the scheduler retained capacity and reaped records.

| vCPUs | Free frames before first construction and after both settlements | Heap used before first construction | Heap used after both settlements |
| --- | ---: | ---: | ---: |
| 1 | 262,218 | 277,800 B | 2,607,656 B |
| 2 | 261,962 | 278,920 B | 2,607,752 B |
| 4 | 261,450 | 280,808 B | 2,607,752 B |

The native heap and PMM samples bracket the transaction and include observer activity and background kernel work. For example, an observer can release its frames between the cancelled child’s before and after samples. The child-specific mapping walk, teardown counts and zero-ownership checks determine the runtime result. The [machine results](vfs-l1c-resource-accounting-results.json) include each raw sample, release and reap counts, and one-based serial line numbers. Local evidence is under `target/vfs-l1c-resource-accounting/`.

## Regression gates and scope

The production release compilation, host VFS registration, IPC, VFS service, descriptor, scheduler, retirement, ZRAM and swap-slot tests passed. The VFS registration suite includes the existing 112-cycle mixed publication and boundary-failure test. The transaction/session and mmap feature checks also passed. The transaction/session build needed the existing generated `session-race-probe` fixture from the local candidate.

The existing guard reported zero new findings in `/tmp/klc-q1/candidate`. The primary workspace produced the same four pre-existing findings before and after this work. Its ordinary Phase 3.0 gate stopped at the guard before boot. Ordinary boot and SWAP-1 passed at 1, 2, and 4 vCPUs through the existing guarded candidate. The accounting matrix was built from the primary workspace. The SWAP-1 pressure gate separately verified ZRAM-backed slot and PMM cleanup.

## Remaining accounting gaps

- The boot-live heap sample lacks byte attribution for each reusable process or queue allocation and for the userspace VFS service heap. The native checks cover exact transaction identities and runtime occupancy of the listed resources.
- The published child used standard descriptors and copied stat. A child with live regular-file descriptions, child-owned IPC endpoints, queued payloads or populated swap needs an equivalent transaction-specific census. SWAP-1 supplies independent coverage of populated swap cleanup and restoration.
- Native allocation accounting across service-registration row reuse in a different service lifetime, authority retirement, and parent loss remains open. The existing fault and race tests cover their previously qualified scope.

VFS-L1C-3 and later migration work were outside this qualification.
