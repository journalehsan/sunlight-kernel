# SunlightD D3.6R2: controlled retirement audit

Status: **blocked before implementation**. The current kernel and SunlightD
interfaces cannot prove that termination of a surviving service is safe after
its manager dies. RecoveryBlocked remains the enforced outcome for every
successor. No retirement, ownership release, or reactivation path was added.

This audit follows the D3.6R ownership contract. Its historical validation
results are in `target/d36r-evidence/validation-summary.json`; they are not new
D3.6R2 measurements. D3.6R2 native tests and regression gates were not run.

## Audited authority and lifecycle

The relevant implementation is in `ipc/src/supervision.rs`,
`kernel/src/arch/x86_64/syscall.rs`, `kernel/src/sched/mod.rs`,
`kernel/src/ipc/mod.rs`, `services/init/src/main.rs`, and
`services/sunlightd/src/main.rs`.

* The kernel binds Init to its exact PID and process generation. Under the
  scheduler lock, Init can launch a manager, which receives an exact identity
  and monotonic supervisor epoch. A successor is diagnostic-only and has no
  active lifecycle authority.
* A bounded ownership record retains a nonreused record ID, service PID and
  process generation, original manager identity and epoch, spawn path, tracked
  flag, and optional physical-reap counters. It contains no controlled
  retirement eligibility, resource-rights snapshot, or successor grant.
* The current manager's D2 reap receipt is watcher- and generation-qualified.
  Manager death clears its watcher association. A successor cannot release the
  old receipt or call the existing manager-only release path.
* `ProcessExit` and the kernel reaper distinguish exit from full reaping. The
  reaper performs existing endpoint/capability and address-space cleanup before
  marking a process Reaped and attaching physical counters to the retained
  record. Record retention does not itself pin private memory.
* Kernel `kill`/signal handling has a PID plus optional generation validation
  path, but it is not an operation authorized by an old ownership record. The
  diagnostic successor syscall allowlist blocks signaling. Enabling it would
  require a new kernel authorization path, not merely a userspace call.
* D2 leases and D3 dependency holds live in the manager. On successor startup,
  SunlightD explicitly reports both as unknown, does not rebuild its service
  graph, and rejects activation and lifecycle mutations.

## Minimum authority contract for any future retirement

A safe narrow operation would need all of the following evidence checked
atomically by the kernel under existing scheduler synchronization:

1. Init's authenticated launch/restart authorization names the exact successor
   manager PID, process generation, and supervisor epoch.
2. The original manager identity and epoch match the retained record, and the
   kernel marks that exact manager fully reaped before authorizing a successor.
3. The requested record ID and exact service PID/process generation match one
   record owned by that original manager incarnation. PID, service name, path,
   and caller-supplied generation alone grant no authority.
4. Before manager loss, trusted configuration and kernel-enforced spawn/resource
   policy marked that service incarnation explicitly disposable and stateless.
   The mark must be bound to that exact record and generation.
5. An authoritative resource ledger proves there are no live leases, dependency
   holds, accepted work, persistence obligations, or external shared-resource
   references that make termination unsafe. Unknown is a deny result.
6. A per-record kernel transition serializes retirement requests. It fences
   duplicate lifecycle operations, issues termination to the exact process
   generation, and stores durable-in-boot transition evidence. It cannot claim
   that this management fence revokes arbitrary data-plane access.
7. Reactivation is permitted only after exact process exit, existing kernel
   reaping and cleanup completion, and safe resolution of that same ownership
   record. A new activation must receive a fresh service generation and endpoint
   authority through the existing single-flight path.

The successor must receive only this per-record retirement right, not the old
manager's spawn, signal, receipt-release, or general service-control authority.
Repeated requests must return pending/completed/blocked based on kernel-owned
state. Stale record IDs and generations must fail, and record IDs must never be
reused while stale references can exist.

## Missing prerequisites

The audited implementation does not provide steps 4 or 5. A spawn path identifies
the unit but does not attest statelessness or a bounded resource contract. The
kernel ownership record does not track the service's complete capability and
endpoint reference graph, shared-memory borrowers, or D2/D3 manager-local state.
The existing D2 watcher is deliberately cleared when the manager dies, and there
is no authoritative replacement source for leases and dependency holds. Physical
reaping is not evidence that independent clients released endpoint tokens or
shared mappings.

Consequently there is no eligible service incarnation that the current system
can prove satisfies the proposed retirement preconditions, even if its test
binary is synthetic and its unit definition has no declared dependencies. A
configuration label alone would not authenticate runtime resource absence.
Adding a signal syscall for the successor or clearing unknown holds would
weaken the existing ownership contract. This phase therefore stops before
R2B–R2G rather than asserting an unverified proof.

## Implementation and evidence status

| Area | D3.6R2 result |
| --- | --- |
| R2A authority audit | Completed; proof contract and missing prerequisites recorded above |
| R2B eligibility policy | Not implemented; no trustworthy pre-loss resource eligibility evidence |
| R2C kernel retirement operation | Not implemented; existing record lacks authorization and safe-release state |
| R2D retirement/reactivation state machine | Not implemented; RecoveryBlocked remains default |
| R2E interruption/race handling | Not run; there is no operation to inject faults into |
| R2F terminal record cleanup | Existing D3.6R bounded retention remains; no new record reclamation |
| R2G synthetic native integration | Not run; the required authority proof is unavailable |
| Host/native regression and performance evidence | No new runs; historical D3.6R results remain historical |

In D3.6R, the record table has 64 slots and IDs do not wrap or get reused.
Capacity exhaustion blocks additional tracked spawns. Existing active-manager
cleanup can remove records with exact reap evidence under its existing policy;
records retained after manager loss remain unresolved and are not automatically
deleted. No D3.6R2 cleanup or counter-exhaustion change was made.

The previous report's 14 unresolved ownership records and its 1/2/4 CPU results
are historical D3.6R measurements only. There are no D3.6R2 raw logs, latency
measurements, resource-drift measurements, or validation summary. The companion
`target/d36r2-evidence/validation-summary.json` records this blocked audit and
explicitly marks execution and measurements as not run.

## D3.6R3 prerequisites

Before controlled retirement or general adoption can be reconsidered:

* Define an authenticated, pre-loss eligibility grant tied to one service
  incarnation and enforce its stateless/disposable resource limits at spawn.
* Provide authoritative lease, dependency, accepted-work, endpoint, and shared
  mapping accounting that survives manager loss, or keep any service using
  those resources RecoveryBlocked.
* Add a narrowly scoped, per-record kernel transition with exact original and
  successor manager identities, generation checks, at-most-one-operation
  semantics, persisted-in-boot interruption evidence, and terminal cleanup
  rules.
* Define how endpoint/capability revocation interacts with existing clients and
  shared mapping lifetime; physical process reap alone is insufficient.
* Only after those prerequisites exist, add a disposable synthetic service,
  model/race tests, interruption injection, and native 1/2/4 CPU evidence before
  considering any broader service class.

General live-service adoption, production-service retirement, and full D3.6
recovery remain unsupported.
