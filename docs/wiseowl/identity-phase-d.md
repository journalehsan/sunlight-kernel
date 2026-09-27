# Wise Owl persistent identity Phase D: passive backup

## Status

Phase D.1 is **in progress**. The host library creates and inspects passive V2
packages when a trusted caller supplies a frozen Wise Owl KV export. There is
no native cross-service barrier, creation path, or reboot evidence. V2 is not
yet verified recovery material. There is no restore or activation code.

Public backup creation is intentionally deferred until caller authentication
is adequate. The current host MemoryDB socket grants every peer admin
capability; it exposes read-only `backup inspect`, but no create request.

## V1 continuity state classification

| State | Class | Treatment |
| --- | --- | --- |
| ROOT, LINEAGE, HEAD | REQUIRED | Exact bytes and identity binding in package. |
| MemoryDB MANIFEST, WAL, sealed segments, relationship snapshot | REQUIRED | Checkpointed, hashed, decoded read-only during inspection. |
| Brain MTM KV `vc`, `gen`, `lp`, `gs`, `sms`, `sis` | REQUIRED | Typed, bounded V2 KV component. Trusted source must enumerate every current key while frozen. |
| Token and source indexes | REBUILDABLE | Recreated from records and provenance. |
| Indexd state and prepared imports | REBUILDABLE | Omitted. Roots, path/inode/device hints, scan cache, and uncommitted preparation are lost. Reindexing requires external documents at usable locations. Committed records, owners, provenance, and source references survive in MemoryDB. |
| Sealed action receipts | OPTIONAL | Declared absent; native durable wiring is incomplete. They do not define memory or personality identity. |
| Active receipt builders, unsealed actions | EXCLUDED | Runtime state. |
| LOCAL, writer lock, boot epoch, endpoint, sessions | EXCLUDED | Never copied as future authority. |
| Memoryd short-term spill and `owl.v1.shortterm.<session>.<memory>` KV promotions | EXCLUDED | Session-bound short-term material. Native memoryd uses this namespace; custom promotion namespaces are not approved for export. |
| External indexed documents | EXCLUDED | Indexed records and provenance survive, original file bytes do not. |
| Foundation Memory | REBUILDABLE | Versioned content ships with the service binary. |

Brain defines exactly six `wb1:<uid-hex>:<kind>` kinds: visit count (`vc`),
last completed onboarding generation (`gen`), last provider (`lp`), greeting
style (`gs`), show machine summary (`sms`), and show index status (`sis`). Brain
writes `vc/gen/lp` through welcome state and `gs/sms/sis` through preferences.
Memoryd separately writes session-bound `owl.v1.shortterm` KV promotions. No
arbitrary KV prefix or global sunlight-kv log is part of the format.

### Source audit details

Brain writes these keys through `save_welcome_state` and `save_preferences`,
called by the successful-response, onboarding-complete, preference-set, and
greeting paths. A preference update performs three separate KV puts; onboarding
can perform up to three. A correct barrier must cover each whole Brain
operation, not just individual KV requests. `format_user_key` narrows the
caller UID to `u32`; the export preserves that effective key owner and cannot
recover or disambiguate discarded upper UID bits. The encoder rejects malformed
or noncanonical keys and duplicate owner/kind entries instead of merging them.

Native memoryd promotion writes
`owl.v1.shortterm.<session>.<memory-id>` with `put_if_absent`; the native
endpoint selects that fixed namespace. The general memory service also accepts
custom namespaces, which are not approved for continuity export. No native
sealed-receipt writer was found. The backup barrier trait remains a host
contract/test seam; no native caller implements it.

The native KV daemon's scan reply returns at most three keys and has no cursor,
while Brain's native adapter exposes only individual get/put operations. It
cannot currently enumerate a complete bounded `wb1:` key set. The kernel also
retains one virtio block device and mounts it at `/state` or `/boot`; it has no
second-volume mount or native backup filesystem writer. The native exporter,
cross-service barrier, creation path, independent-volume tests, and native
interruption tests therefore remain unimplemented. These are implementation
blockers, not verified native behavior.

## Package V2 and scoped KV

Manifest V1 was a host prototype with KV optional and absent. It is
**unsupported** as complete continuity material. Manifest V2 requires the KV
component and retains purpose `Backup`, `activation_allowed=false`, BackupId,
identity binding, bounded descriptors, and SHA-256 per present component.
Optional absent indexd and sealed-receipt descriptors remain explicit. The
passive guard blocks normal active-store detection of packaged MemoryDB.

The KV component has magic `WOKVSNP2`, schema 1, explicit historical 32-bit
owner encoding, BackupId, IdentityId, count at most 4096, and sorted entries.
Each entry stores effective owner `u32`, enumerated kind `1..6`, value schema
1, and at most 64 value bytes. The manifest hashes the whole component. Malformed, duplicate,
noncanonical, oversized, and unknown Wise Owl keys fail encoding. This
preserves the **effective historical owner**, not a lost high UID half. Future
recovery must detect ambiguous ownership; no remapping occurs here.

