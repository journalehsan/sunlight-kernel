# Sunlight Empires M4

Verdict: PASS for M4 playable-game acceptance. Prior M3 validation and
distribution limitations remain explicitly open below. Evidence: 2026-11-18.

M4 adds contextual Canvas commands, military buildings, bounded paid production,
group gathering and a separate deterministic Combat Sandbox. No commit/push,
networking, engine replacement, new sprites or unrelated kernel changes.

## Repository findings and implementation

Inspected `lib.rs`, `combat.rs`, `content.rs`, `input.rs`, `render.rs`, `main.rs`,
replay, audio/sound/music, M1/M1.1–M3 tests/reports, package tooling, native
fixture/QMP tooling and production gate. Existing construction, ownership,
movement, combat, fixed ticks, worker delivery, audio and Canvas are reused.
Previously the bottom buttons were unconditional, gathering chose only the first
worker and the Town Center trained every unit with one active slot.

The bottom 176 pixels now show entity/group details, applicable actions and a
right-hand paid queue. Worker actions expose House, Storage, Barracks, Archery
Range and University, with wood costs; selection switches production actions.
Mixed worker/military selections expose the union of eligible actions. Commands
filter eligible owned units. Military actions include Move, Attack, Attack Move,
Hold and Stop. No selection shows civilization, settlement era and controls.
Worker details show task and cargo; groups show composition. Buttons and queue
slots share render/hit-test geometry. Formatting uses stack buffers rather than
per-frame HUD Strings. Building clicks take priority; panel presses clear world
drags and releases outside the viewport cannot select world objects. Escape
cancels targeting/placement before opening the menu. Existing camera/zoom,
shift/drag selection, audio and speed shortcuts remain.

Town Center trains Worker (30 wood, 60 ticks); Barracks trains Infantry/Guard
(35 wood, 80 ticks); Archery Range trains Archer (45 wood, 100 ticks). Barracks
costs 80 wood; Range costs 100; both take 120 single-worker work ticks. Existing
cooperative speed accounting, 400 health, one-tile occupancy, ownership,
placement/cancel/destruction rules apply. Military buildings have labels and
additional geometric marks. Initial resources and ordinary match setup are
unchanged.

## Resource and production accounting

The inherited militia economy uses wood for all production/building costs.
Food, stone, gold and knowledge retain their existing meaning; this milestone
does not invent food income or a technology economy. HUD reads authoritative
resources and population including paid reservations. Civilization, era and
current paused/0.5x/1x/2x speed are visible. Unaffordable actions appear red.

Each building holds at most five paid items, including the active item. Admission
checks owner, completed production type, funds, population, five slots and the
256-unit global bound including all reservations. Costs and population are
reserved on authoritative admission, never on UI submission. An integer tick
counter runs only the active item. Successful completion spawns on the first
safe deterministic adjacent tile. A blocked exit or lost housing holds the paid
item; no unit is created. Waiting items advance FIFO. Canceling a waiting item
preserves active progress; canceling the active item starts the next at its full
duration. Each cancellation refunds exactly its original wood cost in full.
Destruction discards production/reservations without refunds. No completed
units or paid items can be canceled twice.

Queue slots display active percentage, waiting units and click-to-refund. Train
requests report specific type/funds/housing/queue/global-limit problems. Rally
can be set with its button/R or ordinary right-click on a selected completed
producer. Workers move to rallies; soldiers use existing attack-move behavior.

Right-click gathering submits sorted/deduplicated group commands. It tries the
clicked node then reachable visible nodes of the same resource within four
tiles, retaining deterministic ordering. Existing cargo markers and exactly-once
delivery remain. Workers reserve distinct approaches; failed gather path
replanning releases stale routes. Repeated unreachable approaches retire the
task. Missing/unreachable drop-offs retain cargo and eventually idle instead of
spinning indefinitely. Depleted nodes retire after carried resources are
returned; players can reassign to another patch. The placement preview checks
terrain, unit occupancy and affordability; reachability remains authoritatively
validated on confirmation.

## Beginner pacing and sandbox

The 3600-tick (180-second) peaceful period, 1200-tick attack interval and army
limits remain. AI now buys/constructs military production through normal group
commands, uses the new gathering distribution and rallies new soldiers away
from production exits. No free resources/units or immunity were added to normal
matches. This is a necessary rules migration, not an anecdotal difficulty tweak.
Reproducible tests require preparation before the unchanged first attack,
unattended defeat and a command-driven economic victory.

Press F in the main menu for RTS Combat Sandbox (P/G remain normal matches).
The sandbox starts with 800 wood, Town Center, House, Barracks, Range, three
workers, four friendly soldiers and a nearby enemy base with two soldiers.
Starting entities/resources are explicit scenario data. Normal training,
commands, visibility, damage, AI and Town Center outcome rules apply; there is
no forced outcome. The replay header and checksum include the scenario flag.
Normal starts cannot inherit it.

## Determinism and host evidence

