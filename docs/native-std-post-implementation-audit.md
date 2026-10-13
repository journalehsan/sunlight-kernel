# Native std / Hayro Post-Implementation Audit

Audit date: 2026-10-13.

Scope: the native Rust `std` platform (`tools/native-std/`, target
`tools/targets/x86_64-sunlightos.json`, syscall range 155-158, per-task
`FxState`), and the Hayro 0.8.0 PDF reader integration (`sunlight-docserv`
`hayro-native` backend, `sunlight-docview`, document-service supervisor unit).

Primary references: `docs/native-std-foundation.md`, the live source tree, and
fields regenerated during this audit. `docs/hayro-std-gap-analysis.md` is
treated as historical and was not used as a source of truth.

## Evidence produced during this audit

The prebuilt ISOs and serial logs checked into `target/` predated the committed
source (they still showed an 8 MiB kernel heap). All runtime claims below were
re-verified on freshly rebuilt images from the current worktree:

- Host/native build and std graph check: `bash tools/check_native_std.sh`
  (Hayro 0.8.0 graph + pinned overlay) passes.
- FP/SIMD isolation gate, rebuilt foundation ISO, 2 CPUs and 4 CPUs:
  `target/audit-fp-2cpu.log`, `target/audit-fp-4cpu.log` - both report
  `[NATIVE-STD] x87/XMM/MXCSR preemption isolation OK` and
  `[NATIVE-STD] native PDF parser and vector raster OK`, with
  `[HEAP] Initializing 16 MiB kernel heap`.
- Native Hayro smoke gate (open/render/close, TLS/threads/locks, OOM/panic/group
  exit), rebuilt smoke ISO, 2 CPUs and 4 CPUs:
  `target/audit-docserv-smoke-2cpu.log`, `target/audit-docserv-smoke-4cpu.log` -
  both pass `[DOCSERV] native Hayro IPC open/render/close OK`.

Host-only and historical evidence (not regenerated here) is cited explicitly as
such.

## 1. FP/SIMD correctness

Implemented and re-verified:

- Each task owns a 512-byte, 16-byte-aligned `FxState`
  (`kernel/src/arch/x86_64/cpu.rs`). `fxsave64`/`fxrstor64` bracket scheduling.
- Save happens in `Scheduler::schedule_tick` (timer/IRQ preemption path). Restore
  happens on every dispatch path: idle-AP fast path, normal dispatch, first
  process entry, and post-exec entry. Reaped slots reset to architectural
  initial state (x87 cw `0x037f`, MXCSR `0x1f80`, zeroed XMM).
- New native threads capture the creator's live FP state at spawn
  (`thread.fx_state.save()` in `thread_spawn`), so native threads inherit live
  x87/XMM state; Linux `clone` does the same.
- `OSXSAVE` is explicitly cleared in `init_cpu_features`; the kernel is built
  soft-float (`x86_64-unknown-none`) and never executes FP/SIMD.
- The FP probe (`tools/native-std-probe/src/fp.S`) runs eight borrower threads
  plus the owner, each holding distinct x87 cw, MXCSR, XMM0 and XMM15 values
  across 64 busy intervals interleaved with native `yield` syscalls, and
  verifies aligned `movaps` stores.

Fresh result: passes with 2 CPUs and 4 CPUs (logs above).

Sufficiency boundary:

- AVX/YMM state is not preserved and AVX is disabled in both the userspace
  target (`-avx,-avx2`) and the kernel. Any future AVX build would silently
  corrupt state.
- The probe exercises timer preemption and voluntary yield. It does **not**
  force task migration between cores, so cross-core migration of a task with
  live FP state is inferred, not directly measured. It is a preemption/isolation
  test, not an affinity-pinned migration test.
- Non-scheduling interrupts are safe by construction (the kernel uses no
  FP/SIMD between save and restore), but there is no explicit test that a timer
  IRQ returning to the *same* task leaves FXSAVE state untouched.

## 2. Rust std coverage

Userspace target and sysroot (confirmed):

- `tools/targets/x86_64-sunlightos.json`: `os = "sunlightos"`, System V
  hard-float ABI, `+sse2 -avx -avx2`, `disable-redzone`, `relocation-model:
  static`, `panic-strategy: abort`, no precompiled std (`std: false`).
- `tools/native-std/prepare.py` pins compiler revision `8954863c8`. The installed
  toolchain reports `8954863c8` (2026-06-05), matching. It stages a **separate**
  sysroot at `target/native-std/sysroot` and a rustc wrapper at
  `target/native-std/rustc`; the rustup sysroot is never modified.
