# Wise Owl Persistent Identity Architecture — Design Only

**Status:** Proposed architecture
**Scope:** Persistent identity foundation only
**Implementation status:** Not implemented

## 1. Problem statement

Wise Owl needs a durable artificial identity that remains the same across reboot, service restart, software upgrade, hardware replacement, machine migration, and eventual body replacement.

The identity must be distinct from:

- SunlightOS user and UID
- Graphical or console session
- Session authority and attestations
- Service process and process generation
- Device or hardware identity
- MemoryDB database generation
- MemoryDB storage path
- Runtime client identifiers
- Backup images
- Migration packages

This is an engineering continuity model. It makes no claim that Wise Owl is conscious.

One Wise Owl remains the same Wise Owl when:

1. It retains the same durable `IdentityId`.
2. Its current state is an authorized descendant of that identity’s committed lineage head.
3. Its identity-bound durable state is transferred or recovered according to explicit migration or recovery semantics.
4. No conflicting successor is known to be authoritative.

The identifier alone is necessary but not sufficient. An old backup contains the same historical identity identifier, but it is not automatically an authorized active continuation.

## 2. Current-state constraints

The following conclusions were verified against the current source tree.

### 2.1 Existing persistent state

- `wiseowl-memorydb` is the authoritative long-term memory service.
  - Native state lives under `/state/wiseowl-memorydb`.
  - Host state defaults to `/tmp/sunlight/wiseowl-memorydb`.
  - Its store contains `MANIFEST`, `WAL/`, `SEGMENTS/`, `INDEX/`, and related operational directories.
- MemoryDB records carry `owner`, `scope`, provenance, source information, relationships, payloads, and tokens.
- `OwnerId` is currently a raw `u64`; it must not be repurposed as a Wise Owl identity identifier.
- MemoryDB’s `database_generation` advances during process recovery/open. Its `index_generation` changes during index rebuild or compaction. Neither is persistent identity.
- `wiseowl-indexd` stores checksummed operational state at `/state/wiseowl-indexd/state.bin`.
  - Its own source calls this state medium-term and rebuildable.
  - It contains source manifests, source IDs, paths, file device/inode hints, import state, and pipeline versions.
- `wiseowl-memoryd` stores its restart-safe allocator generation under `/state/wiseowl-memoryd/generation.bin` and writes cold spill blobs under the same directory.
  - The native daemon currently writes spill blobs but does not enumerate and restore them at startup.
  - RAM working state is intentionally not durable.
- `wiseowl-braind` stores preferences and onboarding state in `sunlight-kv`, using UID-derived keys.
- The current MTM key formatter truncates a `u64` UID to `u32`.
- Foundation Memory is an immutable build-time blob compiled into `wiseowl-braind`.
  - Although current documentation calls it identity data, it is product/persona foundation data, not a unique persistent individual identity.
  - It can change with software releases and can be rebuilt from the installed software.
- Action receipt persistence types exist under MemoryDB’s `ACTION_RECEIPTS/` namespace, but current native runtime use is still volatile/test-oriented.
- Native Wise Owl IPC paths frequently construct fixed or administrative caller identities.
- Native `wiseowl-braind` receives a PID badge but not a trustworthy UID directly; several requests still carry UID in their body.
- Host Unix sockets do not currently provide a sufficient authenticated identity-management boundary.
- Filesystem policy permits each service to write only its matching `/state/<service-name>` or `/var/lib/<service-name>` area.

### 2.2 Architectural consequences

- Persistent identity cannot be based on any existing “generation” field.
- Existing record formats must remain unchanged in the first implementation.
- Existing record ownership must remain user/session access control, not be replaced by identity ownership.
- The identity owner needs stronger durability than the current native best-effort file publication paths provide.
- A standalone `/state/wiseowl` owner would require a new filesystem actor or daemon. That is unnecessary for V1.
- Personalized `wiseowl-braind` operation must become dependent on a valid active identity, but SunlightOS boot must not become dependent on it.

## 3. Identity model

### 3.1 Concepts

| Concept | Meaning |
|---|---|
| Persistent identity | The stable identifier, creation record, lineage, cryptographic authority metadata, and identity-bound durable corpus |
| OS user | A SunlightOS account accessing Wise Owl; represented by existing owner/user fields |
| User/session authority | Short-lived proof that a current user and session may issue a request or authorize an operation |
| Device identity | A local installation identifier or authorized device key; replaceable and revocable |
| Runtime/service instance | One process execution of MemoryDB, brain, indexer, or memory service |
| Memory store | A durable component containing memories; bound to an identity but not itself the identity |
| Backup | A passive snapshot that cannot enter normal active startup directly |
| Restore instance | A staged copy created from backup and awaiting explicit recovery activation |
| Migration operation | An intentional transfer of the authoritative active lineage |
| Clone | A copied state attempting to operate independently with the same identity |
| Fork | An explicit creation of a new independent identity derived from copied state |

### 3.2 What belongs to the identity

Core identity consists of:

- Stable `IdentityId`
- Immutable creation record
- Committed lineage history and lineage head
- Public cryptographic identity material or commitments
- Authorized key history
- Continuity generation
- Explicit migration and recovery events
- Retirement state, if ever explicitly set

The identity’s continuity corpus consists of:

- Long-term memories
- Relationships
- Durable provenance
- Personality preferences
- Identity history
- Intentionally retained learned state
- Other components explicitly included in a consistent identity snapshot

### 3.3 What does not belong to the identity

The following are not identity:

