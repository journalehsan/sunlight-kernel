# Light Lens image editor

Light Lens uses a three-column native workspace: Edit/Annotate tools, preview,
and Layers/Info/Histogram. Amber accents and the existing CPU drawing and image
sampling paths are retained. Launch `light-lens /path/to/image.simg` or open an image from Sunlight Files.

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
- The right Histogram tab shows an alpha-weighted histogram of the visible
  composite, with up to 64×64 stratified samples. Click its channel button to
  cycle Luminance, Red, Green, Blue, and RGB combined. Calculations run on idle
  ticks after edits and gestures finish. Channel, zoom, pan, selection, layer
  rename and lock changes reuse cached bins. Image and visible annotation edits
  invalidate them. The Info tab retains metadata and folder location.
- Back/Next and Left/Right preserve folder browsing. Navigation, Escape outside
  Crop, and Ctrl+Q ask before discarding unsaved edits. Choosing No returns to
  editing; Save As can then preserve the work. Arrow navigation is suppressed
  during crop mode or slider focus. Native dialogs own their keyboard routing;
  unmodified character presses and raw modifier/release events are both handled.
- Wheel over either tools tab, the Layers panel, or Info to reach content at
  smaller heights. Header Tools and Details buttons collapse either sidebar;
  abbreviated tabs stay visible and clicking one restores the panel. Widths
  contract below 900 pixels. AI remains disabled and marked Future.

## Layers and annotations (Phase 3)

The Layers tab starts with a protected Background layer. Add an annotation layer
before drawing. Layer rows select the active layer; the panel supports add,
delete, rename, visibility, locking, opacity and raise/lower. The top row is the
topmost layer. Background cannot be removed or reordered and has no editable
annotation objects; its visibility and opacity can be changed. Locking blocks
object selection, modification and deletion, plus layer deletion. Image geometry
operations still transform every layer, including hidden and locked layers.

Annotate provides Select / Move, Pen, Arrow, Line, Rectangle, Ellipse, Text and
Highlighter. Drag on the image to draw. Shapes accept both drag directions;
Escape cancels a preview. Strokes use distance-based point suppression and
periodic decimation at the 256-point cap, preserving the current endpoint.
Highlighters start at 96/255 opacity, and self-overlap within one stroke does not
compound alpha. Color and size buttons open native input dialogs. Opacity cycles
through bounded presets. Shapes expose an independent RGB fill color or `none`;
text exposes font size instead of stroke width. Properties apply to a selected
object as one undo operation, or become the style of the next object.

Select clicks the topmost visible, unlocked object across layers and activates
its parent layer. Drag moves it; Delete removes it. Supported shapes and text
show one lower-right bounding-box handle for resizing. Arrow keys nudge a selected
object one document pixel, or ten with Shift. Escape cancels a drag or deselects.
Text placement opens the existing native text dialog, which owns keyboard focus.
Select placed text and use Edit text or Enter to reopen it. Text is single-line,
up to 128 UTF-8 bytes, with the existing UiRegular glyphs. A bounded glyph coverage
cache is shared by history; font size and transforms scale it without a new font
engine. Text rendering and export use identical glyph coverage.

Annotation geometry is stored in continuous image-edge coordinates. Each object
has a stable ID, style and affine transform; order in its layer determines its
Z-order. IDs are not reused after undo branches or Reset All. The immutable
background is never copied into annotation layers. Image rotation, flip, crop
and resize apply matching affine transforms in the same history transaction.
Crop translates objects and clips their rendering to the new canvas; off-canvas
geometry remains editable data and Undo restores the previous canvas. Text also
rotates/flips with the image. Resize scales all geometry, including stroke widths
and text, on both axes. Unsupported extreme or singular transforms are rejected
atomically with feedback. Tool preview, selection and handles follow zoom, pan,
Fit and window size. Switch to Edit to drag-pan an overflowing image; wheel zoom
also works in Annotate. A window resize or focus loss cancels an unfinished
annotation gesture and retains committed objects.

