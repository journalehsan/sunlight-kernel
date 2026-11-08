# Wise Owl Production Trust Runtime Plumbing v1

## Runtime ownership audit

| Component | Owner and evidence | Classification |
| --- | --- | --- |
| Authenticated desktop session | `sunlight-sessiond` owns one `ActiveSession`, a `SessionRecord` with session and generation, and the authenticated UID/GID grant consumed during `SESSION_CREATE`. | Directly reusable |
| Login/logout/shell restart | TTY consumes the login grant and calls `SESSION_CREATE`; sessiond owns `Logout`, `RestartShell`, component state and generation checks. | Directly reusable |
| IPC caller identity | Kernel IPC overwrites `IpcMsg.badge` with the sender PID; sessiond resolves it with `session_query_process`. | Directly reusable |
| Wise Owl Console registration | The Console currently only registers its UI window with `display_server`; it is not registered as a sessiond component or GUI identity. | Missing |
| Application launch context | `sun_exec` uses `LaunchTrace` and passes it in argv; display associates it with PID. | Diagnostic only / unsafe for trust |
| Display lifecycle | `sunlight-display` owns `CREATE_WINDOW`, window records and source PID; it only emits launch-trace logging today. | Reusable with adapter |
| Application readiness | No authoritative application-registry readiness producer exists. Window registration is insufficient for non-window contracts. | Missing |
| Control Panel page | `ControlPanelApp.page` and `parse_initial_page` own the exact canonical page state, but emit no IPC lifecycle event. | Reusable with adapter |
| braind startup | `sunlightd` owns `wiseowl-braind.service`; it starts after VFS and wants, not requires, KV/MemoryDB/Index. The daemon registers `wiseowl.brain.v1` synchronously. | Reusable with adapter |

## Startup finding

The failed live-action gate did not set a boot injection phase that reaches the
normal desktop service graph. Its serial output stopped in service startup and
never contained braind's process-entry marker. This is a gate boot-fixture
selection problem, not a readiness-evidence result; increasing the timeout
would not address it. Production startup must publish bounded phases after the
endpoint is registered, with optional source absence represented as Degraded.

## Trust decisions

Launch traces, argv, window titles, process names, PID-only state, and logs are
diagnostic only. They cannot issue an attestation or satisfy readiness. The
future runtime adapters must carry the existing opaque executor correlation in
a sealed service-to-service record or capability attachment, never argv,
environment, title, or logs.

## Deferred

Console `SubmitTurn` remains action-disabled. This milestone must first add the
narrow sessiond authority request, daemon-only source authenticated ingress,
and lifecycle adapters; no GUI action is enabled by this audit.

## Delegated session/lifecycle implementation

The missing production interfaces identified by this audit are implemented in
`WISEOWL_DELEGATED_SESSION_LIFECYCLE_IPC_V1.md`. The selected design is a
fixed-purpose kernel-backed delegated caller capability. Sessiond returns an
opaque, braind-bound authority proof; `VerifiedGraphicalSession` remains local
and non-serializable. Display and Control Panel use separate kernel-gated
lifecycle endpoints, and argv/log launch traces remain diagnostics only.

Production actions remain disabled and the launch adapter fails closed.
