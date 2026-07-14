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

Guests transition through `Ready`, `Running`, `WaitingForInput`,
`YieldedUntilTimer`, `Exited`, `Halted`, and `Trapped`. `Running` guests are
limited to a bounded instruction slice per native tick. Exhausting a slice
schedules an input-independent continuation. `INT 28h` moves the guest to
`YieldedUntilTimer` and arms a bounded native deadline; keyboard or mouse input
may wake it early. A blocking `INT 16h` or DOS console read stores pending input
context and moves to `WaitingForInput`; waiting, exited, halted, and trapped
guests execute no instructions until an appropriate wake event.

While a guest is `Running`, Chronos requests app-local ticks from `sunlight-ui`
instead of placing each continuation behind a display `EVENT_POLL` IPC round
trip. The app loop limits this to eight local ticks, then performs one bounded
display poll. This keeps budget continuation independent of input while
preventing continuous guest execution from starving pointer and keyboard
delivery. `YieldedUntilTimer` continues to poll normally until its bounded
deadline or an early input wake.

Chronos records bounded wake evidence containing the wake source, guest state,
wait reason, CS:IP, retired-instruction count, next deadline, and framebuffer,
palette, and mouse generations. This separates guest execution liveness from
native rendering and input delivery when diagnosing partial frames.

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

`INT 10h` supports mode 03h, cursor shape and position set/get, scroll/clear regions, read cell,
repeated character writes, teletype, and mode reporting. The native renderer
uses all sixteen standard DOS colors; attribute bit 7 is treated as VGA blink
semantics visually ignored for now (background remains bits 4–6). CP437
conversion is modular and includes common box-drawing glyphs.

## Prompt 2.5 Polish

The native window identity is consistently **Chronos - Sunlight DOS Terminal**,
with the compact **16-bit real-mode guest** subtitle. The chrome uses MiniType
UI text, while the 80×25 DOS surface uses the embedded MiniType `MonoRegular`
Fira Code asset with fixed 9×16 cell metrics. This keeps DOS glyph placement
cell-perfect without relying on proportional layout or ligatures.

The DOS palette is a deep SunlightOS navy/indigo surface rather than harsh
black, while retaining differentiated DOS attribute colors and a restrained
orange cursor accent. The grid is framed with the existing SunlightOS panel
and border language; no gradients or non-terminal decoration are applied to
guest cells.

Printable teletype output writes the current cell before advancing exactly one
column. CR sets column zero, LF preserves the column and advances a row, CR/LF
therefore starts the next line correctly, Backspace clears one valid preceding
cell, and automatic wrap or scrolling occurs only after the rightmost cell is
written. BIOS cursor state, BDA cursor state, and the native caret share this
single logical cursor model. Direct B8000 writes remain authoritative for
cells and deliberately do not invent cursor movement.

Polling `INT 33h` mouse input is routed in both mode 03h and mode 13h after
the guest shows its DOS mouse cursor. Text-mode guests can select ranges such
as `0..79` and `0..24`; Chronos maps the native 80x25 grid into those ranges.

The status bar reports `Ready`, `Running`, `Waiting for Input`,
`Yielded until timer`, `Exited with code N`, `Guest Halted`, or `Guest Trapped`
from the actual runtime state.
Waiting guests execute no slices; cursor blinking is host-side only and does
not wake the blocked guest. The UI only requests frames for guest/state changes
or the normal 500 ms caret phase change.

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

Native pointer snapshots travel from `sunlight-mouse` through the compositor's
exact content-window ID and the `sunlight-ui` event dequeue into Chronos. Only
the scaled 320×200 graphics viewport is accepted: title/subtitle/status chrome
and letterboxing are excluded, all arithmetic is checked, and the four content
edges map exactly to `(0,0)`, `(319,0)`, `(0,199)`, and `(319,199)`. Button
capture is retained until release, duplicate physical states do not create a
second edge, and focus loss releases capture and clears held guest buttons.

`[CHRONOS-MOUSE]` diagnostics record display polls, available/dequeued events,
wrong-window replies, bounded local ticks, interleaved polls, received motion
and button events, outside-content rejections, mapped coordinates, state and
generation changes, button edges, and the latest `INT 33h AX=0003` BX/CX/DX.
The display server separately reports event polls, available snapshots,
wrong-window polls, and pointer ownership by another window.

Mode 13h deliberately shows two different cursors today: the compositor's
native safety cursor and one guest cursor drawn by Chronos from the emulated
INT 33h state. Chronos does not create two guest overlays and does not hide the
native cursor globally; region-scoped hiding requires a future compositor API.

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
MZ EXE                          Supported subset
DOS drives/files                Supported subset
INT 33h mouse                   Supported
VGA Mode 13h graphics           Supported
386/DPMI                        Not yet
```

## Current Limits

Only real mode and the documented DOS/BIOS/VGA subsets are implemented; there
is no protected-mode or DPMI support. Hardware timing is cooperative rather
than cycle-accurate. The renderer currently repaints its text rectangle when a
frame is requested, while guest memory exposes dirty rows for a future
partial-canvas implementation.
