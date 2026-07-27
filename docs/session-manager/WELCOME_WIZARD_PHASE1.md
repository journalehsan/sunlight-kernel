# SunlightOS Welcome Wizard Phase 1

Status: implemented as an ordinary optional Session Startup App on top of
Session Foundation + Session Configuration.

**Also read:** [WELCOME_WIZARD_IMPLEMENTATION_FIXES.md](./WELCOME_WIZARD_IMPLEMENTATION_FIXES.md)
for permanent bring-up fix notes (desktop race, plan optional=0, Vortex
`APP_REGISTRY_LEN` panic, completion IPC).

## Scope

- Native bundled app `org.sunlight.welcome` (`.sunapp` + `/bin/welcome`)
- Eligible as a Session Startup App after Vortex Shell Ready
- Polished multi-page onboarding UI (immediate welcome, greeting, slides, actions)
- Deterministic local greeting with a future Wise Owl provider abstraction
- Explicit onboarding completion (`SESSION_STARTUP_COMPLETE`) — not process launch
- Manual relaunch from Start Menu / search / `sun-exec welcome --manual`
- Host tests + native ISO gate `./tools/test.sh welcome-wizard`

## Non-goals

Not implemented in this phase:

- Full Wise Owl inference / chat UI
- Pattern Recognition / self-healing / autonomous actions
- Document indexing UI
- Session Restore / application checkpoint restore
- Startup-app configuration redesign, Login redesign, Shell redesign
- Remote content loading, analytics upload
- Arbitrary command execution from the wizard

## Repository audit (summary)

| Finding | Classification |
|---|---|
| Resolved Session Plan + AfterShellReady optionals | **directly reusable** |
| Per-user Session Profile + FirstLogin* policies | **directly reusable** |
| `/etc/sunlight/release-generation` | **directly reusable** |
| `.sunapp` `[session]` eligibility schema | **directly reusable** (extended) |
| `sunlight-sessionctl startup …` | **directly reusable** |
| Optional completion = spawn success (Phase 1 fixtures) | **confirmed product gap** for onboarding |
| Explicit optional completion IPC | **missing → added** (`SESSION_STARTUP_COMPLETE`, `completion_mode`) |
| Auto vs manual launch awareness | **missing → added** (`--manual` for shell/CLI) |
| `default_enabled` profile seeding | **missing → added** |
| Dynamic Start Menu from `/Applications` | **out of scope**; Welcome added to existing shell catalog |
| sunlight-ui widgets (Button, Label, Panel) | **directly reusable** |
| Page / Slide / Wizard toolkit widgets | **missing**; app-local page machine |
| System info via `sysinfo` / display metrics | **directly reusable** |
| sunlight-kv app settings | **future concern** (UI state kept in-memory this phase) |
| Message catalogs / i18n | **missing**; centralized string tables in-app |
| Endpoint/SHM leak inventories | **future concern** (limited telemetry) |

No duplicate app registry, bundle parser, or startup manager was introduced.

## Architecture

```text
Login → sunlight-sessiond → Vortex Shell Ready → org.sunlight.welcome
```

```text
Welcome Wizard
   ├── LocalGreetingProvider          (Phase 1 — always works)
   └── FutureWiseOwlGreetingProvider  (stub / capability-checked)
```

```text
Auto launch → user finishes tour → SESSION_STARTUP_COMPLETE
         → policy state records system generation
         → next login (same generation) skips auto launch
Manual relaunch (Welcome Center) still works anytime
```

The Welcome Wizard is **not** hardcoded into Login, Vortex Shell lifecycle, or
sessiond beyond existing startup contracts and a narrow completion IPC.

## Bundle identity

```toml
[application]
id = "org.sunlight.welcome"
name = "Welcome to SunlightOS"
version = "0.1.0"

[session]
startup_eligible = true
default_enabled = true
default_policy = "first-login-after-system-upgrade"
single_instance = true
completion_mode = "wizard-finished"   # alias: app-reported
launch_path = "/bin/welcome"          # SpawnRequest-short path
```

Installed under trusted root: `/Applications/WiseOwlWelcome.sunapp`.

## Session integration

1. Catalog discovery scans `/Applications/*.sunapp` (existing path).
2. Fresh profiles seed `default_enabled` apps (Welcome).
3. Plan resolution applies `FirstLoginAfterSystemUpgrade` against
   `/etc/sunlight/release-generation`.
4. After Shell Ready, sessiond **schedules** optionals (does not spawn immediately).
5. After a desktop settle delay (~1.5 s) sessiond spawns optionals one-by-one
   (staggered), so Vortex can paint wallpaper/dock before Welcome appears —
   same timing model as ordinary sun-exec apps after the shell is up.
6. Welcome failures are isolated; the desktop session remains Running.

### Completion model

| Mode | Behavior |
|---|---|
| `process-success` (default, fixtures) | Successful spawn/exit completes FirstLogin* |
| `app-reported` / `wizard-finished` | Launch alone never completes; app must call `SESSION_STARTUP_COMPLETE` |

**Policy:**

- Startup launch alone does **not** complete Welcome onboarding.
- Crash or early close / Skip-to-close → incomplete (policy remains eligible).
- Explicit **Finish** → report completion → record system generation.
- Manual relaunch with **Finish** may report completion if policy still pending
  (capability-checked; only the live optional process may complete).
- Early dismiss never records completion.

IPC: `SessionMsg::SESSION_STARTUP_COMPLETE` (`0xC11C`).  
Caller must be the live optional process for `org.sunlight.welcome`.

## Launch modes

