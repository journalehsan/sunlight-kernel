# Normative fence and implementation boundary

Reference: official OASIS VirtIO 1.4 CS01, 8 April 2026, retained as
raw/virtio-v1.4.html and line-oriented text. Sections 2.4.1/2.4.2 define
reset acknowledgment through status zero and prohibit further queue interaction
or notifications until reinitialization. Sections 3.3/3.3.1 govern exposed
buffers on live queues and their removal following device reset. Sections
2.7.5/2.7.6/2.7.8 and 2.7.13/2.7.14 define descriptors, avail publication and
used consumption. Section 4.1.4.10 gives legacy PCI offsets; 4.1.4.11.0.1
identifies the byte-zero legacy reset write. Section 3.1.2 omits modern
FEATURES_OK negotiation steps for legacy initialization. Queue reset provisions
must not be imported without their feature and transport support.

Protocol: close admission; preserve attachment/queue and outstanding identity;
quarantine all uncertain DMA ownership; issue one device reset; observe at most
64 status reads. Zero is RESET_ACKNOWLEDGED. Exhaustion is RESET_TIMEOUT;
unexpected/removal values are RESET_FAILED. Queue reset is RESET_UNSUPPORTED.
All paths retain frames and admission closure. No successful register observation
is called DMA_QUIESCENCE_VERIFIED by the production API.

The specification supports a conforming-device old-queue boundary. It does not
validate an implementation, drain physical posted transactions, invalidate an
IOMMU, authenticate generations or establish disk durability. The QEMU experiment
separately observes partial real payload consumption and cancellation at reset.
Because general access coverage, allocator retirement and competing execution
contexts are unproven, DMA_QUIESCENCE_UNQUALIFIED and TERMINAL_QUARANTINE remain
production outcomes; no free or new epoch is authorized.

QEMU source path: legacy status write invokes virtio_pci_reset / virtio_bus_reset /
virtio_reset / virtio_blk_reset; block reset stops dataplane and blk_drain waits
for backend callbacks before clearing queue state. The test hook owns a separate
pre-backend held request and cancels its main-context timer, undoes sliced iovecs,
detaches/unmaps the element, and frees the device request before reset returns.
This cancellation is part of the test model. It is not evidence that unmodified
QEMU executes that modified path or that arbitrary backends have no stale pointer.
The main-context/BQL serialization means reset occurs between copy instructions.
No claim of simultaneous host memcpy and reset is made.

The guarded QEMU revision additionally protects the original host mappings for
all four queue pages and the request page with PROT_NONE after generic reset
clears queue state. A nonblocking kernel pipe-copy probe must succeed on the original payload
before protection, then fail with EFAULT on each of the five protected pages. The guard stays installed until VM process termination;
the guest's status-readback/refusal/idle observation must survive. This catches
access through those original mappings, including uninstrumented pointer accesses,
but does not contain a hostile hypervisor remapping aliases. It is a negative
access probe, not an allocator release, memory-reuse or physical DMA test.
