# Sunlight Writer Phase 1 architecture audit

## Shared canvas and current users

The framebuffer drawing primitive is `sunlight_ui::paint::Canvas`.  The shared
document widget is `sunlight-ui/widgets/document_canvas.rs` and is re-exported
from `sunlight-ui/widgets.rs`.  It has two deliberately separate producer
paths:

- immediate `DocumentCanvasItem` slices, currently used by Sunlight Writer;
- retained `DocumentScene` render objects, currently used by Rappid Rabbit.

Writer builds its chrome with the immediate-mode `Column`, `LayoutBox`,
`PremiumHeader`, `RibbonBar`, and `StatusBar` primitives.  It converts its
application-owned blocks into `DocumentCanvasItem`s and supplies transient
`TextEditState` metadata.  Before Phase 1, only one fixed-coordinate item was
mutable and the application rebuilt a complete `String` for each edit.

Rappid Rabbit creates a read-only Browser-presentation `DocumentCanvas`,
supplies a retained `DocumentScene`, and owns `render_scroll`.  Its assumptions
are that browser presentation uses the complete viewport, scene coordinates
remain document-local, scene hit testing accounts for pixel scroll, and no
editing state is required.  Those APIs and defaults must remain compatible.

## Text, positions, input, and measurement

Writer's text was stored in `WriterApp::edit_buffer`.  Its caret and optional
selection anchor were UTF-8 byte offsets in `TextEditState`; vertical movement
also retained a preferred pixel X.  Byte offsets are appropriate logical
document positions provided every operation preserves `str::is_char_boundary`.

Keyboard and pointer input arrive through `sunlight_ui::Event` in the `App`
update method.  Decoded text is delivered as `Event::Key`; raw navigation keys
and modifiers as `Event::KeyPress`; pointer press/release/move/click and wheel
events have window-local coordinates.  Resize arrives through
`WindowEvent::Resized` and invalidates the responsive root layout.

Document text uses the optional `VecText` face for `measure_w`, `draw`, and
`line_height`, falling back to the built-in `Canvas` metrics.  The existing
`layout_text_lines` produces document byte ranges and pixel Y offsets.  It
already distinguished hard newlines from soft wrapping, but the old Writer
integration had no full-document selection, scrolling, drag selection,
clipboard commands, page navigation, or cached editor layout.  Immediate item
scrolling was also absent; only retained browser scenes used `scroll_y`.

## Existing services and reusable toolkit facilities

`sunlight-ui/clipboard.rs` already wraps the `clipd` system service with safe
plain-text `get_text`/`set_text_from` operations.  No private Writer clipboard
is needed.  `sunlight-ui/scroll.rs` contains reusable scroll policy/state and
scrollbar painting, while `DocumentCanvas` already uses pixel scroll for
retained scenes.  Writer needs pixel scrolling over wrapped visual lines, so it
can use the same document-coordinate convention without changing Rabbit.

The responsive `Column`, `LayoutBox`, `Sizing`, and `LayoutInvalidation`
contract already computes Writer's fill-sized workspace.  The editor should
derive its wrap width and viewport height from `DocumentCanvas::content_rect`
on every geometry change rather than add fixed window dimensions.

There is no existing undo/redo history.  Shared `TextEditor` and the previous
Writer recognize editing commands but intentionally expose no undo stack.

## Phase 1 boundary

Generic toolkit responsibilities are the plain UTF-8 document string,
byte-boundary invariants, caret/anchor state, editing primitives, visual-line
cache, wrap-aware navigation and hit testing, pixel scrolling, and selection /
caret painting.  These are added as an optional editor path; existing immediate
items and retained scenes remain valid and read-only rendering is unchanged.

Writer remains responsible for focus and shortcut policy, clipboard service
calls, shell/status UI, and future document semantics.  File paths, titles,
toolbar state, rich spans, Markdown/RTF, and persistence do not enter the
generic canvas.  Undo/redo remains deferred; the shared mutation methods form
a single boundary that a future history layer can wrap.
