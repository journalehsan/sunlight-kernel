# M5.3R3 authenticated membership security contract

Status: **BLOCKED**. Production FastLane and production restricted admission
remain disabled. This contract states required behavior; it does not claim that
the missing publication or attribution boundary is implemented.

Process ownership is `(PID, incarnation)`, allocated before resource setup.
Numeric PID, UID, parentage, process group and PTY ID alone confer no login
membership. VM ownership additionally requires the live address-space identity
and generation; successful exec replaces the VM without transferring old backing
receipts to the new VM or another process.

The existing capability broker Account registry is the authority to extend.
`SessionRef { id, epoch }` must designate a live, active, unexpired registration
created by the trusted session service using a one-use Auth proof and exact
owner/issuer identities. A bare SessionRef is an identifier, not authorization.
A future launch attachment must authorize one operation, bind the exact recipient
process incarnation and requesting authority, and be consumed at publication.
Membership validation, capability installation and process publication must commit
under the existing lifecycle lock ordering, or use explicit equivalent lifetime
pins. Validation followed by an unlocked publication is insufficient. Failure
must publish neither a runnable child nor externally reachable child authority.

Children require authorized inheritance of the same live session. Exec retains
membership only while its process incarnation remains the same. Reparenting
cannot migrate membership. Desktop, terminal, detached and service launches need
the same authority handoff. System-owned and unauthenticated processes require an
explicit policy; the present process-only quota fallback is not complete login
accounting. This milestone does not change that policy implicitly.

For shared services distinguish allocator, authenticated requester, resource
owner, mapping holder, quota payer and final reclaimer. Charge an immutable,
kernel-validated request owner, with operation-scoped delegation if a service
allocates for it. Service credentials, a caller-supplied session, UID and a bearer
SHM token cannot authorize charging another session. Unsupported attribution must
be rejected by the future authenticated allocation API. Existing legacy APIs are
not claimed to implement this boundary.

Keep 512 MiB per process and 1 GiB per authenticated session. Preflight both
budgets and ledger capacity atomically before allocating backing; failure rolls
back every provisional charge. A successful receipt belongs to its original
owner/session until backing reclamation. Requester exit, exec, service restart or
session termination must never move or erase that charge while a peer retains
backing. Session termination must fence new attachment, delegation and reservation
before retiring mappings. Existing Account revocation alone does not establish
this rule for legacy SHM.

Retire every mapping, complete the matching synchronous remote invalidation for
CPUs genuinely retaining that VM, then free backing frames, then consume the
move-only quota receipt exactly once. Acknowledgments from unrelated mappings or
requests do not qualify. No frame may be reused while stale translations remain.
Fixture inventory closure and all-system resource closure require separate
measurements. Missing measurements remain unqualified.

Gate A blocks session-wide implementation: current `SpawnRequest`, spawn broker
handoff and scheduler process record lack an Account-authorized launch attachment.
Extending that wire protocol and all launch paths safely is prerequisite work.
No competing session authority or PTY-to-login inference is introduced here.