History combines image parameters and shared annotation scenes. Creation,
deletion, movement, resizing, text/style changes and all layer mutations are
undoable; a pointer drag commits once on release. Selection and tab changes do
not create history. Original comparison hides annotations without changing them.
Layer opacity applies once after compositing its objects, so overlaps fade as a
group. Export and preview use straight-alpha source-over with the same coverage
rules. Annotation overlays do not invalidate the background viewport cache.

## Saving and exporting

Save / Ctrl+S exports flattened pixels to an encodable current path. Save As / Ctrl+Shift+S selects
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

Editable project persistence is deferred. TGA and SIMG contain flattened pixels,
including visible annotations, opacity and Z-order. Neither format stores layers
or objects. When annotations exist, Save explicitly confirms flattened export;
a successful export keeps editable layers and history in memory. Application-owned
navigation and exits warn about discarding editable objects even after an export.
There is no project loader, project serializer, or promise of reopening editable
annotations. The existing non-cancellable titlebar-close limitation still applies.

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
could publish stale edits. Histogram caching uses complete image edit-state equality and visible annotation
content, and is independent of the viewport cache and temporary comparison.

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
- Annotation limits: eight layers including Background, 64 objects per document,
  at most 16 text objects, 256 points per stroke, 128 UTF-8 bytes per text object,
  and 32 KiB per native glyph coverage mask. Font and stroke sizes are 1–128
  document pixels. Layer names are limited to 32 UTF-8 bytes. Geometry and
  transform bounds are validated before commit.
- Undo and redo: at most 32 transactions combined, with oldest entries evicted.
  Image parameter frames share annotation scenes, object geometry and text masks;
  new edits clear redo. Slider and annotation previews do not allocate full-image
  buffers. No image-sized layer or undo snapshots are introduced.

The shared decoders retain their existing format validation. Decoder allocation
failure still follows the platform allocator's failure path; the app's preflight
limits reduce its exposure but do not make every codec allocation fallible.
No kernel API or GPU/AI dependency was added. `libm`, already present in the
workspace lockfile, supplies the small gamma LUT calculation; native dialogs
come from the existing `sunlight-dialogs` crate.

## Partial and deferred work

Core editing, tabbed workspace, layers, vector-style annotation geometry,
single-object selection, move/resize, native text input, adjustment preview,
bounded unified history and flattened TGA/SIMG export are implemented.

Object resizing currently uses one lower-right axis-aligned handle. Multi-selection,
object-specific rotation controls, vertex editing, text wrapping, alignment options,
font choice and an editable project format are deferred. Text scales cached native
glyphs, so large text may show pixel edges. Freehand decimation can simplify very
long strokes. No native latency measurement is claimed for dense annotation scenes;
rendering is bounded and clipped, but remains synchronous.

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

## Phase 3 validation

The focused host suite passes 60 Light Lens tests and 30 sun-img tests. The native
release application builds with the userspace linker command above. Dependency
cfg/dead-code warnings remain; no dependencies, kernel features or shared UI
protocol changes were added.

New tests cover stable layer/object IDs, background protection, visibility and
locking, Z-order, movement and resize hit testing, geometry alignment after both
rotations/flips/crop/nonuniform resize, mixed image/annotation undo and redo,
text modification and glyph sharing, malformed text/geometry and allocation caps,
straight-alpha blending and group opacity, flattened streaming TGA/SIMG round trips,
export failure/checkpoint preservation, native Click-release gestures, cancellation,
panel collapse and small-window clipping, and histogram invalidation. Serialization
tests do not apply because no editable project format is implemented.

Work-count assertions confirm annotation creation and pointer previews reuse the
background cache without additional image sampling. This preserves the existing
sampling path; it is not a native frame-rate or dense-scene latency benchmark.
Offscreen annotated photo renders cover 640×480, 1024×768, 1366×768 and 1920×1080;
640×480 and 1024×768 outputs were visually inspected. Set
`LIGHT_LENS_SCREENSHOT=1` for `/tmp/light-lens-phase3-{width}.ppm` artifacts.
No live VM, native text-dialog interaction, or native filesystem publication test
was performed for this phase. The titlebar-close limitation remains as documented
above; application-owned exits/navigation do prompt for editable objects.