- UID, username, or group ID
- Session ID or session generation
- PID, IPC badge, process generation, or client ID
- Runtime request, conversation, action, or receipt ID
- Hostname
- CPU, motherboard, TPM, disk, MAC address, or serial number
- Laptop or robot chassis identifier
- Filesystem path
- MemoryDB database or index generation
- Memory, segment, source, or document identifier
- Foundation Memory hash or version
- Model, tokenizer, Wise Owl software, or SunlightOS version
- Backup, restore, or migration package ID
- A cryptographic key ID by itself

### 3.4 Immutable and evolving fields

Immutable:

- `IdentityId`
- Creation event ID
- Creation lineage record
- Original creation metadata
- Initial root-key commitment, once cryptographic identity is provisioned

Evolving:

- Lineage sequence and head
- Continuity generation
- Authorized and revoked device keys
- Active cryptographic suite and rotated keys
- Migration and recovery history
- Learned memories, relationships, preferences, and personality state
- Authorized installation and activation state

Encoding and schema versions may evolve without changing the identity.

## 4. Identifier model

### 4.1 Evaluated approaches

**Random 128-bit identifier**

- Collision probability is already extremely low.
- Compact and compatible with UUID-like tooling.
- It provides no authentication.

**Random 256-bit identifier**

- Negligible collision risk over Wise Owl’s expected lifetime.
- Independent of algorithms, hardware, accounts, and keys.
- Easy to encode in a small fixed-width type.

**Public-key-derived identifier**

- Provides a self-certifying naming model.
- Couples identity naming to a cryptographic algorithm and root key.
- Makes key compromise and algorithm migration harder.
- Encourages treating possession of one key as the entire identity.

**Stable random identifier plus key hierarchy**

- Separates stable naming from authentication.
- Allows device authorization, key rotation, recovery keys, and algorithm upgrades.
- Preserves identity if a key must be replaced.

### 4.2 Recommendation

Use:

- A cryptographically random 256-bit `IdentityId`, generated exactly once.
- A separate, versioned cryptographic key hierarchy.
- An identity authority key and authorized installation/device keys when cryptography is implemented.

The identity ID must not be derived from the public key. The public key authenticates lineage operations; it does not define the permanent name.

Recommended representations:

- In memory/on disk: `[u8; 32]`
- Human-facing: checksummed lowercase Base32 with a prefix such as `woid1_`
- Never log the complete ID by default; diagnostics may use a bounded fingerprint

The identifier is not secret.

## 5. Ownership model

### 5.1 Options

**A. MemoryDB owns identity directly**

This offers an existing durable service and suitable startup position, but placing identity inside the database engine or its `MANIFEST` would incorrectly couple identity to a database instance and database format.

**B. Brain owns identity**

Rejected. `wiseowl-braind` is an orchestration consumer, can degrade independently, contains build-time Foundation Memory, and should not have authority to rewrite identity lineage.

**C. A small identity subsystem/library owns the model**

Recommended, provided it is hosted by an existing service.

**D. Another SunlightOS service owns identity**

Rejected for V1. No existing service has a narrower or more appropriate responsibility, and a new daemon is unnecessary.

### 5.2 Recommended authority

Create a small, `no_std`-capable `wiseowl-identity` contracts and state-machine library. Do not create an identity daemon.

Host its authoritative persistence and mutation boundary inside `wiseowl-memorydb`:

- The identity library defines identifiers, codecs, validation, lineage, lifecycle, and package semantics.
- The `wiseowl-memorydb` process owns the identity files and exposes a narrow identity IPC surface.
- The normal MemoryDB database engine does not interpret identity as a memory record.
- Identity state is stored beside, not inside, existing MemoryDB formats.

This is logically option C and operationally uses the existing MemoryDB service.

### 5.3 Consumers and authority

| Component | Authority |
|---|---|
| Identity subsystem in `wiseowl-memorydb` | Sole writer of root, lineage, head, and local activation state |
| `wiseowl-braind` | Read-only identity status and ID fingerprint |
| `wiseowl-indexd` | Read-only active/bound status before durable imports |
| `wiseowl-memoryd` | Read-only status before identity-bound promotion |
| Console/GUI | Sanitized read-only status |
| Migration/recovery tooling | Requests narrow state-machine operations; never writes files directly |
| Session authority | Authorizes a present user, but cannot rewrite identity directly |

### 5.4 Startup and degraded behavior

MemoryDB startup order should eventually be:

1. Validate or recover the identity root.
2. Validate committed lineage and head.
3. Validate local activation state.
4. Open/recover the existing MemoryDB store.
5. Verify that the adjacent store is associated with the loaded identity.
6. Publish identity status.
7. Permit identity-bound durable mutations only when the identity is usable.

If identity is unavailable:

- SunlightOS continues booting.
- MemoryDB may expose read-only maintenance and diagnostics.
- `wiseowl-braind` may provide clearly generic Foundation-based responses.
- It must not claim personalized continuity or expose identity-bound memory.
- `wiseowl-indexd` must not commit new identity-bound generations.
- `wiseowl-memoryd` may operate in RAM-only mode but must not promote into identity-bound durable storage.

## 6. Identity/session/device separation

The authorization equation is:

```text
Valid active persistent identity
        +
Authenticated current OS user/session
        +
Authorized runtime/service instance
        =
Authorized active Wise Owl request
```

Each term answers a different question:

- Persistent identity: “Which long-lived Wise Owl is this?”
- OS user/session: “Who is interacting now, and what may they access or authorize?”
- Runtime instance: “Which running process is exercising this capability?”
- Device authorization: “May this installation host this identity?”
- Body binding: “Which optional embodiment controller may the runtime use?”

The intersections occur at service ingress and durable mutation boundaries:

1. Identity state selects the durable continuity domain.
2. Existing user/session authority selects the caller and applicable owner scope.
3. Process capabilities select which service may perform the operation.
4. Policy evaluates the requested action.

