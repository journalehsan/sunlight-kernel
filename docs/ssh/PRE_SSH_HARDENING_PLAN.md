# Sunlight SSH Phase 0: Pre-SSH Hardening Plan

## Purpose

Sunlight SSH Phase 1 requires a real, standards-compatible SSH server that can
be used by normal OpenSSH clients. The current repository has useful networking,
service-management, PTY, shell, filesystem, random-number, and process
foundations, but several of those foundations do not yet meet the correctness
and security requirements of an Internet-facing authentication service.

This document defines the prerequisite work that must be completed before
implementation of `sunlight-sshd` begins.

The Phase 0 work is intentionally not an SSH implementation. It must not add:

- a custom remote-shell protocol
- SSH packet parsing
- SSH cryptographic primitives
- a temporary plaintext SSH substitute
- a second account or password database
- public-key login, SCP, SFTP, forwarding, or other later SSH features

## Entry Rule for SSH Phase 1

SSH Phase 1 may start only when every blocking gate in this document passes in
both QEMU and VMware, unless a gate is explicitly marked as host-test-only.

A successful compile is not sufficient. Each subsystem must demonstrate its
runtime behavior, cleanup behavior, and failure behavior.

## Current Integration Points

The future SSH daemon will integrate with:

- `sunlightd` for service lifecycle and persistent enablement
- `net_server` for listening TCP sockets and connected streams
- the SunlightOS authentication service for password verification
- `pty_server` for one PTY per interactive SSH session
- `/bin/sshl` for the native Sunlight shell
- the process-spawn layer for authenticated UID/GID and environment setup
- VFS for `/etc/sunlight/ssh.toml` and the persistent server host key
- `rand_service` through `sunlight_libc::getrandom` for cryptographic randomness
- monotonic time APIs for authentication and shutdown deadlines
- existing `debug_log` conventions for bounded service diagnostics

## Blocking Findings

The following issues block a safe SSH implementation:

1. `sunlightd` enable/disable state is currently memory-only.
2. Service stop and restart do not wait for daemon cleanup or process exit.
3. Service unit `User=` and least-privilege capability declarations are not
   enforced by the spawn path.
4. The current login path compares plaintext passwords and contains hardcoded
   fallback credentials.
5. `uac_service` does not currently verify password bytes.
6. Userspace cannot spawn a process as an authenticated target UID/GID with an
   explicit environment and scoped capability set.
7. PTYs do not store or update terminal window dimensions.
8. The TCP readiness API requires repeated non-blocking polling.
9. TCP socket backing buffers are leaked after socket close.
10. The existing SSH library candidate requires runtime facilities not currently
    supplied by SunlightOS userspace.
11. Service journal capture and `sunlightctl` log retrieval remain incomplete.
12. The secure-entropy guarantee has not been validated on every supported VM
    configuration.

---

## Phase 0.1: Central Authentication Service

### Goal

Create one canonical SunlightOS account/password verification path shared by
TTY login, graphical login, `runas`, and the future SSH daemon.

### Required Behavior

- Keep `/etc/passwd` as the username, UID, GID, home, and shell source.
- Keep `/etc/shadow` as the protected password-verifier source.
- Replace plaintext passwords with a versioned, salted password-hash format.
- Use a maintained password-hashing crate that can run in the SunlightOS
  userspace environment.
- Prefer Argon2id if memory and runtime measurements are acceptable.
- Otherwise use a reviewed maintained alternative with documented parameters.
- Perform comparison in constant time where practical.
- Return one generic authentication failure result for unknown users, wrong
  passwords, malformed verifier records, and locked accounts.
- Never expose shadow records to ordinary applications.
- Never fall back to hardcoded credentials when the authentication service or
  VFS is unavailable.
- Never log password bytes, password lengths, salts, hashes, or derived values.
- Zero transient password buffers after verification where the compiler/runtime
  facilities permit it.
- Bound username and password input lengths.

