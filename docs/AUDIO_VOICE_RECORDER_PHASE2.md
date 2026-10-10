# Sunlight Voice Recorder — Phase 2

Native application ID: `org.sunlight.voice-recorder`; executable:
`/bin/voice-recorder`. The Start menu/search entry is **Voice Recorder** and
its window title is **Sunlight Voice Recorder**. The bundle uses the existing
microphone icon, ordinary decorated window, shell task tracking, and native
Save File / confirmation dialogs. Keyboard shortcuts: R Record; P or Space
primary Pause/Resume/Play; S Stop; A Save As; N New; Escape close.

## Architecture and infrastructure audit

The application is an `audiod.capture` client. It never maps HDA, computes a
second microphone level algorithm, or owns DMA. Audiod negotiates capture
format, owns the single capture session, reads completed hardware periods,
computes peak, and provides one shared-memory period per READ. The initial
format is 48,000 Hz stereo signed 16-bit little-endian PCM, with 4096-byte
periods (1024 frames). The recorder validates the returned format and derives
all header fields and duration from it. Mono/stereo S16LE at 8–192 kHz is
accepted by the writer; other formats fail before capture starts.

The UI uses `sunlight-ui::App`, standard buttons, fonts, decoration, cursor
feedback, resize events, and native `sunlight-dialogs`. One persistent native
worker owns capture, the file writer and commands. Shared atomics publish a
small snapshot to the UI. A reserved command slot prevents duplicate and
concurrent commands; Save As paths are copied into a fixed-size slot before
release publication. Audio IPC and disk writes happen on the worker. The
existing `sunlight-media::MediaPlayer` retains its own playback worker.

Capture drains at most eight periods per worker pass into a reused 32 KiB
batch. Writes occur per batch; the final partial batch is flushed on Pause or
Stop. There is no allocation per sample or period and no sync per batch. The
UI polls at 40 ms while active, 200 ms while idle; redraws require a changed
snapshot or input. The level comes directly from audiod's peak.

The shared media layer previously loaded every file into a bounded whole-file
buffer. A several-minute WAV exceeds that bound. PCM WAV now uses a bounded
random-access file source and the same validated RIFF parser, decoder,
resampler, packetizer and audiod sink. Header probing reads chunk metadata;
decode reads at most 4096 bytes at a time. OGG/MP3 retain their existing bounded
compressed-file path. Seeking and replay retain shared player semantics.

## State machine

`state.rs` defines legal transitions independently of native IPC:

- Idle → Recording.
- Recording → Paused → Recording.
- Recording or Paused → Finalizing → Recorded; zero frames → Idle.
- Recorded → Playing → PlaybackPaused → Playing.
- Playing/PlaybackPaused → Recorded on Stop; EOF → Recorded.
- Recorded → Idle on New; Error → Idle on New.
- Save As is accepted only in Recorded and leaves that state intact.

Record, Play and Save cannot overlap. Duplicate commands and invalid actions
are rejected. Capture/storage errors abandon the candidate and enter Error;
New resets it. Save/playback failures preserve the finalized candidate and
permit another attempt. The timer is `frames / sample_rate`, displayed as
HH:MM:SS, and does not count paused wall time.

Pause stops input DMA, drains audiod's bounded FIFO, and flushes the client
batch. It retains the Open capture session, sending a status heartbeat while
paused to renew the existing five-second lease. Resume clears the old FIFO
and restarts capture. Audiod STOP now retains completed queued periods and
READ accepts Open state so a client can drain them; CLOSE still discards them.
This small lifecycle correction avoids throwing away completed audio at Pause
or Stop and keeps monitor-only behavior intact.

## Storage and WAV lifecycle

Live mode records into RAMFS without requiring a SUNSTATE disk. The app
initializes its environment and resolves `HOME`, falling back to private `/tmp/voice-recorder-<uid>` storage when
no usable home directory is available. It creates `Musics` as the default
Save As folder and a private `.voice-recorder-<uid>` directory for candidates.
RAMFS consumes memory as audio grows and saved recordings last until reboot.
The existing `/state/users/<uid>` disk storage remains available as a Save As
location when mounted and permitted by filesystem policy.

A new recording exclusively creates:
`$HOME/.voice-recorder-<uid>/session-<owner-pid>-<monotonic-ms>.part`
(or under `/tmp/voice-recorder-<uid>` without a usable home).
It writes a 44-byte PCM RIFF/WAVE placeholder, streams aligned PCM, and keeps
only counts and the fixed batch in memory. The writer rejects incomplete
frames, unsupported formats, duplicate finalization and RIFF overflow. A
partial/failed write poisons the writer so it cannot be finalized as valid.
Stop freezes and drains capture, closes the capture handle, writes final RIFF
and data lengths at offset zero, syncs and closes. Only success publishes
Recorded. The RIFF limit is just under 4 GiB; available RAM or disk capacity can impose
a lower limit. Empty audio is discarded and reported.

