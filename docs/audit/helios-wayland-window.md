# Helios Wayland window fixture

Execution path: a freestanding static x86-64 Linux ELF stamped with HLNX01
connects to a debug-only native service using the official UID-scoped
listener. The service accepts one client and advertises version 1 of
wl_compositor, wl_shm and xdg_wm_base. The client discovers and binds those
globals, creates wl_surface, xdg_surface and xdg_toplevel, commits with no
buffer, receives toplevel and surface configure, and acknowledges the
surface serial before attaching. It transfers a multipage memfd using
SCM_RIGHTS and creates XRGB8888 buffers. No global test page participates.

The existing native service build uses freestanding no_std Rust; the
existing display event loop services SGP IPC. There is no supported native
Wayland backend with general recvmsg or a demonstrated Smithay guest
build. This fixture therefore uses a bounded, standalone wire dispatcher
and the native sunlight-ui Window API. sunlight-display continues to own
the window, focus and scanout. This is not a public compositor.

Ordinary stream reads stop before the ancillary mark. The dispatcher
keeps a message cursor and associates the received FD with the enclosing
create_pool request, including marks within its body or header. This
fixture requires that the mark occur within create_pool; it does not
dispatch FD arguments spanning groups of unrelated messages. The kernel
allows one outstanding one-FD record per stream and 1–16 marked payload
bytes. The client sends a prefix, one marked byte inside the header, then
the rest. The mapping syscall verifies the declared pool range against
the backing length; buffer range checks then use the validated view.

The first valid attach creates the SGP window. Both commits copy pixels
before releasing the wl_buffer, and callback.done allows the client to
advance. Disconnect drops the window and unmaps the pool.

## Guest evidence

Build and boot with `bash tools/test_helios_wayland.sh kvm tcg`. The script
builds and stamps the freestanding client, builds the native server and
display with the userspace linker flags, builds the debug kernel with
`--features helios_wayland_test`, and packages it with
`tools/make_hybrid_iso.sh`. It assumes the other embedded services from a
normal SunlightOS build are present. The gate uses UEFI, standard VGA,
4 GiB RAM, two vCPUs, and a diskless desktop login via QMP keys. It records
the exact QEMU command in each run's `command.json`.

The passing guests used the same `target/helios-wayland/guest.iso`. After
adjusting the capture gate to wait for the removal redraw, TCG was replayed
with:

```sh
python3 tools/helios_wayland_gate.py target/helios-wayland/guest.iso target/helios-wayland/tcg tcg
```

| Accelerator | Window ID | Frame 1 | Frame 2 | Disconnect |
| --- | ---: | --- | --- | --- |
| KVM | 2 | Buffer 11, `f2d6fd05`, blue vertical stripes | Buffer 12, `21373585`, amber/red horizontal stripes | Window 2 removed |
| TCG | 2 | Buffer 11, `f2d6fd05`, blue vertical stripes | Buffer 12, `21373585`, amber/red horizontal stripes | Window 2 removed |

Both runs show a white center rectangle and the same scanout origin
`(541, 180)` for both frames. The gate checks interior pixels against the
expected patterns, skipping the rounded edges. It captures the guest
framebuffer again after the display removes the window and checks that
neither pattern survives. The server logs configure/ack serial 1, a
170509-byte pool with the FD at request byte 5, and rejection of buffer 10
before the two valid submissions. Both frame hashes are logged at
`wl_surface.commit` and correlate in order with two display redraws for
window 2. The client waits for release and callback.done before advancing.

In `target/helios-wayland/{kvm,tcg}/`, see `serial.log`, `frame-1.png`,
`frame-2.png`, `after-disconnect.png`, and `result.json`. An earlier TCG
capture taken after the service close marker but before the display
redraw is retained under `tcg-close-race/`; the passing replay waits for
the display marker.

## Separate transport baseline

The existing transport gates were not rerun for this window change.
The retained evidence in [the pool audit](helios-fd-shm-pool.md#guest-runs)
records native listener, one-FD SCM_RIGHTS, and multipage pool passes
on KVM and TCG. The original global-page gate passed on TCG and failed at
the final `waitpid` on KVM with `no-child`
(`target/mmap-regression-kvm/serial.log`). That issue remains open; its
assertion and timeout are unchanged.

## Independent libwayland client

Weston 14.0's `simple-shm` target uses Weston's shared helper library,
whose build also brings in Pixman, EGL and libweston dependencies. A Weston
guest binary was not built. The smaller probe uses unmodified libwayland
1.26.0, libffi 3.4.8, static musl, and `wayland-scanner` output for stable
xdg-shell. It connects to the existing UID-scoped socket and hands the FD
to `wl_display_connect_to_fd`; all protocol requests go through the library.

With an x86-64 musl toolchain, a matching host Wayland scanner, the
wayland-protocols XML, Meson, Ninja, and the normal guest build prerequisites:

```sh
bash tools/test_helios_wayland.sh kvm tcg
bash tools/test_helios_libwayland.sh kvm tcg
```

Each boot directory has `command.json` and `serial.log`. The independent
client enters main and connects on KVM, then exits with
`[WL-CLIENT] FAIL registry (errno=95)`. The bridge accepts the connection
and logs `[WL-FIXTURE] FAIL EOF before two frames`. The first run is under
`target/helios-wayland-lib/first-kvm/`. Build logs are in that run's parent
directory and its `build/` subdirectory.

The first failure was transport: the registry roundtrip flush calls
`sendmsg(MSG_NOSIGNAL | MSG_DONTWAIT)` with zero ancillary bytes.
`sys_linux_sendmsg` returned `EOPNOTSUPP` for that shape. The transport now
routes control-free sends and byte-only replies on Linux client endpoints
of native Unix listeners through the existing stream I/O paths. It does
not change the single-FD send path. The probe flushes `create_pool` alone,
so libwayland marks byte zero of that request.

The bridge now handles the initial registry sync and tracks the client
object IDs it accepts, including reuse after callback deletion. The probe
uses the library's timed dispatcher: the earlier untimed roundtrip hit a
poll assertion, and an experimental infinite-poll change did not make it
reliable. No poll change is included.

KVM and TCG both captured window 2 at (541, 180), with frame hashes
`f2d6fd05` and `21373585`, followed by removal on disconnect. The logs
confirm configure/ack serial 1, the 170509-byte pool, and both buffer
releases and frame callbacks. See `target/helios-wayland-lib/kvm/` and
`target/helios-wayland-lib/tcg-replay/` for `command.json`, `serial.log`,
`result.json`, and all three PNGs per run. The first timed TCG run closed
the window during setup; the same ISO passed on replay:

```sh
python3 tools/helios_wayland_gate.py target/helios-wayland-lib/guest.iso \
    target/helios-wayland-lib/tcg-replay tcg libwayland
```

The original fixture also passed against the updated bridge on TCG and
on a KVM replay after the first KVM boot timed out before login. See
`target/helios-wayland/{kvm-replay,tcg}/`.

Standalone one-FD SCM_RIGHTS and multipage pool transfer/reclaim passed
on KVM and TCG. Separately, the original global-page mmap gate passed
through `waitpid` and the final native read on both accelerators. These
runs did not reproduce the historical KVM `no-child` failure; they do not
establish that it was fixed. See
`target/helios-wayland-lib/regressions/` for the build and serial logs.
The standalone native listener gate was not rerun.