### Proposed API

Add a bounded authentication IPC request to the existing authentication broker:

```text
AUTH_PASSWORD(username, password_buffer_cap) -> success/failure + uid/gid/home/shell
```

The password should use bounded shared memory or another protected transfer
mechanism rather than being packed into debug-visible generic register fields.
The broker must consume the input synchronously and clear the shared buffer
before returning ownership.

### Migration

- Add a one-time development-image migration for current plaintext shadow data.
- Remove `fallback_auth`.
- Route TTY and graphical login through the broker.
- Route `runas` through the same verification operation.
- Retain one account database and one verifier implementation.

### Gates

- Correct credentials succeed for every provisioned user.
- Incorrect passwords fail with the same client-visible result as unknown users.
- Authentication fails closed when VFS or the authentication broker is absent.
- No plaintext password remains in `/etc/shadow`.
- No hardcoded password appears in executable code or fallback paths.
- TTY and graphical login continue to work in QEMU and VMware.
- Repeated authentication does not continuously increase memory use.
- Unit tests cover verifier parsing, malformed records, correct passwords,
  incorrect passwords, unknown users, maximum lengths, and buffer clearing.

---

## Phase 0.2: Persistent sunlightd Service State

### Goal

Make `sunlightd` the persistent source of truth for whether a service is enabled.

### Required Behavior

- Store enablement state under an existing protected SunlightOS state location.
- Load persisted state before the autostart queue is created.
- Preserve the compiled default when no persisted override exists.
- Make the future `sunlight-ssh` compiled default disabled.
- Write state atomically using temporary-file, chmod, write, close, and rename.
- Reject malformed state without silently enabling a disabled service.
- Make `enable` and `disable` idempotent.
- Do not store enablement in `/etc/sunlight/ssh.toml`.

### Suggested State Model

Use one bounded versioned file, for example:

```text
/var/lib/sunlightd/enabled-services
```

Each record should contain only a validated service ID and enabled/disabled
state. Unknown service IDs must be ignored with a diagnostic.

### Gates

- A clean installation reports `sunlight-ssh` disabled.
- A disabled service is not started during boot.
- `sunlightctl enable sunlight-ssh` survives a sunlightd restart and OS reboot.
- `sunlightctl disable sunlight-ssh` survives a sunlightd restart and OS reboot.
- A truncated or malformed state file does not enable any service unexpectedly.
- Failed persistence is reported and does not falsely report success.

---

## Phase 0.3: Reliable Service Lifecycle

### Goal

Make start, stop, restart, status, and failure reporting accurate enough for a
network daemon with active sessions.

### Required Behavior

- Deliver `SIGTERM` on stop.
- Allow the daemon a bounded grace period to close listeners and sessions.
- Track actual child exit before reporting `Stopped`.
- Escalate to `SIGKILL` only after the configured grace period.
- Restart only after the previous process has exited.
- Detect unexpected process exit and update state to `Failed`.
- Apply restart policy and restart-rate limits to real exit events.
- Expose the most recent failure reason or exit status through `status`.
- Do not mark a daemon `Running` solely because the spawn request returned a PID.
- Add an optional readiness notification for services that must validate config,
  load keys, and bind sockets before being considered active.

### Suggested Readiness Flow

```text
sunlightd spawn -> Starting
daemon validates dependencies and binds listener
daemon sends READY or STARTUP_FAILED
sunlightd -> Running or Failed
```

### Gates

- Stop waits until the process exits or the grace deadline expires.
- Restart never leaves two instances listening simultaneously.
- Bind failure produces `Failed`, not `Running`.
- A stopped network daemon releases its listener.
- Killing a managed daemon updates status and applies its restart policy.
- Repeated failure hits the restart-rate limit without spinning.

---

## Phase 0.4: Service Identity and Least Privilege

### Goal

Enforce service user identity and explicit capability assignment during spawn.

### Required Behavior

