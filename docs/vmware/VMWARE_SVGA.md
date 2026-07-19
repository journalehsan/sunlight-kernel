# VMware SVGA II 2D display driver

Minimal, correct legacy framebuffer backend for the VMware virtual display
adapter (`15ad:0405`). This is **not** a 3D / `vmwgfx` port: no OpenGL, no
screen objects, no hardware cursor, and no multimonitor.

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

## Manual resolution selection

System Preferences now queries backend-neutral mode capabilities from
`sunlight-display`. It never enables the selector by matching a backend name.

VMware reports a curated set of common 4:3, 16:9, and 16:10 modes after
filtering each entry through:

- live `SVGA_REG_MAX_WIDTH` / `SVGA_REG_MAX_HEIGHT`
- 32-bpp support
- checked visible-byte arithmetic
- live framebuffer extent and BAR1 aperture limits
- the display service's mapped aperture capacity
- the minimum usable desktop geometry

The current mode is always included. Candidate pitch is reported as unknown
until modeset readback; `SVGA_REG_BYTES_PER_LINE` is authoritative after the
host accepts the mode.

VirtIO remains preferred and automatically managed. Limine remains read-only.

## Resolution policy (boot default)

Same spirit as `tools/vm_display_policy.sh` and QEMU VirtIO `xres`/`yres`:

| Rule | Value |
|------|--------|
| Min HD | **1280×720** when the device allows |
| Preferred order | 1366×768 → 1360×768 → 1280×800 → 1280×720 → 1440×900 → 1024×768 |
| Soft auto-max | **1920×1080** |
| Host/window hint | Boot Limine size (often the VM window) and/or last SVGA mode |

**Boot path keeps the firmware mode** (typically 1024×768) so splash and TTY
login stay correct. Modesetting larger while splash/TTY still paint with the
Limine pitch produces diagonal tearing on the SVGA scanout.

The boot activation policy still selects a safe initial VMware mode. Once the
display service starts, background VMware policy modesets are disabled so they
cannot race a user-owned preview transaction.

Without VMware Tools, the host does not push window resizes into the guest; the
guest drives a good HD+ mode and the host can “Autofit Window” to match.

## Backend selection and fallback

Order in `sunlight-display`:

1. **VirtIO GPU** — preferred when QEMU presents a working VirtIO GPU and
   attach succeeds (unchanged path).
2. **VMware SVGA** — selected only when the kernel reports the driver
   **Active** and returned mapping geometry matches the live mode. Presentation
   uses an independently mapped BAR1 aperture and issues `SVGA_CMD_UPDATE`
   after each blit.
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
7. Choose mode via VM policy (host hint + min HD + preferred list + VRAM)
8. Modeset to that size @ 32 bpp when different from firmware
9. Compare Limine FB phys against BAR1+`FB_OFFSET` (identity diagnostic)
10. On full success: inventory **Matched/Bound=`vmw-svga`**, **State=Active**
11. On any failure: **ProbeFailed** with stage/code; boot FB unchanged

Display service (`services/sunlight-display`):

1. Map FB (syscall 118) — uses BAR1 base when VMware is Active
2. Try VirtIO; on success stop
3. Else `svga_get_info()`; if map geometry matches → `DisplayBackend::VmwareSvga`
4. Present: memcpy rect → fence → `svga_update(x,y,w,h)`
5. Manual preview: exact modeset → readback → pointer/stride/buffer reconfigure

## BAR1 mapping and bounds

The compositor does not depend on the Limine mapping length for larger modes.
Syscall 118 maps a driver-owned BAR1 aperture at a dedicated display virtual
address after normal framebuffer authorization. It does not expose an
arbitrary physical mapping facility.

The mapping:

- starts at the validated BAR1 physical base
- covers the validated reusable mode budget plus the current `FB_OFFSET`
- remains bounded by BAR1 and the device-reported framebuffer extent
- returns the current visible pointer as `aperture + FB_OFFSET`
- reports `FB_OFFSET` and mapped capacity through the SVGA info ABI

