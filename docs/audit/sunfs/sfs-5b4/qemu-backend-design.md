# Bounded event-ordering backend

Inspect QEMU 11.1.2 hw/block/virtio-blk.c (request, completion, reset, blk_drain),
hw/virtio/virtio.c (element mappings, generic reset), hw/virtio/virtio-pci.c
(status port and transport reset), hw/virtio/virtio-bus.c, block/block-backend.c,
system/physmem.c:address_space_map/unmap, docs/devel/multiple-iothreads.rst.

BQL serializes the existing hook, reset and realtime timer callbacks. A borrowed
VirtIOBlockReq and its iovecs remain owned until cancellation/detach. Reset must
cancel payload timers before undoing iovec slicing, detaching, or freeing. Any
late observer owns only scalar identity, never a freed request pointer. Each
hold expires with an explicit terminal outcome after a finite callback budget.
No callback waits for another callback while holding BQL. No custom worker
thread, raw asynchronous guest-memory access or callback lifetime extension.

Implement separate before/reset-read/release/error/negative modes on an
exclusively marked 4MiB disposable device. Partial payload read before reset;
optional remainder read at reset entry before quiescence; cancellation; generic
queue clearing; fence event; optional deliberate illegal access; original-mapping
PROT_NONE guard with positive and EFAULT pipe-copy controls. The latter probes
host CPU virtual mappings, not physical DMA/IOMMU protection or alias mappings.

The event stream has monotonic host ns, sequence, device/queue epoch and head.
Guest logical event IDs correlate via descriptor notification and synchronous
status-port reset/readback edges, not clock subtraction. Preserve raw events and
construct a graph with named synchronization edges. O3/O4 must remain
UNQUALIFIED in this architecture. G2/G3 cannot be claimed as true async DMA.


Implementation: a 0xfe test status byte is written after the 256-byte copy,
without advancing used.idx. The guest diagnostic reads this byte before timeout;
normal completion authority still requires used.idx and valid status. This is an
explicit device-to-guest handshake, not a timestamp inference. Its write is part
of the test model, not a new VirtIO feature or production identity mechanism.

Reset runs after three yielding coordinator stages. The fixture requires all
participating CPU timers and independent runnable workers to advance after owner
establishment and before issuing reset. Reset contenders are distinct task
incarnations; event logs record actual CPUs rather than configured CPU count.
The coordinator is feature-gated pending broader native race qualification.

The generic blk_drain API has no new wall-clock timeout. The instrumented held
request is never submitted to it, so the scoped cancellation path introduces no
wait for a held backend callback. This does not bound arbitrary stuck backend
I/O or certify the unmodified device's drain behavior. The payload hold has 3000
10ms callback opportunities before BARRIER_EXPIRED_RETAINED stops rearming.
The late observer retains only scalar identity and no VirtIOBlockReq pointer.
