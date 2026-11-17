# Sunlight Empires M2

**Verdict: PARTIAL.** The playable combat/economy/AI loop is implemented and
host/native evidence is recorded below. The full requested validation matrix
and quantified native performance closure remain incomplete.

Implementation and validation: 2026-11-17. No commit or push requested or made.

## Baseline and prerequisite repairs

The starting checkout was dirty, including M1.1 changes in Empires and broad
unrelated package-version changes. These were preserved. Read M1/M1.1 findings
in `sunlight-empires-m1.md`; that report is not superseded by screenshots.

Before production changes: 16 Empires library tests, three shared foundation
tests, and 57 Silicon Echoes tests passed. Both ordinary native release binaries
compiled. Evidence: `target/empires-m2/baseline-host.log` and
`baseline-native.log`. An initial invocation incorrectly requested an Echoes
`native` feature that does not exist; the corrected build passed.

M1.1 remains **PARTIAL**: its entire requested matrix and civilization-specific
native matrix have not been closed. Historical ordinary-launch/stat failures
remain unresolved unless explicitly tested below. Separate prerequisite repairs:

- Group construction now finds a reachable eligible worker instead of requiring
  the first ID to have a route. Placement/funds checks precede route mutation.
- Crossing future movement reservations resolve by stable ID priority. A unit
  already traversing a segment keeps its reservation; two centered units no
  longer wait forever for the same future cell.
- Exhausted wood/stone becomes walkable ground, after cargo type is recorded.
  This allows access into harvested forests and preserves stone deliveries.
- Completed attack-move/rally orders settle instead of repeatedly selecting new
  nearby destinations. Military work state records arrival; combat interruption
  resets it so the original destination can resume afterward.

## Production changes and rules

The existing no_std simulation, command envelope, normalized group dispatch,
pathfinder, replay, Canvas, Window/App and shared `sunlight-game` foundation
remain in use. No engine or networking was introduced. Shared engine production sources were not changed. Echoes received one native
shutdown prerequisite repair after the audio handoff test exposed an orphaned
stream lease: its ordinary window now requests the existing six-second close
grace, drops the app on connection failure, and completes soundtrack cleanup
before process exit. Its story rules, rendering and music synthesis are unchanged.

`World::new` retains the M1 settlement scenario for regression/diagnostics.
`World::new_match` is the production two-owner match, selected by the native UI.
Entity IDs monotonically increase and vectors retain creation order. Both
civilizations have identical rules. Rules version 3/content version 2 deliberately
reject older replay versions; the existing replay header remains version-gated.

Combat balance is in `content.rs`. Infantry retains the public `Guard` enum name:
100 HP, 12 damage, range 1, cooldown 20 ticks, speed 48 subunits/tick. Archer:
65 HP, 8 damage, range 5, cooldown 25, speed 40. Workers have 50 HP and no attack.
A tile is 256 subunits; ticks remain 50 ms. Range is integer Manhattan distance
between actual centers, not truncated tile coordinates. Sight is Manhattan
radius 7 for units and 9 for buildings; terrain does not occlude sight or arrows.
Buildings have 400 HP, including construction sites.

Attack, attack-move, stop and hold use the existing command envelope and the same
64-member sorting/deduplication/ownership pipeline as movement and construction.
Attack accepts only currently visible live enemies. Idle military defends itself;
stop clears its order and returns to idle defense, while hold attacks in range
without chasing. Attacks use stable target distance/ID tie breaking. Melee seeks
reachable, reserved adjacent cells; excess attackers retry. Archers stop when in
range. In-range damage is immediate; six-tick arrow effects are cosmetic.

All units alive at a tick's combat boundary can fire a simultaneous volley.
Damage then applies in attacker ID order with saturating subtraction. Dead units
and buildings are retired once, without death refunds. Dead or hidden targets
clear attack orders; a current movement segment completes safely. Effects are
bounded at 128 and sound events at 64. Neither enters authoritative checksums.

Town Centers train workers (30 wood/60 ticks), infantry (35/80) and archers
(45/100). Training reserves population and debits once, waits inside if all exits
are occupied, and honors rally points. Town Centers/houses each provide five
population. Total units plus training reservations are capped at 256. There are
no barracks or separate production queues in this milestone.

**Temporary economic simplification:** all training costs wood. Food, gold and
knowledge retain their M1 UI balances but are not training inputs. Neither side
receives passive income; harvesting finite wood and delivering cargo pays for
production. This gives both sides the same replenishment rule without adding a
farming subsystem. Existing construction cancellation refunds once; combat
losses never refund a building or training investment.

