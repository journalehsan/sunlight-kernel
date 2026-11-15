# SFS-5B.5 asynchronous backend design

## Why the previous backend could not show overlap

SFS-5B.4's hook ran entirely inside `virtio_blk_handle_request()` and
`virtio_blk_reset()` on the main loop thread under BQL.  Every payload copy and
the reset handler were therefore strictly serialized, and the milestone
correctly refused to call that simultaneous DMA.

## The new backend

`qemu-async-worker.patch` adds a device-private, joinable `QemuThread` to
`hw/block/virtio-blk.c`.  It is reachable only for the disposable attachment
(`serial=SFS5B3-disposable`), only for the sector-63 OUT request, and only when
`SFS5B3_MODE` starts with `async`.

Lifecycle, in order:

1. `virtio_blk_handle_request()` fetches the request; `virtqueue_pop()` has
   already mapped the payload.  The hook records `MAPPING_ACQUIRED` with the
   mapped host pointer and length, then `REQUEST_FETCHED`.
2. `qemu_thread_create()` starts the worker.  The worker publishes
   `ACCESS_ENTERED` and enters a loop that `memcpy`s the real 512-byte payload
   through the mapped host pointer, incrementing an iteration counter under the
   worker mutex.
3. The main thread publishes `ACCESS_ACTIVE`, then the test-only status byte
   `0xfe` (`GUEST_WITNESS_PUBLISHED`).  The guest polls that byte and only then
   declares its timeout, so the cross-boundary handshake precedes the reset.
4. The guest quarantines, takes reset ownership and writes status 0.
5. `virtio_blk_reset()` calls `sfs5b5_reset_enter()` **while the worker is
   still reading**.  It publishes `RESET_ENTERED`, sets `reset_entered` under
   the worker mutex and waits (bounded, 2000 ms) for the worker to observe it.
   The worker publishes `ACCESS_OBSERVED_RESET_ACTIVE`; the main thread records
   the iteration delta and publishes `ACCESS_ACTIVE_DURING_RESET`.
6. The main thread sets `cancel_requested`; the worker exits and publishes
   `ACCESS_EXITED`; the main thread publishes `CALLBACK_QUIESCED` and joins
   (`WORKER_JOINED`).
7. Only then does the main thread call `virtqueue_detach_element()` (which
   unmaps the payload) and `g_free(req)`, publishing `MAPPING_RELEASED` and
   `OLD_EPOCH_QUIESCED`.
8. `virtio_reset()` clears the queues and calls the guard hook, which publishes
   `RESET_ACKNOWLEDGED`.

The negative model (`async-negative`) skips step 6's join, publishes
`RESET_ACKNOWLEDGED` while the worker is still reading, and lets the worker
publish `NEGATIVE_POST_ACK_ACCESS`.  It still joins before unmapping, so the
emulator itself never performs an illegal access; the violation is a real
post-acknowledgment read that the independent event-order observer detects.

## Why this satisfies the hard concurrency rules

| Rule | How it is met |
|---|---|
| No unprotected guest pointer across a worker lifetime | the pointer is passed together with the mapping that pins its MemoryRegion; the mapping is unmapped only after join |
| No callback touching a destroyed device | the worker is joined inside `virtio_blk_reset()` before the request is freed |
| No mapping released while an authorised user is active | `MAPPING_RELEASED` is emitted after `WORKER_JOINED` |
| No reset callback waiting on a worker-held lock | the reset path waits on the worker mutex, which the worker releases between iterations; the worker never takes BQL |
| No worker waiting on a reset action | the worker only reads an atomic flag and exits |
| No unsafe access from arbitrary host threads | the thread is created and joined by the device and reads only its own retained mapping |
| No detached untracked thread | one joinable thread per fixture, joined on every path |
| No unbounded barrier | every wait is `qemu_cond_timedwait` with a 2000 ms bound and a `QUIESCE_TIMEOUT_BOUNDED` event on expiry |
| No test-only workaround in production code | the whole block is guarded by the disposable serial and `SFS5B3_MODE`; the default transport is unchanged |

## Known limitations (not bypassed)

* The worker reads the mapped host pointer directly.  That is the same access
  model as QEMU's IOThread dataplane, but it is not a physical DMA engine.
* The request is never submitted to the block backend, so `blk_drain()` is not
  exercised by this fixture and no claim is made about arbitrary backend
  quiescence.
* Guest RAM is static and pinned; hot-unplug, migration and allocator reuse are
  out of scope.
* No IOMMU, no hardware, no persistence claim.
