# SunlightOS — VirtIO GPU 2D Driver Implementation Plan

**Status:** Planning  
**Last updated:** 2026-06-27  
**Scope:** Add a VirtIO GPU 2D display path to `sunlight-display` for QEMU/VM environments. No 3D acceleration. No compositor UI changes. Preserve current flicker-free software compositing behavior.

---

## Background

The current display path writes directly to a Limine-provided physical framebuffer. This works on bare metal and in QEMU with `-vga std`, but is not the preferred path in a VirtIO-capable VM. VirtIO GPU 2D gives QEMU a proper display device, improves resize/resolution handling, and is the standard path used by modern Linux guests.

The compositor (`sunlight-display`) already maintains a `back_buffer: Vec<u32>` for all rendering and only copies to hardware in `present_rect()` / `present_back_buffer()`. That present seam is the only thing that needs to change.

---

## Relevant Files

| File | Role |
|---|---|
| `kernel/src/main.rs:33–34` | Limine `FramebufferRequest` static |
| `kernel/src/main.rs:544–597` | Maps Limine FB pages into tty_server |
| `kernel/src/main.rs:1173–1204` | `map_tty_framebuffer()` |
| `kernel/src/arch/x86_64/syscall.rs:3563–3625` | `sys_map_framebuffer` (syscall 118) |
| `services/sunlight-display/src/main.rs:899–929` | `present_back_buffer()` / `present_rect()` |
| `services/sunlight-display/src/main.rs:1332–1357` | `redraw_scene()` — decides full vs. partial present |
| `services/sunlight-display/src/main.rs:1424–1481` | Entry / init — acquires framebuffer |
| `services/sunlight-display/src/dirty.rs` | Dirty-rect tracking |
| `sunlight-virtio/src/pci.rs` | PCI config-space scan, port I/O primitives |
| `sunlight-virtio/src/blk.rs` | VirtIO block — reference implementation pattern |
| `sunlight-net/src/virtio_net.rs` | VirtIO net — reference implementation pattern |
| `kernel/src/arch/x86_64/interrupts.rs:105–173` | IDT / 8259 PIC init |
| `ipc/src/lib.rs:243–298` | SGP message labels |

---

## Current Display Path

```
QEMU firmware
  └── Limine (framebuffer at a physical address)
        └── kernel/src/main.rs
              ├── maps FB pages → tty_server VA 0x0000_0002_0000_0000
              └── syscall 118 (MapFramebuffer)
                    └── sunlight-display (ring-3)
                          ├── back_buffer: Vec<u32>   ← all drawing goes here
                          ├── dirty: DirtyList
                          └── present_rect() / present_back_buffer()
                                └── memcpy back_buffer → *fb (Limine phys pages)
```

---

## Recommended Driver Placement: Kernel-side, proxy-syscall pattern

PCI config-space access (`in`/`out`) is a privileged instruction. This rules out a pure ring-3 driver without additional kernel work.

The established SunlightOS pattern is:
- Hardware driver lives in ring-0 (kernel crate or a crate linked into the kernel).
- A single named userspace service drives it through dedicated proxy syscalls.
- Access is gated by `process.name == "..."` (same as `net_server` / `sys_net_tx`).

A VirtIO GPU kernel driver stored in `static GPU_DEVICE: spin::Mutex<Option<VirtioGpu>>` (matching `NET_DEVICE`) is the correct shape. `sunlight-display` calls proxy syscalls to issue GPU commands; the kernel forwards them to the VirtIO GPU ring.

---

## Minimal VirtIO GPU 2D Command Set

Only six commands are needed to drive a 2D scanout:

| Command | Code | Purpose |
|---|---|---|
| `VIRTIO_GPU_CMD_GET_DISPLAY_INFO` | 0x0100 | Detect scanout resolution |
| `VIRTIO_GPU_CMD_RESOURCE_CREATE_2D` | 0x0101 | Allocate a host-side 2D resource |
| `VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING` | 0x0106 | Bind guest RAM pages to the resource |
| `VIRTIO_GPU_CMD_SET_SCANOUT` | 0x0103 | Wire the resource to display scanout 0 |
| `VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D` | 0x0105 | Upload a dirty rectangle from guest RAM |
| `VIRTIO_GPU_CMD_RESOURCE_FLUSH` | 0x0104 | Signal the host to present to screen |

No 3D, no cursor, no multi-scanout.

---

## Proposed Backend Abstraction

Add `services/sunlight-display/src/backend.rs` (~60 lines). No new crates, no trait-object overhead.