Visibility and explored history are separate arrays per owner, updated at fixed
simulation boundaries. Unexplored tiles are black, remembered terrain is dim,
and hidden enemies are omitted from rendering and rejected as attack targets.
There are no remembered enemy-building silhouettes. Rendering never discovers
information for simulation.

Beginner AI starts with a Town Center, three workers and 180 wood. Every 100
ticks it submits ordinary envelopes to build a house/storage, gather visible
forest resources, train, defend visible intruders near home, and prepare attacks.
It rallies away from its production exits. Offensive orders begin no earlier
than tick 3600 (three simulation minutes), with at least three soldiers, at
1200-tick intervals. Initial army cap is four, rising to seven over later
2400-tick periods; outgoing groups increase from three to six. Economy, terrain,
production time, capacity and sight checks are shared with the player.

AI patrols toward the public map's starting-region waypoint (8,8); it never
looks up a hidden player entity to obtain an attack target. The map and starting
regions are fixed/public, not procedurally secret. Defense targets and resource
choices require its own visibility. There is no scouting memory or sophisticated
recovery after losing all workers. Pacing fields and balance constants live in
`content.rs`; player 1 command admission is enabled only for match scenarios.

A match ends when either owner has no remaining Town Center (unfinished centers
also count). Losing only military is not defeat. Simultaneous center loss is
player defeat. Terminal state freezes ticks, AI and combat. Menu/restart replaces
the whole world, pending commands, IDs, fog, AI state and effects. No advanced
technology or unit-type immunity can prevent militia from destroying a center.

## Native interface and timing

P/G starts or restarts Persia/Greece. Esc cancels placement, returns to the menu,
then exits from the menu. Existing worker building controls remain H/S/U,
X cancellation, R rally; right-click issues contextual movement/gather/attack.
Select a Town Center and press I/B to train infantry/archers. A arms attack-move
for the next right-click; O stops, D holds. Drag selects owned units; Shift adds.
Enemy colors, HP bars, selected HP/training details, destination markers, hit
flashes, cosmetic arrows and damage labels use the existing geometric renderer.

Space pauses/resumes at 1x; 1/2/3 select 0.5x/1x/2x. `timing::Clock` accumulates
integer half-millisecond time debt and executes at most five fixed steps per
update. Excess debt is retained, not silently discarded. Pause advances no ticks
and collects no wall-time debt; outstanding debt is preserved. Gameplay commands
queued during pause execute in (tick, player, sequence) order at the first resumed
boundary. Pan, selection, menus and volume controls remain responsive while
paused. Sustained overload can make simulation lag real time; it cannot produce
an unbounded catch-up loop or change movement/damage formulas. Existing shared
FixedStep/Echoes presentation behavior was left unchanged.

## Audio integration

Inspected Echoes' `soundtrack.rs`, audiod's actual `AudioClient`, stream status,
PCM submission and timeout behavior. Empires uses those same 48 kHz stereo s16le
PCM APIs from one native producer thread. A bounded atomic cue bitmask feeds an
eight-voice application mixer, with duplicate throttling. Native IPC never runs
inside a simulation step or a GUI update. The worker sleeps through the existing
libc monotonic nanosleep/futex path between polls, rather than busy-yielding. Missing/unsupported output is retried
with bounded delays and cues are dropped gracefully.

Original integer procedural tones cover selection, movement, construction start
and completion, training completion, invalid orders, melee, arrows, death,
victory and defeat. No third-party audio assets or dependencies were introduced.
Master/effects defaults are 70/75, with quieter battle tones. M toggles mute,
-/+ changes master, [/] effects, ,/. music, in steps of ten. Music volume is
reserved: **no background music is supplied**. The settings overlay labels it
accordingly. There is no active music stream to release.