Logout revokes interactive session authority. It does not destroy, migrate, or rename the persistent identity. Background maintenance may continue under explicit system policy.

A session may authorize a migration or recovery request, but the identity subsystem validates and performs the transition. A session proof is never direct file-write authority.

## 7. Activation lifecycle

### 7.1 States

| State | Meaning |
|---|---|
| `Dormant` | Identity is valid but no normal active runtime is operating |
| `Activating` | Local lock, lineage, component state, and optional witness lease are being validated |
| `Active` | Normal identity-bound operation is permitted |
| `Migrating` | New writes/actions are quiesced and a transfer is being prepared or committed |
| `Recovering` | Local crash repair or explicit backup restoration is being validated |
| `Suspended` | Integrity, authorization, clone, or lineage conflict prevents normal operation |
| `Retired` | The identity itself was explicitly and irreversibly retired |

`TransferredOut` is an installation state, not an identity state. Migration does not retire the identity.

### 7.2 Normal transitions

```text
Dormant -> Activating -> Active
Active -> Dormant                 clean shutdown
Active -> Migrating -> Dormant    source transferred out
Dormant -> Recovering -> Active   explicit recovery
Any non-retired state -> Suspended
Suspended -> Recovering           explicit authorized repair
Dormant -> Retired                explicit retirement only
```

### 7.3 Required invariants

- Reboot may create a new activation ID, but never a new identity or continuity generation.
- Restarting `wiseowl-braind`, `wiseowl-indexd`, or `wiseowl-memoryd` does not change activation or identity.
- Restarting the identity-hosting MemoryDB process recovers the existing local activation record. It does not create a migration or recovery lineage event.
- An unclean local restart may enter `Recovering(LocalCrash)`. This is not catastrophic backup recovery and does not increment continuity generation.
- `Recovering(BackupRestore)` is explicit and does increment continuity generation.
- Normal personalized requests are denied while activating, migrating, suspended, or retired.

## 8. Backup semantics

A backup is a passive snapshot.

A backup manifest must include:

- Format and schema versions
- `BackupId`
- `IdentityId`
- Committed lineage head and continuity generation
- Snapshot consistency marker
- Component list, roles, lengths, and hashes
- Creation time, if trustworthy
- Source installation fingerprint
- `PackagePurpose::Backup`
- `activation_allowed = false`

A backup must not contain a live activation lease as transferable authority. Local activation state may be retained as forensic data, but an importer must ignore it.

Normal startup must reject a backup directory or backup package as an active store. Only an explicit recovery command may transform a validated backup into a recovery candidate.

Raw filesystem copies cannot be made physically impossible. The protection is that supported startup and tooling distinguish live state from backup state and require explicit recovery lineage before activation.

Backups should eventually be encrypted, but encryption is not part of this phase.

## 9. Migration semantics

### 9.1 Clean migration protocol

```text
Active source
  -> authorize migration
  -> enter Migrating
  -> stop new identity-bound writes and actions
  -> finish/abort open transactions and active actions
  -> checkpoint and verify required stores
  -> seal migration package
  -> mark source installation transferred-out/pending
  -> transfer package
  -> validate package on target
  -> create a new target InstallationId
  -> append MigrationCommitted lineage event
  -> advance continuity generation
  -> acquire target activation
  -> acknowledge successor head
  -> leave source unable to resume normally
```

There should be no interval with two normal active runtimes. The source is quiesced before the destination becomes active.

If the destination fails before committing lineage, the source may resume only through an explicit, audited migration-abort operation. Once a destination may have committed, the source must remain sealed until successor status is resolved.

### 9.2 What migrates

| Current state | Classification | Treatment |
|---|---|---|
| Identity `ROOT`, committed `LINEAGE`, and `HEAD` | MUST MIGRATE | Core continuity |
| MemoryDB `MANIFEST`, `WAL`, `SEGMENTS`, and required snapshots | MUST MIGRATE | Transfer as one consistent store |
| MemoryDB `INDEX/relationships.bin` | MUST MIGRATE currently | Despite its name, checkpointed relationship durability may depend on it |
| Rebuildable MemoryDB indexes | REBUILDABLE | May be verified/rebuilt after restore |
| Durable MemoryDB memories, relationships, and provenance | MUST MIGRATE | Primary learned-memory corpus |
| Identity-scoped personality preferences in `sunlight-kv` | MUST MIGRATE | Current keys require a scoped export/import mechanism |
| Welcome counters and other non-personality MTM | SHOULD MIGRATE | Continuity benefit, but not identity-defining |
| Selected durable short-term promotions in `sunlight-kv` | SHOULD MIGRATE | Only after validation and expiry policy |
| Entire global `sunlight-kv` log | MUST NOT MIGRATE as a unit | It contains unrelated service/user data |
| `wiseowl-indexd/state.bin` | SHOULD MIGRATE | Preserves source IDs and import reconciliation; not core identity |
| `prepared-state.bin` and in-progress imports | MUST NOT MIGRATE live | Reconcile or abort before sealing |
| Managed source documents under indexd state | SHOULD MIGRATE | When they are Wise Owl-managed source material |
| External user documents | MUST NOT be implicitly copied | They follow user-data backup policy |
| Index roots, host paths, device/inode hints | REBUILDABLE / machine-local | Revalidate on the destination |
| Token and lexical indexes | REBUILDABLE | Rebuild after tokenizer or schema changes |
| `wiseowl-memoryd/generation.bin` | MUST MIGRATE until replaced by a stronger allocator | Prevent reuse of historical IDs |
| Short-term RAM, sessions, clients, leases | MUST NOT MIGRATE | Runtime state |
| Cold short-term spill | MUST NOT MIGRATE as active continuity | Drain/promote eligible state; do not revive session-bound TTL state |
| Brain STM and runtime context | MUST NOT MIGRATE | Request, boot, and machine state |
| Foundation Memory | REBUILDABLE | Supplied by installed software |
| Sealed durable action receipts | SHOULD MIGRATE | Audit/provenance continuity |
| Active or volatile action receipt builders | MUST NOT MIGRATE | Seal as interrupted or discard according to action policy |
| Portable identity policy/configuration | SHOULD MIGRATE | Only explicitly identity-scoped settings |
| Hostname, display settings, hardware facts | MUST NOT MIGRATE | Device/runtime state |
| Future identity signing secrets | MUST MIGRATE or be re-authorized securely | Never as unprotected generic metadata |
| Future device keys | MUST NOT transfer as the new device key | Target creates a new key and receives authorization |

