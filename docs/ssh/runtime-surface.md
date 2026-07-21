# SunlightOS runtime compatibility baseline — Phase 0.12

## Current userspace

The workspace uses nightly Rust `1.98.0-nightly` (commit `8954863c8`,
2026-06-05), targets `x86_64-unknown-none`, and has a workspace default of
Rust 2021. The audited `russh` package declares edition 2024 and MSRV 1.85,
which the host compiler meets but the OS target cannot meet because it lacks
`std`.

SunlightOS services are `#![no_std]` and use `alloc`. They have filesystem and
process syscalls, monotonic and realtime clock calls, native thread creation,
and TLS bootstrap. They do not provide Rust `std`, a hosted thread library,
thread join, stack/TLS reclamation, condition variables, thread-local runtime
handles, panic unwinding, environment access suitable for dependencies, or a
general async executor. Panic strategy is `abort` in development and release.

## Existing networking and time surface

`net_server` / `sunlight-net` provides a 128-slot TCP table. A socket identity
packs an ID plus a generation; stale generations are rejected. TCP buffers are
bounded at 8 KiB receive and 4 KiB transmit per socket. The bounded `NetOp::WAIT`
API accepts at most 32 generation-checked identities and wakes for ACCEPT,
READ, WRITE, EOF, RESET, CLOSED, or ERROR, with timeout. It uses deferred IPC
replies and avoids application-side polling while idle.

Solar demonstrates this API with synchronous `try_accept`, SHM-backed reads and
writes, and a wait-set loop. It does not expose a readiness-registration object
that owns a `Waker`, nor `AsyncRead`, `AsyncWrite`, `ReadBuf`, pinning helpers,
or cancellation-safe future-drop behavior.

The kernel and IPC layer use monotonic deadlines, generation-tagged timeout
state, explicit IPC cancellation, and first-terminal-outcome-wins behavior. A
compatibility runtime could build bounded timer registrations on this, but none
exists today.

## Security-adjacent foundations

Phase 0.9's approved path is:

```text
approved entropy -> kernel conditioner -> rand_service ChaCha20 DRBG
                 -> sunlight_libc::getrandom(flags = 0)
```

It fails closed when entropy or `rand_service` is unavailable. The currently
proved custom handler is for `getrandom` 0.2 in `sunlight-tls`; it is not a
generic guarantee for `getrandom` 0.4 / `rand` 0.10.

Phase 0.10 supplies bounded private-secret creation/loading under
`/etc/sunlight/`. Phase 0.11 supplies a bounded, fail-closed
`ValidatedSshConfig`, with TCP/wait-set/PTY limits. These foundations must
remain outside a future SSH library: typed host-key bytes are passed in, and
the library must not choose filesystem paths or create keys.

PTY, UAC identity/spawn, and sunlightd lifecycle facilities exist as separate
services but are explicitly outside this transport-only phase.

## Implication

Solar's stream/readiness primitives make a small runtime-neutral adapter
plausible for a suitable library. They cannot make a Tokio-plus-`std` library
suitable by themselves. The missing layer is not one stream trait; it is an
executor, task lifetime model, waker registration/cleanup model, timer futures,
async synchronization, and an approved randomness provider.
