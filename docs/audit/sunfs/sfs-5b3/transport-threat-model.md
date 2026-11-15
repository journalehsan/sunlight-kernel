# Transport and threat boundary

Scope: native x86_64 SunlightOS, coherent guest RAM, legacy PCI VirtIO Block
1af4:1001, BAR0 port I/O, split queue 0, no MSI-X, IOMMU or physical PCI claim.
Actual features and physical queue/request addresses are printed per invocation.
The 32-bit legacy feature interface cannot negotiate bit 40 RING_RESET. The
existing allowlist is bits 1/5/6/9; prior offered 0x71006ed4 and accepted 0x240
are historical until reconfirmed by each new guest log.

A. Compliant device: published used buffers and acknowledged whole-device reset
supply protocol ownership boundaries under the applicable specification.
B. Implementation defects: wrong callback ordering, incomplete drain or stale mapped
pointer access require independent observation and source review.
C. Malformed data: invalid head/index/chain/status must quarantine. A plausible
but incorrect completion can exceed these checks.
D. Adversarial hypervisor/device: arbitrary guest memory modification is outside
containment. If an adversarial device can forge a completion perfectly
indistinguishable from a legitimate completion, the driver cannot identify it
using information absent from the wire protocol. The legacy head does not carry
an authenticated software generation. No synthetic generation fixes that property.
E. Software races: exclusive mutable ownership serializes this synchronous driver;
software identity, admission latches and pinned resources prevent unsafe reuse.
This is not evidence of independent concurrent reset ownership or scheduler fairness.

The QEMU hook modifies the data consumption of one actual submitted sector-63
OUT request on serial SFS5B3-disposable. It has no physical-device implications.
Only new exclusive 4MiB regular images and an ISO are attached by the runner.
No native SunFS mount, physical formatting or root migration is authorized.
