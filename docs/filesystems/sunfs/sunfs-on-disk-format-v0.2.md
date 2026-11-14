# SunFS on-disk format v0.2 (review proposal)

2026-11-14. Experimental format family 0, revision 2. Minimum reader 0.2.
G4 covers reviewability, not production layout stability. SFS-1 must implement an
explicit byte codec, golden vectors, negative decoding and recovery fixtures before
freezing any offset. Do not cast Rust structs onto disk or rely on repr(C).

## Universal encoding and validity rules

All integers are unsigned little-endian unless explicitly i64. UUID/magic/name byte
arrays have no numeric endianness. Block addresses count 4096-byte filesystem blocks
from the volume start, never whole-disk LBAs. Zero BlockRef means absent only where
explicitly optional; all three fields must be zero. Other pointers must be in the
segment area and within device capacity, with matching epoch and expected checksum.
Every metadata block is 4096 bytes, begins at a 4096-byte address, and has a 96-byte
header below. Fields in each payload table occur sequentially as listed; explicit
reserved/padding bytes are zero. No implicit host padding. Subrecords align to 8;
unused page tails are zero. v0.2 rejects nonzero reserved fields or unknown structure
versions; a future compatible extension needs a length/version rule and new reader.
Every array count/length is checked against page capacity and configured limits
before multiplication, addition, allocation or traversal.

CRC is CRC-64/ECMA-182, polynomial 0x42f0e1eba9ea3693, non-reflected, init=0,
xorout=0. Check value for ASCII `123456789` is 0x6c40df5f0b497347. This is an accidental
corruption detector, not authentication or a collision-proof crash oracle. Failure
model excludes malicious checksum forgery and undetectable checksum collisions.
SFS-1 must independently verify codec vectors and explicitly test checksum failures.
Metadata CRC covers all 4096 encoded bytes including zero padding, with CRC bytes
64..71 zeroed. The parent BlockRef stores that same expected CRC, additionally
binding the expected page identity/type through validation. Header fields bind UUID,
self address, segment epoch and birth generation. No arbitrary 4 KiB atomicity is
assumed. Type-specific validation follows checksum validation, never substitutes for it.

| Header offset | Field | Width | Rule |
|---|---|---|---|
| 0 | magic | bytes[8] | ASCII `SUNFSM02` |
| 8 | type | u16 | 1 SB, 2 CP, 3 table-root, 4 table-page, 5 node, 6 directory, 7 extents, 8 summary |
| 10 | structure_version | u16 | 2 |
| 12 | header_bytes | u16 | 96 |
| 14 | flags | u16 | 0 |
| 16 | volume_uuid | bytes[16] | nonzero |
| 32 | self_block | u64 | equals read address |
| 40 | segment_epoch | u64 | nonzero for segment pages; 0 for fixed SB/CP |
| 48 | birth_checkpoint | u64 | <= selected CP generation; SB=0 |
| 56 | payload_bytes | u32 | <=4000, exact decoded payload |
| 60 | reserved0 | bytes[4] | zero |
| 64 | crc64 | u64 | entire page, checksum field zero |
| 72 | reserved1 | bytes[24] | zero |

