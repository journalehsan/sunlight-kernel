# ADR 0002: SFS-1 executable experimental revision 0.2

Date: 2026-11-14. Status: accepted for the hosted laboratory only.

Keep the v0.2 byte layout and recovery contract unchanged. Codecs are experimental
family 0 revision 2, not a production compatibility promise. SB/CP/header versions,
reserved zeros, page tail zeros, exact payload lengths and CRC domain are strict.
Unknown incompatible bits are unsupported; readonly-compatible bits permit only
degraded read inspection. Compatible bits do not change the decoded structures.

Correct the prose segment-capacity typo: 256-block segments have 96-byte records,
41 records/page and 82 pages/root, hence **3362**, not 3380 segments. The SFS-0
arithmetic evidence already uses 3362; preserve its original sources and evidence.

SFS-1 bounds nodes to 6724, table pages to 82 and extents/node to 6724. Geometry
accepts 512..4096 power-of-two logical sectors dividing 4096 and 64/256/512 blocks
per segment. Three CP slots remain fixed. Metadata lives in the segment area;
there is no separate mutable metadata partition. Capacity includes only complete
segments; sector-aligned trailing bytes are excluded from volume_blocks.

Precompute bootstrap ownership including all table roots/pages and summary pages.
Iterate summary count to a fixed point before encoding or I/O. Summaries use zero
expected CRC for metadata, avoiding reference cycles as specified by v0.2.
Empty directory and orphan roots are real pages with no child pages. Bootstrap
pages are born at generation 1 and shared by CP1 and CP2. CP2 predecessor is 1.

Reserve policy is deliberately conservative: one full segment victim, three
bounded maximum metadata closures (relocation plus two generations' growth),
and three fixed publication writes. The metadata bound uses configured maximum
nodes/orphans, all segment records, empty namespace bootstrap roots, and summary
fixed-point overhead. It is an SFS-1 admission budget, not a cleaner progress proof
for future arbitrary file/namespace transactions. SFS-2 must admit each actual
transaction's complete dependency plan; SFS-4 must qualify relocation progress.

The formatter creates only a new file in a canonical, pinned directory. Existing
files, symlinks, ancestor aliases and nonregular targets are refused. No overwrite
option exists. Hosted Linux file safety uses a pinned directory descriptor and
create_new; the core has no host APIs. UUIDs come from the host random source;
tests inject fixed UUIDs and schedules. The simulator uses an explicit event stream,
never time or threads. Successful flush drains pending writes in submission order;
background reordering preserves same-sector submission order. A separately marked
dishonest mode may drop flushed writes and is excluded from correctness verdicts.

Checker status CLEAN means two or three complete generations, matching SBs and no
malformed nonempty slot. It does not authorize native writable mounting: driver
durability, exclusivity and quiescence are still unqualified. One valid generation
or any damaged candidate with a valid fallback is DEGRADED_RECOVERABLE, inspection
only. No valid closure is CORRUPT; unknown required format is UNSUPPORTED; conflicting
valid SBs/generations or inability to read within the configured budget is
INDETERMINATE. Exit codes are 0, 1, 2, 3, 4 respectively. No checker repairs.
