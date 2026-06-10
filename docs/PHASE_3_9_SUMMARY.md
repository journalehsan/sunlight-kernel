# Phase 3.9 Implementation Summary

## Date: 2026-06-09
## Status: SHELL UX FEATURES ADDED — sysfetch + hostnamectl

---

## Overview

Phase 3.9 adds two "neofetch / hostnamectl style" inspection commands to the
`SunShell` user shell. Both are pure-display commands that render colorful
ASCII art and structured key/value data to the terminal.

- `sysfetch` — displays a sunburst ASCII logo (rendered in yellow ANSI
  escapes) next to a General / System / Appearance / Network breakdown
  of the running system.
- `hostnamectl` — displays the same kind of structured fields as
  systemd's `hostnamectl` (Static hostname, Icon name, Chassis,
  Machine ID, Boot ID, OS, Kernel, Architecture, Hardware, Firmware,
  etc.) but rebranded for SunlightOS / QEMU.

All values are hardcoded. Phase 4+ will source them from real syscalls
(`/proc`-style IPC, ACPI tables, DMI, etc.).

---

## Architecture: Multi-Chunk Output via Drain Endpoint

The default per-message IPC payload is **48 bytes** (8 × u64 words, with
word 0 = length and word 1 = remaining). `sysfetch` produces ~600 bytes of
output, `hostnamectl` ~700 bytes, neither fits in a single message.

### Protocol change

Replies now use a unified layout:

| Word | Meaning |
|------|---------|
| 0    | length of THIS chunk's data (≤ 48) |
| 1    | remaining bytes after this chunk (0 = last chunk) |
| 2..7 | chunk payload (6 words = 48 bytes max) |

A new `DRAIN_LABEL = 4` is added. After receiving a reply with
`word[1] != 0`, tty_server sends a `DRAIN_LABEL` request with
`word[0] = chunk_seq` to fetch the next chunk. sshl responds with the
next 48 bytes (or an empty `word[0] = 0, word[1] = 0` reply if
`seq * 48 >= LONG_OUT_LEN`).

### Code locations

- `sunshell/src/main.rs:130-137` — `DRAIN_LABEL`, `LONG_OUT_*` constants
- `sunshell/src/main.rs:285-340` — `LONG_OUT_BUF` statics + helpers
- `sunshell/src/main.rs:540-590` — `push_art_line`, `push_label_value`,
  `push_section_header`, `push_blank`, `push_line`, `format_uptime`
- `sunshell/src/main.rs:618-790` — `cmd_sysfetch` (with ASCII art) and
  `cmd_hostnamectl`
- `sunshell/src/main.rs:1015-1080` — `_start` main loop, handles
  DRAIN_LABEL and routes KBD_LABEL through the new long-output path
- `services/tty_server/src/main.rs:48` — `DRAIN_LABEL` constant
- `services/tty_server/src/main.rs:511-575` — `append_shell_reply`
  now drains after the primary chunk if `word[1] != 0`

---

## ASCII Art (sysfetch)

The current art is a 10-line sunburst rendered with ANSI yellow
(`\x1b[33m`) and bold (`\x1b[1m`) on the first five rows. The next
five rows are blank to keep labels (OS, Kernel, Uptime, Shell, User)
aligned with the art's right edge. The art is hardcoded byte literals
in `cmd_sysfetch`.

```
             \   |   /
              \  |  /
         .---' \ | / '---.
        ;       \|/       ;
        |     ___|___      |
        ;    /   |   \     ;
         '---;   |   ;---'
              /  |  \
             /   |   \
                                 [General]   OS:    SunlightOS 0.1
                                            Kernel: SunlightOS 0.1.0
                                            Uptime: 0h 22m 17s
                                            Shell:  SunShell
                                            User:   root@ehsan-21ahs1qm00
[System]    RAM:   240MiB / 256MiB
            CPU:   x86_64 QEMU Virtual CPU
            Disk:  2.1GiB / 30GiB
[Appearance] Theme:  SunlightOS Dark
             Colors: Orange/Yellow on Black
             Font:   Builtin 8x16
[Network]    Host:     ehsan-21ahs1qm00
             IP:       127.0.0.1
```

---

## hostnamectl output

