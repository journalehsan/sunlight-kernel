# Wise Owl Persistent Identity Phase A

Status: implemented foundation; migration, recovery, backup, and multi-device
activation are deliberately outside this phase.

## Purpose

Phase A gives one Wise Owl installation a hardware-independent, 256-bit
`IdentityId` exactly once. `wiseowl-memorydb` validates or recovers that identity
before opening MemoryDB and before publishing service readiness. Reboot,
MemoryDB restart, database generation changes, and ordinary software upgrades do
not change it.

The identity contracts live in the small `no_std`-capable
`wiseowl-identity` crate. `wiseowl-memorydb` is the sole persistence owner. No
public client IPC can create or mutate identity state.

## Persistent layout

The native authoritative layout is adjacent to, not inside, MemoryDB:

```text
/state/wiseowl-memorydb/
    IDENTITY/
        ROOT
        LINEAGE
        HEAD
    MANIFEST
    WAL/
    SEGMENTS/
    INDEX/
    ACTION_RECEIPTS/
    ...
```

Creation uses one `IDENTITY.STAGE` directory. Invalid staging on a genuinely
fresh store may be preserved as `IDENTITY.REJECTED`. Additional directories
whose names begin with `IDENTITY.STAGE` are treated as competing candidates;
startup fails closed instead of selecting or deleting one.

Identity is not a MemoryDB record, is not embedded in `MANIFEST`, and does not
change existing record, WAL, segment, index, memory-ID, owner, or session
formats.

## Durable format

All values use explicit, fixed-width, little-endian codecs. Decoders require an
exact length, validate magic and format version, reject nonzero reserved bytes,
reject zero identifiers, and reject unknown event kinds. No host-only `bincode`
encoding is used.

| File | Size | Contents |
|---|---:|---|
| `ROOT` | 116 bytes | Magic, format version, encoded length, `IdentityId`, creation event ID, creation event kind, reserved version metadata, SHA-256 integrity checksum |
| `LINEAGE` | 164 bytes | Magic, format version, length, identity binding, sequence, continuity generation, event ID/kind, zero genesis predecessor, SHA-256 record hash |
| `HEAD` | 124 bytes | Magic, format version, length, identity binding, committed sequence/generation/hash, SHA-256 integrity checksum |

Phase A supports exactly two genesis kinds: `Created` and
`ExistingStateAdopted`. Lineage sequence and continuity generation must both be
1. `ROOT`, `LINEAGE`, and `HEAD` must bind the same identity and creation event,
and `HEAD` must resolve to the validated lineage record.

The hashes are integrity checks, not signatures or an authentication scheme.

## State volume and durability contract

Native `/state` is a dedicated FAT32 volume labeled `SUNSTATE`, backed by a
persistent VirtIO block device. `tools/state-disk.sh` creates the image used by
development and QEMU gates. A volume with that label mounts at `/state`; the
boot volume remains `/boot`.

The block stack now exposes write and flush operations. VirtIO block flush uses
the negotiated `VIRTIO_BLK_T_FLUSH` operation. Identity publication refuses to
claim durability if the filesystem or block device cannot honor file and
directory sync requests. RamFS reports sync as unsupported and therefore cannot
host an authoritative identity.

SunlightOS FAT does not provide a journaled, single-sector POSIX directory
rename. Its strongest available publication operation is destination-first:

1. Create the complete destination directory entry tree.
2. issue a device flush barrier;
3. remove the source directory entry tree;
4. issue another device flush barrier.

A power loss during that rename may leave both `IDENTITY.STAGE` and `IDENTITY`,
but never removes the only published name before the destination is durable.
Startup gives a valid committed `IDENTITY` precedence. Host builds use Linux
`renameat2(RENAME_NOREPLACE)`, followed by parent-directory `fsync`.

The identity is considered fully committed when publication has succeeded and
the containing `/state/wiseowl-memorydb` directory sync has returned success.
If power fails before that point, startup either completes the same valid staged
candidate or validates the already-visible committed candidate. If power fails
after that point, startup loads the committed identity. Once a valid staged
`ROOT` exists, no successful retry generates another `IdentityId`.

## Fresh creation

A store is fresh only when no committed identity exists and no meaningful or
unknown Wise Owl durable state is present. Absence of `MANIFEST` alone is not
proof of freshness. Known empty directories are harmless; content in durable
MemoryDB directories means existing state; unknown entries or interrupted
temporary content make the disposition uncertain and fail closed.

Creation proceeds as follows:

