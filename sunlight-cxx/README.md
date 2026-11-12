> Current status: Phase 22 is complete for the bounded native C++ subset:
> target-built LLVM exception/RTTI/unwind, allocation, bounded std::thread,
> global/TLS lifecycle, and thread-safe function-local static guards. The final
> section below supersedes earlier unavailable/NOT_RUN statements. The original
> archive-free historical command remains an intrinsic LINK_FAIL.

# Bounded native C++ output and string library

This is a compiled output and owning byte-string subset for native SunlightOS x86-64
LP64. It is **not libc++, libstdc++, or a complete ISO C++ library**. There was no
existing target C++ standard-library strategy or target ABI/unwind library to
integrate. Installed Linux and Windows libraries are not target dependencies.

`bash tools/build_native_cxx.sh` builds the canonical C runtime and a separate
`target/native-c/sysroot/lib/libsunlight_cxx.a`. Its own headers are under
`sunlight-cxx/include`, linked into the sysroot as `include-c++`. The tiny
`sunlight-libc/include/iostream` forwarding include preserves the historical
probe's include path; libc owns no C++ symbols or implementation.

Use `python3 tools/native-c/cxx.py SOURCE.cpp -o OUTPUT`. This invokes the native
C driver with hosted C++ language rules (`main` has C linkage), C++17 by default,
static native startup, the native linker script, and only the two native
archives. Compiler resource headers supply fundamental types. No host C++
headers, startup objects or libraries are searched. Exceptions and RTTI remain
enabled in compiler configuration; their runtime dependencies are unavailable
and programs needing them fail to link. No abort-only exception placeholders
or NULL-returning throwing allocation operators are provided.

## Supported output contract

* `std::cout` uses descriptor 1; `std::cerr` and `std::clog` use descriptor 2.
  They are constant-initialized, require no constructor registration, and have
  no destructor or pending output at exit.
* `std::ostream(int fd)` is a native extension; it does not own/close the fd.
* Narrow `char`, valid NUL-terminated `const char*`, bool as 0/1, signed and
  unsigned short/int/long/long long decimal insertion; no width/base/locale
  formatting. Long and long long are 64 bits. Use `char` for character output;
  signed/unsigned char currently undergo integer promotion.
* `write(const char*, long)` and `put(char)` submit bytes immediately. Negative
  size and nonzero NULL ranges set badbit. A NULL C-string sets badbit.
  Short positive backend writes are retried until complete; zero progress or
  error stops, retains the written prefix, and sets sticky badbit. This native
  backend has no EINTR export/contract; all negative errors terminate output.
* `good`, `bad`, `fail`, explicit bool, `rdstate`, `clear`, `setstate` expose
  bounded stream state. Failed streams suppress output until cleared. State
  bit values and object ABI are specific to this library.
* `std::flush`, `flush()` and `std::endl` work with this **unbuffered** backend:
  there are no pending userspace bytes; flush preserves state and performs no
  syscall. It is not fsync/durability or a check of descriptor validity. Endl
  submits a newline then flushes. Tests witness actual descriptor ordering.

Streams require external serialization if shared among threads, including
inspection/reset of stream state. No concurrency guarantee is claimed.

## Explicitly unavailable

No `cin`, input extraction, streambuf, tie/unitbuf flags, locale facets, wide
streams, floating-point/pointer formatting, full ISO exception/thread/RTTI
library conformance, aligned/nothrow `new` variants, DSO/JIT/foreign-frame
unwinding, custom terminate handlers, dynamic loading/`dlclose`, `quick_exit`, or
complete process-lifecycle support is supplied. Raw pthread callbacks with
nontrivial C++ TLS, throwing C callbacks, and unwinding through Rust frames are
outside the accepted boundary. C `atexit`, the existing C/Rust TLS
implementation, and the bounded C++ lifecycle described below retain their
separate contracts.

The unchanged historical `cpp-runtime.cpp` runs through the configured native
driver with exact output. Its original archive-free `clang++ ... -nostdlib`
command is intentionally bare: it supplies no native startup, archive, linker
script, or image configuration, so it remains LINK_FAIL with runtime NOT_RUN.
That is an intrinsic limitation of the old invocation, not a missing
supported-driver capability. A future upstream library port still needs broader
native OS configuration and additional C library surface. This subset must not
be used as evidence of full C++ conformance.

## Validation

`python3 tools/native-c/tests/run_cxx_output.py --out NEW` checks C++14/17/20
headers, exception/RTTI compiler flags, native archive ownership and final ELF
resolution without host libraries. It confirms the unchanged historical source
hash and next missing header (`<stdexcept>` after Phase 21). It never executes a native ELF on Linux.

`python3 tools/native-c/guest.py --suite --cpus 2 --out NEW` and the same command
with `--cpus 4` execute the output fixtures along with historical and C regression
probes in bounded disposable 2048 MiB guests. `cxx-output` checks exact stdout and
stderr, decimal extrema, flush ordering and actual backend errors.
`cxx-backend` uses linker wrapping of write to deterministically force partial
writes, zero progress and a real invalid-descriptor failure, forwarding accepted
bytes to the actual native backend. It checks sticky state and no repeated
writes after failure. This fault injection is isolated to that test ELF.