A long-form structured dump (one `Field:` value per line, with the
field name in yellow bold, matching real `hostnamectl`):

```
[Static hostname:]
ehsan-21ahs1qm00

[Icon name:]
computer-laptop

[Chassis:]
laptop

[Chassis Asset Tag:]
No Asset Tag

[Machine ID:]
658603116ba54b838da9b2f28c288257

[Boot ID:]
35763b833f844053ae2fd0af6eabba4c

[Operating System:]
SunlightOS 0.1 (QEMU)

[Kernel:]
SunlightOS 0.1.0

[Architecture:]
x86-64

[Hardware Vendor:]
QEMU

[Hardware Model:]
Standard PC (i440FX + PIIX, 1996)

[Hardware SKU:]
SUNLIGHT-VM-1

[Firmware Version:]
Limine BIOS (1.17)

[Firmware Date:]
Tue 2024-01-01

[Firmware Age:]
1y 6month 0w 0d
```

---

## Files Changed

| File | Change |
|------|--------|
| `sunshell/src/main.rs` | New `cmd_sysfetch`, `cmd_hostnamectl`, long-output buffer, `DRAIN_LABEL` handling, helpers |
| `services/tty_server/src/main.rs` | `DRAIN_LABEL` constant, `append_shell_reply` drain loop, new word layout (data in words 2..7) |
| `kernel/src/main.rs` | Refactored `setup_key_injection` to switch sequences by `SUNLIGHT_INJECT_PHASE` env var; added `build_phase3_8_sequence` and `build_phase3_9_sequence` |
| `tools/test.sh` | Added `phase3.9` case; passes `SUNLIGHT_INJECT_PHASE=phase3.9` only for that gate |
| `tools/tests/phase3_9.expected` | New expected output for the 3.9 gate |

---

## IPC Drain Loop — Safety Notes

- `append_shell_reply` hard-caps the drain loop at 64 chunks
  (~3 KiB), preventing runaway loops if sshl ever returns a
  non-decreasing `word[1]`.
- The `DRAIN_LABEL` request uses `word[0] = chunk_seq`. sshl validates
  that `seq * 48 < LONG_OUT_LEN`; if not, it returns an empty
  `OUTPUT_LABEL` reply so tty_server exits the drain loop cleanly.
- `LONG_OUT_BUF` is a single static 1 KiB buffer, exclusively owned
  by sshl. There is no concurrent access.

---

## Test Gate — `phase3.9`

```
[PMM] 234/249 MiB free
[ELF]  Static ELF loader initialized
[KERN] spawn endpoint registered
[TTY]  Login success: root (uid=0, gid=0)
[TTY]  Spawning /bin/sshl (pid=5)...
[TTY]  Shell: sshl v0.1.0 running
[TTY]  cmd: whoami -> root
[TTY]  cmd: id -> uid=0(root) gid=0(root) groups=0(root),10(wheel)
[TTY]  cmd: useradd testuser -> OK
[TTY]  cmd: id testuser -> uid=1001(testuser) gid=100(users)
[TTY]  cmd: userdel testuser -> OK
[SunlightOS] Phase 3.8 OK
[TTY]  sysfetch invoked
[TTY]  hostnamectl invoked
```

The 3.8 baseline assertions still pass, plus the two new
"invoked" markers prove the commands reached the dispatch table.
The drain path runs silently after each command and writes the
full multi-chunk output to the TUI render buffer.

---

## Future Work (Phase 4+)

- **Real data sources**:
  - `Uptime` → new `SunlightSyscall::GetTicks` returning kernel
    `TICKS` counter, sshl formats as `Hh Mm Ss`.
  - `RAM` / `Disk` → expose PMM and VFS stats through a
    `SunlightSyscall::QueryStat` call.
  - `CPU` → ACPI/MP-table enumeration or hardcoded QEMU string.
  - `Hostname` → read from `/etc/hostname` via VFS.
- **TTY-side ANSI parsing** so colors render in the framebuffer TUI
  (currently `\x1b[33m` etc. are passed through as literal bytes).
- **Replace hardcoded ASCII art** with a runtime renderer that can
  pick from a small font + color palette.
- **Smaller chunk size** option for slow terminals (40 bytes vs 48).
