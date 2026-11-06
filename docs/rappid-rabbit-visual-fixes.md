# Rabbit Render, backgrounds, parser and worker reflow

The modern two-pass arena renderer remains the only renderer. Raster and SVG
resources still use the shared image decoder/cache and DocumentCanvas path.
No decoder or networking dependency was added. The existing tl 0.7.8 dependency
is now a small local MIT fork under `vendor/tl`; its source is tracked by the
kernel embedding build.

## Root causes and implementation

- Startup selected Source, and DevTools defaulted open. Rabbit now starts in
  Render with the panel closed. An explicit navigation selects Render; its
  asynchronous success/failure never changes a Source/DevTools choice made
  while loading. Toolbar focus transitions are tested.
- Arch loads `/static/archlinux_common_style/navbar.css`, whose custom property
  `--archnavbar-logo` references `archlogo.svg` in that directory. Discovery and
  decode already worked, but modern scenes emitted no background images.
  Shorthand also classified `var(...)` before substitution. URLs now become
  absolute at their declaring stylesheet (including custom properties), and
  shorthand expansion waits for substitution. Computed and declared URL
  backgrounds join the shared cache, including inactive media resources for
  later viewport changes. Ordinary navigation links do not consume the image
  discovery budget. Background objects clip to the element border box and
  paint after its color, before its border and descendants.
- The captured 577-byte example.com response contains
  `<script src=/s.js></script>` at byte 535. tl stopped the unquoted attribute
  at `/` (byte 547), consumed it as a self-closing marker, then failed to read
  `>` at `s`. Its tokenizer emitted `s.js>` as raw body text; Golden Fish
  correctly copied that raw node and layout painted it. The fork fixes only
  the unquoted-value delimiter rule: `/` belongs to the value; HTML whitespace
  and `>` terminate it. The fixture keeps the real paragraph and has no heading.
  Neighbor tests preserve legitimate `/s.js>` text, quoted/root/relative URLs,
  whitespace and malformed neighbors.
- UI resize handlers called full layout synchronously. They now update one
  latest viewport/version. One bounded worker owns a snapshot sharing Arc DOM,
  computed styles, scene and raster pixels. It rebuilds geometry and scene diffs;
  active media rules are recomputed only when a breakpoint changes. UI commits
  validate document generation, viewport version and image revision, then apply
  current form state. Unchanged controls skip scene copies/diffs, hidden Source
  is not scanned, and commits use cached diagnostics instead of logging every
  image again. Superseded results never replace the current document.

## Background subset

One ordinary URL layer on block/inline boxes, scroll attachment, padding-box
origin and border-box clip. Natural/auto, pixel/percentage sizing, contain/cover;
x/y pixel/percentage positions and axis keywords; no-repeat/repeat/repeat-x/
repeat-y. Arch uses 190×40, `background-size:100%`, `no-repeat`, `0 0`.
There is a 4096-tile document budget. Multiple layers, URL escapes,
background-origin/clip overrides, fixed/local attachment, round/space repeat
and shorthand slash sizing are not implemented. Shared SVG restrictions,
response/decoded-pixel/cache limits and the 128 MiB process heap are unchanged.

## Checks

192 Rabbit, 28 Golden Fish, 309 shared image/UI, 31 fetch and 11 Document Viewer
tests pass (571 total). Native wrapper, kernel embedding build, hybrid ISO,
targeted rustfmt and `git diff --check` pass. Commands:

```sh
cargo test -p golden-fish -p rappid-rabbit --target x86_64-unknown-linux-gnu
cargo test -p sun-img -p sunlight-ui --features sunlight-ui/svg --lib --tests --target x86_64-unknown-linux-gnu
cargo test -p sunlight-fetch --lib --target x86_64-unknown-linux-gnu
cargo test -p sunlight-docview --lib --target x86_64-unknown-linux-gnu
bash tools/build_rappid_rabbit.sh
cargo build --package sunlight-kernel
```

The fetch and Document Viewer checks intentionally target their host-supported
libraries. The fetch freestanding binary is not a host test harness. Existing
shared UI doctest issues are outside these library/integration checks.

