# VMware SVGA II 2D display driver

Minimal, correct legacy framebuffer backend for the VMware virtual display
adapter (`15ad:0405`). This is **not** a 3D / `vmwgfx` port: no OpenGL, no
screen objects, no hardware cursor, no multimonitor, and no dynamic resize.

Related guides:

- Driver model notes: [`docs/2026-06-21_RING3_KEYBOARD_DRIVER.md`](../2026-06-21_RING3_KEYBOARD_DRIVER.md)
- Display policy: [`docs/GUI/VM_DISPLAY_POLICY.md`](../GUI/VM_DISPLAY_POLICY.md)
- VirtIO GPU plan (sibling path): [`docs/GUI/VIRTIO_GPU_2D_DRIVER_PLAN.md`](../GUI/VIRTIO_GPU_2D_DRIVER_PLAN.md)
- Serial debugging under VMware: [`docs/vmware/VMWARE_SERIAL_DEBUGGING.md`](VMWARE_SERIAL_DEBUGGING.md)

## Supported hardware

| Field | Value |
|-------|--------|
| Name | VMware SVGA II |
| PCI ID | `15ad:0405` |
| BAR0 | I/O ports (index / value) |
| BAR1 | Framebuffer / VRAM |
| BAR2 | FIFO command memory |
| Feature level | SVGA ID 2 (fallback 1, 0), linear FB + `SVGA_CMD_UPDATE` |

PCI BDF and BAR addresses are discovered at runtime and are never hardcoded.

## Backend selection and fallback

Order in `sunlight-display`:

1. **VirtIO GPU** — preferred when QEMU presents a working VirtIO GPU and
   attach succeeds (unchanged path).
2. **VMware SVGA** — selected only when the kernel reports the driver
   **Active** and the SVGA mode geometry matches the Limine boot framebuffer
   (width, height, pitch). Presentation reuses the Limine mapping and issues
   `SVGA_CMD_UPDATE` after each blit.
3. **Limine framebuffer** — final fallback; never removed.

VMware is never an unconditional default. Failure at any SVGA stage leaves the
boot framebuffer visible and does not publish the backend as Active.

## Authoritative references

Constants and sequencing are taken from VMware `svga_reg.h` as shipped with the
Linux `vmwgfx` driver (GPL-2.0 OR MIT), cross-checked against SerenityOS’s
VMWare SVGA II adapter:

- Register index/value I/O at BAR0 + `SVGA_INDEX_PORT` / `SVGA_VALUE_PORT`
- Version negotiation via `SVGA_REG_ID` (`SVGA_ID_2` → `1` → `0`)
- FIFO layout: `MIN` / `MAX` / `NEXT_CMD` / `STOP`, then `SVGA_REG_CONFIG_DONE`
- Presentation: guest writes pixels, then `SVGA_CMD_UPDATE` + `SVGA_REG_SYNC`

Definitions live in `sunlight-virtio/src/svga_regs.rs`.

## Probe and activation sequence

Kernel (`kernel/src/main.rs`), after VirtIO GPU init:

1. PCI match `15ad:0405`
2. Parse BAR0 (I/O), BAR1 (FB), BAR2 (FIFO) with size probing; enable I/O+MEM
3. Map FIFO via HHDM
4. **Probe-only** register read (version, caps, sizes, current mode) — no blanking
5. Validate BAR/size/offset invariants
6. Initialize FIFO metadata; set `CONFIG_DONE`
7. Prefer existing firmware mode when 32-bpp and usable; otherwise set mode to
   boot Limine width×height @ 32 bpp
8. Compare Limine FB phys against BAR1+`FB_OFFSET`
9. On full success: inventory **Matched/Bound=`vmw-svga`**, **State=Active**
10. On any failure: **ProbeFailed** with stage/code; boot FB unchanged

Display service (`services/sunlight-display`):

