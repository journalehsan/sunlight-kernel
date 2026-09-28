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

File bytes select the decoder; renaming a file to `.wav`, `.ogg`, or `.mp3`
does not convert it. Melody Mina uses `sunlight-media` to decode before sending
PCM to audiod. The HDA driver never receives container headers or compressed
audio packets.

| Input | Decoder / acceptance |
| --- | --- |
| RIFF/WAVE integer PCM | Signed 16-bit little-endian, mono or stereo |
| WAVE_FORMAT_EXTENSIBLE | PCM subtype only, 16 valid bits; unspecified or front-center mono / front L/R stereo layout |
| Ogg Vorbis | One complete logical stream, mono or stereo |
| MP3 (MPEG Layer III) | MPEG-1, MPEG-2, or MPEG-2.5, mono or stereo, with optional ID3v2 / APEv2 / ID3v1 metadata |
| WAV float, 8/24/32-bit PCM, ADPCM, A-law, mu-law | Rejected; no implicit reinterpretation as S16 |
| FLAC, Ogg Opus, video containers | Unsupported |
| Chained or multiplexed Ogg | Rejected before playback; rate/channel changes and per-stream clocks are not implemented |

Playback accepts source rates from **8,000 to 192,000 Hz** and a source file no
larger than **8 MiB**. A streaming linear converter maps source frames to the
48,000 Hz device clock, preserving interpolation state across decoder chunks.
Mono samples are duplicated to left/right; stereo order stays L, R. Output is
interleaved S16LE stereo. FLAC remains unsupported.

MP3 frames are checked for a consistent sample rate and channel layout before
playback. Decoding uses `nanomp3` (pure Rust, `no_std`), with bounded input and
one MP3 frame of PCM at a time. Seeking restarts the decoder and discards frames
up to the requested position to preserve the MP3 bit reservoir. Duration is
computed from the MPEG frame count; encoder delay/padding is not trimmed, so
the displayed time may differ slightly from a gapless-aware player's time.

The WAV parser checks RIFF bounds and chunk padding, frame alignment, byte
rate, block alignment, and extensible format length/subtype/channel mask.
Duplicate format/data chunks are rejected. Unknown metadata chunks are skipped.
The Ogg parser checks page boundaries, version, stream identity, sequence, and
the final EOS granule before accepting a local file; Lewton decodes Vorbis.

### Repository fixture audit (September 20, 2026; MP3 additions September 28, 2026)

Metadata was inspected with FFprobe and each file was decoded to EOF using
both `sunlight-media` and FFmpeg, without gain or resampling.

| File (under `assets/sounds` unless specified) | Actual encoding | Rate / channels | Frames | Player result |
| --- | --- | --- | --- | --- |
| `melody-mina-sample-48k-stereo.wav` | PCM S16LE | 48,000 / 2 | 288,000 | Supported, 6 seconds |
| `melody-mina-test-48k-stereo.ogg` | Vorbis | 48,000 / 2 | 96,000 | Supported, 2 seconds |
| `catch-the-sunlight-48k.ogg` | Vorbis | 48,000 / 2 | 9,345,828 | Supported, about 194.705 seconds; 2,783,951 bytes |
| `002-Zamfir-EinsamerHirte.mp3` | MPEG-1 Layer III | 48,000 / 2 | 12,590,208 | Supported; 4,201,187 bytes, about 262.296 seconds including encoder padding |
| `014-SecretGarden-SongFromASecretGarden.mp3` | MPEG-1 Layer III | 48,000 / 2 | 10,319,616 | Supported; 3,444,342 bytes, about 214.992 seconds including encoder padding |
| Ten `Sunlight Default/*.wav` sounds | PCM S16LE | 48,000 / 2 | Varies | Supported |
| `docs/songs/onaldin_music-catch-the-sunlight-333176.wav` | PCM S16LE | 44,100 / 2 | 8,586,479 | Rejected: 34,345,960 bytes exceeds 8 MiB |

All 12 WAV decodes matched FFmpeg byte for byte. Both Vorbis decodes had
identical frame counts and a maximum absolute difference of one S16 sample
unit (RMS difference approximately 0.706). This verifies the bundled decoded
PCM on the host, not native scheduling, DMA output, or audible playback.
Both MP3 files are seeded in `/home/user/Music` and `/root/Music`, so Melody
Mina finds them in the playlist on first launch with titles/artists read from
ID3 text tags. Host decoder tests check both tracks through EOF, seek, malformed
metadata, and the compressed size bound. Comparing a half-second passage of
each host decode against FFmpeg PCM after accounting for the 2,257-frame
encoder delay yielded an RMS difference of about 0.71 S16 units per track.
This does not establish native DMA output or audible playback quality.

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
Applications (Melody Mina via sunlight-media, Silicon Echoes PCM, audioctl,
              Control Panel, Vortex)
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
* Page 0 holds CORB/RIRB/BDL. Pages 1–12 hold twelve 4096-byte periods,
  providing about 256 ms of DMA address space within audiod's 16-page grant.
