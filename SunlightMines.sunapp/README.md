# Sunlight Mines

A complete beginner Minesweeper (9×9, 10 mines) implemented as a genuine 16-bit real-mode DOS MZ executable using only Chronos-supported DOS, BIOS, VGA Mode 13h and INT 33h interfaces.

All game logic, board generation (safe first click), drawing, input, timer, and persistence execute inside the guest.

## Launch

sun-exec SunlightMines.sunapp

## Controls (guest)

- Left click: reveal cell (first click is always safe)
- Right click: flag / unflag
- R or N or F2: restart
- Esc: exit cleanly

## Persistence

Best time is stored in the bundle's private C: overlay at `C:\STATE\MINEBEST.DAT`.

## Build

See guest/sunmine/build.sh and guest/sunmine/README.md.
The checked-in SUNMINE.EXE was produced from the Free Pascal sources with the i8086-msdos cross compiler.
Ordinary kernel builds do not require Free Pascal.
