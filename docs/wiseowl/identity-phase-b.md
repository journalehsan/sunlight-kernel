# Wise Owl Persistent Identity — Phase B

## Authority and sanitized status protocol

`wiseowl-memorydb` remains the only persistent identity authority. It validates
ROOT, LINEAGE, and HEAD during startup and binds the resulting `LoadedIdentity`
before publishing `wiseowl.memorydb.v1`. Consumers cannot write those files or
submit an IdentityId for MemoryDB to trust. Existing MemoryDB records and their
owner/user semantics are unchanged.

`GetIdentityStatus` is read-only and returns a sanitized status. The host frame
is exactly 36 bytes: `WIDS`, protocol version, state, persistence flag, an
8-byte diagnostic fingerprint, lineage sequence, continuity generation,
genesis kind, identity format version, and validation status. The native form
packs those same semantic fields into exactly four IPC registers: a validated
header, fingerprint, lineage sequence, and continuity generation. Both decoders
enforce exact length/count, protocol version, known enum values, reserved bits,
and valid field combinations. Neither form exposes full IdentityId, ROOT or
LINEAGE bytes, paths, or secret material.

Native caller identity is not currently strong enough to distinguish trusted
Wise Owl services from ordinary diagnostics. Phase B accepts that limitation
because the operation is read-only and the reply is sanitized. Caller
authorization for identity status remains technical debt; this protocol must
not grow into a mutation interface.

When identity or persistence startup fails, MemoryDB does not publish its
normal endpoint. Consumers observe disconnection and fail closed. This is safe
for current Phase B consumers: none needs a live invalid-status endpoint to
make an authorization decision. A separate maintenance endpoint would add
protocol and exposure without enabling required behavior, so startup continues
to withhold the normal endpoint.

## Consumer behavior

The status cache is runtime-only. A disconnect, failed request, malformed
reply, endpoint-generation change, or identity mismatch removes current
authorization. Consumers may retain the last fingerprint only to detect a
different identity if the service returns. A cached fingerprint never proves
that the current endpoint is authoritative.

Braind enables personalized continuity only after a valid status query. A
failed query moves it to generic degraded behavior and clears its current
continuity generation. A reconnect with the same fingerprint resumes
personalized behavior; a different fingerprint moves it to
`SuspendedMismatch` and cannot silently rebind. Restart begins disconnected and
queries MemoryDB again; braind does not reconstruct identity from local state.

Indexd requires a fresh valid status before durable scans/imports and pending
import reconciliation. A failed query pauses new identity-bound durable work
and preserves existing index data. A same-identity reconnect resumes work; a
different identity suspends it. The observed fingerprint is in-memory only.
Native endpoint capability generation is tracked independently from persistent
IdentityId, so a MemoryDB process restart can produce a new endpoint generation
while retaining the same identity.

Memoryd checks MemoryDB status before each durable KV promotion. Short-term RAM
operations remain available where otherwise allowed. Unavailable, invalid,
malformed, or version-mismatched status denies promotion. Same-identity
reconnect restores eligibility; a different fingerprint permanently blocks
promotion for that process. Memoryd does not generate IdentityId or persist its
status cache.

No consumer writes an authoritative identity copy into braind state, indexd
state, memoryd state, or sunlight-kv preferences. Existing index content
fingerprints and file digests are unrelated to Wise Owl persistent identity.
No Phase B status query, reconnect, or consumer restart increments
ContinuityGeneration; the invariant remains exactly 1.

## Verification status