The migration package must describe components explicitly. It must never infer scope by copying all of `/state`.

## 10. Recovery semantics

Recovery is used when the previous active installation cannot complete a migration.

Examples include disk failure, laptop loss, motherboard failure, device destruction, or destruction of a future robot body.

### 10.1 Recovery from the only remaining backup

1. Stage the backup as passive data.
2. Validate identity root, lineage, package purpose, component hashes, and supported schemas.
3. Consult any available activation witness or backup repository for a known newer head.
4. Require explicit recovery authority.
5. Create a new local `InstallationId`.
6. Append `RecoveryCommitted` referencing:
   - prior lineage head
   - prior continuity generation
   - `BackupId`
   - `RecoveryId`
   - reason code
   - known state-loss boundary
7. Increment continuity generation.
8. Atomically commit the new lineage head.
9. Activate the recovered installation.
10. Publish the new head to any available witness.

This remains the same identity because the stable ID and authorized lineage continue. Memories created after the backup may be lost, but that loss is recorded rather than hidden.

### 10.2 If the old device returns

- If it carries an older lineage head, it is stale and must enter `Suspended`.
- If it independently recovered and produced a different child of the same prior head, the two histories are divergent clones.
- Neither side may silently overwrite the other.
- A human-authorized reconciliation must select one continuation.
- V1 does not merge two active identity histories.

Without shared infrastructure or later contact, an isolated old copy cannot always be detected. This is an explicit limit.

## 11. Clone/fork semantics

### 11.1 Accidental clone prevention

Supported tooling must prevent accidental cloning by:

- Giving backups a non-activatable package role
- Giving migrations a distinct one-use transfer role
- Excluding transferable local activation state
- Requiring explicit recovery for backup activation
- Holding an exclusive local state lock
- Recording `InstallationId`, activation ID, lineage head, and continuity generation
- Rejecting stale or conflicting heads when a witness is available
- Suspending on divergent lineage when copies later meet

### 11.2 Detection

Two active copies are detectable when:

- They register different activation IDs for the same identity and lineage head.
- A stale copy presents a head older than a known successor.
- Two lineage records have the same predecessor but different successor hashes.
- A transferred-out installation attempts to reactivate.
- A one-use migration operation is consumed twice.

### 11.3 Limits

Perfect prevention is impossible if an attacker copies every byte and secret, isolates both machines, and controls their clocks and infrastructure.

The architecture guarantees:

- No accidental dual activation through supported paths
- Local single-writer enforcement
- Explicit state roles
- Auditable migration and recovery
- Detection when divergent copies contact a common witness or exchange lineage

It does not guarantee detection between permanently isolated perfect copies.

### 11.4 Fork policy

Clone/fork is unsupported in V1 and must be rejected.

If intentional forking is ever supported, it must:

- Generate a new `IdentityId`
- Create a new genesis lineage
- Record the source identity and snapshot as provenance
- Never allow both descendants to claim the original active identity

## 12. Lineage model

Use a small hash-linked lineage, not a blockchain or distributed ledger.

### 12.1 Counters

- `LineageSequence`: increments for every committed lineage record.
- `ContinuityGeneration`: increments only when active continuity moves through migration or catastrophic recovery.
- `ActivationId`: identifies one local active epoch and is not part of global lineage.
- A separate migration-generation counter is unnecessary; migration count is derived from lineage records.

Creation starts at continuity generation 1.

### 12.2 Lineage record

A bounded lineage record contains:

- Format version
- `IdentityId`
- `LineageSequence`
- `ContinuityGeneration`
- `LineageEventId`
- Event kind
- Previous record hash
- Operation ID, when applicable
- Source and target installation fingerprints, when applicable
- Backup or migration package reference
- Optional trustworthy timestamp
- Bounded event metadata
- Record hash
- Optional versioned signature

Event kinds should initially be limited to:

- `Created`
- `ExistingStateAdopted`
- `MigrationCommitted`
- `RecoveryCommitted`
- `KeyAuthorized`
- `KeyRevoked`
- `SchemaUpgraded`
- `Retired`

Preparation and abort state belongs in the local operation journal, not committed continuity history, unless an audited abort record becomes operationally necessary.

### 12.3 Interpretation

- A higher valid descendant head is newer.
- A lower ancestor head is stale.
- Two different children of the same predecessor are a divergence.
- A hash detects accidental corruption.
- A signature, when implemented, proves authorization.
- A hash without a secret does not prove authenticity.

## 13. Memory/personality association

### 13.1 Record association

For V1, bind the entire adjacent MemoryDB store to one persistent identity. Do not rewrite every durable record.

Existing records retain:

- `owner`
- `scope`
- source
- provenance
- session-derived status
- current IDs and wire formats

The identity association comes from:

1. The identity root stored beside the MemoryDB store.
2. The one-identity-per-store V1 invariant.
3. Snapshot manifests binding exported components to `IdentityId` and lineage head.