Common subrecords (version 2, contained in parent's CRC):

- NodeId: `nid:u64, incarnation:u64` (16 bytes), both nonzero. Root=(1,1).
- BlockRef: `block:u64, segment_epoch:u64, crc64:u64` (24 bytes). Aligned 8.
- Time: `seconds:i64, nanos:u32, reserved:u32` (16 bytes); nanos < 1,000,000,000.
  Invalid times have all-zero fields; validity is in the node mask. Negative seconds
  are supported. A valid epoch-zero time differs from an unavailable timestamp.

## Volume layout, superblocks and compatibility

Fixed locations are SB A=block0, SB B=block1; CP slots=blocks2,3,4. Blocks5 through
`segment_start-1` are reserved zero and never allocated. `segment_start` is
`round_up(5, blocks_per_segment)`. Trailing capacity not forming a complete segment
is unallocated and excluded from usable totals. SB copies contain identical payloads,
except `copy_index`; self_block and CRC naturally differ. Normal commits never
rewrite SB. This is a raw volume format, not an ESP or partition-table format.

| SB payload field, in order | Width | Validity / purpose |
|---|---|---|
| format_major, format_minor | u16 each | 0,2 |
| minimum_reader_major, minimum_reader_minor | u16 each | 0,2 |
| compatible, readonly_compatible, incompatible | u64 each | feature classes below |
| filesystem_block_bytes, blocks_per_segment | u32 each | 4096; 64/256/512 experiment |
| volume_blocks, segment_start, segment_count | u64 each | exact checked geometry |
| checkpoint_slots | u64[3] | exactly 2,3,4 |
| maximum_nodes, maximum_orphans | u64 each | nonzero; orphan cap <= nodes |
| maximum_table_pages, maximum_extents_per_node | u32 each | nonzero reader-bounded limits |
| cleaner_reserve_blocks, maximum_transaction_blocks | u64 each | geometry/model admitted bounds |
| copy_index | u32 | 0 or1, matches location |
| reserved | bytes[4] | zero |

SB payload is 136 bytes. Its UUID is in the common header. All geometry and limits
must match between valid copies. One valid copy permits read-only degraded inspection;
writable mount requires both agree. Differing valid UUID/geometry is ambiguous and
must not be guessed. Formatting creates both copies and initial dependency closure,
flushes, publishes two checkpoints with generations1 and2 for the same empty root,
then flushes. CP predecessors are diagnostic, not a dependency on CP1 staying present.
Interrupted formatting with fewer than two recoverable CPs is read-only/incomplete.

Feature bits initially: incompatible bit0=CHECKPOINTED_TABLES and bit1=CRC64 are
required; all others zero. Future symlink support uses incompatible bit2, currently
rejected. Compatible and readonly-compatible masks initially zero. Unknown compatible
bits can be ignored only when all accessed structures remain parseable under known
versions. Unknown readonly-compatible bits forbid writes but allow validated reads;
unknown incompatible bits forbid mount. A too-new minimum reader or major is rejected.
An unknown minor is rejected by v0.2 until an explicit compatible decoder exists.
No in-place upgrade path is promised for experimental major0. SFS-1 tooling uses
new images; future production upgrades require offline copy or an independently
crash-tested versioned migration, never a version-number overwrite.

## Checkpoint publication record

Each CP is one metadata page in a fixed slot, version2, CRC covering the full page.
The CP is the sole publication record; no mutable `active` bit or mount-time rewrite.
Its birth_checkpoint equals its generation. Fixed-slot header epoch is zero.

| CP payload field | Width | Validity / recovery |
|---|---|---|
| generation, predecessor_generation | u64 each | generation>0; predecessor<generation or0 bootstrap |
| root | NodeId16 | maps to directory (1,1) |
| node_map, segment_table, orphan_table | BlockRef24 each | all mandatory, even empty orphan table |
| next_nid | u64 | > every published nid; never wrap |
| allocated_blocks, free_blocks, live_nodes, orphan_nodes | u64 each | recompute from this CP only, not union free space |
| transaction_blocks | u64 | bounded by SB max; diagnostic admitted allocation |
| state | u32 | 0=incomplete/nonmountable; 1=complete; others invalid |
| reserved | bytes[4] | zero |

The publisher emits only state1. State0 is recognized for inspection/negative fixtures,
never accepted or rolled forward, even with a valid CRC. CP payload is 160 bytes. Self CRC validates torn publication probabilistically under
the stated checksum model. Structural validity does not prove recoverability:
validate the complete reachable graph and accounting before selecting a checkpoint.
Equal generations with different roots/digests are corruption. A repeated identical
payload generation at different slots is not emitted; readers report duplication
and require read-only inspection. No generation wrap comparison is used.
`free_blocks` excludes fixed regions but is snapshot-local; writable admission uses
the union across all still-valid CPs. CP does not encode runtime open counts or locks.

## Immutable paged tables and node mapping

Each table root (type3) payload: `table_kind:u16, reserved:u16, page_count:u32,
entry_count:u64, entry_bytes:u32, reserved:u32`, then page descriptors.
A descriptor is `first_key:u64, last_key:u64, page:BlockRef24,
entry_count:u32, reserved:u32` (48 bytes). At most82 descriptors fit (24+82*48=3960).
Root has no next pointer: v1 capacity is bounded to82 table pages, even if a malicious
SB claims more. Empty tables have page_count=entry_count=0 and entry_bytes identifying
the known record size. Root descriptors are strictly sorted, disjoint, nonempty,
and their counts sum exactly to root entry_count. No tree depth or chaining.

Each table page (type4) payload:
`table_kind:u16, reserved:u16, entry_count:u32, first_key:u64, last_key:u64`, then
fixed-size records, no next pointer. page_count*capacity is checked. Kinds:1 node map,
2 segment state,3 orphan. All tables version2; parent descriptors and header CRCs
protect pages. Segment-state records are fixed for one geometry (bitmap length below).
Table roots/pages participate in segment ownership, summaries and fallback retention.

Node-map entry (48 bytes): `id:NodeId16, node:BlockRef24, kind:u8,
reserved:bytes[7]`. Sorted unique nid; incarnation must match the pointed node;
kind 1=file,2=directory,3=symlink (requires feature). Map includes root, all linked
nodes, and all committed orphans. No unmapped child or duplicate incarnation allowed.
Maximum map entries =82*floor((4000-24)/48)=6724; actual format limits may be lower.
This capacity is deliberately small and explicit, not a scalability claim.

## Node metadata

Node (type5) payload version2; one node/page:

| Field | Width | Validity / role |
|---|---|---|
| id | NodeId16 | matches map |
| kind, encoding | u8 each | 1/2/3; encoding0 only |
| mode | u16 | permission bits; kind carries file type |
| uid, gid, nlink | u32 each | stable stored ownership; exact graph count |
| size_bytes, content_generation | u64 each | checked size; generation<=CP |
| parent | NodeId16 | directory only; root points to self; zero for files/links |
| extents, directory_pages | BlockRef24 each | table roots for corresponding payload, otherwise zero |
| time_valid | u32 | bits0 atime,1 mtime,2 ctime,3 birth; others0 |
| reserved0 | bytes[4] | zero |
| atime, mtime, ctime, birth_time | Time16 each | validity rules above |
| reserved1 | bytes[16] | zero |

Node payload is 200 bytes. nlink=0 requires orphan-table membership; no directory
hardlinks. Directory nlink=2+immediate subdirectory count when linked; removed empty
directory nlink=0. Regular file/link nlink=1 in v1 or0 orphan. Directory size is the
sum of encoded directory record lengths; file/link size is logical bytes. Future
hardlinks require graph and API work before nlink>1 regular files are permitted.
Timestamps can all be unavailable; this proposal does not supply a working clock.

## Directory pages and extents

A directory root uses the generic type3 layout with kind4, entry_bytes=0; descriptor
keys are monotonically assigned page ordinals, not name hashes. Data pages type6
payload: `owner:NodeId16, ordinal:u32, record_count:u32, used_bytes:u32,
reserved:u32` (32 bytes), then variable records sorted by unsigned name bytes across
all pages. Repack directory pages on mutation. At most82 pages; no unbounded chain.

Directory record: `target:NodeId16, name_bytes:u16, kind:u8, flags:u8,
record_bytes:u16, reserved:bytes[10]` (32 bytes), then UTF-8 name and zero padding
to8. name_bytes=1..255; record_bytes=round_up(32+name_bytes,8); no dot, dot-dot,
slash or NUL. flags=0; kind must match target node. Unique names and target generations
are checked. Directory graph must be rooted, acyclic, parent-consistent. Mutation of
both parents in rename is atomic at CP publication. Orphan directories are empty.
Checksums cover records through page CRC; no unchecked spill across pages.

Extent root is type3 kind5, descriptors keyed by logical block ranges; entry_bytes=48.
Extent data page type7 payload: `owner:NodeId16, count:u32, reserved:u32`, then records:
`logical_block:u64, physical_block:u64, segment_epoch:u64,
content_generation:u64, block_count:u32, flags:u32, data_crc64:u64` (48 bytes).
For v1 block_count=1, flags=0, encoding0, nonoverlapping strictly sorted logical
blocks; a missing range is a zero-filled hole. Each physical block holds4096 raw
bytes (no metadata header). Extent data CRC covers the LE encoding of volume UUID,
owner16, logical_block8, segment_epoch8, content_generation8 followed by4096 raw
bytes; physical address is checked by the mapping/summary, allowing relocation to
preserve logical identity but recompute CRC when epoch changes. Parent page CRC
protects expected data CRC. Read verifies before exposing bytes. Beyond-EOF bytes
in the last block are zero; shrink and reextension cannot reveal stale data.
Max file blocks=min(SB extent cap,82*82)=6724, including sparse records; maximum
logical size is additionally bounded to 2^40 bytes for safe v1 arithmetic. Directory
and extent roots use the same descriptor encoding with kind-specific validation. Directory descriptors use
first_key=last_key=zero-based ordinal; descriptor entry_count is page record_count.
Directory used_bytes equals the sum of encoded record_bytes after the32-byte prefix.
Extent descriptors use the first/last logical block in the page; count matches records.
Global table descriptors use first/last nid or segment index. Root entry_count sums
records for every kind; descriptors/owned pages occur in exactly one root position.

Symlink storage, when enabled later, uses the file extent mechanism, size<=4095,
with byte-exact target encoding. A decoder without feature support rejects it; no
special filename or FAT pseudo-link encoding exists.

## Segment state, summaries and free accounting

Segment-state table record is `segment_index:u64, allocation_epoch:u64, state:u16,
reserved0:u16, live_blocks:u32, next_hint:u32, bitmap_bytes:u32,
summary:BlockRef24, reserved1:bytes[8]` (64 bytes), followed by bitmap and zero pad to8.
Bitmap bytes=ceil(blocks_per_segment/8); low bit represents low offset. Bits represent
THIS checkpoint's owned blocks, including data, nodes, tables and summaries. Outside
geometry bits zero. Popcount equals live_blocks; next_hint<=blocks_per_segment and
is advisory, never allocation authority. state on disk:0 FREE,1 OPEN,2 CLOSED,
3 RETIRED; CLEANING/QUARANTINED are runtime states only. FREE requires no current
bits and summary absent; fallback may still pin the segment, so the allocator cannot
use FREE alone. Nonempty segment summary must be present, with exact epoch/owners.
Epoch0 denotes never allocated; transition from never-used FREE assigns1. Reuse of
an emptied segment increments epoch, but only after all CP and runtime pins vanish.

Summary type8 payload: `segment_index:u64, allocation_epoch:u64, ordinal:u32,
entry_count:u32, next:BlockRef24` (48 bytes), followed by entries:
`block_offset:u32, kind:u16, reserved:u16, owner:NodeId16,
logical_key:u64, content_generation:u64, expected_crc64:u64` (48 bytes).
At most82 entries/page. Chain length is exactly ceil(live_blocks/82), bounded by7
for512-block geometry, no cycles or duplicate offsets. Last next is zero. Node/directory/extent metadata owner is its NodeId; global table and summary
metadata owner is zero NodeId (an explicit internal-page exception). Metadata
logical_key packs page type in bits63..48, table kind in47..32 (zero for non-table
pages), and zero-based page ordinal in31..0. Root and node pages use ordinal0;
summary ordinal is within its described segment. Together with segment index/epoch
and physical offset these are cleaning hints, not unique global authority. Data owner
is the file NodeId and logical_key is its logical block. Metadata content_generation
equals the page header birth_checkpoint; data content_generation matches its extent. Current summary pages and
segment table pages themselves are included in owned metadata. Summary entries for
metadata have expected_crc64=0 to avoid self-checksum cycles; their actual CRC is
verified through authoritative root/next references, and all pages must be reachable
through those references. For data, expected_crc64 equals extent checksum. Summary
contents never authorize a block that is absent from the authoritative graph.

Maximum segment count is bounded by82*floor(3976/segment_record_bytes), additionally
by SB limits and layout feasibility. For256-block segments, record96 gives3380
segments, roughly3.3 GiB gross before reserves. Runtime liveness union bitset at
that bound is108160 bytes per snapshot. Table arrays and additional visited sets
need separately admitted memory. No persistent global mutable free list exists.
CP counts and segment bitmaps are checked by graph traversal; fallback union and
runtime reservations determine actual free space. Free-space reconstruction is
read-only inspection, not automatic media repair.

## Orphan table and crash implications

Orphan record (32 bytes): `id:NodeId16, unlink_generation:u64,
reason:u16, reserved:bytes[6]`; reason1=unlinked-open. Sorted unique nid; unlink
generation<=CP; mapped node nlink=0, unreachable from root, and all its backing is
still owned. Runtime open counts do not survive a crash. Recovery may only remove
these records and nodes through a new consistent commit after validation and writable
admission; no mount-time in-place cleanup. Empty table still has a root page.

All structures are immutable while referenced by any valid checkpoint. Fixed CP
slots alone are overwritten, after durable invalidation of the slot being reused.
An invalid slot is all4096 zero bytes, flushed before reuse; a torn invalidation
may leave the old CP and therefore its block ownership intact. Detailed classification,
barriers and conservative corruption handling are normative in the recovery contract.
