# SunlightOS System Sounds Foundation v1

## Mandatory architecture audit

| Component | Current architecture | Classification | Foundation v1 decision |
|---|---|---|---|
| Kernel audio driver | The kernel discovers Intel HDA and grants bounded MMIO/DMA only to the process named `audiod`. It does not own playback or sound policy. | Directly reusable | Unchanged. |
| `audiod` | Single-threaded `audio.v1` service. It owns master volume/mute, the HDA output stream, synthesized test tones, and a bounded 64 KiB general PCM queue. | Reusable with extension | Remains the only system-sound policy and playback owner. |
| Existing audiod IPC | `GET_STATUS`, `GET_DEVICE`, `GET_VOLUME`, `SET_VOLUME`, `SET_MUTE`, `PLAY_TONE`, `SUBMIT_PCM`, and `STOP`. User-session processes can resolve the service. | Reusable with extension | Added versioned semantic playback and system-sound policy operations. |
| PCM/WAV support | Native playback is 48 kHz, signed 16-bit, stereo PCM. Before this phase there was no WAV decoder; Test Sound and volume feedback were synthesized sine tones. | PCM directly reusable; WAV missing | Added a strict allocation-free parser for packaged native PCM WAV only. |
| Master volume | `MasterVolume` validates 0–100, tracks mute and the last nonzero level, and supplies effective gain. | Directly reusable | Unchanged; system gain is multiplied into its effective result with bounded integer arithmetic. |
| Mute semantics | Mute is independent of the stored master level and produces effective gain zero. | Directly reusable | Unchanged and respected by automatic sounds and previews. |
| Volume feedback | Control Panel requests a short 880 Hz tone after committed master-slider changes. | Reusable with extension | The caller cadence remains; playback is migrated to `SystemSound::VolumeChanged`. |
| Control Panel Sound page | Reads audiod state every 750 ms with bounded retry backoff; owns presentation for master slider, mute, and Test Sound. | Reusable with extension | Existing controls remain; a compact System Sounds section was added. |
| Test Sound | Calls audiod `PLAY_TONE` for a one-second 440 Hz device test. | Directly reusable | Unchanged and independent of the system-sounds enabled setting. |
| Vortex notifications | There is no separate notification daemon. IPC helpers persist history and send a bounded SHM wire record to `sunlight-display`; the display service presents toast overlays. | Reusable with extension | Sound policy is applied by `sunlight-display` after visual acceptance/coalescing. |
| Notification urgency | Popup kinds are Info, Warning, Error. Persisted priorities are Low, Normal, High, Critical. | Reusable with extension | Priorities are carried on the versioned popup wire and mapped centrally at presentation time. |
| Notification deduplication | History is bounded; the display toast queue was bounded to four but previously evicted the oldest without duplicate/update coalescing. | Reusable with extension | Stable owner/title identity now updates a toast in place. Content-only updates are silent; escalation may sound. |
| Dialog APIs | `sunlight-dialogs` provides versioned requests for Alert, Confirm, TextInput, and file dialogs; `sunlight-dialogd` presents them. | Reusable with extension | Wire v2 adds semantic severity and an explicit silent flag while decoding legacy v1. |
| Dialog semantic types | Alert/Confirm/TextInput previously had no info/success/warning/error/critical model. | Missing | Added `DialogSeverity` and typed constructors/mapping. |
| Asset loading | Large shared resources are RAMFS files; small service-local resources commonly use `include_bytes!`. SIMG is image-only. | Direct embedding reusable; SIMG unsuitable | The small built-in WAV theme is embedded only in the audiod binary. |
| Settings persistence | audiod owns `/root/.config/sunlight/audio.toml` and writes through a temporary file plus rename. | Directly reusable | Extended the same record; no second settings file or Control Panel copy exists. |
| Service updates | The Sound page already polls audiod state every 750 ms and backs off on failure. There is no audiod subscription protocol. | Directly reusable | External policy changes appear through the existing refresh mechanism. |
| Time/rate limit | `monotonic_millis()` is the established elapsed-time source. | Directly reusable | All cooldowns use monotonic timestamps; no sleeps are used. |
| Authentication boundary | User-session processes may resolve audiod, dialogd, and display. Only audiod may obtain HDA MMIO/DMA. | Directly reusable | Semantic IPC accepts only bounded IDs/flags; it never accepts paths or hardware resources. |

## Ownership and protocol

Applications, dialogs, and notifications own semantic intent. `SystemSound` is
a stable `repr(u16)` vocabulary with protocol version 1:

1. Notification
2. Message
3. Success
4. Warning
5. Error
6. Question
7. Critical
8. DeviceConnected
9. DeviceDisconnected
10. VolumeChanged

Unknown IDs, unknown flags, wrong word counts, and wrong protocol versions are
rejected as bad requests. The semantic request contains no filename, codec,
physical address, or device detail.

`audiod` owns the sole semantic-to-asset match table for the built-in
**Sunlight Default** theme. General PCM submission and the existing Test Sound
remain separate APIs.

## Default assets

The source-controlled generator is `tools/generate_system_sounds.py`. It uses
only Python's standard library at development time. Runtime has no synthesizer
or external codec dependency.

