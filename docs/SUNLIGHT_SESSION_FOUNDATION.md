# SunlightOS Session Foundation

Status as of July 27, 2026: implementation in progress. Core native session-manager plumbing, Login handoff, Vortex Shell component management, CLI, manifest, and host-compilable state-machine pieces are present. Native ISO proof and full resource measurements are not yet recorded in this document.

## Scope

Phase 0 introduces a native desktop session manager:

- `sunlight-sessiond`
- `sunlight-sessionctl`
- versioned session and component contracts
- authenticated Login handoff
- one required managed component: Vortex Shell
- readiness-based session start
- bounded shell restart
- structured session actions
- logout-driven session teardown
- bounded diagnostics and statistics

## Non-goals

This phase does not implement:

- startup-app configuration
- `.sunapp` selection UI
- Wise Owl Welcome
- Session Restore / checkpoints / hibernation
- multi-user switching
- kiosk UI
- compositor redesign
- authentication redesign
- arbitrary process launching via session IPC

## Repository audit

Previous direct flow:

```text
boot -> init -> tty_server/login UI
                     |
                     | desktop auth success
                     v
                mezzo session establish
                     |
                     v
                display SESSION_ACTIVATE
                     |
                     v
             display directly launches Vortex Shell
```

New intended flow:

```text
Authentication:
Login -> sunlight-sessiond -> Vortex Shell
```

```text
Lifecycle:
Created -> Preparing -> Starting -> Running -> Stopping -> Stopped
```

```text
Shell crash:
Running -> Degraded -> Restarting -> Running
```

### Audit findings and classification

| # | Finding | Classification |
|---|---|---|
| 1 | Graphical Login is hosted by `services/tty_server` and started from boot service flow. | directly reusable |
| 2 | Authentication remains in `services/sunlight-uac`; Login calls `authenticate_password_for_session`. | directly reusable |
| 3 | Desktop path previously called Mezzo and then activated display; display launched Vortex directly. | confirmed architectural coupling |
| 4 | Login preserved real `uid/gid` from UAC grant. Username propagation was bounded and ad hoc. | reusable with a small adapter |
| 5 | Shell capabilities came from the existing `UserSession` service-capability profile. | directly reusable |
| 6 | Login remained alive after desktop launch to keep VT/login ownership. | directly reusable |
| 7 | Logout previously depended on shell/display behavior, not a native session broker. | confirmed correctness defect |
| 8 | Lock screen was mediated by Mezzo and remained separate from Login. | directly reusable |
| 9 | Shell crashes were detected by display ownership/window-loss logic. | confirmed architectural coupling |
| 10 | A shell crash effectively ended the desktop path or triggered display-owned respawn. | confirmed correctness defect |
| 11 | Native process creation uses the kernel spawn path plus `SpawnRequest`. | directly reusable |
| 12 | Parent/child identity is tracked by kernel process records and telemetry. | directly reusable |
| 13 | Exit/liveness is observable through kernel process generation/liveness queries. | reusable with a small adapter |
| 14 | `sunlightd` already supervises system services through unit definitions and restart policy. | directly reusable |
| 15 | Nameserver registration/removal is already native and capability-scoped. | directly reusable |
| 16 | Authenticated identity is represented as kernel/UAC session grant + `uid/gid`. | directly reusable |
| 17 | A reusable one-shot authentication result existed as the UAC session grant. | directly reusable |
| 18 | Capability scoping already distinguishes service profiles and `UserSession`. | directly reusable |
| 19 | Per-user storage resolution is mostly path/env-based; kernel defaults are generic. | reusable with a small adapter |
| 20 | TTY and graphical paths coexist in `tty_server` with VT ownership and display activation. | directly reusable |
| 21 | Lock/logout/restart actions were fragmented across shell, display, and Mezzo. | confirmed architectural coupling |
| 22 | Shell-global shortcuts currently go through shell/display-specific routing. | reusable with a small adapter |
| 23 | `.sunapp` launching exists through app/runtime launch code, not session planning. | future concern |
| 24 | No native desktop session broker existed before this phase. | missing and required |
| 25 | RAM/CPU/process telemetry exists; endpoint/SHM/timer inventories are limited. | reusable with a small adapter |
| 26 | Existing QEMU gates cover boot/login/shell flows, but not session-manager lifecycle. | missing and required |

