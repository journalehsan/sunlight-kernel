# Sunlight Empires M1

## Reconnaissance (before implementation)

The real existing game is `sunlight-silicon-echoes` (workspace member), with
`src/main.rs::_start` producing `silicon-echoes`. Its no_std library contains
host-testable narrative rules; the binary uses alloc, sunlight-libc (8 MiB
heap), sunlight-ipc, sunlight-ui/app, sun-font, sunlight-audio and audiod.
`scenery.rs` draws procedural Canvas geometry. No shared game engine exists.

`sunlight-ui/app.rs` provides App::update/view, bounded event polling,
monotonic idle ticks, native SGP Window creation, shared double buffers,
COMMIT_FRAME, focus/resize events and close/Drop cleanup. `event.rs` exposes
button down/up/motion, raw PS/2 keys with modifiers and wheel input. Canvas
clips drawing to its surface; sun-font embeds MiniType fonts. The compositor
is `services/sunlight-display`; games must use Window, not a second protocol.
Silicon Echoes throttles presentation using monotonic_millis, keeps keyboard
state and caches scene text layout. Game rules do not require that clock.
Assets are compiled geometry, fonts and procedural PCM, not a loader engine.

`src/soundtrack.rs` owns a background PCM worker and optional AudioClient.
The audio.v1 application stream is global, with no independent effect mixer.
It explicitly avoids stopping another app's stream. Its worker/Drop protocol
is specific to continuous music; extracting it for short effects is not
necessary for the playable foundation. M1 sound cues remain optional/deferred
rather than issuing blocking IPC on the GUI thread or stealing that stream.

Registration uses VortexShell AppId/AppStateAppId and the start menu array,
RAMFS executable entries, kernel embedded_service_bytes and spawn's executable
lookup. These kernel table additions are packaging only. tools/build.sh,
runs.sh, write.sh and test.sh build embedded userspace ELFs before the kernel.
The established terminal (`sshl`) also recognizes graphical app names.

Existing regression command: `cargo test -p sunlight-silicon-echoes --lib
--target x86_64-unknown-linux-gnu`. Native releases use SERVICE_RUSTFLAGS from
tools/test.sh and services/user-space.ld. The ordinary gate is
`./tools/test.sh phase3.0`, building userspace/kernel/ISO then checking serial
markers in QEMU. Native UI behavior needs separate actual input/screen evidence.

## Design

Reuse Window/App, Canvas, Event, sun-font, Theme, libc allocator and IPC clock
at the adapter boundary without changing Silicon Echoes or shared graphics.
The no_std + alloc simulation library has no platform dependencies. Native
features enable platform dependencies only for the binary. Authoritative ticks
are 50 ms; integer positions use 256 subunits/tile. IDs never recycle. Commands
are ordered by (tick, player, sequence), validated before mutation and retained
for replay. Selection/camera/placement preview remain local. No transport.

Further implementation details, validation evidence and limitations are recorded
below after executing checks.

## Implemented architecture and rules

Following the user's request for a shared engine foundation, `sunlight-game`
provides platform-independent timing and camera helpers. Both games use it:
Silicon Echoes calls `frame_due` for its unchanged 90 ms presentation gate;
Empires uses `FixedStep` and `Camera`. This remains a small foundation on top of
the existing native host, rather than another window/render/input abstraction.
The frame-gate compatibility test covers the old expression exactly.

- `sunlight-empires/src/lib.rs`: authoritative world, integer economy, command
  admission/execution, entity lifetime, construction, movement and collection.
- `content.rs`: versioned Persia/Greece identities, colors, shared unit speeds
  and building costs/durations. Identity grants no technology or stat advantage.
- `path.rs`: deterministic four-neighbor uniform-cost BFS, at most 4096 expanded
  cells, retained parent/queue scratch, fixed neighbor tie order. Routes allocate
  on orders/retries, not each movement tick. Formation destinations are distinct
  nearby free tiles, with a radius-five limit. Dynamic blockers cause bounded
  periodic replanning. This is deliberately a small-group collision system.
- `replay.rs`: explicit little-endian canonical state bytes and FNV-1a 64-bit
  checksum; versioned bounded replay save/load (including future orders).
  Save recovery replays from the fixed initial scenario, not arbitrary externally
  mutated map data. Up to 4096 commands and 72,000 recovery ticks; no disk save UI.
- `input.rs`: local selection, drag/Shift selection, screen/world conversion,
  placement/rally mode and conversion into commands. No SelectUnits world order.
- `render.rs`: culled procedural terrain/actors on Canvas, stable vector/ID
  order, selection/destination/rally markers, progress bars, stack text buffers,
  responsive panel widths and the existing dark Theme/MiniType fonts.
