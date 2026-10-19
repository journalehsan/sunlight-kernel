# SunlightD D3: dependency-aware lifecycle

## D3A audit and contract

The D1/D2 source and retained `target/d2-evidence/validation-summary.json` were
reviewed before editing. Its 164 host checks, 1/2/4 CPU gates and 3148 KiB
synthetic reclamation are historical results, not D3 measurements.

1. Dependencies are bounded manifest `Requires` and `After` lists (eight each),
   with a 32-unit graph. D1 plans managed on-demand closures before reservation
   and advances one spawn per event-loop turn. Both lists gate managed demand
   startup; `Wants` is parsed but advisory. There is no optional dependency
   failure contract. D3 preserves these semantics.
2. Each enabled demand definition can be activated independently through
   ACTIVATE. Resident/external providers must already satisfy their existing
   nameserver contracts. No critical service is converted.
3. D2 combines primary-process zero-work/zero-transaction reports, bounded
   administrative leases, minimum uptime, grace, authenticated QUIESCE/DRAINED,
   an irreversible commit and generation-qualified reaping receipts. Existing
   dependency holds infer retained consumers from state; they do not bind the
   provider incarnation. Lost reports retain residency.
4. D2 client leases identify both owner process generation and service runtime
   generation. They cannot transfer across restarts. Dependency reservations
   need consumer activation identity before spawn and provider runtime binding
   before the consumer can start. They must outlive consumer quiescing and only
   release after retained process ownership ends.
5. Kernel-stamped IPC badges identify the immediate sender; the trusted manager
   can query its live kernel process generation. Readiness additionally validates
   the actual endpoint owner. Payload identities confer no authority. Spawn
   profiles use the existing ServiceCapability mask and kernel IPC mediation.
6. DISCOVER resolves inactive logical definitions and returns metadata, never a
   SEND capability. Nameserver lookup separately checks invocation lookup rights.
   A passed manager token does not bypass kernel activation mediation.
7. Files' image thumbnails run in a shared-address-space worker thread with an
   Arc mailbox, one decoder, generation cancellation, bounded worker and grid
   caches, and the application's file access. PDF previews call DocServ. The
   DocServ thumbnail helper belongs to the larger DocServ process. It performs synchronous
   Open/RenderPage/Close operations; the native server owns multipart uploads,
   downloads and document handles. Separately, `services/sunlight-thumbd` is a
   standalone `thumbd` process, disabled by the compiled default manifest due to
   prewarming failures. It synchronously generates during PATH_END despite its
   asynchronous description. It reads as root, stores one multipart path by PID,
   and writes temporary image/metadata cache files before rename. Cache keys are
   path/size-class hashes, without source content/version identity. It does not
   authenticate the requester's file authority, expire partial uploads, report
   verified readiness, or implement drain. No `thumbnailer` alias was found.
8. Neither path implements the complete D2 primary-process admission/drain
   contract. Extracting Files' worker requires IPC buffer ownership, cancellation,
   cache invalidation, authorized file access and client-death cleanup. Unloading
   DocServ requires draining every document handle and multipart session.
   Enabling thumbd also requires caller-authorized file handles (or equivalent
   existing VFS checks), safe multipart ownership/cancellation, transactional
   cache draining, malformed-source bounds and prewarming repair. Merely adding
   lifecycle messages would expose a privileged path-reading proxy.
   These are separate architectural work; the real pilot remains blocked.

The manager has one mutable owner of its registry, no registry mutex and no
recursive activation lock. Kernel scheduler/process/capability locks retain
their existing ordering. Process exit destroys endpoints and capabilities;
Finished is not Reaped. Cooperative services retain watches until the kernel
reaper confirms active CPUs, borrowers and outgoing stacks are clear. Exec
replaces the address-space incarnation; old readiness/endpoint identities must
fail validation. Service definitions have no runtime dependency mutation API.

