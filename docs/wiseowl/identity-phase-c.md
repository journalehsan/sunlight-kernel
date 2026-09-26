# Wise Owl Persistent Identity — Phase C

## Identity boundaries

IdentityId remains the global persistent identity, validated from ROOT,
LINEAGE, and HEAD by MemoryDB. InstallationId is a random 128-bit value stored
in installation-local IDENTITY/LOCAL. It describes a logical local
installation, not a device: it is not derived from a disk UUID, serial number,
hostname, MAC address, VM identifier, CPU, UID, or robot chassis.

ActivationId is another random 128-bit value. It names one local active epoch.
It is distinct from the MemoryDB process incarnation, represented on native
SunlightOS by the kernel process generation and endpoint ownership. Neither a
PID nor a process generation is persisted into LOCAL.

    Identity A
        |
        +-- Installation I
                +-- Activation X  [old boot]
                +-- Activation Y  [current boot]
                        +-- MemoryDB runtime generation N

Identity is not installation. Installation is not activation. Activation is
not process. Reboot keeps IdentityId and InstallationId, advances the local
activation sequence, and creates a new ActivationId. A MemoryDB restart within
one OS boot keeps all three IDs and changes only the process generation.

## LOCAL format and publication

/state/wiseowl-memorydb/IDENTITY/LOCAL is a 104-byte little-endian WLOC
version 1 record. It contains format version, ActivationState, clean-stop
marker, IdentityId binding, InstallationId, optional ActivationId, a
installation-local activation sequence, a random boot epoch, and an FNV-1a
64-bit corruption checksum. The checksum detects accidental corruption; it is
not a cryptographic signature or identity authority.

ROOT, LINEAGE, and HEAD remain the global continuity record. LOCAL contains no
memory contents, session IDs, hardware identifiers, migration data, or keys.
A malformed record, checksum failure, wrong IdentityId, zero ID, or impossible
state combination suspends startup without recreating IdentityId or changing
global lineage.

Updates create LOCAL.tmp with exclusive creation, write exactly 104 bytes,
sync the file, sync the IDENTITY directory, rename the staged file over LOCAL,
then sync IDENTITY again. The API reports durable success only after the final
directory sync. On the native FAT path, rename also flushes the destination
directory entry before deleting the source and flushes again after deletion.
This gives an earlier recoverable publication point on that filesystem, but
the final parent directory sync remains the caller's durable commit point.
A crash around rename can expose the old LOCAL, the new LOCAL, or both names;
recovery validates every visible candidate. A complete first-install stage is
promoted before a new InstallationId can be generated. An incomplete stage
with no committed LOCAL can be discarded; it contains no recoverable ID.

## Single-writer ownership

Native MemoryDB reserves the private nameserver endpoint
wiseowl.memorydb.authority.v1 before local activation. The kernel ties the
endpoint capability to the owning process incarnation. Init rejects a second
live registration and can replace a registration after its endpoint owner has
terminated. The endpoint reservation is the volatile runtime lease; LOCAL is
only durable state and is never used as proof that a process is alive.

The host daemon uses a host-kernel flock(LOCK_EX|LOCK_NB) held on a sibling
runtime authority file for its lifetime. The file's existence carries no ownership
meaning; the kernel lock is released when the process exits. Host and native
tests cover two host owners contending for the same store. Independent stores
are not compared, and Phase C does not detect clones across devices.

## Activation lifecycle

The implemented states are Dormant, Activating, Active, RecoveringLocal, and
Suspended. Only a fully committed Active LOCAL record is published through
the normal MemoryDB endpoint. Braind, indexd, and memoryd require protocol v2
status with Active state before personalized continuity, durable index work,
or durable memory promotion. Their status caches lose authority on disconnect
or malformed status. A changed activation fingerprint inside one connected
endpoint context fails closed; a new endpoint generation must query again.

