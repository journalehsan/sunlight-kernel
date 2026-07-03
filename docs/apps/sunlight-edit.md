# Sunlight Edit

**Status:** MVP native graphical text editor (`sunlight-edit` / `sunlight-text`).

## Overview

`sunlight-edit` is the first practical desktop text-editing application for
SunlightOS. It opens a single document in a monospace editor area, supports basic
keyboard editing, and reads/writes UTF-8 files through the existing VFS APIs.

## Launch

```text
sunlight-edit
sunlight-edit /root/roadmap.md
```

The app is registered in the Vortex Shell Start Menu as **Text Editor** and
tracks launch state like other desktop apps.

## Current scope

- Single-document buffer (`Vec<String>` lines, UTF-8 safe cursor)
- Toolbar: Save (active), Redo (disabled placeholder)
- Line numbers, caret, vertical scroll, status bar (line/column, counts, dirty)
- Save to the opened path, or `/root/untitled.txt` for untitled buffers
- Ctrl+S and toolbar Save

## Not in this milestone

Syntax highlighting, tabs, search/replace, selection, plugins, and a save-as
dialog. Persian and other non-ASCII text is preserved in files; the current
MiniType fonts render ASCII glyphs in the editor view.

## Tests

```sh
cargo test --package sunlight-edit
```

Unit tests cover buffer insert/delete, line joins/splits, and UTF-8 round-trips.