D3 supports static edges only. A plan is bounded by 32 units and rejects cycles
before state changes; pending activation callers remain bounded at 16. Duplicate
Requires/After edges acquire one ownership record. Each consumer generation owns
one hold per direct provider generation, transitively through its providers.
Shared providers have independent consumer holds. No CPU-activity inference is
used. Ordinary client cancellation removes its waiter; an admitted shared
activation continues to completion or its existing five-second timeout.

The safety invariant for D3B is: reserve the complete closure's ownership before
spawning; bind provider identity before dependent spawn; release only after a
failed unspawned consumer or the retained consumer runtime has ended. Shutdown
initiation and DRAINED do not release its dependencies. A quarantined runtime
retains them. D3C prevents Ready publication and reuse after required-provider
identity loss. Recovery requires an explicit new request after cleanup and the
existing cooldown; there is no automatic chain restart.

## Implementation and validation status

D3A audit: unchanged 25 core and 11 manager checks passed before editing.
D3B: four new core ownership checks and five manager adapter checks pass.
D3C: two further manager checks pass. D3D: 43 IPC checks pass, including one
new scoped-policy test. D3E native and default resident gates are complete.
Generated logs and the machine-readable status live in `target/d3-evidence/`.


## D3B ownership implementation

`dependency.rs` stores at most 128 direct ownership edges. Complete plans are
capacity-checked and reserved atomically before activation state changes. A
consumer identity is (unit slot, checked activation generation); a provider has
that identity plus a once-bound (PID, kernel process generation, endpoint).
SunlightD's sole event loop authenticates acquisition from the trusted compiled
manifest and owns release; there is no public dependency acquire/release RPC.
Existing D2 administrative client leases keep their separate expiry semantics.
Dependency holds have no expiry while a retained consumer can use the provider.

The existing spawn path also reserves/binds edges for legacy explicit Start.
Managed demand consumers use explicit holds; resident consumers retain D2's
conservative static-state holds. Requires and After duplicates collapse. Shared
providers retain one hold per consumer, not per activation requester or endpoint.
For A -> B -> C plus A -> C there are exactly three ownership records. Exiting A
releases two; B continues to hold C until B's own cleanup.

Release is idempotent and generation-qualified. Failed unspawned generations
release on the next existing supervision turn. Cooperative runtimes release only
after the exact reaping receipt, including crashes and forced external exit.
Noncooperative D1 consumers release after validated incarnation death under D1's
existing contract. Quiescing, commit initiation, timeout quarantine and Failed
with a retained runtime do not release holds. Exec cannot inherit bindings.
Dependency replacement is not automatic: identity loss invalidates the consumer,
which must finish cleanup before a new generation can acquire new identities.

## D3C readiness and recovery

Spawn and endpoint registration are intermediate states. READY requires the
expected live sender, local endpoint and generation plus the transitive required
provider closure and previously bound ownership records. Warm resolutions and
completed deferred activation replies revalidate that closure. Supervision
propagates observed required-provider failure to Starting/Ready/IdleCandidate
consumers without table-order dependence. Running consumers become Failed and
unavailable through the manager; their existing direct operations are governed
by service-specific error handling. No synthetic manager can guarantee a provider
will not crash immediately after a validated snapshot.

A dependent already quiescing/stopping continues its admission/drain protocol;
its required holds remain until cleanup. Missing providers at startup fail within
the existing absolute five-second activation budget. No autonomous restart,
degraded optional mode or dependency replacement is added. Quarantine requires
an explicit service-safe exit or existing noncooperative administrative cleanup.
An uncooperative process may intentionally retain providers: releasing underneath
its work would violate the contract. This is residency retained for safety, not
an orphaned owner record. Cancellation of every waiting client does not cancel
an already admitted shared startup; its timeout remains bounded.

Definitions/edges have no runtime mutation API. Enable/disable does not alter an
edge or transfer its generation. A stopping provider returns Busy before any
reservation; a caller retries after reaping and cooldown. All state updates are
serialized by the existing daemon loop, including D2's bounded handshake; queued
requests are evaluated after the irreversible boundary. No new lock or syscall
is introduced. Bounded metadata metrics expose ownership counts; no per-message
logging is added.

