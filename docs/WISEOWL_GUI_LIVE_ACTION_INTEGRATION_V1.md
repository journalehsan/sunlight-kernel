# Wise Owl GUI Live Action Integration v1

## Trust prerequisite

`./tools/test.sh wiseowl-trusted-session-readiness-v1` passed before this
activation work. Its authoritative-session, exact-correlation, trusted-source,
exact-settings-page, diagnostic-trace rejection, GUI-evidence rejection, and
security-boundary markers all passed.

## Production activation boundary

The GUI remains presentation-only. The intended production route is:

```
GUI IPC caller -> graphical-session authority -> VerifiedGraphicalSession
-> GUI Bridge SubmitTurn -> ActionCoordinator -> planner -> policy
-> confirmation authority -> TrustedActionExecutor -> trusted readiness ingress
-> ActionOutcomeObserver -> ActionReceiptLedger -> GUI Bridge events
```

The GUI never supplies authoritative user, session, or PID data, creates a
verified session, confirmation grant, readiness evidence, launch correlation,
or receipt digest, and has no executor, launcher, shell, process-control,
policy-mutation, or MemoryDB-write capability. The existing bridge keeps event
delivery bounded, session-bound, ordered, acknowledged, and terminal-idempotent.

## Presentation and action rules

The old production placeholder text, `Local conversation is online; action
requests are not enabled yet.`, has been removed from `wiseowl-braind`.
Ordinary bounded conversation remains local and does not perform keyword based
execution. The bounded planner is the only action classifier and recognizes
only the canonical `OpenApplication` and `OpenSettingsPage` paths.

Dispatch and readiness are distinct: a dispatch presents `Opening Calculator…`
(`در حال باز کردن ماشین حساب…`) while success is emitted only after trusted
observer readiness. Unsupported requests use `That action is not supported yet.`
(`این عملیات هنوز پشتیبانی نمی‌شود.`); a second executable request uses
`Another action is already in progress.` (`یک عملیات دیگر در حال انجام است.`).

Clarification buttons carry typed candidate IDs, and confirmation keeps the
coordinator's session, request, action, expiry, replay, and proof bindings.
Cancellation is coordinator-owned and never kills or closes an application.
Trusted readiness continues to require capability-authenticated source,
generation, sequence, execution correlation, session, and canonical target;
settings completion specifically requires the exact Control Panel page.

## Verification

- Host: `cargo test --target x86_64-unknown-linux-gnu --package wiseowl-brain
  --no-default-features --features host --lib` passed: 177 tests.
- no_std fixture build: `cargo build --package wiseowl-brain --bin
  wiseowl-braind --features sunlightos,gui-live-action-activation-v1-test
  --no-default-features` passed. Test-only deterministic helpers are feature
  gated; production builds contain no fake authority, readiness source, or
  launcher.
- QEMU prerequisite: `wiseowl-trusted-session-readiness-v1` passed.
- QEMU activation gate: `wiseowl-gui-live-action-activation-v1` is registered
  with the full requested marker set. The attempted run did not start
  `wiseowl-braind` within its 360-second boot window, so no activation markers
  were emitted and the gate is currently failing at service startup.

Manual graphical verification has not been claimed while that QEMU service
startup failure remains unresolved. No action is treated as successful on an
unavailable bridge, missing attestation, unavailable readiness source, receipt
delivery failure, or uncertain request status.

## Runtime plumbing status

The production ownership audit is recorded in
`WISEOWL_PRODUCTION_TRUST_RUNTIME_PLUMBING_V1.md`. `sunlight-sessiond` and the
kernel-authenticated IPC badge are the authority candidates; display launch
traces and Control Panel title/CLI data are not trust evidence. Console action
activation remains explicitly deferred until the sessiond authority adapter and
source-authenticated lifecycle ingress are installed.

## Delegated authority milestone

`WISEOWL_DELEGATED_SESSION_LIFECYCLE_IPC_V1.md` installs the previously
missing Console identity delegation, canonical Console launch registration,
one-shot session authority proof, opaque launch-context contracts, and separate
display/Control Panel lifecycle endpoints. These interfaces do not activate
this integration: Console turns are still not routed to the coordinator and
the Sunlight production launch adapter returns failure for both live action
methods. Availability is informational, not an execution grant.
