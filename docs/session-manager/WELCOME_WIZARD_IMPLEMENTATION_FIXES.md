# Welcome Wizard bring-up: permanent fix notes

Status: **keep this document**. It records real boot failures and the permanent
fixes so the same traps are not reintroduced.

Related design: [WELCOME_WIZARD_PHASE1.md](./WELCOME_WIZARD_PHASE1.md)  
Related stack: Session Foundation + Session Configuration  
Primary crates: `sunlight-welcome`, `sunlight-sessiond`, `sunlight-vortex-shell`

---

## Mission

Phase 1: ship **Welcome to SunlightOS** as the first real optional Session
Startup App:

```text
Login → sessiond → Vortex Shell Ready → (settle) → org.sunlight.welcome
```

Goals that shaped the fixes:

1. Desktop must paint first; Welcome must not steal a black framebuffer.
2. Completion must not fire on mere process spawn (wizard-finished only).
3. Local greeting always works; Wise Owl is a future stub only.
4. No special-case hardcoding inside Login or Vortex lifecycle.

---

## What we implemented (summary)

| Area | Implementation |
|---|---|
| App | `sunlight-welcome` binary `/bin/welcome`, dark sunlight-ui wizard |
| Bundle | `WiseOwlWelcome.sunapp`, id **`org.sunlight.welcome`** |
| Session | `[session] startup_eligible`, `default_enabled`, `default_policy`, `completion_mode = wizard-finished` |
| Seeding | `default_enabled` + explicit Welcome seed on login |
| Launch | After Shell Ready, **deferred** spawn (`DESKTOP_SETTLE_MS` ≈ 2.5s), staggered optionals |
| Completion | `SessionMsg::SESSION_STARTUP_COMPLETE` (`0xC11C`), not process start |
| Greeting | `LocalGreetingProvider` + `FutureWiseOwlGreetingProvider` stub |
| Shell | Start Menu / search / `AppId::Welcome` / `sun-exec welcome [--manual]` |
| Tests | Host tests in `sunlight-sessiond` + `sunlight-welcome`; ISO gate `./tools/test.sh welcome-wizard` |

---

## Bugs found on real ISO boots (and permanent fixes)

### 1. Light UI / unreadable text

**Symptom:** Cream window body with near-invisible text.

**Cause:** Welcome painted a light background while labels used the default
**dark** `Theme` (`theme.text` ≈ white).

**Fix:** Use `theme.panel`, `theme.chrome.*`, `theme.text` / `theme.text_muted`
only. No ad-hoc light cream surfaces.

**Do not:** Hand-roll light backgrounds on top of `Theme::sunlight_dark()`.

---

### 2. Welcome raced the shell (black screen / “no desktop”)

**Symptom:** Welcome appeared on pure black; desktop missing or flashing.

**Cause (two layers):**

1. **sessiond** launched optionals *immediately* on `SESSION_COMPONENT_READY`,
   before Vortex finished first paint / session activation.
