# SunlightOS readiness for a russh SSH server

Audit date: **2026-09-21**. Repository baseline:
`a50ce09b4d8967128a373407ed13686660b58475`.
Scope: source audit, published dependency inspection, and focused host tests.
This change adds documentation only; it neither implements nor enables SSH.

## Decision

**Not ready to implement a working russh daemon on the current native target.**
Networking, authentication, configuration, and PTY foundations exist, but several
required connections between them are missing. Runtime support is the largest
prerequisite; repairing it alone would still leave remote login incomplete.

Use a separate future `services/sunlight-sshd` service. Keep `pty_server` as the
shared terminal broker. An SSH dependency inside that broker would couple local
Terminal availability to the network-facing protocol runtime.

The findings below distinguish source-confirmed behavior from proposed work.
No finding claims a fresh QEMU result.

## 1. Published russh and platform requirements

The crates.io index retrieved during this audit ends at non-yanked **0.63.3**.
Its downloaded archive checksum matches the index:

| Input | Value |
| --- | --- |
| Package | `russh 0.63.3`, Apache-2.0 |
| Archive SHA-256 | `036204edbd199552a5b3832f63c60dcdf395dc44c7f06b4af1c0e8139cc11bce` |
| Embedded VCS revision | `f33baf439be8c59c49cb6cb2ae5976c579495bdf` |
| Edition / minimum Rust | 2024 / 1.89 |
| Local compiler | `rustc 1.98.0-nightly (8954863c8 2026-06-05)` |
| Native service target | `x86_64-unknown-none`, normally `no_std` |

Primary upstream inputs for repeating the audit:

```text
https://index.crates.io/ru/ss/russh
https://static.crates.io/crates/russh/russh-0.63.3.crate
https://static.crates.io/crates/russh-util/russh-util-0.52.0.crate
https://static.crates.io/crates/getrandom/getrandom-0.4.3.crate
```

The crates.io metadata API returned HTTP 403; version/checksum verification used
the index instead. Older documents inspect 0.62.3. No new dependency graph was
resolved or built. Versions from the archive's development lockfile are source
inspection inputs, not an approved SunlightOS lockfile or license closure.

### Runtime blocker

In the 0.63.3 archive, `Cargo.toml` requires Tokio `io-util`, `sync`, and `time`;
the non-Wasm dependency additionally enables `rt-multi-thread` and `net`.
`src/server/mod.rs::run_stream` accepts Tokio
`AsyncRead + AsyncWrite + Unpin + Send + 'static`, creates internal queues,
and spawns `session.run`. `russh-util 0.52.0/src/runtime.rs` delegates native
spawning to `tokio::spawn`. Channel I/O also uses Tokio spawning, mutexes,
notifications, channels, and I/O extensions.

An accepted-stream adapter avoids Tokio's listener at the call site; it does
**not** remove the compiled runtime/network dependencies. Enabling multithread
support does not force the application to run multiple workers: a current-thread
Tokio runtime is a reasonable eventual starting point, but it still needs a
supported platform, timers, synchronization, and wakeups.

Repository evidence:

- [Target configuration](../../.cargo/config.toml) and
  [native thread helper](../../sunlight-libc/src/thread.rs): no native Rust
  `std` target is supplied here. The helper has no join and documents incomplete
  stack/TLS reclamation and non-shared descriptor-table mutations.
- [sunsay proof](../../std-proof/sunlight-sunsay/src/main.rs) is still
  `#![no_std]`. Its directory name and the std planning document do not prove
  hosted Rust `std` support.
- [Linux dispatch](../../kernel/src/arch/x86_64/syscall.rs),
  [translation](../../compat-linux/src/lib.rs), and
  [Helios coverage](../Helios_LinuxCompat/coverage.md): Linux sockets and futexes
  are unsupported; clone goes to an unsupported handler. Existing epoll/poll and
  static command-line programs do not establish Tokio support.

Rust already provides `Future`, `Poll`, and `Waker` in `core`; the missing work
is an OS-backed executor/reactor and the actual dependency runtime. A small
independent executor would not satisfy russh's Tokio calls without a larger port.

### Crypto and randomness

The manifest defaults to AWS-LC, compression, and RSA. `src/lib.rs` requires
either `ring` or `aws-lc-rs`; disabling all defaults is not a complete build.
Evaluate one backend for the chosen platform, including native build tools,
assembly, CPU requirements, licenses, and randomness. Disabling compression and
RSA reduces scope but does not remove Tokio or `std`.

**Correction to the older rejection rationale:** getrandom 0.4.3 has a supported
custom backend. Its `README.md` and `src/backends/custom.rs` define the
`getrandom_backend="custom"` configuration and a single root-provided
`__getrandom_v03_custom` function. The symbol retains `v03` in this release.
Thus the change from the existing 0.2 integration is an engineering and
verification task, not proof that randomness injection requires a fork.