- Apply unit `User=` to the spawned process.
- Add explicit service capability declarations to the unit model.
- Resolve declared dependencies into narrowly scoped tokens at spawn time.
- Do not grant every daemon unrestricted nameserver access by default.
- Prevent an SSH daemon from inheriting unrelated display, device, swap,
  storage-admin, or global control capabilities.
- Provide separate capability classes for:
  - TCP listener and connected-socket operations
  - authentication requests
  - PTY allocation and resize
  - user process spawning
  - configuration-file reads
  - host-key read/write/chmod/rename
  - logging and readiness notification
- Make capability assignment observable in diagnostics without printing raw
  secret token values.

### Future SSH Capability Set

`sunlight-sshd` should receive only:

- network server capability
- authentication verification capability
- PTY broker capability
- constrained spawn-as-user capability
- read access to `/etc/sunlight/ssh.toml`
- read/write access to the configured host-key path
- service lifecycle/readiness capability
- logging capability
- secure random and monotonic time access

### Gates

- A test service cannot access a capability it did not declare.
- `User=` changes the daemon UID/GID as reported by `getuid` and `getgid`.
- An unprivileged daemon cannot mint capabilities or spawn arbitrary identities.
- The declared SSH capability profile excludes display and hardware services.
- Capability failures are reported without crashing `sunlightd`.

---

## Phase 0.5: Spawn an Authenticated User Session

### Goal

Allow a trusted login service to start the native shell under an authenticated
user identity without giving the login daemon unrestricted process authority.

### Required Behavior

- Add a privileged, brokered spawn operation that accepts:
  - verified authentication/session token
  - target UID and GID
  - executable path
  - bounded argv
  - explicit environment
  - PTY session attachment
  - constrained child capability profile
- Require a short-lived authentication grant minted by the authentication
  broker; do not trust caller-supplied UID/GID alone.
- Validate that the requested UID/GID, home, and shell match the account record.
- Set a minimal environment:
  - `USER`
  - `HOME`
  - `SHELL`
  - `PATH`
  - `TERM`
  - locale variables
- Do not inherit daemon-private environment values or capabilities.
- Return a process handle suitable for wait, signal, exit-status, and teardown.
- Terminate the shell and its foreground descendants when its remote session
  closes.

### Gates

- A test login broker spawns `/bin/sshl` as an unprivileged user.
- `whoami`, `id`, `HOME`, and filesystem permissions reflect that user.
- A forged UID/GID request without an authentication grant is rejected.
- Child processes do not inherit daemon-only capabilities.
- Disconnect cleanup terminates the shell and foreground process tree.
- Exit status is available to the session owner.

---

## Phase 0.6: PTY Ownership, Resize, and Lifecycle

### Goal

Make the existing PTY infrastructure sufficient for remote interactive sessions
without creating a separate SSH-specific terminal stack.

### Required Behavior

- Increase or make configurable the PTY session limit so it can satisfy the
  configured SSH connection bound while preserving graphical Terminal usage.
- Add PTY owner identity and session-owner capability checks.
- Add stored terminal geometry:
  - columns
  - rows
  - pixel width
  - pixel height
- Add `SET_WINDOW_SIZE` and `GET_WINDOW_SIZE` operations.
- Apply initial geometry during PTY creation or before shell start.
- Propagate resize to the foreground process using `SIGWINCH` when supported.
- Distinguish master, slave, and control authority instead of returning aliases
  with equivalent authority.
- Define behavior when bounded input/output rings fill.
- Report peer closure separately from an empty non-blocking read.
- Ensure close wakes or terminates blocked users cleanly.

### Gates

- Two sessions receive separate PTY IDs, rings, dimensions, and ownership.
- Resizing one PTY does not modify another PTY.
- `TIOCGWINSZ` reports the last applied size.
- A foreground test app observes `SIGWINCH`.
- Closing the control endpoint releases all PTY buffers and wakes both ends.
- Graphical Terminal tab creation, input, output, and close behavior remain
  unchanged.
