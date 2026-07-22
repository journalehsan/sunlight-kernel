# Silicon Echoes: 1993

`silicon-echoes` is a native SunlightOS philosophical 2D narrative game. It is
embedded as `/bin/silicon-echoes`.

## Scope

- Title screen, new game, continue, four bedroom hotspots, a complete authored
  first chapter, and a direct Chapter Two continuation from its completion
  screen.
- Chapter one moves through the hallway, kitchen, landing, stairwell, street,
  diner, phone, repair shop, transit stop, archive, revelation, and turning
  point. Chapter Two investigates `REVISION 7 / SUNSET LOT 17 / 2013` through
  city records, the river route, the unfinished lot, archive annex, revision
  chamber, and an ambiguous 2013 response. Each exposed route reaches
  implemented content.
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
- Chapter Two adds Echo Overlay at designated annex scenes. It compares a
  physical 1993 layer with a Revision 2013 layer using active-layer objects,
  interactions, hitboxes, and restrained orange outlines rather than free time
  travel.
- Saves a versioned, validated record through `sunlight-kv`. Version 4 persists
  chapter progression, Echo Overlay state, actor knowledge, and Chapter Two
  consequences; it deterministically migrates version-1 through version-3
  saves. Invalid or unsupported records are rejected before replacing in-memory
  game state.
- Uses `sunlight-libc`'s `global-alloc` plus `dynamic-heap-8m`; the story
  naturally uses `Box`, `Vec`, `String`, `format!`, and ordered maps.
- Narrative scenes share an explicit presentation lifecycle: entrance, Unicode
  scalar-safe typewriter reveal, post-reveal pause, player choice, and a
  single transition. The default Normal rhythm is 420 ms entrance, 50 ms
  ordinary text, 150 ms clause (comma/semicolon/colon), 320 ms sentence,
  420 ms paragraph, and 520 ms before choices. Bounded profiles also include
  Slow, Fast, and Instant; Instant is used by deterministic tests. Space or
  Enter during reveal completes prose only and never activates a choice in the
  same input.
- Choices show `[A]` through `[Z]` in their visible order. Arrow keys,
  left/right, Tab/Shift+Tab, Enter, and Space support focus-first play; Space
  or Enter while prose is revealing only completes the reveal. Bedroom
  hotspots and Echo Overlay are also keyboard reachable. Shortcut input is
  debounced across scene and focus changes, while mouse activation uses the
  same StoryAction boundary.

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
- The completed narrative layout is prepared once per scene/width. Rendering
  draws a UTF-8-safe prefix of that layout, so line breaks do not reflow while
  prose appears and no growing text buffer is allocated per frame.

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

`--stress` validates the graph, traverses the authored story, exercises
deterministic save/load and repeated allocation/drop churn beyond the game heap
cumulatively, and checks allocator recovery. `--display-stress` repeats native
window create, redraw, commit, and close lifecycles.

## Deferred

- Later chapters, map/log/status UI, inventory, audio, and additional scenes.
- General animation, particle, physics, scene-editor, 3D, and shader systems.
- Client-side partial-damage or resize protocol work, which belongs in the
  graphics/display stack rather than this game.
