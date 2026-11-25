# Helios static TLS fixture

This describes the original initialized-only milestone and its archived
evidence. For current build and replay commands, see [STATIC-TLS-BSS.md](STATIC-TLS-BSS.md).

The debug kernel feature `helios_dynamic_elf_tls_fixture` extends the existing
trusted interpreter gate. TLS setup accepts the exact embedded TLS application
image. Each launch gets one initial thread and one private, writable,
non-executable TLS page. The page sits below the stack with an unmapped page
between them. The normal process context loads the guest FS base before user
entry, and the interpreter checks the initial value before jumping to the
application.

## ELF and ABI

GCC builds a freestanding x86-64 Linux ABI PIE with `-ftls-model=local-exec`;
the fixture uses direct write/exit syscalls. The emitted object has
`R_X86_64_TPOFF32` references resolved by the static linker to `%fs:-4`.
The final PIE has no relocations, dependencies, constructors, or destructors.
It has one `PT_TLS` at file offset `0x20b0` and virtual address `0x30b0`
in the inspected build. File size, memory size, and alignment are each four
bytes. The initialization image is `11 00 00 00` (17). Its dynamic table
contains HASH, STRTAB, SYMTAB, STRSZ, SYMENT, DEBUG, FLAGS_1 (PIE), and NULL;
there are no TLS-specific dynamic tags.

The x86-64 local-exec layout places the aligned image immediately below the
thread pointer. Fresh pages are zeroed before copying the validated file
range. The probe has no TLS BSS tail, so runtime measurements exercise file
initialization, while tail zeroing uses the existing page initialization.
The TLS load has no TLS-specific stack or auxv requirement. The interpreter
still checks the initial stack alignment, argc/argv, environment, AT_NULL,
AT_PHDR, AT_PHENT, AT_PHNUM, AT_PAGESZ, AT_BASE, AT_ENTRY, AT_RANDOM, and
AT_EXECFN. Only the TLS branch of the interpreter uses FS; the
application needs it for every access.

The fixture does not support live exec into the TLS image, initial-exec,
local-dynamic or general-dynamic TLS, shared library TLS, dynamic TLS
relocations, a dynamic thread vector, thread creation, late module loading,
dlopen, or TLS destructors. Same numeric PID with a new incarnation is
not exercisable under the current allocator contract.

## Reproduce and inspect

```sh
bash tools/helios-dynamic-elf/build.sh tls milestone
python3 tools/helios-dynamic-elf/replay.py tcg --tls milestone
python3 tools/helios-dynamic-elf/replay.py tcg --tls milestone --replay
python3 tools/helios-dynamic-elf/replay.py kvm --tls milestone
cargo test -p sunlight-elf --target x86_64-unknown-linux-gnu
```

Evidence is under `target/helios-dynamic-elf-tls/milestone/`. Each accelerator
directory records the QEMU command, image hash, compiler commands, ELF
inspection, serial trace, host allocator tests, and machine-readable result.
The guest runs the preexisting controls and rollback cases, sequential TLS
children, the parent-before-collection TLS child, and replacement children.
The TLS rollback gate checks resource counts, including pages and page
tables, and checks that the failed process was never published. The stack
and interpreter fault cases must show that TLS was mapped before the fault.
Wayland and transport tests were not rerun.