1. The service-manager singleton and exclusive `IDENTITY.STAGE` directory
   creation establish the sole creation candidate.
2. Obtain 32 bytes from SunlightOS `getrandom` (the rand service), reject the
   all-zero value, and construct `IdentityId`.
3. Generate a nonzero lineage event ID.
4. write and sync `ROOT`;
5. write and sync genesis `LINEAGE`;
6. write and sync `HEAD`;
7. sync the staging directory;
8. publish it as `IDENTITY` without replacing an existing destination;
9. sync the parent directory.

The identity never derives from UID, hostname, PID, session, hardware, path,
MemoryDB generation, or database content.

## Existing-installation adoption

Pre-identity data is adopted only after conservative state detection and
read-only MemoryDB validation. Adoption is explicitly gated:

- host: compile with `identity-adoption` and set
  `WISEOWL_IDENTITY_ADOPT_EXISTING=1`;
- native: compile with `identity-adoption`.

Normal native builds do not enable that feature. The dedicated Phase A QEMU
test build enables it explicitly so adoption can be exercised against a
pre-identity fixture without changing the production startup default.

The genesis kind is `ExistingStateAdopted`, meaning only that the durable
identity was formalized around pre-existing validated state. No historical
creation time is invented. Adoption does not rewrite MemoryDB records,
`MANIFEST`, WAL, segments, indexes, IDs, or owner fields, and does not require
an index rebuild.

## Staged recovery and failure modes

Startup handles identity state before opening MemoryDB:

- valid committed `IDENTITY`: load it;
- no committed identity and one complete valid stage: publish that same stage;
- one partial stage with a valid `ROOT`: finish it using the same identity and
  genesis choice;
- corrupt stage on a genuinely fresh store: preserve it as rejected, then allow
  one new creation attempt;
- corrupt or ambiguous evidence beside existing/uncertain Wise Owl state: fail
  closed;
- multiple staged candidates: fail closed without choosing, deleting, or
  generating a third identity;
- corrupt, incomplete, or unsupported committed identity: fail closed and
  never create a replacement.

Failure leaves SunlightOS bootable, but `wiseowl-memorydb` remains suspended and
does not register readiness. Diagnostics identify the bounded failure class and
log only the first four identity bytes as eight uppercase hexadecimal
characters. They do not log the full identifier or memory payloads.

## Tests and fault injection

Host tests cover strict ID/root/lineage/head codecs, corruption and truncation,
both genesis kinds, set disagreement, conservative store detection, adoption
without MemoryDB rewriting, committed corruption, atomic no-replace behavior,
ambiguous valid candidates, repeated restart, and every creation boundary:

1. before ID generation;
2. after ID generation but before `ROOT` write;
3. after `ROOT` write;
4. after `ROOT` sync;
5. after `LINEAGE` write;
6. after `LINEAGE` sync;
7. after `HEAD` write;
8. after `HEAD` sync;
9. before staging-directory sync;
10. after staging-directory sync;
11. before publication;
12. after publication;
13. before parent-directory sync;
14. after parent-directory sync.

Each recovery test restarts repeatedly and proves convergence to one valid
identity. When recoverable `ROOT` evidence exists, the final identifier must
match it.

The `wiseowl-identity-phase-a` QEMU gate verifies a writable persistent
`SUNSTATE` volume, native secure entropy, identity creation, validation, and
MemoryDB readiness. Reboot testing reuses the same disk image and compares the
`ROOT` bytes across boots.

## Enforced invariants

1. `IdentityId` is a nonzero, cryptographically random 256-bit value generated
   once.
2. Reboot and MemoryDB restart do not change identity.
3. Database generation, hardware, UID, host name, process, and session never
   define identity.
4. Existing Wise Owl data is not rewritten during adoption.
5. Recoverable staged identity evidence is reused.
6. Ambiguity and corruption fail closed.
7. Identity corruption never silently becomes identity replacement.
8. `ROOT`, `LINEAGE`, and `HEAD` must agree.
9. Identity is neither a MemoryDB record nor part of its `MANIFEST`.
10. Existing owner and session-authority semantics are unchanged.
11. Continuity generation and lineage sequence remain 1 in Phase A.
12. No external caller can mutate persistent identity.

## Intentionally not implemented

Phase A does not implement backup or restore, cross-device migration, active
clone detection, activation witnesses, signatures, encryption, identity or
device keys, recovery keys, body/robot integration, semantic search, per-record
identity fields, owner/UID redesign, general IPC authorization cleanup, action
receipt redesign, or unrelated audit repairs. Those require separately reviewed
phases built on this durable root.