1. Map Limine FB (syscall 118) as always
2. Try VirtIO; on success stop
3. Else `svga_get_info()`; if geometry matches Limine → `DisplayBackend::VmwareSvga`
4. Present: memcpy rect → fence → `svga_update(x,y,w,h)`

## Serial log markers

```text
[SVGA] pci device 15ad:0405 found at BB:SS.F rev=...
[SVGA] BAR0 IO port=... size=...
[SVGA] BAR1 FB phys=... size=...
[SVGA] BAR2 FIFO phys=... size=...
[SVGA] probe version=... caps=... vram=... fb_size=... fb_off=... fifo=...
[SVGA] probe mode WxH pitch=... bpp=... enable=... config_done=...
[SVGA] active WxH pitch=... bpp=... fb_phys=... boot_fb_in_vram=... stage=active
[DISPLAY] VMware SVGA backend selected WxH ...
[DISPLAY] display_backend=VMwareSVGA ... reason=vmware-svga-ok-boot-fb-in-vram
```

Failure boundaries name the stage, for example:

```text
[SVGA] probe invariant failed: ...
[SVGA] activation failed at stage boundary: ...
[DISPLAY] VMware SVGA ready but geometry mismatch ...; keeping Limine fallback
```

## Device Manager

Driver short name: **`vmw-svga`** (8-byte packed inventory name).

| Stage | Matched | Bound | State |
|-------|---------|-------|-------|
| PCI found, init incomplete | vmw-svga | — | Loaded |
| Activation failed | vmw-svga | — | ProbeFailed |
| Fully usable | vmw-svga | vmw-svga | Active |
| Not present | — | — | Without driver |

## How to test

### Build

```bash
./runs.sh --build
# ISO: target/sunlightos.iso
```

### VMware

Follow [`VMWARE_SERIAL_DEBUGGING.md`](VMWARE_SERIAL_DEBUGGING.md):

1. Backup the VMX
2. Attach `target/sunlightos.iso`
3. Capture COM1 to `/tmp/sunlight-vmware-serial.log`
4. Boot and confirm the `[SVGA]` / `[DISPLAY]` markers above
5. Exercise desktop: move windows, open menus, expose edges
6. Confirm Devices shows VMware SVGA II as Active with `vmw-svga`
7. Restore temporary VMX serial settings after the run

### QEMU VirtIO regression

```bash
./runs.sh   # or the project’s usual QEMU path
```

Expect VirtIO GPU still preferred; `[SVGA] VMware SVGA II (15ad:0405) not present`.

## Common failure boundaries

| Symptom | Check |
|---------|--------|
| No `[SVGA] pci device` | Guest has no `15ad:0405` (not VMware SVGA II) |
| ProbeFailed after BAR lines | BAR type/size invalid — see serial error |
| Version unsupported | Host rejected ID 2/1/0 |
| Geometry mismatch | SVGA mode ≠ Limine; display keeps Limine intentionally |
| Black screen after Active | FIFO/UPDATE path; look for `[SVGA] update ... failed` |
| Devices still Without driver | Kernel never reached Active inventory update |

## Current limitations

- No screen objects / GBOs / 3D
- No hardware cursor (software cursor only)
- No dynamic resize or multimonitor
- No advanced 2D accel (RECT_COPY, etc.)
- Presentation requires Limine geometry match for the first backend
- No interrupt-driven FIFO; bounded polling on full FIFO
- SVGA3 (different BAR layout) is out of scope

## Implementation map

| Path | Role |
|------|------|
| `sunlight-virtio/src/svga_regs.rs` | Constants from `svga_reg.h` |
| `sunlight-virtio/src/svga.rs` | Driver (probe, FIFO, UPDATE) |
| `sunlight-virtio/src/pci.rs` | `probe_vmware_svga` + I/O BAR parse |
| `kernel/src/main.rs` | Boot init + inventory |
| `kernel/src/arch/x86_64/syscall.rs` | Syscalls 127–128 |
| `ipc/src/lib.rs` | `svga_get_info` / `svga_update` |
| `services/sunlight-display/` | Backend selection + present |
