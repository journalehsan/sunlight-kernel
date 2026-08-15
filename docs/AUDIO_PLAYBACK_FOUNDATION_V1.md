# Audio Playback Foundation v1

SunlightOS plays PCM from userspace. The kernel grants BAR and DMA
resources; `audiod` owns policy and the first hardware backend.

## Selected initial hardware

QEMU is already configured with Intel HD Audio:

```text
-audiodev pa,id=snd0
-device intel-hda
-device hda-output,audiodev=snd0
```

in `tools/runs.sh`. Automated gates use `-audiodev none` so CI does not
depend on host speakers.

| Item | Value |
| --- | --- |
| Controller | Intel HDA (ICH6-compatible) |
| Typical PCI ID | `8086:2668` (QEMU `intel-hda`) |
| Class | `04:03` (multimedia HD Audio) |
| BAR0 | MMIO |
| Codec | QEMU `hda-output` |
| PCM | 48 kHz, signed 16-bit, stereo |

AC'97 was not chosen because HDA is already the configured QEMU device.
The userspace protocol is not HDA-specific.

## Ownership model

```text
Applications (audioctl, Control Panel, Vortex)
        │  IPC  "audiod"  /  audio.v1
        ▼
     audiod
        │  master volume, mute, persistence, PCM queue
        │  software gain
        ▼
  sunlight-audio (generic types + Intel HDA driver)
        │  hda_info / map_mmio / dma_alloc
        ▼
     kernel grant
```

* Kernel: PCI discovery, bus-master enablement, uncached BAR mapping,
  physically contiguous DMA. No PCM policy.
* `audiod`: authoritative volume/mute, one output stream, client cleanup.
* GUI: presentation and user intent only.

This matches the existing USB-mouse userspace driver grant
(`XhciInfo` / `MapMmio` / `DmaAlloc`).

## DMA and interrupt model

* `DmaAlloc` returns a process-owned, physically contiguous region.
* Page 0 holds CORB/RIRB/BDL. Pages 1–4 hold four 4096-byte periods.
* Userspace never supplies a physical DMA address.
* Interrupts are acknowledged by polling `SDSTS` / `INTSTS`. There is
  no userspace IRQ wait yet; the service loop uses an 8 ms timeout.
* If the client exits or stops submitting, the driver plays silence.

## audiod protocol (`audio.v1`)

Registered name: `audiod`.

| Opcode | Meaning |
| --- | --- |
| `GET_STATUS` | state, volume, mute, format, underruns |
| `GET_DEVICE` | friendly device tag + PCI IDs |
| `GET_VOLUME` | same compact status |
| `SET_VOLUME` | 0..100 |
| `SET_MUTE` | 0/1, independent of level |
| `PLAY_TONE` | 440 Hz default, 1 s default |
| `SUBMIT_PCM` | SHM + bounded S16LE stereo |
| `STOP` | drop tone and queued PCM |

Invalid formats, oversized buffers, and missing hardware return typed
errors. The API never exposes AC'97/HDA registers.

## Volume and mute

* `0` is silent. `1..100` increases amplitude.
* Mute is independent. Setting volume while muted keeps mute.
* Unmute restores the last non-zero level when the slider is at zero.
* Effective output is `0` when muted or when volume is `0`.
* Default: 65%.

## Persistence

Same TOML-ish style as desktop wallpaper:

```text
/root/.config/sunlight/audio.toml
```

```toml
[audio]
master_volume = 65
muted = false
last_nonzero = 65
```

A missing or invalid file does not block playback.

## Control Panel

Page id `sound` (aliases: `audio`, `volume`).

```text
control-panel --page sound
```

Shows the output name, readiness, slider, mute, and Test Sound.
Test Sound calls `PLAY_TONE`; the page never writes hardware buffers.

Live updates use the existing bounded Tick refresh (not frame-rate
polling). There is no service event subscription yet.

## Vortex

A speaker icon sits immediately left of the network icon.

| Condition | Icon |
| --- | --- |
| unavailable | volume-off / disabled |
| muted or 0 | volume-off |
| 1..33 | volume-low |
| 34..66 | volume-medium |
| 67..100 | volume-high |

Click opens a popup (slider, output name, Sound Settings). Escape and
outside click close it. Sound Settings launches Control Panel on `sound`,
the same deep-link used by the network popup.

## QEMU

Interactive:

```bash
./tools/runs.sh
```

Device-init and generated test-tone gate (no host audio required):

```bash
./tools/test.sh audio
```

Manual audible test (PipeWire/Pulse host backend):

```text
audioctl test
```

The tester must hear a 440 Hz tone. This document does not claim that
host speakers were heard during implementation.

## Test commands

```bash
cargo test -p sunlight-audio --lib
cargo test -p sunlight-audiod --lib
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build -p sunlight-control-panel --release
cargo test -p sunlight-vortex-shell --lib
cargo test -p sunlight-libc --lib
./tools/test.sh audio
```

## Known limitations

* One output stream. No mixer, capture, resampling, or hot-plug policy.
* Software gain only. Codec amps are unmuted, not used as the master.
* IRQ-driven refill is not implemented; the service polls DPIB.
* `SUBMIT_PCM` is SHM-backed and limited to one page in v1.
* Persistence requires a writable `/root/.config/sunlight`.
* No-device boots stay usable; audiod reports `Unavailable`.

## Next phases

1. Software mixer and per-application streams.
2. Intel HDA IRQ wait + VirtIO Sound / USB Audio backends.
3. Hardware volume/mute via codec widgets where validated.
4. Service events so GUIs do not need Tick refresh.
5. Recording / capture.
