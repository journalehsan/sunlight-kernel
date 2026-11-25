# Helios minimal C startup fixture

The debug-only fixture is a locally compiled, stamped x86-64 static ELF at
`/bin/helios-c-startup`. Its assembly `_start` calls a freestanding C function in
the initial thread. The linker uses `-nostdlib` and supplies no interpreter, TLS
segment, constructors or destructors. The fixture exercises the existing Helios
compatibility dispatch. The numbers below come from that dispatch and its mmap
translation, as documented in [ANON-MMAP.md](ANON-MMAP.md).

## Entry contract

The launcher supplies two arguments, `"c-startup"` and a one-character mode
(`P`, `N`, `A`, `W` or `L`), with a replacement empty environment. At the
first instruction, RSP points at argc and is 16-byte aligned. All table entries
are little-endian 64-bit words.

| Offset from initial RSP | Content |
| ---: | --- |
| 0 | argc = 2 |
| 8, 16 | Guest pointers for argv[0] and argv[1] |
| 24 | argv[2] = NULL |
| 32 | envp[0] = NULL |
| 40–247 | Thirteen auxv tag/value pairs, in the order below |
| 248–259 | Padding with unspecified contents |
| 260–261 | Mode character and terminating NUL |
| 262–271 | `"c-startup\0"` |
| 272–287 | Sixteen guest-owned bytes referenced by AT_RANDOM |

The stack spans 288 bytes from the entry RSP to the exclusive Helios stack top
`0x00007fff_ffff_f000`. RDI holds 2, RSI points at RSP+8, and RDX points at
RSP+32. RIP points at `_start`. The loader initializes the other general-purpose
registers to zero and RFLAGS to `0x202` (IF set, DF clear). The fixture checks
the alignment and three argument registers before using the data. Assembly
passes the original RSP to C, whose function entry follows the x86-64 SysV call
alignment after the return address is pushed. The initial FS base is zero for
this static, TLS-free image. The fixture uses neither FS nor GS as a thread
pointer and requires no TLS or TCB initialization.

| Auxv tag | Value |
| --- | --- |
| 3 AT_PHDR | Guest address of the mapped ELF program header table |
| 4 AT_PHENT | 56 bytes |
| 5 AT_PHNUM | Program header count from the mapped ELF header |
| 6 AT_PAGESZ | 4096 |
| 9 AT_ENTRY | Guest address of `_start` |
| 11 AT_UID, 12 AT_EUID | Guest UID, repeated |
| 13 AT_GID, 14 AT_EGID | Guest GID, repeated |
| 23 AT_SECURE | 0 |
| 31 AT_EXECFN | Guest pointer equal to argv[0] |
| 25 AT_RANDOM | Guest pointer to the final 16 stack bytes. Contents are unspecified to the fixture and omitted from evidence. |
| 0 AT_NULL | 0, ends the table |

The loader copies the pointer table, strings and random bytes into the
child's user stack frames. Their lifetime is the loaded image's address space.
The kernel's fixture-only preflight reads the copied bytes through bounded
guest-memory access before initializing an entry context. The C parser
independently bounds its reads to the copied stack, checks all terminators and
resolves encoded addresses as offsets from RSP. The live probe writes and
restores the mode string and checks the other string stays intact. The host
test reuses the parser on a snapshot after clearing its source storage.

## Guest operations

`syscall` takes the identifier in RAX and the arguments in RDI, RSI, RDX, R10,
R8 and R9. RAX carries the signed result and RCX/R11 are clobbered. The
fixture zeroes unused registers in its C wrapper. The assembly exit path
supplies only its status argument.

| Identifier | Argument order and rules | Result, ownership and invalid arguments |
| --- | --- | --- |
| 1, write → 43 | fd, readable guest buffer, byte count. The fixture uses fd 1 and UTF-8 markers of at most 256 bytes on the serial-backed descriptor. | Returns bytes consumed, potentially short on that descriptor. The fixture requires a full write and retains ownership of the buffer. An unmapped nonzero buffer with positive length yields `-14` in the exercised path. Zero length yields 0. Bad fd and insufficient rights yield `-9` and `-13`. The serial handler rejects a null pointer with all ones. |
| 39, getpid → 33 | No arguments or pointers. | Returns the positive guest process ID for the shim's owner field. There is no invalid-argument path. |
| 9, mmap → 1010 → 50 | Address 0, nonzero byte length, RW protection 3, private anonymous flags 0x22, descriptor -1, offset 0. The shim supplies a whole-page length. | Positive aligned guest base, or negative errno. Invalid length or anonymous descriptor yields `-22`. Resource exhaustion yields `-12`. The process owns the guest mapping. |
| 11, munmap → 51 | Page-aligned guest base and backing byte length. The shim submits its entire live mapping. | Zero on success, otherwise negative errno. Misalignment or range overflow yields `-22`. The process loses the mapping on success. The shim invalidates the handle only after successful unmap. |
| 60, exit → 20 | Low 32 bits of RDI carry a signed status. No pointers or setup syscall. | Terminates the sole thread, publishing its exit status for the parent. It never returns and has no invalid-argument check for the status. Assembly traps on an unexpected return. The teardown path owns reclamation. |