2. **Vortex panic** (see #4) killed the shell after activation, leaving Welcome
   alone on black.

**Fix:**

- On Shell Ready: **schedule** optionals, do not spawn yet.
- `DESKTOP_SETTLE_MS` (2.5s) then spawn one-by-one (`OPTIONAL_STAGGER_MS`).
- Welcome retries display connect / window create briefly.

**Do not:** Spawn optional GUI apps in the same tick as Shell Ready.

---

### 3. Plan had `profile_entries > 0` but `optional = 0`

**Symptom (serial):**

```text
WELCOME_BUNDLE_DISCOVERED PASS
profile_entries=1 or 2
plan_components=1 optional=0
no optionals to schedule after shell ready
```

Desktop ran; Welcome never auto-launched.

**Causes observed / mitigated:**

| Issue | Fix |
|---|---|
| Long bundle id `org.sunlight.wiseowl-welcome` (28 chars) interacted poorly with logging / matching edge cases | Canonical id **`org.sunlight.welcome`** (20 chars) |
| `default_enabled` seed + second seed path created duplicate / mismatched rows | Migrate legacy ids, dedup Welcome entries |
| Resolve skipped pending Welcome | `repair_welcome_optional`: catalog as source of truth, inject into frozen optionals |
| Seed only on `ProfileLoadStatus::Missing` | Also seed empty Ok profiles; always ensure Welcome if catalog has it |

**Diagnostic lines to keep (short, multi-line — avoid one 160-byte heapless line):**

```text
[SESSION-CONFIG] plan c=… opt_plan=… opt_rt=… cat=… prof=… gen=…
[SESSION-CONFIG] prof[i] id=… pol=… en=…
[SESSION-CONFIG] opt[i] id=… path=…
[SESSION-CONFIG] welcome cat path=… def_en=…
[SESSION-CONFIG] optionals deferred until desktop settle
```

**Healthy first-login example:**

```text
opt_rt=1
opt[0] id=org.sunlight.welcome path=/bin/welcome
optionals deferred until desktop settle
… ~2.5s …
SPAWN … /bin/welcome
```

---

### 4. Vortex Shell panic — root cause of “Welcome on black”

**Symptom (serial):**

```text
[SESSION] switched to F2 GraphicalDesktop
[VORTEX] panic at services/sunlight-vortex-shell/src/main.rs:2864
…then…
SPAWN /bin/welcome
```

**Cause:** When adding `AppId::Welcome` as the **16th** app:

```rust
// apps array grown
apps: [DockAppState; 16],

// registry scratch arrays NOT grown
const APP_REGISTRY_LEN: usize = 15;  // BUG

// then:
prev_states[idx] = app.state;  // idx == 15 for Welcome → OOB panic
```

**Fix:** `APP_REGISTRY_LEN = 16` (must equal `APP_COUNT` / `apps.len()` /
`AppId` variant count). `sunlight-shell-appstate::APP_COUNT` was already 16.

**Permanent rule:** Any new `AppId` **must** update:

1. `sunlight-shell-appstate` `AppId` + `APP_COUNT` + registry arrays  
2. Vortex `AppId` + `apps: [DockAppState; N]` + **`APP_REGISTRY_LEN`**  
3. Start Menu catalog length, search palette, paths, icons  

If any of those lag, expect OOB panics after desktop activation—not at compile time.

---

### 5. Completion reported as failed

**Symptom:**

```text
[WELCOME-WIZARD] completion report failed (will remain eligible)
```

**Cause:** Strict match on optional `app_id` / pid / `completion_mode` rejected
valid Finish when entries were repaired or pid bookkeeping was slightly off.

**Fix:** `complete_app_reported` accepts Welcome-family ids, allows Welcome
even if `completion_mode` defaulted oddly, and is less brittle on pid for the
live Welcome process.

**Rule:** Finish must call `SESSION_STARTUP_COMPLETE`; spawn alone must never
complete `FirstLogin*` for Welcome (`completion_mode = app-reported` /
`wizard-finished`).

---

### 6. UI copy / theme

- Dark charcoal panel + titlebar chrome (matches Control Panel / Calculator).
- Word-wrapped body text (Label is single-line).
- Honest placeholders for future Wise Owl actions.

---

## Bundle identity (canonical)

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
completion_mode = "wizard-finished"
launch_path = "/bin/welcome"   # must be short (SpawnRequest path limit)
```

Legacy id `org.sunlight.wiseowl-welcome` is migrated by sessiond on login
(re-arms tour once so users who never saw Welcome still get a run).

---

## Launch sequence (correct)

```text
1. Login auth OK
2. sessiond CREATE → frozen plan (shell + optional Welcome)
3. Spawn Vortex Shell
4. Vortex HELLO + READY
5. sessiond: SHELL_READY, schedule optionals (not spawn)
6. Vortex first frame + session activate (desktop owns framebuffer)
7. After DESKTOP_SETTLE_MS: spawn /bin/welcome
8. Welcome window on top of live desktop
9. User Finish → SESSION_STARTUP_COMPLETE → policy generation recorded
10. Next login same generation: no auto Welcome; manual Welcome Center OK
```

---

## Files / crates map

| Path | Role |
|---|---|
| `sunlight-welcome/` | App crate (lib greeting/flow + bin UI) |
| `WiseOwlWelcome.sunapp/` | Trusted bundle under `/Applications` |
| `services/sunlight-sessiond/` | Plan, seed, defer, complete, repair |
| `services/sunlight-sessionctl/` | CLI; 32-byte app-id packing |
| `services/sunlight-vortex-shell/` | AppId Welcome, Start Menu, APP_REGISTRY_LEN |
| `sunlight-shell-appstate/` | APP_COUNT / AppId lockstep |
| `services/tty_server/` | ISO gate hooks for welcome-wizard |
| `sunlight-fs/src/ramfs.rs` | Bundle + `/bin/welcome` install |
| `kernel/build.rs`, `main.rs`, `spawn.rs` | Embed + map welcome ELF |
| `sunlight-libc/src/sun_exec.rs` | `welcome` alias |
| `ipc/src/lib.rs` | `SESSION_STARTUP_COMPLETE` |
| `tools/test.sh`, `tools/tests/welcome_wizard.expected` | ISO gate |

---

## Regression checklist (before calling Welcome “done”)

- [ ] No `[VORTEX] panic` after session activate  
- [ ] Serial: `opt_rt=1`, `opt[0] id=org.sunlight.welcome path=/bin/welcome`  
- [ ] Serial: `optionals deferred until desktop settle` then spawn welcome  
- [ ] Desktop wallpaper/dock visible **behind** Welcome  
- [ ] Dark theme, readable contrast  
- [ ] Finish records completion (no “completion report failed” for happy path)  
- [ ] Second login same generation does not auto-launch Welcome  
- [ ] Manual Start Menu / `welcome --manual` still opens Welcome Center  
- [ ] New `AppId` updates APP_COUNT **and** APP_REGISTRY_LEN  

---

## Explicit non-goals (still true)

Not implemented: full Wise Owl inference, chat UI, Pattern Recognition,
self-healing, Session Restore, remote content, analytics upload.

---

## Lessons for future optional session apps

1. Treat optionals like sun-exec apps: **after** shell ready **and** settle.  
2. Keep SpawnRequest paths short (`/bin/…`).  
3. Prefer short reverse-DNS bundle ids (≤24).  
4. Never complete one-time policy on spawn alone if the UX is a wizard.  
5. Growing shell `AppId` is a multi-crate size lockstep problem—panic, not compile error.  
6. Prefer several short serial markers over one long heapless line.
