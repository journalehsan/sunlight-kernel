# Helios: one trusted toolchain shared object

```sh
bash tools/helios-dynamic-elf/build.sh real-shared verified
python3 tools/helios-dynamic-elf/replay.py tcg --real-shared verified
python3 tools/helios-dynamic-elf/replay.py tcg --real-shared verified --replay
python3 tools/helios-dynamic-elf/replay.py kvm --real-shared verified
cargo test -p sunlight-elf --target x86_64-unknown-linux-gnu
```

Run artifacts are under `target/helios-dynamic-elf-real-shared/verified/`.
The C PIE and shared object are unmodified GCC/linker output. The shared
object has no entry point. Both use SysV `DT_HASH` and omit RELRO. The PIE
imports one global function through a single `R_X86_64_JUMP_SLOT` in
`DT_JMPREL`; the library has two exports, no dependencies and no
relocations. The dependency-free RELATIVE probe runs separately.

The existing debug feature selects a library from the trusted bundle
allowlist. The application bytes must match its selected build input.
The library must match the canonical bundle path, the expected SHA-256,
and the names in both dynamic tables. Only these selected inputs use the
unstamped System V ELF convention. The loader computes the library bias
from its lowest load page and the chosen address window, then uses the PIE's
relocation index and dynamic symbol name to find a unique executable
definition. The supported JUMP_SLOT form requires a zero addend. The
address space is published only after the staged mapping and writes succeed.

The positive path accepts one trusted dependency; it does not search the
filesystem for libraries. GNU hash, RELRO, TLS, init/fini, lazy binding, and
additional relocation formats on this one-library path remain outside the
supported subset. Neither C image links libc.

The original fixture's incomplete registry image is recorded separately
under `target/helios-dynamic-elf-shared-library/initial/`; its serial log
shows a rejection before spawn, despite the old result's generic failure
label. The corrected fixture image is under that root's `verified/`.
The new milestone's `initial-launch/` records a different pre-spawn
failure: its registry entry was present, but the launcher still required
the legacy stamp. The corrected launcher admits the conventional ELF
identification only for the selected probe; the kernel independently checks
the application's full byte identity.
