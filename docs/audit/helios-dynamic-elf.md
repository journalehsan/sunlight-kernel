# Helios dynamic ELF diagnostic

This milestone measures the existing policy with three tiny x86-64 Linux
executables. It does not enable dynamic execution. The fixture has a separate
debug build feature and uses the normal SunApp GUI manifest and
`sun_exec::launch` entry point. No Qt or Wayland fixture is needed.

## Execution path

- `sunlight-libc/src/sun_exec.rs` checks the GUI bundle manifest, registry,
  executable path and Helios marker, then submits a normal spawn request
  with the executable as argv[0]. The kernel's spawn path in
  `kernel/src/arch/x86_64/syscall.rs` resolves the executable bytes, reserves
  a PID and constructs an unpublished child. It copies arguments and builds
  envp from the child's inherited environment before calling
  `exec_into_process`.
- `kernel/src/process/spawn.rs` selects the personality and constructs the
  new image. `kernel/src/process/elf_loader.rs` first calls
  `sunlight_elf::parse_elf_header`: ELF magic and ELF64 precede the
  `e_type != 2` check that returns `NotStaticExecutable`; machine,
  program-header bounds and nonzero entry checks follow. ET_EXEC is the
  only accepted type; ET_DYN is rejected even without an interpreter.
  `for_each_load_segment` rejects PT_INTERP as `HasDynamicSection`.
  Non-LOAD headers are otherwise skipped by that walker, so this error
  does not mean the loader parsed a dynamic section.
- The loader validates and collects a bounded PT_LOAD plan before modifying
  page tables, checks page collisions, reserves regions, allocates and
  initializes pages (including zero fill), and checks executable entry
  coverage. It takes virtual addresses directly from the image. The ELF
  window starts at 0x1000 and ends at the 4 GiB heap boundary. Mappings
  enforce W^X, including shared boundary pages. The stack has its own
  region near the top of user space.
- The initial stack contains argc, argv, envp and auxv. The Linux auxv has
  program headers, page size, application entry, IDs, secure flag,
  EXECFN, RANDOM and NULL. There is no interpreter base or independent
  interpreter entry. The image starts directly at the application entry;
  the static loader has no symbol resolution, relocation, or dynamic
  metadata parser.
- The syscall bridge provides brk, anonymous mmap, mprotect, munmap,
  arch_prctl/SET_FS, set_tid_address, write and exit_group. The static musl
  runtime initializes its thread pointer itself; the kernel does not
  build a dynamic TLS layout. Neither probe executable nor the inspected
  interpreter has PT_TLS, which does not eliminate musl's initial thread
  pointer requirement. General private file-backed mmap is unavailable:
  `kernel/src/process/mmap.rs::sys_mmap` rejects a nonnegative fd or nonzero
  offset outside the limited shared special-file path. This matters when
  adding an ordinary secondary DSO; this probe's sole SONAME, libc.so,
  is supplied by musl's interpreter image itself. Dynamic startup syscall
  behavior remains untested.
- The child is published only after image setup. Load failures reclaim
  the unpublished address space. The native waitpid identifies a direct
  child, parks the caller if needed, and returns its exit status; scheduler
  reaping handles address-space and process resource cleanup separately.
  The replay checks both wait status and the per-PID cleanup receipt.

## Probe and decision

Run from the repository root:

```sh
bash tools/helios-dynamic-elf/build.sh
python3 tools/helios-dynamic-elf/replay.py tcg
python3 tools/helios-dynamic-elf/replay.py kvm
```

The probe calls `write` and exits zero only on a full write. The dynamic
variants have four LOAD segments, one PT_INTERP naming
`/lib/ld-musl-x86_64.so.1`, and one NEEDED entry, `libc.so`. Both import
`write` through a JUMP_SLOT relocation. The static ET_EXEC has no
interpreter or dynamic section. The installed musl-gcc wrapper emitted a
PT_INTERP binary for `-static-pie`, so it was not used as a static PIE
control. The unmodified toolchain interpreter is inspected and hashed on
the host; it is not installed in the diagnostic guest or executed manually.

Expected first failures are ELF type validation for the PIE and PT_INTERP
policy for the dynamic ET_EXEC. After those gates, the first missing stage
is a trusted interpreter lookup and an independently based, validated
second image mapping. That would need an atomic two-image rollback and a
Linux stack handoff that preserves the application's PHDR and ENTRY
while starting the interpreter with AT_BASE. The selected musl image also
contains RELR and GLOB_DAT records for its own startup. The evidence does
not justify admitting either executable without implementing and proving
that broader handoff. No relocation or dynamic TLS behavior has been
verified in Helios.

Each accelerator directory under `target/helios-dynamic-elf/` contains the
replay command, serial log, classification, build log, manifests, artifact
hashes and ELF inspection reports. A dynamic rejection is a diagnostic
result; it does not satisfy the dynamic execution milestone.

Observed on both two-vCPU TCG and KVM: PIE reached spawn and returned
`NotStaticExecutable`; dynamic ET_EXEC reached spawn and returned
`HasDynamicSection`. Neither published a child. The static ET_EXEC reached
spawn, printed the expected line, exited zero and returned `Ok(0)` from
waitpid. The scheduler logged 517 user frames and eight page-table frames
reclaimed for the child on each replay.
