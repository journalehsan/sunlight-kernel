# Silicon Echoes: 1993

`silicon-echoes` is a native SunlightOS philosophical 2D narrative game. It is
embedded as `/bin/silicon-echoes`.

## Scope

- Title screen, new game, continue, four bedroom hotspots, a complete authored
  first chapter, and direct Chapter Two and Chapter Three continuations, each
  reached from the previous chapter's completion screen.
- Chapter one moves through the hallway, kitchen, landing, stairwell, street,
  diner, phone, repair shop, transit stop, archive, revelation, and turning
  point. Chapter Two investigates `REVISION 7 / SUNSET LOT 17 / 2013` through
  city records, the river route, the unfinished lot, archive annex, revision
  chamber, and an ambiguous 2013 response. Each exposed route reaches
  implemented content.
- Chapter Three (`c3-witness`) opens in an unlisted tape room off the signal
  yard, where the station is already recording what Mara does. Three objects
  reveal partial records — a standing ledger, a reel labelled with a year that
  has not happened, and a window that returns the room wrong — and each can be
  examined once, in any order, before the reading desk opens. The chapter then
  runs the ledger dilemma, a transmission in Mara's own handwriting dated 2013,
  and a threshold choice. The ledger and threshold choices are deliberately
  unscored: neither is presented as the correct one, and the consequence is
  deferred past the chapter end.
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
- Saves a versioned, validated record through `sunlight-kv`. Version 5 persists
  chapter progression, Echo Overlay state, actor knowledge, and Chapter Two and
  Chapter Three consequences; it deterministically migrates version-1 through
  version-4 saves. Invalid or unsupported records are rejected before replacing
  in-memory game state. A migrated save is still bounded by the chapters its
  own format knew about, so a version-4 record cannot claim to be in Chapter
  Three, and a loaded chapter must agree with its current node.
- Version 5 stores closed vocabularies (nodes, choices, flags, facts,
  observations, beliefs, actor state, memories, and delayed IDs) as positions in
  ordered tables rather than as key text. Three chapters of authored state no
  longer fit in one `SHM_PAGE` when spelled out; indices keep a completed
  playthrough near 600 bytes against the 4096-byte transport limit. These tables
  are append-only: reordering or renaming an entry would reinterpret existing
  saves.
- Uses `sunlight-libc`'s `global-alloc` plus `dynamic-heap-8m`; the story
  naturally uses `Box`, `Vec`, `String`, `format!`, and ordered maps.
- Each scene names one ambient audio cue through `scene_ambient_cue`, shown in
  the narrative header. Audio playback itself is still deferred; the cue is
  authored text that a future mixer can consume.
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

The illustration layer in `src/scenery.rs` adds a layered amber skyline, lit
windows, animated rain and water reflections, drifting indoor dust, perspective
floors, and restrained edge shading. The bedroom includes a bed, pinned notes,
cast shadows, window light, keyboard, mug, and CRT scanlines/glow. The title uses
a large pixel-cut wordmark over the city. Chapter objects composite translucent
fills so shelf, terminal, and building details retain their intended contrast.
These effects use bounded loops and no per-frame heap allocation; geometry stays
seed-stable while weather follows the existing 90 ms redraw cadence. Effects are
confined to the illustration and preserve narrative layout and interaction bounds.

Chapter Three's tape room is drawn from the same module: a wall of reel
outlines, a lectern ledger whose ruled rows warm as it is read, a reel with a
hand-pressed year label, a window whose reflection holds one chair too many,
and a handwriting slip drawn as cursive strokes rather than legible words, so
the reply stays unverifiable. Its geometry helpers (`tape_room_ledger_rect`,
`tape_room_reel_rect`, `tape_room_window_rect`, `tape_room_desk_rect`,
`ledger_page_rect`, `ledger_cover_rect`, and `handwriting_rect`) are read by
both the illustration and `scene_object_bounds`, so a hotspot cannot drift away
from the silhouette it belongs to.

No graphics-engine extension or external image assets are required.

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

- Chapters beyond the third, map/log/status UI, inventory, audio playback, and
  additional scenes.
- General animation, particle, physics, scene-editor, 3D, and shader systems.
- Client-side partial-damage or resize protocol work, which belongs in the
  graphics/display stack rather than this game.
