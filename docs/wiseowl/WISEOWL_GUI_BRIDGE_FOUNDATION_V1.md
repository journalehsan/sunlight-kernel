# Wise Owl GUI Bridge Foundation v1

## Scope

This milestone supplies the typed foundation needed for future live Wise Owl
actions.  It does not change the production ConsoleUi placeholder into an
execution path.  In particular, no GUI request reaches the executor in this
milestone.

The audit that motivated this work is retained in
`WISEOWL_GUI_LIVE_ACTION_INTEGRATION_V1.md`.

## Foundation types

`wiseowl-brain::gui_bridge` owns a presentation-only boundary:

- `CoordinatorPresentationUpdate` carries conversation, request, optional
  action, session, monotonically increasing sequence, terminal state, and a
  bounded `PublicPresentationPayload`.
- `CoordinatorPresentationKind` covers accepted/no-action, clarification and
  confirmation, policy, dispatch, awaiting outcome, terminal outcome,
  cancellation, expiry, session invalidation, and registry invalidation.
- The payload contains only a localized key and bounded public text. It cannot
  expose policy rules, grants, executor state, launch tokens, executable paths,
  or raw IPC data.

The daemon will be the only producer of coordinator updates. The GUI receives
and renders them; it does not derive coordinator state itself.

## Verified GUI session binding

`GuiSessionBindingAuthority` issues an immutable
`WiseOwlGuiSessionBinding` from a `VerifiedGraphicalSession` whose fields and
constructor are private to the bridge.
The binding contains the session and requester identity, locale, runtime and
registry generations, issue/expiry times, and an authority integrity digest.
The digest and all fields are private, so a GUI can carry a binding but cannot
construct, alter, or extend it.

Before forwarding a future GUI request to the coordinator, the daemon must
verify the binding against the currently verified graphical session and current
runtime/application/settings generations. Expired, wrong-user, wrong-session,
stale-runtime, and changed-registry bindings fail closed.

The active graphical-session source is intentionally not guessed from a GUI
provided id. Wiring the existing session service to produce
`VerifiedGraphicalSession` is the next daemon integration step.

## Event delivery

`GuiEventBroker` is a bounded daemon-owned queue (default: 64 events). Its
events contain protocol version, event id, conversation id, session id,
monotonic sequence, terminal flag, and either a presentation update or a
receipt summary.

Delivery is session-filtered and acknowledgement is bound to the same session.
Events remain until acknowledgement; a wrong-session acknowledgement is
rejected. Decreasing sequences are rejected and an identical terminal event is
idempotent. This is an intentionally narrow bridge queue, not a generic
publish/subscribe system. A future endpoint integration can use it for bounded
long-poll or subscription delivery without replaying user requests.

## Trusted readiness correlation

`TrustedReadinessCorrelation` is daemon-owned. It registers an accepted
execution only with the executor-minted opaque `LaunchCorrelationToken`, exact
execution id, session id, canonical target id, and whether the target is a
settings page.

It accepts `CorrelatedGuiReadinessEvidence` only when all of these match and
the evidence sequence increases:

- Application evidence comes from `DisplayServer` or `ApplicationRegistry`.
- Settings evidence comes from `ControlPanel` and must name the exact requested
  canonical settings page.
- Evidence is bound to the exact execution, opaque token, session, target, and
  source generation.

PID reuse, a window title, an application display name, elapsed time, and
diagnostic logs are insufficient and are not accepted by this interface.
Diagnostic launch traces remain diagnostic-only; they are never outcome
evidence. The GUI cannot register an execution or inject readiness evidence:
the required `VerifiedGraphicalSession` and executor-minted
`LaunchCorrelationToken` cannot be constructed or obtained by GUI code, and
the daemon owns the authoritative bridge instances.

The subsequent adapter will convert accepted bridge evidence into the existing
`ObservationEvidence` for `ActionOutcomeObserver`. No conversion is made in
this foundation, so no synthetic `Ready` outcome can occur.

## Receipt delivery

`ReceiptSealedView` carries only the receipt id, action id, terminal status,
localized operation/target keys, and `readiness_observed`. It intentionally
omits integrity digests, audit trails, execution ids, correlation tokens, and
cross-session private identifiers. Receipt delivery is downstream-only and
does not authorize another action. The Activity page remains deferred.

## Bounds and capability boundary

The module defines default event capacity 64, deduplication budget 128, public
text 256 bytes, and localized keys 64 bytes. Session binding, event delivery,
and evidence ingestion are daemon-owned. The GUI retains only display/input,
verified-session binding carriage, and typed bridge IPC; it receives no spawn,
executor, shell, process-control, service-control, policy-mutation,
confirmation-grant, MemoryDB-write, or Outcome Observer injection capability.

## Verification

Host tests use the explicit standard target because the workspace default is
`x86_64-unknown-none`:

```sh
cargo test --target x86_64-unknown-linux-gnu --package wiseowl-brain --lib gui_bridge
```

The tests cover binding verification, wrong-user/expired/stale rejection,
presentation ordering, session-filtered acknowledgement, bounded queue
handling, exact application correlation, wrong source rejection, and exact
settings-page activation.

The deterministic no_std QEMU gate is:

```sh
./tools/test.sh wiseowl-gui-bridge-foundation-v1
```

It builds `wiseowl-braind` with
`gui-bridge-foundation-v1-test` and emits the documented
`[WISEOWL-GUI-BRIDGE] ... PASS` markers using typed fake authoritative sources.
It does not launch an application and does not use trace text as evidence.

## Deferred

The following remains deliberately deferred: daemon session-service adapter,
native ConsoleUi bridge operations, authenticated display/control-panel IPC
adapters, conversion into `ActionOutcomeObserver`, receipt-ledger publication,
and all GUI-triggered execution. Those changes are required before the
action-disabled production placeholder can be replaced.
