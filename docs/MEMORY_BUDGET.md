# Memory Budget

SunlightOS desktop memory accounting and targets. This document captures the
post-Day-30 state after the kernel heap was raised to 64 MiB and the desktop
grew a full set of services and applications.

## Targets and observed baseline

| Metric                  | Target      | Observed (idle desktop) | Notes                         |
|-------------------------|-------------|-------------------------|-------------------------------|
| Total physical RAM      | 512 MiB     | ~300 MiB used           | PMM `total_frames - free_frames` |
| Kernel heap capacity    | 64 MiB      | grows, bounded by cap   | `HEAP_SIZE` in `kernel/src/memory/heap.rs` |
| Kernel heap high-water  | —           | see `[MEM]` boot logs   | sampled once per telemetry tick |
| Telemetry page          | 8 KiB       | 8 KiB                   | 2 pages, read-only to userspace |

The 64 MiB heap is currently required: the supervisor, IPC paths, framebuffer
compositing, and per-process page-table walks all allocate from it during boot
and steady state. It is **not** to be reduced without measuring a realistic
desktop session.

## Where memory lives (known heavy components)

- **Kernel heap (64 MiB)** — supervisor bookkeeping, IPC message buffers, VFS
  caches, page-table structures, framebuffer/compositor state. Tracked by the
  heap telemetry fields (`heap_total_kb`, `heap_used_kb`, `heap_high_water_kb`).
- **Per-process user memory** — counted via PML4 walks (`count_user_pages`) and
  exposed as `mem_kb` per process in the telemetry page / Tasks Monitor.
- **zram swap** — `zram_orig_kb` / `zram_comp_kb` in telemetry; compressed.
- **Static buffers (no heap)** — image/MIME icon resolution and several preview
  paths use compile-time-sized static buffers, so they cannot leak.

## Service memory budget table

These are **soft budgets** — documentation and telemetry labels only. No hard
enforcement mechanism exists yet. Values are initial guidance for review, not
measured caps. They exist so that a future regressions (a service doubling its
footprint) is obvious during review.

| Service / app        | Soft budget | What it holds                                            |
|----------------------|-------------|----------------------------------------------------------|
| Display server       | ~16 MiB     | framebuffer, surface list, compositor state              |
| Shell (vortex)       | ~24 MiB     | window manager, taskbar, app launch state, wallpaper     |
| sunlight-files       | ~12 MiB     | file listing, 4 MiB preview src buf, 8 KiB text preview  |
| sunlight-kv          | ~4 MiB      | in-memory key/value store                                |
| sunlight-clipd       | ~1 MiB      | clipboard history (32 entries × ≤2 KiB text)             |
| sunlight-clipman     | ~2 MiB      | clipboard history UI (≤32 rows)                          |
| dialog host          | ~4 MiB      | file picker / dialog transient state                     |
| Light Lens           | ~12 MiB     | 8 MiB image decode buffer + viewer state                 |

## Cache bounds (verified Day 30)

All caches are already bounded — no unbounded growth paths were found:

- **Icon cache** (`sunlight-ui/.../mime_icon.rs`): no heap cache; resolution is
  bounded by construction.
- **Preview buffers** (`sunlight-files`): `PREVIEW_SRC_BUF` 4 MiB, `TEXT_PREVIEW_BUF`
  8 KiB — fixed static buffers.
- **Light Lens** (`sunlight-light-lens`): `IMAGE_BUF` 8 MiB fixed static buffer.
- **Thumbnail cache** (`sunlight-thumbd`): on-disk with `CACHE_MAX_BYTES` 64 MiB
  and `cleanup_cache` eviction.
- **Clipboard history** (`sunlight-clipd`): `HISTORY_LIMIT` 32 entries,
  `MAX_TEXT_BYTES` 2048 per entry, `trim_history` enforces both.
- **Clipboard manager** (`sunlight-clipman`): `MAX_ROWS` 32.

## Telemetry exposure

Heap stats are plumbed end to end:

1. `kernel/src/memory/heap.rs` — `heap_total()`, `heap_used()`, `heap_high_water()`
   (sampled `AtomicUsize`, refreshed once per telemetry tick), `boot_mem_log()`.
2. `kernel/src/telemetry.rs` — `TelemetryPage` v3 adds `heap_total_kb`,
   `heap_used_kb`, `heap_high_water_kb`; populated each snapshot.
3. `sunlight-telemetry/src/lib.rs` — userspace mirror + `snapshot_heap_kb()`
   one-shot reader for boot logs.
4. `sunlight-top` — header shows `HEAP: used/total hw <high-water>`.
5. `services/sunlight-tasks` — System info panel shows `HEAP used / total  hw <hw>`.

## Boot memory logs

Compact `[MEM] <phase> heap <used>/<total> KiB [free <f>] [hw <hw>]` lines are
emitted on serial at:

- `kernel-init` — after Foundation Complete (kernel).
- `pre-scheduler` — just before dropping to Ring 3 (kernel).
- `services-ready` — when sunlightd drains the autostart queue (includes
  live service count).
- `shell-ready` — when the vortex shell enters its event loop.

## Future optimization candidates

These are **not** to be acted on prematurely (per Day 30 constraints). They are
recorded so the next budget review has a starting list:

- Per-process page-table walks on every telemetry tick are the dominant
  steady-state kernel cost — consider caching / dirty-tracking `mem_pages`.
- Framebuffer/compositor allocations dominate the display server budget; a
  shared-dma or lazy-map scheme could cut this.
- The 64 MiB heap high-water should be monitored over a long session; if it
  plateaus well below 64 MiB, the cap could eventually be lowered.
- sunlight-files preview buffer (4 MiB) is held per instance; multiple open
  windows multiply it.
