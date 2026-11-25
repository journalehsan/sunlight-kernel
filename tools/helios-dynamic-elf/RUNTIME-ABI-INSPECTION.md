# Helios runtime ABI: pinned Qalculate! Qt closure

Scope: the existing Qalculate! Qt 5.13.0 application build, staged
libqalculate 5.13.1, Qt 6.12.0 and glibc 2.44 snapshot described in
[the packaging audit](../../docs/audit/helios-qalculate.md). Inspection
verified the staged bundle against its historical SHA-256 manifest and
resolved every ordered DT_NEEDED and PT_INTERP edge to a pinned file in
that bundle. No host library was substituted. No Qt or Qalculate! code
was executed, including on the host for inspection.

## ELF closure

The evidence is in `target/helios-runtime-abi-inspection/runtime-abi-02/`.
`artifact-manifest.json` records absolute paths, digests, program headers
and interpreter/provider identities. `dependency-graph.json` has the
ordered edges from the executable, two selected Wayland plugins and
libqalculate; `historical-loader-order.txt` retains the earlier host loader
listing for each root. The 81 ELF files are ELF64 x86-64 ET_DYN (one PIE);
all carry GNU hash and GNU RELRO, none has SysV hash, and 78 carry DT_RELR.
The executable requests `/lib64/ld-linux-x86-64.so.2`, also named by
libc itself. The historical Linux vDSO is synthesized by the host kernel,
not a staged file. This is the complete static ELF closure of the selected
roots. It does not bound runtime-loaded drivers, NSS modules, additional
plugins or data files; the exact runtime selection is unknown.

Per-file `*.elf.txt` captures ELF and section headers, dynamic tags, notes,
symbols, versions and relocations. Separate files list undefined symbols,
exported symbols and instruction sites. The closure uses RELA and packed
RELR, GLOB_DAT, absolute 64-bit and JUMP_SLOT fixups, TLS fixups and
IRELATIVE. No COPY or TLSDESC relocation was observed. Some images use
DT_JMPREL and DT_PLTGOT; handling only the fixture's single PLT binding
would leave other GOT entries unresolved. The interpreter would need to
map and order the closure, honor symbol visibility and versions, apply the
relocations and IFUNC resolution, enforce RELRO after relocation, build
TLS before consumers enter, and arrange initialization/finalization and
the libc startup handoff. This inventory does not determine whether any
optional lazy binding path must be supported.

`constructors-destructors.txt` records INIT/FINI and array tags;
`cxx-unwind.txt` records `.eh_frame`, `.gcc_except_table`, personality,
RTTI, exception and unwinder evidence. Qalculate! imports versioned
`__libc_start_main@GLIBC_2.34`, `__stack_chk_fail@GLIBC_2.4` and symbols
at GLIBC_2.38 and GLIBCXX_3.4.32. `symbol-versions.txt` and the per-file
reports cover all transitive version families, including private glibc
interfaces. The nine PT_TLS segments include nonzero tails and zero
filesz templates. DTPMOD64 plus resolver calls are consistent with
local-dynamic sequences; symbol-bearing DTPMOD64/DTPOFF64 pairs in
libicuuc, libmpfr and libstdc++ evidence general-dynamic linkage, and
TPOFF64 GOT entries in libc and other DSOs evidence initial-exec
linkage. Link relaxation and stripped code prevent recovering original
compiler flags or a complete local-exec inventory from the linked files.

## Runtime evidence

“Observed” means an import or an identifiable machine-code site in this
snapshot, not a syscall seen executing. “Inferred” refers to reachability
or semantics deduced from those sites. No startup syscall ordering or
option-dependent path was traced. `syscalls.txt` lists candidate imports
and `*.instruction-sites.txt` records nearby instructions for syscall and
TLS resolver call sites. Labels in stripped files may name the nearest
export rather than the enclosing function.

| Interface | Observation and runtime limit |
| --- | --- |
| mmap, munmap, mprotect | Observed imports, including Qt or supporting DSOs. Mapping, release and protection changes are inferred for a real runtime; exact flags and order are unknown. |
| brk | Observed in libc and interpreter disassembly. Allocator use is inferred; startup reachability is unknown. |
| futex | Observed syscall sites in the libc pthread implementation. Futex backed synchronization is inferred from pthread imports and implementation; contention is unknown. |
| clone, clone3 | Observed clone import and both syscall sites in libc. Thread creation is inferred from pthread_create imports; which syscall succeeds or is used as fallback is unknown. |
| arch_prctl, set_tid_address, set_robust_list | Observed syscall sites in the interpreter or libc. TLS and thread bookkeeping use is inferred; exact order is unknown. |
| rt_sigaction, rt_sigprocmask, rt_sigreturn | Observed signal API imports and corresponding libc syscall sites, including a restorer. Signal installation and delivery on a given application path are unknown. |
| clock_gettime | Observed import. vDSO versus syscall reachability is unknown. |
| openat, read, write, close | Observed imports, including openat64. File and descriptor operations are inferred; exact paths and error handling are unknown. |
| ioctl, poll, ppoll, epoll | Observed ioctl, poll, ppoll, epoll_create1, epoll_ctl and epoll_wait imports. Device requests and event-loop path are unknown. |
| getrandom, prlimit, sched_yield | Observed getrandom and sched_yield imports and prlimit syscall site in libc. Whether startup uses any given path is unknown. |
| exit, exit_group | Observed exit import and exit_group site in libc. The normal termination sequence is inferred. |
| Filesystem queries | Observed stat, statx, statfs and statvfs family imports. Mount/path requirements are unknown. |
| Locale and iconv | Observed setlocale and iconv family imports. Exact locale data and conversion modules are unknown. |
| Environment and auxv | Observed getenv and getauxval imports. The staged interpreter consumes Linux startup data; exact optional entries and fallback paths remain unknown. |