* Userspace never supplies a physical DMA address.
* Interrupts are acknowledged by polling `SDSTS` / `INTSTS`. There is
  no userspace IRQ wait yet; the service loop uses an 8 ms timeout.
* If the client exits or stops submitting, the driver plays silence.

### Playback continuity

* Start/restart resets the HDA stream descriptor before programming it. This
  brings the hardware cursor back into agreement with software period zero
  after a pause, stop, or seek.
* All twelve DMA periods are initialized to silence before RUN. Only the
  playing period and one guard period are reserved, allowing the first PCM
  period to be queued immediately after the guard without overwriting DMA.
* Each service pass observes DMA consumption before assigning new period
  ownership and refills up to twelve free periods before waiting for IPC. This
  lets the service catch up after a delayed poll.
* During an active media stream, audiod clears unowned descriptors to silence
  without claiming them. A late producer can still replace one before DMA
  reaches it. Outside a stream, audiod keeps the playing descriptor and one
  guard occupied by silence. The guard provides about 21 ms for the 8 ms poll.
* If RUN is first observed at a nonzero cursor, the refill index follows that
  cursor so samples remain in order across the first ring wrap.
* Test-tone phase advances only when a period is accepted. A full ring must
  not discard part of the waveform or count as a silence underrun.
* If observed DMA progress exhausts the prepared periods, recovery reserves
  the currently playing descriptor and resumes writes after it. This counts
  an underrun without overwriting samples already being read by hardware.
* The media worker collects converted PCM into 1024-frame writes. Decoder
  chunk boundaries cannot make audiod pad a short DMA period with silence;
  only the final period at EOF may be short. Playback first collects five
  periods (about 107 ms) before submitting any music; a shorter track is
  submitted when it reaches EOF. The worker then maintains about 170 ms of
  submitted audio across audiod's queue and the hardware ring. Decode and
  file I/O run outside audiod's hardware polling loop, and visualizer updates
  use atomics rather than a mutex.
* `[MEDIA][format]` logs source and output rate/channel count, period size,
  and target buffer latency. `[MEDIA][playback]` logs submitted, consumed,
  buffered, and underrun frames together with buffer latency in milliseconds.
  Stream underruns are counted from the first PCM submission, excluding the
  HDA counter's earlier idle-silence history.
  Per-request audiod IPC diagnostics are compiled only when
  `SUNLIGHT_AUDIO_IPC_DIAGNOSTICS` is set during the audiod build; the regular
  playback path keeps serial output bounded.
* The decoder worker uses a timed kernel wait when idle. Repeated voluntary
  yields previously triggered the scheduler's short-burst penalty before
  playback began.

Host regressions exercise rejected tone writes, startup DMA ownership,
starvation recovery, and byte-exact PCM refills across delayed polls and ring
wraps. Decoder tests also cover the bundled WAV and Ogg fixtures. These checks
do not establish audible quality on a physical device or the host audio backend.

### Playback starvation investigation (September 28, 2026)

The reported playback log shows about 39–47 underruns per second of submitted
music. Most replies have `buffered_frames=1024` (21 ms), despite the logged
170 ms target. That target is a producer upper bound, not a guarantee of
prepared DMA audio: the five startup periods are collected locally, then sent
one at a time through synchronous IPC. Slow delivery can consume each period
before the next submission arrives.

The native `ProcessYield` syscall used by IPC wait loops previously only set a
reschedule flag. Execution continued until a roughly 10 ms timer tick. It now
uses the existing reschedule IPI after SYSRET, with the scheduler lock released,
so the switch saves a userspace frame without advancing the clock. This removes
the timer wait from the voluntary handoff; it does not grant audio clients
additional privileges or change IPC deadlines.

New bounded diagnostics accompany the existing progress line:

* `[MEDIA][timing] elapsed_ms` measures time since the first PCM submission;
  compare it with `position_ms` to detect playback running behind elapsed time.
* `submit_max_ms` includes gain, SHM allocation/copy, synchronous submission,
  cleanup, and queue-full retries, for the last progress interval.
* `producer_gap_max_ms` measures time between a completed submission and the
  next attempt. It includes decoding, visualization, scheduling, logging, and
  intentional buffer-limit waits; it is not a decoder-only measurement.
* `[AUDIOD][starvation]` is emitted at most once per second while stream
  underruns increase. `poll_gap_max_ms` measures the longest gap between
  service pump passes since stream start or the previous starvation report.
  `queue_frames` samples the software queue after pumping; an empty queue alone
  does not prove that the hardware ring is empty.

