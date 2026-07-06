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
- it is intended for future file-manager copy/paste integration

## Current Behavior Notes

- history is bounded
- consecutive identical clipboard values are deduplicated when possible
- restarting the service preserves history when `sunlight-kv` persistence is available
- empty or missing clipboard state should not crash the service

## Current Limits

- text is the main supported payload today
- image/binary payloads are not a complete end-user workflow yet
- there is no graphical `Win+V` picker in this doc version
- `sunlight-files` is not wired into clipboard copy/paste yet

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