```rust
pub enum DisplayBackend {
    /// Limine physical framebuffer mapped via syscall 118.
    Limine {
        fb: *mut u32,
        pitch_words: usize,
    },
    /// VirtIO GPU scanout driven via kernel proxy syscalls.
    VirtioGpu {
        width: u32,
        height: u32,
        // back_buffer pages are pinned by GpuAttachBacking at startup.
    },
}
```

`present_rect()` becomes:

```rust
fn present(state: &CompositorState, rect: Rect) {
    match &state.backend {
        DisplayBackend::Limine { fb, pitch_words } => {
            // existing memcpy path, unchanged
        }
        DisplayBackend::VirtioGpu { .. } => {
            syscall::gpu_transfer_to_host_2d(rect.x, rect.y, rect.w, rect.h);
            syscall::gpu_resource_flush(rect.x, rect.y, rect.w, rect.h);
        }
    }
}
```

The `back_buffer: Vec<u32>` stays as-is. With VirtIO GPU it is the scanout buffer — the kernel driver reads directly from those guest pages. No extra copy.

---

## Phased Implementation Plan

---

### Phase 1 — PCI MMIO BAR support

**Files:** `sunlight-virtio/src/pci.rs`

All current VirtIO drivers use legacy I/O BARs. VirtIO GPU (device ID `0x1050`) uses a modern PCI capability structure with MMIO BARs. This phase adds the missing infrastructure.

Tasks:
1. `read_bar(bus, slot, func, bar_index) -> Option<(phys: u64, size: u64, is_mmio: bool)>`  
   Write `0xFFFF_FFFF` to the BAR register, read back, mask off type bits, two's-complement to get size. Handle 64-bit BARs (type bits `[2:1] == 0b10` → next BAR holds the high 32 bits).
2. `map_mmio_bar(phys: u64, size: u64) -> *mut u8`  
   Maps via the HHDM offset already used elsewhere (`PHYSICAL_MEMORY_OFFSET`).
3. `find_virtio_gpu() -> Option<VirtioGpuPci>`  
   Scans for vendor `0x1AF4`, device `0x1050`. Returns bus/slot/func plus a pointer to the mapped common config region.

**Gate:** `find_virtio_gpu()` returns `Some(...)` in QEMU launched with `-device virtio-gpu-pci`.

---

### Phase 2 — VirtIO GPU ring and command types

**Files:** `sunlight-virtio/src/gpu.rs` (new), `sunlight-virtio/src/queue.rs` (new), `sunlight-virtio/src/lib.rs`

Tasks:
1. Extract `VirtqDesc` / avail-ring / used-ring types from `blk.rs` into a shared `queue.rs`. Remove the duplicate in `virtio_net.rs`.
2. Implement the six `#[repr(C)]` command structs:
   - `VirtioGpuCtrlHdr` (type, flags, fence_id, ctx_id, padding)
   - `VirtioGpuRespDisplayInfo`
   - `VirtioGpuResourceCreate2d` (resource_id, format, width, height)
   - `VirtioGpuResourceAttachBacking` (resource_id, nr_entries) + `VirtioGpuMemEntry`
   - `VirtioGpuSetScanout` (r: Rect, scanout_id, resource_id)
   - `VirtioGpuTransferToHost2d` (r: Rect, offset, resource_id)
   - `VirtioGpuResourceFlush` (r: Rect, resource_id)
3. Implement `VirtioGpu` struct:
   - `init(common_cfg: *mut u8) -> VirtioGpu` — modern VirtIO device handshake: reset → ACKNOWLEDGE → DRIVER → negotiate features → FEATURES_OK → DRIVER_OK.
   - Single `controlq` (queue index 0). No cursor queue.
4. Implement `send_command(cmd: &[u8], resp: &mut &[u8])` — places a two-descriptor chain (cmd read-only, resp write-only) into the avail ring, kicks the device via the notify register, polls the used ring (spin with `fence(SeqCst)`, matching existing drivers).

**Gate:** `GET_DISPLAY_INFO` returns a non-zero width and height.

---

### Phase 3 — Kernel integration and proxy syscalls

**Files:** `kernel/src/main.rs`, `kernel/src/arch/x86_64/syscall.rs`

Tasks:
1. In `kernel/src/main.rs`, after network init, call `find_virtio_gpu()`. On success, init `VirtioGpu` and store in:
   ```rust
   static GPU_DEVICE: spin::Mutex<Option<VirtioGpu>> = spin::Mutex::new(None);
   ```
2. During kernel init, send `RESOURCE_CREATE_2D` for resource ID 1 at the detected resolution.
3. Add four new proxy syscalls (next IDs after 118):

