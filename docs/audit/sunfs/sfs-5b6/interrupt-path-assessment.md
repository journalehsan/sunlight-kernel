# Interrupt applicability

The inspected `sunlight-virtio/src/blk.rs` uses used-ring polling in
`poll_transfer`, and a register read in `poll_terminal_reset`. Negotiation
accepts the legacy low feature word; no queue-reset feature is negotiated.
The guest baseline reports offered/accepted words independently.

VirtIO PCI supports interrupt signaling, but this driver has no block IRQ
completion owner/wakeup path. Existing x86 interrupt infrastructure does not
by itself supply queue callback synchronization, cancellation or teardown
ownership. Adding that production architecture is unnecessary to qualify the
current polling storage path and is outside this milestone's narrow backend
instrumentation. No interrupt handler is added.

Historical R4/R5 remain visible: scope applicability is NOT_APPLICABLE to the
current polling path; hypothetical IRQ-path execution is NOT_RUN. A separate
interrupt submilestone must specify ISR registration, queue synchronization,
wakeups, reset masking and teardown lifetime before qualification.
