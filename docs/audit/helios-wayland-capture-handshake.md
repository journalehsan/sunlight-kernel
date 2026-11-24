# Helios Wayland capture handshake

Before this change, the initial bufferless commit triggered xdg configure,
which the clients acknowledged before attaching a rendering buffer. For each
of two frames, the fixture logged the numbered buffer commit and checksum,
then submitted the SGP commit. The display service logged a redraw with the
window ID. The clients consumed the buffer release and frame callback, then
held for five seconds. The host counted redraws for the same ID and polled QMP
`screendump` for the corresponding frame pattern. The delay did not prove that
the host had captured anything: its expiry permitted frame 2 or disconnect,
respectively. After socket EOF, the fixture closed its SGP window. The host
waited for removal of that window ID, then polled for changed pixels and
absence of both fixture patterns. Neither the initial nor final capture was
accepted on elapsed time alone.

The four visual clients now wait after each buffer release and frame callback.
The client commits frame 2 only on token 1; it closes its socket only on token
2. It waits for token 3 before exiting successfully. The host sends token 3
after observing peer EOF, removal of the matching SGP window, and the changed
scanout. Each acknowledgement watchdog fails after 25 guest monotonic seconds.
The raw client also bounds its configure and frame I/O; libwayland uses its
existing bounded dispatch. The original raw client's short FD fragmentation
delays remain in the transport fixture setup.

Tokens go over a per-VM QEMU serial socket; a debug Wayland test kernel exposes
one nonblocking UART receive operation to the static test clients. The bridge
does not consume tokens. Standard Wayland configure, attach, commit, release,
callback and disconnect still drive rendering and window lifetime. The
separate pointer fixture keeps its existing behavior.

The host records numbered transitions with client, run, accelerator, window
ID, commit sequence and ISO digest. Before acknowledging a frame it correlates
the commit checksum, SGP redraw and client wait marker, checks the 1280 × 800
scanout and selected 256 × 160 crop against hashes from the prior successful
controls, and decodes the saved PNG to verify its pixels. It verifies the
removed-window crop independently and requires both frame patterns to be
absent. The evidence directory also records PNG checksums and the build's
artifact manifest; the kernel digest identifies the embedded initramfs.

Set `HELIOS_WAYLAND_STALL_CAPTURE=1` on a visual gate run to withhold the first
capture and token. The expected outcome is a failed gate with an observed client
capture-ack timeout, one commit/redraw, no frame 2 or first acknowledgement,
and removal of the matching window. This is recorded separately from passing
matrix runs.

New evidence: `target/helios-wayland-capture-handshake/`. Earlier controls and
failures remain under `target/helios-wayland-baseline-fix/`. Transport is outside
this matrix. The historical KVM waitpid issue remains separate; the Canvas
SunApp waitpid assertion and timeout are unchanged.

All 12 two-vCPU matrix slots passed under `verified/`: initial TCG, same-image
TCG replay, and KVM for each client. For SunApp, use `tcg-initial-retry/`:
the first attempt is retained and shows SGP removal before the first commit.
The evidence does not establish why that window was removed. Image digests
match across all three passing runs for each client.

The TCG stall under `wire/final/stall-tcg/` failed as intended: frame 1 timed
out awaiting acknowledgement, frame 2 never committed, and SGP removed the
matching window. An earlier KVM stall attempt timed out during boot and did
not exercise acknowledgement. KVM was available and passed all four regular
matrix slots.
