# PALCYCLE.COM

`PALCYCLE.COM` is the Prompt 5A.1 8086 real-mode palette-animation guest. It
draws a fixed indexed Mode 13h image, snapshots VGA DAC entries 32 through 63,
and then animates only by rotating those entries through ports `03C8h` and
`03C9h`. Escape restores the snapshot through the DAC, selects Mode 03h, and
exits with code zero.

Rebuild the deterministic checked-in shell asset with:

```sh
./guest/palcycle/build.sh
```

This requires NASM and writes
`ChronosDosShell.sunapp/Program/TESTS/PALCYCLE.COM`. No host animation or image
asset is involved.
