# VM Display Policy

## Audit

SunlightOS now diagnoses display-mode availability inside the OS, but it still
does not force unsupported modes. The active desktop size comes from whichever
layer created the framebuffer or scanout first:

- Kernel boot splash:
  `kernel/src/main.rs` keeps the existing safe Limine framebuffer request and
  uses the current bootloader-selected width and height.
- Display server fallback path:
  `kernel/src/arch/x86_64/syscall.rs` maps that same Limine framebuffer into
  `sunlight-display`, which treats it as the physical fallback backend.
- VirtIO GPU path:
  `kernel/src/main.rs` probes `GET_DISPLAY_INFO`, logs non-empty reported
  scanout rectangles, then caches scanout 0 as the current size.
- Compositor render size:
  `services/sunlight-display/src/main.rs` uses the VirtIO GPU size when present,
  otherwise the Limine framebuffer size, otherwise a safe internal fallback.

That means guest-side mode switching still does not happen inside the kernel or
display service. For QEMU VM boots, actual resolution selection happens in the
host launcher when the selected video device exposes explicit `xres` / `yres`
properties. For VMware, default display-service logs confirm the active
framebuffer. Full GOP alternate-mode dumps require a safer bootloader/protocol
path before they can be enabled by default.

There is no in-OS mode switcher yet:

- no display-service mode enumeration or mode-set API
- no physical-hardware mode switch path in this patch
- no dynamic resize handling beyond taking the initial scanout size
- no default `sunlight.resolution=WIDTHxHEIGHT` parser; adding the Limine
  command-line request needs separate boot validation before it is safe

## Policy In This Patch

This patch keeps physical hardware unchanged. VM preference is applied only when
the host-side QEMU launcher is selecting an explicit VM resolution.

Preferred VM order:

1. `1366x768`
2. `1360x768`
3. `1280x800`
4. `1280x720`
5. `1440x900`
6. `1024x768`

If none of those exact modes exists, ranked fallback prefers practical VM/demo
modes with:

- width at least `1280`
- height at least `720`
- aspect ratio near `16:9` or `16:10`
- automatic cap at `1920x1080`
- current safe mode retained if candidates look suspicious

The runner only requests a resolution when the selected QEMU device reports
`xres` / `yres` support via `qemu-system-x86_64 -device ... ,help`.
If that support is missing, it preserves the previous behavior.

Boot logs now include mode discovery and fallback explanations:

- `[display] available mode 0: WIDTHxHEIGHT pitch=... bpp=... format=limine-framebuffer current=yes`
- `[display] available virtio scanout N: WIDTHxHEIGHT ... format=virtio-gpu`
- `[display] current mode: WIDTHxHEIGHT`

## Backend Selection Diagnostics

Backend selection in `sunlight-display` is explicit: the VirtIO GPU becomes the
active backend only after resource create and backing attach succeed. Any
failure logs its exact step and reason, then reverts every render dimension
(compositor, mouse bounds, shell screen size, window placement) to the Limine
framebuffer mode — VirtIO and Limine dimensions are never mixed.

`SET_SCANOUT` itself is deferred until the first `SESSION_ACTIVATE` from
`tty_server` (i.e. after the user logs in with the Desktop session selected,
or presses Ctrl+F2). QEMU's `virtio-vga` keeps displaying the VGA-compat
output — the Limine framebuffer holding the TTY login screen — until the first
non-zero `SET_SCANOUT`, so deferring it keeps the login screen visible during
boot instead of jumping straight to the desktop. The same deferral removes the
old startup present, which used to overwrite the login screen on the Limine
backend too.

The scanout resource is created with `VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM`
(memory bytes B,G,R,X — a little-endian u32 holding `0x00RRGGBB`, matching the
compositor back buffer). It was previously `X8R8G8B8_UNORM`, which swapped the
red/green channels and dropped blue entirely, producing a green-tinted screen.

The startup summary is a single serial line:

```
[DISPLAY] display_backend=<VirtIO|Limine> requested=<WxH> virtio_scanout=<WxH|none> final=<WxH> reason=<...>
```

- `requested` — the mode the host asked for via the VirtIO scanout report
  (QEMU `xres`/`yres` override or current window size); without a VirtIO GPU
  this is the Limine framebuffer mode.
- `virtio_scanout` — raw scanout 0 size from `GET_DISPLAY_INFO`, or `none`.
- `final` — the size every consumer (compositor, `GET_SCREEN_INFO` clients,
  mouse clamps, Vortex Shell) uses.
- `reason` — one of `virtio-attach-ok-scanout-deferred`,
  `virtio-attach-backing-failed`, `virtio-buffer-alloc-failed`,
  `virtio-invalid-size`, `no-virtio-gpu`, `limine-invalid-size-safe-fallback`.

On failure the kernel additionally logs the exact VirtIO response code, e.g.
`[GPU] RESOURCE_ATTACH_BACKING (N entries) failed: 0x1205 (ERR_INVALID_PARAMETER)`,
and the display server logs the proxied reason
(`gpu_attach_backing FAILED reason=... detail=0x...`).

