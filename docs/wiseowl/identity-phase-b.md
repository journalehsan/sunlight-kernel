# Wise Owl Persistent Identity — Phase B

## Authority and protocol

`wiseowl-memorydb` remains the sole persistent identity authority. It validates
ROOT, LINEAGE, and HEAD during startup, derives `LoadedIdentity` from that
validated set, and binds it before publishing the database endpoint. No
consumer can write these files or submit an IdentityId for MemoryDB to trust.
Existing MemoryDB records and their owner/user semantics are unchanged.

MemoryDB exposes `GetIdentityStatus` as a read-only operation. The sanitized
wire object is exactly 36 bytes: `WIDS`, protocol version, state, persistence
flag, 8-byte diagnostic fingerprint, lineage sequence, continuity generation,
genesis kind, identity format version, and validation status. The decoder
rejects unknown versions/states, wrong lengths, and invalid state/field
combinations. The native IPC form is a fixed five-word response containing the
same public fields. The host development IPC mirrors the typed result through
its existing bincode request/response layer. Status requests do not modify the
database.

Native IPC currently uses fixed/admin caller metadata in some paths and does
not securely distinguish trusted Wise Owl services from ordinary diagnostics.
For that reason the protocol returns only the short fingerprint, never the full
IdentityId. The host CLI `wiseowl-memorydbctl identity` displays the sanitized
status. If persistent storage or identity validation fails, Phase A currently
withholds the MemoryDB endpoint; consumers see a disconnected/unavailable
status. The status enum reserves distinct failure states for future service
manager reporting, but this version does not keep a failed MemoryDB endpoint
alive to return them.

## Service binding

```text
          validated ROOT/LINEAGE/HEAD
                       |
                       v
                wiseowl-memorydb
                       |
             read-only identity status
                /          |          \
               v           v           v
          wiseowl-     wiseowl-    wiseowl-
           braind       indexd      memoryd
             |
             v
       GUI / applications
```

The status cache is runtime-only. Braind queries MemoryDB status with its
status snapshot and has explicit `Personalized`, `GenericDegraded`, and
`SuspendedMismatch` runtime modes. On unavailable or invalid status it stays in
generic behavior; it does not infer identity from UID, session, Foundation
Memory, or a replacement runtime. The existing session authority remains
separate and still decides which current user/session may interact with the
brain. A valid persistent identity does not grant GUI authority.

Indexd requires a valid status before starting a durable scan/import or
reconciling pending durable imports. On outage it pauses new commits and keeps
its reconstructable state. It pins the first fingerprint for its process
lifetime; a changed fingerprint after reconnect sets a suspended/degraded
condition and blocks identity-bound work. It does not delete indexed data due
to the outage.

Memoryd checks the sanitized MemoryDB status before durable KV promotion. RAM
short-term operation remains available. It pins only the fingerprint in its
current process, refuses promotion while MemoryDB is unavailable, resumes when
the same status returns, and remains blocked after detecting a changed
fingerprint. It does not generate an IdentityId. This does not alter cold spill.

Every service must rediscover/requery after a MemoryDB disconnect; a cached
status is never durable authority. Consumer restart reloads status from
MemoryDB. A same-identity reconnect resumes allowed behavior. A different
identity is a fail-closed mismatch, not an implicit migration.

## Identity boundaries

Persistent identity answers which Wise Owl installation this is. Session
authority answers which user/session is interacting now. Logout has no effect
on persistent identity. Foundation Memory remains software-provided generic
knowledge and is not identity; changing Foundation Memory or the OS image must
not change IdentityId. No identity value is added to existing MemoryDB records,
and ContinuityGeneration remains 1 throughout Phase B.

## Current limitations and Phase C boundary

The native status caller path is sanitized because caller trust is not yet
reliably distinguished. MemoryDB startup failures are observed as endpoint
unavailability rather than distinct live `IdentityInvalid` or
`PersistenceUnavailable` responses. Braind and indexd use status on their
existing request/reconnect paths; the Phase A QEMU identity gate still
exercises MemoryDB multi-boot persistence and does not restart the service
manager stack inside the guest. Native memoryd promotion gating is implemented,
but full multi-service restart orchestration is not available in this gate.

Phase C may add local activation and single-writer enforcement. It must define
the authorization and recovery semantics before adding mutation, migration,
private key material, or robot/body semantics. Phase B introduces none of
those workflows.
