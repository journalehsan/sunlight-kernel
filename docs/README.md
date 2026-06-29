# SunlightOS Documentation

This directory contains project documentation beyond the root quick-start guide.

- `README_TUI.md` — Graphical boot TUI usage and troubleshooting.
- `PHASE_2.5_SUMMARY.md` — TUI implementation summary and architecture notes.
- `PHASE_3_ROADMAP.md` — Split roadmap, sub-prompts, and gates for Phase 3.0/3.5/3.6.
- `TOOLS_SUMMARY.md` — Runner script reference.
- `FINAL_SUMMARY.md` — Full Phase 2.5 overview.

## GUI / Display Stack

- `GUI/` — Initialize Phase documents for the native display protocol and graphical interface.
  - `GUI/INITIALIZE_PHASE_DISPLAY_PROTOCOL_AND_GRAPHICAL_INTERFACE_ROADMAP.md` — SGP design, blocking event model, 4-phase Eyes Tracker plan, and compositor notes.

## Framebuffer Login Background

The framebuffer login screen supports an optional static background image:
- Asset path in VFS: `/usr/share/sunlightos/backgrounds/login-background.tga`
- The image is embedded directly in the `tty_server` binary at compile time
- Format: TGA type 2 (uncompressed true-color), 24 bpp
- Rendered with aspect-fill behaviour + ~40% dark overlay for readability
- If the image is missing or malformed at compile time the build fails; at
  runtime the decoder validates the header and falls back to the plain dark
  background on parse failure.