- TTY login and native shell behavior remain unchanged.

---

## Phase 0.7: Event-Driven TCP Server Support

### Goal

Provide a bounded server socket API that can sleep until useful work exists.

### Required Behavior

- Support bind by local IPv4 address and port.
- Reject invalid addresses and port zero.
- Add a blocking or notification-based readiness operation for:
  - listener accept readiness
  - readable connected sockets
  - writable connected sockets
  - peer close
  - socket error
- Support more than eight watched sockets or provide multiple bounded wait sets.
- Keep accept non-blocking after a readiness notification.
- Distinguish timeout, would-block, EOF, reset, and internal errors.
- Support bounded read and write deadlines.
- Ensure listener close immediately prevents new accepts.
- Ensure socket ownership is capability-scoped to the daemon.

### Gates

- An idle listener blocks without repeated userspace polling.
- Idle CPU is negligible over a meaningful QEMU and VMware observation period.
- Listener bind conflicts return a specific error.
- Closing the listener releases the configured port.
- Read readiness wakes only when data, EOF, or error is available.
- Slow clients cannot create unbounded queued output.
- Multiple simultaneous connections progress independently.

---

## Phase 0.8: TCP Memory and Cleanup Correctness

### Goal

Eliminate continuous memory growth from repeated socket allocation and closure.

### Required Behavior

- Remove the current per-socket `Box::leak` ownership pattern.
- Keep smoltcp buffers valid without permanently leaking each allocation.
- Bound receive backlogs and reject or apply backpressure when full.
- Remove all socket state, SHM mappings, buffers, and queued data on close.
- Reap half-closed and abandoned sockets deterministically.
- Add socket allocation and release counters to diagnostics.

### Gates

- Ten thousand connect/close cycles do not continuously reduce free memory.
- Abrupt peer disconnect releases the socket.
- Failed connect and failed accept paths release all allocations.
- The maximum socket count remains enforced.
- Existing fetch, TLS, Solar, DNS, QEMU, and VMware networking tests pass.

---

## Phase 0.9: Secure Randomness Qualification

### Goal

Establish that host-key generation and SSH ephemeral key exchange receive
cryptographically suitable random bytes on every supported platform.

### Required Behavior

- Document the entropy sources used with and without CPU RDRAND.
- Replace predictable timestamp-only or weak fallback behavior.
- Make the kernel entropy call return a detectable failure when no approved
  entropy source is available.
- Propagate failure through `rand_service` and `getrandom`.
- Ensure cryptographic callers never silently select the non-cryptographic PRNG.
- Add startup health checks without logging generated bytes.

### Gates

- QEMU with RDRAND enabled produces successful secure-random requests.
- QEMU with RDRAND disabled either uses a reviewed entropy source or fails
  clearly and closed.
- VMware secure-random requests succeed.
- Killing `rand_service` causes crypto `getrandom` to fail, not downgrade.
- Host-key generation refuses to run when secure randomness is unavailable.

---

## Phase 0.10: Atomic Secret Storage

### Goal

Provide a reusable safe path for creating and replacing private service keys.

### Required Behavior

- Create files with restrictive permissions before secret content is exposed.
- Support exclusive creation or an equivalent race-resistant operation.
- Write to a temporary file in the destination directory.
- Close and validate the temporary file.
- Atomically rename it into place.
- Refuse symlink-like or unexpected target types if such filesystem objects are
  introduced.
- Preserve an existing valid key during failed replacement.
- Never log secret bytes.
- Define crash-consistency expectations while `fsync` is unavailable.

### Gates

- First creation results in a root-owned `0600` file.
- A failed write leaves no partially valid destination key.
- Concurrent creation attempts result in exactly one retained key.
- Restarting a test service reloads byte-identical key material.
- Unprivileged processes cannot read or replace the key.

---

## Phase 0.11: Strict Service Configuration

### Goal

