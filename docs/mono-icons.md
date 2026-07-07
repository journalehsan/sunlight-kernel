# SunlightOS Monochrome Icon Pipeline

SunlightOS now has a small monochrome icon path for theme-aware UI glyphs. This is separate from the existing TGA icon theme and separate from any future wallpaper pipeline.

## Scope

- Source assets: PNG
- Build-time output: raw 1-bit rows
- Runtime loading: `include_bytes!`
- Draw-time tinting: caller supplies the theme color

No wallpaper logic, Bing download logic, or general image framework is included here.

## Build-time conversion

The current implementation uses a tiny local PNG-to-raw converter in `services/sunlight-control-panel/build.rs`. It accepts PNG input, emits deterministic 1-bit row-major bytes, avoids PBM headers, and fails cleanly on invalid input.

Equivalent CLI shape for a future standalone wrapper:

`sun-mono-img -o output.raw input.png`

Reference command shape from the evaluated crate idea:

`embedded-mono-img -o generated/icons/save.raw assets/icons/save.png`

Current SunlightOS convention prefers `OUT_DIR` for generated icon bytes, then includes them with:

`include_bytes!(concat!(env!("OUT_DIR"), "/icons/preferences-symbolic.raw"))`

## Runtime API

Shared runtime support lives in `sunlight-ui/image/mono_icon.rs`.

- `MonoIcon` stores width, height, and raw bytes
- `draw_mono_icon` draws `On` pixels in the supplied color
- `Off` pixels are transparent/skipped
- The same icon can be drawn with normal, hover, accent, or disabled colors without regeneration

Example:

`const ICON: MonoIcon = MonoIcon::new(16, 16, include_bytes!(concat!(env!("OUT_DIR"), "/icons/preferences-symbolic.raw")));`

## Theme mapping

`sunlight-ui/theme.rs` now exposes dedicated icon colors:

- `theme.icon_foreground`
- `theme.icon_muted`
- `theme.icon_disabled`
- `theme.accent`
- `theme.warn`

Suggested state mapping:

- Normal → `theme.icon_foreground`
- Hover → `theme.accent`
- Disabled → `theme.icon_disabled`
- Selected → `theme.accent`

## Current integration

`services/sunlight-control-panel` includes one generated PNG-based monochrome icon and draws it in the header with `draw_mono_icon`.

This provides the seed pipeline for future toolbar, menu, checkbox, and symbolic action icons while leaving the wallpaper path untouched.