For a native port, route that provider through
[libc's secure path](../../sunlight-libc/src/rand.rs) with flags zero; initialize
uninitialized destination memory before creating a byte slice, fill completely,
and propagate failures. Audit every resolved getrandom version, rand thread RNG,
and crypto-backend entropy path. The existing 0.2 registration does not configure
0.4. For a Helios route, separately qualify `sys_linux_getrandom`: it uses kernel
conditioned entropy directly rather than the native rand-service route.

### Resource bounds

0.63.3 defaults include a 2 MiB channel window, 32 KiB maximum packet, 100 queued
channel messages, and 10 queued session events (`server::Config`). These are
not a SunlightOS memory budget. `run_stream` creates an **unbounded priority
queue**, and session code maintains pending data and request collections.

Set explicit windows/queue sizes and enforce one session channel per connection.
Trace allocation producers, including control traffic and rekey buffering;
prove a bound or require a narrow upstream solution. An unbounded queue is a
review item, not by itself a demonstrated remote memory-exhaustion exploit.
Measure total tasks, stacks, SHM pages, packet buffers, and crypto working memory
at the maximum configured connection count.

## 2. PTY findings that block an interactive remote shell

Evidence: [broker](../../services/pty_server/src/main.rs),
[wire messages](../../ipc/src/lib.rs), [bulk buffer](../../ipc/src/pty.rs),
[libc](../../sunlight-libc/src/lib.rs), and
[Terminal client](../../services/sunlight-terminal/src/main.rs).

| Priority | Source-confirmed finding | Required change / acceptance gate |
| --- | --- | --- |
| P0 | `openpty()` documents `(master, slave)` but returns `CREATE` caps unchanged. The broker returns **master and control**; the helper also discards ID/generation. | Add a typed client handle with identity, endpoint, master and control; obtain the slave through `ATTACH_SLAVE`. Migrate callers and test role/stale-handle rejection. |
| P0 | `create` records the creator's UID/GID. Nonzero-target `attach_slave` and `set_foreground_process` require that same identity. | A daemon-owned PTY cannot directly attach another user's shell. Design an authenticated session broker/worker or an atomic grant-backed create-and-spawn operation. Never trust a client-supplied UID or relax ownership checks globally. |
| P0 | `attach_slave` accepts PID zero; `Authority::matches` then treats the token as bearer authority usable by any PID holding it. | Replace the pre-spawn wildcard with a bounded, single-use attachment handshake for remote sessions. Bind the child before execution, preserving local Terminal behavior. |
| P0 | `_start` only receives/replies to IPC; no owner-death sweep or exit subscription exists. `CLOSE_SESSION` performs full destruction. | Reclaim sessions after daemon/worker/shell crashes, wake waiters, invalidate generations, and bound output draining. Repeated crashes must return slots to baseline. |
| P0 | No PTY wait/subscribe operation exists. Reads return would-block; closes do not deliver asynchronous events. | Add bounded generation-checked readable/writable/hangup waits with cancellation, or common IPC notifications. Idle SSH sessions must wake for PTY output even without incoming TCP bytes. |
| P1 | Bulk SHM covers slave-to-master output only. Master-to-slave input carries eight bytes per IPC. | Preserve partial input writes and bound retained data; add symmetric bulk input if measurements require it. Never discard a paste when the input ring fills. |
| P0 | Mode flags are stored but not applied by byte transfers. Resize updates geometry; the broker does not implement POSIX line discipline, signal dispatch, or process groups. | Define terminal/foreground behavior and test echo, erase, Ctrl-C, EOF, resize, and password entry. Flag storage alone is not termios/job-control support. |

The broker already provides 16 sessions, independent 8 KiB input/output rings,
generation checks, role tokens, validated geometry, and explicit close operations.
The configured eight-session SSH allowance is arithmetic in `ssh_config`, not a
reservation enforced by the PTY allocator. Admission must handle competition
with local terminals without stealing their slots.

Geometry allows 512 columns, 256 rows, and 16384 pixels per dimension. SSH
supplies wider integers: reject or deliberately clamp unsupported values before
conversion, without truncation. Publish initial size before child execution and
verify resize reaches foreground applications.

### Authenticated spawning does not yet carry PTY startup data

[UAC](../../services/sunlight-uac/src/auth.rs) supplies centralized password
verification and caller-bound session grants.
[Kernel spawn handling](../../kernel/src/arch/x86_64/syscall.rs) consumes a grant
for `SPAWN_AUTHENTICATED`, selects the user-session capability profile, and calls
the loader with **empty argv**. Terminal's `spawn_shell` passes PTY identity and
tokens through argv using `libc::spawn`; that kernel path inherits the
**caller's** UID/GID and capabilities.

Neither path provides authenticated identity plus PTY arguments together. Add
a restricted, bounded spawn contract installing identity, capabilities, terminal
attachment, environment, and startup data before shell execution. Do not change
a shared daemon's identity per login. Account for `TERM`, home/cwd, shell
selection, and permitted environment variables.

The [grant table](../../kernel/src/capability/mod.rs) has 64 entries. Consumption
removes a grant, but minting does not sweep abandoned expired grants. Add expiry
and owner-exit reclamation: successful authentication followed by disconnect must
not permanently exhaust logins. Reconcile grant expiry with the delay between
SSH authentication and a later shell request; an expiring one-shot grant cannot
serve as the connection's permanent identity.

### The PTY shell expects graphical Terminal

[sunshell](../../sunshell/src/main.rs)::`start_pty_shell` emits custom OSC 9001
prompt/application messages. Its normal input path expects complete lines from
Terminal's footer. `Shell::handle_byte` accumulates printable bytes without echo
and handles backspace 0x08; common remote erase input may instead be 0x7f.

A raw SSH relay therefore does not provide the same usable prompt/editing
experience. Add an explicit standard-terminal shell mode with inline prompts,
consistent erase/echo/CRLF behavior, and no required GUI footer messages. Keep
password input non-echoing. Test foreground programs separately; arbitrary SSH
`exec` commands are not implied by an interactive shell implementation.

## 3. Networking, authentication policy, and persistence

| Area | Existing evidence | Remaining work |
| --- | --- | --- |
| TCP | [Manager](../../sunlight-net/src/tcp.rs): owner-scoped generation handles, 128 slots, partial I/O, close/reap logic. [net_server](../../services/net_server/src/main.rs): deferred `WAIT`, 32 identities per wait, 64 waiters. | Cancellation-safe async readiness; preserve EOF/reset/would-block distinctions, SHM ownership, and partial writes. |
| Adapter | [Solar](../../services/solar/src/net.rs) demonstrates bind/accept/SHM/waits. | Extract typed reusable APIs. Synchronous IPC and `write_all` cannot be copied into executor poll methods that must return promptly. Define flush/shutdown and timer/PTY/process-exit wakeups. |
| Authentication | UAC verifies shadow hashes and returns grants. SSH config bounds attempts/connections/login time. | Wire limits into handlers, establish explicit remote-account/root policy, rate-limit across reconnects, bound UAC calls, and require account provisioning/password changes before remote exposure. Never log passwords. |
| Identity lifetime | Caller-bound grants and a restricted user-session profile exist. | Reclaim grants and bind worker/PTY/shell lifetime to the connection. Expose trustworthy peer-address metadata in the TCP API if per-peer policy is required. |
| Configuration | [Parser](../../sunlight-libc/src/ssh_config.rs): eight required fields, max eight connections and one session each. [ramfs](../../sunlight-fs/src/ramfs.rs) supplies the file. | Implement `ValidatedSshStartup` and enforce limits live. `password_authentication=false` must not accidentally allow another authentication method. |
| Host keys | [secret_store](../../sunlight-libc/src/secret_store.rs): bounded private atomic file operations. | Connect library key generation/parsing; retain fingerprint through restart/reboot. `RequireDurability` explicitly fails: atomic visibility does not prove disk durability or persistent `/etc` backing. |
| Supervision | [sunlightd](../../services/sunlightd/src/main.rs) has dependency, capability, and persistent-enable infrastructure; no SSH unit. | Disabled-by-default unit, narrow capabilities, readiness, graceful stop/restart, admission shutdown followed by bounded drain. |
| Packaging | [init](../../services/init/src/main.rs) starts PTY. [runs.sh](../../tools/runs.sh) already forwards host 2222 to guest 22. | Add daemon consistently to workspace/build/image/loading paths, including embedded-binary recovery if used. Forwarding is not listener readiness. |

Do not block the whole executor on synchronous broker IPC or password verification.
Choose a bounded IPC completion design or worker model during the platform proof.
Thread-per-request does not resolve current thread lifecycle limitations.

## 4. Ordered implementation plan

### Gate A — prove the runtime platform

For a native service, scope a real SunlightOS `std` platform plus Tokio support
and the TCP/IPC reactor, preserving the libc/service architecture. Alternatively,
a Linux-musl russh process through Helios needs Linux sockets, synchronization,
runtime support, and a restricted native PTY/UAC bridge. Neither route is ready.

Deliver a pinned dependency graph, backend/license report, and guest runtime
probe covering tasks, timers, wakeups, bounded channels, cancellation, RNG
failure, and cleanup. Then test a host in-memory russh handshake and the same
transport on the guest. A host-only build does not pass this gate.

Keep protocol/crypto edits at zero; record narrow platform patches and their
upstream/maintenance plan. If platform work exceeds the accepted budget, keep
SSH deferred instead of emulating Tokio inside the daemon.

### Gate B — complete session foundations independently of SSH

1. Repair the typed PTY client and share it with Terminal.
2. Add owner-exit cleanup and bounded cancellation-safe readiness waits.
3. Add authenticated PTY/shell creation, grant reclamation, and atomic child
   attachment. Test that one user's tokens cannot operate another user's PTY.
4. Add standard-terminal shell mode and foreground lifecycle behavior.
5. Prove key storage on the intended boot/storage configuration.

These can be validated without introducing russh. The recommended first task is
the typed PTY API repair: it is small, source-confirmed, and useful to both local
and eventual remote terminals. Runtime platform work remains the critical path.

### Gate C — implement the daemon adapter

Keep listener ownership/admission in the daemon; hand accepted streams to
`russh::server::run_stream`. Proposed handler mapping:

| Callback/event | Sunlight action |
| --- | --- |
| `auth_password` | Bounded UAC verification, remote-account policy, connection identity. |
| `channel_open_session` | At most one session channel after authentication. |
| `pty_request` | Validate TERM/modes/geometry; prepare authenticated PTY, then reply. |
| `shell_request` | Start exactly one restricted shell in standard-terminal mode. |
| `data` / PTY-readable | Bounded relay with partial-I/O accounting; read only with downstream capacity. |
| `window_change_request` | Validate/publish geometry to the foreground session. |
| EOF / close / shell exit | Distinguish input EOF from disconnect; bounded output drain, exit status, channel close, resource release. |

Initially reject unsupported `exec`, subsystem/SFTP, forwarding, X11, agent,
environment, and signal requests explicitly. Add terminal signals only through
an authorized foreground-session operation. Reject duplicate/out-of-order
PTY/shell requests and roll back partial creation on every error.

### Gate D — acceptance before enablement

- Adapter tests: partial I/O, backpressure, stale handles, wakeup races,
  cancellation, and resource accounting.
- Real OpenSSH client with isolated known-hosts file: prompt, echo, erase,
  Unicode/escape policy, password non-echo, foreground input/output, resize,
  Ctrl-C, EOF, exit status, and disconnect.
- Refused credentials, disabled password auth, stalled identification/KEX/login,
  malformed traffic, extra channels, explicit algorithm allowlists.
- Missing entropy fails closed. Service restart/reboot preserve fingerprint and
  enabled state; no silent replacement of an existing host identity.
- Maximum connections plus local Terminal use; slow-reader/large-paste tests;
  idle CPU; repeated disconnect, broker failure, shell crash, and daemon restart
  cycles with stable memory/socket/PTY/grant counts.
- QEMU boot gate and recorded serial evidence; hardware/network validation where
  required. Track host checks, guest execution, and human usability separately.

## 5. Verification performed

Commands used the existing lockfile and cached packages:

```sh
cargo test --offline --target x86_64-unknown-linux-gnu \
  -p pty_server -p sunlight-libc -p sunlight-net --lib --bins
cargo test --offline --target x86_64-unknown-linux-gnu -p sunlight-net --lib
cargo test --offline --target x86_64-unknown-linux-gnu \
  -p sunlight-libc --lib ssh_config::tests -- --test-threads=1
cargo test --offline --target x86_64-unknown-linux-gnu -p sunlight-libc --lib \
  alloc::tests::free_list_exact_fit_unsplittable_tail_and_full_recovery \
  -- --exact --test-threads=1
```

- PTY: **8 passed**. Ring/limit/generation and selected pure-logic tests do not
  execute live IPC credential/attachment handlers.
- Networking, run separately: **19 passed**, host logic only.
- SSH configuration, isolated: **6 passed**.
- Combined default-parallel run: allocator exact-fit failure, then stalled;
  interrupted. The allocator test **passed in isolation**. Concurrent tests
  sharing heap state are a plausible cause, not a confirmed runtime defect.
- Full libc rerun with `--test-threads=1`: timed out under a 45-second wrapper
  (exit 124), last printing
  `secret_store::tests::default_options_are_private_and_bounded`. Source tracing
  shows `validate_options` calls native `getuid/getgid` wrappers without a host
  mock. The full suite is **not passed**; use injected identity or guest tests.
- Existing warnings were emitted. No runtime code, dependency, lockfile, key,
  service unit, or listener was added. No QEMU boot, guest SSH handshake, crypto
  qualification, live storage/reboot test, or performance measurement was run.

Foundation code alone is not SSH readiness. The older stop decision remains
valid for the current platform, with the randomness/core-future clarifications
above.
