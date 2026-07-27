# Session Manager documentation

This folder collects permanent design notes and implementation fix records for
the SunlightOS **session manager** stack:

| Component | Role |
|---|---|
| `sunlight-sessiond` | Session plan, Vortex Shell lifecycle, optional Startup Apps |
| `sunlight-sessionctl` | CLI for session status and startup profile |
| Vortex Shell | Required graphical shell component |
| Optional Startup Apps | After-Shell-Ready apps (e.g. Welcome Wizard) |

## Documents

| Doc | Contents |
|---|---|
| [WELCOME_WIZARD_PHASE1.md](./WELCOME_WIZARD_PHASE1.md) | Phase 1 design: scope, architecture, completion model, tests |
| [WELCOME_WIZARD_IMPLEMENTATION_FIXES.md](./WELCOME_WIZARD_IMPLEMENTATION_FIXES.md) | What broke during bring-up and how it was fixed (keep forever) |
| [../SUNLIGHT_SESSION_FOUNDATION.md](../SUNLIGHT_SESSION_FOUNDATION.md) | Session Foundation (shell as required component) |
| [../SUNLIGHT_SESSION_CONFIGURATION.md](../SUNLIGHT_SESSION_CONFIGURATION.md) | Session Configuration (startup apps, profile, policies) |

## Quick architecture

```text
Graphical Login
      ↓
sunlight-sessiond
      ↓
Vortex Shell (required) → Shell Ready
      ↓
settle delay (~2.5s)
      ↓
Optional Startup Apps (e.g. org.sunlight.welcome)
```

Welcome is an **ordinary** optional startup app. It is not hardcoded into Login
or Vortex Shell lifecycle.