## Guest evidence

Generated artifacts are under `target/rabbit-visual-evidence/`; they are not
committed. The guest uses QEMU TCG, 2 vCPUs and 3072 MiB RAM. `hashes-final.txt`
identifies the ISO, embedded kernel and native Rabbit. Parser token before/after
probes, serial logs, timed logs, UI driving scripts and build/test logs are kept
beside screenshots.

- `final-startup-render.png`: automatic initial navigation in Render, tools closed.
- `arch-final.png`: visible logo and RSS. The shared canvas logged 1852 written
  logo pixels from the 600×126 SVG raster in a 190×40 CSS box, and 64 RSS pixels.
- `example-final.png`: live 577-byte response, real paragraph, no `s.js>` and no
  heading. `example-resize-and-url-edit.png` additionally shows the correct
  example scene after maximizing to 1254×653 while preserving an address edit.
- `stallman-final.png`: HTTP navigation generation 3 after example.com and a
  failed HTTPS navigation. The 1,523,651-byte 2000×2588 JPEG decoded, attached
  at 320×400 and painted all 128,000 pixels. Heap after publication was
  62,716,448 / 134,217,728 bytes. No stale example scene replaced it.
- `guest-final.serial.log`: final image stages, layout/commit measurements and
  navigation generations. No panic, OOM or fault markers were found. Worker
  threads exited normally. `no-facebook.svg` still fails the shared restricted
  SVG policy. Stallman HTTPS failed with TLS daemon code 50; HTTP retry worked
  without changing fetch limits. An earlier HTTP portrait fetch also failed
  with TCP wait failure; that unsuccessful run is retained separately.

Measurements (guest clocks have 10 ms granularity):

| Stage / page | Observed result |
| --- | --- |
| Baseline Arch, final decode → first scene log | 25.7 s in the first run; 57.0 s in a host-contended repeat; combines styles and scene work |
| Final Arch initial style / scene construction | 22.85 s / 0.78 s, worker |
| Final example.com initial style / scene construction | 30 ms / 30 ms, worker |
| Final Stallman initial style / scene construction | 31.58 s / 25.56 s, worker |
| Baseline acknowledged viewport toggles, input → scene log | 4.55–7.04 s; synchronous UI reflow |
| Final acknowledged viewport toggles, input → commit | 2.36–4.22 s |
| Final Arch viewport worker reflows | 370, 2150, 620 ms |
| Final Arch UI submission / commit | submission below 10 ms resolution; commits below 10, 200, 50 ms |
| Real example.com maximize to 1254×653 | worker 10 ms; submission and commit below 10 ms resolution |

`baseline-resize-timings.txt` and `final-reflow-timings.txt` retain successful
input-to-log measurements. Input holds and scheduling are included; these are
not pure layout benchmarks or a general speedup claim. Guest input automation
sometimes failed to deliver a click; unacknowledged attempts/timeouts are
excluded from the ranges above, with raw records retained. The before/after
runs used live pages and differing host contention, so initial timing intervals
are diagnostic rather than a controlled comparison.

The first worker implementation still copied/diffed unchanged controls and
serialized all image diagnostics on UI commit (210–1180 ms measured). The final
no-op control guard and cached diagnostic summary removed that redundant work.
Host tests separately cover 300 rapid viewport requests, stale versions,
navigation during reflow, late image revisions and cancellation, including final
geometry and media-breakpoint reuse. The three acknowledged guest viewport
changes and the maximize finished with the correct scene geometry.

## Remaining limits

Dense initial style/layout remains slow under TCG. Layout already executing on
its owned snapshot is not interrupted mid-pass; only one job runs, and the next
submission is the latest request. UI painting, changed form-state patches,
scroll clamping and ownership release still run on the UI thread. Initial page
publication still waits for bounded subresource processing. Existing floats,
tables and text-indent limitations remain; Arch's link text can overlap its now
visible logo because its negative text-indent is not implemented. This change
does not claim complete CSS layout or backgrounds support.
