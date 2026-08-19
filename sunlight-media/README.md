# Sunlight Media

`sunlight-media` is the reusable, application-independent playback layer used
by Melody Mina. Applications interact with `MediaPlayer`; container parsing,
codec decoding, playback state, buffering, output timing, seeking, software
stream gain, and visualization analysis remain inside this crate.

Phase 2 deliberately uses an in-process library and one bounded worker. The
existing `sunlight-audiod` process already owns the system audio device, so a
second media daemon would duplicate lifecycle and IPC infrastructure. The
decoder and `AudioSink` traits preserve replaceable codec and output boundaries.

## Phase 2 format contract

- Container/codec: Ogg Vorbis and RIFF/WAVE PCM.
- Output: signed 16-bit little-endian interleaved PCM at 48 kHz.
- Channels: mono (upmixed to stereo) or stereo. Other layouts are rejected.
- Resampling: not implemented; non-48-kHz streams are rejected rather than
  played at an incorrect rate.
- Input: local seekable files up to 4 MiB. The compressed input is bounded but
  currently loaded once because the selected no-std Ogg reader is slice-based.
- Seek: exact decoder restart plus bounded decode/discard to the requested
  frame; targets past known duration are clamped.
- WAV parsing validates RIFF/WAVE signatures, PCM integer encoding, 16-bit
  mono/stereo layout, checked chunk sizes, and RIFF padding. Unknown chunks are
  skipped safely.
- Volume: per-stream software gain, 0 through 100, with saturating S16 scaling.
  It never changes audiod's system master volume.

The PCM producer is bounded by audiod's 64 KiB queue and the HDA four-period
ring. Position is derived from controller-consumed frames reported by audiod,
not GUI repaint time. Visualization is a replaceable latest-frame atomic
snapshot, so a slow UI cannot block audio.

`MediaSnapshot::generation` changes for every accepted source and lets clients
discard stale source updates. Open publishes `Loading` before returning and a
second Open is rejected until that load resolves. Pause, Stop, and Seek flush
both audiod's producer queue and hardware-resident periods; the output clock is
then rebased to the decoder's exact position so resume and post-seek timing do
not restart at zero.

## Decoder dependency

The crate pins the no-std Lewton/Ogg adaptation from `petamoriken/pxtone-rs` at
commit `2e088b0df1ce05c28a4458ac514217ef06a80c6b`. Lewton is licensed MIT OR
Apache-2.0; its Ogg dependency is BSD-3-Clause. Dependency-specific types are
private to `decoder.rs` and do not appear in the public media API.
