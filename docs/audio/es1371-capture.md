# ES1371 capture support

SunlightOS now uses the ES1371 ADC alongside the existing Playback 2 engine.
The driver allocates one contiguous low-memory DMA region containing a 48 KiB
playback ring and a 48 KiB capture ring. ADC periods are copied only after the
hardware cursor crosses a complete 4096-byte period, so audiod never reads a
partially written or stale period.

The capture path negotiates 48,000 Hz, signed 16-bit stereo PCM, matching the
existing `audiod.capture` protocol and Recorder application. AC'97 record
select supports microphone and line-in; both channels are programmed to the
selected source. The ES1371 ADC sample-rate converter is configured lazily on
capture start so opening the playback device does not alter the working DAC2
setup. Capture gain and mute are applied through AC'97 record gain, and the
capture IPC extension exposes source, gain, and mute controls to clients that
need them. HDA clients continue to receive an unsupported-control response.

The service keeps the established single-owner capture lease, bounded
eight-period audiod queue, shared-page read protocol, Recorder lifecycle, and
microphone capability check. ES1371 playback and capture use separate control
bits, serial-interface bits, register pages, and DMA offsets; playback polling
restores page `0x0c` before reading DAC2 while capture polling uses page `0x0d`.

Host validation covers ADC register constants, AC'97 source mapping, period
wraparound, playback interrupt-latch acknowledgement, and I/O failure paths.
Freestanding package checks pass for `sunlight-audio`, `sunlight-audiod`, and
the kernel. Actual VMware microphone recording, intelligibility, and sustained
full-duplex playback still require a VMware run with a connected virtual
microphone; this repository change does not claim that hardware result.