Create a fail-closed, testable configuration parser suitable for
`/etc/sunlight/ssh.toml`.

### Required Behavior

- Parse the required top-level TOML fields:
  - `listen_address`
  - `port`
  - `host_key_file`
  - `password_authentication`
  - `max_auth_attempts`
  - `max_connections`
  - `max_sessions_per_connection`
  - `login_timeout_seconds`
- Reject duplicate fields.
- Reject unknown fields unless a documented compatibility policy says otherwise.
- Report the exact field and reason for malformed input.
- Reject port zero, invalid IPv4 addresses, invalid paths, zero limits, and
  values beyond documented maximums.
- Do not silently replace malformed security values with defaults.
- Keep service enablement out of the file.
- Read and validate the complete configuration before opening a listener.

### Gates

- Valid example configuration parses exactly.
- Every invalid field has a focused unit test and useful diagnostic.
- Missing configuration behavior is explicitly defined and tested.
- Malformed configuration prevents listener creation.
- Configuration changes take effect only after restart and this is documented.

---

## Phase 0.12: Runtime Compatibility Layer for a Maintained SSH Library

### Goal

Make a maintained SSH server implementation usable without forking its protocol
or cryptographic logic into a SunlightOS-specific SSH stack.

### Candidate

The current preferred candidate is `russh`, subject to a fresh dependency,
license, and algorithm audit when implementation begins.

### Required Investigation

- Pin a specific crate version and commit.
- Record licenses for the complete dependency tree.
- Identify every dependency on:
  - `std`
  - Tokio
  - `mio`
  - Unix sockets
  - libc
  - threads
  - filesystem APIs
  - wall and monotonic clocks
  - `getrandom`
- Determine whether the library can accept custom stream, timer, spawning, and
  randomness adapters without maintaining a large fork.
- Determine the minimum SunlightOS `std` and Tokio compatibility surface.
- Confirm server password authentication and session-channel callbacks.
- Confirm PTY request and window-change callbacks.
- Confirm configurable packet, window, and buffer limits.

### Stop Conditions

Stop before SSH implementation if:

- cryptographic primitives would need to be reimplemented locally
- the maintained library requires an impractical permanent fork
- unsupported runtime behavior cannot be adapted safely
- dependencies require OS facilities with no bounded replacement
- required algorithms cannot interoperate with a current OpenSSH client

### Gates

- A host-side adapter test drives the chosen library over an in-memory stream.
- A SunlightOS stream adapter implements the required async traits.
- Timers and socket readiness do not busy-poll.
- Secure randomness is routed only through approved `getrandom`.
- The dependency/license report is committed under `docs/ssh/`.
- A minimal server transport handshake succeeds before password or shell work is
  added.

---

## Phase 0.13: Service Logging and Diagnostics

### Goal

Provide useful daemon diagnostics without leaking credentials or producing
packet-level noise.

### Required Behavior

- Implement bounded service journal capture or define a reliable equivalent.
- Implement `sunlightctl` log retrieval or document the supported diagnostic
  command.
- Associate startup failure with the service status response.
- Rate-limit repetitive connection and authentication failures.
- Add explicit debug tracing flags that are disabled by default.
- Prohibit password, secret key, decrypted packet, and per-character logging.

### Gates

- Invalid configuration is visible through service status or logs.
- Bind conflict is visible through service status or logs.
- Startup, ready, stop, and unexpected-exit events are visible.
- Authentication failure logs contain no password properties.
- Normal interactive input does not create one log line per character or packet.

---

## Phase 0.14: Test and Measurement Harness

### Goal

Prepare automated and manual infrastructure that can prove the later SSH daemon
works and remains lightweight.

### Required Tooling

- QEMU networking setup that exposes a guest TCP port to the host.
- VMware networking instructions for reaching the guest.
- A host-side OpenSSH test script with isolated temporary `known_hosts`.
- A test user provisioned through the normal SunlightOS account path.
- Commands to query:
  - process resident memory
  - total free memory
  - process CPU/runtime counters
  - live socket count
  - live PTY count
  - live process/session count
