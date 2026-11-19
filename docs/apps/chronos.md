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

### XLAT and segment prefixes

The decoder carries a segment override and the 80386 operand/address-size
prefix state into instruction dispatch. For the modeled real-mode segment
registers, `26h`, `2Eh`, `36h`, and `3Eh` select ES, CS, SS, and DS; the last
segment prefix in a prefix run wins. Prefix state belongs to one instruction.
This CPU state currently has no FS or GS registers, so `64h` and `65h` remain
unsupported instead of aliasing another segment. `66h` does not affect XLAT's
byte result. `67h` selects EBX-based XLAT addressing in the 386 profile; the
resulting real-mode offset is still checked against the 64 KiB segment limit.

XLAT reads one byte at `DS:(BX + zero-extended AL)` by default. An override
selects the segment while leaving the offset calculation unchanged. With
address size 32, the base is EBX and addition is 32-bit before segment-limit
validation. The byte is written through the AL alias, preserving upper EAX,
BX/EBX, FLAGS, and the normal guest memory bus behavior.

### CWD and CDQ

Opcode `99h` follows the decoded operand size. The default 16-bit form (CWD)
sign-extends AX into DX; with the 80386 real-mode profile, a `66h` prefix
selects the 32-bit form (CDQ), sign-extending EAX into EDX. Both forms leave
the accumulator and FLAGS unchanged. Prefix state is consumed with the
instruction, so a following unprefixed `99h` uses the default 16-bit width.

### Signed integer division