- `tools/build_native_std.sh` runs `-Z build-std=std,panic_abort` with
  `-Z build-std-features=` (empty), i.e. no `std_detect`/`backtrace`/`unwind`;
  the overlay supplies PAL, System alloc, args/env, clocks, entropy, stdio,
  files/paths, TLS, threads and sync selection. `prepare.py` rewrites the
  `cfg_select!` for `mutex`, `rwlock`, `condvar`, `once`, `thread_parking` to
  select the upstream futex implementations for `target_os = "sunlightos"`.

Implemented and exercised on the guest:

- Real `std` collections (`HashMap`), `io::Cursor`/`Read`/`Seek`, `Instant`,
  `SystemTime`, `thread::sleep`, `Mutex`, `OnceLock`, `Condvar` (incl. timeout),
  `RwLock`, `thread::spawn`/`join`, thread-local keys with destructors,
  `env::var`/`set_var`/`remove_var`, `current_dir` (absolute path), absolute-path
  replacement in `PathBuf::join`, `fs::File` open/read/seek and shared file
  offsets across threads, and panic/abort exit semantics.

Partially implemented:

- Filesystem: `open`/`read`/`write`/`seek`/`stat`/`close` only. `fsync`,
  `datasync`, file locks, `truncate`, `duplicate`, `set_permissions`,
  `set_times`, and all timestamp reads return `UnsupportedPlatform`.
- `paths::temp_dir`/`home_dir` derive from the std-owned env snapshot;
  `current_exe` is unsupported.
- `thread::available_parallelism` always errors (`UNKNOWN_THREAD_COUNT`);
  `current_os_id`/`set_name` are inert.

Unsupported (by design, documented):

- Networking (`std::net` uses the upstream `unsupported` shim; `TcpStream`,
  `UdpSocket`, DNS).
- `std::process` Command/fork/exec, directory iteration (`readdir`), links,
  `rename`, `mkdir`, `remove_dir_all`, `canonicalize`, `copy`.
- Compiled TLS/AVX codegen, panic unwinding, `catch_unwind` recovery.

## 3. Threading and TLS

- Spawn: `std::thread` maps to native syscall 158 (`sl_std_spawn`), which opts
  into shared descriptors and cwd. Legacy syscall 22 keeps its prior
  descriptor-private behavior.
- Join: `sl_std_join` polls syscall 156 (`NativeThreadReaped`) until the target
  is `Reaped` (off every CPU), then unmaps its stack and TLS page and frees the
  handle.
- Detach: bounded table of 256 handles in `sunlight-libc/src/std_runtime.rs`;
  swept on the next spawn/join/detach; overflowing the table exits 101. Final
  VM teardown releases outstanding mappings.
- TLS: key-based, 255 usable process-wide keys in the TCB page immediately
  after `errno` (`fs:0` self-pointer, `fs:8` errno, table at `base + 32`).
  Destructors run via `run_dtors` (5 passes) at thread completion and at main
  cleanup.
- Verified in the guest: `[THREAD-LIFECYCLE] created=N finished=N reaped=N`,
  `borrower_slots_reused`, `max_live=4`, `stale_wake_rejected=12`, and exactly
  32 thread-local destructor drops for 32 `DropWitness` instances across 8
  rounds of 4 threads. Thread creation/join churn and slot reuse are stable to
  `created=41`.

Gaps:

- TLS keys are never reclaimed except explicit `destroy`; 256 keys is a hard
  ceiling that panics on exhaustion (fine for current std usage, not for
  libraries that leak keys).
- Detached threads keep their mappings until the next spawn/join/detach sweep or
  VM teardown - "immediate detached mapping recovery" is explicitly out of
  scope.
- `set_name`/`current_os_id` are no-ops, so thread identity introspection is
  unavailable.

## 4. Synchronization

- Primitive: native futex syscall 155. Compare and enqueue are serialized under
  the scheduler lock; wake scans blocked tasks by shared address-space identity
  plus aligned faulting word; timeouts wake via the BSP timekeeper; `munmap`
  cancels matching registrations (`cancel_native_futex_range`).
- Algorithms above the primitive are upstream Rust `futex` mutex/rwlock/condvar/
  once/thread-parking (not reimplemented).
