# Wallpaper and notification playback regression

## Source audit and changes

The shell used to read desktop configuration and wallpaper files and decode
images inside `view()`. Every redraw also rescaled the wallpaper per pixel,
following a redundant full-frame clear. File reads used 128-byte syscalls.

A single process-lifetime worker now polls configuration every 500 ms, reads
files in 4 KiB chunks, decodes and prepares opaque pixels at the canvas size.
The UI takes one complete result at a draw boundary; it returns the previous
image/configuration to the worker for destruction. Acquire/release mailboxes
transfer ownership without a display lock. Timer events request a redraw when
a result is ready. Normal redraws copy prepared rows, without scaling or an
underlay clear. Resolution transitions temporarily retain the old cover image
until the new size is ready. Existing image formats, sampling and solid fallback
are preserved. No display protocol or texture-upload protocol changes.

Prepared pixels use two reusable anonymous mappings, separate from the 16 MiB
shell heap used by decoding. This fixes the native 1898x1157 regression: a
screen-sized image needs 8,783,944 bytes, exceeding the logged largest free heap
block (6,523,976 bytes) while the decoded source was live. Keeping the displayed
image in that same heap would also starve subsequent decodes. The worker reuses
the retired mapping only after the UI returns ownership; steady-size changes
allocate no further screen buffers. Two buffers at the reported resolution use
17,571,840 pixel bytes, plus page rounding. The shell heap limit is unchanged.
The single native worker adds its standard 2 MiB stack/TLS mapping. Old-size
spares are unmapped before replacement; the visible image is retained until the
replacement is ready. Mapping failure preserves the current wallpaper and logs
an error. If thread creation fails, a startup-only fallback is used.

Native validation also exposed an mmap cursor collision: native borrower threads
started with `mmap_next=0` despite sharing the parent's address space. Mapping
placement, frame ownership and swap accounting now resolve the address-space
owner, while native file-descriptor sharing rules remain unchanged. The existing
mapping collision checks, transaction commit and unmap shootdown remain intact.

Audiod previously selected system-sound PCM **instead of** music PCM for each
hardware period. Music remained queued, so a notification paused its consumption
without necessarily incrementing HDA underruns. Both inputs now advance in the
same period, with independent gains and saturating addition. Only music frames
are tagged in the stream progress tracker. Input cursors advance only after
successful hardware submission. System sounds use static embedded native PCM,
validated and cached at service startup; playback performs no file I/O or decode.
Full-scale overlapping inputs can clip through saturation, rather than wrap.
The explicit `audioctl test` diagnostic tone remains an exclusive test output.

The PCM pump uses fixed stack/queue storage and has no wallpaper/display locks.
Audio-settings persistence is deferred while a music owner or system sound is
active. Changes take effect immediately but may not reach disk until music is
stopped. The media worker already decodes independently of UI work and targets
8 x 1024 frames (~171 ms at 48 kHz), with five startup periods. Buffer sizes are
unchanged. Existing synchronous audiod IPC, per-chunk shared-memory allocation,
and producer pacing remain; this patch does not make the IPC/kernel allocator
real-time. There is no cross-service wallpaper lock in that path.

## Diagnostics

Build services with `SUNLIGHT_UI_AUDIO_DIAGNOSTICS=1` in the environment:

```sh
SUNLIGHT_UI_AUDIO_DIAGNOSTICS=1 \
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' \
cargo build --release -p sunlight-vortex-shell -p sunlight-audiod -p melody-mina
```

Keep the flag set when rebuilding the boot image so embedded services match.

- `[WALLPAPER]`: read, decode, load/decode, scale/preparation and ownership-swap
  durations in milliseconds. Millisecond resolution can report zero.
- `[AUDIOD][timing]`: cumulative late pump ticks (> one ~21 ms period), maximum
  pump gap, queued music frames and pending system sounds, at most once/second.
- `[AUDIOD][starvation]`: existing HDA underrun deltas and queue context.
- `[MEDIA][timing]`: existing submit/producer timing plus cumulative crossings
  below two buffered periods, sampled after successful submissions. This is not
  a continuously sampled queue minimum and cannot detect every transient dip.
- `[IPC-AUDIO]`: endpoint depth and pending callers, mapped to process name/PID,
  including audiod, display and Melody Mina (which hosts sunlight-media; it is
  a library, not a separate service). These lines require the same environment
  flag **and** the kernel's existing `verbose_diag` feature. This heavyweight
  scheduler dump holds scheduler locks and writes serial output; use it only
  for a separate diagnostic capture, not the smooth-playback acceptance run.

No new lock is introduced in wallpaper preparation or mixing. Longest allocator,
display-service and scheduler lock holds are not measured by this patch.
`Window::commit_with_desktop_overlay` still copies the hidden drawing buffer
into the shared visible surface and waits for `COMMIT_FRAME`. The display
service's `present_back_buffer` copies rows to Limine/SVGA framebuffer memory,
then issues an SVGA update or Virtio GPU flush as appropriate. These full-frame
publication/presentation costs remain on the UI/display path; no wallpaper file
I/O or decoding occurs there after preparation. They have no dependency from
audiod's pump. Worker preparation also does not remove existing synchronous
shell status/service queries.

