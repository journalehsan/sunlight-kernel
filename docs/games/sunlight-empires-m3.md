# Sunlight Empires M3

**Final verdict: PARTIAL.** Licensed optional music, credits, preferences,
reproducible native SunApp staging, host stabilization coverage, native PCM
identification, ordinary Search launch/relaunch, and measured guest frame
intervals are implemented and exercised. Arbitrary native SunApp loading is
not implemented by the platform. Native victory remains unverified. The
remaining matrix and distribution review below prevent a production PASS.

Work/evidence: 2026-11-17–18. No commit or push at time of writing. Generated artifacts, captures,
intermediate failures and image manifests are under `target/empires-m3/`.
These artifacts are local evidence, not tracked source or release publications.

## Baseline and scope

Read the actual M1 report (including its M1.1 appendix), M2 report, simulation,
Silicon Echoes soundtrack/shutdown adapter, audiod client/protocol, RAMFS,
`sunlight-libc/src/sun_exec.rs`, existing native/Chronos manifests, and the
Chronos and SunFS installation documents. Some initially searched guessed
paths did not exist; actual platform sources are under `services/` and
`sunlight-ui/app.rs`. The initial checkout was clean.

Repeated baseline: 52 Empires unit tests plus its isolated lifetime test,
three foundation tests and 57 Echoes tests PASS. See `baseline-empires.log`
and `baseline-regression.log`. Historical M2 results were not treated as a
new-source validation. No simulation rules, civilization balance, replay
versions, AI decisions or authoritative checksum encoding were changed.
No Package Manager, App Store, networking, new civilization/era or engine
rewrite was introduced.

## Changes and stabilization

Added presentation-only music policy/PCM helpers, streaming through the existing
native producer, native frame instrumentation, offline credits, per-user KV
preferences, selected licensed resources, and package/conversion/capture tools.
The existing simulation and shared game/Silicon Echo production sources remain
unchanged. New host tests cover both-civilization cooperative cancellation,
distinct approach positions and cooperative work accounting, unreachable-site
rejection without resource/ID changes, and group movement around an obstacle
with duplicate/stale IDs. Existing combat, AI, fog, restart and replay tests
continue to run.

The first added construction tests failed because their site `(12,10)` was
occupied by the starting Guard. This was a test-fixture mistake, not a game
placement defect. `construction-attempt1.log` is retained; the corrected open
site passes in `construction-attempt2.log` and the final suite. Functional
construction/navigation systems were not rewritten to accommodate the tests.

Adding resource entries exceeded Tmpfs's 512-entry static-namespace bound.
`RAMFS_MAX_ENTRIES` is now 576 and a compile-time assertion checks the static
inventory against it. The affected filesystem's 112 host tests pass. Initial
resident assets also caused a 1 GiB boot to fail during kernel heap initialization.
The initial 60-second PCM set was reduced to 30-second excerpts, and original
MP3s remain in the external staging bundle rather than the resident RAMFS image.
No kernel allocator/compositor changes were made. The final default 1 GiB
production gate passes (`production-licenses-final.log` and its serial log).

Earlier intermediate images stalled in VFS startup. Kernel embedding accepts
an existing valid service ELF without checking its source freshness; image
builds sharing release artifacts must be serialized. `tools/empires_m3_fixture.py`
explicitly rebuilds Empires, Echoes and VFS before using the existing init-only
fixture builder. Native evidence before this repair is retained separately.
The M3 wrapper builds those packages together, with Cargo feature unification;
its VFS ELF hash is recorded separately from the ordinary production gate.

## Music provenance and license

The official archive was actually downloaded and inspected: 31 MP3s, no separate
readme/license file, and no composer tags in the selected MP3s. Archive SHA256:
`b04049a17fe33ff5ff87bf243dd81d4b2790e9d68e76f44cf459a1ce63407cea`.
The complete inventory is `archive-inventory.json`; the archive/download logs
remain under the evidence directory. An initial download command failed because
its output directory creation raced a separate command; the subsequent completed
archive is the one hashed and inspected.