New per-record identity fields are not required for the initial implementation.

If multiple persistent identities ever share one physical database, a versioned per-record identity namespace will be required. That is outside V1.

### 13.2 Owner versus identity

Both concepts remain necessary:

- Identity answers which Wise Owl owns the continuity domain.
- Owner answers which OS user or system scope may access a record.

A migration must not silently reinterpret owner IDs. If UIDs differ across machines, V1 must either:

- preserve the relevant account IDs, or
- use an explicit reviewed owner mapping at the access layer.

It must not bulk-rewrite existing record formats during identity introduction.

### 13.3 State categories

**A. Core identity**

- Identity root
- Creation record
- Committed lineage
- Public key history
- Continuity generation

**B. Personality continuity**

- Explicit personality preferences
- Long-lived behavioral settings
- Relationship state
- Stable user-specific preferences intended to travel
- Identity-scoped policy choices

**C. Learned memory**

- MemoryDB records
- Durable provenance
- Relationships
- User-confirmed knowledge
- Session summaries intentionally promoted to long-term memory
- Selected durable medium-term state

**D. Reconstructable derived state**

- Lexical/token indexes
- Query indexes
- Indexer caches
- Health snapshots
- Tokenizer-derived state
- Rebuildable source discovery data
- Foundation Memory supplied by software

**E. Machine/runtime state**

- Hostname and hardware facts
- Graphical session and login
- Process IDs
- Request and client IDs
- Runtime context
- STM
- SHM leases
- Open transactions
- Active action flows
- Device/inode hints
- Body controller state

Foundation Memory describes product defaults, role, and safety principles. It does not define the unique persistent identity.

## 14. Self-healing boundaries

Self-healing may automatically repair:

- Rebuildable indexes
- Token caches
- Search caches
- Health snapshots
- Stale runtime locks after validated process death
- Incomplete temporary files
- Recoverable WAL tails under existing database rules
- Indexer operational state that can be reconstructed without changing source identity
- Local activation state after a proven same-installation crash

Self-healing must never silently regenerate or replace:

- `IdentityId`
- Creation record
- Identity authority key
- Committed lineage
- Continuity generation
- Migration or recovery history
- Known lineage head
- A transferred-out installation marker

If root integrity fails:

1. Stop identity-bound normal operation.
2. Enter `Suspended`.
3. Expose bounded diagnostics.
4. Attempt read-only validation against known backups.
5. Require explicit recovery or repair.
6. Never create a replacement identity merely because the old root cannot be read.

The governing invariant is:

> Identity corruption must not silently become identity replacement.

## 15. Upgrade compatibility

### 15.1 Software upgrade

A Wise Owl binary upgrade must load the existing identity before personalized operation. Software version is not part of identity.

### 15.2 MemoryDB schema upgrade

MemoryDB may upgrade its own schema using existing transactional rules. It must preserve the adjacent identity root and bind the upgraded store to the same identity.

### 15.3 Tokenizer upgrade

Tokenizer changes may rebuild derived index state. They do not modify identity or continuity generation.

### 15.4 SunlightOS upgrade

An OS upgrade must retain the identity directory and all required identity-bound component state. Reinstalling binaries or Foundation Memory does not create an identity.

### 15.5 Hardware replacement

Hardware replacement is either:

- ordinary repair, if storage and authorized installation state remain intact;
- clean migration, if state moves intentionally; or
- recovery, if restored from backup after loss.

Hardware identifiers never enter the identity ID.

### 15.6 Identity schema upgrade

Identity formats must use bounded, explicit versioned codecs.

Upgrade rules:

- Decode supported older versions.
- Validate fully before mutation.
- Write the upgraded form to a temporary file.
- Preserve `IdentityId`, creation record, and lineage.
- Atomically publish the new form.
- Append `SchemaUpgraded` without incrementing continuity generation.
- Refuse unknown newer required versions.
- Keep a recoverable previous copy until commit is durable.
- Make schema migration idempotent after crashes.

## 16. Security/cryptographic readiness

The logical model must support cryptography without depending on it for naming.

Define interfaces for:

- `EntropySource`
- `IdentitySigner`
- `IdentityVerifier`
- `IdentityKeyStore`
- `PackageProtector`
- `ActivationWitness`
- `RecoveryAuthority`
- `TrustedClock`, where available

Recommended future hierarchy:

- Stable random `IdentityId`
- Identity authority public key recorded in root/lineage
- Protected authority private material
- Per-installation activation keypairs
- Source-authorized target installation key during migration
- Recovery key or recovery policy independent of a single device
- Key rotation through lineage

No TPM is required for V1. Hardware-backed keys may later implement `IdentityKeyStore`.

Until signatures exist:

- Checksums and hashes provide corruption detection only.
- Local filesystem and process authority provide local access control.
- Migration over untrusted media must not be treated as confidential or strongly authenticated.
- Production remote migration should wait for package protection and authenticated transfer.

Private keys must never be stored as ordinary fields in `ROOT`, `LINEAGE`, or an unencrypted manifest.

## 17. Interaction with audit findings

