# Sunlight UI Canvas over Helios Wayland

Run the build, debug ISO bundle, and capture gate with:

```sh
bash tools/test_helios_libwayland.sh --canvas kvm tcg
```

The script builds `sunlight-ui-wayland` for `x86_64-unknown-linux-musl`,
links it into the existing static musl/libwayland probe, stamps the executable
with the Helios ELF marker, and bundles it and the native bridge into the
`helios_wayland_test` debug ISO. The debug guest starts the client
automatically. The script without `--canvas` still builds the stripe probe.
The native Sunlight UI runner and native Wayland fixture are unchanged.

To replay the same ISO without rebuilding:

```sh
python3 tools/helios_wayland_gate.py target/helios-wayland-lib/canvas/guest.iso \
    target/helios-wayland-lib/canvas/tcg-replay tcg canvas
```

The client binds only `wl_compositor`, `wl_shm`, and `xdg_wm_base` at
version 1. It creates one toplevel, commits without a buffer, receives the
initial configure, and flushes its acknowledgment before attaching.
As in the original probe it flushes `create_pool` independently so the FD
mark lands on byte zero of that request. The client uses the existing
100 ms timed dispatcher in place of the unreliable untimed poll; each
registry, configure, or frame wait allows at most 150 dispatch calls.
After each frame it also holds for capture for five seconds.

The 331,776-byte shared pool contains two nonoverlapping 256×160 buffers at
offsets 4,096 and 167,936. Both use 1,024 bytes per row, passed to Canvas
as 256 `u32` pixels per row. Canvas draws a solid background, bitmap text,
and progress bar directly into each mapped slice. It checks an asymmetric
accent pixel's native bytes against the expected little-endian B, G, R, FF
sequence and checks every pixel has an opaque high byte. The bridge accepts
`WL_SHM_FORMAT_XRGB8888`, reads little-endian B, G, R, X, and sets alpha to
FF for SGP. No `ARGB8888` or alpha blending compatibility is assumed.
Each buffer tracks its own release and the client waits for release and
frame callback before advancing. The two-frame run uses each slot once.

The TCG replay captured both frames at (541, 180). Client Canvas checksums
matched the bridge for buffers 10 and 11: `4fbc9f29` and `a5734799`.
The capture gate checks the progress bar, accent rule, background, and
text ink on scanout. The client exited with code 0, the bridge completed
disconnect cleanup, and the display removed window 2. The final capture
contains neither frame. `serial.log`, `command.json`, `result.json`,
`frame-1.png`, `frame-2.png`, and `after-disconnect.png` are under
`target/helios-wayland-lib/canvas/tcg-replay/`. KVM could not start on
this host because `/dev/kvm` is unavailable.

The existing multipage pool, single-FD SCM_RIGHTS, and global-page mmap
gates all passed on TCG. Their logs are in `regression-pool/`,
`regression-scm-rights/`, and `regression-mmap/` under the same Canvas
output directory. The mmap gate reached the final native read after
Helios exit. The historical KVM `waitpid` failure (`no-child`) remains
unresolved; this TCG result does not establish a fix. The standalone
native listener gate was not rerun.
