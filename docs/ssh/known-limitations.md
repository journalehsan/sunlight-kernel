# Phase 0.12 known limitations and stop record

- There is no `sunlight-ssh`, `sunlight-sshd`, or `sunlight-ssh-runtime` crate.
- No SSH library dependency or lockfile was added to the workspace.
- No host-side in-memory handshake, OpenSSH interoperability test, QEMU test,
  VMware test, resource measurement, or algorithm negotiation occurred.
- The complete transitive license closure remains unapproved because no
  supported candidate feature set exists.
- Existing Phase 0.7 through 0.11 foundations were inspected but not modified.

## Exact stop conditions encountered

1. The selected release requires Rust `std` on a target whose services are
   `no_std`.
2. Tokio is a mandatory, non-feature-gated architecture dependency; on this
   non-Wasm target it requests multithread runtime and networking features.
3. The required Tokio subset is not small: transport/session/channel code uses
   I/O traits/extensions, MPSC, oneshot, mutex, notify, timers, `select!`,
   spawning, and joins.
4. Cryptographic randomness uses mandatory `getrandom` 0.4 and `rand` 0.10,
   not the qualified SunlightOS `getrandom` 0.2 handler.
5. The default crypto backend brings an unsupported native C/CMake build path.

The next phase action is a bounded alternative-library desk audit only. It
must not begin a custom SSH implementation or a permanent russh fork.