| Mode | How | UI |
|---|---|---|
| Automatic | sessiond spawn (no argv) | Prominent onboarding |
| Manual | Start Menu / search / `welcome --manual` | Welcome Center; does not auto-consume policy on open |

Single-instance: sessiond skips duplicate plan entries; shell blocks second
launch while the process is alive.

## Greeting provider abstraction

```rust
pub trait WelcomeGreetingProvider {
    fn generate_greeting(&self, request: &WelcomeGreetingRequest)
        -> Result<WelcomeGreeting, GreetingError>;
}
```

Request is bounded: optional display name/locale, version, machine summary
(CPU cores, RAM MiB, device class, optional model, screen size). No serials,
filesystem contents, or private documents.

`resolve_greeting` prefers Wise Owl when available; otherwise local fallback.
Timeouts/errors fall back quietly.

### Local fallback examples

- “Welcome to SunlightOS. Your desktop is ready.”
- Optionally mentions CPU cores / RAM
- Encourages continuing the tour

### Future Wise Owl boundary

`FutureWiseOwlGreetingProvider` is a stub that returns `Unavailable` unless
explicitly marked available. No inference backend is required or linked.

## UI flow

1. **Immediate welcome** — branding, “Your desktop is ready.”, Get Started / Skip  
   (renders without waiting on providers or system queries)
2. **Greeting** — short provider text + system summary card
3. **Slides** (7 static topics): desktop, search, Control Panel, files, terminal,
   reliability, local-first / Wise Owl direction
4. **Action cards** — open existing apps/pages or honest placeholders

Keyboard: Enter = primary, Esc = dismiss incomplete, arrows for slides.

## Action-card behavior

| Card | Action |
|---|---|
| Personalize desktop | `control-panel --page wallpaper` |
| Open Control Panel | `settings` |
| Browse files | `files` |
| Open Terminal | `terminal` |
| Learn more | `control-panel --page about-os` |
| Meet Wise Owl later | **Coming Soon** (honest placeholder) |

No pretended Wise Owl optimization. No arbitrary shell commands.

## State persistence

| Kind | Storage |
|---|---|
| Session onboarding completion | Session Profile `policy_state` via sessiond |
| UI-local | In-memory for this phase (no chat, no history) |

Does **not** use Wise Owl MemoryDB.

## Capability model

Normal user application with `UserSession` caps when launched as a startup app.

May: read bounded system summary, report own startup completion, open allowed
apps/pages, render UI.

Must not: create sessions, edit other users, grant capabilities, execute
arbitrary commands, inspect private data, reconfigure Startup Apps beyond its
own completion signal.

## Failure handling

| Case | Behavior |
|---|---|
| Wise Owl unavailable | Local greeting |
| System info partial | Reduced summary; continue |
| Early close | Incomplete; session intact |
| Crash | Session remains Running; policy not consumed |
| Completion IPC fails | Eligible for later relaunch |
| Action unavailable | Bounded status message |

## ISO test sequence

```text
./tools/test.sh welcome-wizard
```

Real QEMU ISO. Required serial markers:

```text
[WELCOME-WIZARD] BUNDLE_DISCOVERED PASS
[WELCOME-WIZARD] SESSION_ELIGIBLE PASS
[WELCOME-WIZARD] SHELL_READY_FIRST PASS
[WELCOME-WIZARD] AUTO_LAUNCH PASS
[WELCOME-WIZARD] FALLBACK_GREETING PASS
[WELCOME-WIZARD] SLIDESHOW PASS
[WELCOME-WIZARD] ACTION_CARD PASS
[WELCOME-WIZARD] COMPLETION_RECORDED PASS
[WELCOME-WIZARD] NO_REPEAT_AFTER_COMPLETION PASS
[WELCOME-WIZARD] MANUAL_RELAUNCH PASS
[WELCOME-WIZARD] FAILURE_ISOLATION PASS
[WELCOME-WIZARD] RESOURCE_BASELINE PASS
[WELCOME-WIZARD] IDLE_CPU PASS
[WELCOME-WIZARD] FINAL PASS
```

Inject phase `welcome_wizard` builds sessiond/tty/welcome with test auto-drive
for deterministic slide/completion flow after real Shell Ready launch.

## Host tests

```text
cargo test --package sunlight-sessiond --lib --target x86_64-unknown-linux-gnu
cargo test --package sunlight-welcome --lib --target x86_64-unknown-linux-gnu
```

## Resource measurements

Measured on real ISO via gate markers (telemetry poll baseline + idle window).
Report page-backed process allocations from telemetry when available; do not
label as RSS unless the OS exposes RSS.

The app exits after Finish/Close and must not remain resident.

## Known limitations

- Spawn path must stay short (`/bin/welcome`) for sessiond `SpawnRequest`
- Shell catalog is still hardcoded (Welcome added explicitly; not dynamic scan)
- IPC app-id packing extended to 32 bytes; older 16-byte clients need update
- UI-local state not persisted across process restarts this phase
- Optional restart `OnFailureOnce` for optionals remains incomplete (pre-existing)
- Endpoint/SHM per-object inventories limited for leak gates

## Future Wise Owl integration entry criteria

- Greeting provider can call a local Wise Owl service with a short timeout
- Capability check before any interactive action
- No blocking of first paint or tour navigation
- Still fall back to `LocalGreetingProvider` on any failure
- No chat UI until a dedicated product phase

## Explicit non-implementation confirmation

Full Wise Owl inference, chat UI, Pattern Recognition, Session Restore, and
self-healing were **not** implemented in this phase.