Rules version 4/content version 3 explicitly reject older-version replays.
New building tags append existing tags. Queue contents, active progress,
scenario choice and new commands are included in canonical state/save recovery.
Timing, entity ordering and simulation independence from audio/rendering remain.

`target/empires-m4/host.log`: 72 Empires unit tests plus the isolated lifetime
test pass. New coverage includes all three unit types, FIFO work, active/waiting
cancellation/refunds, bounds, affordability, ownership, blocked exits, housing
loss, producer destruction, military cooperative construction, group gathering
conservation, lost drop-offs, scenario isolation and genuine sandbox victory.
Replay equality covers production, cancellation, rallies, construction, group
gathering and group combat. The updated M2 economic strategy constructs and
rallies a Range through ordinary commands and wins at tick 5605, checksum
`87b4e9cf483f8310` (126 commands); its replay reproduces. Original M1.1,
combat/AI/fog, audio/M3 and timing tests remain in the suite.

`target/empires-m4/regressions.log`: three shared foundation and 57 Silicon
Echoes tests pass; shared game/Echoes sources are unchanged.

The isolated allocation test now cycles normal economy, normal match and sandbox
starts, queued training/cancellation and replay recovery over 128 lifetimes,
requiring the exact starting allocation count and requested bytes after each.
It passes. Native simulation and window lifetime checks also pass, as recorded
below. These allocator checks do not establish total OS/compositor RSS bounds.

## Native, SunApp and audio evidence

The ordinary production `./tools/test.sh phase3.0` gate passes at its default
1 GiB/two-vCPU configuration (`boot.log`, `boot.serial.log`). The separate
interactive guest used the same ordinary ISO, KVM, two vCPUs, 2 GiB, a 1280×800
desktop, USB mouse and HDA WAV backend. No fixture image or gameplay state
injection was used for acceptance. `production-image-audit.json` verifies exact
current Empires/init ELF inclusion and absence of the fixture-launch marker.
Empires ELF SHA256:
`a2d6d0cd315ef4fa5e4f810fd02cf58cb8c1372ea13751c3332742921a59c0fd`.
The staged executable matches these same bytes.

Native artifacts are under `target/empires-m4/native/`: `native.serial.log`,
`qmp-actions.jsonl`, bounded PCM captures and numbered screenshots. The
ordinary Search launches are independently logged as `source=shell`, PIDs 39
and 41. The following results were observed in actual guest state:

| Acceptance check | Result and evidence |
| --- | --- |
| Ordinary Search/menu/normal start | PASS; screenshots 03–06 |
| Town Center worker training | PASS; 30 wood charged, visible 23% progress at 0.5x, completion at tick 87; 07–09 |
| New-worker gathering/delivery | PASS; wood 210→250 through four real deliveries; 10 and serial |
| Group selection/gathering | PASS; IDs 2/3/4 assigned together, balance increased to 350; 11–12 |
| Placement feedback/cooperative House | PASS; occupied TC preview is red; one House charged 40 wood and completed; 13–14 |
| Barracks construction | PASS; one group project charged 80 wood and completed; 15 |
| Infantry FIFO/cancellation | PASS; three paid items charged 105 wood, active/waiting slots visible; cancel slots 1 and 0 refunded 70; 16–18 |
| Rally points | PASS; Barracks (11,7), Range (10,7), visible flags and soldier destinations; serial/17–22 |
| Range/archer production | PASS; group construction charged 100 wood, four timed archer completions; 20–22 |
| Military/group actions | PASS; two infantry/four archers selected with one worker, combined actions/composition; 23–24 |
| Attack-move/combat | PASS; ordinary input submitted target (36,36); melee/arrow/death events and casualties; 28–33 |
| Pause/0.5x/1x/2x/camera/zoom | PASS; paused command ticks stayed fixed across long pointer sequences; 07/25/27 and input trace |
| Normal terminal outcome | PASS; six-soldier attack lost, AI destroyed player TC; Defeat event tick 5323, world stopped at 5324; 35 |
| Exit and independent Search relaunch | PASS; clean exits and distinct PIDs; confirmed retry 40–42 |
| Separate Combat Sandbox | PASS; F starts 800 wood, prepared production buildings and separate armies; 42–43 |
| Sandbox genuine victory/health reduction | PASS; attack-move to (22,10), enemy TC destroyed, Victory tick 462; surviving Guard HP 76/100; 44–46 |
| Native normal restart/scenario isolation | PASS; diagnostic interactive windows started normal Persia, then restarted Greece with ordinary 240 wood; 50–51 |
| Font/music/resource preservation | PASS; rendered game text, packaged-resource open logs, PCM source comparisons and package hashes |

`native-host-replay.log` rebuilds authoritative host worlds from the actual UI
command trace and matches complete guest checksums, then save/reloads each:

- Normal match: Defeat, tick 5324, 18 UI commands, `2a7ff47ccaf85359`.
- Sandbox: Victory, tick 462, one group attack-move, `4ba546311ccf5a03`.

