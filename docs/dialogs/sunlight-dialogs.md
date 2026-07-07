# Sunlight Dialogs

Sunlight Dialogs provides shared native SunlightOS dialogs so apps use one consistent system UI for common prompts and path selection instead of building one-off widgets.

## Purpose

- Keep dialog request and result types stable across apps and services.
- Centralize normal alerts, confirmations, text prompts, and file path selection.
- Reuse existing SunlightOS file, MIME, and icon infrastructure where practical.
- Leave room for future system dialogs without changing the transport shape again.

## Components

- `sunlight-dialogs`: shared request/result types, validation, and wire encoding.
- `sunlight-dialogd`: dialog host that receives IPC requests, shows the window, and returns the result.
- `sunlight-dialog`: CLI client for scripts, testing, and app bring-up.

## Supported Dialogs

- `Alert`
- `Confirm`
- `TextInput`
- `OpenFile`
- `OpenFolder`
- `SaveFile`

Future request kinds are reserved for:

- `ColorPicker`
- `FontPicker`
- `PrintDialog`

## Request API

Current request variants:

- `DialogRequest::Alert(AlertRequest)`
- `DialogRequest::Confirm(ConfirmRequest)`
- `DialogRequest::TextInput(TextInputRequest)`
- `DialogRequest::OpenFile(OpenFileRequest)`
- `DialogRequest::OpenFolder(OpenFolderRequest)`
- `DialogRequest::SaveFile(SaveFileRequest)`

### Alert

- `title`
- `message`

### Confirm

- `title`
- `message`
- `style`: `OkCancel` or `YesNo`
- `default_button`

### Text Input

- `title`
- `message`
- `default_value`
- `allow_empty`

### Open File

- `title`
- `initial_dir`
- `allowed_mime_types`
- `allowed_extensions`
- `allow_multiple`
- `show_preview`
- `confirm_button_label`

### Open Folder

- `title`
- `initial_dir`
- `confirm_button_label`

### Save File

- `title`
- `initial_dir`
- `suggested_name`
- `default_extension`
- `allowed_extensions`
- `overwrite_confirm`
- `confirm_button_label`

Apps should use the shared types directly instead of inventing custom dialog payloads.

## Result Semantics

Current result variants:

- `Ok`
- `Cancel`
- `Yes`
- `No`
- `TextSubmitted(String)`
- `Dismissed`
- `FileSelected(String)`
- `FilesSelected(Vec<String>)`
- `FolderSelected(String)`
- `SavePathSelected(String)`
- `Cancelled`
- `Error(String)`

Notes:

- `Confirm` stays explicit so callers can distinguish `yes/no` from `ok/cancel`.
- `SaveFile` returns a path only; the caller writes file contents.
- `Cancelled` is used by file dialogs for a clear script-friendly cancel path.
- `Error(String)` is reserved for host-side failures that should be surfaced to the caller.

## File Dialog Behavior

- `OpenFile` starts in `initial_dir` when valid, otherwise falls back safely to the normal home/default path.
- `OpenFile` allows folder navigation, returns a selected file path, and keeps folders navigable even when file filters are active.
- `OpenFolder` returns the selected folder, or the current folder when no subfolder is selected.
- `SaveFile` shows a filename input, supports `suggested_name`, and appends `default_extension` when the entered name has no extension.
- Existing paths are never silently accepted when `overwrite_confirm` is enabled; the user must press `Save` again after the overwrite warning appears.
- The shared host currently shows lightweight details for the selected row instead of a heavy live preview.

## CLI Examples

```sh
sunlight-dialog alert --title "Warning" --message "Something happened"
sunlight-dialog confirm --title "Delete?" --message "Are you sure?"
sunlight-dialog confirm --title "Replace?" --message "Overwrite existing file?" --style ok-cancel
sunlight-dialog input --title "Rename" --message "New name:" --default "file.txt"
sunlight-dialog open-file --title "Open Image" --initial-dir /home/demo/Pictures --ext png,jpg
sunlight-dialog open-folder --title "Choose Project Folder" --initial-dir /home/demo
sunlight-dialog save-file --title "Export Note" --initial-dir /home/demo --suggested-name note.txt
```

CLI output is script-friendly:

- `ok`
- `cancel`
- `yes`
- `no`
- submitted text for `TextInput`
- selected path for `OpenFile`, `OpenFolder`, and `SaveFile`
- `cancelled` when a file dialog is dismissed

## Keyboard Behavior

- `Alert`: Enter accepts, Escape exits cleanly.
- `Confirm`: Enter activates the primary/default action, Escape cancels.
- `TextInput`: Enter submits, Escape cancels.
- `OpenFile`: Enter opens the selected folder or accepts the selected file.
- `OpenFolder`: Escape cancels; Enter chooses the current or selected folder.
- `SaveFile`: Enter attempts save using the current filename value; empty names stay rejected.

## Window Behavior

- Dialogs use the native dark/orange SunlightOS dialog styling.
- File dialogs use a compact shared structure: title, current path, file list, lightweight details, optional filename field, and action buttons.
- Dialog windows use dialog/transient overlay flags and stay on top where supported.
- The host closes the window immediately after a final result is produced.
- If parent-relative placement is unavailable, placement falls back safely to compositor-managed centering.

## App Rules

- Apps should use Sunlight Dialogs for normal alerts, confirms, text prompts, and file/folder/save path selection.
- Apps should not invent custom dialogs unless they need specialized UI.
- Request and result types should remain stable and easy to consume from app code and scripts.
- MIME detection and file icon resolution should be reused centrally instead of duplicated in each app.

## Current Limitations

- Only one dialog request is handled at a time; additional requests are rejected as busy.
- `allow_multiple` is parsed in the shared API, but the current host still behaves as single-select and reports that limitation in the UI.
- Preview is lightweight details-only for now; no expensive content decoding or recursive scans are performed.
- There is no breadcrumb editor, bookmarks pane, network locations view, device sidebar, or drag/drop yet.
- Parent-window centering is not wired yet.
- No true system-wide modal freeze is attempted.

## Planned Later

- `ColorPicker`
- `FontPicker`
- `PrintDialog`
- richer parent-window attachment
- queued dialog requests
- richer previews and optional multi-select support