At startup MemoryDB validates the global identity, reserves writer authority,
and reads a random boot epoch stored in volatile /tmp RAMFS. A narrow filesystem
policy grants only MemoryDB access to this exact runtime marker. It writes
Activating/RecoveringLocal before opening the database, validates and opens
MemoryDB, then durably commits Active before endpoint publication. A process
restart in the same boot reuses the ActivationId when the LOCAL state and
volatile boot epoch prove that it is the same epoch. A different boot epoch
recovers a stale Active record locally and creates a new ActivationId.

    Dormant -> Activating -> Active
    Active -> RecoveringLocal -> Active    (process restart or unclean boot)
    Active -> Dormant                      (clean deactivation policy)
    invalid/ambiguous -> Suspended

The service manager sends SIGTERM, waits for confirmed death, and may escalate
to SIGKILL. Native MemoryDB has no pre-termination handler or bounded quiesce
callback that could durably publish Dormant. It therefore does not claim a
clean Dormant transition. A stop leaves LOCAL=Active; the kernel revokes the
dead process's endpoint, and a replacement enters RecoveringLocal. On a new
boot, the volatile boot epoch differs and the replacement creates a new
ActivationId. The extra recovery work is a lifecycle cost, not an identity
safety gap. The pure Dormant transition and clean-stop format are tested; no
native shutdown hook was added.

The volatile boot epoch distinguishes a daemon restart from a reboot. It is
generated from the system random source and held under /tmp; it is not an
installation or device identifier. If /tmp contains malformed evidence,
startup fails closed.

## Continuity invariants

Activation, deactivation policy, process restart, local recovery, duplicate
writer rejection, and unclean VM stop do not append LINEAGE events or modify
ROOT/LINEAGE/HEAD. Native activation also checks
LineageSequence == 1 and ContinuityGeneration == 1 before proceeding.
IdentityId corruption never causes a new identity to be generated over LOCAL.

## C.1 commit and restart model

| Transition | Pre-commit and staged state | Publication | Durable commit and restart interpretation |
| --- | --- | --- | --- |
| No LOCAL to first InstallationId | No LOCAL; `LOCAL.tmp` is Activating with InstallationId I and no ActivationId | Rename to LOCAL | Final IDENTITY directory sync. A valid first stage found after crash is promoted and I reused; no valid stage permits first adoption retry. |
| Dormant or first installation to Activating | Committed prior LOCAL; stage contains ActivationId X | Rename over LOCAL | Final directory sync. Before publication, prior LOCAL wins and a fresh X may be generated. A visible published X in the same boot is retained. |
| Activating to Active | Committed Activating X; stage is Active X | Rename over LOCAL | Final directory sync occurs before endpoint registration. Crash before completion reopens MemoryDB and republishes Active X in the same boot. |
| Active to RecoveringLocal | Committed Active X; stage is RecoveringLocal with X on the same boot or Y on a new boot | Rename over LOCAL | Final directory sync. Committed Active remains authoritative before publication; same boot keeps X, new boot advances sequence and creates Y. |
| RecoveringLocal to Active | Committed RecoveringLocal; stage is Active with the same activation | Rename over LOCAL | Final directory sync before writable readiness. Interrupted publication revalidates MemoryDB and republishes Active. |
| Active to Dormant | Pure policy transition only | No native publication | Native stop is handled as a crash; no clean-stop guarantee. |

`LOCAL.tmp` is never independent authority when a valid committed LOCAL exists.
A plausible unrenamed successor is discarded; a conflicting InstallationId,
ActivationId, sequence, or boot epoch suspends startup. An unknown `LOCAL.*`
candidate also suspends. A corrupt committed LOCAL suspends even if a stage is
present. Filename order and timestamps never choose a winner. FNV-1a checks
accidental corruption and torn records only; it is not authentication.

The native boot marker is generated from the kernel random source, written
once to `/tmp/wiseowl-memorydb-boot-epoch`, and held in the root RAMFS. The
only persistent copy is historical data in LOCAL, never authority by itself.
The RAMFS is reconstructed on every kernel boot; `/state` alone cannot carry
the current boot marker to another running system. The single writer endpoint
must be acquired before the marker is read or LOCAL is changed.

## C.1 verification matrix