- Repeated connection and teardown loops.
- Simultaneous-client test support.
- Serial-log collection and timeout handling.

### Measurement Method

For each measurement:

1. Boot the same image with the same VM memory and CPU settings.
2. Wait for a documented settling period.
3. Record at least three samples.
4. Report baseline, peak, and post-cleanup values.
5. Record exact QEMU or VMware configuration.
6. Never infer RAM or CPU usage from binary size.

### Gates

- The harness can detect whether a configured TCP port is open or closed.
- It can measure an idle daemon for at least 60 seconds.
- It can measure one unauthenticated and one authenticated session.
- It can run at least 100 connect/login/exit cycles.
- It can verify memory returns to a stable range after cleanup.
- It can run two simultaneous interactive clients.

---

## Dependency Order

Implement the phases in this order:

1. Central authentication service
2. Persistent sunlightd state
3. Reliable service lifecycle
4. Service identity and least privilege
5. Authenticated user-session spawning
6. PTY ownership and resize
7. Event-driven TCP server support
8. TCP memory and cleanup
9. Secure-randomness qualification
10. Atomic secret storage
11. Strict service configuration
12. Maintained SSH library runtime compatibility
13. Logging and diagnostics
14. Test and measurement harness

Some work may be developed in parallel, but the verification gates must preserve
this dependency order. In particular, SSH authentication and shell spawning must
not begin before Phases 0.1, 0.4, 0.5, and 0.6 pass.

## Cross-Subsystem Regression Gates

After every Phase 0 subsystem change:

- run focused host unit tests
- run `cargo check --workspace` when supported
- run `./tools/test.sh`
- boot QEMU and verify the existing serial gate
- boot VMware for changes involving network devices, timing, entropy, process
  lifecycle, or memory cleanup
- verify graphical Terminal tabs still create, accept input, render output, and
  close cleanly
- verify native TTY login and shell behavior
- verify fetch/TLS/Solar networking behavior
- verify service start, stop, restart, status, enable, and disable commands

Do not modify unrelated behavior merely to make a new gate pass.

## Definition of Ready for Sunlight SSH Phase 1

Sunlight SSH Phase 1 is ready to begin only when:

- password verification is centralized and hash-based
- no hardcoded or plaintext fallback authentication exists
- enablement is persistent and `sunlight-ssh` can default to disabled
- stop/restart accurately wait for daemon exit and cleanup
- least-privilege capability profiles are enforced
- an authenticated user shell can be spawned with correct UID/GID and PTY
- PTY window sizing and resize propagation work
- network waiting is event-driven and does not spin
- socket allocation and teardown are leak-free under stress
- secure randomness is qualified or fails closed
- private-key files can be created atomically with restrictive access
- SSH configuration parsing is strict and fail-closed
- a maintained SSH library has a documented, tested SunlightOS runtime path
- diagnostics expose startup failures without exposing secrets
- QEMU and VMware resource measurement tooling is available

Only after these conditions pass should the repository add:

- `services/sunlight-sshd/`
- the `sunlight-ssh` sunlightd unit
- `/etc/sunlight/ssh.toml`
- `/etc/sunlight/ssh_host_ed25519_key`
- SSH protocol integration and OpenSSH acceptance tests

## Deliverables Before Phase 1

- hardened authentication broker and migrated account database
- persistent sunlightd enablement state
- reliable lifecycle and readiness protocol
- enforced per-service identities and capability profiles
- authenticated spawn-as-user API
- resizable, owned, independently closable PTYs
- event-driven, leak-free TCP server APIs
- documented secure-randomness guarantee
- atomic private-key storage helper
- strict SSH configuration parser
- SSH library compatibility and license report
- QEMU and VMware regression and measurement harness
- updated architectural documentation for each changed subsystem

