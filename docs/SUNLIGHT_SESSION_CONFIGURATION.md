# SunlightOS Session Configuration Phase 1

Status: implemented on the completed Session Foundation (`sunlight-sessiond`).

## Scope

This phase adds configurable optional **Startup Apps** on top of the required
Vortex Shell session:

- versioned per-user **Session Profile**
- trusted `.sunapp` eligibility discovery
- deterministic **Resolved Session Plan** at login
- optional apps launch only after Shell Ready
- Control Panel page **Login & Session → Startup Apps**
- `sunlight-sessionctl startup …` CLI
- host tests and native ISO gate `./tools/test.sh session-configuration`

## Non-goals

Not implemented in this phase:

- Wise Owl Welcome
- Wise Owl inference / Pattern Recognition / self-healing
- Session Restore / snapshots / window restore / hibernation
- multiple graphical sessions or user switching
- arbitrary command / script / raw path startup entries
- environment-variable or free-form argument editing
- remote discovery, install, or uninstall of applications

## Architecture

```text
Control Panel / sunlight-sessionctl
        |
        | profile configuration IPC
        v
sunlight-sessiond
        |
        +-- validates caller
        +-- validates Bundle IDs against trusted catalog
        +-- persists per-user profile atomically
        +-- generates preview plan
        |
Next authenticated login
        |
        v
sunlight-sessiond
        |
        +-- load system session definition
        +-- load installed eligible .sunapp catalog
        +-- load per-user profile
        +-- resolve immutable session plan
        +-- launch Vortex Shell (required)
        +-- wait for Shell Ready
        +-- launch optional Startup Apps in order
```

Control Panel never launches Startup Apps and never edits the system manifest.

```text
System session definition
        +
Installed eligible .sunapp bundles
        +
Per-user startup preferences
        ↓
Resolved immutable session plan
        ↓
Required Vortex Shell
        ↓
Optional Startup Apps
```

## Immutable system session definition

Path: `/etc/sunlight/sessions/sunlight-desktop.toml`

Defines required components only (Vortex Shell). Users cannot disable, remove,
reorder, or re-point the shell through profile APIs.

## Per-user Session Profile

Stored under:

```text
/root/.config/sunlight/session-profile.v1.<uid>
```

Namespace concept: `session.profile.v1.<user-id>` (file-backed by sessiond).
Phase 1 stores under the root home tree because sessiond currently runs as the
root User VFS actor, which may write there without a separate service-state
capability grant.

Not stored in Wise Owl MemoryDB.

### Schema

- `format_version` (u16, currently 1)
- `profile_id`, `user_id`, `base_session_id`
- `revision` (optimistic concurrency)
- `entries[]` StartupEntry
- `policy_state[]` first-login / upgrade completion
- `created_at` / `updated_at`
- integrity checksum (FNV-1a)

### StartupEntry

Identifies an installed trusted application by **Bundle ID** only:

- `entry_id`, `app_id`, `enabled`
- `policy` ∈ {EveryLogin, FirstLoginOnly, FirstLoginAfterSystemUpgrade, Disabled}
- `launch_phase` = AfterShellReady (only phase in this release)
- `order` normalized to `0..n-1`
- `restart_policy` ∈ {Never, OnFailureOnce} (default Never)
- `added_at`

Rejected in user configuration: executable paths, shell commands, env vars,
capability lists, passwords.

### Persistence and revisions

```text
serialize → checksum → write .tmp → rename (atomic)
```

Every mutation requires `expected_revision`. Mismatch returns
`ERR_PROFILE_REVISION` (`ProfileRevisionConflict`). Failed writes leave the
previous valid profile intact.

Corrupt/missing profiles fall back to required shell only and keep Login usable.

## Startup policies

| Policy | Behavior |
|---|---|
| EveryLogin | Launch every successful desktop session start |
| FirstLoginOnly | Launch once after add/reset; completion only after successful start |
| FirstLoginAfterSystemUpgrade | Launch once per stable release generation (`/etc/sunlight/release-generation`) |
| Disabled | Never included in the Resolved Session Plan |

Failed spawns never complete one-time policies.

## `.sunapp` eligibility

Trusted root: `/Applications/*.sunapp`

Manifest extensions (adapted to existing schema):

```toml
[session]
startup_eligible = true
default_enabled = false
default_policy = "every-login"
single_instance = true
launch_path = "/bin/su1"   # short path for SpawnRequest (≤31 chars)
```

Eligible only when installed under trusted roots, manifest valid, Bundle ID
valid, launch executable present, `startup_eligible = true`, and complete.

Phase 1 launch uses SpawnRequest identity-preserving spawn; launch paths must
fit the existing spawn wire limit (short `/bin/…` helpers). Chronos `.sunapp`
paths that exceed that limit are catalog-skipped until a future launcher path
exists.

## Resolved Session Plan

Built once at authenticated login and frozen for the session lifetime:

