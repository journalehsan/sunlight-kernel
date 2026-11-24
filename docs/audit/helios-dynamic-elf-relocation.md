# Helios relative relocation probe

The fixture is an x86-64 ET_DYN PIE with the trusted embedded interpreter.
GNU ld emits one R_X86_64_RELATIVE at image offset 0x4000, with addend 0x2000.
The application compares the relocated pointer with its independently computed
RIP-relative address and uses the pointer to print its entry marker.

PT_DYNAMIC is at file offset 0x2f00 and image address 0x3f00. DT_RELA is
0x350, DT_RELASZ and DT_RELAENT are both 24, and DT_RELACOUNT is 1.
DT_SYMTAB is 0x330, DT_STRTAB is 0x348, and both contain only their null entry.
DT_FLAGS_1 contains PIE. There are no DT_REL/DT_RELSZ/DT_RELENT, DT_NEEDED,
DT_PLTREL, DT_PLTGOT, DT_FLAGS or PT_TLS entries. The fixture needs no
symbol resolution.

The debug kernel validates and writes relocations after mapping both images
and before publication. This puts relocation errors inside the existing exec
rollback path; parsing and writes are in the kernel, since the current
userspace interpreter cannot run before publication. The interpreter checks
the initial stack and jumps to AT_ENTRY with the original stack pointer.
The fixture has a separate packaging feature; loader code uses the existing
debug handoff gate.

Results under `target/helios-dynamic-elf-relocation/verified/`:

| Run (2 vCPUs) | Entry marker / exit | Wait | Reclaimed frames / tables |
| --- | --- | --- | --- |
| TCG | yes / 0 | Err(Failed) | 521 / 10 |
| TCG replay | yes / 0 | Ok(0) | 521 / 10 |
| KVM | yes / 0 | Ok(0) | 521 / 10 |

All runs used ISO SHA-256
`963548c900808579988273d13a0e51a6967975803073e600569dddc236363532`.
Both handoff controls, the unchanged static probe, nine original rollback cases,
and two new rollback cases passed on all three runs. In the failed TCG run,
the child exited 0 and was reaped; its process slot was reused before the parent
reported the wait error. The first failure was waitpid. The scheduler and wait
contract were left unchanged.

Host tests: 14 passed, including explicit malformed-table, range, permission,
bias and relocation-type failures plus a failure after a prior relocation
write. The real rollback after an injected post-relocation stack allocation
failure was tested in the guest. Process allocation itself happens before
relocation; there is no host test for process allocation after relocation.
Transport and Wayland tests were not rerun.