- Verified: Mutex counter to 2048 across threads; `OnceLock` init; `Condvar`
  `wait_while` wake and `wait_timeout` with confirmed `timed_out()`; `RwLock`
  with four writer threads (`shared == 128`); `thread::sleep` non-spurious
  (`elapsed >= 20 ms`). Passes on 2 and 4 CPUs.

Concerns:

- Timeout expiry runs on the timekeeper core tick (~10 ms quantum), so a
  sub-tick `park_timeout`/condvar timeout can fire late. Correctness is
  preserved (no early wake), latency is not tight.
- A task can be registered on only one futex address at a time (single
  `native_futex_wait`/`native_futex_deadline` slot), which is sufficient for the
  std parking/lock algorithms but is a constraint for any future multi-wait
  design.
- Wakeup-race behavior is exercised indirectly by the lock/condvar stress, not
  by a dedicated adversarial compare-vs-enqueue race test.

## 5. Memory management

- Native renderer and viewer each opt into a bounded 128 MiB heap
  (`dynamic-heap-128m`) via `sunlight-libc`; the kernel heap was raised from
  8 MiB to 16 MiB to hold service state, anonymous-page tracking, the
  replacement tracking buffer and mmap rollback journals while both app heaps
  and workers are live.
- Host cycle measurement (`target/hayro-metrics.log`, alloc-tracking probe):
  repeated open/parse/render of the pinned PDF returns to a flat
  `live = 1839` bytes after close for 21 consecutive cycles; incremental peak
  ~10.36 MB; `max_rss_kib = 19604`.
- Guest repeated-lifecycle check in the smoke service: five six-page
  open/render/close cycles assert `requested_user_bytes` does not grow after
  warmup, then the four-page fixture renders at 595x841 with per-page and
  color-channel pixel checks.
- Guest accounting on reap: the render smoke process reports
  `user_frames=34551 page_tables=76` (~135 MiB mapped) and `owned_frames=0`
  after reap. OOM/panic/exit probe processes report `user_frames=0` on reap,
  showing no page leak after failure.

Instrumentation limits:

- The kernel reports **owned page frames** and page-table pages per process
  (`PMM-DIAG owned_frames`, `SCHED ... user_frames`). There is no native
  resident/shared/private (RSS/PSS) instrument in the guest, so mapped vs
  resident vs shared cannot be separated there. The only fine-grained number is
  the userspace allocator's requested/backing bytes.
- The 128 MiB per-app heap is a hard bound: a very large or complex PDF can
  exhaust it even below the protocol's 16-megapixel limit, producing a clean
  OOM exit (101) rather than a partial render.

## 6. PDF rendering

Verified:

- Four-page bundled fixture renders all four pages at 1.0x as 595x841 with
  >1000 non-white pixels per page and page-2 color thresholds (red >500,
  orange >5000, blue >15000).
- Six-page `sample.pdf` renders each page at 0.5x with non-white pixels and no
  bitmap-size mismatch, across five open/render/close lifetimes.
- Native viewer gate: window ready, `[DOCVIEW] native window navigation/zoom OK`,
  Files native PDF thumbnail worker OK, and `[DOCVIEW] native close OK`.
- Helios viewer against the native service: six-page forward/back navigation,
  zoom, resize, cache recovery (cache_pages/cache_bytes tracked), Files
  thumbnail and clean close.
- Malformed input at the engine boundary: the probe rejects `b"not a PDF"`
  (`InvalidPdf`); the service bounds reads to 32 MiB and rejects empty
  documents; host integration tests reject NaN/Inf/0/negative scale, out-of-range
  pages, foreign handles, and double-close without killing the service.

Gaps:

- No guest test drives a *malformed/corrupt* PDF through the IPC open path;
  malformed-input coverage today is host-side and engine-side only.
- No guest test of a page that deliberately exceeds the pixel/dimension budget
  through IPC (only engine rejection paths are host-tested).
- Text extraction is unsupported on the Hayro backend
  (`Hayro text extraction is not implemented`), and PS/EPS/XPS/CBZ/EPUB are out
  of scope for this backend.

## 7. Failure handling

- Allocation failure: host-side and guest-side. The service's `--native-oom-test`
  allocates 129 MiB, confirms the pointer is null and
  `failed_allocation_count > 0`, then `handle_alloc_error` exits 101.
- Worker panic: `--native-panic-test` panics a worker; the *whole process* aborts
  with code 101 and the borrower thread is reaped (`native-exit-group`).
- Group exit: `std::process::exit(43)` terminates every task sharing the VM,
  code 43.
- Resource cleanup: reaped processes report `owned_frames=0`, and the document
  handle registry is empty after close; `reap_closed_clients` drops documents of
  dead clients.
