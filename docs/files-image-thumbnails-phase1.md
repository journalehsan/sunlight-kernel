# Files image thumbnails, Phase 1

## Audit and restoration decision (before implementation)

Commit `411f5dfea` records the original fatal instruction-fetch fault: a native
preview worker exiting unmapped its parent's code. It removed the asynchronous
mailbox and replaced it with synchronous preview loading. Current native thread
and address-space code explicitly distinguish borrowers from owners (MM-0).
Files still reads and decodes selected images in its UI callback, keeps an 8 MiB
static source buffer, and stretches previews. There is no directory image grid:
Directory is a detailed list, and Home contains folder cards. Restore previews
in the existing selection pane; keep detailed-list icons unchanged.

The separate thumbd service is disabled in sunlightd because malformed-image
prewarming panicked and left a zombie-ready loop. It also uses a 512 KiB bump
allocator with no reclamation, permits 32 MiB input, writes lossy-alpha TGA24,
and hashes paths without modification metadata. It is unsuitable for restoration
without a service redesign. Leave it disabled and reuse the shared image pipeline
inside Files, with one persistent native worker and existing notifications.

sunlight-ui image::simg already supplies owned decoding, alpha-aware scale_fit,
and ARGB rendering. sun-img supplies strict header inspection and PNG/JPEG codecs.
TGA type 2 (24/32-bit), legacy SIMG/TGA, SIMG v2, static PNG and baseline/progressive
JPEG have working decoders. BMP is detected but is not decoded. sunlight-media
is an audio pipeline and is outside Phase 1.

Filesystem enumeration is paged, preserving order and selection. DirEntry and
Stat expose size/type but no inode or modification time. Cache validation must
therefore read bounded content on the worker; path/extension/size alone cannot
validate a preview. A content fingerprint detects same-size edits on a new request.

Minimal strategy: replace synchronous image selection loading with a generation
validated mailbox, a one-job worker, bounded input and pixel budgets, and an LRU
memory cache of scaled ARGB images. Selection is the only visible preview request.
The UI consumes completion on its existing event cadence without new polling loops.

## Implemented components and pipeline

- `sunlight-files/src/thumbnails.rs`: image provider, request/result slots,
  generation validation, content-validated LRU and persistent worker.
- `sunlight-files/src/main.rs`: selection submits a path; normal ticks collect
  completion; existing ARGB drawing centers an aspect-fitted preview. Removed
  the 8 MiB static image buffer, synchronous image loader and unsafe borrowed-to-
  static TGA rendering. Compact/detailed row icons, ordering and interactions
  remain intact. Text previews retain their existing bounded implementation.
- `sunlight-files/Cargo.toml`: uses the existing 16 MiB reclaiming heap option.
  Previously Files had an 8 MiB heap **plus** an 8 MiB image BSS buffer; replacing
  the latter with temporary decoding space keeps the same nominal combined
  budget and releases full-resolution pixels immediately after scaling.
- `sun-img/src/compressed.rs` and `sun-img/tests/compressed.rs`: reject a JPEG
  whose decoder dimensions disagree with the first inspected frame header,
  before allocating entropy-decoding output. This prevents a later conflicting
  SOF header from bypassing the thumbnail provider's preflight pixel budget.

The kernel audit found native `ipc_notify_send` is a no-op and notify-wait cannot
be woken through it. The final implementation therefore uses the **existing**
private endpoint and deferred-reply APIs. The worker calls its UI-owned endpoint
when idle. The UI uses nonblocking receive and defers that reply until work is
pending, then completes it without waiting. The worker can then take the latest
request from shared memory. No nameserver registration, external service,
new message protocol, or kernel functionality was added. The mailbox has an
acquire/release try-lock: neither side spins on it. Contention defers work to the
next existing tick. One result, one mailbox request and at most one UI deferred
request are retained; newer selection replaces queued work. There is only one
active decoder. Worker ownership uses an Arc, independent of the movable app.

Directory/view changes and selection changes increment a generation. Stale
requests are skipped before decoding; stale completions cannot reach rendering.
Only the selected image has a visible preview, so no offscreen directory images
are read or queued. A normal themed image icon serves as both pending placeholder
and failure fallback. A preview uses at most 128x128 ARGB pixels, preserves alpha
and aspect ratio, and fits the existing pane again on resize. It causes no layout
shift. The isolated provider returns a small Preview value; future providers can
produce that value without changing the file manager's drawing or selection code.

