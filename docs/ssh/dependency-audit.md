# russh 0.62.3 dependency audit (stop-stage)

## Reproducible input

The audit used the published `russh-0.62.3.crate` archive, SHA-256
`059dd24c0fe20721639f7acad7b82cd51ec3dd3254ed8cf7a0b7df6c20eaff1c`, its
embedded `Cargo.lock`, normalized `Cargo.toml`, original manifest, and VCS
metadata. No dependency was added to the SunlightOS workspace and no lockfile
was changed.

The embedded lockfile is a package-development lockfile and includes examples,
benchmarks, tests, and dev-dependencies. It is **not** a resolved SunlightOS
feature set, because the candidate was rejected before such a set could exist.
It must not be copied to the workspace as an integration lockfile.

## Direct dependency findings

| Dependency / feature | Status | Relevance |
| --- | --- | --- |
| `std` | mandatory | Source imports `std::io`, collections, `Arc`, `LazyLock`, `SystemTime`, paths, and files throughout the shipped library. |
| `tokio` | mandatory | Manifest enables `io-util`, `sync`, and `time`; non-`wasm32` targets additionally enable `rt-multi-thread` and `net`. |
| `futures` | mandatory | Futures/combinators are part of the transport/session implementation. |
| `getrandom` 0.4 with `wasm_js` | mandatory | Cryptographic RNG path is not injectable through SunlightOS's approved `getrandom` 0.2 custom handler. |
| `rand` 0.10 with `thread_rng` | mandatory | KEX and key routines call `rand::rng()` / `rand::random`; this introduces a second random-provider selection path. |
| `aws-lc-rs` | default feature | Requires `aws-lc-sys`; embedded lock includes `cc` and `cmake`, so it requires native build tooling. |
| `ring` | optional alternative | Still requires the unsupported `std`/Tokio stack and needs independent target/assembly validation. |
| `flate2` | default feature | Enables compression; compression is not needed for the initial transport and must remain disabled if a future candidate is accepted. |
| `rsa` | default feature | Adds RSA and key-file dependencies not required for an initial Ed25519-only transport. |
| `pkcs8` with `std` | mandatory | Directly enables `std`, including key-format support outside the desired typed host-key boundary. |

## Tokio surface actually compiled by the server path

| API | Classification | Consequence |
| --- | --- | --- |
| `tokio::io::{AsyncRead, AsyncWrite, ReadBuf}` | required | The generic stream accepted by `run_stream` implements these exact Tokio traits. |
| I/O extensions | required | Identification, packet flush, and stream splitting use them. |
| `tokio::time::{Instant, sleep, sleep_until, timeout}` | required | Authentication rejection, identification, keepalive, inactivity, and session scheduling use them. |
| `tokio::select!` and `tokio::pin!` | required | The server loop races stream reads, handler events, timers, and shutdown. |
| `tokio::spawn` / Tokio `JoinHandle` | required | `run_stream` always spawns the session through `russh-util`; channel I/O also spawns. |
| `tokio::sync::mpsc` | required | Session event and channel queues are constructed inside protocol state. |
| `tokio::sync::oneshot` | required | Channel confirmations and replies use it. |
| `tokio::sync::{Mutex, Notify}` | required | Channel I/O uses them in shipped non-test code. |
| `tokio::sync::broadcast` | convenience runner only | Avoidable only by not using `run_on_socket`; it does not remove other required Tokio APIs. |
| `tokio::net` | compiled non-Wasm | Avoidable at call site through `run_stream`, but enabled by target-specific manifest. |
| `tokio::runtime` | dependency/runtime requirement | The non-Wasm feature set forces `rt-multi-thread`. |
| `tokio::signal`, `tokio::fs` | unsupported / not needed | No transport integration need was found. |

## Platform classification

| Component | Classification | Reason |
| --- | --- | --- |
| Solar TCP ownership, generations, bounded `WAIT` | A | Existing API has bounded, generation-checked sockets and useful readiness states. |
| Solar stream as Tokio `AsyncRead`/`AsyncWrite` | B in isolation | An adapter could preserve partial I/O and EOF/reset if a real future/waker runtime existed. |
| monotonic timers | B in isolation | The kernel/IPC deadline model can underpin bounded sleeps, but does not expose a Rust future timer today. |
| Rust `std` | D | No supported userspace `std` target exists. |
| Tokio runtime, task/sync/channel model | D | Mandatory, distributed throughout active server/session/channel code; a shim is a broad runtime reimplementation. |
| `getrandom` 0.4 / `rand` 0.10 | D | Cannot be proven to route only through Phase 0.9 without upstream-supported provider injection. |
| default AWS-LC backend | D | Native C/CMake toolchain and target support conflict with current build constraints. |

The audit stops before producing a workspace `Cargo.lock`, a dependency build,
or a feature-resolution test. Creating those would falsely suggest that an
integration is approved.
