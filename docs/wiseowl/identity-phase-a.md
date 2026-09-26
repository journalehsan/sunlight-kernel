# Wise Owl Persistent Identity — Phase A

Phase A gives one Wise Owl installation a durable, hardware-independent identity
root and genesis lineage. `wiseowl-identity` owns only identity data and codecs;
`wiseowl-memorydb` owns persistence and startup policy. No external IPC can
mutate identity.

## Storage boundary and layout

Wise Owl depends on `/state/<service-name>/` and the SunlightOS durability API.
The state volume may be a virtual disk or physical storage; its format, device
serial, and backing technology are not identity inputs.

```text
immutable/replaceable SunlightOS image
            |
            v
   wiseowl-memorydb runtime
            |
            v
    /state durability API
            |
            v
    persistent state volume
            |
            +-- IDENTITY/
            +-- MemoryDB state
```

Native layout:

```text
/state/wiseowl-memorydb/
    IDENTITY/ROOT
    IDENTITY/LINEAGE
    IDENTITY/HEAD
    MANIFEST
    WAL/
    SEGMENTS/
    INDEX/
    ACTION_RECEIPTS/
    ...
```

Identity is adjacent to MemoryDB, never a database record or part of its
manifest, WAL, segment, index, or record formats.

## Native durability and publication

Before reading or generating identity state, startup verifies the service
directory with `sunlight_libc::dir_sync("/state/wiseowl-memorydb")`. If it is
absent on a fresh state volume, it creates the directory and syncs it. Native
FAT sync flushes every dirty sector on that volume, including the parent entry,
before the device barrier. RamFS rejects the sync, so a missing state disk
produces an explicit persistence
unavailable state before any staged identity or entropy request.

The native sequence uses:

1. exclusive file creation and `sunlight_libc::write`;
2. `sunlight_libc::file_sync(fd)` (syscall 153);
3. `sunlight_libc::rename_no_replace(old, new)` (syscall 149);
4. `sunlight_libc::dir_sync(path)` (syscall 154).

FAT sync flushes dirty block-cache sectors, then issues the negotiated VirtIO
`VIRTIO_BLK_T_FLUSH` stable-media barrier. Sync fails if that barrier is not
available. RamFS file and directory sync return unsupported.

FAT directory publication is destination-first rather than a journaled atomic
rename: it creates the destination directory link, flushes, removes the stage
link, and flushes again. A stop during publication may leave both names. The
only commit point is successful return from the subsequent sync of
`/state/wiseowl-memorydb` after publication. Before that barrier the candidate
may remain staged or be finalized on restart; after it, the committed identity
is authoritative. A valid committed set wins over an identical stale stage.
A distinct valid staged set is ambiguity and fails closed.

## Durable identity formats

All formats have bounded exact-length little-endian encodings, explicit magic
and version, reserved-byte validation, nonzero identifier checks, and SHA-256
integrity hashes. Unknown versions and trailing bytes are rejected. Hashes are
integrity checks, not signatures.

| File | Length | Contents |
| --- | ---: | --- |
| `ROOT` | 116 bytes | magic, version, `IdentityId`, creation event ID/kind, reserved bytes, checksum |
| `LINEAGE` | 164 bytes | identity binding, sequence 1, continuity generation 1, event ID/kind, zero genesis predecessor, record hash |
| `HEAD` | 124 bytes | identity binding, sequence/generation, committed lineage digest, checksum |

Phase A genesis kinds are `Created` and `ExistingStateAdopted`. A valid identity
requires all three files to validate and agree. `IdentityId` is a nonzero random
256-bit value obtained from the native secure entropy API; it is not derived
from hardware, storage, OS image, users, sessions, or MemoryDB generations.
Debug output and logs show only an eight-hex-character fingerprint.

## Creation, adoption, and recovery

The service singleton plus exclusive `IDENTITY.STAGE` directory creation
serializes creators. The parent directory is synced after creating the stage
and before generating an ID, so a subsequent durable `ROOT` cannot be stranded
behind a lost stage-directory entry. Fresh stores use `Created`. Conservatively detected,
read-only-validated pre-identity stores use `ExistingStateAdopted` by default.
Adoption does not rewrite existing records, IDs, ownership, provenance,
segments, manifest, or indexes.

Creation writes and syncs `ROOT`, `LINEAGE`, and `HEAD`, syncs the staging
directory, publishes with no-replace rename, then syncs the parent directory.
Once a valid staged `ROOT` exists, recovery reuses its identity and genesis
choice. Multiple stage candidates, a conflicting valid stage, corrupt committed
identity, or corrupt identity evidence beside existing/uncertain database state
fails closed. A corrupt stage on a genuinely fresh store is preserved under
`IDENTITY.REJECTED`; retry is allowed only when no valid generated ID remains.

MemoryDB validates or recovers identity before opening its database, binds the
read-only `LoadedIdentity` context to the runtime database, and only then
registers normal readiness. Validation failure suspends MemoryDB without
rewriting identity or database data; SunlightOS continues booting.

## Tests and evidence

Host tests cover strict codecs, corruption, all creation fault boundaries,
staged recovery, conflicting candidates, adoption without durable-file changes,
service restart, database/index generation independence, and a simulated OS
image replacement over the same state directory.

The `wiseowl-identity-phase-a` QEMU gate uses one raw `SUNSTATE` image across
four VM starts: it injects a stop after synced staged `ROOT`, recovers and
commits that same root on the next boot, verifies the committed root after
another independent boot, then boots without the state disk and verifies
explicit persistence-unavailable behavior with no MemoryDB readiness. This
demonstrates survival across QEMU process stop/start with the same disk image;
it does not prove sudden power-loss behavior for physical devices or every host
cache configuration. ISO replacement is simulated deterministically by the
host storage contract test; two distinct ISO builds are not exercised by this
gate.

`/tmp` remains in RamFS. A non-`SUNSTATE` block volume retains the existing
`/boot` mount selection; the OS image itself boots independently of `/state`.

## Deferred work

Phase A does not implement backup/restore, migration, clone detection, witnesses,
signatures, encryption, device or recovery keys, robot/body binding, semantic
memory, per-record identity fields, UID redesign, public mutation APIs, or
multiple identities.