The losing normal six-soldier attack is not presented as a native winning
strategy. The host economic victory uses a larger army. No balancing change was
made to turn the observed loss into a win. Normal-match native victory remains
unverified; native normal defeat and sandbox victory satisfy the requested
victory-or-defeat acceptance paths.

An initial relaunch sequence was intercepted by the pre-existing Welcome
window. Captures 36–38 show Welcome, not a game launch, and are not counted as
successful relaunch/sandbox evidence. The repeated Search at 40 and new PID 41
are the successful observations. A later pointer-template lookup failed over
Terminal; moving onto the desktop and repeating Search produced the confirmed
Echoes launch. Earlier host implementation failures are retained under
`target/empires-m4/attempts/`; final suites supersede them.

## Native performance, lifetimes and regression smoke

After gameplay acceptance, Terminal ran the existing `--native-suite`
diagnostic entry point against the same executable. This was used only for
lifecycle diagnostics, not for gameplay outcomes. Its 64 simulation/replay
lifetimes report PASS, 51,200 simulated/replayed ticks, 520 native ms, zero live
allocations/bytes before and after, high-water 160,960 requested bytes. Both
interactive windows were closed normally. Twelve window/audio lifetimes then
report PASS in 2860 ms, with zero live allocations and requested bytes, followed
by `native fixture suite PASS` (the existing diagnostic's marker name does not
mean a fixture image was used).

Representative normal-game cumulative pacing at frame 4096: draw p50/p95/p99
[0,0,10] ms and simulation [0,0,0] ms, 5237 ticks, 59 live allocations/66,524
requested bytes. At frame 3840, frame intervals p50/p95/p99 were [30,70,100] ms,
maximum 330 ms, 296 intervals over 50 ms. These mixed-speed cumulative windows
include input/render/compositor waits; they are not isolated FPS benchmarks or
an improvement claim. Clock granularity is 10 ms, so zero means below that
resolution. Total OS/framebuffer/compositor RSS and large-army deadlines remain
unquantified. Command logs, queued items and additional entities legitimately
increase live allocations during a match; lifetime recovery is the leak check.

Silicon Echoes: three foundation and 57 Echoes host tests pass. In the same
guest after Empires shutdown, ordinary Search launched Echoes (PID 60), rendered
its title, entered the first scene and exited normally (54–57). Its HDA capture
has peak 9568, RMS 1820.3 and no clipped samples; the post-exit capture is silent.
This is a native launch/play/audio/shutdown smoke test, not a complete native
story/save regression. The initial title reported the existing save service
asleep/session-playable condition; durable save persistence is not established.

Menu music source-envelope correlation was 0.9969 versus 0.5265 for the wrong
peace source. Quiet peace capture was 0.9351 against its source. Construction
and combat captures contain SFX and provide weaker music correlation; they are
not described as clean source-identification tests. Empires and Echoes measured
post-exit segments are silent. No physical listening or glitch-free playback
claim is made. The stopped VM's original WAV has zero RIFF/data size fields;
`native-audio-finalized.wav` repairs only those fields and has identical PCM
SHA256 (`wav-header-repair.json`). The original remains preserved. The task's
interactive VM is no longer running; the unrelated pre-existing VM was left
untouched.

The existing packaging tool builds the production native ELF and verifies the
21-file staging tree, manifest, resources, music hashes, font notices and
licenses. Selected assets, licenses, audio leases, settings and music code are
unchanged. Existing Started, Selection, Movement, Invalid, Completed and Trained
cues provide training/placement/rally/construction feedback. Simulation rejection
no longer triggers an additional duplicate UI invalid cue. Duplicate pointer
release events do not repeat selection audio.

## Known limitations and recommended M5

Preserve M3's unresolved native bundle installation/loading contract, public
release licensing review, clipboard credits, native fault/audio/hung-worker
matrix, physical listening/glitch and large-army/RSS validation gaps. Existing
staging remains staging, not arbitrary SunApp activation. No license clearance
is claimed. Original licensed attribution and bundle notices are unchanged.

Recommended M5: add idle-worker guidance and resource-patch retargeting;
validate a native normal-match winning strategy and larger-army/frame pacing;
exercise narrow-window accessibility and remaining M1.1/native audio fault
variants. Scope native bundle activation and public-distribution review
separately. LAN work should retain the now-verified ordinary UI progression.

Reproduction commands:

```sh
cargo test -p sunlight-empires --target x86_64-unknown-linux-gnu -- --nocapture
cargo test -p sunlight-game -p sunlight-silicon-echoes --lib --target x86_64-unknown-linux-gnu
python3 tools/package_empires.py
SUNLIGHT_TEST_SERIAL_LOG=target/empires-m4/boot.serial.log ./tools/test.sh phase3.0
python3 target/empires-m4/replay_native.py
```

Serialize image builds. The native boot/input drivers and exact captured inputs
are local artifacts under `target/empires-m4/native/`. Window origins cascade;
inspect screenshots before replaying pointer coordinates. No commit or push was
performed. No dependencies, music/license payloads, shared foundation, Silicon
Echoes production source or unrelated kernel code changed.
