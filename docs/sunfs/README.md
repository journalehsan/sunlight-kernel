# SunFS — Initial Architecture Proposal

**Document version:** 0.1 — Draft

**Status:** Proposal for review; the on-disk format is not yet finalized.

## 1. Mission

SunFS is SunlightOS's native filesystem. It draws inspiration from the
segment-based, log-structured design of F2FS, prioritizing data correctness,
predictable recovery, and incremental development.

Priority order:

```text
Correctness → Recoverability → Simplicity → Performance → Features
```

In this proposal, SunFS is not compatible with the F2FS on-disk format. We are
borrowing its ideas, rather than necessarily building an F2FS-compatible driver.
This decision gives us design freedom, but also makes us responsible for our own
formatting, checking, and recovery tools.

## 2. Scope of the First Version

### In scope

- One filesystem on one block device.
- Files, directories, and symbolic links.
- Stable inode/node identifiers.
- Segment-based allocation.
- Out-of-place writes for modified data and nodes.
- Checkpoints and recovery after an unexpected shutdown.
- Checksums for critical metadata structures.
- Simple but correct garbage collection (GC).
- Tools to create, inspect, and check the filesystem.
- Automated crash and I/O fault tests.

### Out of scope for the first version

- Native snapshots and block sharing between versions.
- Multiple disks, RAID, and storage pools.
- In-filesystem deduplication.
- Native encryption.
- A persistent write-back cache.
- Compression, until the storage and recovery paths are stable.

LZ4 has not been dropped; it simply should not be introduced alongside the first
recovery implementation. We will prepare the format for its addition, but first
make the uncompressed version correct.

## 3. Responsibility Boundaries

```text
Applications
    │
SunlightOS VFS / File Interface
    │
SunFS Engine
    ├── Inode & Directory Management
    ├── Node Mapping
    ├── Segment Allocator
    ├── Checkpoint / Recovery
    ├── Integrity Checks
    └── Cleaner
    │
Block Device Interface
```

Supporting services:

```text
GC Policy Service ── Requests cleaning with a specified budget
Prefetch Service  ── Prefetches frequently accessed data
Backup Service    ── Provides versioned backups
```

These are logical boundaries. We are not yet deciding exactly which process or
kernel component will run each one.

Only the SunFS Engine owns block relocation. The GC service proposes when and how
aggressively to clean; it does not modify on-disk structures directly.

## 4. Initial Storage Model

### Core units

| Concept | Role |
| --- | --- |
| Block | Basic filesystem addressing unit |
| Segment | A collection of blocks used for allocation and cleaning |
| Node ID | A stable node identifier, independent of its physical location |
| Node Map | Maps a node identifier to its current location |
| Segment State | Allocation information, live blocks, and segment status |
| Checkpoint | The publication point for a consistent filesystem state |

The initial proposal for experimentation is a 4 KiB block size; this is not a
final decision. We will choose the segment size after simulation and benchmarking.

### Conceptual layout

```text
┌───────────────────────────────┐
│ Superblock Copies             │
├───────────────────────────────┤
│ Checkpoint Slots              │
├───────────────────────────────┤
│ Versioned Mapping / Metadata  │
├───────────────────────────────┤
│ Segment Area                  │
│  ├── File Data                │
│  ├── Nodes                    │
│  └── Segment Summaries        │
└───────────────────────────────┘
```

The Node Map and allocation information must also be crash-safe. Writing data to
new locations is not enough if we corrupt its address table in place.

The precise versioning mechanism for these tables will be one of the main
decisions in the next document.

## 5. Correctness Contract: Non-Negotiable Rules

### Rule 1: A published checkpoint must be complete

Every valid checkpoint must reference a consistent set of structures. All
structures it requires must be durable before the checkpoint is published.

### Rule 2: Data needed for recovery must not be freed prematurely

Blocks still needed by a valid recovery path must not be reused.

Having two checkpoints alone is not enough: if the older checkpoint is intended
as a fallback, the blocks it requires must also be preserved. The policy for
checkpoint retirement and block reclamation must be explicit.

### Rule 3: Successful `fsync` means a real durability commitment

For the initial version, the proposal is:

