# Melody Mina Phase 1

Melody Mina is a native, UI-only SunlightOS music-player frontend. It contains
no codec, PCM, audio-device, playback-thread, or media-service implementation.

## Architecture audit

The existing platform pieces reused directly are:

- `sunlight_ui::App`, `Window`, `WindowConfig`, and normal compositor chrome;
- `LayoutConstraints`, `Measure`, `Arrange`, `LayoutBox`, `Sizing`,
  `AxisSizing`, `Row`, `Column`, and `LayoutInvalidation`;
- `Theme` and semantic chrome roles, MiniType typography through `sun-font`,
  built-in `UiSymbol` glyph rendering, rounded drawing, and TGA rendering;
- the shared `Slider`, `ScrollState`, scrollbar drawing/hit testing, clipped
  zero-copy sub-canvases, mouse events, focus events, keyboard events, ticks,
  and typed client resize propagation;
- the kernel embedded-binary resolver, `sun_exec` command registry, Vortex
  running-app registry, Start menu catalog, and search palette.

The small reusable gap was a glyph-only button with normal, hovered, pressed,
disabled, primary, and keyboard-focus presentation. `sunlight-ui` now provides
`IconButton`, backed by the shared theme and `UiSymbol` renderer. Generic media
symbols were added to the same glyph set.

Album artwork, now-playing metadata, playlist rows, timeline composition, and
visualization remain Melody-Mina-specific because their semantics are not yet
general toolkit concepts.

## Component and state boundaries

```text
MelodyMinaApp
├── NowPlayingViewModel (presentation state only)
├── AlbumArtView (optional decoded TgaImage input + fallback)
├── PlaylistView (static item slice + ScrollState)
├── Slider (timeline presentation)
├── IconButton × transport/header actions
├── Slider (volume presentation)
└── VisualizerView
    └── VisualizationFrame <- VisualizationSource
                              └── DemoVisualizationSource (Phase 1 only)
```

`NowPlayingViewModel` contains only displayable identity, time, seekability,
and playback-state values. Demo data lives in one static model module. The
visualizer consumes a bounded `VisualizationFrame`; it never reads audio
buffers. A future shared media service can replace the demo state/source with
high-level playback snapshots and analyzed visualization frames without adding
codec or driver knowledge to the UI.

## Responsive behavior

Mode selection derives from measured minimum composition widths rather than a
device-name breakpoint:

- Large: square album art beside metadata and playlist.
- Medium: square album art beside metadata, with the playlist below.
- Narrow: album art, metadata, playlist, visualizer, timeline, and transport
  form one vertical composition, with compact height metrics when necessary.

All three paths use `sunlight-ui` Row/Column measurement and arrangement.
Artwork remains square, the playlist is clipped and independently scrollable,
and all geometry uses saturating arithmetic.

## Visualizer resource behavior

The visualization path uses fixed `[u8; 48]` storage. Bar count responds to
available width (24–48), and no vector grows during ticks or painting. The demo
producer implements fast attack and slower decay with deterministic integer
math. Rendering is capped near 30 fps while focused and reduced to 10 fps when
unfocused. The current window protocol has focus and pointer ownership events
but no explicit visibility/minimize event delivered to applications, so full
animation suspension while hidden is not yet available through `sunlight-ui`.

## Integration

The executable is `/bin/melody-mina`. It is registered as `MelodyMina` in the
shared shell app-state registry, Start menu, search palette, running-app icon
resolution, kernel embedded-binary resolver, and `sun_exec` command registry.
The normal native app registry is used; a `.sunapp` bundle is not required for
this non-startup native binary under the current conventions.

Phase 1 controls only change visibly labeled presentation state. They do not
claim successful playback, advance time automatically, seek media, or modify
system audio.