- Isolation: the renderer runs as its own process with the supervisor unit
  (`Restart=on-failure`, `RestartSec=1`); a renderer abort cannot take down the
  desktop or other services, but it *does* drop every open document for every
  client, and clients receive a transport failure and must reopen handles.

Important correction to the wording in `docs/native-std-foundation.md`:

- "Worker panic containment" is **process-level**, not thread-level. The native
  target uses `panic = abort`, so `catch_unwind` in
  `sunlight-docserv/src/backends/hayro_backend.rs` cannot recover an aborting
  Hayro panic. The service's panic path terminates the entire process (verified:
  `code=101`, borrower terminated). On the Linux GNU host build `catch_unwind`
  does work; this difference must be stated for native.
- The original FP foundation gate and the ordinary Phase 3.0 boot regression
  were not regenerated in this audit; only the rebuilt FP/foundation and
  document smoke gates were. The 12-CPU UEFI desktop launch and Open Document
  dialog claim rests on historical evidence
  (`target/native-docview-launch-final.log`).

## Regressions

- None observed in the freshly rebuilt gates. All regenerated runs show the
  16 MiB kernel heap and pass cleanly.
- Stale artifacts are a real hazard: `target/*.iso` and most `target/*.log`
  document runs predate commit `f6975c42f` and reflect an 8 MiB kernel heap.
  Treat them as historical until rebuilt. The audit regenerated only the FP and
  smoke gates.

## Security concerns

- Syscalls 155-158 do not widen authority beyond the caller's VM: futex wake is
  keyed on shared address-space identity and an aligned in-VM address; thread
  reaped reports only for `native_thread` tasks in the same VM; exit-group
  terminates only tasks sharing the caller's VM identity; native spawn validates
  trampoline/stack/func against the user address range.
- Entropy fails closed: `sl_std_entropy` returns `-1` when the conditioned
  approved-source entropy is not ready; there is no fallback RNG. Hash seeds and
  `random` therefore fail rather than degrade.
- The native service chatty path watches its parent so a dead supervisor cannot
  leave it running.
- Residual risk: because native panic = abort, a single crafted PDF that induces
  a Hayro internal panic will take down the renderer for all clients rather than
  failing one document. This is the most significant robustness gap for a
  production reader.

## Prioritized recommendations

1. Add guest-side malformed/adversarial PDF tests through the IPC open/render
   path, and a test that a corrupted page cannot abort the process (or, if
   Hayro can panic, mitigate with a per-document renderer process or upstream
   panic-freeing).
2. Add an explicit cross-core FP migration test (pin, run, migrate, verify) and,
   if AVX is ever enabled, implement YMM save/restore and set `OSXSAVE`.
3. Add guest resident/shared/private memory instrumentation (or a documented
   proxy) so PDF open/render/close can report real RSS/PSS, not just owned
   frames.
4. Reduce futex timeout latency (program a timer at the earliest deadline
   instead of relying on the 10 ms tick) if lock/condvar latency becomes
   material.
5. Regenerate and check in the 12-CPU UEFI launch/Open-Document-dialog gate
   against the committed source, and re-run the original FP foundation and
   Phase 3.0 boot regression.
6. Correct the "worker panic containment" wording to state process-level
   isolation, and document the `catch_unwind`/`panic = abort` difference for
   native.

## Readiness assessment

Not production-ready as a general-purpose PDF reader, but a credible,
well-instrumented preview.

- Solid: FP/SIMD isolation on 2/4 CPUs, core `std` thread/TLS/sync semantics
  with destructors and reclamation, bounded-heap leak-free repeated
  open/render/close, capability-scoped IPC, and process-isolated failure
  reporting (OOM/panic/group exit observed).
- Blocking for production: a renderer panic aborts the whole service for all
  clients; malformed/corrupt-PDF handling is not exercised through the guest IPC
  path; there is no guest resident-memory instrumentation; PS/EPS/XPS/CBZ/EPUB
  and text extraction are unsupported; AVX/cross-core-migration FP behavior is
  untested (and AVX is intentionally disabled).
- Recommended positioning: production-preview for well-formed PDFs of bounded
  complexity on the pinned toolchain, with an explicit statement that the
  renderer service is a single point of failure per abort and that memory is
  hard-capped at 128 MiB per app.

---

# Addendum: PDF Reader Zoom & Navigation Toolbar

