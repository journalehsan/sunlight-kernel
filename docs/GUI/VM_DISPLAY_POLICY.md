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
- VMware SVGA II path:
  `kernel/src/main.rs` probes PCI `15ad:0405`, negotiates SVGA, initializes the
  FIFO, applies the VM resolution policy (min HD 1280×720, preferred list,
  host/window hint from the boot FB), modesets, and only marks Active when a 2D
  present path is ready. See `docs/vmware/VMWARE_SVGA.md`.
- Compositor render size:
  `services/sunlight-display/src/main.rs` prefers VirtIO GPU when attached,
  else VMware SVGA when Active (map uses SVGA VRAM at the policy mode), else the
  Limine framebuffer size, otherwise a safe internal fallback. VMware may
  re-apply policy on session activate and on a short poll interval.

That means guest-side mode switching still does not happen inside the kernel or
display service. For QEMU VM boots, actual resolution selection happens in the
host launcher when the selected video device exposes explicit `xres` / `yres`
properties. For VMware, default display-service logs confirm the active
framebuffer. Full GOP alternate-mode dumps require a safer bootloader/protocol
path before they can be enabled by default.

System Preferences now exposes manual modes only when the active backend reports
safe preview support through the display-mode capability API.

- VMware SVGA II: validated manual modes, transactional preview, confirmation,
  timeout/explicit rollback, and confirmed-mode persistence
- VirtIO GPU: automatically managed and read-only in the UI; existing host
  scanout behavior is unchanged
- Limine framebuffer: current mode is visible but runtime changes are read-only

## Backend-neutral mode API

`ipc/src/display_modes.rs` defines the shared model used by the display service
and System Preferences:

- current mode: width, height, bpp, pitch when known
- available modes with current/recommended flags
- management state: manual, automatic, or read-only
- a concise read-only reason
- preview transaction token, applied readback mode, and system deadline

The Monitor page uses only these capability fields to enable or disable manual
selection. Backend labels are presentation text, not feature switches.

## Policy

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

Selection requires a fully usable backend, in this order:

1. VirtIO GPU after resource creation and backing attachment succeed.
2. VMware SVGA II after kernel activation and framebuffer-layout validation.
3. The original Limine framebuffer as the explicit immutable fallback.

The Limine descriptor is authoritative and read-only: width, height, pitch,
bits per pixel, RGB mask layout, mapped address, and mapped length are adopted
together. `pitch * height` is checked for overflow and must fit the mapped
length; unsupported channel layouts are rejected rather than treated as generic
32-bpp XRGB. User aliases inherit the complete Limine page-cache selection,
including the 4 KiB PAT leaf bit used for write-combining, so the TTY and
display service do not create a conflicting write-back alias of firmware
framebuffer memory.
Limine never reads `display.vmware.mode`, starts a preview transaction, invokes
SVGA modesetting, or treats a Recommended UI entry as a boot-time request.
While the TTY login owns Limine, `sunlight-display` publishes capabilities but
defers allocating and painting its compositor buffer until `SESSION_ACTIVATE`.

## Physical Limine Cache-Alias Regression

The physical-laptop regression fixed on July 20, 2026 presented as a partially
painted login wallpaper, the old splash footer (`Status: OK`) remaining visible,
and a missing login form. The login service and input path were still alive; the
login renderer itself was not the defect.

The first broken boundary was the user mapping of the Limine framebuffer. The
kernel splash used Limine's original write-combining mapping, but the TTY and
display-service aliases were created through the generic user-page mapper. That
mapper preserved PCD/PWT but not the 4 KiB leaf PAT bit, so the same physical
framebuffer was accessed through both WC and WB mappings. Real hardware could
retain or reorder those writes, while QEMU happened to tolerate the conflicting
aliases. This explains why different boots exposed different amounts of the
login background without proving a login race or framebuffer bounds failure.

On x86_64, Limine's mapping selects PAT entry 5 for WC with PWT plus the leaf
PAT bit. At the P1/4 KiB level, bit 7 is PAT, not the huge-page flag. Every
virtual alias of the same framebuffer physical range must reproduce the
original cache type; do not force WB and do not replace WC with UC merely to
avoid the alias conflict. Preserve PCD, PWT, and the leaf PAT selection
together.

When diagnosing a similar physical-only failure:

1. Record the Limine physical base, virtual base, width, height, pitch, bpp,
   pixel masks, page offset, and mapped length.
2. Compute `required_len = pitch.checked_mul(height)`; never substitute
   `width * height * 4`.
