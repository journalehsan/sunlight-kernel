# Helios static TLS: BSS tail

The debug TLS gate accepts only the two embedded application images:
`elf-tls-init` is the initialized-only control; `elf-tls` exercises the
BSS tail. Each application runs with one initial thread and one private,
writable, non executable TLS page below the stack guard. The existing
transaction owns the mapping through the process address space and installs
the guest FS base before entering the interpreter.

GCC compiles the freestanding x86-64 Linux ABI PIE with
`-ftls-model=local-exec`. The object uses `R_X86_64_TPOFF32`; the linker
resolves these references into FS-relative accesses with no final relocations.
The initialized value is at TLS offset 0 (FS−8), and the zero-initialized
tail is at offset 4 (FS−4). The build verifies those offsets and rejects
the BSS fixture unless `p_filesz=4 < p_memsz=8`. Equal sizes would never
exercise loading a zero-initialized tail. In this build, the segment has
`p_offset=0x20f8`, `p_vaddr=0x30f8`, and `p_align=4`.

The loader validates size, alignment, and containment in one application
load segment, including the file-backed range for the initialization image.
The existing mapper clears the fresh page before copying the four bytes
`11 00 00 00`. The four tail bytes stay zero; bytes at that position in
the ELF file are not part of the TLS initialization image. The interpreter
verifies 17 before entry. The application verifies 17 → 29 → 42 and
independently observes the tail at 0 → 53 → 67. A successful marker and
exit status 0 require all those checks.

The contract excludes initial-exec, local-dynamic and general-dynamic TLS,
shared-library TLS, dynamic TLS relocations, thread creation, live exec
into the TLS fixture, late module loading, `dlopen`, constructors,
destructors, libc, and C++ runtime integration.

```sh
bash tools/helios-dynamic-elf/build.sh tls milestone
python3 tools/helios-dynamic-elf/replay.py tcg --tls milestone
python3 tools/helios-dynamic-elf/replay.py tcg --tls milestone --replay
python3 tools/helios-dynamic-elf/replay.py kvm --tls milestone
```

Evidence is in `target/helios-dynamic-elf-tls-bss/milestone/`.
`build.log` and `toolchain.txt` record the commands and tool versions;
`tls.json` records segment metadata, symbol offsets and both hashes.
The disassembly and `readelf` files show the relocations and linked layout.
Each accelerator directory has the guest command and ISO hash, the serial
trace, host test logs and the validated `result.json`.

| Two-vCPU boot | Controls | Fresh BSS children | Parent exits before collection | TLS rollback | Graph rollback | Allocator / host ELF |
| --- | --- | ---: | --- | ---: | ---: | ---: |
| Initial TCG | PASS | 7/7 | PASS | 10/10 | 16/16 | 2/2 · 14/14 |
| Same-image TCG replay | PASS | 7/7 | PASS | 10/10 | 16/16 | 2/2 · 14/14 |
| KVM | PASS | 7/7 | PASS | 10/10 | 16/16 | 2/2 · 14/14 |

Controls include static non-PIE, handoff-only, relative relocation, shared
and one-library probes, the two-library graph, and initialized-only TLS.
The seven BSS children comprise three sequential waits, one child whose
parent exits without collecting it, and three replacement waits. The
trace ties setup, output, exit and reclamation to guest process identities
and verifies fresh state across slot reuse. Rollback retains the original
nine cases and adds `p_filesz == p_memsz` for the required-tail fixture.
Stack and interpreter faults exercise rollback after TLS initialization.
The TLS path uses borrowed embedded image bytes and acquires no shared
image references.

Same numeric PID recycling is **not exercisable under current allocator
contract**. Wayland and transport were not rerun.