| Behavior | Implementation | Host verified | Native verified | Known limitation |
| --- | --- | --- | --- | --- |
| InstallationId persistence | First Activating stage and LOCAL recovery | Ten-boundary fault matrix; valid stage reuse | Two-boot LOCAL comparison | Deletion of every LOCAL candidate is indistinguishable from first adoption; no crash path in the FAT publication contract does this. |
| ActivationId on new boot | Boot epoch mismatch and checked sequence increment | State-machine and fault matrix | QEMU two-boot comparison | QEMU stop is not physical power loss. |
| Same-boot MemoryDB restart | Preserve X if boot epoch matches | Process restart and fault matrix | Test-only service-manager stop, confirmed death, replacement; process generations differ | No physical power-loss test. |
| Duplicate writer | Kernel owned private endpoint; host flock | Eight separate-process races | Second native daemon rejected before activation | No clone detection across systems. |
| Stale writer recovery | Endpoint revocation on process exit | Replacement after first process killed | Replacement accepted after A confirmed dead | PID reuse itself is covered by unique endpoint IDs and revoked capabilities, not forced in QEMU. |
| Unclean system recovery | Active to RecoveringLocal to Active | New-boot fault matrix | QEMU state image reboot | No physical power-loss test. |
| Clean Dormant shutdown | Pure transition only | Codec and policy | No | Service manager has no durable pre-termination callback. |
| LOCAL fault boundaries | Ten checkpoints per publication | First install, Dormant to Activating, Active to RecoveringLocal on same/new boot, and both Activating and RecoveringLocal to Active | Native persistence path and two-boot recovery, without in-guest fault stops at each checkpoint | Host injection models process interruption, not device write reordering. |
| Consumer gating | v2 Active status and endpoint generation | Brain, index, memoryd status tests | Test-image requests prove brain Personalized → GenericDegraded → Personalized, index pause/resume, memory durable promotion 1 → 0 → 1, and RAM session creation while MemoryDB is absent | Probes are compiled only for the identity gate image. |

The `wiseowl-identity-phase-c` image compiles a narrow service-manager gate.
It launches duplicate MemoryDB B after A and the consumers are ready, confirms
B's live-owner rejection, kills B, stops A, probes braind, indexd, and memoryd
while MemoryDB is absent, then starts a replacement and probes all three again.
The test image uses tagged health replies for braind and memoryd and invokes
memoryd's existing RAM `CreateSession` operation during the outage. Those
probe replies are unavailable in production builds.
The gate verifies
two distinct accepted process generations, one rejected process, the same
ActivationId and boot epoch across the service restart, and a new ActivationId
and boot epoch after a VM stop and second boot. It saves serial logs, LOCAL
snapshots, and comparisons under `target/wiseowl-identity-phase-c-evidence/`.
No production guest command channel or general execution hook is present.

The completed C.1 evidence is sufficient for local Phase C identity safety:
an interrupted publication cannot replace a recoverable InstallationId, only
one live local writer can publish, a same-boot replacement retains the
activation, and a new boot rotates it without touching global lineage. A
passive Phase D design may start after this gate; Phase D implementation is
outside this verification. A physical power-loss trial remains a separate
coverage gap and is not claimed as native verified.

The final regression run passed 556 host tests across `wiseowl-identity`,
`wiseowl-memorydb`, `wiseowl-brain`, `wiseowl-index`, `wiseowl-memory`,
`sunlight-ipc`, `sunlight-fs`, and `sunlight-fat`. The unfiltered run has one
unrelated RAMFS document fixture failure
(`why_sunlightos_exists_document_is_present_in_default_homes`); skipping only
that fixture leaves no failures. The Phase A, Phase B, Phase C, and trusted
session readiness QEMU gates passed. A direct host `cargo test` of the kernel
binary's IPC module cannot link because its no_std panic and allocation
handlers conflict with Rust's std test harness. The native init registry
integrity self-test passed in the Phase C boot, alongside the live duplicate
and stale-owner replacement scenario.

## Deferred

Phase C introduces no migration, backup activation, cross-device clone
detection, catastrophic recovery, witnesses, signatures, device key hierarchy,
or continuity-generation advancement. Passive identity-aware backup remains
future work and must treat LOCAL as installation-local state rather than
global identity.
