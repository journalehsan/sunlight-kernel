# Helios and native C/C++ runtime linkage audit

Helios is the kernel's Linux syscall personality. It does not provide a Linux
`libc.so` or substitute `sunlight-libc` for musl. The supported static Linux
programs carry their own runtime. Native SunlightOS programs use separately
linked libraries: `sunlight-libc` and `sunlight-cxx` are not daemons.

| Program | Runtime | How it reaches the OS |
| --- | --- | --- |
| Helios Linux executable | Runtime linked into the executable; the bundled `helios-note` uses musl | Linux syscalls enter the kernel's Helios dispatcher |
| Native C executable | `libsunlight_native_c.a`, built with the `sunlight-libc` native C feature | Native syscalls and capability IPC |
| Native C++ executable | `libsunlight_cxx.a`, `libsunlight_eh.a`, and the native C archive | C++ runtime uses native C and its OS interfaces |

## Linux startup and syscall dispatch

The [ELF classifier](../../sunlight-elf/src/lib.rs#L110) selects the Linux ABI
from the Linux OSABI value or a Helios marker. An unmarked OSABI-zero image is
classified as native. The [spawn code](../../kernel/src/process/spawn.rs#L348)
builds the Linux auxiliary vector, including the program header information
that lets musl find `PT_TLS` and initialize thread-local storage.

The [syscall dispatcher](../../kernel/src/arch/x86_64/syscall.rs#L614) checks the
process personality and translates Linux syscall numbers, routing some calls
to Linux-specific handlers. It also supports an explicitly tagged, allowlisted
native ABI bridge for programs that deliberately use Sunlight IPC or shared
memory. The bridge preserves native permission checks and does not replace
the executable's libc.

## Native C and C++

The [C driver](../../tools/native-c/cc.py) supplies the native archive and
linker script and does not search host system libraries. Native C functions
may use [native syscall wrappers](../../sunlight-libc/src/sys.rs), or they may
call other processes: [cryptographic randomness](../../sunlight-libc/src/rand.rs)
uses the `rand` service and [network socket operations](../../sunlight-libc/src/net_bridge.rs)
use IPC. By comparison, [Helios getrandom](../../kernel/src/arch/x86_64/syscall.rs#L7966)
calls kernel entropy directly.

The [C++ driver](../../tools/native-c/cxx.py) adds the C++ and exception archives
and [wraps main](../../sunlight-cxx/src/entry.cpp) for runtime lifecycle handling.
For example, [std::cout ultimately calls native C write](../../sunlight-cxx/src/ostream.cpp).
The [Phase 22 contract](../../sunlight-cxx/README.md#phase-22-executable-native-runtime-bounded-acceptance-complete-for-this-subset)
covers a bounded string, stream, thread, and lifecycle implementation, with
target-built LLVM exception, RTTI, and unwind support. This is not the full
libc++ or libstdc++ standard library.

## Loader limitations

The [Helios strategy](Helios_ABI_Translation_and_Library_Shimming_Strategy_v2.md)
discusses dynamic musl and optional library substitution. The implementation
[requires ET_EXEC](../../sunlight-elf/src/lib.rs#L129) and
[rejects PT_INTERP](../../sunlight-elf/src/lib.rs#L212). Bundling `libc.so`,
`ld-musl`, or `ld-linux` therefore does not enable ordinary dynamically linked
Linux executables. Native [dlopen](../../sunlight-libc/src/dlfcn.rs) only exposes
a fixed export table in the main executable; it cannot load shared objects.
Static glibc is a separate compatibility question: acceptance by the loader is
not evidence that the runtime works.

## Evidence and next check

Source inspection and `readelf` inspection of the existing `helios-note`
artifact found an `ET_EXEC` executable with a TLS segment and libc startup
symbols, with no `PT_INTERP` or dynamic section. This matches the
[static musl build recipe](../../tools/build.sh#L126). No fresh build or QEMU
guest run was performed for this audit.

A follow-up guest check should run a Helios musl executable and a native C++
executable against equivalent file I/O, TLS, and thread-exit checks, recording
which ABI and backend each reaches.
