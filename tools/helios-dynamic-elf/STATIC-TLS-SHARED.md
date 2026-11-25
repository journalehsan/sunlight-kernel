# Shared TLS fixture inspection

The freestanding GCC PIC build of `tls-shared-value.c` uses local-dynamic
TLS. The debug fixture adds a fixed startup ABI for this image.

| ELF | Model | PT_TLS file / memory / alignment (bytes) | Initialization bytes | TLS symbol offsets |
| --- | --- | --- | --- | --- |
| Application | Local-exec | 4 / 4 / 4 | 17 00 00 00 | application: 0 |
| libhelios_tls.so | Local-dynamic | 4 / 8 / 4 | 31 00 00 00 | initialized: 0, tail: 4 |

The application's only dependency is the fixture DSO. It calls
`helios_tls_value` through the PLT and checks the initial values and writes
across two calls; an optional digit argument varies the writes. The DSO has
no DT_NEEDED entries, constructors, or destructors. Neither image has
TLS-specific dynamic tags.

The application object uses R_X86_64_TPOFF32, resolved to FS:-4. The DSO
object uses R_X86_64_TLSLD and R_X86_64_DTPOFF32. The linked DSO has
R_X86_64_DTPMOD64 at 0x31e8 and R_X86_64_JUMP_SLOT for the undefined
`__tls_get_addr` at 0x3210. There is no linked DTPOFF64 relocation. The
local-dynamic call passes a pointer in RDI to two consecutive little-endian
64-bit words: module ID and module-relative offset. The offset word is zero.
The resolver returns a guest block address in RAX, and the DSO accesses the
two symbols at displacements 0 and 4. The DSO itself does not read FS.

The loader verifies the application against the embedded application and
compares the library SHA-256 against the embedded trusted library. It fixes
module IDs to application=1 and DSO=2. Module 1 still uses FS:-4. FS:0 holds
the guest address of a read-only resolver descriptor, which identifies the
one compiler-emitted index and the read-only two-row module table. Each row
records the verified image digest, the template file range, sizes, alignment,
the guest block base, and the fixture's symbol offsets. Module 2 accepts only
offset zero; the application needs no resolver call. All four pages belong
to the process address space and are reclaimed by the existing VM teardown.
There is no general dynamic TLS, thread creation, TLS destructor, or late
module loading.

The emitted dynamic tags for TLS handling are the normal RELA and PLT
relocation tags. The trusted path accepts precisely one symbol-zero
DTPMOD64 with a zero addend and a writable 16-byte index initialized to
zero, plus the resolver JUMP_SLOT. The DSO has no dependency; no initial-exec,
general-dynamic, or TLSDESC sequence was emitted.

To build and validate the same image:

```sh
bash tools/helios-dynamic-elf/build.sh tls RUN
python3 tools/helios-dynamic-elf/replay.py tcg --tls-shared RUN
python3 tools/helios-dynamic-elf/replay.py tcg --tls-shared RUN --replay
python3 tools/helios-dynamic-elf/replay.py kvm --tls-shared RUN
```

The observed run is under
`target/helios-dynamic-elf-tls-shared/inspection-rm1TzW/`:
`compiler-linker.txt` records commands, `elf.txt` and `disassembly.txt`
record the ABI, and `SHA256SUMS` records the ELF and TLS image hashes.
That inspection is historical; execution logs, host tests and the result
files belong to their respective execution run directories. Wayland and
transport were not rerun. Same numeric PID recycling is not exercisable under
current allocator contract.
