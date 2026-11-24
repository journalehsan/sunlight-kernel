# Helios dynamic ELF handoff

The `helios_dynamic_elf_handoff` debug build admits one interpreter identity:
`/lib/helios/ld-handoff.so.1`. This path is a token for a read-only resource
embedded in the kernel at build time. It never selects a guest VFS file or
opens a host file at runtime. An absent resource, any other path, duplicate
or malformed PT_INTERP, and nested PT_INTERP are rejected. Application and
interpreter require marked Linux x86-64 ELF64, little endian, ABI version 0.
The interpreter must be ET_DYN and pass the shared PT_LOAD validation.
The feature leaves the normal static loader policy and live exec preflight
in place.

| Stage | Source |
| --- | --- |
| Manifest and argv[0] | `sunlight-libc/src/sun_exec.rs`: `launch_bundle` and `spawn_environment` |
| Executable bytes, PID, unpublished task | `kernel/src/arch/x86_64/syscall.rs`: `sys_spawn`; `kernel/src/process/mod.rs`: `Process::try_new` |
| ELF type, PT_INTERP and PT_LOAD | `sunlight-elf/src/lib.rs`: `parse_elf_header`, `DynamicImage::parse`, `plan_image` |
| Address layout | `kernel/src/process/layout.rs` and `kernel/src/process/elf_loader.rs`: `handoff::load`; ET_EXEC keeps its addresses, ET_DYN app bias is 0x20000000, interpreter bias is 0x80000000, and the interpreter window ends at 0x81000000 |
| Shared mapping, permissions, local rollback | `kernel/src/process/elf_loader.rs`: `map_image`, `reserve_final_regions`, `map_segment`, `rollback_elf_pages`; PTE and ledger protections enforce W^X |
| Frame and page-table allocation | `kernel/src/memory/pmm.rs`: `alloc_frame_owned`; `kernel/src/process/address_space.rs`: `try_new`, `map_page`, `create_next_table`, `rollback_created_tables` |
| Stack, argv/envp and auxv | `kernel/src/process/spawn.rs`: `build_exec_image`, `map_user_stack`, `setup_exec_stack` |
| Handoff and scheduling | `kernel/src/process/mod.rs`: `init_context`, `set_initial_args`; `sys_spawn` publishes with `kernel/src/sched/mod.rs::add_process_after_reaping` only after setup |
| Transaction rollback | `kernel/src/process/spawn.rs`: `exec_into_process_inner`; `kernel/src/process/address_space.rs`: `reclaim_user_space` retires the failed VM. The caller drops the unpublished process; no scheduler slot is linked. |
| Wait and teardown | `kernel/src/arch/x86_64/syscall.rs`: `sys_waitpid`; `kernel/src/sched/mod.rs`: `reap_finished_processes`, `reap_process_resources` |

The application and interpreter share one new VM. Both are planned before
mapping. Any failure before publication destroys that VM, including the prior
successful image and the stack. Interpreter bytes are borrowed kernel data;
the application buffer is borrowed for loading and dropped by the spawn
caller. No interpreter file handle, refcount, or file-backed mapping exists.
The debug boot test injects nine failures into the unpublished exec transaction.
It checks a cleared region ledger, unchanged free, page-table and PID-owned
frame counts, and no scheduler slot for the candidate PID. It does not
simulate file reference lifetimes, since this loader takes no retained
references. The missing interpreter case replaces the resource with absence;
the machine and malformed cases feed wrong-machine and truncated ELF bytes.

Initial RSP is 16-byte aligned and points to argc, followed by argv, a NULL
pointer, envp, a NULL pointer, then auxv. The interpreter starts at its biased
ELF entry. Auxv supplies:

- `AT_PHDR`, `AT_PHENT`, `AT_PHNUM`: the application's mapped program headers,
  record size, and count. The complete table must fit in the file-backed
  part of a load segment.
- `AT_ENTRY`: the application's entry after applying its own bias.
- `AT_BASE`: the interpreter's load bias.
- `AT_PAGESZ`, `AT_UID`, `AT_EUID`, `AT_GID`, `AT_EGID`, `AT_SECURE`:
  4096-byte pages, process IDs, and the existing zero secure flag.
- `AT_EXECFN`, `AT_RANDOM`, `AT_NULL`: argv[0], 16 bytes of kernel entropy,
  and the terminating zero pair.

All other entries are omitted, including vDSO/syscall entry pointers,
hardware capability bits, platform strings, clock ticks, and signal-stack
size. The fixture checks the metadata in userspace and exits at the
handoff marker. Both application entries trap if accidentally called. The
interpreter has no relocations, TLS, or dependencies. A general interpreter's
next unimplemented stage is relocation (starting with its own image where
needed); no attempt was made to start musl, load a dependency, or initialize
dynamic TLS. A verified handoff has no first failure.

Run the matrix with:

```sh
bash tools/helios-dynamic-elf/build.sh handoff final
python3 tools/helios-dynamic-elf/replay.py tcg --handoff final
python3 tools/helios-dynamic-elf/replay.py tcg --handoff final --replay
python3 tools/helios-dynamic-elf/replay.py kvm --handoff final
```

The build and per-run receipts are under
`target/helios-dynamic-elf-handoff/final/`. Replay checks the ISO digest
against the initial TCG run. Each result records handoff metadata, waitpid,
and the scheduler's reclamation receipt for the same PID.