1. Validate system session definition
2. Add required components
3. Load latest valid user profile
4. Load trusted catalog
5. Resolve enabled entries by Bundle ID
6. Apply startup policy
7. Skip unavailable/invalid safely (entries retained in profile)
8. Enforce component limits
9. Sort optional apps: launch phase → order → Bundle ID → entry ID
10. Freeze plan

Profile edits apply at the **next login**. Control Panel shows
“Changes apply at your next login.”

## Vortex Shell protection

Shown in Control Panel under **Required Session Components** with no disable,
remove, or reorder controls. System manifest remains the only authority.

## Optional application launch

After Shell Ready:

1. Session Running
2. Resolve AfterShellReady entries from frozen plan
3. Launch in deterministic order with user `uid/gid` and `UserSession` caps
4. Record startup result

Capabilities come only from the installed bundle / system policy — never from
startup configuration.

Optional failure does not terminate Shell or Session. Restart is bounded
(`Never` or `OnFailureOnce`).

## Capability model

Ordinary users may inspect/modify their own profile, list eligible apps, and
preview their next plan. They may not:

- modify another user’s profile
- edit the system manifest / required components
- add arbitrary paths or uninstalled bundles
- grant capabilities via startup configuration

## Control Panel

Page: **Login & Session** (`control-panel --page session`)

- Required Components: Vortex Shell (protected)
- Startup Apps list with enable toggle, policy, order, Move Up/Down
- Add dialog over eligible installed apps only
- Empty state: “No optional applications start automatically.”
- Immediate atomic apply through sessiond IPC

## CLI

```text
sunlight-sessionctl startup list
sunlight-sessionctl startup eligible
sunlight-sessionctl startup add <bundle-id>
sunlight-sessionctl startup remove <bundle-id>
sunlight-sessionctl startup enable <bundle-id>
sunlight-sessionctl startup disable <bundle-id>
sunlight-sessionctl startup policy <bundle-id> <policy>
sunlight-sessionctl startup move-up <bundle-id>
sunlight-sessionctl startup move-down <bundle-id>
sunlight-sessionctl startup reset
sunlight-sessionctl startup preview
sunlight-sessionctl startup status
```

## Diagnostics

Bounded counters include profiles loaded/missing/corrupt, updates/conflicts,
eligible discovery, launches, readiness timeouts, first-login completions,
resolved plans, etc. Serial markers use `[SESSION-CONFIG] … PASS`.

## Host tests

```text
cargo test --package sunlight-sessiond --lib --target x86_64-unknown-linux-gnu
```

Covers profile contracts, mutations, plan resolution, policies, checksums.

## Native ISO test

```text
./tools/test.sh session-configuration
```

Uses real QEMU ISO boot, real IPC, and test-only fixtures:

- `org.sun.test.su1` → `/bin/su1`
- `org.sun.test.su2` → `/bin/su2`

## Resource measurements

Session profile + plan storage is bounded (≤4 KiB profile blob, ≤16 startup
entries, ≤20 plan components). Optional fixtures are tiny native binaries.
Idle path uses existing sessiond yield loop (no settings busy-poll).

Exact ISO RAM/CPU/endpoint/SHM numbers are produced by the native gate’s
RESOURCE_BASELINE / IDLE_CPU markers on a real boot.

## Remaining limitations

- SpawnRequest path length limits short native launch helpers for optionals
- Catalog scan is `/Applications` only
- Optional readiness is “running without readiness protocol” for fixtures
- No drag-and-drop reorder (Move Up/Down provided)
- No Start Now / Stop Now runtime controls
- Chronos-only long `.sunapp` paths not yet launchable as startup apps

## Welcome Wizard (implemented)

Welcome is now an ordinary optional startup component. Design and permanent
fix notes live under **session manager** docs:

- [`session-manager/README.md`](./session-manager/README.md)
- [`session-manager/WELCOME_WIZARD_PHASE1.md`](./session-manager/WELCOME_WIZARD_PHASE1.md)
- [`session-manager/WELCOME_WIZARD_IMPLEMENTATION_FIXES.md`](./session-manager/WELCOME_WIZARD_IMPLEMENTATION_FIXES.md)

```toml
[session]
startup_eligible = true
default_enabled = true
default_policy = "first-login-after-system-upgrade"
single_instance = true
completion_mode = "wizard-finished"
launch_path = "/bin/welcome"
```

Bundle id: `org.sunlight.welcome`. No Welcome special-case inside Login or
Vortex lifecycle; sessiond uses the generic optional path plus a short desktop
settle delay before spawn.

## Session Restore boundary

Startup Profile = what should normally launch.  
Session Snapshot = what was open previously.  
This phase implements only the Startup Profile side.

## Explicit non-implementation confirmation

Full Wise Owl inference, Session Restore, Pattern Recognition, and self-healing
were **not** implemented in Session Configuration Phase 1 (or Welcome Phase 1).