## D3D scoped activation

The existing ServiceCapability system adds `demand-activate-test`. Kernel IPC
mediation accepts ACTIVATE only for the exact synthetic `d3-parent` logical name
(with optional `.service` suffix), using the caller's actual process profile.
Malformed names, arbitrary service/dependency names, legacy controls, residency
lease administration and resource accounting remain denied. The manager's
trusted static closure may start only that definition's declared dependencies.
No capability or service operation is forwarded on the client's behalf.

The activation capability grants neither SunlightD lookup nor the metadata-only
DISCOVER operation. The existing ServiceNotify or ServiceLifecycle profile is
required for DISCOVER. Transport access can be passed explicitly or separately
authorized with ServiceNotify; invocation requires the existing independent
nameserver lookup and service operation policy.
The synthetic UID 1000 native application receives a manager SEND token and only
the scoped activation capability, with no ServiceLifecycle authority. Its payload
cannot forge readiness or lease ownership. Defaults and production manifests
remain administrative. General per-service ACL configuration is a D4 follow-up;
this fixed allowlist deliberately covers only the test definition, which exists
only with the explicit dependency-test feature.

## D3E native evidence and limits

The synthetic native matrix passed with 1, 2 and 4 virtual CPUs under QEMU TCG.
Each CPU run completed six activation/unload cycles, including concurrent
requests for a shared dependency, provider crash during parent startup, client
death during activation, caller policy checks, D2 draining and confirmed
reaping. Baseline D1, D2 and IPC foundation assertions also passed. Host suites
report 177 checks (164 historical checks retained plus 13 new focused checks);
scheduler lifecycle probes and the identity phase C feature compile passed.
Raw command output and serial transcripts are in `target/d3-evidence/`.

The measured graph had 14 managed processes before activation, 17 while its
three-unit dependency closure ran, and 14 after confirmed unload. It held three
direct dependency leases while active. Each cycle reclaimed 2,322 private
frames, 36 page-table frames and three shared pages (9,444 KiB total physical
frame delta); shared pages are reported separately and are not attributed to
one provider. Free frames returned to the same starting count after each cycle
on all CPU counts. Across six cycles, cold-chain Ready time was 340–370 ms on
one CPU, 310–330 ms on two CPUs, and 290–330 ms on four CPUs. Warm endpoint
resolution was 0–10 ms at the timer's 10 ms resolution. Confirmed whole-graph
unload took 2,210–2,260 ms including staged grace/drain, and reported idle CPU
time was zero. These are synthetic QEMU measurements, not production service
performance claims. The historical D2 independent fixture reclaimed 3,148 KiB;
the graph footprint is larger because it runs three test processes. An
always-running versus real D2/D3 service comparison was not measured.

The default resident boot compatibility gate is recorded separately in
`target/d3-evidence/resident-gate.log` and its status in
`target/d3-evidence/validation-summary.json`. No production manifest or
resident service was converted. Thumbnailer remains a blocked pilot for the
authority, multipart ownership, cache correctness, cancellation and drain gaps
listed above; the synthetic service is the only native integration.

### D4 prerequisites and remaining risks

- Define manifest-level per-service activation ACLs instead of the fixed
  synthetic test capability allowlist.
- Add caller-authorized file handles, request cancellation and bounded upload
  ownership before extracting thumbnail work into a standalone service.
- Make thumbnail cache keys include source identity/version and define safe
  cleanup on deletion and client disappearance.
- Add manager restart/reconciliation semantics for service state and dependent
  owners; D3 intentionally does not persist leases across manager restart.
- Add a longer SMP soak and measurements against a production workload. Current
  memory evidence covers six cycles per CPU count in QEMU TCG.
- Define service-specific behavior when a required provider fails after Ready.
  D3 marks the dependent unavailable to new managed operations and retains its
  dependency holds until safe cleanup; it does not interrupt arbitrary in-flight
  service work or restart a chain automatically.
