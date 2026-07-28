# Wise Owl Policy Engine v1

Policy Engine v1 is Wise Owl's immutable, fail-closed permission layer. It
decides what an action is allowed to do; it does not decide what action Wise
Owl wants to take. No AI reasoning, learned behavior, installer intelligence,
daemon, or IPC protocol is part of this milestone.

## Layer Boundaries

- **Foundation Memory** contains immutable identity, capabilities, and permanent
  safety principles. It says not to invent live state, not to assume services
  such as networking exist, and to degrade safely when trusted facts are
  unavailable.
- **Runtime Snapshot** is a read-only, transient view of the current boot and
  session. Policy may inspect it, but never changes or persists it.
- **Policy Engine** applies shipped, versioned rules to a requested operation
  and the Runtime Snapshot. It returns `Allowed`, `Denied`,
  `ConfirmationRequired`, or `Unknown`.
- **Conversation** supplies the user's current request.
- **Reasoning** may propose a typed Action Intent, but cannot evaluate a bare
  operation or call a service.
- **Action Intent validation** checks the full typed operation, target,
  parameters, and Runtime Snapshot generation before policy.
- **Learned Memory** is durable knowledge acquired from use or documents. It is
  not a policy source and is not implemented by this milestone.

The pipeline is:

```text
Foundation Memory
        |
Runtime Snapshot
        |
Conversation
        |
Reasoning
        |
Action Intent validation
        |
Policy Engine
        |
Action Decision
```

Foundation remains unchanged; Runtime Snapshot adds only a publication
generation for freshness binding. The existing
`CognitivePipeline` owns the in-process Action Intent evaluator and exposes
`evaluate_action`, which refreshes Runtime Snapshot if due, validates the
complete proposal, and only then invokes policy. There is no operation-only
public evaluation path and no action-execution path in the current bounded
pipeline.

## Policy v1 Model

The v1 rules are a static read-only slice compiled into `wiseowl-brain`. Each
rule maps a typed operation and category to an effect. The engine itself holds
only a version and a shared immutable rule slice, so future Installer,
Recovery, Enterprise, OEM, Server, or Developer Mode groups can ship their own
versioned static slices without changing the evaluator contract.

Categories are Read, Observe, Recommend, Execute, Modify, Delete, and Critical.
Confirmation levels are None, Soft, Strong, and Critical.

Core behavior includes:

- hostname, timezone, network reads, observation, recommendations, calculator
  launch, and Control Panel launch are explicitly allowed
- service restart requires Soft confirmation
- package installation and file deletion require Strong confirmation
- installer disk writing is allowed only when Installer Mode is known
- disk erase is denied in Desktop Mode and requires Critical confirmation in
  Installer or Recovery Mode
- recovery maintenance is allowed only in known Recovery Mode
- formatting a disk and modifying the bootloader are always protected

Missing, conflicting, or unrecognized facts remain `Unknown`. An unknown
operation has no matching rule and also remains `Unknown`; there is no fallback
from `Unknown` to `Allowed`.

## Explainability and Logging

Every `PolicyDecision` contains the policy version, operation, category,
result, confirmation level, and a stable reason. `audit_record` emits a bounded
operator-facing record such as:

```text
POLICY
operation=DiskErase
result=Denied
reason=DesktopMode
confirmation=None
version=1.0
```

The record explains the decision without exposing rule layout, provider
failures, or other internal implementation details to users.

## Non-Goals

Policy v1 does not execute actions, prompt for confirmation, infer installer
intent, learn rules, update Foundation Memory, mutate Runtime Snapshot, start
background work, or add a transport. Those responsibilities stay outside this
pure decision layer.
