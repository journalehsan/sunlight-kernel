# Rappid Rabbit browser interaction and chrome

The browser uses Sunlight UI's existing `ScrollState` as its single page-scroll
state. Keyboard scrolling, native wheel events, thumb dragging and track paging
share the offset consumed by DocumentCanvas, image priority and progressive
presentation. Wheel events are restricted to the visible Render pane, retain
signed HID detent counts / larger 120-unit deltas, and do not request layout.
The automatic 10px scrollbar has a stable reserved gutter, so it never covers
links or changes the layout width as images arrive. Geometry updates clamp the
offset after resize, reflow or DOM mutation. Native left-button release is a
`Click`; both that event and explicit `MouseUp` end dragging without activating
content beneath the release. Focus/ownership loss and resize cancel dragging.

Cursor classification uses the same clipped, reverse-paint-order Canvas hit test
as link/control activation. Normal text uses the arrow; links/buttons use Hand;
inputs, textarea and contenteditable text use Text. Live DOM attributes determine
control cursor semantics, including SunJS disabled/type changes. Idle publication,
scrolling and resize refresh the cursor under a stationary pointer. No CSS cursor
engine or independent document hit-test system was added.

Chrome has one active title tab, disabled history arrows, a primary address field,
Reload/Stop, Source (`<>`) and DevTools (`{}`). The outer toolbar frame, GET selector,
Fetch/Open button and permanent HTTP status label are removed. Loading appears in
the tab and the reload button becomes Stop. Titles are bounded and refreshed by
document/DOM revision, with the final URL as fallback. The connection icon is
neutral: an HTTPS URL never implies a verified connection. Address Enter takes
precedence over previously focused forms; URL normalization and validation stay
in the navigation pipeline. Source/DevTools controls have hover labels; F12 toggles
tools and Ctrl+R reloads the committed URL. Narrow layouts hide unavailable history
arrows while retaining address editing and tools.

Stop rejects the result by advancing the navigation generation. Blocking network
IO cannot be interrupted by the existing backend. Its receiver is drained before
another navigation is allowed, preserving the single-worker memory/queue bound.
HTTP details remain in Network and errors remain in Console/document presentation.
Networking, authentication, Account Broker, KV and TLS are unchanged.

## Changed files

- `rappid-rabbit/src/main.rs`: scroll-state integration, viewport gutter,
  responsive chrome layout, dispatch/focus ordering, keyboard shortcuts and
  progressive-image regression tests using actual wheel events.
- `rappid-rabbit/src/browser_ui.rs`: scrollbar event adapter/painting, cursor
  classification, single-tab chrome/title, hover labels and bounded Stop/Reload.
- `rappid-rabbit/src/browser_ui_tests.rs`: deterministic browser interaction tests.
- `rappid-rabbit/tests/fixtures/browser-interaction.html`: local long-page fixture.
- `sunlight-ui/scroll.rs`: preserve wheel magnitude and partial large detents;
  saturating scroll arithmetic and focused regression test.
- `docs/rappid-rabbit-browser-ux.md`: this report.

## Validation

- Browser host tests: **285 passed** with `--features js-dom-fixture --lib --bins
  --tests --target x86_64-unknown-linux-gnu`.
- Shared Golden Fish, image, UI and fetch host tests: **379 passed**, including SVG.
- Document Viewer shared-widget regression tests: **11 passed**.
- `rustfmt --check` on changed Rust files and `git diff --check`: passed.
- `bash tools/build_rappid_rabbit.sh`: native release build passed, including the
  current embedded binary used by the boot image.
- `bash tools/test.sh phase3.0`: **passed**, with `[SunlightOS] Foundation
  Complete`, USB mouse ready, and `Phase 3.0 gate PASSED`.
- Isolated native QEMU desktop: **passed** Enter navigation/title, link/input/button/
  address/normal/scrollbar cursors, wheel down/up and outside-viewport routing,
  track paging, thumb drag to bottom, keyboard Home, and F12 DevTools access.
  Three down/up detents restored the document viewport pixel-for-pixel; wheel
  input over the toolbar left it pixel-identical. Thumb extents moved from
  y=148–351 to y=193–396 after three downward detents, and y=596–797 at bottom.
- Native delayed-image fixture (1800ms per image): wheel input during progressive
  presentation, two visible JPEG paints, and page-ready at 6.64s passed without
  guest faults. These are local fixture timings, not network benchmarks.

Native evidence is under `target/rabbit-ux-native/`: `results.json`, `serial.log`,
`page-top.png`, `page-wheel-down.png`, `scrollbar-drag-bottom.png`, `cursors.png`,
`devtools-open.png`, and `progressive-complete.png`. All are generated, uncommitted
artifacts. The VM used an isolated ISO and local HTTP servers with no user disk.
The VM and servers were shut down after validation. Native resize, Stop under real
blocking IO, live HTTPS, and guest SunJS mutation were not separately exercised;
resize, Stop/generation handling, and SunJS mutation have deterministic host tests.

To serve the same fixtures locally:

```sh
python3 -m http.server 8766 --bind 127.0.0.1 --directory rappid-rabbit/tests/fixtures
python3 rappid-rabbit/tests/fixtures/progressive/serve.py --image-delay-ms 1800
# Guest URLs: http://10.0.2.2:8766/browser-interaction.html
#             http://10.0.2.2:8765/archwiki.html
```

Logs: `target/rabbit-ux-{tests,shared-tests,docview-tests,native-build,qemu}.log`.
Tests cover wheel direction/magnitude/boundaries and routing; drag/track/release;
keyboard synchronization; resize and document shrink; no layout on scroll;
progressive image arrival, resource priority and retained form edits; cursors,
clipping/overlap, navigation and SunJS mutations; Enter, links and GET forms;
disabled history; reload/stop and stale output; title fallback/updates; DevTools
and narrow layouts. Existing HTTPS, HTTP, layout and security regressions remain
in the broader suites. Host tests do not establish live guest HTTPS behavior.

## Existing limits

- Scrolling is page-level. CSS overflow clipping does not provide independently
  scrollable nested regions; no horizontal scrolling or scroll animation was added.
- There is no navigation history, tab manager or user zoom UI. The arrows are
  disabled; there is exactly one document and one visual tab. Existing scaled
  scene/presentation tests cover the layout boundary, not a new zoom interaction.
- The input ABI mixes small HID counts and large 120-unit wheel deltas without
  unit metadata; arbitrary sub-detent touchpad input cannot be distinguished
  reliably. The browser follows the existing shared helper's convention.
- Textarea/contenteditable receive the correct cursor; their editing functionality
  is still limited by the existing document/form implementation. SunJS mutation
  tests use the existing fixture host and do not add production script execution
  or DOM APIs. POST forms remain explicitly unsupported by Rabbit; underlying
  HTTP request-method functionality is preserved.
- After Stop, Reload waits for outstanding blocking IO to finish or time out.
  This avoids an unbounded queue of canceled requests.

No dependencies added. No commit or push performed.
