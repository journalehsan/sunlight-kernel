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

## Melody Mina file formats

File bytes select the decoder; renaming a file to `.wav` or `.ogg` does not
convert it. Melody Mina uses `sunlight-media` to decode before sending PCM to
audiod. The HDA driver never receives WAV headers or compressed Vorbis packets.

| Input | Decoder / acceptance |
| --- | --- |
| RIFF/WAVE integer PCM | Signed 16-bit little-endian, mono or stereo |
| WAVE_FORMAT_EXTENSIBLE | PCM subtype only, 16 valid bits; unspecified or front-center mono / front L/R stereo layout |
| Ogg Vorbis | One complete logical stream, mono or stereo |
| WAV float, 8/24/32-bit PCM, ADPCM, A-law, mu-law | Rejected; no implicit reinterpretation as S16 |
| MP3, FLAC, Ogg Opus, video containers | Unsupported |
| Chained or multiplexed Ogg | Rejected before playback; rate/channel changes and per-stream clocks are not implemented |

Playback requires **48,000 Hz** and a source file no larger than **4 MiB**.
These are player limits, separate from the decoder's ability to inspect or
decode a valid 44.1 kHz WAV. There is no resampler. Mono samples are duplicated
to left/right; stereo order stays L, R. Output is interleaved S16LE stereo.

The WAV parser checks RIFF bounds and chunk padding, frame alignment, byte
rate, block alignment, and extensible format length/subtype/channel mask.
Duplicate format/data chunks are rejected. Unknown metadata chunks are skipped.
The Ogg parser checks page boundaries, version, stream identity, sequence, and
the final EOS granule before accepting a local file; Lewton decodes Vorbis.

### Repository fixture audit (September 20, 2026)

Metadata was inspected with FFprobe and each file was decoded to EOF using
both `sunlight-media` and FFmpeg, without gain or resampling.

| File (under `assets/sounds` unless specified) | Actual encoding | Rate / channels | Frames | Player result |
| --- | --- | --- | --- | --- |
| `melody-mina-sample-48k-stereo.wav` | PCM S16LE | 48,000 / 2 | 288,000 | Supported, 6 seconds |
| `melody-mina-test-48k-stereo.ogg` | Vorbis | 48,000 / 2 | 96,000 | Supported, 2 seconds |
| `catch-the-sunlight-48k.ogg` | Vorbis | 48,000 / 2 | 9,345,828 | Supported, about 194.705 seconds; 2,783,951 bytes |
| Ten `Sunlight Default/*.wav` sounds | PCM S16LE | 48,000 / 2 | Varies | Supported |
| `docs/songs/onaldin_music-catch-the-sunlight-333176.wav` | PCM S16LE | 44,100 / 2 | 8,586,479 | Rejected: 34,345,960 bytes exceeds 4 MiB; rate also unsupported |

All 12 WAV decodes matched FFmpeg byte for byte. Both Vorbis decodes had
identical frame counts and a maximum absolute difference of one S16 sample
unit (RMS difference approximately 0.706). This verifies the bundled decoded
PCM on the host, not native scheduling, DMA output, or audible playback.

The host diagnostic below bypasses the player size/rate limits intentionally
so a valid but unplayable source can still be examined. Its output preserves
the source channel count and sample rate; it does not play sound.

```bash
cargo run -p sunlight-media --example decode_pcm --target x86_64-unknown-linux-gnu -- \
  assets/sounds/melody-mina-sample-48k-stereo.wav /tmp/mina.s16le
ffmpeg -v error -y -i assets/sounds/melody-mina-sample-48k-stereo.wav \
  -c:a pcm_s16le -f s16le /tmp/reference.s16le
cmp /tmp/mina.s16le /tmp/reference.s16le
```

## Ownership model

```text
Applications (Melody Mina via sunlight-media, audioctl, Control Panel, Vortex)
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

### Playback continuity

* Start/restart resets the HDA stream descriptor before programming it. This
  brings the hardware cursor back into agreement with software period zero
  after a pause, stop, or seek.
* All four silence periods are prepared and marked occupied before setting
  RUN. The first producer write waits for a completed period, so it cannot
  overwrite the descriptor currently being played at startup.
* Each service pass observes DMA consumption before assigning new period
  ownership and refills up to four free periods before waiting for IPC. This
  lets the service catch up after a delayed poll.
* An empty producer queue only tops up silence to the playing descriptor plus
  one guard descriptor. Other free slots remain available for PCM arriving
  before its playback deadline, avoiding premature silence gaps. The guard
  provides at least one 1024-frame period (about 21 ms) for the 8 ms poll.
* If RUN is first observed at a nonzero cursor, the refill index follows that
  cursor so samples remain in order across the first ring wrap.
* Test-tone phase advances only when a period is accepted. A full ring must
  not discard part of the waveform or count as a silence underrun.
* If observed DMA progress exhausts the prepared periods, recovery reserves
  the currently playing descriptor and resumes writes after it. This counts
  an underrun without overwriting samples already being read by hardware.

Host regressions exercise rejected tone writes, startup DMA ownership,
starvation recovery, and byte-exact PCM refills across delayed polls and ring
wraps. Decoder tests also cover the bundled WAV and Ogg fixtures. These checks
do not establish audible quality on a physical device or the host audio backend.

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

For Melody Mina, play both bundled 48 kHz WAV and Ogg samples through EOF,
then repeat with pause/resume, Stop/Play, and seeks. Listen for repeated blocks,
gaps, or clicks at period boundaries and check that the position reaches the
track duration. This exercises the media producer as well as the tone path.

## Test commands

```bash
cargo test -p sunlight-audio -p sunlight-audiod -p sunlight-media --lib --target x86_64-unknown-linux-gnu
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build -p sunlight-control-panel --release
cargo test -p sunlight-vortex-shell --lib
cargo test -p sunlight-libc --lib
./tools/test.sh audio
```

## Known limitations

* One output stream. No mixer, capture, resampling, or hot-plug policy.
* Melody Mina accepts only the bounded file formats listed above; support for
  a container does not imply support for every codec carried by it.
* Software gain only. Codec amps are unmuted, not used as the master.
* IRQ-driven refill is not implemented; the service polls LPIB.
* Polling must keep up with the roughly 85 ms DMA ring. Whole-ring laps cannot
  be reconstructed from the modulo hardware cursor alone; long scheduling
  stalls can still cause playback discontinuities.
* `SUBMIT_PCM` is SHM-backed and limited to one page in v1.
* Persistence requires a writable `/root/.config/sunlight`.
* No-device boots stay usable; audiod reports `Unavailable`.

## Next phases

1. Software mixer and per-application streams.
2. Intel HDA IRQ wait + VirtIO Sound / USB Audio backends.
3. Hardware volume/mute via codec widgets where validated.
4. Service events so GUIs do not need Tick refresh.
5. Recording / capture.
