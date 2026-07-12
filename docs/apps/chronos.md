# Chronos Milestone Two

Chronos is a native SunlightOS DOS text-mode compatibility application:

```text
SunlightOS keyboard event -> BIOS keyboard queue -> 16-bit guest
-> BIOS/DOS console or B8000 write -> guest video memory -> native window
```

## Components

- `chronos-core` owns private 1 MiB real-mode memory, CPU state, `.COM`
  loading/PSP initialization, decoder, BIOS/DOS dispatch, keyboard queue,
  video model, execution state, and traps.
- `sunlight-chronos` is a normal `sunlight-ui` window. It injects only its
  focused window events into the core runtime and renders the guest's B8000
  cells. It does not reproduce guest UI in host code.
- `chronos-core/guests/chronos-interactive.asm` documents the bundled
  interactive `.COM` guest; the checked-in byte image is in `sample.rs`.

## Execution Model

Guests transition through `Ready`, `Running`, `WaitingForInput`, `Exited`,
`Halted`, and `Trapped`. `Running` guests are limited to a bounded instruction
slice per native tick. A blocking `INT 16h` or DOS console read stores pending
input context and moves to `WaitingForInput`; waiting, exited, halted, and
trapped guests execute no instructions until an appropriate wake event.

The decoder supports the milestone-one instructions plus 16-bit ModR/M memory
addressing, segment overrides, register/memory and segment `MOV`, `LEA`,
stack/call/return/control flow, arithmetic/flag operations, `80h/81h/83h`
groups, direction/string operations including resumable `REP`, and foundation
flags/register operations. Unsupported opcodes and malformed prefixes retain
structured diagnostics.

## Text Video And BIOS Data Area

Color text memory at physical `0xB8000` is authoritative: 80×25 cells occupy
4,000 bytes, with character then attribute. Direct byte, word, and wrapped
guest memory writes mark their video row dirty. BIOS output uses the same
memory, so no independently mutable text grid exists.

Chronos initializes the supported BIOS Data Area fields in segment `0040h`:
mode (`49h` = `03h`), columns (`4Ah` = 80), page size (`4Ch`), page-zero
cursor position (`50h`), cursor shape (`60h`), and active page (`62h`).
Cursor state is synchronized among those fields, BIOS calls, and rendering.

`INT 10h` supports mode 03h, cursor set/get, scroll/clear regions, read cell,
repeated character writes, teletype, and mode reporting. The native renderer
uses all sixteen standard DOS colors; attribute bit 7 is treated as VGA blink
semantics visually ignored for now (background remains bits 4–6). CP437
conversion is modular and includes common box-drawing glyphs. The native text
grid uses the MiniType `MonoRegular` Fira Code face at a larger 8×16 cell size.

## Input

`sunlight-ui::Event::Key` supplies printable text once. `KeyPress` supplies
control and extended PS/2-set-1 keys so printable input is not duplicated.
The translation yields `BiosKey { ascii, scan_code }`; extended keys carry
ASCII zero. Shift/Ctrl/Alt state is exposed through `INT 16h/AH=02h`.

- `INT 16h/AH=00h`: blocking read without polling.
- `INT 16h/AH=01h`: nonblocking check preserving the queued key and `ZF`.
- `INT 16h/AH=02h`: available modifier flags.
- `INT 21h/AH=01h`, `06h`, `07h`, `08h`, `0Ah`, `0Bh`, and `0Ch`: basic DOS
  console reads, nonblocking status, queue flushing, and line editing.

The DOS buffered-line service checks all guest-provided bounds, supports
printable insertion, Backspace, and CR termination, and never blocks the host
thread.

## Security Boundary

Chronos exposes only bounds-checked guest memory and logical BIOS/DOS calls.
Guest `CLI`/`STI` alter only emulated flags. No guest code can execute host
code, issue native syscalls, touch ports/devices/files, access global keyboard
input while unfocused, or disable host/kernel interrupts.

## Compatibility

```text
Feature                         Status
------------------------------------------------
.COM loading                    Supported
Real-mode execution             Supported
DOS text output                 Supported
BIOS keyboard input             Supported
DOS console input               Supported
80×25 color text mode           Supported
Direct B8000 writes             Supported
DOS cursor                      Supported
MZ EXE                          Not yet
Filesystem                      Not yet
Mouse                           Not yet
VGA graphics                    Not yet
386/DPMI                        Not yet
```

## Current Limits

Only page zero and text mode 03h are implemented. There are no DOS drives,
filesystems, EXE loading, mouse/video graphics, ports, timer hardware, or
protected-mode support. The renderer currently repaints its text rectangle
when a frame is requested, while guest memory exposes dirty rows for a
future partial-canvas implementation.