Pause silences future simulation events; UI cues still play. Speed changes event
cadence, not PCM pitch/duration. Muting clears voices and stops this stream when
safe; restart/menu clears voices without creating another worker. Exit requests
shutdown, waits for completion and native kernel thread quiescence (syscall 156,
the same query used by libc's pthread bridge), then unmaps stack and TLS. A stalled
worker is never freed underneath execution: if shutdown exceeds two seconds its
mappings remain until process exit. This exceptional timeout path has not been
fault-injected.

**Existing audio.v1 limitation:** there is one global application queue with an
exclusive badge lease, not per-client mixing. Inspection of `submit_pcm` showed
that the lease survives an empty queue until STOP. Native testing also exposed
idle starvation counters from retaining that lease after a short cue. The final
adapter releases the lease when its queued and in-flight PCM tail drains, so an
idle open window does not hold the application queue indefinitely. Empires drops
cues when another app is busy and follows Echoes' submitted-frame check before
STOP. STOP remains global (including system sounds); the adapter does not claim
cross-application mixing or change Echoes' audio. Concurrent application handoff
still has protocol limits. Initial reverse-handoff testing failed because
Echoes was killed with code 143 before its soundtrack Drop; its close-grace
repair is recorded separately from Empires changes. Exceptional service-failure
recovery remains a separate validation gap.

## Validation

Generated evidence lives under `target/empires-m2/` and is not committed. Host
coverage exercises actual worlds, commands, paths, combat and economy, rather
than mock gameplay. `final-empires.log` contains the complete current results,
replay checksum, workload timings and isolated allocator test. Baseline and final
Silicon Echoes logs are separate. Native fixture hashes identify exactly which
ELF/ISO produced each run; preliminary and final fixtures are separate.

| Check | Result | Evidence |
| --- | --- | --- |
| Baseline host libraries | PASS: Empires 16, foundation 3, Echoes 57 | baseline-host.log |
| Baseline native releases | PASS: both games | baseline-native.log |
| Final Empires rules | PASS: 52 | final-empires.log |
| Isolated heap recovery | PASS: 128 alternating settlement/match lifetimes including replay | final-empires.log |
| Final shared foundation / Echoes | PASS: 3 / 57 | final-regression.log |
| Both games native release | PASS | final-native.log |
| Final audio adapter release | PASS | final-audio-native.log |
| Ordinary production Phase 3.0 boot | PASS before final native audio/close-grace adjustments | production-boot.log, production-boot.serial.log |
| Final ordinary init/kernel/ISO compilation | PASS after those adjustments; exact game bytes match final native fixture | final-production-restore.log, final-image-audit.json |
| Exact init source restoration | PASS, SHA verified | production-verification.json, fixture manifests |

Combat coverage includes damage/range/cooldown, lethal simultaneous volleys,
stale/hidden/dead targets, cleanup, normalized mixed-owner groups, five melee
attackers approaching one building, hold and order override. AI coverage includes
exact wood conservation (balance + cargo + buildings + units + reserved training),
production/capacity, peaceful period/preparation/defense, hidden-player changes
not altering its decisions, and complete default matches. Fog covers radius,
exploration persistence, per-owner visibility and hidden targeting rejection.
Lifecycle covers center-loss rules, terminal freeze, reset and heap recovery.
Audio covers actual sound events, deterministic original PCM, mute, zero volume,
voice limits and throttling. Scheduling covers all three speeds, pause, command
retention, replay under differing presentation schedules and draining retained
catch-up debt. Original 16 regression tests remain passing; extra tests cover
crossing traffic, depleted resources, worker rally and stable attack-move arrival.

The economic player scenario wins through submitted build/gather/train/attack-move
commands against the **default** AI at tick **6596**, checksum
**84f42295ee24cb8c**, 112 logged player/AI commands. Reloading the command stream
reproduces its checksum. An unattended default match reaches defeat. A separate
4100-tick match replay checks exact world equality including AI state; differing
10/100 ms presentation schedules reach identical checksums. No PRNG or hidden
wall-clock input is used by the fixed map/opponent.

Paired host combat workloads (two worlds, each 400 ticks) measured 12,685 / 47,020 /
85,468 / 148,757 microseconds for initial populations 8 / 32 / 64 / 128 in one run.
Casualties reduce population during these workloads; these are not constant-load
FPS benchmarks. Each pair ends with equal checksums and bounded effects/events.

Native observations and remaining gaps follow below.

## Native execution evidence

All native inputs were ordinary QMP keys/buttons/motion through the existing
UI. No world-state injection or privileged gameplay spawning was used. The
fixture instruments **init launch only**, restores its exact source bytes, and
builds a separate ISO. Ordinary production init/kernel/ISO were rebuilt after
fixture creation. `final-image-audit.json` checks the extracted fixture kernel
against its manifest, verifies exact embedded bytes of both final game ELFs in
both fixture and production kernels, and confirms normal init is embedded only
in the production kernel.

Evidence is deliberately separated by executable revision:

- `final-fixture/`: complete interactive economy/training/combat/defeat/restart
  run, before the native audio idle-lease repair. Game SHA starts `97a3d10a`.
- `audio-fixture/`: final Empires audio adapter (`865f2385`), selection/mute
  captures and forward music handoff. Its **failed reverse handoff** is preserved;
  Echoes exited with SIGTERM/code 143 and orphaned the service lease.
- `handoff-fixture/`: final Empires **and repaired Echoes**. This is the final
  native binary pair, with successful bidirectional handoff and group controls.
- `missing-audio/`: the same final handoff ISO without an HDA device. It tests
  actual backend absence, not a mock or generated-event substitute.

| Native action | Result | Raw evidence |
| --- | --- | --- |
| Three-worker selection | PASS | final-fixture serial selection `[2,3,4]` |
| Queue house while paused | PASS | submit tick 16, wood remains 240 in `07-paused-queued-house.png` |
| Resume at 2x / charge once | PASS | wood 240→200 at tick 17; Double indicator |
| Cooperative completion | PASS | completion event tick 58 |
| Train infantry | PASS | submit tick 456, wood 200→165, trained tick 535 |
| Train archer | PASS | submit tick 2571, wood 165→120, trained tick 2670 |
| Native first-match combat | PASS | actual melee/arrow/death events, then defeat tick 5777 |
| Final executable combat and defeat | PASS | handoff-fixture melee frame `07-final-combat.png`, defeat tick 4329 and `08-final-defeat.png`; post-match Greece restart `09-final-restart.png` |
| Defeat overlay | PASS | final-fixture `08-defeat.png` |
| Restart as Greece | PASS | `09-restarted-greece.png`, blue units, initial 240 wood and 4/5 population |
| Ordinary desktop launch | PASS | source=shell launch, new PID 54, final-fixture `12-ordinary-launch.png`; final binary also launches as PID 56 in handoff-fixture `14-final-ordinary-launch.png` |
| Independent process relaunch | PASS | PID 54 cleanly exits; new source=shell PID 56 launch; `15-independent-relaunch.png` |
| Final-ELF military group | PASS | handoff-fixture selection `[2,3,4,5,12]`; AttackMove at tick 1758, workers excluded by authoritative eligibility |
| Stop and hold override | PASS | handoff-fixture commands and `05-group-hold.png` |
| Actual output and mute | PASS | final adapter captured selection peak 954 / 8,812 nonzero samples; muted capture peak 0 / zero nonzero samples |
| Empires → Echoes audio | PASS | final pair music peak 9579; 262,205 of 266,240 samples nonzero while Empires stays open/paused |
| Echoes → Empires audio | PASS | Echoes logs audio shutdown complete and exits code 0; next Empires selection peak 954 / 8,812 nonzero samples |
| Missing audio hardware | PASS | audiod explicitly reports no output device; game advances to tick 116, two scenes and twelve surface lifetimes complete |
| Native heap recovery | PASS | 64 simulation/replay lifetimes, 51,200 ticks, exact live allocation/byte recovery; twelve window lifetimes recover exactly after warm-up |

The first failed ordinary relaunch attempt appended to the retained search query
(`EmpiresEmpires`); after clearing the text, the real second launch succeeded.
An early attack-move click in final-fixture landed outside the game window and
was not accepted. Neither attempt is counted as successful gameplay. The final
handoff-fixture group command was accepted inside the viewport and is recorded
in its actual command trace.

WAV capture uses QEMU's HDA output backend. The adapter submits 48 kHz stereo;
QEMU's WAV backend resamples to 44.1 kHz stereo s16le. Captures are proof of
nonzero native output through the actual service/device path, not a claim that
physical speakers were listened to. Small isolated PCM/WAV segments and JSON
byte offsets are preserved alongside complete WAVs. Final combat sound activity
is recorded in `handoff-fixture/audio-activity-after-handoff.json`; the final
battle's PCM is preserved as `combat-audio-tail.wav`. This segment crops the last
contiguous sound activity, rather than the later silent menu period, without
altering captured samples. UI cue audio during pause
was actually captured. System sounds were idle during isolated cue comparisons.

The final pair's native handoff test also exercises Echoes' ordinary launcher,
window, music and graceful exit. The earlier native Echoes run entered its real
bedroom scene (`audio-fixture/05-echoes-play.png`). All 57 library tests passed
again after its close-grace repair (`echoes-cleanup-regression.log`). No story or
synthesis logic was changed.

Native HDA starvation diagnostics were observed during the earlier busy-yield /
retained-lease run, and while Echoes played under VM load. Releasing the idle
lease removed continuing idle starvation reporting; the final selection test
still logged a single tail underrun. Captured music is nonzero, but this is not
proof of glitch-free audio under all loads.

## Performance and limits

The native simulation/replay stress high-water heap was **160,736 bytes**, with
zero live allocations and requested bytes before/after the isolated suite. The
historical M1 documentation reports 116,640 bytes; its original raw log is not
present here, so that older figure is a documented comparison, not a repeated
baseline measurement. Surface lifetimes also returned to zero requested bytes;
the final audio adapter's twelve-window suite took 5,120 native ms in one run
and 2,160 ms in the final handoff fixture.
This measures heap recovery, not total OS memory or frame pacing.

The native process reserves the established 8 MiB heap. A 960×680 double pixel
buffer uses about 4.98 MiB of separate SHM, and the audio worker uses a 2 MiB stack
plus TLS, explicitly unmapped after kernel-confirmed thread exit. Final Empires
ELF is 733,816 bytes. GUI FPS, input latency, total compositor/kernel growth and
large native armies have not been quantified. Host workloads reach 128 initial
units; production caps units plus reservations at 256, buildings at 128, command
logs at 4096 and replay recovery at 72,000 ticks (one simulation hour).

Known limits and unexecuted validation:

- M1.1's entire original matrix, including all contributor-overflow/navigation
  cases and both-civilization native construction variants, remains incomplete.
  Normal desktop launch/relaunch is now observed; historical M1.1 status must
  not be silently rewritten as PASS.
- The full native winning UI scenario and a native explicit right-click attack
  target have not been separately captured. Victory, hidden-target rejection,
  target death, group pursuit and replay are covered by actual-world host tests.
- 0.5x is host-tested; native evidence covers pause and 2x. Missing output hardware
  is tested; an absent audiod service and a deliberately hung audio worker are
  not fault-injected.
- No soundtrack is supplied; music volume is reserved. Application queues are
  exclusive, so simultaneous music from another app can cause cues to be dropped.
  This avoids stealing its stream, but is not cross-application mixing.
- Fog remembers exploration, not resource snapshots or enemy-building history.
  Terrain/resource clearing uses current map terrain even in dim explored areas.
- There is no stalemate/draw detector, AI worker replacement strategy, barracks,
  farming, projectile travel damage, or advanced congestion/formation solver.
  Defeated entities never refund investments. Economic exhaustion can require a
  restart; menu/restart always remains available.
- No networking, technology progression, siege/naval units or sprite overhaul
  was added. Large opposing streams through narrow passages need broader stress
  and navigation validation before increasing army limits.

## Reproduction and next milestone

```sh
cargo test -p sunlight-empires --target x86_64-unknown-linux-gnu -- --nocapture
cargo test -p sunlight-game -p sunlight-silicon-echoes --lib --target x86_64-unknown-linux-gnu
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' cargo build -p sunlight-empires -p sunlight-silicon-echoes --features sunlight-empires/native --release
SUNLIGHT_TEST_SERIAL_LOG=target/empires-m2/production-boot.serial.log ./tools/test.sh phase3.0
python3 tools/empires_m1_fixture.py --output target/empires-m2/reproduction
```

Boot that separate fixture ISO with two CPUs, 2 GiB RAM, virtio-rng, USB mouse,
QMP, `-audiodev wav,id=snd0,path=...` and `-device intel-hda -device
hda-micro,audiodev=snd0`; log in to the development desktop before the 60-second
fixture spawn. Use `EMPIRES_EVIDENCE_DIR=... python3 tools/empires_m1_qmp.py
SOCKET ...` to preserve real input actions/screenshots. Omitting the HDA options
reproduces unavailable output. Restore normal sunlight-init release, kernel and
ISO afterward as described by the fixture tool. Exact execution helper scripts
used during this run are preserved under `target/empires-m2/validation-scripts/`.

Recommended M3: complete native winning/replay and remaining M1.1 matrix coverage,
measure native frame/audio pacing with larger armies, harden audio lease recovery
on abnormal client death, and improve economy/navigation recovery before adding
more unit types. Keep networking as a later milestone.


## Final assessment

**PARTIAL** against the entire requested validation checklist. Functional combat,
a constrained deterministic opponent, fog, terminal match rules, native output,
restart, group controls and ordinary desktop launch are implemented and observed.
The final binary pair is verified in native QEMU, including combat/defeat,
post-match restart, sound output, bidirectional Echoes handoff, missing-device
operation and exact heap recovery. Host victory and deterministic replay pass.
The remaining gaps above concern exhaustive native winning/targeting scenarios,
M1.1 matrix closure, exceptional service failures and quantified performance.
They are not promoted to PASS from screenshots or generated cue events.