| Behavior | Implemented | Host verified | Native verified |
| --- | --- | --- | --- |
| MemoryDB owns and returns sanitized status | Yes | Yes, codec tests | Yes, QEMU Phase B gate |
| Braind binds, degrades on disconnect, resumes same A, rejects B | Yes | Yes, unit and multi-service integration tests | Initial binding to same fingerprint verified; in-guest reconnect not yet verified |
| Indexd pauses durable work, preserves index, resumes same A, rejects B | Yes | Yes, service and multi-service integration tests | Initial binding to same fingerprint verified; in-guest reconnect not yet verified |
| Memoryd durable promotion gate, same-A resume, B mismatch | Yes | Yes, runtime gate and multi-service integration tests | Initial status binding verified; promotion/reconnect sequence not yet driven in guest |
| Four-register native representation matches host semantics | Yes | Yes, exact codec cross-checks | Yes, all three consumers accepted the MemoryDB reply |
| Phase A staged recovery and no-state fail-closed behavior | Yes | N/A | Yes, Phase A native gate |
| MemoryDB restart while guest remains running | Existing rediscovery paths | Host reconnect and generation tests | Not yet verified (test infrastructure limitation below) |

The Phase A gate captures complete serial logs and the staged state-directory
listing under `target/wiseowl-identity-phase-a-evidence/`. It checks Boot 1's
interruption point and synced staged ROOT, compares ROOT bytes through recovery
and commit on Boots 2 and 3, boots without the state disk to check fail-closed
behavior, then reattaches the original disk and checks the same identity and
fingerprint return. Boot 1 originally interrupted after `file_sync(ROOT)` but
before `directory_sync(IDENTITY.STAGE)`. The saved trace showed the file barrier
succeeded while the ROOT directory entry had not yet crossed its own barrier.
A later graceful QEMU shutdown made ROOT readable in a rerun, but that did not
prove the namespace entry would survive the intended interruption. This was a
product persistence bug at the injected interruption boundary, with QEMU's
stop/flush behavior making the test result nondeterministic. The minimum fix
syncs the stage directory after syncing ROOT and before reporting the
`AfterRootFlush` boundary. The Phase A gate now checks each condition
separately, saves the complete serial output and FAT directory listing, and
reports whether the marker, pre-publication stop, or external ROOT read failed.

After a successful Phase A run, the Phase B native gate reuses the same
`target/state-test.img` and checks that MemoryDB loads the Phase A fingerprint
before checking consumer propagation. When Phase B runs by itself, it creates
its own state image and still checks agreement among all four services.

The Phase B native gate records a full serial log at
`target/wiseowl-identity-phase-b-evidence/boot-1-serial.log` and asserts order:
`/state` mount, MemoryDB startup and identity-status readiness, then successful
braind, indexd, and memoryd bindings. It checks that all four services print
the same short fingerprint and that continuity generation is 1. The first
native run exposed a real IPC representation defect: the status reply used five
register words, but the kernel's register transport carries only four. The
fifth metadata word arrived as zero, so memoryd rejected otherwise valid
identity status. Packing the metadata into the header and sharing the exact
four-word decoder fixed the native propagation gate; this was a product
protocol bug, not stale expected output.

The service manager maps the `wiseowl-memorydb` dependency to the actual
`wiseowl.memorydb.v1` readiness endpoint. Memoryd and indexd start after
MemoryDB, and their service capability profiles include MemoryDB access. The
native gate verifies those startup relationships by log order and successful
status queries.

The current QEMU harness launches the guest headless and has no deterministic
guest command channel to stop and restart only MemoryDB while consumers remain
running. The service manager has restart support, but using it without a guest
control channel would not be a real test. In-guest restart and reconnect remain
an explicit native verification gap. Deterministic host integration tests
exercise disconnect, same-identity reconnect, wrong-identity reconnect,
endpoint generation change, and consumer re-query behavior.

For this phase, that one native restart gap is accepted under the documented
host-equivalent verification option: native multi-boot durability and native
four-service propagation pass, while host tests cover restart/disconnect and
reconnect semantics. Phase B is complete with this gap recorded; Phase C has
not been started.

## Identity boundaries

Persistent identity answers which Wise Owl installation this is. Session
authority answers which user/session is interacting now. Logout has no effect
on persistent identity. Foundation Memory is generic software-provided
knowledge, not identity. Changing Foundation Memory or the OS image must not
change IdentityId. No identity value is added to existing MemoryDB records.

Phase B adds no activation lifecycle, installation or activation IDs,
single-active enforcement, migration, backup, restore, clone detection,
witness, signing, or robot/body concepts. Phase C has not started.
