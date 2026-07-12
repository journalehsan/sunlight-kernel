# Chronos Milestone One

Chronos is a native SunlightOS compatibility application for a deliberately
small DOS vertical slice:

```text
DOS .COM bytes -> 16-bit interpreter -> DOS/BIOS dispatch -> text surface -> native window
```

## Components

- `chronos-core` is a `no_std`-compatible library. It owns the guest CPU state,
  private 1 MiB memory, `.COM` loader, instruction decoder, interrupt dispatch,
  runtime state, traps, and 80×25 DOS text surface.
- `sunlight-chronos` is a `no_std` native `sunlight-ui` application. It embeds
  the small Hello Chronos `.COM` guest and runs it in bounded `Event::Tick`
  slices before drawing the core-owned text surface in a normal window.

## Guest Model

Guest physical addresses use real-mode 20-bit wrapping:

```text
((segment << 4) + offset) & 0xFFFFF
```

Each `.COM` guest receives PSP segment `0x1000`, code/data/stack segments set
to that same segment, `IP=0x0100`, and `SP=0xFFFE`. The PSP contains `INT 20h`
at offset `0` and an empty command tail at `0x80`. MZ executables are rejected.

The interpreter currently supports `NOP`, immediate register `MOV`
(`B0-B7`, `B8-BF`), `INT imm8`, short and near relative `JMP`, and `HLT`.

## Interrupts

- `INT 20h`: successful termination.
- `INT 21h/AH=02h`: write `DL`.
- `INT 21h/AH=09h`: bounded `$`-terminated `DS:DX` string output.
- `INT 21h/AH=30h`: DOS 5.0 compatibility version.
- `INT 21h/AH=4Ch`: exit with `AL`.
- `INT 10h/AH=0Eh`: teletype text output, including CR, LF, and backspace.

Unsupported opcodes and interrupt services transition the guest to a structured
trap; they never panic or report false success.

## Security Boundary

Chronos gives guests only bounds-checked private memory and logical text
output. There is no host filesystem access, hardware port I/O, raw host
pointer exposure, syscall passthrough, drive layer, direct video memory, or JIT.
Instruction execution is capped per native event-loop tick.

## Current Limits And Next Step

Milestone one supports the embedded Hello Chronos `.COM` demonstration only;
native command-line/file loading is intentionally deferred because this
milestone has no safe virtual DOS drive layer. The next logical step is a
scoped virtual file source plus read-only DOS file APIs, then a slightly wider
instruction subset needed by small real-world text-mode programs.