Audit date: 2026-10-13. Scope: the new toolbar zoom dropdown, status-bar zoom
slider, and Previous/Next page buttons in the native Hayro reader
(`sunlight-docview/src/{zoom.rs,ui.rs,viewer.rs}`,
`sunlight-docserv/src/{ipc.rs,service.rs}`,
`sunlight-ui/widgets/{section.rs,document_canvas.rs}`). Hayro remains the only
rendering backend and the Document Canvas architecture is unchanged.

## Evidence produced during this audit

All runtime claims below were re-verified on images rebuilt from the current
worktree:

- Native std graph check: `bash tools/check_native_std.sh` passes.
- Native UI smoke gate, 2 CPUs and 4 CPUs: `target/audit-zoom-nav-2cpu.log`,
  `target/audit-zoom-nav-4cpu.log` - both report
  `[DOCVIEW] native window navigation/zoom OK` and `[DOCVIEW] native close OK`,
  with the Files thumbnail worker OK and zero `[FAULT]`/`[EXCEPTION]` lines.
- Native service smoke gate (OOM/panic/group exit, heap stability), 2 CPUs:
  `target/audit-zoom-smoke-2cpu.log` - passes
  `[NATIVE-STD] shared descriptors/OOM/group exit OK`,
  `[NATIVE-STD] worker panic containment OK`, and
  `[DOCSERV] native Hayro IPC open/render/close OK`.
- Host suites: `cargo test -p sunlight-docview -p sunlight-docserv -p sunlight-ui
  --target x86_64-unknown-linux-gnu --features
  sunlight-docserv/hayro-host,sunlight-docview/std` - 11 viewer + 9 UI + 4/6
  docserv + 283 sunlight-ui tests pass.

## Implemented and verified capabilities

Zoom model (`zoom.rs`, pure and unit-tested):

- `ZoomMode::{FitPage, FitWidth, Fixed}` resolves to a pixels-per-point scale
  from the page's point dimensions and the live viewport. `FitPage` takes the
  min of the width/height ratios; `FitWidth` uses the width ratio.
- `sanitize_scale` rejects non-finite/non-positive scales (falls back to 1.0),
  clamps to `[0.1, 4.0]`, and additionally reduces the scale so the bitmap
  cannot exceed 16 MP (`MAX_PAGE_PIXELS`, matching the service) or `u16::MAX`
  per dimension. Overflow of image dimensions or allocation sizes is therefore
  structurally impossible before the RPC is sent.
- `ZoomPreset::ALL` provides Fit Page, Fit Width, Actual Size, and
  70/85/100/125/150/175/200% (10 entries); `step_percent` is a 25-point step
  clamped to `[70, 200]`.

Toolbar and status bar (`ui.rs`):

- Zoom dropdown (right of the toolbar) lists all presets, marks the active one,
  and selecting one re-renders at that mode. Verified live: dropdown opens and
  125% is applied.
- Status-bar zoom slider (70-200%, default = the resolved fit percentage) plus
  optional `-`/`+` buttons; the slider, dropdown label, and percentage readout
  stay synchronized through `refresh_slider`. Manual changes switch the active
  mode to `Fixed` (Custom). Verified live: slider clicks select 70% and 200%,
  and the step buttons land on the expected 25-point boundaries.
- Previous/Next page buttons with a `Page n / total` counter, disabled at the
  document boundaries; keyboard Left/Right (`0x4b/0x49`, `0x4d/0x51`) and
  Page-Up/Down still work. Verified live: Next walks 2..=6 and is inert on page
  6; Previous is inert on page 1.
- Window resize recomputes fit zoom. Verified live in Fit Width: resizing
  800x700 -> 1000x800 raised the fit zoom 131% -> 163% and back.
- Fit zoom uses cached page points for an immediate preview and a debounced
  (`150 ms`) worker render for the exact scale.

Rendering, cache, and IPC:

- New `DocRequest::PageInfo` / `DocResponse::PageDimensions` RPC supplies point
  dimensions; the viewer queries them once per page and caches them.
- The render worker receives the live viewport in each `Intent`, so fit modes
  resolve identically on the worker and the UI.
- Rapid zoom is coalesced: slider drags only schedule a settle timer and issue
  no `RenderPage` RPC until the debounce window closes (host test asserts zero
  intermediate renders across 8 drag positions). Visible feedback during the
  window is a preview rebase (clamped 0.1-10x) of the last rendered bitmap,
  so the page tracks zoom without a blank frame.
