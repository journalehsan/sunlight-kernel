# ZRAM Swap — Complete Implementation Plan (Phase 6.6)

Status: **PLANNED** · Follow-up to Phase 6.5 Userland · Target: real demand-paged
swap backed by LZ4-compressed ZRAM, surfaced in `free -h` and `sysfetch`.

---

## 0. Where We Are Today (baseline)

The current ZRAM is a **storage backend stub**, not a swap subsystem:

| Component | State | File |
|-----------|-------|------|
| ZRAM block store | Working (Vec-based, 256 MiB, 4 KiB blocks) | `kernel/src/memory/zram.rs` |
| `compress_page()` | **No-op copy** — LZ4 not wired despite docs | `zram.rs:63` |
| `stats() -> (total, used, used_bytes)` | Working | `zram.rs:141` |
| sysinfo swap fields (`swap_total_kb`, `swap_used_kb`) | Working, sourced from `zram::stats()` | `kernel/src/arch/x86_64/syscall.rs:1801` |
| `free` applet `Swap:` line | Working | `sunlight-utils/src/main.rs:549` |
| Boot smoke block (`init_swap_smoke`) | Writes 1 block so accounting is visible | `kernel/src/main.rs:1011` |
| `sysfetch` swap line | **Missing** | `sunshell/src/sysfetch.rs` |
| `freezram` demo command | **Missing** | — |
| Page tracking / fault / reclaim / page-in-out | **Missing** | — |

So "complete swap" means turning the block store into an actual page lifecycle:
track pages → detect pressure → evict (page-out, compress) → fault back in
(page-in, decompress) → free physical frames, all wired into the PMM.

---

## Step 1 — Real LZ4 Compression (foundation)

The doc claims LZ4; the code copies bytes. Fix this first so every later step
stores genuinely compressed payloads and `used_bytes` reflects real ratios.

- Add `lz4_flex = { version = "0.11", default-features = false }` to
  `kernel/Cargo.toml` (confirm it builds `no_std`; it does with default-features off).
- `compress_page()` → `lz4_flex::compress(src)`; if result `>= ZRAM_BLOCK_SIZE`
  store the page **uncompressed** with a 1-byte header flag (`0 = raw, 1 = lz4`)
  so incompressible pages never inflate.
- `decompress_page()` → branch on the header flag; `lz4_flex::decompress_into`
  for the lz4 case, plain copy for raw.
- Keep the slot model identical so nothing downstream changes yet.

**Gate:** existing zram round-trip unit tests pass; a zero-page compresses to
< 100 bytes (verifiable via `stats().2`).

---

## Step 2 — Page Tracking

Introduce a per-physical-frame tracking table so the kernel knows what is
swappable and what is pinned.

- Add `struct FrameDesc { owner_pid: u32, vaddr: u64, flags: FrameFlags }`.
  Flags: `PINNED` (kernel/DMA), `ANON` (process anonymous page, swappable),
  `ACCESSED`, `DIRTY`.
- Maintain a reclaim-candidate list (FIFO ring to start; CLOCK/second-chance
  later) of `ANON` frames keyed by `(pid, vaddr)`.
- Hook frame ownership at the point user pages are mapped (the VMM map path
  used by `Spawn`/heap growth) so anonymous user pages register as candidates.
- New API surface in `kernel/src/memory/`:
  - `swap::track_anon(pid, vaddr, frame)`
  - `swap::untrack(frame)` (on unmap / process exit)

**Gate:** spawning a userland util registers N anonymous frames; process exit
untracks them (assert candidate count returns to baseline).

---

## Step 3 — Swap Manager + PMM Integration

A `swap` module that owns the policy and bridges PMM ⇄ ZRAM.

- `swap::swap_out(frame) -> Result<SwapSlot>`:
  1. Read the 4 KiB frame contents.
  2. `zram::write_page()` → `BlockId`.
  3. Record `vaddr → SwapSlot(BlockId)` in the owning process's page table as a
     **not-present, swapped** PTE (use available PTE bits to encode the slot).
  4. `pmm.free_frame(frame)` — the physical frame returns to the allocator.
- `swap::swap_in(pid, vaddr) -> frame`:
  1. `pmm.alloc_frame()`.
  2. `zram::read_block(slot)` → decompress into the frame.
  3. Re-map PTE present; `zram::discard_block(slot)`.
- Reclaim trigger `swap::reclaim(target_pages)`: pop candidates and `swap_out`
  until target met or list empty.

**Gate:** unit/integration test — write a known pattern to a tracked frame,
force `swap_out`, assert frame is free and PTE is not-present; `swap_in` and
assert the pattern round-trips byte-for-byte.