Timing uses the existing approximately 10 ms monotonic clock. Logs are bounded
to the kernel DebugLog limit of 256 bytes per line. A submission timeout now
has media detail 9 (`Audio PCM submission timed out`); previously it shared
detail 3 with status-read errors. Therefore the original report's final
`kind=9 detail=3` cannot identify which request timed out from that line alone.

Native comparison used isolated QEMU TCG guests with four CPUs, 1 GiB RAM,
1280x800 VirtIO graphics, Intel HDA, and the WAV capture backend. Both runs
launched Melody Mina from the TTY and played the bundled six-second WAV.
The original ISO produced 241 underruns and about 11.13 seconds between its
first and last nonquiet captured samples. With the yield change and timing
instrumentation, the player reached all 288,000 frames with 13 underruns:
those were confined to startup, then the buffer stayed about 168–189 ms and
the underrun count remained unchanged through EOF. The captured WAV passage
occupied about 6.26 seconds after the change. The first two timing
intervals exposed a 340 ms producer gap and a 260 ms submission span; later
submission maxima were 10–20 ms. This is a substantial improvement, not a
claim of uninterrupted startup or verified speaker output.

The MP3 follow-up exposed a separate producer bottleneck. In the same patched
guest, `Einsamer Hirte` reached only 55.972 seconds of media in 120.760 seconds
of elapsed submission time, with 3,012 underruns. Audiod's observed poll gaps
were usually 20–30 ms and submission maxima usually 10–30 ms, while producer
gaps reached 50–260 ms. This localizes the remaining starvation upstream of
the audiod queue; the yield fix does not make native MP3 playback fluent.

The release build also confirms an expensive decoder path: `nanomp3` uses
floating-point transforms, while the native `x86_64-unknown-none` target keeps
software floating point even with `-C target-cpu=x86-64-v2`. A symbolized build
shows `nanomp3::minimp3::L3_imdct36` calling `__subsf3` and `__addsf3` through
the GOT; `__mulsf3` and `__divsf3` are also linked. `rustc --print cfg` with the
actual target/CPU flags does not enable SSE/SSE2. This is a strong candidate
for the MP3 producer cost, but the measured producer gap includes scheduling
and visualization, so it is not a decoder-only CPU profile.

Do not infer native decoder performance from host decode tests: those use a
different target. The next performance investigation should measure native
decode time separately and evaluate a fixed-point decoder or properly supported
hardware floating point. Merely adding SSE flags is insufficient: the current
process context and context-switch assembly have no per-task FP/SIMD save and
restore path. This investigation leaves that architecture change unimplemented.
Symbol/GOT disassembly and effective target flags are saved under
`target/mina-lag-evidence/`.

The patched guest also passed its built-in 1,000-call IPC round-trip check and
Phase 2.6/3.0/3.7 markers. Focused host tests passed (27 audio, 13 audiod, 36
media), as did the kernel build and native audiod/Melody Mina builds/checks.
The test ISO predates only the subsequent error-message distinction, which
was covered by its host regression and native build. Evidence is retained in
`target/mina-lag-baseline.log`, `target/mina-lag-fixed.log`, and their WAV
captures; these generated artifacts are not source-controlled. Physical audio
and the user's 1898x1037 desktop workload still need audible verification.

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

For Melody Mina, play bundled WAV, Ogg, and both MP3 playlist entries through
EOF, then repeat with pause/resume, Stop/Play, seeks, and Next/Previous between
the two MP3 tracks. Listen for repeated blocks, gaps, or clicks at period
boundaries and check that the position reaches the track duration. This
exercises the media producer as well as the tone path.

Silicon Echoes synthesizes a 16-second music loop directly in native PCM on a
worker thread. It keeps roughly five 1024-frame chunks buffered, advances its
composition clock only after `SUBMIT_PCM` succeeds, and flushes its stream on
game music mute, focus loss, or exit. `M` toggles game music. The scene ambient
cue text is not yet an audio effect. In an interactive QEMU session, listen
across the loop boundary and test the toggle, focus changes, and window close.

## Test commands

```bash
cargo test -p sunlight-audio -p sunlight-audiod -p sunlight-media --lib --target x86_64-unknown-linux-gnu
RUSTFLAGS="$SERVICE_RUSTFLAGS" cargo build -p sunlight-control-panel --release
cargo test -p sunlight-vortex-shell --lib
cargo test -p sunlight-libc --lib
./tools/test.sh audio
```

## Known limitations

* One output stream. No mixer, capture, or hot-plug policy. Resampling uses
  linear interpolation; high-fidelity band-limited conversion is not yet
  implemented.
* Melody Mina accepts only the bounded file formats listed above; support for
  a container does not imply support for every codec carried by it.
* Software gain only. Codec amps are unmuted, not used as the master.
* IRQ-driven refill is not implemented; the service polls LPIB.
* Polling must keep up with the roughly 256 ms DMA ring. Whole-ring laps cannot
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
