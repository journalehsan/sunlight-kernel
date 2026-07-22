# `sunlight-libc` Core Stdio Audit

**Audit date:** July 22, 2026  
**Scope:** current `sunlight-libc` only; no filesystem, VFS IPC, syscall ABI,
or POSIX-surface redesign.

## Result

`sunlight-libc` does not currently implement a stdio stream layer.  There is
no `FILE` type, `stdio` module, `stdio.h`, standard-stream object, stream
registry, stream buffer, or exported symbol for `fopen`, `fdopen`, `freopen`,
`fclose`, `fflush`, `fread`, `fwrite`, character/line operations, indicators,
buffer configuration, seeking, or pushback.

The sole public C header in the crate is `sunlight-libc/include/string.h`.
The prior libc proof plan also explicitly lists full stdio as a non-goal.
Consequently, adding a new `FILE` ABI and the listed C functions in this
focused hardening phase would violate the rule to preserve the present
exported surface and to implement only an already declared, exported, required,
or clearly intended core interface.

This audit intentionally makes no stdio API or header additions.

## Existing I/O Architecture

Applications currently use the Rust raw-descriptor interface:

```text
application
    -> sunlight_libc::{open, read, write, close, lseek}
    -> synchronous native kernel syscall
    -> kernel VFS or descriptor handle
    -> short count / generic failure / EAGAIN
```

`vfs_server` remains a separate IPC path and is not used by these native libc
wrappers.  `STDIN`, `STDOUT`, and `STDERR` are value constants for descriptors
0, 1, and 2; they are not `FILE` objects.

The raw layer already establishes the necessary future stream boundary:

- `read` and `write` are single-shot and preserve validated short counts.
- `close` delegates to the consume-once native descriptor contract.
- `lseek` and `fstat` are available for future supported seek behavior.
- raw outcomes distinguish generic failure from `EAGAIN`.
- `errno` is TLS-backed after TLS bootstrap and has an early fallback cell.
- `write_all` is an explicit raw helper rather than hidden stream buffering;
  it returns the existing error result and does not silently convert it.

No current raw wrapper logs through stdio, and there is no stream lock or
global stream lock that could recurse through an error logger.

## Lifecycle, Buffering, and State Findings

There is no published or partially initialized stream storage to audit:

- no allocation or ownership of `FILE` storage;
- no descriptor ownership transfer from `fopen` or `fdopen`;
- no standard-stream initialization or cleanup path;
- no buffer allocation, caller-buffer ownership, pending output, or read
  cursor;
- no registration list for `fflush(NULL)`;
- no stream lock, close race, registry race, or lock ordering;
- no EOF, error, last-direction, or `ungetc` state.

Therefore, there is no existing implementation defect in `fread`, `fwrite`,
`fflush`, `fclose`, mode parsing, update-stream transitions, or seeking to
patch.  Treating raw descriptor behavior as if it supplied these stream
semantics would be incorrect: it would hide short I/O, create a new ABI without
a public declaration, and blur ownership of descriptors 0, 1, and 2.

## Standard Streams

The current standard-stream contract is limited to the stable raw descriptor
values:

- descriptor 0 is `STDIN`;
- descriptor 1 is `STDOUT`;
- descriptor 2 is `STDERR`.

The kernel determines their actual routing.  There are no libc-owned buffers,
allocations, registrations, or `FILE` objects to leak or corrupt when a
descriptor is closed.  Existing no-std applications explicitly handle their
own short-I/O loops where required.

## Verification Performed

- Inspected repository status and preserved unrelated manifest/version updates.
- Inspected all `sunlight-libc` modules, the only public C header, public
  exports, libc history, TODO markers, feature declarations, and native
  callers.
- Searched the workspace and a current native `sunlight-libc` archive for the
  stdio symbol inventory; none was present.
- Verified the existing isolated host-proof pattern: modules are included
  directly in `tools/alloc-proof.rs` and `tools/mem-string-proof.rs` to avoid
  linking the complete libc into host test programs.
- On July 22, 2026, `cargo test -p sunlight-libc --target
  x86_64-unknown-linux-gnu --no-run` linked successfully, but executing the
  resulting full test binary panicked in the host Rust time runtime with
  `invalid timestamp`.  The complete libc's exported `clock_gettime` replaces
  the host implementation and is therefore not safe for that monolithic host
  runtime.  Keep using the isolated-test pattern rather than redesigning
  `clock_gettime` in this phase.

No stdio fault-injection, integration, soak, concurrency, QEMU, or bare-metal
test can be added honestly before a stdio implementation and a testable raw-I/O
seam exist.  Adding tests for nonexistent exported functions would create the
same out-of-scope surface this audit rejects.

## Deferred Stdio / Certification Work

A dedicated future stdio phase must first establish and publicly document:

1. the intended header and opaque-or-stable `FILE` ABI;
2. the exact supported stream function inventory and mode grammar;
3. descriptor ownership rules for `fopen`, `fdopen`, and any `freopen`;
4. a bounded stream-state model (`Open`, `Closing`, `Closed`) and safe
   standard-stream initialization;
5. a non-logging, deterministic raw-I/O adapter for short-I/O, `EAGAIN`,
   allocation, registry, seek, and close fault injection;
6. bounded buffer/registry/locking rules that never hold a global registry lock
   across blocking raw I/O;
7. precise element-count, EOF/error, update-direction, and seek behavior;
8. isolated host tests plus target QEMU and bare-metal stream verification.

That later phase must preserve the existing native raw-FD contracts and record
their current generic-error limitation.  It must not use this deferred work to
redesign the filesystem architecture, VFS IPC protocol, filesystem formats,
drivers, or the native syscall error ABI.
