# SunlightOS Documentation

This directory groups project documentation by subsystem and purpose. Historical
phase plans and session reports are kept under `history/`; current work should
be documented alongside its subsystem.

## Subsystems

- `Arch/` — architecture notes.
- `audio/` — audio capture, playback, and regression notes.
- `GUI/` — display protocol, graphical interface, and desktop notes.
- `ipc/` — IPC, capability, and performance documentation.
- `kernel/` — kernel internals, scheduling, memory, and hardware notes.
- `libc/` — libc and userspace compatibility work.
- `networking/` — network services, DNS, TLS, and HTTP notes.
- `security/` — authentication, authorization, secrets, and security audits.
- `services/` — service design and implementation notes.
- `storage/` — filesystems, block devices, mounting, and swap.
- `terminal/` — terminal behavior, lifecycle, and responsiveness.
- `wiseowl/` — Wise Owl contracts, design, and implementation notes.
- `apps/` — application documentation.
- `rappid-rabbit/` — browser project notes.

Other existing folders cover SSH, Helios Linux compatibility, VMware, Solaris,
session management, shell, Unix conformance, icons, fonts, images, and fixtures.

## Guides and history

- `guides/` — user and developer guides, including TUI usage and tooling.
- `history/phases/` — numbered phase plans, summaries, and milestone records.
- `history/session-notes/` — session summaries and dated working notes.

The early foundation phases are complete. New work is tracked by subsystem and
milestone rather than by numbered phase.

## Framebuffer Login Background

The framebuffer login screen supports an optional static background image:

- Asset path in VFS: `/usr/share/sunlightos/backgrounds/login-background.tga`
- The image is embedded directly in the `tty_server` binary at compile time.
- Format: TGA type 2 (uncompressed true-color), 24 bpp.
- It is rendered with aspect-fill behavior and an approximately 40% dark overlay
  for readability.
- A missing or malformed image fails the build; at runtime the decoder validates
  the header and falls back to a plain dark background on parse failure.