- The page cache is bounded to `MAX_CACHED_PAGES = 5` and
  `CACHE_BUDGET_BYTES = 40 MiB`; `prune_cache` evicts the farthest page first.
  Navigation reuses cached bitmaps and does not re-render unchanged pages
  (viewer unit test asserts a stable `RenderPage` count).

## Implemented but insufficiently tested

- Fit zoom during resize is asserted in the window/guest gate and in the viewer
  unit tests, but not against a *maximized/tiled* desktop window driven by the
  compositor; the resize path here is the app-level `WindowEvent::Resized`.
- Conditional overflow protection is exercised by `zoom.rs` unit tests
  (5-5000 pt pages, NaN/Inf/negative scales) but not by an end-to-end guest
  render of a pathological page through IPC.
- The slider step buttons and dropdown were exercised in the guest smoke and
  host UI tests, but a full drag-and-settle through the *native* event loop is
  not separately timed in the guest (the guest clicks; the host test drags).
- Preview rebasing is verified only through the widget pixel test; there is no
  frame-time measurement of the preview path under a slow worker.

## Known unsupported / limitations

- Text extraction is unsupported on the Hayro backend; PS/EPS/XPS/CBZ/EPUB
  remain out of scope. No new engine was added.
- The slider range is fixed at 70-200%; "Actual Size" (100%) and scales above
  200% are only reachable via the dropdown, and Fit percentages outside 70-200%
  are clamped for *display* in the slider (the resolved render scale is not
  clamped by the slider range).
- The status-bar slider is a raw value track with 70-200 integer steps; it does
  not snap to the named presets, so dragging produces arbitrary percentages
  (mode becomes Custom, by design).
- Cache accounting reports bytes retained by the viewed-process bitmap cache,
  not process RSS/PSS; there is no guest resident/shared/private instrument
  (unchanged from section 5).

## Test results and performance measurements

- Host memory stability (`repeated_open_render_zoom_close_is_memory_stable`,
  six open/render/navigate/zoom/close cycles of the six-page fixture): resident
  set stayed flat (~8964 -> 10096 KiB across cycles, no monotonic growth) and
  the service handle count returned to 0. Per the constraint that successful
  compilation is not proof, this is a measured RSS-stability bound, not a leak
  proof.
- Guest render cache observed during the UI gate on a 6-page document: at 82%
  fit, `cache_pages` 3-5 and `cache_bytes` 3.9-6.6 MB; at 200% the same page
  cache reached 20.7 MB, still under the 40 MiB budget
  (`target/audit-zoom-nav-2cpu.log`). Cache pages never exceeded 5.
- Both 2- and 4-CPU UI gates completed all 31 smoke steps (keyboard nav, step
  buttons, Next/Prev with boundary checks, dropdown 125%, slider 70%/200%,
  custom-zoom persistence across navigation, Fit Width, resize recompute) and
  reported clean close; no faults.

## Regressions and security concerns

- No regressions observed in the rebuilt gates or host suites.
- The new `PageInfo` RPC is scoped to the caller's authenticated handle and VM
  in the same way as the existing `PageCount`; it widens no authority.
- The condition "zoom calculations cannot overflow image dimensions or
  allocation sizes" is enforced in three independent places (UI `desired_scale`
  via `sanitize_scale`, viewer `set_view` via `sanitize_scale`, and the backend
  `render_page` budget check), so a bad scale cannot reach the allocator.
- Residual risk unchanged from the service audit: a Hayro panic aborts the whole
  renderer (process-level), dropping all clients' handles.

## Prioritized recommendations

1. Add a guest IPC test that renders a page engineered to exceed the pixel /
   dimension budget and asserts a clean `DocResponse::Err` (no abort).
2. Measure preview-path frame time under a deliberately slow worker to confirm
   the rebase keeps panning/zooming responsive.
3. Add resident/shared/private memory instrumentation in the guest so PDF
   open/render/close reports real RSS/PSS, not just cache bytes.
4. Consider snapping slider drags to the named presets (with a Custom region) to
   reduce arbitrary percentages, if UX testing shows value.

## Readiness assessment (native PDF Reader)

Production-preview for well-formed PDFs of bounded complexity. The zoom model,
cache budgets, and debounce path are correct and independently verified on hosts
and in guest smoke on 2/4 CPUs; navigation buttons and boundaries are verified
live; memory is stable across repeated cycles. It is not production-ready as a
general-purpose reader because renderer panics remain process-level, corrupt-PDF
handling is still not exercised through the guest IPC path, and there is no
guest resident-memory instrumentation.
