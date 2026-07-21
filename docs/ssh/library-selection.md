# SSH library selection — Phase 0.12 stop decision

## Decision

**REJECTED FOR SUNLIGHT SSH:** `russh` is not suitable for Phase 0.12 on the
current SunlightOS userspace target.

This is a runtime and platform rejection, not a reimplementation proposal. No
SSH protocol, packet, authentication, or cryptographic code was copied into
the repository.

## Candidate audited

| Item | Value |
| --- | --- |
| crate | `russh` |
| inspected published version | `0.62.3` |
| crate archive SHA-256 | `059dd24c0fe20721639f7acad7b82cd51ec3dd3254ed8cf7a0b7df6c20eaff1c` |
| crate VCS revision | `2ae5d725ab19f5e3f3c4bb8d7bf576cfd4eb83b1` |
| declared repository | `https://github.com/warp-tech/russh` |
| published license | `Apache-2.0` |
| Rust edition / MSRV | 2024 / 1.85 |

The archive was downloaded from the crates.io static package URL and inspected
outside this worktree. Its `.cargo_vcs_info.json` supplies the revision above.
At the audit date, the GitHub release page identified `v0.62.2` as the latest
tagged GitHub release at `c4be19f`, while docs.rs exposed the newer published
crate `0.62.3`. That provenance mismatch is itself a release-verification item;
it is not acceptable to solve it by tracking an unpinned branch.

## Why the candidate stops here

`russh` 0.62.3 is a `std` crate and declares a non-optional Tokio dependency.
For every target other than `wasm32` it also enables Tokio `rt-multi-thread`,
`time`, `net`, and `io-util`. The server session path requires Tokio I/O,
bounded MPSC and oneshot channels, async mutexes and notifications, spawned
tasks, timers, `select!`, and Tokio join handles. The generic `run_stream`
entry point does accept an already-accepted generic async stream, but it still
spawns its transport session through `russh-util`.

SunlightOS userspace currently has `#![no_std]`, `alloc`, synchronous
capability IPC, and a bounded readiness wait. It does not provide Rust `std`,
Tokio, futures/wakers, async synchronization, or a task executor. Its native
thread helper has no join and does not reclaim stacks or TLS blocks. Supplying a
crate named Tokio plus enough `std` to compile this dependency is a broad,
long-lived runtime port, not the small compatibility boundary permitted by this
phase.

The candidate also hard-depends on `getrandom` 0.4 and `rand` 0.10. Its default
features enable `aws-lc-rs`, whose resolved package set includes `aws-lc-sys`,
`cc`, and `cmake`; this conflicts with the current bare-metal/no-C-toolchain
build policy. Disabling defaults avoids that backend but leaves a mandatory
crypto-backend choice and does not remove the `std`, Tokio, or randomness
problems.

This is category **D — impractical fork**. It triggers the phase stop
conditions for unsupported runtime facilities, a broad permanent Tokio
emulation, and inability to prove that cryptographic randomness is routed
exclusively through the approved SunlightOS `getrandom` path.

## Smallest next investigation

Do not integrate or fork `russh`. Perform a bounded desk audit of one
maintained server library whose supported build has all of the following:

- a true `no_std + alloc` mode for the selected SSH transport and crypto;
- generic stream traits not tied to Tokio;
- injectable monotonic timers and secure randomness;
- no mandatory native crypto/C toolchain; and
- bounded server-side packet/channel queues configurable before a connection.

Only after a candidate passes those manifest-level gates should a host-side
in-memory transport spike be proposed.