| Audit finding | Classification | Identity interaction |
|---|---|---|
| Native short-term spill not restored | CAN WAIT | Short-term session state must not define identity or migrate as active continuity. Clean migration should drain/promote eligible data. |
| Fixed/admin IPC caller identity | BLOCKS IDENTITY IMPLEMENTATION | Identity mutation, migration, recovery, and retirement cannot be exposed through a caller model that grants every request administrative authority. |
| Weak host socket authentication | BLOCKS IDENTITY IMPLEMENTATION for host mutation paths | Host identity APIs need peer credentials, restrictive permissions, or a read-only-only host mode before mutation is exposed. |
| UID truncation in MTM keys | SHOULD BE FIXED BEFORE MIGRATION | Truncation can collide personality/preferences across users and makes scoped export ambiguous. It does not define the identity ID. |
| Volatile action receipts | SHOULD BE FIXED BEFORE MIGRATION | Sealed receipts are useful audit continuity. Active receipt builders must still not migrate. |
| Native persistence durability differences | BLOCKS IDENTITY IMPLEMENTATION | Identity creation and lineage commit require atomic publication, durable ordering, and recoverable crash semantics. |
| No backup/restore identity | SHOULD BE FIXED BEFORE MIGRATION | The new package-purpose and recovery-lineage model supplies the missing semantics. |
| No clone detection | SHOULD BE FIXED BEFORE MIGRATION | Local locks, lineage divergence checks, transfer markers, and optional witness leases are required before migration is enabled. |

These classifications do not authorize unrelated refactoring in this phase.

## 18. Existing-installation migration plan

Existing installations are adopted without changing existing MemoryDB, spill, index, KV, or record formats.

### 18.1 First-run adoption

```text
Acquire exclusive identity creation lock
  -> find no committed ROOT
  -> inspect for a staged creation candidate
  -> validate existing Wise Owl stores
  -> generate IdentityId once
  -> write staged ROOT and creation lineage
  -> fsync staged files
  -> publish identity directory atomically
  -> commit HEAD
  -> record ExistingStateAdopted
  -> bind adjacent existing state
  -> activate normally
```

The adoption record should note that historical creation time is unknown and that identity was first formalized from an existing installation.

### 18.2 Crash safety

The first random ID must be persisted before any operation that can be retried.

On restart:

- A complete valid staged root is finalized using the same ID.
- It is not discarded merely because publication was interrupted.
- An incomplete staging file is quarantined.
- Multiple valid candidates cause suspension and manual resolution.
- If existing Wise Owl data is present and identity creation evidence is corrupt, the service must not generate another ID automatically.
- A fresh empty installation may retry creation only when no committed or recoverable candidate exists.

### 18.3 Compatibility

- Existing MemoryDB remains byte-for-byte valid.
- Existing records are not rewritten.
- Existing indexed data remains valid.
- Existing owner IDs retain their meaning.
- Subsequent boots load the committed root.
- Index and tokenizer rebuilds are unnecessary solely because identity was introduced.
- Adoption can initially be feature-gated, but once committed the installation must never fall back to anonymous legacy operation.

## 19. Proposed Rust types/modules

Repository naming favors concise domain names such as `MemoryId`, `SourceId`, `SessionId`, and `ActionReceiptId`. Therefore use `IdentityId`, not `WiseOwlIdentityId`, inside a crate already named `wiseowl-identity`.

### 19.1 Modules

```text
wiseowl-identity/
    src/
        lib.rs
        id.rs
        root.rs
        lineage.rs
        activation.rs
        package.rs
        binding.rs
        codec.rs
        validation.rs
        crypto.rs
```

`wiseowl-memorydb` would add a hosting adapter for storage, locking, IPC, startup, and component coordination.

### 19.2 Types

| Type | Responsibility | Persistence | Copy/clone semantics | Serialization | Owner and validation |
|---|---|---|---|---|---|
| `IdentityId([u8; 32])` | Stable global identity name | Persisted in root and manifests | `Copy` is safe; copying the value does not authorize activation | Fixed 32 bytes; checksummed text form | Created once from CSPRNG; non-zero; immutable |
| `IdentityRoot` | Minimal creation and authority anchor | Persisted | May be cloned in memory for validation; no `Default` | Versioned bounded LE codec | Identity subsystem only; ID and creation fields immutable |
| `IdentityFormatVersion(u16)` | Durable encoding version | Persisted | `Copy` | LE integer | Known supported range |
| `LineageSequence(u64)` | Orders all lineage records | Persisted | `Copy` | LE integer | Strictly increments by one |
| `ContinuityGeneration(u64)` | Orders migrations/recoveries | Persisted | `Copy` | LE integer | Starts at one; increments only on migration/recovery |
| `LineageEventId([u8; 16])` | Uniquely names a lineage event | Persisted | `Copy` | Fixed bytes | Random, non-zero |
| `LineageEventKind` | Restricts legal history events | Persisted | `Copy` | Explicit numeric discriminants | Unknown required kinds rejected |
| `LineageRecord` | Hash-linked committed history entry | Persisted | Cloneable for validation; immutable after commit | Length-delimited, bounded, checksummed/signed | Identity subsystem; previous hash and counters validated |
| `LineageHead` | Commits current sequence, generation, and hash | Persisted | `Copy` | Fixed versioned record | Must point to a valid committed record |
| `InstallationId([u8; 16])` | Names one installed host of the identity | Local persistent | `Copy`; excluded from portable identity authority | Fixed bytes | Random per installation; never identity |
| `ActivationId([u8; 16])` | Names one active epoch | Local persistent/runtime | `Copy`; not portable | Fixed bytes | Random; bound to installation and lineage head |
| `RuntimeInstanceId([u8; 16])` | Names one service process instance | Runtime-only | `Copy` | IPC/diagnostic only | Regenerated per process |
| `ActivationState` | Dormant/active/migration/recovery state machine | Local persistent plus runtime view | `Copy` enum | Explicit discriminants | Legal transitions only |
| `LocalActivationRecord` | Installation, activation, lock, clean-stop, and witness state | Local persistent | Cloneable snapshot; excluded from packages | Versioned bounded codec | Identity host; must match root/head |
| `BackupId([u8; 16])` | Names a passive snapshot | Package | `Copy` | Fixed bytes | Random and non-zero |
| `BackupManifest` | Binds passive component snapshot to lineage | Package | Byte copying is possible; activation remains prohibited by policy | Versioned bounded codec | Backup exporter; hashes/components validated |
| `MigrationId([u8; 16])` | Names a one-use transfer | Package and operation journal | `Copy` value; consumption enforced durably | Fixed bytes | Must be unconsumed and match source head |
| `MigrationManifest` | Binds a sealed transfer to a predecessor head | Package | Avoid deriving `Clone` to discourage casual duplication; security does not rely on Rust ownership | Versioned bounded codec | Migration state machine |
| `RecoveryId([u8; 16])` | Names one explicit restore activation | Persisted in lineage | `Copy` | Fixed bytes | Unique and authorized |
| `ComponentSnapshot` | Describes one migrated/backed-up component | Package | Cloneable metadata | Bounded list with hashes and role | Known component kind, size, and digest |
| `IdentityStatus` | Sanitized read-only runtime view | Runtime-only | Cloneable | Separate versioned IPC wire format | Derived from validated authoritative state |
| `PublicIdentityKey` | Versioned public key material | Root/lineage | Cloneable public data | Algorithm-tagged bounded bytes | Supported suite and length |
| `IdentityKeyHandle` | Opaque reference to protected private authority | Local runtime/persistent keystore | Must not implement `Copy` or expose secret bytes | Not serialized in ordinary manifests | Key-store implementation only |

