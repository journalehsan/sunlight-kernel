# Helios minimal C utility DSO

The debug-only C startup matrix includes a direct startup control
and a PIE built from the same freestanding `_start` and startup parser. The PIE
starts through the embedded handoff interpreter, then validates argc, argv,
envp, auxv (including AT_BASE), argument registers and stack alignment. It
allocates one 37-byte buffer with the bounded malloc shim, verifies the zeroed
backing page, exercises every utility export, writes `HELIOS-C-DSO-OK`, frees
the allocation and verifies stale-handle rejection. Its success path exits
through the DSO with status 0. The launcher collects two independent children
and verifies a second wait fails for each.

## Call ABI

Calls use x86-64 SysV. The caller aligns RSP to 16 bytes before CALL. All
addresses and lengths are unsigned 64-bit values. The two-word allocation view
is passed by value in RDI (base) and RSI (requested length). Byte data can be
unaligned. The malloc shim provides a 4096-byte-aligned base. Results are
signed 64-bit values in RAX unless the function terminates the process.

| Export | Arguments after the allocation view | Result |
| --- | --- | --- |
| `helios_memset` | RDX destination, low 32 bits of RCX byte value, R8 length | 0 after filling exactly the requested range with the low eight bits. |
| `helios_memcpy` | RDX destination, RCX source, R8 length | 0 after copying exactly the requested bytes. Any nonempty overlap, including identical ranges, returns -22 before writing. |
| `helios_strlen` | RDX source, RCX scan limit | Bytes before the first NUL or -61 if the validated range contains no NUL. |
| `helios_write` | RDX source, RCX length, R8 descriptor (kernel uses low 32 bits) | Existing guest write syscall 1 result, including short writes and kernel errors. The probe demands a full write to fd 1 and tests -9 with an invalid fd. |
| `helios_exit` | No view. Low 32 bits of RDI hold the status. | Existing guest exit syscall 60 terminates the child and publishes its status. An unexpected syscall return traps. |

The startup caller owns the allocation and resolves its live handle through the
shim before constructing the view. The DSO trusts that view as an internal
contract with this exact verified caller. Callers must keep the view live and
mapped for the call. The DSO validates each candidate range against the
requested bytes in the view before touching memory. The page-rounded portion
outside the requested bytes is excluded. Overflow of either exclusive end
returns -75. A nonempty out-of-view range returns -14. Both memcpy ranges must
pass before overlap is checked or any write occurs. Zero-length memory and
write calls return zero without inspecting either pointer or issuing a syscall.
A zero-limit strlen returns -61 without reading. For write, the kernel
independently validates guest memory for the serial output path. The memory
routines operate directly on guest memory within the trusted view.

## Identity and rollback

GCC builds `libhelios-util.so` as an x86-64 ET_DYN with a matching SONAME.
`build.log` records the compiler and linker invocations, and
`c-utility.readelf` records the PT_LOAD layout and the symbol, dynamic tag,
section, relocation and version inventory. The DSO has no PT_TLS,
initialization image, imports, DT_NEEDED entries, relocations, symbol versions,
constructors or destructors. The PIE has one DT_NEEDED and five zero-addend
R_X86_64_JUMP_SLOT relocations for the five allowed imports. The build rejects
unsupported library metadata before embedding. The runner verifies local
artifact SHA-256s, including the application, DSO and ISO, before boot. The
kernel matches the executable against the exact embedded app and compares the
embedded library resource SHA-256 to its trusted build input before mapping.
The normal interpreter and loader path still handles the existing controls.

Utility binding validates the full relocation set before writing any slot.
Fault injection covers an unsupported import, unexpected dependency,
unsupported relocation, missing export and failures after mapping, after
resolution, and during child stack setup after all slots were written. The
last stage occurs inside the staged address space before process publication.
Failures discard the staged address space and leave the original process state
intact. The rollback harness checks empty regions, unchanged frame, page-table
and scheduler counts, zero owned frames and FS base, and an unpublished entry
and argument block. The utility has no TLS or module metadata. Successful
children demonstrate fresh allocations and independent VM generations with
complete mapping, frame, page-table and stack reclamation and correct waitpid.

This contract covers the exact fixture and utility above. It supplies no libc
startup ABI, glibc, `__libc_start_main`, errno storage, locale, filesystem,
pthreads, signals, futexes, constructors, destructors, dlopen, dlsym, C++,
exceptions or unwinding.

## Regression matrix

```sh
export CARGO_NET_OFFLINE=true GIT_ALLOW_PROTOCOL=file
bash tools/helios-dynamic-elf/build.sh c-dso matrix
python3 tools/helios-dynamic-elf/replay.py tcg --c-dso matrix
python3 tools/helios-dynamic-elf/replay.py tcg --c-dso matrix --replay
python3 tools/helios-dynamic-elf/replay.py kvm --c-dso matrix
```

Evidence is in `target/helios-runtime-c-dso/matrix/`, with separate `tcg/`,
`tcg-replay/` and `kvm/` results. The matrix boots two vCPUs with no guest NIC
and verifies ISO identity on replay. It retains direct C startup, TLS,
resolver rejection, ELF, graph rollback, waitpid, mmap, page allocator,
bounded malloc and parent-before-collection controls.

| Run | Guest matrix | Kernel | ELF | PID allocator | Page allocator | Malloc shim | C startup |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Initial two-vCPU TCG | PASS | 212/212 | 14/14 | 2/2 | 7/7 | 8/8 | 16/16 |
| Same-image two-vCPU TCG replay | PASS | 212/212 | 14/14 | 2/2 | 7/7 | 8/8 | 16/16 |
| KVM | NOT RUN — /dev/kvm unavailable | | | | | | |

- ISO SHA-256 `09e41ac3b0485c375a22face121e8573d2d333527e8a306cd521f8b8d0b43bde`
- DSO SHA-256 `a88118c667e249b96167dc14302a08ef2245e29e0d189b2b07a6d4448d4321a4`
- Stamped PIE SHA-256 `abbf4073567cafc96b2b26ab07c5b02888bf779cd19e7f72c858eaab2c35e216`

Both TCG boots passed all seven utility loader rollback cases and both
utility child attempts. Each successful child reclaimed 525 frames and 11
page tables.

Same numeric PID recycling remains unavailable under the current allocator contract. Wayland and
transport were not rerun. Qt and Qalculate! remain unlaunched.
