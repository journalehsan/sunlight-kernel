# QEMU 11.1.2 concurrency audit — VirtIO block, DMA mapping and reset

Source: the pinned official `qemu-11.1.2` release archive
(`sha256 731b5681…838016`), unmodified except for the two test-only patches in
this evidence directory.  Every statement below cites the file and function in
that tree.  Statements about the *normative* VirtIO contract are separated from
statements about *this emulator's implementation*.

## Execution contexts

| Context | What runs there | BQL |
|---|---|---|
| Main loop thread | device `reset`, MMIO/PIO handlers, `virtio_queue_notify`, BHs, timers | held |
| TCG vCPU threads | guest code; enter the main loop only through the BQL | acquire on entry |
| `iothread` (not used here) | virtqueue processing for a virtio-blk with `iothread=` | not held |
| Block coroutines | `blk_aio_pwritev` backend I/O in the device AioContext | not held |
| Our SFS-5B.5 worker | `sfs5b5_worker_fn` in `hw/block/virtio-blk.c` | not held |

The SFS-5B.4 and SFS-5B.5 fixtures are launched with `ioeventfd=off` and no
`iothread=`, so all virtqueue processing and `virtio_blk_reset()` execute on the
main loop thread under BQL.  That is why the previous milestone could only show
serialized event order.

## Answers to the required questions

1. **Which context performs device-side payload access?**
   In stock virtio-blk, `virtqueue_pop()` maps the guest buffers
   (`virtqueue_map_desc` → `dma_memory_map`), and the payload is read by
   `virtio_blk_handle_request()` on the queue's AioContext — the main loop
   thread under BQL here, or the IOThread when one is configured.  The block
   backend then copies from the mapped host pointer inside a coroutine.
2. **Which contexts may execute concurrently?**  With `iothread=`, the IOThread
   virtqueue path and the main loop (reset, hotplug, migration) run
   concurrently, synchronised by `virtio_blk_reset()`'s `blk_drain()` and the
   `s->rq_lock` request list.  Without an IOThread, everything device-side is
   serialized by BQL.  A block coroutine always runs off-BQL.
3. **Which locks protect the device request state?**  `VirtIOBlock.rq_lock`
   protects `s->rq`; `VirtIODevice` state is BQL-protected; the virtqueue itself
   is protected by the queue's AioContext.  `Owner`-style request fields are
   otherwise single-threaded.
4. **Can an outstanding callback outlive the device object?**  Yes, if it is not
   drained: `blk_drain()` waits only on the block layer, and QEMU's own device
   teardown (`virtio_blk_instance_finalize` → `blk_drain`) relies on that.
   A device-private timer, BH or thread is *not* covered by `blk_drain()` and
   must be cancelled/joined by the device itself.
5. **Can a DMA mapping outlive its backing MemoryRegion?**  No.
   `address_space_map()` takes `memory_region_ref(mr)` before returning the host
   pointer, and `address_space_unmap()` calls `memory_region_unref(mr)`
   (`system/physmem.c`).  The mapping therefore pins the region for its lifetime.
6. **How are in-flight mappings pinned?**  By that MemoryRegion reference, plus
   the `VirtQueueElement` that owns the mapped `iov` until
   `virtqueue_unmap_sg()` runs.
7. **What happens when reset races an active callback?**  `virtio_blk_reset()`
   first asserts the dataplane is stopped, then `blk_drain()`s the block
   backend, then detaches and frees queued requests under `rq_lock`.  A callback
   that is not a block-layer request is unaffected by `blk_drain()` and would
   keep running across the reset unless the device stops it explicitly.
8. **What must complete before old request structures can be destroyed?**  Every
   user of the request's mapping, i.e. every callback/worker that dereferences
   the mapped pointer, must have exited, and the mapping must be unmapped after
   that.  `virtqueue_detach_element()` performs the unmap; calling it while a
   user is still active is a use-after-unmap.
9. **Does reset cancel, drain or merely detach pending backend operations?**
   It drains the *block backend* (`blk_drain`), and it detaches queued requests
   that never reached the backend.  It does not cancel or drain arbitrary
   device-private asynchronous work.  `blk_drain()` has no wall-clock bound: it
   is `AIO_WAIT_WHILE(..., blk->in_flight > 0)`.
10. **Can a worker access guest memory after reset acknowledgment?**  Only if
    the device lets it.  Nothing in `virtio_reset()` stops a device-private
    worker; the acknowledgment (`virtio_set_status(vdev, 0)` returning and the
    guest reading status 0) says nothing about such a worker.

## Ownership graph

```
VirtQueue (vq, AioContext, BQL or IOThread)
  └─ VirtQueueElement (elem, owned by VirtIOBlockReq)
       ├─ mapping: in_sg/out_sg host pointers
       │    └─ MemoryRegion reference (address_space_map/unmap)
       │         └─ guest RAM (pinned, coherent, no hot-unmap here)
       └─ VirtIOBlockReq (g_malloc'd, freed by virtio_blk_free_request)
            ├─ backend: blk_aio_pwritev → coroutine → BlockBackend.in_flight
            │    └─ drained by blk_drain()
            └─ device-private async work (timer / BH / SFS-5B.5 worker)
                 └─ NOT drained by blk_drain(); device must join it
```

Lifetime boundaries and their controlling authority:

| Boundary | Authority | Failure if violated |
|---|---|---|
| mapping → MemoryRegion | `memory_region_ref/unref` | region freed under a live pointer |
| element → mapping | `virtqueue_map_desc` / `virtqueue_unmap_sg` | use-after-unmap |
| request → element | `VirtIOBlockReq.elem`, freed only after detach | double detach / stale descriptor |
| backend → drain | `blk_drain` / `BlockBackend.in_flight` | reset returns with backend I/O live |
| device-private worker → device | **device code only** | callback after reset/free |

The last row is the gap SFS-5B.4 identified and SFS-5B.5 closes for the
instrumented backend: the worker is created by the device, holds the mapping,
and is joined *before* `virtqueue_detach_element()` and before the
acknowledgment boundary.

## What this audit does and does not establish

* It establishes, from source, that a mapping cannot outlive its MemoryRegion
  and that `blk_drain()` does not cover device-private work.
* It establishes that a worker thread reading the mapped host pointer off-BQL is
  the same access model QEMU's own IOThread dataplane uses.
* It does **not** establish hardware DMA fencing, IOMMU isolation, or that a
  physical device stops issuing bus transactions after a status-zero write.
  Those remain normative/conditional claims (see `transport-assumptions.md`
  in the SFS-5B.4 namespace).