There are no guest calls to mprotect, exit_group or waitpid. The native
launcher observes the raw status with its existing waitpid and checks repeated
collection. The broader matrix retains mprotect and the existing waitpid
controls.

## Bounded heap and failure paths

The fixture uses the explicit checked handle API in
[BOUNDED-MALLOC.md](BOUNDED-MALLOC.md). It starts with a zeroed per-process heap
and sets the owner returned by getpid. A 37-byte request takes one 4 KiB page.
The probe verifies alignment, checks every backing byte is zero and
reads back markers at requested offsets 0 and 36. Explicit free empties the
heap and both resolve and a second free reject the stale handle. The success
marker is `HELIOS-C-STARTUP-OK` and the expected status is 0.

| Mode or fault | Result |
| --- | --- |
| `P` | Explicit free, verified marker, status 0. |
| `N` | The C parser rejects eleven individually mutated snapshots, including bad argc, pointers, string and table terminators, auxv value, alignment, overflow and truncation. Status 42 confirms the negative suite ran. |
| `A` | A deterministic backend failure returns `-12` during heap initialization. No handle, pages or generation is published. Status 43. |
| `W` | Write uses the unmapped guest stack-top address, returns `-14`, and exits with one live allocation and status 44. |
| `L` | Exits with one touched live page and status 0. The normal parent collects one such child. The orphan control exits its parent before collection of another such child. |
| Kernel pre-entry faults | With this same ELF, the rollback harness injects argc, argv terminator, unterminated string, envp terminator, auxv terminator and alignment errors into the prepared stack. It also forces stack-space exhaustion during construction and injects a failure after successful preflight. |

All kernel pre-entry failures leave entry RIP and process argument metadata
unpublished. The harness verifies zero regions and unchanged free-frame,
page-table, owner-frame and scheduler counts. Exited guest processes reclaim
all regions, owned frames, page-table roots and kernel stacks. The observer
cannot collect the orphan's status before or after slot reuse. Three
replacement children each verify fresh zeroed pages and an initial shim
generation of one. Exit needs no fallible setup operation.

The fixture provides a startup contract for this one image. Full libc, the
libc malloc ABI, glibc and `__libc_start_main`, pthreads, thread creation,
signals, futexes, C++ ABI, exceptions, dynamic loading, locale and filesystem
operations are outside its scope.

## Reproduce the matrix

```sh
export CARGO_NET_OFFLINE=true GIT_ALLOW_PROTOCOL=file
bash tools/helios-dynamic-elf/build.sh c-startup matrix
python3 tools/helios-dynamic-elf/replay.py tcg --c-startup matrix
python3 tools/helios-dynamic-elf/replay.py tcg --c-startup matrix --replay
python3 tools/helios-dynamic-elf/replay.py kvm --c-startup matrix
```

Evidence is written to `target/helios-runtime-c-startup/matrix/`. The runner
verifies the locally generated artifact hashes, boots with two vCPUs and no
guest NIC, and verifies the ISO SHA-256 on replay. Each boot records the guest
entry and syscall markers, argument values, wait results, reclamation and host
suite counts. The host startup suite checks the C parser and runs the assembly
entry with a test exit capture in place of `syscall`.

| Run | Guest startup and regression controls | Kernel | ELF | PID allocator | Page allocator | Malloc shim | C startup |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Initial two-vCPU TCG | PASS | 212/212 | 14/14 | 2/2 | 7/7 | 8/8 | 16/16 |
| Same-image two-vCPU TCG replay | PASS | 212/212 | 14/14 | 2/2 | 7/7 | 8/8 | 16/16 |
| KVM | NOT RUN — /dev/kvm unavailable | Unrun | Unrun | Unrun | Unrun | Unrun | Unrun |

Both boots passed the eight pre-entry rollback cases, eleven guest parser
negatives and all startup heap modes. The regression gate also passed ELF,
application and DSO TLS, direct TLS resolver rejection, TLS and graph
rollback, mmap, page allocation, bounded malloc, lifecycle and waitpid.

- ISO SHA-256 `71ad02f7ff9c1d54e3db2bc507d431faeedce6835521a74d1d7822e9ac4b3865`
- Stamped C startup ELF SHA-256 `5aff44f41f833c22018719d57f4535ce27c50d9c17494a9a3ff401a837427576`

Same numeric PID recycling remains **not exercisable under the current
allocator contract**. Wayland and transport were not rerun. Qt and Qalculate!
remain unlaunched.