- `main.rs`: native App/Window adapter, paced 20 Hz ticks independent of input
  frequency, bounded catch-up after GUI stalls, dirty redraw, focus/key cleanup,
  diagnostics and explicit Drop before ProcessExit. No kernel game logic.
- `audio.rs`: optional cue boundary, silent in M1; no worker, handles or external
  audio dependency. Silicon Echoes' global-stream limitations are not bypassed.

World positions are tile centers at 256 subunits/tile. Workers move 32 subunits
per tick; Guards move 48. A replacement order finishes the reserved current
segment before following its new route. No snapping or diagonal shortcuts.
Units reserve current/next cells; buildings reserve their single tile immediately.
All world iteration is ordered. No simulation randomness is needed.

An Envelope carries tick, PlayerId and sequence. M1 accepts local player zero;
other player IDs are reserved. Commands are ordered by (tick, player, sequence),
unit IDs are sorted/deduplicated, duplicate player sequences and past orders are
rejected. Execution validates world conditions at the intended tick. The command
log is canonical and replayable. Renderer, mouse, window and wall clock never
participate in rule decisions. No networking transport or multiplayer claim.

Starting resources: 200 food, 240 wood, 100 stone, 100 gold, zero knowledge.
Three Workers, one Guard and a completed Town Center start on a 64×64 map.
Forests and stone each have 200 units per node. A worker spends 20 adjacent
work ticks collecting up to ten, carries it to a reachable completed Town Center
or Storage (nearest first, stable ID ties), credits it exactly once, then returns.
Cargo survives reassignment. Food/gold/knowledge production is not implemented.
Depleted resource tiles remain impassable stumps/deposits in this first scenario.

House costs 40 wood/60 work ticks; Storage 60/80; University 100/120.
Placement requires free traversable terrain, sufficient funds and a reachable
adjacent cell. The future site is excluded from its builder's route. Charge is
once at acceptance, completion requires adjacent worker ticks, and unfinished
cancellation refunds exactly once, removes occupancy and clears builder tasks.
Only completed stockpiles deliver resources. University has no research or income.
Population shows four units with capacity five, plus five per completed House;
recruitment/cap enforcement for future training is outside M1.

## Native integration and controls

The workspace adds `sunlight-empires` and `sunlight-game`. Build scripts build
`sunlight-empires --features native` with the established service linker flags.
The normal packaging adds only the embedded ELF and executable lookup entries
in the kernel, `/bin` and `/usr/bin` RAMFS wrappers, sun-exec identity/alias,
append-only shell app ID, desktop search and paginated Start Menu entries.
A small original fortress SVG is used for the launcher. Existing IDs are stable.

Intended launches: desktop search `empires`, Start Menu → Games → Sunlight
Empires, or `/bin/sunlight-empires`. Actual launcher validation status is below.

- Left click selects a unit or building; drag selects units; Shift adds units.
- Right click ground moves selected units. Right click forest/stone assigns
  the first selected Worker to gather that node.
- H / House button, S / Storage button, U / University button then left click
  ground places the structure using the first selected Worker. Esc cancels preview.
- Select an unfinished structure, then X / Cancel refunds and removes it.
- Select a completed building, R / Rally then right click sets its rally marker.
  Unit recruitment and rally-following by newly trained units are future work.
- Arrows scroll, wheel zooms 16–40 pixels/tile. Esc exits without a preview.
- P/G start a fresh Persia/Greece settlement, discarding the current offline world.

`--stress` runs native simulation/replay/allocation churn without a window.
`--display-stress` creates/closes twelve real native windows and checks exact
allocator recovery after warm-up. `--native-suite` opens two interactive game
scenes sequentially, then runs those native diagnostics. It is a test-only launch
mode with no synthetic mouse input or injected gameplay commands.

## Validation and evidence

All generated logs, manifests, ISO images and native screenshots are under
`target/empires-m1/` and are not intended for commit. No commit or push was made.

