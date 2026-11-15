# SFS-5B.1 typed native receipts

Scope: implemented for direct legacy VirtIO requests. `sunlight-block/src/contract.rs`
is allocation-free and `no_std`. This is not a native SunFS adapter or a fence API.

`Geometry` records a backend enum, checked attachment incarnation, optional persistent
identity, logical size, independently unknown physical size, capacity, request/byte
limits, I/O and pointer alignment, read-only/flush/FUA/cache knowledge and fence class.
`Knowledge::{Known,Unknown,Unsupported}` prevents missing information from becoming
an invented guarantee. `validate/check` reject malformed sizes, byte-capacity overflow,
LBA+count and multiplication overflow, capacity violations, excessive/zero transfers,
wrong buffer length and alignment before ownership acquisition. Direct VirtIO and USB
operations are one 512-byte block; the VirtIO bounce buffer accepts unaligned caller
pointers. A prospective SunFS adapter must additionally validate its own volume format,
required capabilities, current attachment and backend qualification before any write.

| Representation | Meaning |
| --- | --- |
| `Owner::Submitted`, `Completion::Submitted` | Published/accepted; no success or durability promise. Synchronous public calls return terminal receipts; the owner records this intermediate state. |
| `Completion::Completed` | Device status OK, or a backend's documented complete copy. VirtIO bytes are exact only under the device's status contract. |
| `Completion::Partial`, `Amount::{Exact,AtLeast}` | Type can represent authoritative partial/lower-bound progress. Native VirtIO/BOT do not produce a prefix from unreliable length/residue. No tested native partial producer is claimed. |
| `Completion::Failed` | Terminal device failure, not proof that a write had no side effects. |
| `Completion::Uncertain` | Deadline/malformed completion; no known completed amount and retained DMA ownership. |
| `Completion::Unsupported` | Local feature refusal has no request and zero transfer. Device status UNSUPP has request provenance and unknown amount. |
| `Completion::Unavailable` | Quarantined attachment refuses a new request; earlier I/O remains uncertain. |
| `Completion::Rejected(reason)` | Validation/exhaustion refusal before submission, zero transferred. |
| `Durability::NotEstablished` | A successful write is visible/completed, without a persistence cut. |
| `Durability::VirtualBarrier { through_generation }` | A successful real flush covers earlier completed generations of this exclusive serialized virtual queue, under the recorded backend/cache contract. |
| `Durability::Uncertain` | Write/flush failure or ambiguous completion; prior media side effects are unresolved. |
| `Durability::NotApplicable` | Read receipt; volatile MemDisk persistence is also not applicable, but MemDisk has no full typed receipt producer yet. |

Each `IoOutcome` retains device identity, optional generation-qualified request,
operation, LBA, requested blocks/bytes, completed amount, status source and durability.
Rejected operations have no submitted request identity. After quarantine, the refused
new operation has no request; `outstanding()` separately returns the retained old owner.
A failed flush never asserts that earlier writes were absent. Reset is not exposed as
success, cancellation or retirement. Readback proves visibility only.

`IoOutcome::legacy` only succeeds for completed I/O. Any uncertain durability, partial,
submitted or uncertain completion maps to the distinct `BlockError::Uncertain`.
Unavailable, unsupported and definite read failure remain distinguishable. Reasons for
rejection collapse to the old `OutOfRange` error only in this intentionally narrow
compatibility bridge; the typed receipt retains the precise reason. Kernel VirtIO
adapters use this bridge and the driver retains its latest receipt for diagnostics.

Migration is incomplete: USB publishes geometry and conservatively returns Uncertain
then permanent Unavailable on runtime faults, but still has no per-command full receipt.
MemDisk copies synchronously into volatile RAM; legacy flush means RAM visibility and
its geometry refuses persistent flush. Cache writes acknowledge software acceptance,
partition flush forwards to the whole parent, and FAT/VFS still erase typed errors.
Cache, partition and `KernelDisk` geometry deliberately default to Unsupported; the new
query cannot admit these wrappers to a future native SunFS adapter. Existing FAT behavior
has not been certified as SunFS-safe. There is no silent native SunFS adapter claiming
that these old errors are definite, retryable or fenced.

OASIS VirtIO 1.2 sections 2.7.8.1 and 5.2.6.3 recommend ignoring legacy used lengths.
The driver records that field for diagnostics and uses validated used head/index,
owned descriptors and status for completion. A status-OK legacy response cannot prove
an independently measured byte count or detect a dishonest short transfer. This keeps
short-transfer qualification and complete receipt migration PARTIAL.