Helios syscall translation entries in `compat-linux/src/lib.rs` are
source evidence only. The guest gate here verifies raw write and exit
and the bounded startup stack; it does not establish a syscall
compatibility matrix. clone3 and prlimit64 translation entries were not
observed in that table, which by itself does not prove system-wide
absence. No other runtime syscall family in this table was tested here.

| libc/C++ boundary | Classification for the selected closure |
| --- | --- |
| Basic guest write/exit and parent wait | Already supported for the tested contract. |
| Initial environment/auxv, interpreter entry, static app TLS and one startup DSO | Supported only by the bounded debug fixture at the dynamic-ELF boundary. The real interpreter stack, libc TCB and environment services need a new ABI milestone. |
| glibc versions, __libc_start_main, stack protector and allocator | Requires a new ABI milestone. The versioned startup, stack-check, malloc and free imports are observed; native or static libc tests do not prove this boundary. |
| pthread, futex synchronization and thread creation | Requires a separate ABI milestone. pthread imports include pthread_create and synchronization; syscall site evidence is listed above. |
| libgcc_s, libstdc++, RTTI, exceptions and unwind | Requires a separate ABI milestone. DT_NEEDED, CXXABI/GLIBCXX/GCC versions, RTTI, throw, personality and unwind references are observed; actual throw/unwind reachability is unknown. |
| dlopen/dlsym | Requires a separate ABI milestone. Imports are observed; arbitrary plugin loading is out of scope. |
| Locale, filesystem, environment and timezone | Requires a new ABI milestone for the real libc contract. Locale/conversion, filesystem, environment and `tzset` imports are observed. Timezone data use is inferred; exact tzdata files and their reachability are unknown. |
| Dynamic TLS and TLS destructors | Requires a separate ABI milestone. The TLS relocations above and C++ thread-exit destructor registration interfaces are observed. The bounded resolver has no general DTV, late module allocation or per-thread destructor support. |
| Display, graphics drivers, network behavior and Qalculate!/Qt execution | Out of scope. |

## Resolver and regression evidence

Four published trusted children call the existing resolver directly from
guest user mode. Three use the genuine writable index with exactly one
modified field (module 3, offset 8, or offset UINT64_MAX); the fourth
passes pointer 1, testing rejection before dereference. A return from the
resolver or a probe setup failure exits 97. Each observed call exits 1
through the existing reject path, releases the same child/VM incarnation,
leaves zero owned frames and no regions or page-table root, and returns
`Ok(1)` once followed by `Err(Failed)` to the parent. The launcher publishes
one child as needed to run each test; no additional process publication
was observed. This is independent of the 15 pre-publication rollback
cases. The fixture tests exact supported index semantics, not an
unrestricted resolver contract.

Initial two-vCPU TCG, same-image TCG replay and two-vCPU KVM passed on
ISO SHA-256 `446ac46b07166313dfd9531e2f1a2e0ba6caaed533a295575a9421e09907d397`.
The same gates passed seven positive shared TLS children, parent-exit
cleanup and slot reuse, 15 rollback cases, 14 host ELF tests and two
allocator tests. Same numeric PID recycling remains not exercisable.
Wayland and transport were not rerun. The earlier argument-based fixture
attempt is retained separately as `runtime-abi-01` in the shared TLS test
tree; the interpreter's unchanged argc check made that attempt unsuitable.

```sh
python3 tools/helios-dynamic-elf/inspect_runtime.py target/helios-qalculate target/helios-runtime-abi-inspection/RUN
bash tools/helios-dynamic-elf/build.sh tls RUN
python3 tools/helios-dynamic-elf/replay.py tcg --tls-shared RUN --resolver-negative
python3 tools/helios-dynamic-elf/replay.py tcg --tls-shared RUN --resolver-negative --replay
python3 tools/helios-dynamic-elf/replay.py kvm --tls-shared RUN --resolver-negative
```

## One next milestone: anonymous mapping contract probe

Use one trusted, single-threaded, freestanding guest fixture to verify
Linux x86-64 `mmap(NULL, 4096, PROT_READ|PROT_WRITE,
MAP_PRIVATE|MAP_ANONYMOUS, -1, 0)`, a zero-filled page, one write/read,
`mprotect` to read-only with data retained, and `munmap` followed by
resource reclamation and expected parent wait status. Verify the mapping,
protection and removal in guest VM evidence; do not assume an invalid
memory access is needed to prove protection. Record actual syscall
returns and fail on unexpected behavior. This checks a real dependency
of the staged runtime without requiring an interpreter change.
File-backed mappings, fixed mappings, executable mappings, a libc
allocator, general TLS, threads/futex, signals, unwind and symbol-version
resolution are separate follow-on work. This inspection does not begin
implementation of the mapping milestone.
