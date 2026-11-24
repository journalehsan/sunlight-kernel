# Helios Canvas SunApp

Run `bash tools/test_helios_canvas_sunapp.sh tcg`. The script stages a
two-file bundle in `target/helios-sunapp-sunlight-ui/SunlightCanvas.sunapp`,
builds the native debug services and kernel, installs the bundle through the
static initramfs, makes the hybrid ISO, and boots QEMU with two vCPUs.
`build-install.log` records build and install commands. Each run directory
contains the QEMU command, serial output, and any captured scanout.

The input is `target/helios-wayland-lib/canvas/client.elf` (299,432 bytes,
SHA-256 `577ac203166af6171f5ae2789a4a9da9d39ae40dc6d801188288703a8995ba9e`).
Its entire byte sequence occurs at offset 281787126 in the retained direct
Canvas ISO. It is a static little-endian x86-64 ELF64 executable with neither
PT_INTERP nor a dynamic section. The script pins that digest and copies the
ELF without relinking or stamping it again. It needs no resource files.

The installed command is `/Applications/SunlightCanvas.sunapp`.
`canvas-sunapp-launcher` is a native debug driver that waits for the display
service and calls `sun_exec::launch_with_environment`. The old debug direct
client is omitted from this ISO. The native bridge executable and Wayland
client source are unchanged.

The existing format-1 manifest reader in `sun_exec.rs` looks up TOML sections
and keys with a bounded parser. Chronos bundles use their own runtime and
entry fields. The previous Helios path required `kind = "terminal"` and
wrapped the program in Sunlight Terminal for display launches. The new
`kind = "gui"` uses the same validated Helios entry and starts it directly
when display access is requested. The demo registration is limited to the
debug harness. There is no new manifest syntax, resource lookup, or installer.

The registered bundle root, manifest, Program directory, and executable
must be root owned and must not be writable by group or others. The relative
entry is restricted to one component below Program. The launcher checks
execute permissions and reads the header with O_NOFOLLOW, requiring ELF64
and the `HLNX01` marker. A launch error is returned as `LaunchError` and the
driver logs it. On success the resolved executable and sole argv element
are both `/Applications/SunlightCanvas.sunapp/Program/sunlight-ui-wayland`.
The existing native spawn syscall inherits cwd, UID, GID and capabilities
from the parent, then loads the file using the Helios ELF path. The driver
waits for that child with the existing waitpid wrapper.

The bridge is a guest native Unix listener named
`/sunlight/native-unix/wayland-fixture`. The fixed ELF opens that address
itself and calls `wl_display_connect_to_fd`. The bridge waits for the graphical
desktop before serving the connection. There is no host socket, passed socket
descriptor, or alternate route around kernel authorization. The caller
chooses an empty child environment at launch: the existing ELF consumes
neither XDG_RUNTIME_DIR nor WAYLAND_DISPLAY, and needs no HOME, PATH, locale,
or resource setting. This fixture uses UID 0 for both its listener policy and
the launching process; it does not establish production session discovery
or non-root desktop launch. The listener authorization path checks the
connecting UID before allocating a connected socket pair.

The first guest attempt reached validated program resolution and failed at
**ELF execution**: native spawn returned `SpawnFailed(Failed)` before loading
the image because the boot driver had neither a system allocation nor account
membership. The debug boot code now gives that driver the existing system
allocation used by boot services. The normal spawn admission check, UID rule,
and listener policy are unchanged. The failing serial log is preserved in
`target/helios-sunapp-sunlight-ui/tcg-first-launch/`. An earlier sandboxed
attempt could not bind the host QMP socket and never booted.

The corrected ISO passed on a two-vCPU TCG replay:

```sh
python3 tools/helios_wayland_gate.py target/helios-sunapp-sunlight-ui/guest.iso \
    target/helios-sunapp-sunlight-ui/tcg-replay tcg canvas-sunapp
```

PID 742 inherited UID/GID 0, cwd `/`, and zero IPC capability entries.
The kernel logged argc 1 and envc 0. The process discovered
`wl_compositor`, `wl_shm`, and `xdg_wm_base`, and acknowledged configure
serial 1. The 331,776-byte pool held two 256×160 XRGB8888 buffers at
stride 1024, used once each. Both release and frame callback completed
before advancing. Window 2 drew frame 1 at (541, 180) with buffer 10 and
checksum `4fbc9f29`, then drew frame 2 at the same location with buffer 11
and checksum `a5734799`. The capture gate found the expected Canvas
background, accents, text and progress bar in both images, then found
neither frame after the bridge removed window 2. Kernel exit and parent
waitpid both reported 0. See `serial.log`, the three PNGs, `result.json`,
`command.json`, and `SHA256SUMS` in the replay directory.

The retained direct TCG replay under
`target/helios-wayland-lib/canvas/tcg-replay/` is the control; the direct
build command and capture assertions remain available. QEMU TCG here is
ordinary two-vCPU emulation; replay means booting a retained ISO again.
KVM is unavailable because `/dev/kvm` does not exist.

| Check | Status for this milestone |
| --- | --- |
| Host launcher tests | 10 passed, including both Helios manifest kinds. |
| Multipage memfd pool | Not rerun; retained direct Canvas audit records a TCG pass. |
| SCM_RIGHTS | Not rerun; retained direct Canvas audit records a TCG pass. |
| Global-page mmap | Not rerun; retained direct Canvas audit records a TCG pass. |
| Native listener | Passed on the retained listener ISO in two-vCPU TCG; wrong UID received ECONNREFUSED and left the queue empty. This is a separate listener endpoint. |
| Original Wayland fixture | Standalone wire client not rerun; the native bridge served the SunApp run. |
| libwayland probe | Standalone stripe probe not rerun. |
| sunlight-ui Canvas Wayland | SunApp TCG replay passed; direct test not rerun, retained TCG replay passed. |
| Historical KVM waitpid no-child | Not rerun; KVM unavailable, issue remains unresolved. |
