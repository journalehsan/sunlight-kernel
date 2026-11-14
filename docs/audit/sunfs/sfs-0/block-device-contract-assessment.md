# Block and persistence assessment

Current source: sunlight-block/src/lib.rs:20-39, partition.rs:15-66,
sunlight-virtio/src/blk.rs:73-244, sunlight-block/src/usb_msc.rs:262-335,
kernel/src/main.rs:1885-2018. These bytes are bound by the source manifest.
This assessment is static; device durability and late-completion tests are NOT_RUN.

| Requirement | Present interface / implementation | Missing or unqualified |
|---|---|---|
| Read/write | Exact512-byte trait arrays, Result<(),BlockError> | Multi-sector4096 aggregation, byte/short-count interface |
| Flush | Mandatory method, Unsupported required if no barrier | Query in common trait; driver/platform honesty still needs execution |
| FUA | No common or VirtIO API | Optional; not needed by selected protocol |
| Sector size | Constant512; USB rejects other size | General logical/physical sector metadata |
| Capacity | block_count/u64 | Checked byte geometry and stable media incarnation |
| Alignment | Arrays/driver-specific DMA allocation | Common transfer/DMA constraints and max request size |
| Error propagation | Io/OutOfRange/Unsupported; VirtIO boolean | Completed-prefix, uncertainty, timeout/offline provenance |
| Completion order | Synchronous single mutable borrow; cache can defer/reorder | Completion is not persistence; timeout late-DMA fencing |
| Cache | CachedBlockDevice flush slots then inner flush | Explicit cache acceptance vs device completion; failed barrier error latch |
| Partition | Checked view bounds/offset, forwards flush | Preserve exclusive parent-device ownership and alignment |
| VirtIO | Negotiated flush bit, reports read-only, polls used ring/status | Boolean timeout does not prove cancellation or request-buffer safe reuse |
| USB MSC | SCSI sync-cache, marks offline on failure; temporary read buffer | Physical persistence/removal/reset qualification, no FUA |
| AHCI/NVMe | No dedicated implementation located in scoped filename/dependency scan | No assumptions about these transports |

Important observed distinction: cache eviction clears a dirty bit after a completed
inner write, not after stable-media flush. That is fine only if later flush reliably
drains the underlying accepted write and failures latch at the filesystem. A failed
flush cannot be hidden by a clean software-cache bit. Existing single bool/Io errors
must be conservatively interpreted as uncertain. No new production block API was added.

Future required API is in the recovery contract: geometry/capabilities, exact reads/
writes, completion certainty, media incarnation and quiesce/reset fencing. A conservative
adapter may retain the existing trait and treat every Io as uncertain, but cannot
qualify writable VirtIO use until timed-out requests are fenced before buffer/request
reuse. If flush is unsupported, admit read-only only. If flush fails after mount,
freeze mutations, preserve ownership, report EIO; never claim fsync durability.

The existing flush API supplies a useful intended persistence contract; missing
capability/error/fencing metadata and unexecuted qualification remain real integration
blockers. No statement here certifies fsync on today's native drivers against power loss.