> Implement `fsync` as a full commit, even if it is slower than an optimized path.

The data and metadata needed to recover the file's contents must be durable
before success is reported. The durability of name and path changes, such as
creation or rename, must also be covered by an explicit directory synchronization
contract.

### Rule 4: Do not assume arbitrary writes are atomic

The design assumes that the following can occur:

- Partial writes or torn writes.
- Write reordering by lower layers.
- Power loss while data remains in the device cache.
- Read or write errors.

Durability guarantees depend on correct support for flush and, if used, Force
Unit Access (FUA) throughout the I/O path.

### Rule 5: Do not hide corruption

Checksums detect corruption; they do not promise repair. If no healthy copy
exists, an error must be reported instead of returning suspect data as correct.

## 6. Commit and Recovery Cycle

Proposed conceptual cycle:

```text
1. Prepare changes.
2. Write new data and nodes.
3. Write a consistent version of mappings and allocation information.
4. Make dependencies durable.
5. Write the new checkpoint.
6. Make the checkpoint durable.
7. Report success and retire old states according to policy.
```

This sequence does not replace a precise persistence algorithm; that must be
specified according to the block device contract.

At mount time:

1. Validate the superblocks.
2. Validate checkpoint structure, checksums, and generations.
3. Check the consistency of required metadata.
4. Select the newest recoverable state.
5. Reject a writable mount if safe recovery is not possible.

The first version has no roll-forward recovery. The durable state is the last
complete commit, so `fsync` must perform the necessary commit as well.

## 7. GC: Internal Mechanism, External Policy

The first cleaner will deliberately be simple:

1. Select a segment with a lower proportion of live blocks.
2. Verify block liveness against the valid mapping.
3. Relocate live blocks.
4. Record the new mapping through the normal commit path.
5. Free the segment only after relocation is safe.

The execution policy has three modes:

| Condition | Behavior |
| --- | --- |
| Plenty of space and an idle system | Gentle cleaning with a limited budget |
| Declining number of free segments | Gradually increase cleaning |
| Critical pressure | Foreground cleaning and, if necessary, write throttling |

The cleaner's emergency reserve is separate from space available to users.
Thresholds will be determined through testing; we will not choose magic numbers
for them now.

## 8. Future Capabilities

### LZ4

The format must be able to record:

- Encoding type or algorithm.
- Logical length and stored length.
- Information needed for validation.
- Structure version.

Compression will operate on independent clusters. If it does not provide enough
savings, the data will be stored uncompressed.

### Intelligent prefetching

The first stage is to record access patterns and prefetch into RAM. A dedicated
on-disk cache area will be added only after measurements demonstrate its benefit.

### Versioned backups

An independent service providing:

- A selectable destination on the same disk or another disk.
- A configurable quota.
- A version retention policy.
- Integrity checks and restore testing.
- Coordination with applications for active data.

We will choose between chunking/deduplication and xdelta in a separate backup
design document. This decision must not hold up the base SunFS format.

## 9. Implementation Roadmap

### Phase A — Storage laboratory

Before integrating with the operating system, build a hosted environment that
works on a disk image and can simulate:

- Interruption after every write.
- Partial writes.
- I/O errors.
- The distinction between volatile and durable data.
- Permitted persistence orderings.

### Phase B — A small vertical slice

```text
format
  → mount
  → create
  → write
  → fsync
  → simulated crash
  → remount
  → verify
```

### Phase C — Deletion and GC

Test overwrites, truncation, file deletion, a full disk, and crashes during block
relocation.

### Phase D — SunlightOS integration

Once the engine can be tested, connect it to the VFS and real drivers.

### Phase E — Additional capabilities

LZ4, better GC policies, hot/cold separation, prefetching, and the backup service.

## SunFS's First Success Criterion

Our first milestone is not speed. It is this:

> Create a file, write to it, call `fsync`, simulate power loss at different
> points, and read the acknowledged data intact after remounting—without double
> allocation of blocks or damage to other files.

That will be SunFS's first solid foundation. We start with this small cycle,
rather than a super-cache and ten features at once. First, keep the sun shining;
then make it brighter! ☀️