---

## Step 4 — Page-Fault & Reclaim Triggers

Wire the manager into live execution.

- **Page-fault handler** (`#PF`, vector 14): if the faulting address has a
  swapped PTE, call `swap::swap_in` and resume; otherwise existing
  fault behavior (kill/log).
- **Reclaim trigger**: when `pmm.alloc_frame()` returns `None` (or free frames
  drop below a low-watermark, e.g. 5% of RAM), invoke `swap::reclaim()` then
  retry the allocation once.
- Guard against recursion/deadlock: reclaim must not allocate from the heap
  while holding the PMM lock (pre-allocate the LZ4 scratch buffer at init).

**Gate:** allocate under artificial memory pressure (shrink PMM or allocate a
balloon), observe `[SWAP] page-out` / `[SWAP] page-in` serial logs, and confirm
the workload completes correctly.

---

## Step 5 — `freezram` Demo Command (controlled live activity)

A safe, explicit way to prove swap works on demand before relying on organic
pressure — extends the existing boot smoke block.

- Kernel: add a small syscall or debug ioctl `swapctl(op, n)`:
  - `op=fill`: allocate + `swap_out` `n` synthetic pages (filled with a known
    pattern) → ZRAM usage climbs.
  - `op=verify`: `swap_in` them, assert pattern, free → usage drops.
  - `op=stat`: return `zram::stats()`.
- Userland: `freezram [n]` applet in `sunlight-utils/src/main.rs` that calls
  `swapctl`, then prints before/after swap usage so the user sees ZRAM toggle
  live (and the compression ratio from real LZ4).

**Gate:** `freezram 64` raises `Swap: used` by ~256 KiB then `freezram verify`
returns it to baseline; no leaks across repeated runs.

---

## Step 6 — Surface Swap in `free -h` and `sysfetch`

Reporting is mostly done for `free`; add the visual to `sysfetch`.

- `free`: already renders the `Swap:` row from sysinfo (`main.rs:549`). After
  Step 1, also display the **compression ratio** (`used_blocks*4KiB /
  used_bytes`) when `-h` — e.g. `Swap: 256M total / 4M used (3.1x)`.
- `sysfetch` (`sunshell/src/sysfetch.rs`):
  - Add `swap_used` / `swap_total` params to `render_sysfetch_to_buffer`.
  - Render a color-coded `Swap:` line and a 10-block bar mirroring the existing
    `Memory:` block (reuse the green/yellow/red threshold logic at
    `sysfetch.rs:92`). Only emit the line when `swap_total > 0`.
  - Update the caller that builds the sysfetch payload to pass the swap fields
    already available from the sysinfo syscall.
  - Keep total output under the 512-byte payload threshold noted in the file.

**Gate:** `sysfetch` shows a Swap line + bar that tracks `freezram` activity;
payload stays within budget; `free -h` ratio matches `zram::stats()`.

---

## Implementation Order & Risk Notes

1. **Step 1 (LZ4)** — lowest risk, unblocks honest accounting.
2. **Step 6 reporting** can land right after Step 1 (using the smoke block) for
   immediate visible feedback, before the harder kernel work.
3. **Steps 2–4** are the real kernel surgery — do them behind serial logging and
   the `freezram` harness (Step 5) so each is independently verifiable.
4. **Deadlock discipline**: never compress/alloc-heap while holding the PMM
   lock; pre-allocate scratch buffers at `zram::init()`.
5. **PTE encoding**: reserve swap-slot bits carefully so a swapped PTE is never
   mistaken for a present mapping; document the bit layout next to the handler.

Confirm between steps (per the Phase 6.5 working agreement). Each step has its
own gate and should compile + boot in QEMU before moving on.

---

## File Touch List

| File | Change |
|------|--------|
| `kernel/Cargo.toml` | add `lz4_flex` (no_std) |
| `kernel/src/memory/zram.rs` | real LZ4 compress/decompress + header flag |
| `kernel/src/memory/swap.rs` *(new)* | frame tracking, swap_out/in, reclaim |
| `kernel/src/memory/mod.rs` | `pub mod swap` |
| `kernel/src/arch/x86_64/` (PF handler) | swapped-PTE fault path |
| PMM alloc path | low-watermark reclaim trigger |
| `kernel/src/arch/x86_64/syscall.rs` | `swapctl` syscall; sysinfo already done |
| `kernel/src/main.rs` | replace/extend `init_swap_smoke` |
| `sunlight-utils/src/main.rs` | `freezram` applet; `free -h` ratio |
| `sunshell/src/sysfetch.rs` | swap line + bar in `render_sysfetch_to_buffer` |
