# Wise Owl Action Intent v1

Action Intent v1 is the typed proposal boundary between Wise Owl reasoning and
the existing Policy Engine. It represents and evaluates possible actions, but
does not execute them. It adds no daemon, service call, IPC protocol, learned
memory, or general reasoning system.

## Boundary and responsibilities

The complete path is:

```text
Foundation Memory
        |
Runtime Snapshot
        |
Conversation
        |
Reasoning
        |
Action Intent proposal
        |
Validation
        |
Policy evaluation
        |
Action Decision
        |
Future confirmation and executor (not implemented)
```

The terms are deliberately distinct:

- **Reasoning** selects a possible operation based on the conversation and
  trusted context. It may only produce an `ActionIntent`; it has no service,
  syscall, shell, executable-path, or executor interface.
- **Action Intent** is an immutable proposal containing a typed operation,
  typed target, operation-specific bounded parameters, identity and session
  metadata, Runtime Snapshot generation, creation time, risk hint, and
  provenance. It is not permission and has no effect by itself.
- **Validation** checks the complete operation/target/parameter combination and
  snapshot generation. Only `Valid` produces the private validation token that
  policy accepts. `Invalid`, `Unsupported`, and `Unknown` stop here.
- **Policy Decision** applies the immutable versioned policy to a validated
  proposal. Its public `ActionDecision` envelope contains the intent ID, policy
  version, result, confirmation level, public reason code, Runtime Snapshot
  generation, and audit ID. `Unknown` remains fail-closed.
- **Confirmation** is future user authorization for one exact intent, target,
  parameter set, policy version, and Runtime Snapshot generation. The v1
  `ConfirmationBinding` prepares that identity check only; it has no UI and
  does not turn a decision into execution. Any changed field or stale
  generation invalidates the binding.
- **Execution** is a future component that may consume an eligible decision and
  any required confirmation. No executor type, command representation, OS
  service adapter, syscall, or execution result exists in Action Intent v1.

Reasoning cannot directly invoke services because doing so would bypass typed
validation, versioned policy, confirmation identity, freshness checks, and
audit. Keeping reasoning on the proposal side makes every future effect pass
through one reviewable, fail-closed boundary.

## Typed and bounded model

`ActionOperation` is a closed taxonomy: Observe, application/settings/utility
opening, service restart/stop, package install/removal, file modification or
deletion, boot-configuration modification, disk erase, and
`UnknownOperation`. Unknown values are retained as unknown and are never
coerced to an allowed operation.

`ActionTarget` distinguishes applications, settings pages, utilities, services,
packages, files, disks, the system, and unknown targets. Identifiers are
bounded and accept only a small safe character set. File paths are bounded,
absolute, reject parent traversal and command punctuation, and are never
written to policy audit records. There is no raw-command or arbitrary
executable-path variant.

`ActionParameters` is a closed enum. Validation rejects a parameter variant
that does not belong to the operation, unsupported options, ambiguous targets,
oversized values, and malformed identifiers. Invalid data is reported rather
than truncated or silently discarded.

## Freshness, confirmation, and audit

Each published `RuntimeContextSnapshot` has a monotonically increasing
generation. An intent is valid only against the exact generation it names.
Policy decisions repeat that generation, and confirmation bindings require it
to remain current.

`AuditLog` is an in-memory fixed-capacity ring. It records proposal receipt,
validation outcome, and policy outcome. On overflow it explicitly evicts the
oldest entry and increments an eviction counter. Entries contain IDs, enums,
public reason codes, and target kinds only: no conversation text, parameter
values, secrets, full file paths, or internal rule layout.

## Security invariants

- No raw shell commands or shell strings.
- No arbitrary executable paths or syscall representations.
- No reasoning-to-service path.
- No policy evaluation before successful validation.
- No `Unknown`-to-`Allowed` fallback.
- No confirmation reuse after intent, target, parameters, policy version, or
  snapshot generation changes.
- No action execution in this milestone.
