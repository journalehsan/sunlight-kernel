# VGALAB.COM

`VGALAB.COM` is the Prompt 5A 8086 real-mode graphics regression guest. It
selects BIOS Mode 13h, fills and draws directly through `ES=A000h`, blocks in
`INT 16h` until Escape, restores Mode 03h, and exits through `INT 21h/AH=4Ch`.

Rebuild the checked-in bundle asset with:

```sh
./guest/vgalab/build.sh
```

The output is `ChronosDosShell.sunapp/Program/TESTS/VGALAB.COM`. No host image
asset is involved; the source constructs the complete indexed framebuffer.
