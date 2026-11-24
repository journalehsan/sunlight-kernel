# Helios Canvas SunApp pointer replay

Build and run with `bash tools/test_helios_canvas_sunapp.sh --input tcg`.
The script builds and stamps a separate interactive ELF, copies it byte for
byte into the existing manifest bundle, and runs the same SunApp launcher in
the guest. A successful launch resolves the validated manifest entry
`/Applications/SunlightCanvas.sunapp/Program/sunlight-ui-wayland`. Build and
install commands are in `client-build.log` and `build-install.log`, next to
the ISO. The replay directory contains `command.json`, `serial.log`, the
three PNG captures, `result.json` (input and frame checksums), and
`SHA256SUMS` (ELF, bundle files, ISO, and captures).

Before this change the bridge advertised only `wl_compositor`, `wl_shm`, and
`xdg_wm_base`. It already generated configure serial 1, checked its ack,
released copied buffers, completed frame callbacks, and dropped the pool
mapping and SGP window on disconnect. It had no seat, pointer object, pointer
focus, or enter, leave, motion, or button events.

The `pointer-fixture` bridge build advertises one version 1 seat with only
pointer capability and accepts one `get_pointer`. After frame one has been
released and its callback completed, the client holds the frame for the
existing five-second capture interval. It then sends a display sync, which
the guest bridge uses once as the fixture trigger. The sync carries no input
coordinates or surface ID. The bridge targets the mapped Wayland surface on
that connection and emits enter, left press, and left release. QMP captures
scanout and handles the existing login steps; it sends no pointer input.

| Item | Contract |
| --- | --- |
| Surface and buffer | 256 × 160, scale 1; logical pixels map one to one to buffer pixels. |
| Buffer storage | Two XRGB8888 slots in the existing pool, stride 1,024 bytes; wait for release and frame callback for each commit. |
| Pointer coordinates | Signed Wayland 24.8 fixed point, surface local. Reject negative values and values at or beyond the surface dimensions before dropping the fractional bits. |
| Hit rectangle | `24 <= x < 128`, `96 <= y < 128`; the fixture targets `(64,112)`, encoded as `(16384,28672)`. |
| Focus and serials | Enter must name the demo surface and carry serial 2. Only button serial 3 is accepted; release serial 4 is ignored. These serial values are specific to this one-shot fixture. |
| Button and state | Linux `BTN_LEFT` (272), pressed (1); release (0), other button codes, and other states do not update the counter. |

The client invalidates the stored position on leave, motion, or loss of the
pointer capability. The fixture does not generate motion or transfer focus;
disconnect ends focus and drops the connection's objects. The Rust backend
translates the accepted press to Sunlight UI `Event::MouseDown` with button 0,
hit-tests it, and changes the counter from zero to one once. Existing Canvas
bitmap text and a color change show the result in the same SGP window.

The non-interactive control is independent. Replay the retained original ISO
with:

```sh
python3 tools/helios_wayland_gate.py target/helios-sunapp-sunlight-ui/guest.iso \
    target/helios-sunapp-sunlight-ui/tcg-input-control tcg canvas-sunapp
```

To rebuild the original control with its pinned ELF, run
`bash tools/test_helios_canvas_sunapp.sh tcg`. Neither mode changes the
launcher waitpid assertion or its timeout.

The two-vCPU TCG and KVM replays of the interactive ISO both passed. In both,
Wayland surface 3 received the press, and SGP window 2 redrew at (541, 180).
Client and bridge checksums matched for buffer 12 (`5545147f`, count 0) and
buffer 13 (`0f539927`, count 1). Both commits released their buffer and
completed their frame callback. The child exited with code 0, waitpid returned
`Ok(0)`, and the compositor removed window 2 explicitly. The final captures
contain neither Canvas frame. Evidence is in
`target/helios-sunapp-sunlight-ui-input/{tcg,kvm}-replay/`.

The first TCG attempt reached the click and passed waitpid but missed the
explicit close marker; its log shows the display's owner-exit reaper instead.
That attempt did not reach the final capture. The interactive gate records
which cleanup path runs and checks that the same window is gone from scanout.
The first KVM attempt reached explicit removal, but the client split the hit
record across serial writes. Those attempts are preserved in
`tcg-first-cleanup/`, `tcg-explicit-close-missed/`, and `kvm-first-capture/`.

| Check | Rerun status |
| --- | --- |
| Native listener gate | Not rerun standalone. |
| Multipage memfd pool gate | Not rerun standalone; both interactive replays used the two-slot pool. |
| SCM_RIGHTS gate | Not rerun standalone; both interactive replays transferred the pool FD. |
| Global-page mmap gate | Not rerun standalone; both interactive replays mapped and rendered the pool. |
| Standalone Wayland fixture | Not rerun. |
| Standalone libwayland probe | Not rerun. |
| Direct Canvas | Not rerun. |
| Non-interactive Canvas SunApp | Retained ISO passed TCG in `tcg-input-control-retry/` with original checksums `4fbc9f29` and `a5734799`. The earlier `tcg-input-control/` attempt timed out at registry dispatch. |
| Interactive Canvas SunApp | Passed TCG and KVM. |