Historical note: `RESOURCE_ATTACH_BACKING` used to fail for scanouts larger
than 1024 pages (about 1024x1024x32bpp) because the scatter-gather list was
silently truncated while the command header still claimed the full entry
count; the device rejected the malformed command and the display fell back to
Limine. The kernel now coalesces physically-contiguous pages into ranged
entries (a multi-megabyte buffer typically needs only a handful) and fails
loudly with `sg-overflow` if the list still cannot fit.

## Override

QEMU override is available through either:

- `./tools/runs.sh --resolution 1280x720`
- `SUNLIGHT_QEMU_RESOLUTION=1280x720 ./tools/runs.sh`

Kernel command-line override (`sunlight.resolution=WIDTHxHEIGHT`) is not enabled
in the default kernel. It was left out because the required Limine command-line
request needs separate boot validation; unsupported or unsafe bootloader
requests must not break the safe framebuffer fallback.

VMware and VirtualBox launcher paths do not yet negotiate guest display modes
from this repo. On VMware, use the boot logs to confirm whether the firmware
exposed `1366x768`, `1280x800`, or `1280x720`.

## Scope And Limitations

- Physical hardware boot/display behavior is intentionally unchanged.
- VMware and VirtualBox remain hypervisor-managed in this patch; the current
  repo launcher paths for them do not yet negotiate guest modes.
- No display settings UI is added here.
- No dynamic resize correctness work is added here.
- No scaling or multi-monitor work is added here.

## Current Fallbacks

- QEMU launcher: exact preferred order above -> ranked best practical mode ->
  backend default/current mode
- Kernel/display diagnostics: current Limine framebuffer mode and non-empty
  VirtIO scanouts are logged; unsupported modes are not forced
- Display server render size: valid VirtIO GPU size -> valid Limine framebuffer
  size -> internal `1280x800` allocation fallback only to avoid a bogus `0x0`
  compositor buffer
- Physical hardware: unchanged bootloader/kernel/display fallback path

## Manual Validation

- Default boot: inspect `[display] available mode ...`, `[display] current
  mode`, and selected/fallback reason in serial logs
- VMware fullscreen/windowed: confirm whether fullscreen changes the guest
  framebuffer resolution or only scales the VM window
- QEMU: boot with `./tools/runs.sh` and confirm `Resolution: 1366x768
  (vm-policy)` when the VirtIO backend exposes `xres` / `yres`
- QEMU: open Start Menu, Terminal, Calculator, Files, Settings, and Task Manager
  and confirm layout, hit-testing, and dock/top-bar placement remain correct
- QEMU fallback: test an environment or device path without explicit resolution
  support and confirm the runner warns then keeps the backend default
- Override: boot with `./tools/runs.sh --resolution 1280x720` and confirm the
  runner reports `Resolution: 1280x720 (override)`
- Hardware: confirm normal physical boots still use their existing safe/default
  framebuffer path with no forced `1366x768`

Known VMware limitation: fullscreen may scale the VM window without changing the
guest framebuffer. If `1366x768` is not present in the mode dump, SunlightOS
cannot safely select it from inside this patch.

## Display Metrics Source Of Truth

`ipc/src/display_metrics.rs` defines `DisplayMetrics`, shared by:

- `sunlight-display` compositor (`GET_SCREEN_INFO` IPC reply)
- `sunlight-vortex-shell` desktop sizing
- `sunlight-mouse` TTY fallback clamp bounds
- `sunlight-control-panel` read-only monitor page
- `sunlight-utils display-status` CLI applet

`GET_SCREEN_INFO` reply words:

| Word | Contents |
|------|----------|
| 0 | `width \| (height << 32)` |
| 1 | `stride_bytes \| (pixel_format << 32)` |
| 2 | `scale_fp \| (backend << 32)` — `65536` = 1.0× native |
| 3 | `refresh_hz` — `0` when unknown |

Support level (honest):

| Capability | Status |
|------------|--------|
| Detect framebuffer / VirtIO scanout size at boot | Supported |
| Desktop layout adapts to detected size | Supported |
| Boot-time QEMU resolution via `runs.sh` | Supported when device exposes `xres`/`yres` |
| Kernel `sunlight.resolution=` boot arg | Not enabled (needs Limine validation) |
| Runtime mode switching | Not supported |
| HiDPI / per-monitor scale | Not supported (`scale_fp` placeholder only) |
| Multi-monitor | Not supported |

CLI: `display-status` (via `sunlight-utils`) prints current metrics when the
desktop session is active.

## Future Work

- Add a kernel/display-server resolution override channel if mode switching
  becomes available in the boot or GPU backend.
- Surface resolution controls in Settings once the compositor can switch modes
  safely.
- Support remembered per-machine VM resolution preferences.
- Add dynamic resize for VirtIO GPU scanout changes.
- EDID / full mode enumeration when the driver path is safe.
- HiDPI scaling and per-monitor scale factors.
