# Rappid Rabbit native std runtime

Rabbit now uses the same pinned SunlightOS Rust std overlay as Document Viewer.
It has a normal Rust main function, native thread/TLS startup and the reclaiming,
locked libc/System allocator with a 128 MiB process heap budget. The kernel keeps
its freestanding target; Rabbit no longer embeds a 32 MiB static heap.

The UI thread owns the window and widgets. A Tokio current-thread runtime owns
one blocking worker, with a 4 MiB stack. A navigation runs DNS, TLS, response
collection, HTML parsing, stylesheet/image loading and initial scene construction
on that worker. A one-shot channel moves only owned document state back to the
UI. UI ticks poll the channel without waiting and redraw when state changes.
A navigation already in progress rejects additional fetch clicks, bounding the
number of live pipelines and buffers. Completion preserves URL edits and the
current developer-panel configuration, checks the document generation, and
reflows if the window changed size during loading.

The previous source, DOM and caches are released when navigation begins. Worker
scratch spaces and fetch responses are dropped when pipeline processing ends;
the source and final document/cache remain owned by the UI until the next
navigation. Existing body, CSS, image and parser limits remain in force. The
128 MiB heap is still bounded; arbitrary pages are not guaranteed to fit.

Tokio enables `rt`, `sync` and `time`, with its socket reactor disabled. The
existing capability IPC transport supplies networking; native std's unsupported
`std::net` implementation is not used. Native thread spawn/join, TLS, futex
parking and clocks supply Tokio's requirements without epoll/eventfd stubs.
The runtime currently schedules blocking jobs directly; it does not drive an
async event loop alongside `Window::run`. Future timer/async futures must be
driven with `block_on` or a separate executor thread.

On close, the UI drops its receiver and shuts the runtime down without waiting
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