## Owning strings (Phase 21)

`<string>` supplies a deliberately bounded `std::string` class, not a
`basic_string` template or libc++/libstdc++ ABI. Do not combine it with another
C++ standard library. Its header is installed through the existing `include-c++`
sysroot link and a libc-directory forwarding header for historical commands.
The compiled implementation belongs only to `libsunlight_cxx.a`.

Discovery found the existing Rust implementation in
`sunlight-libc/src/string.rs`, public C declarations in `include/string.h`, and
memory primitives in `src/mem.rs`. These already provide target-tested C byte
operations but no owning object. The string class reuses their `strlen`,
`memcpy`, `memmove`, and `memcmp` exports and the paired native `malloc/free`
from `sunlight-libc/src/alloc.rs`. No third-party string source or host C++
runtime is imported. Rust `alloc::String` has a different ABI and UTF-8 invariant;
`heapless::String<N>` has fixed capacity. Neither can serve directly as this
C++ byte string. The Linux syscall-only sbase helper is also unsuitable.

Supported operations:

* Empty/default, C-string, `(pointer, length)`, deep copy and move construction;
  destruction; copy/move/C-string assignment including self-assignment and
  assignment from an interior C-string pointer. Moved-from strings are empty,
  NUL-terminated, and reusable.
* `size`, `length`, `empty`, `capacity`, `max_size`, `c_str`, `data`, and `[]`.
  `data()` is mutable starting with C++17, const-only before then. `[]` is
  unchecked: index must be at most size; at size only the NUL value may be
  written. No `at()` or throwing bounds interface is supplied.
* `append(string)`, `append(C-string)`, `append(pointer, length)`, `+=` with a
  string/C-string/char, and `string + string` (including implicit C-string
  conversions). Self-append and interior aliasing work with and without growth.
  C-string inputs must be valid NUL-terminated ranges; explicit-length ranges
  must be readable. `(nullptr, 0)` is supported; nonempty NULL ranges fail fast.
* `compare(string)` and all six relational operators compare unsigned bytes,
  then lengths. Length-based construction, copying, appending, comparisons, and
  stream insertion preserve embedded NULs. Every object retains a trailing NUL.
* Output insertion uses `ostream::write(data(), size())`, preserving its partial
  write retries and sticky errors. C++17 nonthrowing manipulator insertion lets
  owning strings and `endl`/`flush` work without exception cleanup dependencies.

There is no fixed inline buffer cap. Capacity grows geometrically through the
native allocator. `max_size()` is LONG_MAX-1 so allocation size and the signed
stream length stay representable; requests also remain subject to the real
native heap's capacity. Size arithmetic is checked before allocation or reads.
Growth keeps old storage live until aliased bytes have been copied. Ordinary
stack destruction frees storage through the same allocator that created it.

All string operations are `noexcept`. Invalid nonempty NULL ranges, size
overflow, and allocation failure immediately terminate through `sl_std_exit(134)`;
they do not return NULL, truncate, throw `bad_alloc`, unwind, or run atexit
callbacks. This is a native fail-fast policy, **not the ISO throwing failure
contract**. General `operator new/delete` (including sized/aligned variants) are
neither required nor provided. Native C ABI calls have private nonthrowing
declarations in the compiled C++ layer; existing streams now declare their
nonthrowing output/state contract explicitly. Compiler exceptions/RTTI remain
enabled; this does not provide their runtimes. A potentially throwing custom
manipulator or other call while a string is live may still require unavailable
unwind symbols. The driver defaults to C++17; C++14/20 header compatibility is
checked, with native runtime acceptance on the default configuration.

No traits, allocator template, iterators, Unicode interpretation, input streams,
full standard-library conformance, dynamic global initialization/destruction or
nontrivial thread-local lifecycle is established by this phase. In particular,
the global and TLS strings in the historical probe remain runtime NOT_RUN.

`python3 tools/native-c/tests/run_cxx_string.py --out NEW` checks headers,
mutable-data language rules, include order, archive ownership and target-only
linkage. The native suite adds string semantics and 2000 allocation/free cycles,
exact binary output, short/zero/error write injection, and isolated allocation,
size-overflow and invalid-range failures with exact output/status checking.
Run it on both 2 and 4 CPUs as described above.

## Phase 22 partial class implementation — not runtime accepted

Reproduce dependency discovery with
`python3 tools/native-c/tests/run_cxx_exception_discovery.py --out NEW`.
This checks the unchanged historical source and actual compiler-emitted class
and throw/catch dependencies. It does not execute exception objects or establish
native runtime acceptance. Native pthread callbacks cross a Rust extern-C frame;
exceptions must not escape that boundary. Target ABI/unwind integration and EH
table discovery remain prerequisites.

