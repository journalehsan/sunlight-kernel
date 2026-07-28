# Wise Owl Trusted Action Executor v1

The v1 executor is the final, capability-bounded stage of the existing action
pipeline:

```text
ActionIntent
    -> Policy Decision
    -> optional Confirmation Grant
    -> ReadyForExecution
    -> Execution Authorization
    -> typed Dispatch
    -> ExecutionResult
```

## Boundary meanings

- **Action Intent** is a typed proposal containing the exact operation, target,
  parameters, requester, session, runtime generation, and provenance. It does
  not authorize or execute anything.
- **Policy Decision** binds a policy version and result to the complete intent.
  Denied and unknown decisions cannot become ready.
- **Confirmation Grant** is an exact, expiring approval bound to the intent,
  target, parameters, policy, runtime generation, session, and responder.
- **ReadyForExecution** is the only input accepted by the executor. Readiness
  reserves an exact confirmation grant so a second readiness envelope cannot
  be produced from it; the executor consumes the readiness/grant immediately
  before its single dispatch attempt.
- **Execution Authorization** is the executor's final read-only validation. It
  rechecks envelope bindings, current policy, runtime freshness, active
  session, requester, confirmation, supported operation, and authoritative
  registry membership.
- **Dispatch** is one typed request to the existing SunlightOS launcher. No
  command string, executable path, argument list, environment, or callback
  crosses the Wise Owl executor boundary.
- **Application Readiness** is a later lifecycle state owned by the application
  and shell. Executor v1 does not wait for it.
- **Execution Result** reports whether authorization failed or the authoritative
  launcher accepted/rejected the typed request. `Succeeded` means dispatch was
  accepted; it does not mean the application finished startup or later exited
  successfully.

## Capability and failure model

Executor v1 supports only `OpenApplication(Application(bundle_id))` and
`OpenSettingsPage(SettingsPage(page_id))`. Every other operation returns
`UnsupportedOperation` without partial execution.

The executor never parses natural language because language interpretation
belongs before typed intent validation. It never accepts raw commands because
that would bypass intent, policy, confirmation, and readiness bindings.
Application IDs are resolved by the shared authoritative `sun_exec` registry.
Settings pages use the same bounded `ControlPanelPage` registry consumed by the
Control Panel. Unknown IDs fail closed and never select a similar or home-page
fallback.

Brain receives only a `TrustedLaunchAdapter`, which can check registration and
submit the two typed request forms. The executor has no filesystem mutation,
service, package, device, authentication-bypass, or generic spawn interface.
The SunlightOS adapter delegates final resolution to the existing launcher; no
new daemon or generic IPC endpoint is introduced.

There are no automatic retries. A readiness identity is recorded immediately
before dispatch, including failed dispatch attempts, so replay returns
`AlreadyConsumed`. Replay storage is fixed-size and fails closed at capacity
instead of evicting consumed identities.

## Audit

Execution audit records are fixed-size and contain only bounded enums and IDs:
execution ID, intent ID, operation, target kind, public result, audit ID, and
timestamp. They cannot contain conversation text, raw payloads, target values,
paths, arguments, environment variables, secrets, or internal policy rules.
