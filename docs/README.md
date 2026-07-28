# SunlightOS Documentation

- `WISEOWL_BOUNDED_ACTION_PLANNER_V1.md` — proposal-only English/Persian
  conversation-to-intent planning, registry aliases, clarification, and the
  Trusted Action Flow boundary.
- `WISEOWL_CONVERSATIONAL_ACTION_COORDINATOR_V1.md` — bounded session-bound
  multi-turn action lifecycle, cancellation, invalidation, replay protection,
  localization, and trusted dispatch.

This directory contains project documentation beyond the root quick-start guide.

- `README_TUI.md` — Graphical boot TUI usage and troubleshooting.
- `PHASE_2.5_SUMMARY.md` — TUI implementation summary and architecture notes.
- `PHASE_3_ROADMAP.md` — Split roadmap, sub-prompts, and gates for Phase 3.0/3.5/3.6.
- `TOOLS_SUMMARY.md` — Runner script reference.
- `FINAL_SUMMARY.md` — Full Phase 2.5 overview.

The initial six foundation phases are complete. New work is tracked by subsystem
and milestone rather than by numbered phase.

## GUI / Display Stack

- `GUI/` — Initialize display protocol and graphical interface documents.
  - `GUI/INITIALIZE_PHASE_DISPLAY_PROTOCOL_AND_GRAPHICAL_INTERFACE_ROADMAP.md` — SGP design, blocking event model, Eyes Tracker plan, and compositor notes.

## Framebuffer Login Background

The framebuffer login screen supports an optional static background image:
- Asset path in VFS: `/usr/share/sunlightos/backgrounds/login-background.tga`
- The image is embedded directly in the `tty_server` binary at compile time
- Format: TGA type 2 (uncompressed true-color), 24 bpp
- Rendered with aspect-fill behaviour + ~40% dark overlay for readability
- If the image is missing or malformed at compile time the build fails; at
  runtime the decoder validates the header and falls back to the plain dark
  background on parse failure.
