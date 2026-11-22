# FreeDOS reference setup

Reference media: [FreeDOS 1.4 Floppy Edition](https://www.freedos.org/download/),
`FD14-FloppyEdition.zip`, pinned in [upstream.sha256](upstream.sha256).
The files below are local scratch copies only. Use a fresh scratch directory
(the captured monitor driver expects `/tmp/dn22`) with QEMU, mtools, NASM and
ImageMagick. Keep the original DN tree read-only for the experiment.

```sh
d=docs/audit/chronos-dn/dn-22
mkdir -p /tmp/dn22
unzip -p /tmp/dn22/fd14.zip 144m/x86BOOT.img >/tmp/dn22/boot.img
cp -a target/dos-navigator/dn151 /tmp/dn22/ref-drive-verified
nasm -f bin "$d/reference-exec.asm" -o /tmp/dn22/ref-drive-verified/RUNDN.COM
printf 'LASTDRIVE=Z\r\nBUFFERS=20\r\nFILES=40\r\nSHELL=\\FREEDOS\\BIN\\COMMAND.COM \\FREEDOS\\BIN /E:2048 /P=\\FDAUTO.BAT\r\n' >/tmp/dn22/FDCONFIG.SYS
printf '@echo off\r\nver\r\nC:\r\n' >/tmp/dn22/FDAUTO.BAT
printf '@echo off\r\nRUNDN.COM\r\nif errorlevel 1 goto fail\r\necho DN exit code 0\r\ngoto end\r\n:fail\r\necho DN exit code nonzero\r\n:end\r\n' >/tmp/dn22/ref-drive-verified/CHECKDN.BAT
mcopy -o -i /tmp/dn22/boot.img /tmp/dn22/FDCONFIG.SYS /tmp/dn22/FDAUTO.BAT ::
DN22_OUT=/tmp/dn22 TMPDIR=/tmp/dn22 python3 "$d/reference-monitor.py"
```

The setup boots FreeDOS to C:\ with no installer, device drivers, or DN
state. QEMU uses the copied directory as a writable FAT drive; DN may write
history there. The monitor script captures the shell, direct DN startup,
the drive picker, both populated panels, focus/selection and the recovered
shell. The guest has A: and C: in the drive picker; host Chronos instead
reports C:, D: and T:. Full-screen draws can take several seconds on the
FAT-backed VM, so check the actual captures when replaying. Monitor
register dumps sample whichever code owns the CPU at the time.
