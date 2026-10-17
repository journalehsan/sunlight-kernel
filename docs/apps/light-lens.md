# Light Lens image editor

Light Lens retains its three-column native UI: tools, preview, and image info.
Launch `light-lens /path/to/image.simg` or open an image from Sunlight Files.

## Available editing features

- Zoom buttons and mouse wheel, anchored at the pointer where pan bounds allow.
  `F` fits the window, `1` selects 100%, and `+` / `-` zoom. Manual zoom ranges
  from 1% to 1600%; Fit can go outside that range. Drag the preview to pan.
- Rotate left/right by 90 degrees and flip horizontally/vertically. Dimensions
  follow the rotation. Source alpha survives every operation.
- Crop: enable Crop, choose Free, 1:1, 4:3, or 16:9, then drag on the image.
  The overlay follows zoom and pan. Apply or Enter commits; Cancel or Escape
  discards the selection. Starting another drag replaces the selection. Fixed
  ratios use exact integer pixel multiples. Selection alone does not edit pixels.
- Brightness, contrast, saturation, and gamma preview immediately. Click an
  individual Reset or Reset Adjust. Gamma's neutral value is zero; the slider
  maps exponentially to a gamma of 0.25–4. A drag is one undo transaction.
- Click the filter chip to cycle None, Grayscale, Sepia, Invert, Blur, Sharpen,
  and Edges. No Filter clears it. One filter is active at a time. Spatial
  filters use a bounded 3×3 neighborhood and retain the center pixel's alpha.
- Resize opens a native input dialog. With Aspect On, enter the width and
  height is calculated; with Aspect Off, enter `width x height`. Resampling
  uses nearest neighbor.
- Undo/Redo buttons, Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y, and undoable Reset All.
  Modified state appears in the header, info panel, and zoom status.
- Back/Next and Left/Right preserve folder browsing. Navigation, Escape outside
  Crop, and Ctrl+Q ask before discarding unsaved edits. Choosing No returns to
  editing; Save As can then preserve the work.
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

There is one retained decoded ARGB image, no full-resolution preview buffer,
and no full-image undo snapshots. View operations never modify the editor state.
Preview work scales with the visible viewport; filters sample at most nine
source locations per displayed pixel. The gamma/brightness/contrast lookup
contains 256 entries. Preview rendering is synchronous, so very large viewports
with spatial filters and many geometry steps can still have higher input latency.

The current conservative limits are:

- Input file: 8 MiB plus the 36-byte SIMG header, using the static read buffer.
  This also permits reopening a raw export at the maximum image size.
- Decoded SIMG/TGA and edited geometry: 2,097,152 pixels, at most 8,192 on
  either edge. This includes all eight bundled 1312×816 sample photos and
  1920×1080 images. PNG/JPEG inputs retain a separate 1,048,576-pixel limit
  because their shared decoders need additional full-image scratch buffers.
  Dimensions are inspected before decoding; automatic downscaling is absent.
- Heap: 16 MiB demand-mapped budget for the original and shared decoder scratch
  allocations. A SIMG/TGA original takes at most 8 MiB; a PNG/JPEG original
  takes at most 4 MiB.
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

Crop has a drag outline and thirds grid, without resizable handles. Rotation
supports right angles. Bilinear interpolation, arbitrary angles, exposure,
temperature, grayscale intensity, PNG/JPEG encoding, multiple stacked filters,
and asynchronous/tiled viewport rendering are deferred.

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
from 0×0 through 1220×760. Set `LIGHT_LENS_SCREENSHOT=1` for the render test to
write `/tmp/light-lens-preview.ppm`.

No QEMU boot gate or live VM interaction was run for this change. Native dialog
operation, file publication/failure behavior on SunlightOS filesystems, pointer
capture outside the window, and interactive performance still need VM checks.
