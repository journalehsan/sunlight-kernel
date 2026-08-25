# Sunlight Writer

**Status:** Phase 3 document editor with Markdown/TXT persistence.

## Overview

`sunlight-writer` is the premium document application shell for SunlightOS.
Writer owns the rich document model; the shared `DocumentCanvas` and
`DocumentEditor` widgets provide layout, editing, selection, scrolling, and
styled rendering.

Phase 3 adds:

- Open, Save, and Save As through `sunlight-dialogs`
- UTF-8 Markdown (`.md`, `.markdown`) and plain text (`.txt`) import/export
- atomic temporary-file replacement with a safe direct-write fallback
- current path, format, revision, dirty-state, and unsaved-change tracking
- direct opening of a supported path passed in launch argv

## Persistence policy

Markdown supports ordinary paragraphs plus `*italic*`, `**bold**`, and
`***bold italic***`. Unsupported syntax remains visible as text. Plain text is
literal and never parses Markdown. UTF-8 BOMs and LF/CRLF/CR newlines are
handled; invalid UTF-8 is rejected. Underline text is preserved but underline
formatting is dropped when exporting Markdown or TXT.

An untitled Save invokes Save As. Save As infers format case-insensitively from
`.md`, `.markdown`, or `.txt`; a missing or unknown extension defaults to
Markdown. Failed reads, imports, writes, and canceled dialogs leave the
current document and dirty state unchanged.

## Architecture audit

- Existing `sunlight-dialogs` typed OpenFile/SaveFile/Confirm requests are reused.
- Existing `sunlight_libc` bounded file-descriptor APIs provide reads, writes,
  and rename; Writer writes a temporary sibling before replacement.
- Helios Note's atomic-save approach informed fallback behavior, while Writer
  keeps serialization independent in `src/persistence.rs`.
- The shared Canvas remains file-format agnostic and Rapid Rabbit-compatible.

## Deferred

RTF, DOCX, ODT, HTML, PDF, recent documents, autosave, recovery files, and
system-wide MIME registration remain future work.