## Automated checks

```sh
cargo test --target x86_64-unknown-linux-gnu \
  -p sunlight-audio -p sunlight-audiod -p sunlight-media --lib
rustc --edition 2021 --test services/sunlight-vortex-shell/src/wallpaper_buffer.rs -o /tmp/wallpaper-buffer-tests
/tmp/wallpaper-buffer-tests
cargo check -p sunlight-kernel -p sunlight-vortex-shell -p sunlight-audiod -p sunlight-media
bash tools/test.sh audio
```

Mixing tests verify additive samples, short-effect tails, mute and saturation in
both polarities. Existing queue/progress tests cover buffer ownership and DMA
retirement. The QEMU audio gate checks device setup, tone output and DMA progress;
it does not run the desktop-interaction cases below or prove audible quality.

## Desktop acceptance (normal VM configuration)

Use a fresh build, the usual VM memory/CPU count and audio backend. Record CPU
count, KVM/TCG, resolution, music format and serial log. Keep `verbose_diag` off.
Use a track longer than the test and avoid intentionally starting/stopping it
within an observation interval.

1. Play music in Melody Mina for 30 seconds to establish baseline counters.
2. Change between two bundled wallpapers 20 times, roughly once per second.
   Keep moving a window or pointer. Verify no blank intermediate wallpaper,
   no UI freeze and no click/pause in the music. Check completion timings.
3. During playback, run `audioctl preview notification` repeatedly, about once
   per second for 20 requests. Also trigger real shell notifications. Expect
   each accepted sound to overlay continuous music. Existing cooldown/queue
   policy can suppress excess requests; suppression is not a music underrun.
4. Repeat wallpaper changes with the existing CPU Benchmark app running, then
   repeat with notifications. Stop the benchmark and verify recovery to baseline.
5. Repeat with a cleared wallpaper, an unreadable file, and a display-size change.
   Check documented fallback behavior and that music continues.
6. Change master/system-sound volume, stop music, and verify persistence. Confirm
   ordinary Play/Pause/Stop/seek and next/previous behavior still works.

For each interval, compare underrun counters before/after using `audioctl status`
and serial timing lines. Normal-load acceptance requires no new underruns, no
music discontinuity and no visible UI stalls. Do not count zero underruns or
advancing DMA alone as audible success: the original exclusive-notification
bug could pause music while DMA continued normally.

## Verification recorded September 29, 2026

- Host library tests: sunlight-audio 27, sunlight-audiod 15, sunlight-media 36;
  all passed.
- Native cargo checks: kernel, shell, audiod and media passed.
- Release ELF builds: shell, audiod and Melody Mina passed with diagnostics.
- `bash tools/test.sh audio`: Audio playback foundation gate passed.
- Desktop repeated-wallpaper/notification/benchmark matrix and audible playback:
  **not verified** by these automated checks; run the acceptance steps above.

## Black wallpaper regression follow-up

The first implementation failed at 1898x1157 because the additional full-screen
Vec could not fit in the shell heap; the initial black fallback remained and
Apply repeated the allocation failure. Dedicated reusable pixel mappings and
the native mmap owner/cursor correction address that observed failure.

Regression commands for the screen buffer are in Automated checks above. For a
native worker/UI test, build the shell with `SUNLIGHT_WALLPAPER_RELOAD_TEST=1`
and `SUNLIGHT_UI_AUDIO_DIAGNOSTICS=1`, rebuild the kernel/ISO, and log into the
desktop. It alternates two bundled wallpapers for six ownership round trips,
then restores the actual configuration. It never writes desktop.toml. Require
`[WALLPAPER-TEST] step=0` through `step=6`, each followed by `[WALLPAPER] swap`,
and no mapping/allocation failures. Rebuild the shell without the reload flag
and rebuild the kernel/ISO afterwards.

Observed on four-vCPU QEMU TCG, 2 GiB RAM, VirtIO GPU, 1898x1157:

- Seven preparations/swaps completed (six alternate images plus restoration).
- Only two screen-sized wallpaper mmap calls, with reuse for later swaps.
- Alternate-image read/decode 100-170 ms, prepare 20-30 ms; restored default
  read/decode 360 ms and prepare 70 ms. Swaps measured 0 ms at timer resolution.
- No wallpaper mapping, heap-allocation or unmap errors in the capture.
- Three host buffer tests passed, including 20 swaps and size/overflow checks.
- `bash tools/test.sh mm2d` passed the native munmap/shootdown regression gate.
- Evidence: `target/wallpaper-reload-regression-serial.log` and
  `target/wallpaper-mm2d-serial.log` (generated artifacts, not committed).

QEMU screendumps also showed scanout skew at this odd resolution, both before
and after successful wallpaper preparation. This test establishes buffer
allocation/load/swap success, not compositor visual correctness or audible
playback. GUI Apply clicks were not the driver of this automated reload test.
The normal rebuilt ISO has the automatic reload test disabled.
