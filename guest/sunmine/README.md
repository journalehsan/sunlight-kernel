# Sunlight Mines guest

9x9 beginner Minesweeper as 16-bit real-mode DOS MZ.

## Build

PPC8086=ppc8086 FPC_I8086_RTL=/path/to/rtl/units/msdos ./guest/sunmine/build.sh

When the Free Pascal i8086 cross toolchain is unavailable, the script builds
the checked-in 8086-compatible NASM fallback instead. Both paths refresh the
application bundle and the DOS shell test copy.

The script also copies to ChronosDosShell.sunapp/Program/TESTS for direct launch tests.

## Checked-in binary

SUNMINE.EXE is included prebuilt. Ordinary builds use the checked-in asset.

## Features implemented in guest

- Safe first click (no mine or immediate neighbors)
- Iterative bounded queue flood fill (no recursion)
- **Text mode 03h** with direct B8000 writes (no pixel rendering)
- Event-driven redraw: only changed cells are redrawn
- DOS text-mode mouse via INT 33h
- Keyboard via INT 16h
- INT 28h cooperative yield (every 8 loop iterations)
- DOS time for timer
- Best time persistence to C:\STATE\MINEBEST.DAT (SMIN magic, v1, LE u16)
- R/N restart, Esc clean exit to mode 03h
- Win: FIELD CLEARED , Loss: MINE TRIGGERED
- Mine counter, elapsed, BEST display

All inside the DOS guest. No host Mines logic.