The creator requires a `WiseOwlBackupBarrier`. `begin` must stop new relevant
durable operations and drain in-flight ones; `export_kv` must enumerate the
approved keys while frozen; `end` thaws writers on every normal failure path.
MemoryDB is exclusively borrowed, rejects open transactions, checkpoints,
captures identity, exports KV, and copies required files while the barrier is
held. A host test double verifies this contract and thaw after export failure.
**No native Brain, memoryd, indexd, sunlight-kv, and MemoryDB coordination
implementation exists yet.** Native writes can still straddle stores. KV
unavailability fails creation; there is no memory-only backup class.

Checkpointing may change physical MemoryDB MANIFEST, WAL, segments, and index
snapshot. ROOT, LINEAGE, HEAD, and LOCAL must stay byte-identical. IdentityId,
InstallationId, ActivationId within a live backup, lineage sequence, and
continuity generation remain unchanged. No lineage event is created.

## Publication and inspection

The host creator syncs private staging components and manifest, syncs staging
directories, renames without replacement, **syncs the destination parent**,
writes a bounded `COMMITTED` seal, syncs that file, and syncs the final package
directory. The seal contains magic/version, purpose, BackupId, and manifest
digest. The intended commit point is successful final package directory
`sync_all`. Live host inspectors share a Linux directory lock with the creator
and cannot report committed while the creator is between seal creation and
that sync. Ordinary pre-commit errors remove the seal.

Offline inspection reads package bytes only. It requires no active MemoryDB,
LOCAL, InstallationId, ActivationId, or session. It rejects V1, missing or
corrupt required components, extra files, symlinks, identity mismatch, and
invalid MemoryDB contents. The status API distinguishes valid committed,
valid uncommitted, corrupt, and unsupported packages.

**Crash-model limit:** `fsync` completion is not encoded in package bytes. If
a VM stops after seal file sync but before final directory sync, the seal may
survive. An offline byte-only validator cannot prove whether that last sync
completed. The strict native A-D interruption condition and universal
pre-commit rejection property remain unverified. The host lock establishes the
concurrent *live-process* boundary only. No physical sudden-power-loss claim
is made. Publication needs a resolved durable contract before Phase D can be
called complete.

Destination directories must be private (`0700`); files are `0600`.
Inspection prints fingerprints and metadata, never memory payloads or full
IdentityId. SHA-256 detects corruption, not authorization. **Backups are not
encrypted** and contain personal memories and preferences.

## Native storage limit

The current kernel keeps one virtio block device in `VIRTIO_BLK` and mounts
its FAT volume at `/state` (label `SUNSTATE`) or `/boot`. It cannot
simultaneously mount a second backup volume. The host-only backup module uses
`std::fs` and is not compiled into native no-std MemoryDB. A native test-only
trigger, scoped KV export, independent `/backup-test` volume, backup-only boot,
reattachment, and native interruption/reboot gates remain outstanding. A
backup directory is not scanned by active identity startup, and no activation
path exists; this has not been exercised with a native backup disk.

## Verification matrix

| Behavior | Implemented | Host verified | Native verified | Limitations |
| --- | --- | --- | --- | --- |
| V2 typed required Brain KV component | Host | Yes | No | Native trusted exporter absent. |
| Cross-store mutation freeze | Contract only | Test double | No | No participating native services. |
| Memoryd promotion, index import, and Brain write races | No | No | No | Native services do not join one barrier. |
| MemoryDB checkpoint and passive identity copy | Host | Yes | No | No native creator. |
| V1 rejection and four inspection statuses | Host | Yes | No | V1 is unsupported as complete backup. |
| Parent sync before commit seal | Host | Yes | No | Final dir sync has crash ambiguity. |
| Concurrent validator at all 22 publication boundaries | Host | Yes | No | Marker-visible validation waits for final directory sync; process lock is not crash evidence. |
| Publication fault boundaries | Host | Yes, 22 points | No | VM crash differs from injected error cleanup. |
| Offline read-only validation | Host | Yes | No | No native package yet. |
| Independent-volume reboot survival | No | No | No | Kernel mounts one block volume. |
| Backup-only boot stays inactive | Structural only | No | No | Native second-volume gate absent. |
| Phase A/B/C regression gates | Existing | Yes | Yes | All three passed on 2026-09-27; these gates do not test Phase D creation. |

All 86 MemoryDB library tests and 7 MemoryDB binary tests passed. The native
Phase A, B, and C QEMU gates were rerun and passed on 2026-09-27. Phase A boot
evidence is retained under `target/wiseowl-identity-phase-a-evidence/`.

A deterministic host fixture with 64 records, one relationship, eight sealed
segments, and representative KV values produced 135,705 component bytes.
On this development run: durable-write freeze 2,808 µs; checkpoint 25 µs;
component read/write copy 63 µs; measured SHA-256 work including copy
verification 211 µs; total 2,809 µs. These are development observations on
the host filesystem, not native or sudden-power-loss measurements.

Phase E catastrophic recovery is deliberately absent. V2 packages should not
be consumed by recovery until native barrier, crash, volume, reboot, and
authorization properties are verified.