| Check | Status | Evidence |
| --- | --- | --- |
| Empires rules | PASS | 15 unit tests, `host-tests.log` |
| Exact host allocation recovery | PASS | 128 scene/replay lifetimes, one isolated counting-allocator integration test |
| Shared foundation | PASS | Three timing/camera/compatibility tests |
| Silicon Echoes regression | PASS | 57 library tests, `echoes-appstate-tests.log` |
| Shared app registry | PASS | 30 tests in the same log |
| Combined workspace host libraries | PASS | `workspace-check.log` |
| Native Empires/Silicon Echoes releases | PASS | `native-build.log`, initial combined release build |
| Native SVG desktop release | PASS | Existing `tools/build_svg_apps.sh -p sunlight-vortex-shell` |
| Launcher suite | PARTIAL | 65/66 tests pass, `launcher-tests.log`; unchanged sidebar telemetry test fails |
| Ordinary Phase 3.0 QEMU boot | PASS | Initial and final gates, `boot-initial.log`, `boot-final.log`, `boot-final.serial.log` |
| Normal desktop and TTY app resolution | BLOCKED | `launcher-blocked.serial.log`, `07-tty-launch.png` |

The launcher failure was actually observed for both Empires and the existing
Calculator: native desktop search found each entry but sun-exec returned
`app_not_found` before spawn. The authenticated TTY also returned command not
found for `/bin/sunlight-empires`. The ELFs and RAMFS wrappers are built and
present in the source/embedded image. This checkout has substantial pre-existing
VFS stat/inheritance work; this milestone does not modify its implementation or
weaken its policy to make a launch appear successful. This is an unresolved
native integration blocker, despite the ordinary boot gate passing.

The remaining launcher host failure is
`sidebar::tests::telemetry_missing_before_first_sample_is_unavailable`.
`SidebarState::new` already initializes unavailable=true, so the test's first
`observe_telemetry(None)` returns false. The file has no changes in this task.
Empires search and second-page pagination tests pass.

`tools/empires_m1_fixture.py` creates a separate, explicitly labelled diagnostic
image. It temporarily inserts a direct spawn into init at native time >=60 s,
builds that init, then restores the exact source bytes before building the
fixture kernel/ISO. The manifest records the restored init SHA and game/kernel/
ISO hashes. No persistent init changes belong to M1. Normal init/kernel/ISO
artifacts are rebuilt after fixture creation (`production-restore.log`). This
fixture can validate actual game rendering/input/simulation on SunlightOS, but
cannot clear the normal launcher blocker. `tools/empires_m1_qmp.py` reuses the
existing QMP client and sends ordinary keyboard/mouse input only; its pointer
position feedback matches the existing cursor bitmap in real screenshots.

Native gameplay observations and allocator measurements follow below.

The corrected adapter accepts both native `Click` and explicit MouseUp for left
release, and uses text events for printable shortcuts. Native testing also
confirmed that the compositor normally sends SIGTERM immediately when an app's
last window closes. Empires now uses the existing `Window::set_close_grace(6000)`
API to finish Window/App drops and exit itself. No compositor policy is changed.
The earlier gameplay run (`gameplay-verified.serial.log`, `gameplay-manifest.json`)
proved selection, movement, gathering and construction before this shutdown-only
change; the final fixture run is `lifecycle-verified.serial.log` with hashes in
`fixture-manifest.json`.

Native simulation stress: **PASS**. In the final fixture, 64 independent scene
lifetimes with deterministic replay (51,200 total ticks) took 220 native ms on
QEMU TCG, two virtual CPUs. Heap statistics returned exactly from zero live
allocations / zero requested bytes to zero / zero; high-water allocation was
116,640 bytes. This is a simulation/replay throughput observation, not a GUI
frame-rate or input-latency benchmark. The app reserves the established 8 MiB
native heap; framebuffer SHM is separate. The default 960×680 double pixel
buffer requires about 4.98 MiB before compositor metadata. Native fixture RAM
was 2 GiB for the complete OS image, not a game-only memory measurement.

Additional resolver registry checks: **PASS**, nine tests in
`resolver-tests.log`, including canonical IDs and locale alias collision checks.

### Actual final native observations

The final game ELF in the fixture and production output has the same SHA256
(`d8ceb4d854ac6ee8e8d789cc35d8727635975db8907e86c9a4d9e78a26b82d1c`).
`validation-summary.json` records hashes and checks against the raw serial log.
Normal init source restoration was verified against the pre-fixture SHA256.