After every modeset the display service recomputes the visible pointer from the
hardware readback offset. It rejects and rolls back any mode whose
`offset + pitch * height` exceeds the mapped aperture.

## Preview transaction

`sunlight-display` owns the only active preview transaction:

1. validate the requested mode against current capabilities
2. snapshot the previous geometry
3. issue the exact kernel modeset
4. re-read width, height, bpp, pitch, offset, and extent
5. reconfigure compositor metrics, pointer bounds, window placement, and the
   reusable aperture-sized back buffer
6. restart the fullscreen Vortex desktop surface at the new dimensions
7. invalidate and redraw the full screen
8. wait for Keep, Revert, caller exit, or the 30-second deadline

The token is random when the random service is available and is always bound to
the kernel-authenticated caller PID. Stale tokens, other callers, and concurrent
transactions are rejected. The service timer owns automatic rollback; the
Control Panel countdown is only a visual reflection of the deadline.

Confirmed VMware modes are stored under `display.vmware.mode`. The setting is
validated and applied only when VMware is active; invalid or rejected settings
are ignored with a bounded diagnostic.

## Serial log markers

```text
[SVGA] pci device 15ad:0405 found at BB:SS.F rev=...
[SVGA] BAR0 IO port=... size=...
[SVGA] BAR1 FB phys=... size=...
[SVGA] BAR2 FIFO phys=... size=...
[SVGA] probe version=... caps=... vram=... fb_size=... fb_off=... fifo=...
[SVGA] probe mode WxH pitch=... bpp=... enable=... config_done=...
[SVGA] active WxH pitch=... bpp=... fb_phys=... boot_fb_in_vram=... stage=active reason=preferred-hd host_hint=... max=...
[DISPLAY] VMware SVGA backend selected WxH ...
[DISPLAY] display_backend=VMwareSVGA ... reason=vmware-svga-ok-policy-mode
[SVGA] modeset WxH pitch=... reason=...   # optional later resize
[DISPLAY] SVGA resized to WxH ...
[DISPLAY] mode_change_requested backend=VMwareSVGA old=... requested=...
[DISPLAY] mode_change_validated requested=...
[DISPLAY] mode_readback WxH pitch=...
[DISPLAY] display_buffers_reconfigured WxH pitch=... mapped_bytes=...
[DISPLAY] full_redraw_requested
[DISPLAY-MODE] preview active token=... deadline=30s hardware readback=...
[DISPLAY-MODE] confirmation dialog shown
[DISPLAY-MODE] confirmed persisted=yes|no
[DISPLAY-MODE] reverted reason=explicit|timeout|owner-exited|ui-closed result=ok|failed
[DISPLAY-MODE] failed stage=... error=...
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
- No multimonitor
- No VMware Tools RPC “Autofit Guest” (host→guest window push)
- No advanced 2D accel (RECT_COPY, etc.)
- Manual modes are limited to the validated mapped aperture
- No interrupt-driven FIFO; bounded polling on full FIFO
- SVGA3 (different BAR layout) is out of scope
- No HiDPI/logical scaling, refresh-rate selection, dynamic DPI, or multiple monitors

## Implementation map

| Path | Role |
|------|------|
| `sunlight-virtio/src/svga_regs.rs` | Constants from `svga_reg.h` |
| `sunlight-virtio/src/svga.rs` | Driver (probe, policy modeset, FIFO, UPDATE) |
| `sunlight-virtio/src/pci.rs` | `probe_vmware_svga` + I/O BAR parse |
| `kernel/src/main.rs` | Boot init + inventory |
| `kernel/src/arch/x86_64/syscall.rs` | Syscalls 127–129 |
| `ipc/src/lib.rs` | `svga_get_info` / `svga_update` / `svga_set_mode` |
| `services/sunlight-display/` | Backend selection, present, resize |