## Cache and resource bounds

The cache holds at most 1 MiB of scaled pixel storage and 64 entries (including
negative entries), with LRU eviction. Path data is bounded by Files' 256-byte path
limit. Each thumbnail is at most 64 KiB; mailbox/UI references can temporarily
retain additional evicted previews. Cache counters measure cache pixels only,
not total process memory. Full-resolution sources and decoder intermediates are
not cached.

Limits are 4 MiB encoded input (512 KiB for PNG/JPEG to bound encoded
intermediates), 2048 pixels per dimension, and 1,100,000 decoded
pixels for PNG/TGA/SIMG. JPEG is limited to 131,072 decoded pixels because the
existing progressive decoder additionally retains coefficient buffers. These
bounds are checked using shared signature/header inspection **before** full
decoding. The provider's stricter budget supplements the codecs' existing
validation, CRC checks, checked arithmetic and scan limits. This enables existing
1312x816 SIMG samples while protecting the bounded heap; many camera JPEGs and
large PNGs intentionally fall back to icons. No new codec or dependency was added.

Stat lacks timestamps/inode IDs. Every new selection request therefore reads
bounded content in the worker and computes a 64-bit FNV fingerprint. A matching
path and fingerprint reuses pixels (or a cached decode failure); same-size edits
cause replacement. Missing/unreadable sources remove their path's cache entry.
Renamed paths use new keys; unreachable old keys expire through LRU. Refresh and
focus refresh reset selection and invalidate the displayed result. Writes from
another app require refresh/reselection: there is no filesystem watch API in this
implementation. Concurrent same-size replacement during a read cannot be made
atomic with the exposed metadata; a subsequent selection revalidates content.
Fingerprint collisions remain a theoretical limitation; extensions are never
used as evidence of cache validity.

Read loops handle short reads, reject incomplete/changed-size sources and close
handles on every read outcome. Allocation of the encoded input uses try_reserve.
Decoder/scaler allocations remain those of the existing shared infrastructure;
resource bounds constrain them, but shared decoders do not expose fallible
allocation for every internal buffer. Corrupt/unsupported/over-budget data returns
the normal image icon. Negative cache entries prevent repeated decoding while
resident; eviction permits another attempt. Cancelling an in-progress decode
invalidates its result rather than preempting the codec. Worker spawn failure
leaves icon-only operation. Close terminates the worker with the owning process;
the existing native thread API still defers individual stack/TLS unmapping.

## Formats

