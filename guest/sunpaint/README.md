# SUNPAINT.COM

`SUNPAINT.COM` is the Prompt 5B 8086 real-mode paint guest. It resets and
polls the virtual DOS mouse through INT 33h, selects Mode 13h through BIOS,
and performs every toolbar, clear, line, and erase write itself at A0000.

The virtual mouse reset default is the conventional `0..639 x 0..199` range.
SUNPAINT explicitly selects `0..319 x 0..199`, giving one logical coordinate
per Mode 13h framebuffer pixel.

Rebuild deterministically from the repository root:

```sh
./guest/sunpaint/build.sh
```

The output is installed as `C:\TESTS\SUNPAINT.COM` in the Sunlight DOS Shell
bundle. Controls: left drag paints, right drag erases, toolbar clicks or keys
1 through 8 select a color, C clears the canvas, and Escape exits.