Durable codecs should not use host-only `bincode`. Use explicit bounded encodings compatible with `no_std`, matching existing project practice.

## 20. Proposed storage layout

Because filesystem policy already authorizes `wiseowl-memorydb` to write its own state, use:

```text
/state/wiseowl-memorydb/
    IDENTITY/
        ROOT
        LINEAGE
        HEAD
        LOCAL
    MANIFEST
    WAL/
    SEGMENTS/
    INDEX/
    ACTION_RECEIPTS/
    ...
```

Host equivalent:

```text
${WISEOWL_MEMORYDB_DIR}/IDENTITY/
```

### 20.1 Authority

- `ROOT`, committed `LINEAGE`, and `HEAD` are authoritative identity data.
- `LOCAL` is authoritative only for this installation’s activation state.
- MemoryDB’s existing `MANIFEST` remains authoritative for the database, not identity.
- Backups and migration packages are external artifacts and are never auto-discovered as active state.

### 20.2 Publication requirements

**Root creation**

1. Create a private staged directory.
2. Write `ROOT`, genesis `LINEAGE`, and initial `HEAD`.
3. Flush every file.
4. Flush the staging directory.
5. Rename staging to `IDENTITY`.
6. Flush the parent directory.

**Lineage commit**

1. Append a complete length-delimited record.
2. Flush `LINEAGE`.
3. Atomically replace `HEAD`.
4. Flush `HEAD` and its directory.
5. Only then expose the transition as committed.

**Local state**

- Write temporary file
- Flush
- Rename
- Flush parent directory

The existing native best-effort create/write behavior is insufficient for this boundary and must be strengthened before implementation.

### 20.3 Recovery

- A valid complete lineage tail beyond `HEAD` is uncommitted and must not be activated automatically.
- An operation journal in `LOCAL` determines whether a migration/recovery operation may resume or must suspend.
- A `HEAD` that does not resolve to a valid record causes suspension.
- A corrupt root causes suspension.
- Unknown newer formats cause read-only degradation.
- Normal recovery never manufactures another ID.

### 20.4 Permissions

- `IDENTITY/`: `0700`
- Files: `0600`
- Read-only status is exposed through IPC, not direct user filesystem access.
- Secret key material, when introduced, uses a separate protected key-store interface.

## 21. New architectural invariants

1. Persistent identity is never derived from hardware.
2. Persistent identity is never derived from UID, username, or OS account.
3. Persistent identity is never derived from hostname, device name, or body serial number.
4. Reboot does not create a new identity.
5. Service restart does not create a new identity.
6. Hardware replacement does not inherently create a new identity.
7. A MemoryDB path is not an identity.
8. MemoryDB database generation is not an identity.
9. MemoryDB index generation is not an identity.
10. Foundation Memory does not define persistent identity.
11. Derived indexes do not define persistent identity.
12. A backup is passive and is not an active identity.
13. A backup cannot enter normal active startup.
14. Migration preserves `IdentityId` and advances authorized continuity.
15. Catastrophic recovery preserves `IdentityId` and records state loss.
16. Fork or clone never occurs implicitly.
17. A supported fork, if ever added, receives a new identity.
18. Identity corruption never silently creates a replacement identity.
19. Session authority cannot modify identity arbitrarily.
20. Logout revokes session authority but does not end persistent identity.
21. Existing user ownership remains distinct from Wise Owl identity.
22. A device or body is replaceable and revocable.
23. Only one local writer may host an active identity store.
24. Normal operation requires a valid root and committed lineage head.
25. Stale and divergent lineage cannot be silently overwritten.
26. Migration packages and backup packages are not interchangeable.
27. Runtime, session, and activation IDs are never stable identity identifiers.
28. Cryptographic key rotation must not change `IdentityId`.
29. Hash integrity must not be described as cryptographic authorization.
30. One persistent identity per MemoryDB store is the V1 rule.

## 22. Laptop-to-robot validation scenario