Supported within the above bounds: legacy SIMG (TGA bytes), SIMG v2, uncompressed
true-color TGA type 2 at 24/32 bits, static PNG (palette, grayscale, transparency,
16-bit reduction, Adam7) and baseline/progressive JPEG (including the shared
codec's grayscale/CMYK conversion). BMP, GIF, WebP, APNG, SVG, RLE/paletted TGA,
audio, video, PDF and documents have no Phase 1 thumbnail provider.

## Validation and measurements

Host tests cover supported PNG/JPEG/SIMG/TGA, alpha, corrupted/truncated/unknown
input, limits, empty models, mixed workloads, rapid request replacement, stale
results, try-lock deferral, cache reuse/eviction, same-size modification, cache
removal, decode failures, clipped rendering, resize geometry and 100 model
construction/destruction cycles. Host lifecycle tests do not simulate native
thread teardown. Shared codec tests include conflicting JPEG frame headers.

Commands:

```sh
cargo test -p sunlight-files --target x86_64-unknown-linux-gnu
cargo test -p sun-img -p sunlight-ui --lib --tests --target x86_64-unknown-linux-gnu
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' cargo build -p sunlight-files --release
./tools/test.sh
./tools/test.sh phase_sec
```

The complete shared test command including doctests encountered an existing
`sunlight-ui/horizon.rs` line 5 diagram parsed as Rust (unexpected `|`). Unit and
integration suites pass. Existing dependency warnings remain. No workspace-wide
or graphical claim is inferred from these targeted checks.

An isolated host cache workload (128x128 synthetic TGA; every third entry invalid,
all unique paths, then one last-entry repeat) measured 1.763/9.882/43.974/82.236 ms
for 10/100/500/1000 entries. Peak **cache pixel** storage was 393,216 bytes for 10
and 1,048,576 bytes for the larger runs. Each run recorded N misses and one hit:
its deliberately cold unique-path hit rate is 1/(N+1). Repeated valid-path tests
also confirm the exact same Arc is reused. These timings were captured before
the final source-limit adjustment; that adjustment does not affect these small
fixtures. They are cache/provider timings, exclude filesystem/IPC/compositor
latency, and are not guest UI latency or whole-process peak-memory measurements.
Native debug builds log elapsed request milliseconds, hit/miss counts, current
and peak cache pixels, and active/queued bounds. Release builds omit those logs.

## Exact graphical acceptance steps (not yet verified)

1. Build/boot the updated QEMU desktop with `./tools/runs.sh --build --gui`
   (or deploy the rebuilt binaries to the existing VMware guest using the normal
   image workflow). Log in and launch Sunlight Files.
2. Open Pictures and select a shipped 1312x816 SIMG sample. Check that the themed
   icon becomes a centered preview, with readable labels and unchanged selection.
   Also select a small PNG, baseline JPEG, progressive JPEG and TGA within limits.
3. Use a transparent PNG/TGA over both light and dark themes. Verify transparent
   edges blend into the pane and portrait/landscape images keep their aspect ratio.
4. Use a directory with 10 images, then 100 and 500 images, then 1000 mixed files.
   Scroll, use arrow keys, select images, open files, and use context menus while
   decoding. Only selected images should decode; row icons/order remain unchanged.
5. Rapidly select several images, click Up/breadcrumb/sidebar, navigate Back, and
   select a different file. Old results must never appear for a new selection.
6. Leave and re-enter the directory and reselect an unchanged image. In a debug
   build confirm a cache hit. Modify it without changing byte length, refresh and
   reselect; confirm changed pixels. Delete or rename it and refresh; verify no
   stale preview. Open a corrupt file, unsupported BMP and oversized image twice;
   check icon fallback and no crash or repeated decode for resident negative hits.
7. Resize from 960x620 to 800x500, 1280x800 and back during loading. Check centered
   aspect fit, no label overlap, stable selection highlighting and scroll behavior.
8. Close/reopen Files at least 20 times, including while a decode is active.
   Check native process/thread counters and serial logs for faults or runaway
   workers. Compare memory after closing against the baseline.
9. For latency, record selection-to-preview time separately from the worker's
   decode/read duration. Record navigation and wheel response during misses and
   cache hits. Actual guest input latency and process peak memory remain unmeasured.

## Phase 2 recommendations

Add inode/change-version metadata and filesystem change notifications before disk
caching. Consider bounded decoder-side JPEG reduction/scanline scaling to preview
large photos without retaining full frames. Add a real image-grid mode only as a
separate UI feature with viewport scheduling. Make shared codec allocation
fallible, measure guest heap peaks, and address native stack/TLS reclamation.
Future audio/video/document providers should return the same bounded Preview,
use the same generation checks/cache policy, and stay disabled until audited.

Final results: Files host suite **13 passed**; shared sun-img suite **23 unit +
7 compressed integration tests passed** (CLI suite has zero enabled tests in this
configuration); sunlight-ui targeted suite **262 passed**. Native Files release
build passed. QEMU Phase 3.0 and Security Hardening gates both passed; the latter
reported native borrower lifecycle (12 finished/reaped + 12 recreated/reaped),
thread address-space lifecycle, and IPC deadline/race checks. These gates validate
kernel/IPC foundations, not the new Files GUI worker end to end. Graphical
acceptance and guest responsiveness measurements remain manual and unverified.

## Phase 1.1 — directory grid and session thumbnail cache

The audit found that Directory had no grid/icon mode: detailed rows always drew
MIME icons and the working image worker accepted only selection requests. The
old `services/sunlight-thumbd` is still disabled and unchanged. Its disk cache,
non-reclaiming allocator and synchronous IPC generation are not used. Phase 1.1
changes only `sunlight-files/src/main.rs`, `sunlight-files/src/thumbnails.rs` and
this document, on top of the existing Phase 1 working tree changes.

Directory now defaults to a native icon grid with fixed 128x112 cells and centered,
aspect-fitted 72x72 icon bounds. Filenames retain their existing font and are
clipped to their cell; selection highlighting, opening, multi-selection and
context menus use the same entry indices. The grid uses the existing themed MIME
icon until its own thumbnail is ready. F6 or the background context menu's
“Icons / Details” action switches to the original detailed list, whose MIME
icons remain unchanged. Grid arrow keys move by column/row; Alt+Left retains
parent navigation. Hit testing, selected-item visibility, page scrolling and
scrollbar content height use the same grid geometry. Lexical path normalization
now removes duplicate separators, `.` and `..` for stable absolute cache paths.

Both grid and selection Preview use the **same persistent worker**, bounded
source reader, header preflight, `decode_simg`, alpha-aware `scale_fit`, and ARGB
renderer. Preview still returns its existing 128-edge image and original source
dimensions; Phase 1.1 preserves Preview's resolution; the PNG scratch limits below apply to both uses for resource safety. No original
pixel buffers are retained. Grid stores the same reduced image through `Arc`,
without copying its pixel buffer. Supported formats and source limits
remain those listed above: SIMG v2/legacy, true-color TGA, static PNG and
baseline/progressive JPEG. Unsupported, malformed, inaccessible and oversized
sources keep their normal icon. PNG pixel limits are additionally capped at
262,144 pixels for 16-bit inputs and 524,288 pixels for 8-bit Adam7 inputs. Header
bytes select these caps before allocation. Inspection of the existing zune-png
implementation confirmed that 16-bit native-depth output coexists with inflated
scanlines and Adam7 uses extra pass scratch; its parser rejects multiple IHDR
chunks. These conservative caps protect Files' 16 MiB heap when the grid cache is
full. Standard 8-bit PNG, TGA/SIMG and JPEG retain Phase 1's limits. Large 16-bit
or interlaced PNGs that previously passed the pixel preflight now fall back to
icons, including in Preview, rather than risking heap exhaustion.

The UI-owned session LRU permits a cache hit before any file read or decode.
Identity is the normalized absolute path plus enumerated file size. The exposed
filesystem API supplies no mount ID, inode, timestamp or change version. Refresh,
focus refresh and Files' create/paste refresh therefore invalidate **all immediate
children of the refreshed directory**, including negative entries, and increment
the directory generation. Returning to a directory retains matching cached
entries until refresh or eviction; size changes invalidate on lookup. The existing
worker cache revalidates source content with its fingerprint after a grid miss,
so unchanged refreshes can still reuse decoded buffers. Deletion, rename,
replacement and same-size modification are handled on refresh. Without refresh,
same-size external edits or mount replacement can remain stale. Concurrent
same-size replacement during reading and hash collisions retain Phase 1's
limitations. No filesystem watch or disk cache was introduced.

`GRID_CACHE_BYTES` is configurable and defaults to **4 MiB**, with at most 512
entries, including cached failures. A 16 MiB thumbnail cache would exhaust Files'
existing 16 MiB total reclaiming heap; this smaller budget leaves decoder and UI
headroom without changing the allocator or kernel. Accounting includes pixel
**capacity**, path capacity, image/entry storage, estimated Arc/allocator overhead
and a conservative reservation for the entry vector. The existing worker LRU
remains bounded to 1 MiB/64 entries; its accounting now also includes capacity,
paths, entry/image storage and overhead. Shared buffers are conservatively counted
in both caches. Active jobs, mailbox references and the selected Preview may
retain a few additional 128x128 buffers outside cache ownership. Full process
peak memory has not been measured. Eviction drops owned Arcs; buffers free when
the last worker/cache/Preview reference is released.

Scheduling has one active decoder, one queued Preview request, one queued grid
request and bounded completion slots. Preview takes priority. Grid submission
walks visible entries in viewport order and schedules at most one outstanding
grid request; fully offscreen cells are not submitted. Duplicate/pending requests
coalesce; cached failures suppress repeated reads/decodes until invalidation or
LRU eviction. Navigation and view switches advance a separate directory
generation without invalidating selection Preview's independent generation.
Obsolete requests are skipped before decoding, obsolete completions are discarded,
and a completion is associated with its exact path and enumerated size. The
worker also checks read length against requested size. An active codec cannot be
preempted: it finishes and its stale result is discarded. The try-lock and
existing deferred IPC wake protocol remain; neither UI nor worker spins. Existing
UI ticks consume completions and submit further visible work; no new timer,
service or polling loop was added.

The App API returns only a repaint boolean; it has no per-item damage submission.
Completion therefore requests the existing window repaint. It does not enumerate,
sort or rebuild the directory, change scroll/selection, or change item dimensions.
Each tile uses a clipped framebuffer view, protecting headers, adjacent filename
cells and the details pane during partial-row scrolling.

Focused host tests cover grid hits/misses, LRU order, conservative memory budget,
negative caching, size changes, refresh invalidation, directory revisits,
duplicate suppression, rapid navigation, stale completions, corrupted fallback,
500 mixed entries, 1000-entry cache workloads, lexical paths, scrolled hit testing,
and distinct per-file pixel rendering. Phase 1 Preview/codec tests remain enabled.

Phase 1.1 validation results (2026-10-11):

- Files host suite: **22 passed**. Includes unchanged Preview tests plus explicit
  single-worker Preview priority, per-file grid rendering and cache tests.
- Shared suites: sun-img **23 unit + 7 integration passed**; sunlight-ui **262
  passed** (`--lib --tests`, host target). Existing dependency warnings remain.
- Native Files release build and normal desktop kernel embedding build passed.
  No kernel source change was needed. `rustfmt --check` and scoped
  `git diff --check` passed.
- `./tools/test.sh`: **Phase 3.0 gate PASSED**, ending with
  `[VFS] Native stat/mkdir RAMFS OK`, `[VFS] Stat OK`, and
  `[SunlightOS] Foundation Complete`. This is a serial boot gate.
- `cargo check --workspace` was attempted and failed in existing std-dependent
  dependencies (including serde_core) because `x86_64-unknown-none` has no `std`.
  Targeted native builds and host suites above passed independently.
- The synthetic 1000-entry mixed workload retained 4,143,060 accounted entry/image
  bytes, with peak 4,143,060 and 95 entries. Its entry-vector capacity was 128
  (6,144 bytes), giving **4,149,204 bytes** including the vector allocation, below
  4,194,304 bytes. This is conservative host cache accounting, not measured guest
  RSS or a decoder/process peak. Worker-cache entry/image accounting peaked at
  986,254 bytes in the separate 1000-source workload, within its 1 MiB bound.
- QEMU booted a normal desktop ISO containing the updated Files binary, and root
  login reached the graphical desktop. The welcome window was dismissed and
  application search opened. Guest keyboard/mouse routing was inconsistent;
  Files could not be reliably launched through that path. **Grid thumbnails,
  progressive replacement, interactive responsiveness and Preview were not
  graphically verified.** No guest latency or memory measurement is claimed.
  Evidence logs and the desktop/search screenshot are in
  `target/files-phase11-evidence/` (generated and not committed).
- `vmrun` exists, but the configured SunlightOS VM
  `/home/ehsantor/vmware/SunlightOS/SunlightOS.vmx` does not. The available VM
  configurations are unrelated Rocky Linux/Windows guests. VMware acceptance
  was not performed.

Remaining graphical acceptance: open Pictures and observe individual sample
thumbnails replacing icons; select images and verify the original Preview;
scroll a mixed 500-image directory; rapidly navigate out/back; inspect cache hits
in a debug build; modify/delete/rename an image and refresh; use F6 and the view
context-menu action; test transparent PNG/TGA, malformed data and oversized JPEG;
resize and close while decoding. Confirm labels, selection, context menus,
double-click behavior and stable scrolling. Refresh deliberately invalidates all
children because filesystem metadata cannot identify the one changed file.
Negative-cache eviction can permit another attempt; codec cancellation is
cooperative through stale-result rejection. These limitations supplement the
Phase 1 decoder/allocator/lifecycle limits documented above.


## Phase 1.2 — Grid / Details toggle and filename layout

The repository audit found a working Phase 1.1 grid, original Details rows,
shared directory entries/selection/actions, and session-only `grid_view` state.
F6 and the context menu already switched modes. There was no toolbar toggle.
Grid text was one centered line; Details drew full names across metadata columns.
Sunlight UI exposes font metrics through `VecText`, and Files uses `sun-font`.
Its resolved emoji sequences are not a complete Unicode grapheme segmenter, and
neither library provides runtime bidi reordering or Persian/Arabic shaping.
The existing `SegmentedTabs` widget supplies shared toggle geometry and chrome.

The directory toolbar now uses that control with readable **Grid / Details**
labels and a single highlighted mode. `SegmentedTabs::with_font` lets it use the
same native vector font as Files; other users retain the bitmap fallback.
F6 and the context-menu action remain available. Mode changes only update the
session boolean and scroll geometry; they do not enumerate, sort, refresh,
rebuild entries, change the path, or recreate the thumbnail provider. Both modes
continue to use the same ordered entry vector and selection bitset. Indices are
safe across switching because the vector is neither copied nor reordered. The
filesystem still exposes no stable inode identity; refresh selection behavior is
unchanged. Current directory-first/name ordering is retained. The existing search
field is a presentation placeholder, without an implemented filtering model;
this phase does not add search/filtering or sortable column headers.

Switching preserves the current pixel offset where possible and ensures the
selected item is visible after updating content height. Resize recalculates
columns and likewise keeps selection visible. Double-click tracking resets on a
mode change so clicks in two different layouts cannot combine accidentally.
Grid arrows use the current column count; Details Up/Down use one row. Enter,
Ctrl/Shift click multi-selection, context actions and directory opening continue
through the existing handlers. With no selection, directional navigation picks
the first/last entry according to direction. Breadcrumb pointer hit testing now
checks both axes, preventing a file click below the toolbar from navigating a
breadcrumb with the same x-coordinate.

`sunlight-ui/font/label.rs` adds a small label helper around `VecText`, without a
second font renderer. It measures actual font advances, wraps after whitespace,
underscores, hyphens or dots where practical, and otherwise uses extended Unicode
grapheme boundaries. The final visible line measures complete prefix-plus-`…`
candidates. No UTF-8 scalar, combining sequence, emoji ZWJ sequence, skin-tone
sequence or flag pair is split. If the first grapheme cannot fit, an ellipsis is
shown; if even the ellipsis cannot fit, the label is empty. Extension preservation
is not implemented. Each label draws through a bounded framebuffer view so glyph
ink/overhang cannot escape its rectangle even during partial scrolling. Cached
line widths avoid measuring again during drawing.

Grid cells are 128x128 with centered 72x72 thumbnail/icon bounds and two reserved
filename lines beginning at y=84. Short labels remain on the first line. The
one-column narrow case constrains cell width to the available pane width. The
existing responsive row/column geometry drives rendering, hit testing, keyboard
movement and scrolling; no horizontal scroll area is created. Selection bounds
and cell height do not change when thumbnails arrive. Files keeps at most 128
cached layouts keyed by filename, available width and line count/font role.
A filename tooltip wraps the full name with the same measured grapheme breaks;
it is computed when hover changes and discarded on resize, navigation, mode
change or other input. Very small windows may physically clip an over-wide
individual grapheme or an extremely tall tooltip; the source name remains intact.

Details keeps the original 34px rows, MIME icons, type labels and right-aligned
sizes. Names and type labels use measured single-line ellipsis with hard clipping.
Columns share one width calculation: Modified appears at pane widths >=620px,
Type at >=380px and Size at >=220px. Less essential metadata disappears to keep
Name usable in narrow panes. Modified remains `--` because directory entries
provide no timestamp. Headers and partially visible rows are clipped to their
pane/viewport. Hover over an entry in either view to see its full filename.

`sunlight-files/src/thumbnails.rs` is unchanged. Selection Preview, image formats,
decoding, rendering, cache identities and cache bounds remain unchanged. Switching
does not advance the directory generation or invalidate the cache: an outstanding
grid job may complete into the session cache while Details is visible. Details
never submits new grid requests. Returning to Grid schedules visible misses and
reuses cached buffers directly. Navigation/refresh retain their existing
cancellation and invalidation behavior. No kernel source, service or background
worker was added. Sunlight UI now directly uses `unicode-segmentation` 1.13.3
with default features disabled; this no_std dependency was already present in the
workspace lockfile. No shaping engine or new registry package was introduced.

Validation commands (2026-10-11):

- `cargo test -p sunlight-files -p sunlight-ui --target
  x86_64-unknown-linux-gnu --lib --tests`: **29 Files + 281 UI tests passed**.
  Existing thumbnail tests remain enabled. New tests cover both mode directions,
  toolbar/F6 switching, unchanged entry storage/path/multi-selection, selected
  visibility, exact thumbnail Arc reuse, Grid/Details keyboard strides, Enter
  dispatch after switching, empty directories, mixed types, resizing/narrow cells,
  actual font measurements, single/two-line names, ellipsis, Unicode boundaries,
  complete tooltip text, bounded cache reuse, and pixel clipping with ink overhang,
  nontrivial framebuffer stride and partially scrolled rows. Host Enter coverage
  uses an invalid UTF-8 entry to exercise the original handler without launching
  a host process; successful guest file opening needs graphical acceptance.
- Native Files release build with the existing userspace linker flags: passed.
- Normal desktop kernel embedding build (`cargo build -p sunlight-kernel`):
  passed. No kernel source changes were required.

Known limits: the current primary font does not include Persian glyphs; those
characters appear as fallback question marks. Missing emoji/combining glyphs also
retain the current fallback behavior. Runtime RTL/bidi shaping remains absent;
grapheme-safe boundaries preserve the source name but do not imply correct complex
script appearance. Sortable headers, timestamp metadata, functional search and
extension-preserving truncation remain future improvements. Existing Phase 1/1.1
cache invalidation and resource limitations still apply.

Phase 1.2 visual evidence and acceptance limits:

- A normal desktop ISO containing the updated Files binary booted in QEMU with
  `-vga virtio`, reached root Desktop login and showed the compositor/Welcome
  window. A dual-GPU trial showed the old login framebuffer on the captured
  primary display; the single-GPU trial corrected that capture issue. Desktop
  keyboard/mouse actions still failed to reliably reach windows, so Files could
  not be opened through the dock/search path. **Files views, guest file launching,
  interactive resizing and thumbnail responsiveness were not visually verified
  in QEMU.** Both test VMs were stopped afterward. No VMware test was performed.
- The explicitly run ignored `write_layout_previews` host test passed and rendered
  the actual Files view methods at 960x620 and 800x500, using a synthetic shared
  entry model and a decoded/scaled shipped SIMG fixture seeded into the thumbnail
  cache. Host Grid and Details previews were inspected: two-line wrapping and
  ellipsis fit their cells, thumbnails stayed centered, Details names fit their
  column, sizes aligned right and metadata adapted to the smaller width.
  These are host framebuffer previews, **not guest screenshots or a latency test**.
  The missing Persian glyphs and some combining/emoji fallback are visible.
- Generated PPM/PNG previews, ISO and guest screenshots/serial logs are under
  `target/files-phase12-evidence/`, excluded from commits. The host preview command
  is `cargo test -p sunlight-files --target x86_64-unknown-linux-gnu
  write_layout_previews -- --ignored`. Before-change Files screenshots are not
  available. Ordinary tests leave this artifact-writing preview test ignored.
- `rustfmt --check` for touched Rust files and `git diff --check`: passed.
  Existing dependency and host-only unused-code warnings remain.

Modified files: `sunlight-files/src/main.rs` (shared view controls/geometry,
filename layout cache/tooltips, row clipping and focused tests),
`sunlight-ui/font/label.rs` (measured grapheme-safe labels and tests),
`sunlight-ui/font/mod.rs` (helper export), `sunlight-ui/widgets/sidebar.rs`
(optional vector font for the existing segmented control), `sunlight-ui/Cargo.toml`
and `Cargo.lock` (existing grapheme library as a direct UI dependency), and this
layout/thumbnail document. Existing thumbnail source and kernel source are unchanged.

Remaining guest acceptance: launch Files in Pictures; select an image, switch
Grid -> Details -> Grid using both toolbar and F6, confirm selection/preview and
cached thumbnails; open a file with Enter/double-click after switching; navigate
folders/breadcrumbs; exercise Ctrl/Shift selection/context actions; resize and
scroll a mixed directory; inspect narrow Unicode names and the full-name tooltip.
