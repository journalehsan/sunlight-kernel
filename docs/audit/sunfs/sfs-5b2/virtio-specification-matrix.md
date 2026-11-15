# VirtIO specification and evidence boundary

Reference: OASIS **Virtual I/O Device (VIRTIO) Version 1.4, Committee
Specification 01, 8 April 2026**, downloaded from the official latest-stage HTML.
Exact downloaded bytes and extracted text are retained under `raw/virtio-v1.4.*`;
source-integrity.json seals their hashes. These requirements are applied to a
conforming legacy PCI interface, not assumed to qualify hostile hardware.

| Section | Applicable rule | Implementation/evidence | Limit |
| --- | --- | --- | --- |
| 2.1, 2.1.1 | DEVICE_NEEDS_RESET removes reliance on in-flight completion | Check status exactly 7 before publication and retirement; unexpected state quarantines | Device state can change later; no malicious-device isolation |
| 2.2, 2.2.3; 4.1.4.10 | Legacy exposes feature bits 0–31; upper bits unacknowledged | Offered `0x71006ed4`; accepted `0x240`, bits 6/9; allowlist 1/5/6/9 | No FEATURES_OK handshake in legacy |
| 2.4.1 | Status zero acknowledges reset; no subsequent queue interaction/notifications until reinitialized | Driver reads zero after writing legacy status zero | Device compliance assumed; no independent active-DMA/platform drain witness |
| 2.4.2 | Driver should recognize reset complete on reading zero | Initial bounded readback; terminal reset one read per poll, 64 maximum | A write or expired budget supplies no acknowledgment |
| 2.6.1, 2.6.1.1.1/.2 | Individually reset queues require negotiated RING_RESET and verified transport completion; release permission follows successful queue reset | **UNAVAILABLE**: bit 40 cannot be negotiated by this 32-bit legacy interface | No queue_reset field access, no importing its explicit release permission into device reset |
| 2.7.2 | Legacy split-ring layout with aligned used ring | Four contiguous pages, checked queue extent, used ring aligned to 4096 | No packed ring or noncoherent architecture support claimed |
| 2.7.8, 2.7.14 | Used entries identify descriptor chain head; index advances and driver consumes entries | One owned head0; exactly-next used index, status and full chain checks | Wire carries no software generation/incarnation |
| 2.7.13.3/.3.1 | Publication exposes chain and buffers; ordering required before index update | SeqCst fences before available-index publication and notification | CPU ordering is not DMA termination or storage persistence |
| 3.1, 3.1.2 | Initialize after reset; legacy omits FEATURES_OK steps | Init now observes zero before feature negotiation/ring publication | No recovery/reinitialization exposed |
| 3.3, 3.3.1 | After device reset queues are no longer live; exposed buffers must remain unchanged while live | Quarantine keeps all exposed resources untouched; reset does not release them | Normative conforming-device ownership boundary, not an independent platform-DMA proof |
| 4.1.4.10 | Legacy BAR0 common layout maps status to byte18, config to byte20 without MSI-X | PCI IO BAR0 at `0xc000`, status `0xc012`; fixture `00:02.0`, `1af4:1001` independently inventoried | Driver assumes MSI-X not enabled; no general modern PCI capability path |
| 4.1.4.11.0.1 | Legacy reset signature is single-byte zero write at BAR0+18 | Used with 2.4 reset acknowledgment contract | Section describes compatibility handling; not an independent queue-reset guarantee |
| 4.1.4.3/.3.1/.3.2 | Modern common config has reset/queue-reset fields and requirements | **Not this transport**; cited only to distinguish it | No modern queue_reset/queue_enable permission used for legacy |
| 5.2.6, 5.2.6.3 | Block status semantics and legacy used-length exception | Status0/1/2, real negotiated type4 flush; used.len diagnostics | No authoritative arbitrary short-count inference |

Whole-device reset requires no RING_RESET negotiation. The exact completion
condition implemented is a status-zero read following this driver's single
status-zero write, bound to its immutable device and queue incarnation. For a
conforming device, old queue interaction must cease until reinitialization. No
physical PCI posted-write drain, IOMMU invalidation, malicious-device compliance,
physical durability or safe new queue epoch is inferred. All five frames remain
pinned even with that acknowledgment.

Split-ring normal reuse follows conforming completion ownership semantics. The
preserved T17 injection writes a wire-identical old head and B's current status
slot after B publication. It is an adversarial-device counterexample, not proof
of a compliant device issuing stale completions. The current implementation does
not solve that stronger threat; G2 remains PARTIAL.

Raw QEMU traces observe reset **entry**, while the driver observes status-zero
readback. They are independent request/reset-path observations, not two
independent DMA-drain or reset-completion witnesses. The initial experimental
post-reset QMP status observer aborted QEMU and is excluded from qualification.