3. Include the physical page offset when sizing the page mapping and verify the
   returned visible mapping still covers `required_len`.
4. Compare the kernel/HHDM framebuffer cache attributes with every TTY and
   display-service alias.
5. Require `[BOOT-DISPLAY] tty mapping cache=wc ...` and
   `[DISPLAY-LIMINE] mapping cache=wc ...` before treating the mapping as valid.
6. Confirm the Limine backend remains immutable, retains firmware geometry, and
   performs no runtime mode request or VMware preference lookup.
7. Confirm `[LOGIN-TUI] background complete`, `status bar complete`,
   `form complete`, `first frame complete`, and `input ready`.

The physical repair must be tested independently of VMware and VirtIO. Build
with `./runs.sh --build`, boot the resulting `target/sunlightos.iso`, record the
actual Limine geometry, type in the login fields, complete login, test Ctrl+F1
and Ctrl+F2, leave the system running to catch late display-service faults, and
repeat cold and warm boots. The July 20, 2026 laptop test completed this
procedure successfully: the complete login TUI became visible and usable.

The scanout resource is created with `VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM`
(memory bytes B,G,R,X — a little-endian u32 holding `0x00RRGGBB`, matching the
compositor back buffer). It was previously `X8R8G8B8_UNORM`, which swapped the
red/green channels and dropped blue entirely, producing a green-tinted screen.

## Cursor

The compositor always uses a software cursor: 32×32 TGA sprites
(`assets/cursors/*.tga`, generated from `docs/images/cursors/*.svg` by
`tools/gen_cursors.sh`) are alpha-blended into the back buffer with a
per-shape hotspot, so the cursor is visible on both the Limine and VirtIO
backends and appears in screendumps.

The VirtIO hardware-cursor plane (`UPDATE_CURSOR`/`MOVE_CURSOR`) is
deliberately not activated: QEMU UIs map that sprite onto the host pointer,
which is hidden while a relative-pointer (PS/2) grab is active, leaving no
visible cursor. The upload path (`upload_hw_cursor_if_needed`) is kept
functional for future absolute-pointer (virtio-tablet) setups.

To change a cursor: edit the SVG in `docs/images/cursors/`, run
`./tools/gen_cursors.sh`, and rebuild `sunlight-display`. Hotspots live in
`cursor_asset()` in `services/sunlight-display/src/main.rs`.

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

For a QEMU Limine-only regression boot:

```bash
./runs.sh --limine-only
```

Expected serial boundaries are `VirtIO GPU not present`, `VMware SVGA II ...
not present`, `display_backend=Limine`, `[LOGIN-TUI] first frame complete`, and
`[LOGIN-TUI] input ready`. The display service must not acquire framebuffer
ownership or clear/present it before an authenticated desktop activation.

VMware and VirtualBox launcher paths do not yet negotiate guest display modes
from this repo. On VMware, use the boot logs to confirm whether the firmware
exposed `1366x768`, `1280x800`, or `1280x720`.

## Scope And Limitations

- Physical framebuffer geometry remains firmware-authoritative.
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
- `sunlight-control-panel` capability-driven Monitor page
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
| Runtime VMware mode switching | Supported through preview/confirm/revert |
| VirtIO manual mode switching | Not supported; automatically managed |
| Limine runtime mode switching | Not supported; read-only |
| HiDPI / per-monitor scale | Not supported (`scale_fp` placeholder only) |
| Multi-monitor | Not supported |

CLI: `display-status` (via `sunlight-utils`) prints current metrics when the
desktop session is active.

## Verification sequence

Build with `./runs.sh --build`, then verify:

1. VMware: 1024×768 → 1280×720 → 1440×900 → 1024×768
2. readback pitch and mapped-byte diagnostics after every transition
3. Keep, explicit Revert, timeout rollback, and Control Panel termination
4. confirmed-mode reboot restore
5. window dragging, maximize/restore, menus, wallpaper, panels, dock, desktop
   icons, cursor edges, and newly exposed regions
6. repeated switching at least five times
7. QEMU VirtIO automatic sizing and Limine fallback

Compilation alone is not runtime proof. VMware serial output and `vmware.log`
must be retained for the final validation report.

## Future Work

- VMware host-window Auto-fit through guest integration
- HiDPI/Retina-style logical scaling and dynamic DPI
- multiple monitors
- refresh-rate selection
- hardware cursor and advanced acceleration
- Add dynamic resize for VirtIO GPU scanout changes.
- EDID / full mode enumeration when the driver path is safe.
- HiDPI scaling and per-monitor scale factors.