| Syscall | Number | Arguments | Kernel action |
|---|---|---|---|
| `GpuGetInfo` | 119 | — | Returns `(width u32, height u32)` from `GET_DISPLAY_INFO` |
| `GpuAttachBacking` | 120 | `vaddr: u64, npages: u64` | Walks VA→physical per 4KiB page, sends `RESOURCE_ATTACH_BACKING` scatter-gather list |
| `GpuSetScanout` | 121 | — | Sends `SET_SCANOUT` wiring resource 1 to scanout 0 |
| `GpuFlush` | 122 | `x, y, w, h: u32` | Sends `TRANSFER_TO_HOST_2D` then `RESOURCE_FLUSH` |

All four are gated by `process.name == "display_server"`.

`GpuAttachBacking` does not require the caller's pages to be physically contiguous: `RESOURCE_ATTACH_BACKING` accepts a scatter-gather list of `(phys_addr, len)` pairs, one per 4KiB page.

**Gate:** `GpuGetInfo` returns the correct resolution. `GpuFlush` does not fault.

---

### Phase 4 — Display server backend detection and present path

**Files:** `services/sunlight-display/src/main.rs`, `services/sunlight-display/src/backend.rs` (new)

Tasks:
1. Add `src/backend.rs` with the `DisplayBackend` enum shown above.
2. At startup, after the existing `map_framebuffer()` call, also attempt `syscall::gpu_get_info()`:
   - If it returns a non-zero size → use `VirtioGpu` backend.
   - Otherwise → fall back to `Limine` backend (existing behavior, completely unchanged).
3. If `VirtioGpu` is selected:
   - Call `GpuAttachBacking` with the VA and page count of `back_buffer`.
   - Call `GpuSetScanout`.
   - The Limine framebuffer pointer is not written to in this mode.
4. Replace the bodies of `present_back_buffer()` and `present_rect()` with the `match &state.backend` dispatch shown above.
5. For the `VirtioGpu` branch, call `GpuFlush(x, y, w, h)` per dirty rect — preserving the existing partial-present optimization from `dirty.rs`.

**Gate:** Full desktop (wallpaper, panel, window decorations, Vortex Shell) renders correctly in QEMU with `-device virtio-gpu-pci`. No flicker regression.

---

### Phase 5 — Limine fallback and QEMU flag documentation

**Files:** QEMU launch script / Makefile, `docs/GUI/CURRENT_STATE.md`

Tasks:
1. Confirm that removing `-device virtio-gpu-pci` from the QEMU command falls back cleanly to the Limine path with no panic.
2. Add a note to the QEMU launch script or Makefile distinguishing the two modes.
3. Update `CURRENT_STATE.md` to reflect the new dual-backend display path.
4. Optional: if GPU is detected at kernel boot, skip registering the Limine framebuffer mapping entirely to avoid the double-map.

**Gate:** Both launch modes boot to a working desktop without modification to any service binary.

---

## Risks and TODOs

| Risk | Severity | Mitigation |
|---|---|---|
| No MMIO BAR support today | Blocker for Phase 1 | Must land before any GPU driver work; existing I/O BAR code is not reusable |
| `GpuAttachBacking` requires guest-physical addresses of `Vec<u32>` pages | Medium | PMM already has `virt_to_phys`; walk VA per 4KiB page; use scatter-gather — no physical contiguity required |
| `VirtqDesc` duplicated in `blk.rs` and `virtio_net.rs` | Low | Phase 2 deduplication; de-risk before adding a third copy |
| Completion polling on GPU commands blocks kernel during flush | Low | `TRANSFER_TO_HOST_2D` is fast in QEMU software rendering; interrupt-driven path is a future improvement |
| Resolution mismatch between Limine and VirtIO GPU | Low | Assert `GpuGetInfo` dims match `map_framebuffer` dims at startup; resize `back_buffer` if needed |
| TTY server still holds the old Limine FB pages | Low | TTY only uses the FB during early-boot splash; once `display_server` takes over the session, the mapping is dormant — leave it for now |
| Process-name gate is fragile | Low | Matches existing pattern; acceptable for now; future work: capability token |
| No hotplug | Low | GPU assumed present at boot; detection happens once during kernel init |

---

## Non-Goals

- 3D acceleration (Vulkan, OpenGL).
- Multi-scanout / multiple displays.
- VirtIO GPU cursor resource.
- MSI-X interrupts (requires LAPIC work; left for future SMP phase).
- Redesigning the compositor UI or window protocol.
