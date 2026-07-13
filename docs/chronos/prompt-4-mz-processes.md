# Chronos Prompt 4: MZ Loading and DOS Processes

## Executable detection

Chronos classifies guest program bytes centrally rather than trusting the file
extension. Non-MZ input is loaded as a flat `.COM` image at `PSP:0100`. `MZ`
(and deliberately accepted historical `ZM`) input is parsed as a DOS MZ
executable. When `e_lfanew` points at a recognized extended signature, `PE`,
`NE`, `LE`, and `LX` are rejected; Chronos does not execute Windows, OS/2, or
protected-mode formats as real-mode DOS programs.

The MZ parser reads explicit little-endian fields: magic, page and last-page
sizes, relocation count/table offset, header paragraphs, min/max allocation,
stack and entry CS:IP values, checksum, overlay number, and optional
`e_lfanew`. It bounds all calculations, requires `e_ovno == 0`, caps
relocations, requires the table to remain in the header, and requires each
relocation target to remain within the loaded image.

## Memory layout

`GuestMemory` remains a bounded 1 MiB, 20-bit real-mode address space.
Chronos reserves low memory/IVT/BDA below physical `0x10000` and video/BIOS
regions from `0xA0000` upward. The DOS paragraph arena is therefore
`0x1000..0xA000` (physical `0x10000..0x9FFFF`) and never overlaps reserved
regions.

`DosMemoryArena` allocates 16-byte contiguous blocks, tracks owner PSPs,
detects invalid owner and double free attempts, resizes only in place, reports
the largest free block, and coalesces neighbouring free blocks. A zero-size
resize is rejected. The current MCB-like representation is host-side only but
is intentionally shaped for later guest-visible MCB support.

MZ layout is:

```text
PSP segment
PSP + 0x10 = image load segment
image bytes
minimum extra paragraphs
```

The loader copies bytes after the declared header, applies `word +=
load_segment` for every validated relocation, initializes `CS:IP` and `SS:SP`
from the header, and sets `DS`/`ES` to the PSP.

## PSP and environment

Each process receives an initialized PSP containing the INT 20h entry,
allocation end segment, saved-vector slots, parent PSP segment, initial
standard handle table, environment segment, empty FCB areas, and a bounded
count/CR command tail. FCB file APIs are not claimed.

Every process has a DOS environment block:

```text
PATH=C:\0 TEMP=T:\0 TMP=T:\0 APPID=<id>\0 COMSPEC=\0 \0
word 1
guest executable path\0
```

Child `EXEC` with environment segment zero clones the parent variables; an
explicit segment must be valid guest-arena memory owned by the active process.
No host path is placed in the environment.

## Process execution

The runtime has one active `DosProcess` and, for this milestone, one suspended
parent. A context stores CPU registers, PSP and parent PSP, environment/DTA,
handle table, current drive, executable format, allocation details, and the
last child result.

`INT 21h/AH=4Bh`, AL=00 validates the bounded parameter block and command-tail
far pointer, resolves only virtual-drive paths, detects COM/MZ content, and
switches to the child. The parent executes no guest instructions while the
child is active. Standard and ordinary handles are copied as safe in-memory
handle metadata; child-only ordinary handles are dropped on exit and parent
handles remain valid.

`INT 20h`, `AH=00h`, and `AH=4Ch` terminate normally. Child termination frees
its process and environment blocks, clears searches/DTA/input wait state,
restores the parent context, and records the return code. `AH=4Dh` returns and
clears that stored result. Trapped children restore the parent and record
termination type 2 rather than trapping the parent runtime.

Memory services implemented are `AH=48h` allocate, `AH=49h` free, and
`AH=4Ah` resize. `AH=50h`, `AH=51h`, and `AH=62h` expose only the active valid
PSP; arbitrary PSP spoofing is rejected.

## CPU coverage

The real-mode interpreter retains the Prompt 3 subset and adds far immediate
and indirect calls/jumps, `RETF`, `RETF imm16`, `IRET`, `ADC`, `SBB`, `NEG`,
`NOT`, unsigned/signed `MUL`/`DIV`, shifts and rotates, `LAHF`/`SAHF`,
`LDS`/`LES`, `POP r/m16`, `JCXZ`, `INT 3`, and controlled `INTO` behavior.

`PUSH imm`, `ENTER`, and `LEAVE` require the explicit 80186-or-newer bundle
profile. Unsupported profile instructions produce a controlled guest trap;
they are not no-ops. Divide-by-zero and quotient overflow are guest divide
traps.

## Sample and current limits

`ChronosMzLab.sunapp` is a reproducible NASM-built sample. Its genuine MZ
parent has a non-zero relocation count and initialized stack metadata, reads
`MESSAGE.TXT`, allocates/resizes/frees memory, executes `CHILD.COM` with
`/from-parent`, receives exit code 42, and writes `C:\MZLAB.LOG` to the
private C: overlay.

Chronos still does not provide COMMAND.COM, batch parsing, load-only/overlay
EXEC modes, MZ overlays, protected mode, DOS extenders, raw guest MCB access,
multitasking, graphics, or full FCB/interrupt-vector ownership. Malformed
inputs can fail their Chronos instance but cannot expose host descriptors,
paths, or executable launching.