`F7 /7` divides signed DX:AX by a word, or signed EDX:EAX by a dword
with the `66h` prefix in the 80386 real-mode profile. The divisor can be a
register or memory addressed through the existing 16-bit ModR/M decoder.
The quotient truncates toward zero and goes into AX/EAX; the remainder
goes into DX/EDX. Chronos leaves FLAGS unchanged, including the arithmetic
status flags that Intel defines as undefined for IDIV. A zero divisor or
a quotient outside the signed destination range produces the runtime's
`DivideError` trap at the instruction start without writing either result.
A dword memory operand crossing the real-mode segment limit produces
`SegmentLimit` before division. See the [DN compatibility report](dos-navigator.md#dn-17-signed-dword-division).

## PC System Control Port B

Chronos exposes a subset of DOS-era PC devices, rather than a complete XT or
AT chipset. CPU instruction profiles are independent of the virtual device
map. Port `0x61` uses the AT-compatible System Control Port B layout (IBM
Personal Computer AT Technical Reference, system board timer/counter;
Phoenix CBIOS for IBM PS/2 Computers and Compatibles, section 52).

Byte writes store bits 0–3 per runtime: PIT channel 2 gate, speaker data
enable, active-low parity-check enable, and active-low channel-check enable.
Byte reads return those bits, initially all zero. The gate and speaker bits
are control readback only: there is no virtual PIT channel 2 or audio output.
Refresh and PIT output status (bits 4–5) are fixed low, independent of time
and read count; no channel or parity errors are generated (bits 6–7).
Writes to bits 4–6 are ignored. Bit 7 is a write-one Timer 0 latch reset; it
has no effect because Chronos has no virtual Timer 0/IRQ0 latch, and never
persists in the port. Word accesses and adjacent unsupported ports still trap.

## INT 15h BIOS fallback

Chronos installs a read-only BIOS handler at `F000:E000` and writes its address
to INT 15h's guest IVT entry at physical `00054h`. The handler is part of the
virtual PC device model shared by all CPU instruction profiles. No INT 15h
subservices are currently implemented. The handler returns the IBM AT
unsupported-service status (`AH=86h`, `CF=1`), preserves AL and the other
registers, and changes only CF in the saved FLAGS before IRET. IBM's AT BIOS
V3 used a far return and could expose other live flags; Chronos adopts the
documented error status, with explicit preservation of other caller flags.
This is a minimal BIOS policy, not an IBM ROM image or a TopView service.

INT 15h reads the live IVT and enters through the ordinary guest interrupt
frame: push FLAGS, CS and next IP, then clear live IF and TF. DOS vector
services and direct IVT writes still let guests replace the vector. The hook
can return on its own, tail-jump to the saved predecessor, or call it with
PUSHF and a far call. In the latter case the predecessor sets CF in the inner
frame; the hook decides how to report status to the original caller. There is
no host recursion or redispatch through the vector inside the BIOS routine.
Guest code, including a hook that calls itself, remains subject to the usual
instruction budget. Other DOS and BIOS dispatch paths are unchanged.

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

`INT 10h/AH=1Ch` supports query, save and restore for the BIOS data area
only (`CX=0002h`). The size query returns two 64-byte blocks: a zero-filled
20h-byte header with the little-endian BIOS area offset `0020h` at header
offset 02h, followed by 60h bytes from `0040:0049h–00A8h`. Save and restore
require an `ES:BX` range in guest RAM with no segment-offset or physical
wrap. Restore validates the header and modeled BDA fields before updating
the live mode, cursor, and BDA. It does not perform a BIOS mode set or reset
video memory, the palette, or the blink/intensity setting.

The hardware register file and the complete VGA DAC controls are not
modeled. Empty masks, masks containing any other bit (including mixed
masks), invalid subfunctions, and invalid buffers return `AL=00h`.
Success returns `AL=1Ch`; only a successful size query modifies `BX`.
Flags and the other registers are preserved. This profile follows the
offset header in Phoenix's *System BIOS*, pp. 254–256, and the 96-byte
BDA range in Ralf Brown's Interrupt List.

Only real mode and the documented DOS/BIOS/VGA subsets are implemented; there
is no protected-mode or DPMI support. Hardware timing is cooperative rather
than cycle-accurate. The renderer currently repaints its text rectangle when a
frame is requested, while guest memory exposes dirty rows for a future
partial-canvas implementation.

## VGA CPU address decoding

`INT 10h / AH=12h / BL=32h` uses the CPU address-decoding contract of
the IBM PS/2 VGA BIOS (see IBM, *PS/2 and PC BIOS Interface Technical
Reference*, INT 10h Alternate Select, and Phoenix, *System BIOS for IBM
PC/XT/AT Computers and Compatibles*, Video Subsystem Enable/Disable,
pp. 184–185).
The IBM integrated VGA implementation uses motherboard setup port 46E8h;
the Phoenix description identifies video memory and video I/O addresses
as the targets. This is distinct from display refresh control
(`BL=36h`). RBIL also warns that some adapters do not implement
`BL=32h`. Chronos selects this service for its single virtual color VGA
adapter, independently of the guest CPU profile.

`AL=00h` connects the CPU decoder and `AL=01h` disconnects it. Both
return `AL=12h`; AH, the remaining registers, and FLAGS are preserved.
BH is not an input. Reserved AL values leave decoding unchanged and
return `AL=00h` as Chronos policy. A guest INT 10h hook can intercept
this service. The older built-in INT 10h paths still lack ROM chaining.

The same decoder state controls guest memory at A0000h–AFFFFh and
B8000h–BFFFFh and the modeled DAC I/O at 03C7h–03C9h. Disconnected
reads of modeled addresses return FFh per byte; writes are ignored.
Conventional RAM, BIOS ROM, unrelated ports, and the policy for
unmodeled I/O operations are unaffected. Display reads of stored text
cells and indexed pixels bypass the CPU decoder, so the current image
and palette remain available. The service changes neither the mode,
page, cursor, blink settings, nor refresh. Chronos does not model the
IBM setup port, switchable physical adapters, monochrome VGA decoding,
a full VGA register file, or scan timing.

## AAM and keyboard status

Opcode `D4 ib` divides the original AL by the immediate byte, returning
the quotient in AH and remainder in AL. A `66h` prefix does not widen the
operation. SF, ZF, and PF reflect AL; Chronos preserves the undefined
CF, AF, and OF. A zero base raises the runtime's `DivideError` at the
instruction start without updating the result or FLAGS.

BIOS `INT 16h/AH=11h` peeks at the same FIFO of translated BIOS key
events as `AH=01h`. When an event is queued it returns its scan code in
AH and character byte in AL with ZF clear. When empty it sets ZF and
preserves AX. The check leaves the event queued and preserves all other
FLAGS. The FIFO carries ASCII-zero special keys but has no separate
legacy versus enhanced translation or filtering. This service does not
add enhanced read (`AH=10h`) or extended shift status (`AH=12h`).

## DOS free disk space

INT 21h/AH=36h queries the current drive with DL=00h or an explicit
drive with DL=01h for A:, 02h for B:, 03h for C:, and so on. A drive
with allocation metadata returns sectors per cluster in AX, free
clusters in BX, bytes per sector in CX, and total clusters in DX.
The query preserves FLAGS and the selected drive.

The drive table accepts an allocation snapshot from the backing
filesystem. The snapshot is invalidated when the drive's file maps
change; a backend must also refresh it after external allocation changes.
Chronos returns AX=FFFFh for unmapped drives, missing snapshots, and
figures that cannot be represented accurately in the DOS registers.
On these paths BX, CX, and DX are preserved too.

The current native and host diagnostic adapters populate file maps,
which have no disk geometry, cluster allocator, or finite volume
capacity. Imported file sizes and per-file import limits cannot supply
these figures. Those drives therefore report capacity unavailable.