| Native action | Status | Actually observed |
| --- | --- | --- |
| Open game | PASS (fixture) | Native Window connected, real framebuffer visible |
| Select Worker | PASS | Selection `[2]`, visible outline and Worker panel |
| Move | PASS | Tick 1209 MoveUnits to (12,8), accepted; Worker visible at destination in `40-final-movement.png` |
| Gather wood | PASS | Tick 1678 AssignWorker to forest (14,8); first credited delivery at tick 1773, wood 240→250 |
| Build House | PASS | Tick 2805 build at (11,9); wood 330→290 at tick 2810, charged once |
| Complete House | PASS | Completion log and visible House, population capacity 5→10, `41-final-house.png` |
| Close first scene | PASS | Explicit clean exit, tick 3084, checksum `612da686dbd3cb8f` |
| Reopen scene/window | PASS | Second native scene opens with initial resources; `42-reopened.png` |
| Greece identity | PASS | G creates Greece with food 200/wood 240/stone 100/gold 100/knowledge 0/population 4/5; `43-greece.png` |
| Close second scene | PASS | Explicit clean exit, tick 142 after reset, checksum `07820a37a7edd8cc` |
| Window allocation recovery | PASS | Twelve additional 640×480 native windows, 15,630 ms total, zero live allocations/zero requested bytes after each warm-up lifecycle |
| Process/resource cleanup | PASS | Suite PASS, process exits code 0; kernel reaps 2728 user frames and 13 page tables; 14 total windows attached and 14 detached |
| Independent process relaunch from desktop/TTY | NOT_RUN (BLOCKED) | Resolver failure prevents this; two window/scene lifetimes in one native process must not be described as two ordinary app launches |
| Native audio cues | NOT_RUN | Silent optional boundary; no audio handles allocated |

`44-after-cleanup.png` shows the desktop with no game window remaining.
`qmp-actions.jsonl` records actual input/capture actions. These were real QEMU
keyboard/mouse events and native rendering/simulation, not host renderings or
mocked game commands. Final native gameplay repeated the earlier evidence after
the shutdown grace change. No native stone-gather, cancel, storage-construction,
minimap or independent-process restart is claimed; economy/cancel/storage logic
is covered by host tests (including unfinished stockpile rejection and refund
of a partially constructed structure).

### Reproduction

```sh
cargo test -p sunlight-empires -p sunlight-game --target x86_64-unknown-linux-gnu
cargo test -p sunlight-silicon-echoes -p sunlight-shell-appstate --lib --target x86_64-unknown-linux-gnu
cargo test -p sunlight-libc --lib --target x86_64-unknown-linux-gnu sun_exec::
cargo check --workspace --lib --target x86_64-unknown-linux-gnu
cargo test -p sunlight-vortex-shell --features native-svg --bin sunlight-vortex-shell --target x86_64-unknown-linux-gnu
RUSTFLAGS='-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C target-cpu=x86-64-v2 -C no-redzone' cargo build -p sunlight-empires --features native --release
SUNLIGHT_TEST_SERIAL_LOG=target/empires-m1/boot.serial.log ./tools/test.sh phase3.0
```

For the separate diagnostic fixture, build the game release, run
`python3 tools/empires_m1_fixture.py`, then boot its `native-fixture.iso` using
two CPUs, 2048 MiB, TCG, virtio-rng, qemu-xhci/usb-mouse and QMP. Enter the normal
development desktop before the 60-second direct-spawn checkpoint, perform the
input sequence and close both interactive scenes to reach the stress results.
Use `tools/empires_m1_qmp.py --help` for real input and screenshot capture.
Rebuild normal sunlight-init, kernel and the ordinary ISO after creating the
fixture, as was done here. The fixture is an explicit diagnostic workaround for
the separate resolver blocker, not a normal launch method.

## Status, limitations and M2

Overall M1: **PARTIAL**. The deterministic core, native scene and interactive
movement/gather/construction loop work. Normal launch integration is registered
and built but **BLOCKED** in this checkout's shared resolver/stat path. This
must be resolved before calling the ordinary end-user M1 delivery complete.

The production ELF is approximately 669 KiB including embedded fonts. Drawing
uses culled tiles and a few actors, with no widgets per tile, no external sprite
assets and no heap Strings for frame text. Full-frame commits follow the native
Canvas host; idle worlds do not redraw continuously. GUI frame rate, input
latency and total compositor/kernel memory growth were **NOT_RUN** as quantified
benchmarks. Native heap/scene/window recovery is measured above.

Known limits: one local player and four starting units; single-tile buildings;
simple reservations and bounded small-group destinations can stall in traffic;
no unit training, combat, opponents, win condition, diplomacy or research;
rally markers reserve future training behavior. No disk save UI, transport,
matchmaking or multiplayer validation. Replay recovery is bounded and based on
the fixed scenario. Audio cues and minimap are deferred. Scope-excluded features
were not added. University is an inactive future research placeholder.

Recommended M2: first clear the normal native launch/stat blocker and prove
independent process relaunches; then add deterministic unit production with
population enforcement, improved traffic recovery, replay/checkpoint file UI
and a small offline combat scenario. Keep multiplayer transport for a separate
milestone with lockstep and desynchronization tests. Add sound cues when the
existing audio service can isolate/mix application streams.
