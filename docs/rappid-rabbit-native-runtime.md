# Rappid Rabbit native std runtime

Rabbit now uses the same pinned SunlightOS Rust std overlay as Document Viewer.
It has a normal Rust main function, native thread/TLS startup and the reclaiming,
locked libc/System allocator with a 128 MiB process heap budget. The kernel keeps
its freestanding target; Rabbit no longer embeds a 32 MiB static heap.

The UI thread owns the window and widgets. A Tokio current-thread runtime owns
one blocking worker, with a 4 MiB stack, for DNS, TLS, response collection,
HTML parsing, stylesheet loading and initial scene construction. A second
single-worker runtime prepares images independently, with one shared lane guard
across navigation generations. Initial scene publication precedes image work.
A one-shot channel moves only owned document state back to the UI. UI ticks poll the channel without waiting and redraw when state changes.
A navigation already in progress rejects additional fetch clicks, bounding the
number of live pipelines and buffers. Completion preserves URL edits and the
current developer-panel configuration, checks the document generation, and
requests a worker reflow if the window changed size during loading. Startup
selects Render with DevTools closed. Explicit navigation selects Render; an
asynchronous completion never overrides Source/DevTools choices made during
loading, on success or failure.

Viewport events only replace a latest viewport request and increment its
version. UI ticks submit at most one reflow to the same bounded blocking pool.
The snapshot shares immutable Arc DOM, computed styles, retained scene and
raster pixels; it does not contain widgets or borrow UI objects. Layout,
geometry construction and scene diffing run on the worker. Width-dependent
media styles are recomputed there when necessary. A result commits on the UI
thread only when document generation, viewport version and image revision all
match. Superseded results are dropped and the latest request is submitted next.
The displayed scene remains usable until replacement; no queue of resize jobs
accumulates. Navigation drops the reflow receiver, and queued work checks it
before starting. An already executing layout is not interrupted mid-pass.

The UI still applies changed form-state patches, clamps visible scroll positions
and paints the scene. Unchanged controls preserve the shared scene without a
copy/diff; reflow commits use cached diagnostic counts rather than serializing
all image objects again. Hidden Source text is not scanned to clamp its scroll. Reflow reapplies current form state after acceptance so edits
made while the worker ran survive. Inspector DOM/styles share the same owned
snapshots, and accepted media styles invalidate its text caches. Image completion
revision checks prevent an old reflow from replacing newly attached pixels.
Image resources now complete independently of presentation, and viewport-aware
paint admission progressively exposes cached pixels. The original layout worker
still serializes all DOM/layout operations. Image response/decode/cache limits
remain bounded. See [progressive presentation](rappid-rabbit-progressive-presentation.md)
for resource readiness, cache metadata, cancellation and measurements.

The previous source, DOM and caches are released when navigation begins. Worker
scratch spaces and fetch responses are dropped when pipeline processing ends;
the source and final document/cache remain owned by the UI until the next
navigation. Existing body, CSS, image and parser limits remain in force. The
128 MiB heap is still bounded; arbitrary pages are not guaranteed to fit.

Tokio enables `rt`, `sync` and `time`, with its socket reactor disabled. The
existing capability IPC transport supplies networking; native std's unsupported
`std::net` implementation is not used. Native thread spawn/join, TLS, futex
parking and clocks supply Tokio's requirements without epoll/eventfd stubs.
The runtimes schedule blocking jobs directly; they do not drive an
async event loop alongside `Window::run`. Future timer/async futures must be
driven with `block_on` or a separate executor thread.

On close, the UI drops its receivers and shuts both runtimes down without waiting
for blocking network timeouts. An already running blocking job cannot be
cancelled by Tokio; normal native std process group exit terminates the workers.
If a worker drops its sender, the UI reports a navigation error. Native aborting
panics and OOM still terminate the process.

## Build and validation

The workspace defaults to `x86_64-unknown-none` for the kernel. That target has
no std. Use the wrapper to compile Rabbit for `x86_64-sunlightos` with build-std:

```sh
bash tools/build_rappid_rabbit.sh
# Equivalent platform build:
bash tools/build_native_std.sh build -p rappid-rabbit --release
cargo test -p rappid-rabbit --target x86_64-unknown-linux-gnu
cargo build --package sunlight-kernel
rustfmt --edition 2021 --config skip_children=true --check \
  rappid-rabbit/src/main.rs rappid-rabbit/src/lib.rs kernel/build.rs
```

The ordinary desktop build/run/write/boot-test scripts invoke the Rabbit wrapper.
The kernel build script builds it in its separate embedded-build target directory
and embeds `target/x86_64-sunlightos/release/rappid-rabbit`.

Host tests exercise a document with more than 900 DOM nodes on a real Tokio
blocking worker, nonblocking UI polling, resize during loading, URL-edit and
panel preservation, stale generations and a closed worker channel. These tests
use a synthetic response; they do not establish guest DNS/TLS behavior or a live
Arch/AUR browsing session.
