# Wise Owl Trusted Session Attestation and Readiness Evidence v1

## Scope

This milestone adds the bounded trust boundary needed before GUI action
activation.  It does not enable GUI-triggered execution: the native Console UI
handler continues to return the action-disabled response and no GUI code gains
an executor, launcher, shell, process-control, policy, or readiness-injection
capability.

## Authoritative graphical-session source

The source of truth is the active graphical-session authority (the sessiond /
desktop-session integration), not the GUI request body.  The implementation in
`wiseowl-brain::trusted_session_readiness` accepts only an
`AuthoritativeGraphicalSession` installed by that authority.  The type is
crate-private; no GUI IPC, planner, coordinator, executor, or production test
adapter can construct it.

`TrustedGraphicalSessionAuthority` issues immutable
`VerifiedGraphicalSession` values only for the registered Wise Owl GUI process
and instance of the active session.  An issued value carries an opaque
attestation id, session and authority generations, bounded locale, registry and
runtime generations, and an expiry.  The public type has no constructor.

The authority validates caller PID and GUI instance against its authoritative
record, then validates every use against active session identity, requester,
session generation, authority generation, and expiry.  GUI-supplied session
IDs, user IDs, and payload PIDs are therefore not authoritative inputs.

Attestations are revoked on session logout/deactivation, GUI-process exit, and
authority restart.  A changed authority generation invalidates all old
attestations; a GUI restart requires a new instance attestation.  The bounded
attestation table holds at most 32 records and fails closed when full.

## Launch correlation and lifecycle evidence

The existing executor-created opaque `LaunchCorrelationToken` remains private
to trusted execution and lifecycle paths.  It is never sent to the GUI, window
title, command line, environment, or diagnostic log.

`TrustedReadinessIngress` registers accepted executions and accepts evidence
only with a `TrustedReadinessSourceCapability` held by the trusted display
server, application-registry, or Control Panel adapter.  The capability has no
public constructor, so the GUI cannot register a source or inject evidence.
The ingress then requires the existing exact correlation table to match:

- execution correlation token;
- session ID and canonical target ID;
- source class and source generation;
- strictly increasing source sequence.

The queue is bounded to 64 evidence records, source-sequence tracking is
bounded to 32 sources, and registered executions use the existing bounded
correlation table.  Overflow and sequence/correlation failures are rejected;
they never become readiness.

Application evidence may be `ApplicationRegistered`,
`FirstWindowRegistered`, or `ApplicationReady`, according to the target's
registry-defined observer contract.  PID-only, process-created, window-title,
elapsed-time, and diagnostic-trace information have no ingress representation
and cannot satisfy the observer.

For settings, only the Control Panel source can emit page evidence, and it must
match the exact canonical requested page.  A Control Panel process, home page,
different page, display title, or diagnostic trace is rejected before it reaches
the Outcome Observer.

## Delivery and restart semantics

The ingress uses typed, bounded records and does not parse logs.  Source
generation changes are rejected; trusted adapters must issue a fresh capability
after restart.  Missing evidence remains incomplete/unknown and must never be
treated as Ready.  This milestone does not add persistence or recovery that
would turn an interrupted observation into success.

## Testing and verification

Host tests cover active-client issuance, wrong caller/instance rejection,
cross-process replay, logout/process-exit/authority-restart revocation, exact
token/session/target correlation, source authentication, monotonic sequencing,
and exact Control Panel page activation.

Run:

```text
cargo test --target x86_64-unknown-linux-gnu --package wiseowl-brain \
  --no-default-features --features host
```

The deterministic no_std QEMU gate is
`wiseowl-trusted-session-readiness-v1`.  It uses only feature-gated in-process
trusted adapters; it neither launches an application nor parses diagnostics.
The test constructor remains behind `cfg(test)` and the gate feature only.

## Deferred

GUI action activation, real Calculator and Control Panel launches, receipt
delivery, Activity/Health/Privacy pages, notifications, voice, and new action
types remain explicitly deferred.  The next activation milestone must obtain a
fresh verified session through this authority and feed observer outcomes only
through the capability-bound readiness ingress.
