# Audio capture phase 1

## Architecture audit

Playback uses QEMU `intel-hda` with the `hda-output` codec, 48 kHz S16LE
stereo, and twelve 4096-byte DMA periods (1024 frames each). `audiod` owns the
HDA BAR, CORB/RIRB command rings, DMA allocation, software volume and the
bounded playback queue. The kernel only discovers PCI, enables bus mastering,
and grants MMIO and contiguous DMA to audiod. The service polls LPIB every
8 ms and acknowledges stream status; it does not wait for IRQs. Playback PCM
arrives through a one-page shared memory IPC request, is copied into audiod's
queue, and the sender's mapping is released before the reply. The client
reuses its page only after acknowledgement. Playback is sender scoped, but
`audiod` currently has one application output stream.

The Sound page previously showed output device, master volume/mute, system
sounds, and test tone. It refreshes on a bounded Tick. The existing
`audio.v1` endpoint is available to ordinary user sessions, so microphone
operations use a distinct `audiod.capture` endpoint requiring the new
`Microphone` service capability. Only root `audioctl` and the Control Panel
spawn profile (including ordinary desktop users) receive that capability in
this phase. Playback access alone
does not authorize microphone lookup. The kernel still validates endpoint
tokens and shared memory capabilities.

## Device and format

The QEMU runner and audio boot gate now attach `hda-micro` to the existing
Intel HDA controller. This codec preserves output and adds an ADC and input
pin. `sunlight-audio` discovers those widgets from the audio function group,
programs the ADC to stream tag 2, enables the input pin, and uses the first
HDA input stream descriptor. The QEMU codec's input amplifier is set to its
advertised 0 dB index; leaving its default index at zero produces silence.
Output remains on its existing stream tag 1.
Capture's only offered format is 48,000 Hz, signed 16-bit little-endian,
interleaved stereo. An open request for another format is rejected.

The single audiod DMA grant now covers one control page, twelve output periods,
and twelve separate input periods (25 pages total). The control page contains
distinct output and input BDLs. The kernel's audiod grant limit is 32 pages.
If the 25-page contiguous grant fails, audiod retries the original 13-page
output grant and reports input unavailable while keeping playback usable.
The two hardware streams are started and stopped separately. HDA input is
polled for completed periods, and the driver never hands the currently active
DMA period to another layer. A full unobserved DMA ring lap cannot be inferred
from LPIB alone; HDA FIFO errors are counted as lost periods.

QEMU's codec layout follows its [HDA codec source](https://github.com/qemu/qemu/blob/master/hw/audio/hda-codec.c).

## Ownership and API

The capture service accepts one owner PID at a time. Its states are Closed,
Open, and Running. `CAPTURE_OPEN` selects the fixed PCM format and optional
monitor-only mode. `CAPTURE_START`, `CAPTURE_READ`, `CAPTURE_STOP`,
`CAPTURE_CLOSE`, `CAPTURE_STATUS`, and `INPUT_DEVICE` are on the restricted
endpoint. Duplicate open/start and read before start fail; stop while open is
safe. Closing stops DMA and discards pending periods. A five-second idle lease
releases a vanished client; service restart also destroys all process-owned
DMA and IPC state.

For recording, completed DMA periods are copied once into an eight-period
FIFO (32 KiB). If it fills, the oldest unread period is dropped and counted.
The client supplies its own one-page shared memory mapping for `CAPTURE_READ`.
Audiod copies at most one period to that page, unmaps its view, and then
replies. The client reuses the page only after the reply; a failed or timed
out request retires the page. This keeps DMA, audiod queue, and client buffer
ownership distinct. `CaptureClient` exposes open, start, read, status, stop,
close, and input availability. Monitor-only mode computes a recent peak
without queueing PCM. The Sound page uses that mode for its level meter and
does not claim hardware gain or mute controls.

`audioctl capture` is the diagnostic. It captures for three seconds and prints
format, received frames, completed periods, dropped periods, sample min/max,
and recent peak. It returns failure if no frames or only zero samples arrive.
It always stops and closes on its normal and read-error paths.

## Validation and limits

Host tests cover the existing playback ring, input period completion,
capture FIFO overflow and peak, shared page timeout ownership, malformed
open requests, four-word IPC reply shape, and microphone capability
separation. The native audiod and Control Panel checks and kernel check pass.
The `./tools/test.sh audio` QEMU playback gate passes with `hda-micro`.

An interactive QEMU/KVM run connected `hda-micro` to a dedicated PipeWire
monitor source. FFmpeg generated a 440 Hz signal into that source; a direct
host capture measured −4095 to +4095 S16 samples. `audioctl capture` in the
guest then received 144,384 stereo frames in 141 completed periods over its
three-second window, with min −4095, max +4094, and zero dropped periods.
It exited successfully. A later run measured min −6754, max +6735 and a
nonzero observed peak of 6754, which exercises the same peak calculation
used by the Sound page meter. A second capture after `audioctl test` also received
144,384 frames, zero drops, and nonzero samples. The audiod serial log showed
the playback tone start and completion between the two capture runs. Both
capture commands stopped and exited, and the second open succeeded, which
exercises device release and restart. No long-duration memory-growth test or
graphical level-meter observation has been completed. With `-audiodev none`,
the codec may be detected but its input is silence.

Each period is 1024 frames (about 21.3 ms), the audiod queue holds at most
eight periods (about 171 ms), and the input DMA ring spans about 256 ms.
These are buffering bounds, not measured end-to-end latency.

There is no hardware gain/mute, resampler, WAV writer, application recorder,
IRQ-driven capture, or multi-client capture in this phase. The next phase
should add a simple Voice Recorder using `CaptureClient`, stream to a bounded
PCM WAV file, and test playback of that file in Melody Mina.