`<exception>` and `<stdexcept>` declare std::exception and std::runtime_error.
The latter owns an immutable reference-counted message, uses atomic reference
updates, and copies without allocating. Constructors retain the native status
134 allocation-failure policy. No logic_error family is supplied.
The compiled object requires sized delete and the Itanium class_type_info and
si_class_type_info vtables; these are unresolved native prerequisites. It is
an archive member, not a usable native exception runtime. No throw/catch,
unwinding, general RTTI or new/delete support is claimed. Runtime validation
of these classes is NOT_RUN. Historical compilation now stops at `<thread>`.
Earlier Phase 20/21 runner expectations for `<stdexcept>` have not been updated
and those runners need review before use with this partial implementation.

## Phase 22 executable native runtime (bounded acceptance; complete for this subset)

The earlier partial-class status above is historical. LLVM libc++abi and
libunwind 21.1.0 sources are pinned under `vendor/`, including licenses and
per-file SHA-256 provenance. `tools/build_native_cxx.sh` builds them for the
native target, never using host ABI/unwind objects. `libsunlight_cxx.a` owns
classes/typeinfo/delete; `libsunlight_eh.a` owns the personality, unwinder,
exception lifecycle, allocation, native TLS bookkeeping and C++ entry/lifecycle.
The native libc retains allocator, pthread, TLS and process termination ownership.

`cxx.py` links these with native startup, retains indexed unwind metadata and
wraps main to catch exceptions before Rust startup frames. Real typed/base and
catch-all handling, nested rethrow, cleanup landing pads, exception reclamation,
per-thread handlers and expected uncaught termination have executed in guests.
No Rust-frame unwind is supported. Scalar/array throwing new throws bad_alloc
on heap failure; exception-storage exhaustion terminates with group status 134.
String and runtime_error message construction retain their separate fail-fast
allocation policy. Sized/unsized delete pair with native free. Message copies
remain noexcept, shared and immutable; string messages retain embedded NUL bytes.

`<thread>` supports nullary callable objects, moves, joinable and join through
the existing pthread lifecycle. Callable and TLS cleanup precede return to the
Rust trampoline. A joinable destructor and worker uncaught exception terminate
the process group. Create/join failures throw runtime_error, not conforming
system_error. No argument packs, detach, ids, this_thread or full threading
library is supplied. Descriptors remain per-thread snapshots.

Global constructors run from retained .init_array after native TLS startup.
Static-image __cxa_atexit and native per-thread __cxa_thread_atexit registries
handle global/main/std::thread destruction. One of native C's 32 atexit slots
is reserved for C++ finalization.

Function-local static guards use the unmodified upstream LLVM
`libcxxabi/src/cxa_guard.cpp` and `cxa_guard_impl.h` (21.1.0, recorded in
`vendor/libcxxabi/PROVENANCE.json`) with the upstream futex implementation
selected. A private `sys/syscall.h` adapter maps the two operations the
implementation uses onto the native private-address-space futex syscall (155)
and a per-thread identity; unknown syscall numbers return -1. No global mutex is
held across user initialization, waiters block in the native futex, and the
guard byte is published with release/acquire ordering. `__cxa_guard_abort`
resets ownership after a throwing initializer so a later call can retry.
Recursive initialization of the same guard is detected by thread identity and
terminates the process group with status 134 and an explanatory diagnostic.
The guard objects are the compiler-emitted 8-byte Itanium layout; generated
code references `__cxa_guard_acquire`, `__cxa_guard_release` and
`__cxa_guard_abort` and registers successful local-static destructors through
`__cxa_atexit`/`__dso_handle`.

DSO/JIT frames, raw-pthread nontrivial TLS cleanup, pthread_exit/cancellation
inside C++ workers, throwing C callbacks, custom .fini_array traversal,
aligned/nothrow new and complete RTTI/library conformance are not accepted.
Tested RTTI covers simple pointer casts/typeid; unlisted inheritance/EH matching
cases remain unvalidated.

Fresh validation: **93/93 buildable native cases PASS on both 2 and 4 CPUs,
2048 MiB**, including class lifetime, real EH, thread/TLS cleanup, four isolated
fatal cases, and six local-static guard cases (basic, concurrent, exception
retry, nested/independent, destruction order, and isolated recursive
termination). Separate global/TLS/atexit ordering and balanced-allocation tests
also PASS on both. The unchanged historical source runs through the native
driver, printing exactly `exception OK`, `TLS OK`, `static initialization OK`,
`atexit OK` on successive lines, with exit 0 and clean reaping. Its original
archive-free command is preserved and remains **LINK_FAIL, runtime NOT_RUN**
because `-nostdlib` supplies neither native startup/archive nor image
configuration; this is an intrinsic limitation of that old invocation, not a
missing supported-driver capability. Phase 22 is complete for the bounded
subset documented here.

Run `tools/native-c/tests/run_cxx_classes.py` and `run_cxx_runtime.py` with fresh
`--out` paths for target ABI/image checks; run `guest.py --suite --cpus 2`
(and 4) `--timeout 180 --out NEW` for execution. Source/build provenance,
the full historical dependency matrix, exact commands, regression results,
remaining boundaries and separate pre-existing failures are recorded in
[the current Phase 22 audit](../docs/audit/std-progress.md) and
[its evidence](../docs/audit/probes/native-foundation/results-2026-11-11-cxx-runtime/).
The C socket bridge and deterministic service/client networking acceptance
remain deferred.
