# Sunlight UI text-editing integration audit

## Baseline inventory

The investigation for this patch found these text-related Sunlight UI types:

- `TextInput`: bounded, single-line text entry. Before this patch it had a
  caret and basic insertion/deletion/navigation, but no selection, clipboard,
  context menu, shortcut handling, or cursor request.
- `TextView`: read-only multiline display with vertical scrolling. It remains
  intentionally non-interactive; static labels and ordinary text views are not
  made selectable implicitly.
- `DocumentCanvas` plus `TextEditState`: retained document rendering and text
  hit-testing for structured applications such as `sunlight-writer`. It can
  render a caret and selection metadata, but the application owns document
  mutations and event policy.
- `BoundedSearchField`: a search-palette-specific bounded editing model. It is
  coupled to the palette's immediate-mode layout and is not a general form
  input.
- `Label`: static display text and deliberately not selectable.

Outside the toolkit, the audit also found `sunlight-api-lab/src/text_area.rs`,
the Vortex start-menu `SearchField`, terminal/footer input models, login input,
and `sunlight-writer`'s structured document mutation code. These are not all
semantically interchangeable with a normal text field.

## Entry, selection, and cursor behavior

- Text entry existed in `TextInput`, `BoundedSearchField`, the API Lab text
  area, the Vortex start-menu field, and application-specific editor models.
- Before this patch, general-purpose read-only selection did not exist in
  `TextView`. `DocumentCanvas` exposed selectable/editable metadata, while
  `sunlight-edit` implemented its own range selection.
- Widgets request cursors through `set_client_cursor(CursorShape)`. The app
  event loop applies the latest request with `SgpMsg::SET_CURSOR`, and the
  display server maps `CursorShape::Text` (wire discriminant 9) to
  `assets/cursors/text.tga` with its configured hotspot.
- Cursor changes were application-managed. `sunlight-writer` explicitly maps
  document text hit targets to `CursorShape::Text`; the old `TextInput`,
  `TextView`, and `sunlight-edit` surface did not request it automatically.

## `sunlight-edit` baseline

`sunlight-edit` stored UTF-8 text as `Vec<String>` lines in `TextBuffer`, with a
character-column caret. The buffer implemented UTF-8-safe insertion, newline
splitting/joining, forward/backward deletion, line/document/word movement,
range extraction/deletion/replacement, word and line ranges, select-all, and
find-all.

The application separately owned selection anchor/drag/click state, vertical
scroll position, caret blinking, hit-testing, selection painting, local
shortcuts, and a right-click menu. Copy/cut/paste encoded and decoded the
clipboard daemon's shared-memory wire format directly. `Ctrl+A/C/X/V` were
implemented; `Ctrl+Z/Y` were recognized but intentionally did nothing. There
was no undo or redo history to preserve.

The following parts were application-specific and remain in `sunlight-edit`:

- file open/save/save-as and temporary backing files;
- dirty-document prompts and dialogs;
- find/replace state and match highlighting;
- toolbar, hamburger application menu, header, and status messages;
- document title/path and persistence policy.

## Classification and patch boundary

### Reusable widget behavior

- UTF-8 text buffer and caret/range primitives;
- single-line and multiline caret, selection, drag, and navigation behavior;
- local editing shortcuts while a widget is focused;
- capability/state-driven text context menus and shortcut labels;
- text clipboard get/set operations;
- editable and read-only-selectable multiline modes;
- automatic I-beam requests over interactive text.

These now live in `sunlight-ui` as `TextBuffer`, `TextInput`, `TextEditor`,
`TextEditorState`, `TextContextMenu`, and the text-only `clipboard` API.
`TextEditor::selectable` is the explicit read-only selectable category; normal
`TextView` and `Label` instances remain static.

### Duplicated behavior removed in this patch

- `sunlight-edit`'s private clipboard client and wire parser;
- its private multiline selection/caret renderer and pointer hit controller;
- its private right-click editing menu;
- its copy/cut/paste/select-all dispatch path;
- its private copy of the multiline `TextBuffer` implementation (the old
  module is now a compatibility re-export).

### Duplicated behavior not widened into this patch

- API Lab's fixed-capacity request-body area and the Vortex search fields use
  specialized storage/lifecycle contracts. Converting them requires an API
  migration rather than copying their behavior into the shared editor.
- `sunlight-writer` edits structured `DocumentCanvas` content by byte offsets
  and layout objects. Its document command model should adopt the shared text
  commands in a dedicated writer integration, not be forced into the plain
  line-oriented buffer.
- File Manager context menus and clipboard operations include file-list
  payloads and therefore are not text-widget behavior.
- Clipboard producers such as Emoji Picker and Control Panel still need their
  explicit copy actions; only editing widgets use the shared automatic path.

### Missing shared behavior filled here

- selection and UTF-8-safe replacement in `TextInput`;
- standard state-sensitive editing menus with visible `Ctrl+A/C/V/X` labels;
- clipboard-backed local widget shortcuts;
- reusable editable/read-only multiline text surfaces;
- text cursor requests, including arbitration that keeps a specific widget
  cursor from being overwritten by a fallback pointer request in one event.

### Architectural follow-up outside this patch

- global/configurable shortcut registries and desktop shortcut routing;
- terminal/TTY shortcut policy and terminal `Ctrl+C` semantics;
- undo/redo history (neither the prior editor nor shared widgets had one);
- rich text, syntax highlighting, and a new document model;
- optional migration of specialized palette, API Lab, and structured writer
  editors after their storage and command contracts are defined.
