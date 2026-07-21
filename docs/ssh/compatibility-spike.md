# Compatibility spike status — Phase 0.12

## Status: not started by design

No compatibility crate, Tokio shim, stream adapter, timer adapter, host-side
duplex test, on-device listener, host key, or SSH daemon was added.

The source audit reached a documented stop condition before any spike could be
safe or useful. A compile-only experiment would require a local `std`/Tokio
port or a broad fake Tokio API and would violate the phase's fork budget.

## What was proved from source inspection

`russh::server::run_stream` can consume an already accepted generic value
implementing Tokio `AsyncRead + AsyncWrite + Unpin + Send`. This is the right
socket-ownership shape in principle: SunlightOS could retain bind/listen/accept
and connection limits.

However, `run_stream` writes the identification, creates an internal Tokio
MPSC channel, reads the client ID under Tokio timeout, initializes session
state, and unconditionally spawns `session.run` through `russh-util`. That
session races reads, handler messages, rekey, keepalive, and inactivity timers
using Tokio tasks and synchronization. The boundary is therefore not
runtime-neutral.

## Fork budget

Allowed: zero protocol/crypto edits; at most narrow upstreamable target cfg or
trait-injection patches, documented with file count and upstream link.

Actual patches: **0 files, 0 lines**.

Required hypothetical work is far beyond budget: supplying Rust `std`, a
Tokio-compatible task runtime and synchronization implementation, a reliable
waker ABI, timer futures, and a new `getrandom` provider. Replacing this by
editing `russh` session/channel internals would touch protocol state and is
forbidden.