## Display-manager vs compositor distinction

`sunlight-sessiond` is the session broker and desktop-session manager. It is not the compositor.

- `tty_server` remains the login/authentication frontend.
- `sunlight-display` remains the graphical compositor/window system.
- `sunlight-sessiond` owns session identity, component launch, supervision, and teardown.
- `sunlight-vortex-shell` remains the desktop UI shell, now as a managed session component.

## Service identity

- service binary: `sunlight-sessiond`
- endpoint: `sunlight.session.v1`
- CLI: `sunlight-sessionctl`

The service is registered as a native `sunlightd` service and embedded into the ISO image.

## Session ID design

Shared contracts live in `ipc/src/lib.rs`.

- `SessionId`
- `SessionGeneration`
- `SessionComponentId`
- `SessionClientId`

Design properties:

- zero is rejected
- not raw PIDs
- generation is separated from PID identity
- wire layout is fixed-width and architecture-independent
- stale handles can be rejected with `(SessionId, SessionGeneration)`

## Session lifecycle

Implemented state enum:

- `Created`
- `Preparing`
- `StartingRequiredComponents`
- `Running`
- `Degraded`
- `Locking`
- `Locked`
- `Stopping`
- `Stopped`
- `Failed`

`sunlight-sessiond` enforces explicit transitions in the shared session record model.

## Login handoff

The graphical Login still authenticates exactly as before through UAC.

After success, the desktop path now:

1. keeps the UAC-issued one-shot authenticated grant
2. sends a bounded `SESSION_CREATE` request to `sunlight.session.v1`
3. waits for required shell readiness before activating the desktop framebuffer

The handoff does not forward password text or raw authentication secrets.

## User context propagation

The shell is spawned by `sunlight-sessiond` with:

- authenticated `uid/gid`
- the existing `UserSession` capability profile
- native process generation tracking

This avoids inheriting unrestricted service identity from `sunlight-sessiond`.

## Session manifest

Current system manifest path:

`/etc/sunlight/sessions/sunlight-desktop.toml`

Current content shape:

- `format_version = 1`
- one component
- app id `org.sunlight.vortex-shell`
- role `shell`
- required
- `launch_policy = session-start`
- `restart_policy = on-failure`
- `restart_limit = 3`
- `restart_window_seconds = 30`
- `readiness_timeout_seconds = 10`

Parsing is strict and rejects:

- duplicate component IDs
- duplicate shell roles
- missing shell
- unsupported format version
- malformed app IDs
- excessive component count

## Component model

Shared enums define:

- `SessionComponentRole`
- `SessionLaunchPolicy`
- `SessionRestartPolicy`
- `SessionComponentState`

The runtime contract records:

- component identity
- app identity
- role
- required flag
- process id/generation
- component state
- launch count
- restart count
- bounded last-exit reason

## Vortex Shell launch behavior

Normal authenticated desktop login no longer lets `sunlight-display` launch or respawn the shell.

Instead:

1. Login authenticates
2. Login requests session creation
3. `sunlight-sessiond` resolves the shell from the trusted system manifest
4. `sunlight-sessiond` spawns Vortex Shell
5. Login waits for shell readiness
6. only then does Login activate the display session

## Shell readiness protocol

Native IPC operations now include:

- `SESSION_COMPONENT_HELLO`
- `SESSION_COMPONENT_READY`
- `SESSION_COMPONENT_STOPPING`

`sunlight-vortex-shell` now:

1. resolves `sunlight.session.v1`
2. sends component hello
3. receives its assigned session/component binding
4. sends explicit ready after window registration

`sunlight-sessiond` marks the session `Running` only after `COMPONENT_READY`.

## Shell supervision

`sunlight-sessiond` now owns required-shell supervision:

- one active shell component
- readiness timeout
- process liveness detection
- crash classification
- restart budget
- restart exhaustion -> session failure -> stop

Suggested current policy is `3 restarts / 30 seconds`.

## Restart policy

Current implementation:

- required shell only
- `OnFailure`
- readiness timeout participates as failure
- restart counter resets after a stable-enough run window

## Logout

Structured session action `Logout` now drives:

1. session validation
2. session state -> `Stopping`
3. shell stop signal
4. bounded grace timeout
5. forced termination if needed
6. session record transition to `Stopped`
7. return to Login path

Restart is disabled once logout has begun.

