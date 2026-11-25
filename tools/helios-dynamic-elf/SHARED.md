# Helios shared library fixture

This describes the original fixture milestone. For the current loader and
the same-image regression matrix, see [REAL-SHARED.md](REAL-SHARED.md).

`bash tools/helios-dynamic-elf/build.sh shared <run>` builds and records the
artifacts. Run `python3 tools/helios-dynamic-elf/replay.py tcg --shared <run>`,
then repeat with `--replay` and run separately with `kvm`. All three use the
same ISO. The run directory contains the full program headers, dynamic tags,
symbol and relocation tables, artifact hashes, and serial evidence.

The debug fixture accepts one literal DT_NEEDED name, `libhelios-value.so`,
for the shared probe. It selects `Program/libhelios-value.so` in the embedded
SunApp bundle and checks its bytes against the kernel's build input. No path is
constructed from DT_NEEDED, and no mutable guest file or host file is opened.
The library has one global, default visibility function. Its ELF entry points
to that function to use the same executable segment validation as
the interpreter; the loader never dispatches to the library entry point.

The loader maps the application, interpreter, then library in one unpublished
address space. The shared PIE uses bias `0x20000000`, the interpreter uses
`0x80000000`, and the library uses `0x90000000`. Load virtual addresses are
added to the bias with checked arithmetic. The application must fit below the
interpreter; the interpreter and library each have a separate 16 MiB window.
The mapper also checks for occupied pages before allocating. PT_LOAD provides
read, read/execute, or read/write permissions. This app and library omit RELRO;
the fixture rejects it. The GOT slot is written before publication, with no
lazy resolver. The initial stack keeps AT_BASE pointing to the interpreter and
AT_PHDR/AT_PHENT/AT_PHNUM/AT_ENTRY describing the application. The interpreter
checks the stack before jumping to AT_ENTRY.

The probe uses SysV DT_HASH and one RELA entry through DT_JMPREL, a
`R_X86_64_JUMP_SLOT` with zero addend. It has no regular RELA or REL table.
The library has a SONAME and no dependencies or relocations. Neither emits TLS
or constructor/destructor metadata. The existing RELATIVE control uses the
unchanged dependency-free relocation path.

The process image transaction releases all staged mappings on error. The
immutable bundle bytes are kernel-owned for the boot lifetime; mapping the
library acquires no process-owned file reference. Existing process exit,
collection, and address-space cleanup rules apply to all three images.
