---
name: control-panel-milestone
description: SunlightOS Control Panel (System Preferences) — Day 22/23 first version; Mouse + Monitor pages
metadata:
  type: project
---

Control Panel (`services/sunlight-control-panel`) is a graphical System Preferences app with a Mac-like icon grid.

**Why:** User wanted a simple first-version control panel with Mouse and Monitor settings on Day 22/23.

**How to apply:** Binary is at `/bin/control-panel`. Launchable from the runner by typing `control-panel`.

## Pages implemented
- **Grid** — Two icon cards: Mouse and Monitor. Clicking a card navigates to that settings page. Back button returns to grid.
- **Mouse** — Pointer Speed slider (1-10, maps to ~0.6×–1.8× sensitivity_fp), acceleration checkbox. Apply sends `SgpMsg::SET_MOUSE_SETTINGS` (0xA109) to `display_server`.
- **Monitor** — Shows current resolution via `GET_SCREEN_INFO`. Placeholder resolution options (disabled). TODO: mode switching.

## IPC added
- `SgpMsg::SET_MOUSE_SETTINGS = 0xA109` in `ipc/src/lib.rs`
  - words[0] = sensitivity_fp (i32 as u64; FP_ONE=65536 → 1.0×)
  - words[1] = acceleration_enabled (0=off, 1=on)
- Handler added in `services/sunlight-display/src/main.rs` (just before SESSION_ACTIVATE)
  - Updates `state.pointer.motion.sensitivity_fp` and `acceleration_enabled` live

## Build wiring
- Workspace: `Cargo.toml` → `services/sunlight-control-panel`
- `kernel/build.rs` → package `sunlight-control-panel`, output `control-panel`
- `kernel/src/main.rs` → `SUNLIGHT_CONTROL_PANEL_ELF_BYTES` include_bytes
- `kernel/src/process/spawn.rs` → `/bin/control-panel` and `/usr/bin/control-panel`

## TODOs
- Persist mouse settings via `sunlight-kv` when persistent config infrastructure is wired
- Monitor mode switching (VMware/QEMU virtual display mode-set not yet implemented)
