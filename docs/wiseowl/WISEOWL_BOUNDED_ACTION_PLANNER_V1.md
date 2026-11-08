# Wise Owl Bounded Action Planner v1

The Bounded Action Planner converts a small, explicit conversation grammar
into an untrusted typed proposal. It supports only `OpenApplication` and
`OpenSettingsPage`. It does not authorize, confirm, queue, dispatch, or execute
anything.

## Boundaries

The complete action path is:

```text
Conversation
  -> PlannerInput
  -> PlannerResult::Proposed(ActionIntentDraft)
  -> ActionIntent construction and validation
  -> PolicyDecision
  -> Confirmation when policy requires it
  -> ReadyForExecution
  -> TrustedActionExecutor
  -> ExecutionResult
```

- **Conversation** is user-facing dialogue. Merely mentioning a target is not a
  launch request.
- **Planner Input** is one bounded relevant request slice with request,
  conversation, session, requester, locale, runtime-generation, timestamp, and
  provenance bindings. It does not carry arbitrary history.
- **Action Proposal** is an `ActionIntentDraft` containing one supported
  operation and one typed registry ID. It is untrusted and has no authority.
- **Action Intent** is the final typed proposal consumed by validation. A draft
  must be constructed into a fresh intent; clarification never mutates one.
- **Policy Decision** determines whether the validated intent is allowed,
  denied, or needs confirmation. Planner confidence cannot affect policy.
- **Confirmation** is an exact, expiring, session-bound authorization step when
  policy requires it. Clarification is not confirmation.
- **ReadyForExecution** is constructed only after validation, policy, current
  runtime/session checks, and any required confirmation.
- **Execution** is performed only by `TrustedActionExecutor`, which consumes
  `ReadyForExecution` and dispatches a typed registered target.

There is no planner-to-executor path. The planner module has no executor,
process-launch, shell, path, argument, environment, service, filesystem, raw
IPC, callback, or readiness capability.

## Grammar and normalization

v1 recognizes bounded English application forms (`Open`, `Launch`, and
`Start`), English settings forms (`Open ... settings` and `Show ... settings`),
and explicit Persian forms ending in `را باز کن`. Supported polite wrappers
are limited to `please` and `لطفا`.

Normalization trims surrounding whitespace, lowercases ASCII, removes harmless
terminal punctuation, and conservatively maps Arabic yeh/kaf forms to their
Persian equivalents. It does not use fuzzy matching or broad semantic
rewrites.

Questions, explicit negation, quoted text, and code blocks produce `NoAction`.
Unsupported operations remain unsupported and are never approximated. A single
request can produce at most one draft; joined requests such as “Open Calculator
and Files” are rejected.

## Registry-backed aliases

Application targets are read through the canonical `sun_exec` application
registry. Settings targets use the same `ControlPanelPage` registry consumed by
Control Panel and typed executor dispatch. Planner-visible registry metadata
contains only canonical IDs, display names, and explicit locale-bound aliases;
executable paths remain private to `sun_exec`.

The alias model is bounded and versioned. Aliases are exact, locale-aware, and
auditable. They are shipped registry data, never learned from conversation,
memory, or previous behavior. An unknown alias fails closed. A collision
creates a bounded `ClarificationRequest`; the planner never chooses the first
match.

Clarifications contain only authoritative candidates and are bound to their
conversation, session, expiry, and planner version. Responses re-enter the
planner and create a new draft. Expired, replayed, cross-session, and
non-candidate responses fail closed.

## Audit and testing

Planner audit is a fixed-capacity ring. Entries contain request IDs, bounded
input digests, typed operation/target kinds, and public result codes. They do
not contain conversation history, source text, paths, payloads, hidden
reasoning, or policy internals.

Host tests cover exact and alias resolution, English and Persian commands,
unknown targets, collisions, ambiguity, multiple actions, unsupported
operations, questions, negation, quoted/code text, input bounds, malformed
UTF-8, stale snapshots, wrong sessions, clarification lifecycle, policy
separation, and audit redaction.

The feature-gated QEMU test is:

```sh
./tools/test.sh wiseowl-planner-v1
```

It uses a fake typed launch adapter and proves that text becomes a draft, the
draft becomes a validated `ActionIntent`, policy is evaluated,
`ReadyForExecution` is constructed, and typed fake dispatch succeeds. It does
not launch a host process and adds no production backdoor.
