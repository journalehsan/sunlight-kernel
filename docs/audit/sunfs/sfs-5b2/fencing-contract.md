# SFS-5B.2 fencing contract (written before reset implementation)

Scope: coherent x86_64 legacy PCI BAR0 I/O, split queue 0, no MSI-X, no IOMMU
claim. The implemented feature allowlist is bits 1,5,6,9; actual features must be
captured from each device. RING_RESET (40), VERSION_1 (32), RING_PACKED (34)
are unavailable through the 32-bit legacy feature interface. Queue reset is
unsupported; its resource-release permission must not justify device reset.

OASIS VirtIO 1.4 sections 2.4.1 and 2.4.2 define whole-device reset completion:
read device status zero after requesting reset. The device must cease queue
interaction and notifications until reinitialization. Section 4.1.4.10 defines
the legacy BAR0 status register (byte offset 18). Sections 2.4 and 3.1 define reset and its place in initialization;
4.1.4.10 maps status to the legacy register (4.1.4.11.0.1 explicitly
identifies a single-byte zero write at BAR0+18 as legacy reset). 3.1.2 applies the general initialization sequence
with FEATURES_OK steps omitted. No reset feature negotiation is required for
whole-device reset. The generic contract is used; modern queue_reset/queue_enable
semantics are not imported into the legacy interface.

Production operation: an exclusive mutable device owner may initiate ONE terminal
whole-device reset for its immutable device/queue identity. Admission closes
before status write. Poll calls each perform at most one status read, at most 64
total. Repeated begin/poll calls cannot issue another write, change ownership,
clear original uncertainty or reopen admission. Zero establishes only
ResetVerifiedRetained. 0xff (possible removal), unexpected bits or exhausted
budget establish terminal retained failures. No reset coordinator wait depends
on an interrupt, timer, scheduler or another queue owner. The port I/O instruction
itself assumes a responsive platform; instruction completion is not software-bound.

The reset acknowledgment gives a normative conforming-device boundary for old
queue interaction, including its descriptor/buffer accesses. It does not prove
PCI posted DMA drain on arbitrary physical platforms, IOMMU invalidation, hostile
device compliance, or disk persistence. No independent active-DMA reset witness
exists at contract creation. Therefore ALL FIVE PMM PAGES remain pinned even on
acknowledged reset. No frame release, old request retirement, new epoch, queue
reinitialization or automatic retry is authorized by this implementation.

Normal conforming completion still permits serialized logical descriptor reuse
under sections 2.7.8, 2.7.13 and 2.7.14 after one used entry, exact owned chain,
status and software owner checks. A malicious old-head-plus-new-status write after
legitimate reuse is physically indistinguishable from B completion. The preserved
T17 witness is not repaired by generation counters or reset acknowledgment. Full
G2 remains PARTIAL until an enforceable ownership/epoch or isolation policy exists.

Uncertain write/flush receipts remain uncertain after reset. Reset does not imply
rollback, definite failure, flush success or permission for writable SunFS.