| Role | Track from official archive | Credited composer | Attribution verification |
| --- | --- | --- | --- |
| Menu | Honor Bound | Omri Lahav | Official linked Bandcamp track and Omri's Music Blog #2 |
| Peace | The Road Ahead | Omri Lahav | Official linked Bandcamp track |
| Combat | First Sighting | Jeff Willet | Official linked Bandcamp track explicitly identifies composer and combat role |

Sources: [official soundtrack and 3.0 declaration](https://play0ad.com/media/music/),
[archive](https://play0ad.com/files/0ad-music.zip),
[Honor Bound credits](https://play0ad.bandcamp.com/track/honor-bound-0-a-d-main-theme),
[composer's blog](https://play0ad.com/music-blog-2/),
[The Road Ahead credits](https://play0ad.bandcamp.com/track/the-road-ahead),
[First Sighting credits](https://play0ad.bandcamp.com/track/first-sighting).

The archive is offered under CC BY-SA 3.0. Current Bandcamp pages link to 4.0
and carry a 2018 Materia Collective release notice. These additional notices
and performer credits are preserved, with an explicit distinction between the
archive files and the later recording release. No later Bandcamp audio download
is substituted for the official archive. Full per-file SHA256s, credits,
verification URLs, converter version and modifications are in
`SunlightEmpires.sunapp/Licenses/attribution.json`.

Retained original MP3s are unmodified. Playback WAVs take the first 30 seconds,
decode/resample to 48 kHz stereo signed 16-bit PCM, add a 20 ms fade-in and a
one-second end fade, and loop at EOF. There is no loudness normalization.
`tools/empires_music.py` reproduces the conversions and rejects input/output
hash drift. Canonical WAV headers are emitted by Python's wave module.
The WAV adaptations retain CC BY-SA 3.0. No other 0 A.D. assets are used.

Human-readable credits and full 3.0 legal code are included, together with
Inter/Fira Code, Noto, Material Icons and OpenMoji font notices. The game uses
its existing original geometric art, launcher icon and procedural effects.
Final review found that the system package-level Noto notice was Apache while
the actual embedded Noto Serif font identifies OFL 1.1 and Copyright 2022 The
Noto Project Authors. The incorrect first notice is retained under
`font-notices-attempt1/`; the final bundle uses the matching upstream OFL text
from notofonts/latin-greek-cyrillic. Music and executable bytes did not change.

**Public release review remains open.** CC BY-SA 3.0's adaptation definition
addresses synchronization with moving images. Review the obligations for
combined interactive audiovisual distribution and recordings before public
release. No top-level game-source license grant was found in this checkout;
resolve the game/platform dependency license inventory separately. The music
license does not license the game source. See the included notices and
[3.0 legal code](https://creativecommons.org/licenses/by-sa/3.0/legalcode).
No endorsement, original-Sunlight-music claim or completed legal clearance is made.

## Audio architecture and controls

One existing audiod producer thread mixes music into the existing procedural
SFX chunk. It owns at most one open track and fixed 4096-byte music/PCM buffers;
whole tracks are not loaded into the game's heap. Files are opened read-only
at `/Applications/SunlightEmpires.sunapp/Resources/music/`. File reads, PCM
submission and audiod IPC occur on the producer, outside GUI/simulation ticks.
Canonical 48 kHz stereo PCM WAVs are supported; malformed/unsupported headers
fail closed. Missing/read-failed resources retry with bounded delays and leave
SFX/gameplay available. Header corruption is host-tested; native corrupt/missing
resource injection has not been completed.

Menu/peace/combat transitions replace the single track and stop its queued tail;
there are no overlapping background tracks. Combat activity holds combat music
for 12 seconds, with an eight-second minimum switching interval. Menu/restart
and terminal state are presentation decisions, outside World/checksums. Terminal
matches choose silence; separate victory/defeat music is not supplied.

M mutes; -/+ controls master; [/] SFX; ,/. music; N independently pauses/resumes
all audio while gameplay continues. Space pauses gameplay and audio. C opens
credits without network access. Credits display title, version, compatibility,
track/composer/license/source/modification information. URLs are displayed for
transcription and are present in copyable offline text files; the Canvas screen
does not implement selectable/copy-to-clipboard links. That UI requirement
remains incomplete.

The existing audio.v1 global application queue grants an exclusive lease, not
cross-application mixing. Handoff was tested with Empires explicitly paused and
Echoes subsequently closed gracefully. Continuous music does not automatically
release on focus loss; switching focus alone is not claimed as a handoff test.
Existing submitted-frame ownership checks and STOP/shutdown behavior remain in
use. A stuck worker's stack/TLS are not unmapped underneath it; the existing
bounded shutdown fallback retains mappings until process exit.

## Bundle and user data

`python3 tools/package_empires.py` produces the unpacked staging directory:
`target/empires-m3/package/SunlightEmpires.sunapp`. The final inventory reports
21 files, 27,301,752 bytes. `package-inventory.json` records every file size/hash.
The tree contains the native ELF in Program/, an existing-format Manifest.toml,
icon, three WAVs, three original MP3s, compiled-content/SFX source definitions,
credits, machine-readable attribution and license texts. Procedural civilization
rules/SFX/fonts remain compiled in the ELF; no source-tree runtime path is used.
No test recordings, ISO images, system shared-library duplicates or developer
absolute paths are intended bundle resources.

The manifest follows VoiceRecorder/WiseOwl: bundle.format=1, runtime.type=native,
application.kind=windowed and entry.executable=/bin/sunlight-empires. The actual
native app-id/executable registry supplies that ELF. RAMFS stages playback
resources/notices at /Applications/SunlightEmpires.sunapp; it omits original MP3
source copies. Search finds the existing Games catalog entry and uses sun-exec.
No special-case Search entry or registration bypass was added.

**Platform gap:** direct `.sunapp` launch dispatch currently validates Chronos
only and rejects runtime.type=native. The native examples reference registered
/bin entries; no arbitrary Program/ ELF loader, signed activation, native bundle
installer, dynamic bundle discovery, package dependency solver or App Store is
implemented. Thus the generated bundle contains all app payloads, but copying
it alone is not an end-to-end native installation. Replacing the staged bundle
alone does not replace the embedded executable. See the bundle README and
`docs/chronos/prompt-3-bundles-and-storage.md`; the SunFS installation contract
is explicitly future work, not an implemented installer.

Runtime requirements are the existing native ELF ABI, SGP Window/SHM, libc
thread/futex/clock services and VFS. audiod/output hardware and sunlight-kv are
optional. Per-user settings use the existing KV service under
`app.empires.settings/<uid>/v1`. The six-byte E/1 format validates all fields and
ignores unknown versions. Load precedes gameplay; graceful exit saves with bounded
IPC. Native independent process relaunch retained the tested volume values.
Power-cycle durability with a persistent state disk was not tested. The app
never writes into the bundle; current static RAMFS resources are staging, not a
new installation or filesystem convention. No disk save-game/replay UI or
save migration is claimed. Bundle removal does not delete the KV record.

## Final source validation matrix

| Check | Result | Evidence / practical limit |
| --- | --- | --- |
| Empires host rules, combat, AI, fog, outcome/restart | PASS | verified-final-host.log: 60 unit tests |
| Isolated host allocation recovery | PASS | Same log: one lifetime test, 128 worlds |
| Added M1.1 approach/work/cancel/unreachable/group movement | PASS | Four new actual-world tests, both civs for cancellation |
| Entire original M1.1 native/civilization/overflow matrix | NOT_RUN | Host coverage does not close every native variant |
| Deterministic winning command replay | PASS | tick 6596, checksum 84f42295ee24cb8c, 112 commands; reload reproduces |
| Shared foundation / Silicon Echo host tests | PASS | final-regression.log: 3 / 57 |
| Filesystem host tests after namespace-bound change | PASS | fs-host.log: 112 |
| Native releases | PASS | native-staging-final.log and validated-fixture.log |
| Final ordinary 1 GiB production boot gate | PASS | production-licenses-final.log and serial; earlier production-boot-final also passes |
| Preliminary native winning gameplay attempt | FAIL | Real commands ended in defeat tick 5794; retained separately from final revision |
| Final-ELF native command-driven victory | NOT_RUN | Unverified; no test-only outcome mutation or forced victory used |
| Native victory UI, post-victory AI freeze and restart | NOT_RUN | Host terminal freeze/restart passes; no native victory claim |
| Native default-AI combat / cooperative house / group move | PASS | validated-native trace and screenshots 04–07 |
| Native combat restart | PASS | screenshot 10 and fresh initial resources |
| Menu, peace and combat track playback | PASS | Native WAVs plus expected/wrong-source comparisons below |
| Master/music/SFX zero and nonzero, mute | PASS | controls-analysis-final.json; corrected pre-input SFX capture retained |
| Independent audio pause/resume and gameplay pause | PASS | Captures 03/04/11 and continuing native pacing ticks |
| Clean exit and independent ordinary relaunch | PASS | Captures 17/19 silent; Search source=shell PIDs and screenshots 11–14 |
| Bidirectional Echoes audio handoff | PASS | Captures 15/16; explicit pause and graceful Echoes close |
| Both games without audio hardware | PASS | missing-hardware/ trace, screenshots and native suite PASS |
| Native missing/corrupt/unsupported music files | NOT_RUN | Parser/unit handling covered; no native fault injection |
| Deliberately hung audio worker/service | NOT_RUN | No supported STOP/CONT injection performed |
| Native absent audio service during an existing match | PASS | Actual audiod PID 21 SIGKILL/code 137; match continued and exited cleanly at tick 1217 |
| Service failure during verified active playback / absent-service startup | NOT_RUN | Pre-kill PCM was silent; a later standalone sun-exec retry returned app not found |
| Bundle schema/content/hash and conversion checks | PASS | package-final.log, conversion-final.log and inventory |
| Selected music attribution/license inclusion | PASS | Original hashes, official sources, credits and legal code included |
| Public distribution legal/license review | BLOCKED | Audiovisual ShareAlike and separate source/dependency licensing review |
| Packaged runtime resource resolution through ordinary Search | PASS | Existing registered ELF opens staged music; not arbitrary bundle loading |
| Standalone native bundle installation/discovery/activation | BLOCKED | Native bundle loader/installation contract absent |
| Per-user preferences record and process relaunch | PASS | Versioned codec tests, native save logs and retained settings |
| Save migration / power-cycle persistence | NOT_RUN | No save-game disk UI; persistent-state reboot not tested |
| Selectable/copyable URLs inside Credits Canvas | NOT_RUN | Offline text is copyable; native Canvas interaction incomplete |

## Native audio evidence

The final native pair is audited byte-for-byte in validated-native/image-audit.json:
Empires `1e554442e2095f520f8225aea0471ab02655a8857eb3ccc30e0029ef7abb4898`,
Echoes `6106abaa4e435d8b441ebaec7b2bf6ba06f0a1e29de2bbf7b50d6a0443d4b031`.
Fixture kernel SHA256 is
`e4a558024f3af2f769b53926595a6b17e9a4a8844681e36190330d2cef3c3df4`.
The fixture changes init launch only and restores exact source bytes. Its direct
spawn is not Search evidence; later independent shell launches are separate.

QEMU HDA captures actual guest output at 44.1 kHz stereo s16le after backend
resampling from the submitted 48 kHz stream. Segments preserve PCM bytes and
reconstruct WAV headers; capture-offsets.jsonl identifies byte ranges. The QEMU
raw file has zero placeholder RIFF/data lengths and is not itself a valid
finalized WAV in the observed run. Preserve it; `tools/empires_wav_finalize.py`
creates a separate header-corrected copy without changing PCM. This limitation
must not be hidden by calling the original raw file valid.

| Capture | Peak | RMS | Expected-source envelope correlation | Wrong-source control |
| --- | ---: | ---: | ---: | ---: |
| Final menu | 1677 | 399.5 | 0.9987 | 0.4637 (peace) |
| Final peace | 1208 | 280.0 | 0.9726 | Not separately computed |
| Final combat | 8331 | 1359.0 | 0.8532 | 0.3055 (peace) |
| Corrected SFX-only | 1683 | 240.4 | Not a soundtrack identification test | — |
| Echoes handoff | 9562 | 1955.4 | Existing Echoes music; not compared to 0 A.D. | — |
| Reverse handoff | 7667 | 1355.0 | Native Empires output | — |

Captures for audio pause, music zero with no cues, SFX zero with music zero,
master zero, mute, gameplay pause and both measured post-exit periods have peak
and RMS zero. All listed final control captures have zero clipped samples.
Master 90/music enabled produces nonzero output (peak 4552, RMS 981.6); different
musical phrases mean these RMS values are not a calibrated linear-gain test.
Host PCM tests check gain and saturation mathematically.

Identification uses three band-limited RMS envelopes (80–400, 400–1400 and
1400–3500 Hz), 20 ms bins, an energetic up-to-three-second capture window and a
search over the expected excerpt. This is stronger than a non-silent WAV but is
not physical listening or proof of uninterrupted playback. The analyzer also
records zero runs and adjacent-sample deltas; fades, square-wave SFX, pauses and
transitions make those unsuitable as unconditional glitch detectors. Audiod
starvation/underrun diagnostics occurred under load. Glitch-free playback,
seamless loop quality and all-load discontinuity bounds are not established.

## Native frame pacing and memory

Instrumentation measures guest wall-clock intervals between draw starts,
drawing duration, simulation step duration, steps/frame, interval bands and
allocator counters. It excludes menu/pause/terminal gaps from interval budgets;
new games reset the sample series. Native observed clock quantization is 10 ms:
a reported 0 ms means below that resolution, not zero computational cost.
Drawing timing excludes Window commit/poll/compositor waits. Intervals include
them. No physical display/input latency or compositor presentation timestamp is
claimed. QEMU used KVM, two vCPUs, 2 GiB for interactive evidence, 1280×800 desktop
and a 960×680 game window. Other host VM activity was not controlled.

Representative cumulative records from the final fixture:

| Scenario coverage / record | p50 / p95 / p99 interval | Maximum | Intervals >50 ms |
| --- | --- | ---: | ---: |
| Initial idle/peace, tick 252, frames 256 | 50 / 60 / 60 ms | 110 ms | 112 |
| Audio controls and pause/resume, tick 1325, frames 1408 | 40 / 60 / 90 ms | 180 ms | 522 |
| 2×, construction/movement/fog, tick 3172, frames 3072 | 40 / 60 / 80 ms | 190 ms | 567 |

At the third record the interval bands [0–16,17–33,34–50,51–100,101–250,251+]
are [27,1340,1106,552,15,0]; steps/frame [0,1,2,3,4,5+] are
[137,2739,163,28,2,3]. These are measured distributions, not an invented average
FPS. The first two frame-work percentiles are below 10 ms; drawing p99 reaches
10 ms in the later workload, while simulation p99 remains below 10 ms.
Combat and later process records remain in the raw trace. These cumulative
windows combine scenarios; a fully isolated per-scenario benchmark matrix is
not complete. The >50 ms count is a reference-budget exceedance count, not a
measured scheduled/compositor missed-deadline counter. Audio/input/commit stall
attribution and sub-millisecond tick timing remain unresolved; no unrelated
kernel or compositor optimization was attempted.

Final native simulation/replay stress: 64 scene lifetimes, 51,200 ticks,
zero live allocation/byte recovery, high-water 160,736 bytes. The final fixture's
12-window suite returns to zero live allocations/bytes in 2490 native ms.
The no-HDA suite also passes, with 2570 ms window churn. The high-water is a
process allocator value shared with preceding diagnostics, not total OS/game
RSS. During the measured first match live requested bytes are around 60–63 KiB;
allocation counters grow (formatting/events allocate), so this is not an
allocation-free-frame claim. Existing 8 MiB heap, separate framebuffer SHM and
2 MiB audio-thread stack remain. Large-army native pacing and total compositor/
kernel memory growth have not been quantified.

## Reproduction and next milestone

```sh
cargo test -p sunlight-empires --target x86_64-unknown-linux-gnu -- --nocapture
cargo test -p sunlight-game -p sunlight-silicon-echoes --lib --target x86_64-unknown-linux-gnu
cargo test -p sunlight-fs --lib --target x86_64-unknown-linux-gnu
python3 tools/empires_music.py
python3 tools/package_empires.py
SUNLIGHT_TEST_SERIAL_LOG=target/empires-m3/repro-boot.serial.log ./tools/test.sh phase3.0
python3 tools/empires_m3_fixture.py --output target/empires-m3/repro-native
```

Serialize image builds. Boot the separate fixture with the actual QEMU command
in validation-scripts/validated-boot.py (two vCPUs, 2 GiB, KVM, virtio-rng, USB
mouse, HDA WAV backend and QMP). The script uses the development login; its
launch is diagnostic. Scripts final-controls.py, combat-and-sfx.py and
relaunch.py preserve ordinary UI input/capture sequences. Inspect screenshots
before accepting pointer/focus-sensitive actions. The initial service-failure
attempt reached the game, not Terminal, and is explicitly invalid evidence.
The resumed run also retained pointer-navigation and command-entry failures.
No successful service fault may be inferred from its misleading capture name.
Attempt 3 actually killed audiod through /bin/kill: the kernel confirms PID 21
SIGKILL/code 137. Both immediate pre/post captures were silent, so this verifies
service absence handling, not failure during active output. After that death the
game restarted, ran at 2× through tick 1217, and exited cleanly with checksum
658bdc600de53022. A host default-match replay reproduces that exact checksum.
Native no-HDA tick 5 (181e2f254177bc49) and ordinary music-enabled tick 90
(b1227c0c85ee92bb) also match host replay; see native-checksum-comparison.log.
A subsequent standalone sun-exec command returned app not found; it is not
counted as an absent-service independent relaunch. The ordinary Search
launch/relaunch checks before the fault remain separate successful observations.
Use empires_audio_analyze.py on reconstructed segments and compare wrong-track
controls. Python standard library, Pillow (QMP pointer matching), ffmpeg,
QEMU/KVM, xorriso/Limine and the repository toolchain are the relevant tools.
Optional numpy/scipy installation attempts were unnecessary; the final analyzer
uses standard Python and ffmpeg. Converter drift is rejected rather than silently
updating asset hashes. Game/package reproducibility is hash-recorded; universal
bit-identical builds across compiler/ffmpeg versions are not claimed.

Normal init source was restored by each fixture builder. Ordinary init/kernel/
ISO artifacts were rebuilt afterward (`production-restore.log`), replacing
fixture launch behavior in target/sunlightos.iso.

Recommended next milestone: native command-driven victory and post-victory
freeze/restart closure; remaining M1.1 native variants; isolated pacing/audio
failure/hung-worker tests; native missing/corrupt resources; clipboard credits;
and a separately scoped native bundle loader/registration contract. Resolve
public-distribution licensing review before release. None requires implementing
a Package Manager, App Store, new historical content or multiplayer.


## Continuation and final evidence boundary

The native VM remained running across the user-requested continuation. The long
WAV therefore includes an unattended interval; it is not a controlled benchmark
window. Reported pacing uses the identified earlier records, and track/control
comparisons use bounded segments with stored byte offsets. The final raw capture
is preserved, and a separate finalized WAV has matching PCM SHA256 after header
repair (`validated-native/wav-header-repair.json`). Native QEMU instances started
for M3 were closed after evidence collection; unrelated pre-existing VMs were
left untouched.

The final notice correction changes only Noto licensing text and human-readable
credits, not the validated game ELF or music payload hashes. The package was
rebuilt/revalidated, and the default production gate repeated for those final
resource bytes. The earlier diagnostic ISO retains the original incorrect Noto
notice as failed intermediate licensing evidence; it is not the distribution
bundle. Use the final package tree, not an intermediate fixture ISO, for review.

Final production audit (`final-production-audit.json`) confirms exact current
Empires/init/VFS ELF bytes in the ordinary kernel, absence of the fixture launch
marker, inclusion of the corrected Noto OFL text, and restored init source SHA256
8f5a30bbcb6b73f473fa3b0cfa9b78575a7a46e216098949e28819ec2f77e22d.
The final license-resource production gate passes at the default 1 GiB setting.