| Year/event | Changes | Persists | Event | Why it remains the same identity |
|---|---|---|---|---|
| 2027: Laptop A | Initial installation and adoption | Identity ID, genesis lineage, memory, preferences | Creation/adoption; continuity generation 1 | First durable identity root is established |
| 2030: Laptop B | Device, installation ID, runtime, paths, hardware facts | Identity ID, lineage, MemoryDB, selected MTM, relationships | Clean migration; continuity generation advances | Laptop A is sealed and Laptop B becomes the authorized successor |
| 2034: Robot Body V1 | Optional body binding and device authorization | Identity and continuity corpus | Migration or authorized runtime relocation | The body is a replaceable endpoint used by the active runtime |
| 2038: Compute board replacement | Installation/runtime/device key | Identity ID, lineage, memory and personality state | Repair, migration, or recovery depending on state availability | Compute hardware is not an identity field |
| 2042: Entirely new body | Body/chassis identity and controllers | Persistent identity and authorized lineage | Clean migration to new body | Body replacement changes embodiment, not identity |
| 2050: No original hardware remains | All 2027 physical components are gone | Identity ID, descendant lineage, memories, preferences, relationships, provenance | No special event beyond prior migrations | Continuity is carried by durable state and authorized lineage, not original matter |

V1 does not implement robot controllers. It only avoids making laptop properties part of identity.

## 23. Risks and unresolved questions

- SunlightOS needs a verified cryptographic random source available during first identity creation.
- Native filesystem support must demonstrate file and directory durability adequate for root and lineage commit.
- A consistent multi-service snapshot protocol does not yet exist.
- `sunlight-kv` needs identity-scoped export/import; copying its global log would migrate unrelated state.
- Existing UID-derived MTM keys collide above 32 bits and need a compatible transition.
- Owner mapping across machines remains unresolved when account IDs differ.
- It must be decided whether one Wise Owl identity serves multiple OS users or whether each user eventually gets a distinct identity.
- Indexer state contains machine-specific paths and device/inode hints that need invalidation rules.
- Current native short-term spill cannot be relied upon for migration.
- Current action-receipt runtime wiring does not yet deliver complete durable continuity.
- An activation witness may be local, LAN-based, backup-repository-based, or remote. V1 must not require permanent network access.
- Offline clone-conflict resolution requires explicit user-facing policy.
- Recovery authority and recovery-key custody need a later security design.
- Migration package confidentiality is unresolved until encryption is implemented.
- Hardware-backed keys may improve protection but must remain optional.
- Retention and compaction policy for lineage must preserve all continuity-changing events.
- Reliable wall time may be unavailable. Ordering must rely on counters and hashes, not timestamps.
- Migration must define exact quiescence requirements for MemoryDB transactions, pending imports, KV writes, and active actions.
- The old installation’s handling after a lost migration acknowledgement needs deterministic recovery tests.
- The user experience for `Suspended` identity must avoid offering an easy “create new identity” shortcut over existing data.

## Minimal implementation sequence

### Phase A — Identity contracts and durable root

Purpose: introduce `wiseowl-identity`, codecs, validation, and crash-safe root creation hosted by MemoryDB.

- No existing record changes
- No migration or restore
- Deterministic codec, corruption, and staged-creation crash tests
- Existing installation adoption behind an explicit feature gate
- Boot remains possible with Wise Owl degraded

### Phase B — Read-only identity status and store binding

Purpose: expose validated identity status to brain, index, memory, and diagnostics.

- Brain remains operational in generic degraded mode when identity is unavailable
- No identity mutation through consumer IPC
- Tests verify reboot/service restart retain the same ID
- Existing MemoryDB and index data remain untouched

### Phase C — Local activation and single-writer enforcement

Purpose: implement `Dormant`, `Activating`, `Active`, local recovery, `Suspended`, installation IDs, activation IDs, and exclusive locking.

- No cross-device migration
- Deterministic duplicate-writer and unclean-restart tests
- Service restarts do not advance continuity
- Bootability is preserved

### Phase D — Passive identity-aware backup

Purpose: create consistent passive backup manifests and component inventories.

- Backups cannot activate
- No restore yet
- No encryption yet
- Deterministic package-purpose, hash, truncation, and crash tests

### Phase E — Explicit catastrophic recovery

Purpose: restore a passive backup through an authorized `RecoveryCommitted` lineage event.

- Continuity generation advances
- Stale-head checks are mandatory
- Old installations suspend when a newer head is observed
- Deterministic old-device-return and dual-recovery divergence tests

### Phase F — Clean migration

Purpose: quiesce the source, transfer one authoritative lineage, activate the destination, and seal the source.

- Requires fixed identity write authentication
- Requires native durable publication
- Requires scoped KV export/import
- Requires deterministic crash testing at every prepare/commit/acknowledgement boundary

### Phase G — Cryptographic activation hardening

Purpose: add signatures, installation keys, protected migration, recovery keys, and optional activation witnesses.

- Does not change `IdentityId`
- Does not require a TPM
- Maintains an offline degraded path with explicit limitations
- Includes key rotation and copied-secret conflict tests

Each phase must preserve existing data, have one reviewable purpose, and provide host tests plus a deterministic SunlightOS boot gate where target behavior changes.

## What NOT to build yet

- A new identity daemon
- Robot motor, sensor, chassis, or body-control code
- Semantic search, embeddings, or vector indexes
- Active-active Wise Owl replicas
- Automatic cross-device synchronization
- Blockchain or distributed consensus
- A cloud service required for ordinary boot
- TPM-only identity
- Backup encryption implementation
- A general-purpose secrets manager
- Automatic identity merge
- Fork/clone support
- Per-record identity rewrites
- Changes to existing MemoryDB, spill, index, or receipt formats
- Replacement of current owner/user semantics
- Automated personality inference
- Claims about consciousness
- Automatic identity regeneration after corruption
- Unrelated fixes from the prior audit
- Migration of arbitrary global `sunlight-kv` contents
- Migration of live sessions, SHM leases, open actions, or request-local state

The V1 objective is a small, durable, inspectable identity root and lineage that existing Wise Owl services can adopt incrementally without destabilizing memory, session, IPC, or storage behavior.
