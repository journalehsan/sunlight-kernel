# Sunlight DOS Shell guest source

`SUNSH.EXE` is intended to be compiled as a Free Pascal 3.2.2 `i8086-msdos`
MZ executable. It uses Turbo Pascal-compatible syntax and bounded static
strings; it does not participate in normal Cargo builds.

## Rebuild

Provide a Free Pascal cross compiler and matching minimal i8086/MS-DOS RTL:

```sh
PPC8086=ppc8086 FPC_I8086_RTL=/path/to/units/msdos ./guest/sunshell/build.sh
```

The compiler invocation is:

```sh
ppc8086 -Tmsdos -Pi8086 -Mtp -WmLarge -Wh -Xs -O1 -Fu/path/to/units/msdos -FD/usr/bin -XP -oChronosDosShell.sunapp/Program/SUNSH.EXE guest/sunshell/sunshell.pas
```

The large model and huge unit-code option keep the i8086 RTL within real-mode
segment limits. The target is DOS MZ, not GO32/DPMI, PE, NE, LE, or LX.

## Current guest smoke path

`SUNSH.EXE /C C:\MIDTERM.BAT` is exercised by the Chronos core regression. The
bundled batch test uses guest-side `CLS`, `ECHO`, `MD`, `COPY`, `TYPE`, `DIR`,
`DEL`, `RD`, and `EXIT`, then returns `0` after printing
`CHRONOS MIDTERM: PASS`.

The shell remains an incremental implementation rather than the complete Prompt
4.5 command language. In particular, it does not yet provide the requested
line editor/history, variable expansion, `IF`/`GOTO`/`CALL`/`SHIFT`, redirection,
or the full external-program test suite.

## Desktop launch

The normal SunlightOS initramfs includes the bundle at
`/Applications/ChronosDosShell.sunapp`. The desktop's **Sunlight DOS Terminal**
entry routes through `sun-exec`, which validates the bundle and starts the
native Chronos adapter with `SUNSH.EXE` as its guest entry. Interactive mode
shows the guest-side prompt:

```text
CMD C:\>
```

`./tools/runs.sh --build` rebuilds `SUNSH.EXE` only when both `PPC8086` and
`FPC_I8086_RTL` are set; otherwise it keeps the checked-in DOS MZ asset so
ordinary Rust-only builds remain reproducible.

## Compatibility findings

The current Free Pascal RTL startup requires 8086 `SHR`, accumulator-immediate
logic instructions, `REPE`/`REPNE` string scans and comparisons, and DOS
`AH=44h`, `AH=59h`, and safe `AH=71h` rejection. These are implemented in
Chronos generally, with focused Rust regression coverage.
