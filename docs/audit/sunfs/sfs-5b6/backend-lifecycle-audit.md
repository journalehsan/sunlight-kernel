# Pinned QEMU lifecycle and ownership audit

Scope: local `/tmp/sfs5b6-qemu-src`, copied from the preserved SFS-5B.5
QEMU 11.1.2 tree. `qemu-real-backend.patch` stacks on that tree; it is
test instrumentation, not a production drain change. No IOThread, hotplug,
memory migration, IOMMU or physical DMA qualification is inferred.

The authoritative functions are `virtio_blk_handle_request`, `submit_requests`
and `virtio_blk_rw_complete` in `hw/block/virtio-blk.c`;
`blk_aio_prwv`, `blk_aio_write_entry`, `blk_aio_complete`, `blk_drain`,
`blk_root_drained_poll` and `blk_set_aio_context` in `block/block-backend.c`;
`bdrv_aio_cancel_async` in `block/io.c`; `virtqueue_unmap_sg`,
`virtqueue_detach_element` and `virtqueue_push` in `hw/virtio/virtio.c`;
and `address_space_map`/`address_space_unmap` in `system/physmem.c`.

| Transition | Owner/context | Synchronization and retirement authority |
|---|---|---|
| Guest request → queue chain | Guest attachment; scheduler tasks | Guest request identity and queue incarnation; admission closes before terminal reset |
| Chain → DMA mapping | VirtQueueElement on queue AioContext | Queue pop maps descriptors; MemoryRegion references pin backing memory; does not pin guest allocator ownership |
| Mapping → VirtIOBlockReq | Device request | Element retains mapped iov; detach or used publication unmaps; no unmap while backend references qiov |
| Request → BlockBackend | blk_aio_prwv | In-flight increment precedes coroutine entry; AIOCB owns callback and opaque request pointer |
| Backend → callback | blk_aio_complete on submitting AioContext | Callback runs before backend in-flight decrement; AIOCB then unrefs itself |
| Callback → queue completion | Normal virtio_blk_rw_complete | Normal path publishes status/used ring and frees request; errors may retain requests on the retry list according to configured error action |
| Test callback → reset cleanup | sfs5b6_complete / sfs5b6_reset | Test callback records terminal result and retains request/mapping; drain returns before detach/free; no used entry published |

Cancellation authority belongs to the submitting device's live AIOCB, not
the guest timeout observer. On this AIOCB asynchronous cancellation has no
implementation and grants no terminal authority. Completion authority stays
with the submitting AioContext until `blk_aio_complete` calls the stable
opaque request callback. That callback cannot retire guest allocations.
The reset owner may detach the retained element only after backend callback
terminal state and any private join. The guest allocator has separate
retirement authority, which this experiment never grants.

The guest submission and timeout/reset tasks may run on different online
CPUs; reset ownership is generation-qualified and single-owner. The device
submission, backend coroutine, callback and reset cleanup share the main
AioContext. Their suspension/reentrancy is mediated by coroutine timers and
drain polling, not arbitrary concurrent BlockBackend calls. The private
payload reader alone is a separate host thread; its mutex and join close
access before detach. Under the declared static coherent guest-RAM model,
descriptor mappings remain device-owned while these users run. Guest PMM
ownership is still pinned after device unmap. IOMMU fencing, memory hotplug,
live migration and physical devices remain outside this proof.

At every transition an accepted write can still be partial or uncertain to
the guest. A successful backend callback is not an acknowledged guest
completion; observed sector contents are not a durable barrier. The exact
guest PMM allocation generation is also unobserved, preventing allocator
retirement even when device-local callback/mapping closure succeeds.

The fixture validates the real guest request through the existing VirtIO path,
including the existing bounds check and accounting. It submits its original
qiov to `blk_aio_pwritev` on the main AioContext under BQL. A coroutine-only
hook inside `blk_co_do_pwritev_part`, after `blk_wait_while_drained` and
`bdrv_inc_in_flight`, observes the actual backend in-flight count.
It yields through `qemu_co_sleep_ns`; it never calls BlockBackend from a
private pthread. Execution subsequently continues through the original
`blk_co_do_pwritev_part` body into `bdrv_co_pwritev_part`, except the explicit
injected EIO case.

For the during-drain case, the coroutine waits until the same backend's
`blk_drain` entry publishes the release edge, then yields another 20 ms.
The process supervisor bounds failure; its expiry cannot establish quiescence.
The hook is confined to backend name `sfs5b5`, sector 63, 512 bytes, and the
explicit test environment. One selected request per process is supported.

`blk_drain` references the BlockDriverState, calls `bdrv_drained_begin`, waits
for backend in-flight zero using `AIO_WAIT_WHILE`, ends drain and unreferences
the node. The recursive polling machinery permits coroutine timer and backend
completion progress while reset is in drain. It has no wall-clock deadline.
It does not account for unrelated private workers, timers or mapping users.

The preserved v1 failure matters: a hold before `blk_wait_while_drained`
allowed the request to reach that function during drain. QEMU then decremented
in-flight and parked it on `queued_requests`, to resume after drain. Thus
drain returned without the test callback being terminal. The retaining
callback assertion stopped the process before detach/free. This is not proof
of a QEMU defect: the hook artificially delayed request admission. It proves
that observing one earlier in-flight increment is insufficient to infer a
terminal callback at drain return. The v2 hook holds an admitted operation
with the BlockDriverState in-flight reference already acquired.

Cancellation is particularly constrained: `blk_aio_em_aiocb_info` supplies
only `aiocb_size`, no `cancel_async`. `blk_aio_cancel_async` delegates to
`bdrv_aio_cancel_async`, which does nothing when that hook is absent. Completion
must still occur. This path cannot demonstrate a cancellation-wins case.
Synchronous cancellation retains a reference and waits; it is not a bounded
escape from an indefinitely held coroutine.

Reset drains before retry-list destruction under `rq_lock`. Unrealize and
finalize drain too; those calls alone do not close extra private owners.
IOThread processing, AioContext migration and removal need separate observed
interleavings; this fixture does not enable them. TCG guest threads execute
concurrently, but the instrumented device and backend coroutine use the main
AioContext. Timing does not prove independent device thread execution.

The backend callback is deliberately replaced with a retaining test callback.
This qualifies backend admission/drain and the test callback's ownership, not
the full stock used-ring completion race. A backend terminal result is not a
guest completion and does not erase the guest's original write uncertainty.