## Lock-screen integration

The existing lock screen remains in Mezzo.

This phase only adds structured routing:

- Vortex Shell requests `SessionAction::Lock`
- `sunlight-sessiond` validates and forwards to Mezzo
- session state tracks lock-related state without absorbing lock UI

Unlock-complete routing exists in the protocol surface but is not yet fully exercised end-to-end in native evidence.

## Session actions

Shared action enum:

- `Lock`
- `UnlockCompleted`
- `Logout`
- `RestartShell`
- `QueryStatus`

This gives one structured path for shell-originated session control instead of ad hoc compositor or shell-only behavior.

## Capability model

The phase maps existing authority into a narrower session model:

- Login: may create authenticated sessions
- Shell: may register, announce ready, inspect its session, and request allowed session actions
- Admin tooling: may inspect/control all sessions

Current implementation uses:

- trusted tty-service caller validation for login handoff
- trusted session-service validation where required
- caller UID checks for own-session control

## IPC protocol

Current protocol surface in `ipc/src/lib.rs`:

- `SESSION_CREATE`
- `SESSION_GET`
- `SESSION_LIST`
- `SESSION_GET_COMPONENTS`
- `SESSION_COMPONENT_HELLO`
- `SESSION_COMPONENT_READY`
- `SESSION_COMPONENT_STOPPING`
- `SESSION_ACTION`
- `SESSION_LOGOUT`
- `SESSION_RESTART_COMPONENT`
- `SESSION_GET_STATS`
- `SESSION_GET_HEALTH`

The wire protocol uses fixed integer fields, not raw Rust struct layouts.

## Cleanup

Implemented cleanup covers:

- shell process reference removal
- component runtime reset
- session stop finalization
- last-closed bounded diagnostic record

Cleanup is intended to be idempotent, but repeated login/logout soak validation still needs native proof.

## Observability

`sunlight-sessiond` maintains bounded counters for:

- session creation/start/running/failure/stop
- login handoffs and failures
- component launches/readiness/restarts/crashes
- logout requests/completions/timeouts
- stale and unauthorized requests

`sunlight-sessionctl` exposes:

- `status`
- `list`
- `inspect <session-id>`
- `components <session-id>`
- `restart-shell <session-id>`
- `logout <session-id>`
- `health`

## QEMU test sequence

Added gate target:

```bash
./tools/test.sh session-foundation
```

Expected markers:

- `SERVICE_READY PASS`
- `LOGIN_HANDOFF PASS`
- `SESSION_CREATED PASS`
- `SHELL_STARTED PASS`
- `SHELL_READY PASS`
- `SHELL_CRASH_RESTART PASS`
- `SINGLE_SHELL PASS`
- `LOGOUT PASS`
- `LOGIN_RETURN PASS`
- `SECOND_SESSION PASS`
- `STALE_HANDLE_REJECT PASS`
- `RESOURCE_BASELINE PASS`
- `IDLE_CPU PASS`
- `FINAL PASS`

Native proof is not yet recorded as passing.

## Resource measurements

Available kernel surfaces today provide:

- RAM totals/used via `sysinfo`
- process count via telemetry
- CPU usage and per-core scheduler ticks via telemetry

Not yet cleanly exposed:

- endpoint inventory
- SHM lease inventory
- restart-timer inventory

Because of that, full requested measurement tables are still outstanding.

## Known limitations

- Native ISO gate result is not yet captured as passing.
- Exact endpoint/SHM/timer leak accounting is not yet exposed by kernel telemetry.
- Lock/unlock native evidence is incomplete.
- Current implementation manages one active desktop session and one required component.
- Closed-session diagnostics remain inspectable as bounded history; stale-control rejection is stronger than stale-read rejection.

## Control Panel entry criteria

Startup-app and session-settings UI work must wait until:

- session manifest resolution is stable
- component role/policy contract is stable
- native create/logout/restart path is proven on ISO

## Welcome Wizard entry criteria

Wise Owl Welcome should not be added until:

- session-start component launch is proven for the required shell
- non-shell optional startup components are admitted by the manifest resolver
- failure policy for optional components is specified

## Session Restore future boundary

Future restore/checkpoint work may attach persisted state to:

- `SessionId`
- session manifest resolution
- startup component plan

This phase does not persist restore state and does not infer application recovery from shell restart.
