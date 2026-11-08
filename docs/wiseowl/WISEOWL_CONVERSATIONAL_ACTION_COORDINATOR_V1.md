# Wise Owl Conversational Action Coordinator v1

The coordinator owns the bounded, session-bound lifecycle of one action
request. It connects existing typed components; it does not add reasoning,
target resolution, policy rules, executable operations, IPC, or launch
capabilities.

## Component boundaries

```text
bounded conversation input
  -> Coordinator
  -> Planner
  -> Action Intent validation and Policy
  -> Confirmation Authority when required
  -> ReadyForExecution
  -> Trusted Action Flow
  -> Executor
  -> presentation-safe response
```

- **Planner** recognizes the small action grammar and resolves a registered
  application or settings-page target. It proposes; it cannot authorize or
  dispatch.
- **Coordinator** advances one action through explicit states, retains bounded
  bindings, rejects conflicts/replay, and translates typed outcomes into
  public views. It is not a planner and never resolves a target.
- **Policy** evaluates a validated intent against the current immutable policy
  and runtime snapshot. The coordinator obeys the typed result; it does not
  inspect or reproduce policy rules.
- **Confirmation Authority** creates an exact challenge, validates the typed
  response, and creates a single-use grant. Clarification chooses what a
  request means; confirmation authorizes an already exact action.
- **Trusted Action Flow** is the only route from intent through policy,
  optional confirmation, final readiness, and execution.
- **Executor** consumes readiness and passes one narrow typed
  `OpenApplication` or `OpenSettingsPage` request to the trusted adapter. The
  coordinator has no direct adapter route.
- **Conversation presentation** receives `ActionResponseView` and optional
  `ConfirmationView`: localized text, opaque clarification choices, public
  reasons, and expiry—not nonces, digests, executable targets, registry
  internals, service handles, or internal errors.

## State and binding

```text
Idle
  -> Planning
  -> AwaitingClarification -> Planning
  -> EvaluatingPolicy
  -> AwaitingConfirmation
  -> PreparingExecution
  -> Dispatching
  -> Completed
```

Any stage may terminate as `Rejected`, `Cancelled`, `Expired`, or
`Invalidated`. A terminal record never resumes. Policy, readiness, and
confirmation cannot be skipped.

At most one action is pending for a conversation/session. A second action
returns `ActionAlreadyPending`; it is neither merged nor substituted. Exact
bounded cancellation controls are recognized only while an action is pending.
Informational input can return `NoAction` without changing the pending binding.

Clarification binds the conversation, session, requester, original request,
planner version, candidate set, and expiry. Public choices use opaque indexes;
the coordinator maps a choice to an offered canonical candidate and submits a
new planner evaluation. Clarification is not confirmation.

Confirmation reuses `ConfirmationAuthority`. Natural-language agreement never
becomes approval; only typed `ConfirmationResponse` can create a grant.
Confirmation is not execution, and final readiness revalidation is mandatory.

## Invalidation, expiry, and replay

Pending state binds to the session, requester, runtime snapshot generation,
policy version, and relevant application/settings registry generation. Logout,
session replacement, or a safety-relevant generation change terminates the
flow. No action crosses a session boundary.

Clarification, confirmation, readiness, and record retention use separate
bounded lifetimes. Expiry never restarts planning. Dispatch failure consumes
readiness and is terminal; there is no retry, confirmation reuse, or fallback.

Bounded replay tracking makes duplicate request, clarification, confirmation,
cancellation, session-end, and invalidation delivery idempotent. Saturated or
unknown replay state fails closed. Executor replay protection remains the
final defense against duplicate readiness consumption.

After cancellation, rejection, invalidation, expiry, or dispatch failure, a
new request and coordinator action ID are required.

## Public meaning and audit

`ActionCompleted` means the trusted typed launcher accepted dispatch. It does
not mean the application initialized or became ready; readiness must be
observed separately.

English and Persian messages are selected from typed public status/reason
codes. The bounded audit ring stores IDs, state, operation, timestamps, and
public results. It never stores full user text, conversation history,
confirmation proof material, policy rules, registry contents, executable
paths, shell strings, raw IPC, secrets, or hidden reasoning.

## Verification

Host tests live beside `coordinator.rs`. The feature-gated native gate is:

```sh
./tools/test.sh wiseowl-coordinator-v1
```

It uses a fake typed adapter and adds no production execution route.
