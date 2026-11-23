# BTop terminal rendering after BTop-22

BTop-22 verified interaction and storage accounting, but its dashboard
captures are missing borders, graphs, and parts of labels. The shared TTY
grid advanced the cursor for every UTF-8 byte while both renderers drew
ASCII only. It also mapped RGB and extended palette colors to 16 colors.

The grid now decodes UTF-8 across read boundaries, keeps the colors from
SGR, and supplies both the desktop TTY and windowed terminal with shared
rendering for common single-cell graphics. This does not implement
double-width characters, combining sequences, or full Unicode font coverage.

The host split-read regression failed on the old grid and passed on the
new one. All 87 host tests across the TTY, framebuffer renderer and
windowed terminal passed.

The mounted replay in `target/btop-ui/verified-mounted/` used the unchanged
pinned ELF on a diagnostic image containing both rebuilt terminal binaries.
The screenshots showed borders, Braille graphs, meters and theme colors.
Help opened and closed, the worker and leader were reaped, and both btop
and a separate `/bin/proc-native` command exited 0 on the same TTY.
The final CD-only replay in `target/btop-ui/final-cd/` includes the
network throughput triangles and passed the same interaction checks.

To reproduce the image, explicitly rebuild `sunlight-tty-server` and
`sunlight-terminal` with the service linker flags
`-C link-arg=-Tservices/user-space.ld -C relocation-model=static` and copy
the release ELFs into `target/x86_64-unknown-none/release/` before the
BTop-22 image recipe. The kernel build can reuse existing service ELFs;
a newly built kernel alone does not prove the renderer changed.

Load average remains a separate Helios gap. The guest trace shows Linux
`sysinfo` (99) returning ENOSYS. Disassembly of the pinned static musl
`getloadavg` (at `0x60a980`, located via `target/btop-01/btop.map`)
shows it reading the result buffer without checking the syscall result.
Those displayed numbers are not guest load measurements. A proper Helios
implementation needs real scheduler load accounting.