New/normal shutdown delete the candidate. A recording interrupted by capture
loss, FIFO drops, three seconds without frames, write failure or full storage
is safely abandoned. Startup removes only app-pattern candidates whose owner
PID is no longer alive and skips live sessions. Cleanup scans bounded pages.
An abrupt crash may leave a `.part` until the next recorder startup; it is
never presented as a valid recording. PID reuse can delay stale-file removal.

The capture worker inherits the spawning application's endpoint restrictions.
The recorder spawn profile receives the Microphone capability, including for
ordinary users; playback permission alone does not grant capture access.

## Playback and Save As

Play opens the finalized candidate through shared `MediaPlayer`. Pause,
resume, Stop, EOF and replay use existing player commands. The recorder never
runs capture and playback together. New releases the loaded media source
before removing the candidate.

Save As starts in `$HOME/Musics` (or `/tmp/voice-recorder-<uid>/Musics`), suggests
`Recording <UTC Unix seconds>.wav`, and filters WAV. Native dialogs select a
path and confirm overwrite; the app also confirms a target observed after the
dialog so it cannot silently replace a newly appeared file. The worker
normalizes the path and the kernel enforces existing write policy.

Save copies through a 32 KiB buffer into an exclusively created sibling
`.voice-save-<pid>-<monotonic-ms>.part`, verifies exact WAV size, syncs/closes,
then publishes by filesystem rename (no-replace unless confirmed), and syncs
the parent on disk storage. The native recorder adapter skips disk sync for
the live RAMFS namespaces `/home`, `/root` and `/tmp`; RAMFS itself continues
to report unsupported durability, so other clients retain their guarantees.
Failure/cancellation removes staging and preserves the source.
Shutdown cancels copying between batches. A directory-sync failure after
rename can leave a complete destination while Save reports failure; the
original remains available. FAT rename is a logical atomic publish under the
VFS lock; it is not a power-loss transactional filesystem. Unexpected process
termination during Save can leave sibling staging in the selected folder.

## Window and process cleanup

QEMU demonstrated that the display server immediately sent SIGTERM when the
last window closed, preventing client destructors. The recorder opts into a
new owner-only bounded close-grace IPC (maximum six seconds). The compositor
removes the window; EVENT_POLL reports it invalid; the normal app loop returns
and drops the worker before process exit. A stuck owner is still killed after
the grace. Other clients retain their default immediate-close policy.
The worker cancels Save, stops/closes capture, abandons unfinished storage and
releases shared playback. A bounded shutdown timeout retains shared memory
until process teardown rather than freeing it under a live worker.

The runtime also demonstrated lost keyboard events: TTY forwarded them with a
100 ms IPC deadline, shorter than some TCG desktop redraws. Keyboard forwarding
now uses a bounded two-second deadline. Other TTY operations retain their
existing deadlines. This fixes native controls/file-dialog input without
changing capture or playback architecture.

## Verification

Host commands (explicit Linux target):

```sh
cargo test -p sunlight-media -p sunlight-voice-recorder -p sunlight-fs \
  -p sunlight-audiod -p sunlight-shell-appstate --lib \
  --target x86_64-unknown-linux-gnu
cargo test -p sunlight-display --target x86_64-unknown-linux-gnu app_lifecycle::tests
```

The focused suite passes 179 tests: recorder 15, media 37, filesystem 72,
audiod 25, shell app state 30. Recorder tests cover state transitions, duplicate
commands, frame-duration conversion, pause accounting, valid/empty WAV,
write/sync failure, RIFF length/format limits, exclusive temporary creation,
cleanup, successful/failed/cancelled Save As and a three-minute streamed file.
A sparse 34.56 MB media-source test proves bounded WAV probing/reading and
seeking near EOF without allocating the recording. Three display lifecycle
tests pass, including owner restriction and the six-second cap.

Broader host suites expose six failing display geometry/damage tests and one
shell sidebar unavailable-telemetry test; these are outside recorder/audio
paths. Native recorder, media/audiod, shell/display/TTY and kernel builds are
checked separately to avoid unrelated global-allocator feature-union conflicts.

Runtime measurements and remaining acceptance checks are recorded below after
QEMU validation. Serial and screenshots alone do not establish human-audible
voice clarity.
