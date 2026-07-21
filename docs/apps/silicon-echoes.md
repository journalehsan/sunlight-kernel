# Silicon Echoes: 1993

`silicon-echoes` is a native SunlightOS philosophical 2D narrative game. It is
embedded as `/bin/silicon-echoes`.

## Scope

- Title screen, new game, continue, four bedroom hotspots, and a complete
  authored first chapter after the original bedroom sequence.
- Chapter one moves through the hallway, kitchen, landing, stairwell, street,
  diner, phone, repair shop, transit stop, archive, revelation, and turning
  point. Each exposed route reaches implemented content.
- Story state keeps stable scene and actor IDs, visit counts, choices, facts,
  observations, beliefs, memories, relationships, flags, bounded delayed
  consequences, tendencies, and a deterministic game seed separate from UI.
- The rule-based `ScriptedDirector` validates structured actions and targets
  before applying any transition. It is the narrow boundary for a future
  Director implementation; it does not generate prose.
- Choices affect trust, knowledge, beliefs, and delayed events without a
  win/lose or moral score. The chapter resolves multiple delayed consequences,
  includes intentional convergences, and includes an archive closure caused by
  another character's independent decision.
- Uses only Obsidian (`#0A0A0C`), Bone (`#EDE6D8`), and Sunlight (`#FF9800`),
  including alpha/intensity variants.
- Saves a versioned, validated record through `sunlight-kv`. Version 2 retains
  the full world state and deterministically migrates the prior version-1
  bedroom saves. Invalid or unsupported records are rejected before replacing
  in-memory game state.
- Uses `sunlight-libc`'s `global-alloc` plus `dynamic-heap-8m`; the story
  naturally uses `Box`, `Vec`, `String`, `format!`, and ordered maps.

## Native Graphics Integration

The game follows the established `sunlight-ui::Window` lifecycle used by
Calculator and Light Lens:

- `Window` creates an SGP display-service window backed by a shared-memory,
  double-buffered ARGB framebuffer.
- `Canvas` supplies clipped rectangle, border, alpha compositing, rounded
  rectangle, line, and TGA/image primitives; the game uses the existing
  primitives only.
- `sun-font` supplies antialiased embedded MiniType text.
- `Window::run` owns mouse, keyboard, focus, cursor, commit, and surface
  cleanup. Current client presentation is full-frame `COMMIT_FRAME`; no
  client damage API or resize callback is exposed, so no parallel protocol was
  introduced.
- Ambient CRT variation uses the non-cryptographic random service only;
  narrative state is deterministic.

No graphics-engine extension was required for this slice.

## Validation

```sh
cargo test -p sunlight-silicon-echoes --lib --target x86_64-unknown-linux-gnu
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static -C no-redzone" \
  cargo build -p sunlight-silicon-echoes --release
```

Within SunlightOS:

```sh
/bin/silicon-echoes
/bin/silicon-echoes --stress
/bin/silicon-echoes --display-stress
```

`--stress` validates the graph, traverses the authored chapter, exercises
deterministic save/load and repeated allocation/drop churn beyond the game heap
cumulatively, and checks allocator recovery. `--display-stress` repeats native
window create, redraw, commit, and close lifecycles.

## Deferred

- Later chapters, map/log/status UI, inventory, audio, and additional scenes.
- General animation, particle, physics, scene-editor, 3D, and shader systems.
- Client-side partial-damage or resize protocol work, which belongs in the
  graphics/display stack rather than this game.
