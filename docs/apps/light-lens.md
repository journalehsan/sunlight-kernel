# Light Lens image editor

Light Lens retains its three-column native UI: tools, preview, and image info.
Launch `light-lens /path/to/image.simg` or open an image from Sunlight Files.

## Available editing features

- Zoom buttons and mouse wheel, anchored at the pointer where pan bounds allow.
  `F` / Ctrl+0 fits the window, `1` / Ctrl+1 selects 100%, and `+` / `-` zoom.
  Wheel steps scale with the delta (120 units per detent), including fractional
  detents. Manual zoom ranges from 1% to 1600%; Fit can go outside that range.
  Drag an overflowing preview to pan. Clicking a fitted image retains Fit on
  subsequent window resizes; fitting axes always have zero pan offset.
- Hold `\` or press and hold the Before / After control in the preview title
  bar to compare. The control reads Original while held; narrow previews show
  an Original indicator instead. Release, Escape, or focus loss restores edits.
  Comparison preserves the edit recipe, history, save checkpoint, zoom and pan.
  After crop/rotation/resize, the original is letterboxed in the edited image's
  display rectangle, preserving its own aspect ratio. This is a whole-image
  comparison, not a correspondence between cropped and original pixels.
  Comparison starts between editing gestures, and pauses other editing input.
- Rotate left/right by 90 degrees and flip horizontally/vertically. Dimensions
  follow the rotation. Source alpha survives every operation.
- Crop: enable Crop, choose Free, 1:1, 4:3, or 16:9, then drag on the image.
  The overlay follows zoom and pan. Apply or Enter commits; Cancel or Escape
  discards the selection. Outside the selection is dimmed; a thirds grid and
  eight corner/edge handles remain visible. Drag inside to move, drag a handle
  to resize, or drag elsewhere on the image to replace the selection. Cursor
  shapes distinguish movement and resizing. Fixed ratios use exact integer
  pixel multiples; edge resizes center the opposite axis. Apply is disabled
  below four pixels on either axis (or the full axis size on smaller images).
  Selection changes never edit pixels or invalidate the sampled preview.
- Brightness, contrast, saturation, and gamma preview immediately. Click an
  individual Reset or Reset Adjust. Gamma's neutral value is zero; the slider
  maps exponentially to a gamma of 0.25–4. Neutral ticks, aligned numeric values,
  adjusted-label accents and focus outlines improve scanning. Double-click a
  slider or its label to reset it; click the label to focus without changing it.
  Focused Left/Down and Right/Up adjust by one unit, with existing -100..100
  limits. Enter/Escape releases slider focus. A drag is one undo transaction,
  including a release outside its track or focus loss. Unchanged drag positions
  do not request redraws.
- Click the filter chip to cycle None, Grayscale, Sepia, Invert, Blur, Sharpen,
  and Edges. No Filter clears it. One filter is active at a time. Spatial
  filters use a bounded 3×3 neighborhood and retain the center pixel's alpha.
- Resize opens a native input dialog. With Aspect On, enter the width and
  height is calculated; with Aspect Off, enter `width x height`. Resampling
  uses nearest neighbor.
- Undo/Redo buttons, Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y, and undoable Reset All.
  Modified state appears in the header, info panel, and zoom status.
- Image Info includes a collapsible, alpha-weighted luminance histogram of the
  currently edited output, with up to 64×64 stratified samples. It computes on
  the next idle tick after an edit, never during a slider drag. Stale data is
  replaced by an Updating label until ready; an empty document shows No image.
  Metadata stays visible. The section hides in panels shorter than 450 pixels.
- Back/Next and Left/Right preserve folder browsing. Navigation, Escape outside
  Crop, and Ctrl+Q ask before discarding unsaved edits. Choosing No returns to
  editing; Save As can then preserve the work. Arrow navigation is suppressed
  during crop mode or slider focus. Native dialogs own their keyboard routing;
  unmodified character presses and raw modifier/release events are both handled.
- Wheel over the left panel to reach tools at smaller window heights. AI is
  disabled and marked Future.

## Saving and exporting

Save / Ctrl+S saves to an encodable current path. Save As / Ctrl+Shift+S selects
an export destination. PNG/JPEG inputs open Save As because their encoders are
unavailable. The only accepted output extensions are `.tga` and `.simg`, case
insensitive. Both export top-down, straight-alpha BGRA8 pixels with correct
edited dimensions. SIMG uses the existing v2 codec header, uncompressed payload,
and the format's optional-CRC mode; it does not run the compression candidate
search. TGA uses type 2, 32 bits, and the shared codec header.

An existing destination always requires a separate replacement confirmation.
Export writes to an exclusively created temporary file in the destination
folder, then publishes using existing rename APIs. A new destination uses
rename-no-replace to protect files created while export is running. Failures
and cancellation remove the temporary file; the source is never opened for
truncation. Filesystem support for the rename operation is required. This does
not add a crash-durability guarantee or a cross-filesystem copy fallback.

Export processes at most 2,048 pixels per event-loop tick with an 8 KiB scratch
buffer. Editing and navigation pause while the saved state is being rendered;
Escape cancels. No older background result can replace a newer edit. The
modified flag clears only after publication succeeds. Reset All continues to
refer to the original image loaded for this session, including after Save As.

## Processing and resource limits

The reusable core is `sunlight-light-lens/src/editor.rs`. It keeps a bounded
sequence of inverse geometry operations and global adjustment/filter values.
Rendering maps visible output coordinates back into the immutable original.
Geometry runs first, then adjustments, then the selected filter. Filter changes
replace the current filter rather than baking or stacking effects. Nearest
neighbor resize steps retain their sampling behavior when followed by edits.

There is one retained decoded ARGB image and no full-image undo snapshots.
View operations never modify the editor state. `src/preview.rs` retains one
sampled viewport buffer, capped at 1,048,576 ARGB pixels (4 MiB). Its key includes
source identity, the complete edit state, image display rectangle, visible clip,
and interaction quality. Opening another document releases the old cache before
decoding. A cache hit avoids source sampling, geometry/filter work and LUT
construction; framebuffer composition and UI drawing still run.

Slider drags sample one pixel per 2×2 displayed block; release regenerates full
quality. The buffer is reused with fallible bounded growth. If the full-quality
viewport exceeds the cap or reservation fails, drawing falls back to direct
sampling. Preview work scales with the viewport, not full source dimensions;
spatial filters sample at most nine source locations per sampled output pixel.
The 256-entry gamma/brightness/contrast LUT also survives geometry, saturation,
filter and view changes when its own parameters are unchanged.

Preview rendering remains synchronous; expensive filters and long geometry
chains can still delay an input frame. There are no background preview jobs that
could publish stale edits. Histogram caching uses complete edit-state equality
and is independent of the viewport cache and temporary original comparison.

The current conservative limits are:

- Input file: 8 MiB plus the 36-byte SIMG header, using the static read buffer.
  This also permits reopening a raw export at the maximum image size.
- Decoded SIMG/TGA and edited geometry: 2,097,152 pixels, at most 8,192 on
  either edge. This includes all eight bundled 1312×816 sample photos and
  1920×1080 images. PNG/JPEG inputs retain a separate 1,048,576-pixel limit
  because their shared decoders need additional full-image scratch buffers.
  Dimensions are inspected before decoding; automatic downscaling is absent.
- Heap: the existing 16 MiB demand-mapped budget covers the original, shared
  decoder scratch and viewport cache. A SIMG/TGA original takes at most 8 MiB;
  a PNG/JPEG original takes at most 4 MiB. The viewport cache retains at most
  4 MiB and falls back to uncached rendering if it cannot grow. Decoder scratch
  does not coexist with the old document's cache during navigation.
- Geometry: 16 steps per edit state. An additional step is rejected with a
  message; Undo or Reset All makes room. Crops/resizes also count as steps.
- Undo and redo: 32 parameter states each, with oldest entries evicted. New edits
  clear redo. Slider previews do not allocate full-image buffers.

The shared decoders retain their existing format validation. Decoder allocation
failure still follows the platform allocator's failure path; the app's preflight
limits reduce its exposure but do not make every codec allocation fallible.
No kernel API or GPU/AI dependency was added. `libm`, already present in the
workspace lockfile, supplies the small gamma LUT calculation; native dialogs
come from the existing `sunlight-dialogs` crate.

## Partial and deferred work

Core editing, sidebar/toolbar wiring, adjustment preview, bounded history,
nearest-neighbor resizing, filters, and TGA/SIMG export are implemented.

Rotation supports right angles. Optional split comparison is deferred: hold
comparison provides the requested inspection without another viewport gesture.
Bilinear interpolation, arbitrary angles, exposure, temperature, grayscale
intensity, PNG/JPEG encoding and stacked filters remain outside this phase.
Asynchronous/tiled viewport rendering is deferred in favor of the bounded cache
and interactive sampling; native latency profiling would guide that larger
change. AI Enhance remains disabled and deferred. No enabled placeholder was
added.

Unsaved-change prompts cover application-owned navigation and keyboard exits.
The current `sunlight-ui::Window` host receives an invalidated window after a
compositor titlebar close and exposes no cancellable close event. Titlebar or
forced process closure therefore bypasses the prompt; save first. No shared
window protocol or kernel changes were made for this task. Forced closure can
also leave an unpublished hidden temporary export file.

## Validation

Run focused host tests:

```sh
RUSTFLAGS='' cargo test -p sunlight-light-lens -p sun-img --target x86_64-unknown-linux-gnu
```

Build the native application with the repository's userspace linker flags:

```sh
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' cargo build -p sunlight-light-lens --release
```

Tests cover inverse rotation/flips, crop coordinates with zoom/pan, exact aspect
ratios, adjustment defaults and alpha, history transactions, invalid dimensions,
resize sampling, loading/editing all eight bundled sample images, codec-specific
input bounds, streaming TGA/SIMG round trips, native Click-release drag
completion, viewport clipping, scrolling controls, and rendering at client sizes
from 0×0 through 1920×1080. Phase 2 adds comparison press/release and focus-loss
coverage, character-event keyboard routing, slider focus/reset and no-op moves,
crop-handle movement under zoom/pan, zero-selection restart, histogram correctness
and invalidation, preview quality/state/source/view invalidation, and cache and
history memory bounds. Save tests use a host file adapter around the same export
state machine: header/payload write failures (including simulated disk-full),
cancellation, destination races, successful Save As identity/checkpoint changes,
and Undo/Redo around the save checkpoint. The adapter does not validate native
filesystem rename semantics or native dialog behavior.

Validation for Phase 2: 43 Light Lens tests and 30 sun-img tests passed; the native
release application built successfully. Existing dependency cfg/dead-code warnings
remain. Offscreen rendering covers 1024×768, 1366×768, 1600×900, and 1920×1080
with a real bundled photo, crop handles/dimming, comparison and histogram.
The 1024×768 render was visually inspected. Set `LIGHT_LENS_SCREENSHOT=1` to write
`/tmp/light-lens-preview.ppm` and `/tmp/light-lens-phase2-{width}.ppm`.

Performance evidence is deterministic work counting, not native latency timing:
20 identical cached redraws add zero sampled pixels; a 100×100 interactive
preview samples 2,500 rather than 10,000 pixels, then returns to full sampling
on release. The crop-handle test confirms that overlay movement adds zero source
samples. Cache capacity remains at or below 4 MiB, and histogram sampling is
bounded at 4,096 edited-output pixels. No measured frame-rate or latency gain is
claimed.

No QEMU boot gate or live VM interaction was run for this phase. Native dialog
operation, file publication/failure behavior on SunlightOS filesystems, physical
disk exhaustion, rapid filesystem navigation/open-close cycles, pointer capture
outside the window, and interactive performance still need VM checks.
