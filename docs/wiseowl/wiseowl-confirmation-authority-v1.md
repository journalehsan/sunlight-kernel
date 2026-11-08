# Wise Owl Confirmation Authority v1

Confirmation Authority v1 turns an exact confirmation-required policy decision
into a typed, session-bound, single-use grant and then performs final
revalidation. It stops at an immutable `ReadyForExecution` data envelope. This
milestone adds no executor, execution result, daemon, UI, service call, IPC
protocol, command representation, syscall, callback, general reasoning, or
learned memory.

## Boundary and terminology

The non-executing path is:

```text
Validated Action Intent
        |
Policy Decision
        |
Confirmation Challenge
        |
User Confirmation Response
        |
Confirmation Grant
        |
Final Revalidation
        |
ReadyForExecution
        |
Execution Result (future; not implemented)
```

These objects have intentionally different meanings:

- A **Policy Decision** is the versioned Policy Engine's result for a validated
  intent. It says `Allowed`, `Denied`, `ConfirmationRequired`, or `Unknown` and
  records the required confirmation level. It is neither a prompt nor user
  authorization.
- A **Confirmation Challenge** is an immutable, expiring request for one exact
  intent, policy version, Runtime Snapshot generation, and session. It carries
  bounded public summaries and an anti-replay nonce, but no internal rule
  details. `Denied`, `Unknown`, and confirmation level `None` cannot create one.
- A **Confirmation Response** is a typed reply to an existing challenge. Soft
  approval is explicitly selected. Strong approval includes the challenge
  nonce. Critical approval includes the nonce, an exact-target digest, and a
  structured consequence acknowledgement. Rejection, cancellation, expiry,
  and invalidity are explicit variants. There is no arbitrary text proof.
- A **Confirmation Grant** is immutable authority output after a valid approval.
  It binds the challenge and intent IDs, complete intent digest, operation,
  target, parameters digest, provenance, policy version, Runtime Snapshot
  generation, session, responder, level, and validity interval. Its issuing
  authority tracks it as single-use.
- **ReadyForExecution** is a capability-neutral immutable data envelope
  containing the intent, decision, optional grant, successful final-validation
  marker, policy and Runtime Snapshot versions, readiness time, and bounded
  audit ID. It has no execute method or effect-bearing field.
- An **Execution Result** would describe the result of a future executor. No such
  type or execution path exists in this milestone.

## Why confirmation is not a boolean

A boolean cannot identify what was approved, who approved it, which session it
belonged to, what policy and runtime facts were current, when approval expires,
or whether it has already been used. A loose `true` could therefore be reused
for a changed target or future action.

The v1 grant instead binds every security-relevant field. The intent and its
operation-specific parameters use domain-separated SHA-256 identity digests;
the target is also retained as its exact typed value. Authority-owned lifecycle
state prevents response replay and grant reuse. Any mutation, mismatch,
unknown state, stale snapshot, policy change, inactive session, unauthorized
responder, expiry, or confirmation-level downgrade fails closed.

Approval acceptance itself receives trusted current intent, Runtime Snapshot,
and session authorization context. The authority revalidates these before it
creates a grant, so approval submitted after intent mutation, policy change,
runtime staleness, session loss, or responder change creates no grant. The same
checks run again before readiness.

## Interaction contracts

`ConfirmationView` is the only presentation object. It contains a title, action
summary, target summary, consequence summary, confirmation level, explicit
approve/reject/cancel choices, and expiry. It never contains an Action Intent,
and it explicitly has no default choice. A trusted UI can render the view but
cannot create or modify an intent; it only submits a typed response for the
challenge ID it received.

Soft confirmation requires `SoftExplicit`. Dismissal, missing response, and
timeout never produce this value. Strong confirmation additionally requires
the exact challenge nonce. Critical confirmation cannot be downgraded: its
closed proof contract requires the exact nonce, exact-target restatement digest,
and structured consequence acknowledgement. This is the v1 equivalent of a
stronger typed-confirmation interaction; no typed phrase or conversation text
is accepted or stored.

## Lifecycle and final revalidation

The bounded authority owns an explicit forward-only lifecycle:

```text
Created -> Issued -> Approved -> Consumed
                  -> Rejected
                  -> Cancelled
                  -> Expired
                  -> Invalidated
```

`Approved` means only that a valid grant was created. `Consumed` means only
that the grant was used to produce one readiness envelope. Neither state means
an action ran.

Before readiness, the authority validates the intent again and checks its exact
binding against the original decision and grant. It re-evaluates the current
immutable policy, checks policy version and required confirmation level, checks
Runtime Snapshot generation and bounded age, requires a known active session
and authorized responder, verifies challenge/grant expiry, and rejects a
consumed or changed grant. `Unknown` never becomes ready.

Allowed decisions can produce an envelope without a grant. A
confirmation-required decision must have the exact approved grant. Denied and
unknown decisions can never produce readiness.

## Bounded redacted audit

The fixed-capacity ring records challenge creation/issuance, response outcome,
grant creation/invalidation/consumption, and readiness production/denial. On
overflow it evicts the oldest event and increments a counter. Events contain
only typed IDs, levels, states, target kinds, public denial reasons, and audit
IDs. They never contain target values or full paths, parameters, secrets,
critical proof content, conversation text, or policy rule structure.