All files are RIFF/WAVE, PCM, signed 16-bit, 48 kHz, stereo.

| Semantic sound | Duration | File size |
|---|---:|---:|
| VolumeChanged | 120 ms | 23,084 bytes |
| DeviceConnected | 220 ms | 42,284 bytes |
| DeviceDisconnected | 220 ms | 42,284 bytes |
| Message | 280 ms | 53,804 bytes |
| Success | 300 ms | 57,644 bytes |
| Question | 320 ms | 61,484 bytes |
| Notification | 340 ms | 65,324 bytes |
| Warning | 360 ms | 69,164 bytes |
| Error | 420 ms | 80,684 bytes |
| Critical | 650 ms | 124,844 bytes |

The restrained family uses sine fundamentals with a quiet octave partial,
short attack/release envelopes, and a generated peak below full scale.

## Playback, queue, cache, and failure policy

- Effective gain is `(master_effective * system_sound_volume + 50) / 100`,
  clamped to 0–100 with `u16` intermediate arithmetic.
- Automatic sounds honor `system_sounds_enabled`; explicit Control Panel
  previews bypass only that switch.
- Automatic sounds and previews both honor master mute, master volume zero,
  and system-sound volume zero.
- A four-entry semantic queue stores only `(SystemSound, mode)` references.
  It never stores PCM copies.
- Identical active/pending sounds coalesce.
- Identical automatic cooldowns are deterministic: VolumeChanged 120 ms;
  Message/Success 300 ms; Notification/Question 350 ms; Warning/Error 500 ms;
  device events 600 ms; Critical 750 ms.
- At capacity, a higher-priority sound may replace one lower-priority pending
  entry; otherwise the request is rejected as overflow.
- Explicit Test Sound has precedence, then semantic sounds, then the existing
  general PCM queue. No new mixer was introduced.
- WAV headers are strictly validated when an asset becomes active. PCM is read
  directly from the audiod binary's immutable embedded bytes and streamed one
  4096-byte HDA period at a time. There is no decoded duplicate cache.
- Invalid assets are skipped, and each bad semantic asset logs at most once per
  audiod lifetime.

## Dialog policy

`DialogSeverity` maps as follows:

| Severity | Sound |
|---|---|
| Information | Silent |
| Success | Success |
| Warning | Warning |
| Error | Error |
| Critical | Critical |
| Question | Question |

`sunlight-dialogd` emits the request once after a window is available and
ignores audio failure, so the visual dialog continues normally. The common
options include `silent`, allowing deliberate suppression without requiring
applications to manipulate global audio state. The dialog border/title color
uses the same semantic severity.

## Notification policy

- Low priority is silent.
- Normal Info uses Notification.
- Normal Warning/Error retain Warning/Error.
- High uses Warning.
- Critical uses Critical.
- An explicit silent notification never emits a sound.
- A new visual toast may emit one sound.
- A matching owner/title identity updates the existing toast and is silent for
  content-only changes or repeated delivery.
- An urgency escalation may emit one new sound.
- Visual and audio acceptance share the same bounded presentation decision;
  applications do not play notification sounds themselves.

## Control Panel behavior

The existing Sound page retains output status, master volume, mute, and the
ordinary Test Sound. The new section reads and writes audiod's authoritative:

- `system_sounds_enabled`
- `system_sounds_volume`

It exposes semantic previews for Notification, Success, Warning, Error, and
Critical. Preview calls use the real semantic audiod API and remain available
while automatic system sounds are disabled. External changes are reflected by
the existing 750 ms state refresh.

## Persistence

`/root/.config/sunlight/audio.toml` remains the only audio settings record:

```toml
[audio]
master_volume = 65
muted = false
last_nonzero = 65
system_sounds_enabled = true
system_sounds_volume = 60
```

Missing/malformed fields recover to validated defaults; volumes clamp to
0–100. Writes retain the existing temporary-file-and-rename path.

## Manual QEMU verification matrix

1. Boot with the existing working QEMU HDA backend.
2. Confirm audiod reports the Sunlight Default theme and a four-entry queue.
3. Open Control Panel -> Sound.
4. Confirm master volume and mute still work.
5. Confirm the existing Test Sound still plays.
6. Toggle Play system sounds off and on.
7. Change System Sounds volume and hear only one committed feedback sound.
8. While automatic sounds are disabled, preview Notification and Warning.
9. Preview Success, Error, and Critical.
10. Run `audioctl preview warning`.
11. Show warning, error, critical, and silent dialogs.
12. Send normal, high, critical, low, and explicitly silent notifications.
13. Repeat an identical notification rapidly and confirm one toast/one sound.
14. Update the same notification body and confirm no new sound.
15. Escalate the same notification to Critical and confirm one Critical sound.
16. Disable system sounds and confirm dialogs/notifications remain visual.
17. Confirm ordinary Test Sound/general playback still works while disabled.
18. Inspect serial for semantic request, asset resolution, PCM submission, and
    playback-complete markers.

## Future compatibility

Callers depend only on stable semantic IDs. A later theme selector can replace
the audiod mapping/assets without changing application, dialog, notification,
or Control Panel call sites.
