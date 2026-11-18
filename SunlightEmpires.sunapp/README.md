# Sunlight Empires native staging bundle

Build with `python3 tools/package_empires.py`. Rebuild the optional soundtrack
using `python3 tools/empires_music.py` (requires ffmpeg matching the lock).

This follows the existing native manifest examples (VoiceRecorder/WiseOwl),
not the DOS entry convention. `sun-exec path.sunapp` currently accepts Chronos
only. It rejects this native bundle with UnsupportedRuntime. No installer,
signature/trust verifier, archive activation, or arbitrary native-bundle loader
is implemented. Do not describe this as an installable standalone native package.

The generated target bundle contains Program/sunlight-empires, packaged music,
originals, icon, credits, licenses and immutable content definitions. The manifest
uses the currently supported native entry `/bin/sunlight-empires`. The existing
embedded executable registry supplies that ELF; RAMFS stages playback resources and notices (excluding original MP3 source copies) at
/Applications/SunlightEmpires.sunapp. Ordinary Search registration remains the
existing Games entry; it does not discover or execute Program/ from this bundle.
Replacing a staged bundle alone does not replace the registered executable.

Runtime requires SunlightOS native ELF ABI, SGP Window, libc thread/SHM/futex
APIs and fonts compiled into the ELF. audio.v1 and HDA output are optional.
No system shared libraries are duplicated; native dependencies are statically
linked as in existing games. Simulation civilization definitions and SFX are
compiled code; Resources/content.rs and sound.rs preserve their versioned data.

The app opens only Resources/music/*.wav read-only. It never writes to the bundle.
Volume preferences use the established sunlight-kv service under
`app.empires.settings/<uid>/v1`. The six-byte E/1 record validates all fields;
unknown versions are ignored. Load occurs before play and save on graceful exit,
with bounded IPC timeouts. If KV is unavailable preferences remain session-only.
No save-game or replay disk UI or save migration support is claimed. Bundle
replacement does not delete KV preferences; abrupt process death can lose the
most recent unsaved preference changes.

M: mute; -/+: master; [/]: SFX; ,/.: music; N: audio pause/resume (independent of
simulation); Space: gameplay pause (also pauses audio); C: offline credits.
Missing, malformed or unsupported music falls back to procedural effects.
See Licenses/CREDITS.txt before redistribution/public release.
