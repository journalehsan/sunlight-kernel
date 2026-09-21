# Sunlight ClipBoard Usage

This directory documents the current clipboard foundation in SunlightOS.

## Components

- `sunlight-clipd` — clipboard service/daemon
- `sunlight-clip` — CLI client for reading and updating clipboard state
- `sunlight-kv` — persistence backend used by the clipboard history service

## What Works Today

- text clipboard items
- clipboard history
- restoring a previous history item
- clearing the current clipboard
- clearing clipboard history
- file-list/path clipboard items through CLI commands
- file and folder copy/cut/paste between Sunlight Files and the Vortex desktop
- multi-selection, Copy Path, new folders/text files, and refresh in both interfaces

## Start The Service

Make sure the clipboard daemon is running:

```sh
/sbin/sunlight-clipd
```

If your normal SunlightOS session startup already launches it, you do not need
to start it manually.

## CLI Commands

### Get current clipboard

```sh
sunlight-clip get
```

- prints the current text payload directly
- for non-text items, prints a summary instead
- prints `(empty)` when no current clipboard item exists

### Set text clipboard

```sh
sunlight-clip set "hello"
```

Example:

```sh
sunlight-clip set "SunlightOS clipboard test"
sunlight-clip get
```

### Show history

```sh
sunlight-clip history
```

Typical output includes:

- current marker
- history index
- item id
- item kind
- short preview/summary

### Restore a history item

You can select by index:

```sh
sunlight-clip use 0
sunlight-clip use 1
```

Or by item id:

```sh
sunlight-clip use 0x00000001
```

After restoring a text item:

```sh
sunlight-clip get
```

### Clear current clipboard

```sh
sunlight-clip clear
```

### Clear history

```sh
sunlight-clip clear-history
```

## File Path Clipboard Items

Single path:

```sh
sunlight-clip set-file /home/user/readme.txt
```

Multiple paths:

```sh
sunlight-clip set-files /home/user/a.txt /home/user/b.txt
```

Notes:

- this stores paths only
- it does not copy file contents
- paste these items in Files or on the desktop to copy their contents

## Files and Desktop Actions

Use the right-click menu or these shortcuts in the directory/desktop:

| Action | Shortcut |
| --- | --- |
| Copy selection | Ctrl+C |
| Cut selection | Ctrl+X |
| Paste into current folder / Desktop | Ctrl+V |
| Copy selected paths as text | Ctrl+Shift+C |
| Select all files | Ctrl+A |
| New folder | Ctrl+Shift+N |
| Refresh | F5 |

Files supports Ctrl-click to toggle selection and Shift-click to select a range.
On the desktop, drag a selection rectangle or use Ctrl+A; right-clicking an
already selected icon preserves the group. Computer, Home, Network, and mounted
drive shortcuts are not movable desktop files. Desktop folder icons open Files
at that folder. Both interfaces offer New Text File in their background menu.

Copy preserves the sources, including nested folders, empty files, and binary
contents. Copying beside the original produces names such as `report (copy).txt`.
Cut only marks the clipboard; the files move when pasted successfully. Cutting
into the original location reports that the item is already there. Transfers
reject overlapping source selections and pasting a folder inside itself.

A successful cut consumes the clipboard. If a multi-item move stops partway,
only the completed prefix is consumed, leaving the remaining paths for retry.
A clipboard value copied by another app during the operation is preserved.
Failures appear in the Files status bar or a desktop notification. Refresh or
refocus the source window after a transfer from the other interface.

### File-list protocol and build requirements

Both interfaces use `sunlight_ui::clipboard` and the existing `clipd` service;
there is no private application clipboard. Kind 2 retains the existing
NUL-separated UTF-8 path payload. Copy uses `x-sunlight/file-list`; cut uses
`x-sunlight/file-list;operation=cut`. The MIME field preserves the operation
through clipboard history and persistence. Existing CLI file lists remain copies.

`SET_CLIPBOARD` accepts an optional expected current item ID in word 1, and
`CLEAR_CLIPBOARD` accepts it in word 0; zero retains unconditional behavior.
These comparisons occur in the daemon before mutation, so finishing a move does
not overwrite a newer clipboard item.

Rebuild the kernel, clipd, Files, and Vortex together. New native syscalls
`ReadDirFrom` (148, fourth argument = entry offset), `RenameNoReplace` (149),
and `FileReserve` (150, fd plus expected final size) provide complete paged
listings, atomic destination-conflict checking, and safe large RAMFS copies.
`FileReserve` does not change the visible length. Files and Vortex reserve the
destination before streaming so RAMFS does not repeatedly reallocate the whole
file. Allocation pressure becomes a normal copy error and removes the incomplete
destination instead of triggering a kernel allocation panic.
Legacy ReadDir (60) and Rename (66) retain their existing ABI. RAMFS folder
moves update descendant paths together; read-only/static trees cannot be moved.

### Manual desktop verification

1. In a writable folder, create a text file and a folder containing several files.
2. Copy a selection in Files, focus the desktop, and paste. Confirm contents and
   original files remain, then copy from the desktop back into another folder.
3. Cut a desktop selection and paste into Files. Confirm the sources disappear,
   the destination is complete, and a second paste reports an empty clipboard.
4. Try pasting into an existing name or into a selected folder's child. Confirm
   an error appears and neither existing data nor source data changes.
5. Copy a folder containing more than 64 entries and verify every entry, including
   nested/empty files. Copy a file beside itself and verify its generated name.
6. Copy plain text in an editor, try file paste, and confirm the clipboard remains
   text. Copy Path and paste into the editor to verify the shared text clipboard.

## Current Behavior Notes

- history is bounded
- consecutive identical clipboard values are deduplicated when possible
- restarting the service preserves history when `sunlight-kv` persistence is available
- empty or missing clipboard state should not crash the service

## Current Limits

- text and file lists are supported; pasting text into a folder does not create a file
- image/binary payloads are not a complete end-user workflow yet
- there is no graphical `Win+V` picker in this doc version
- moves require a writable source and destination on the same mounted filesystem
- existing destination names are reported as conflicts; replacement and folder merging are not implemented
- file lists are limited to 2048 payload bytes by clipd
- transfers run synchronously and are bounded to 8192 entries / 64 levels; listings are bounded to 4096 entries
- native paths must fit the 256-byte syscall buffer and copied child names the 64-byte directory-entry field
- after a failed folder copy, completed child files/directories may remain at the destination; sources are retained

## Recommended Quick Test

```sh
sunlight-clip set "hello"
sunlight-clip get
sunlight-clip set "world"
sunlight-clip history
sunlight-clip use 1
sunlight-clip get
```

Expected flow:

- `get` returns `hello`
- history shows both `hello` and `world`
- `use 1` restores the older item
- final `get` returns `hello`
