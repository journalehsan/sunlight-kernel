# Graphical terminal per-tab session lifecycle

This document records the native lifecycle contract for a graphical
`sunlight-terminal` tab. It covers the PTY broker, the native shell, kernel
execution/session state, and cleanup after tab or application closure.

## Root cause

The graphical tab already created a generation-qualified PTY and attached its
shell as the authenticated session controller. Closing the tab sent
`CLOSE_SESSION`, which reached the kernel's trusted foreground operation. That
operation cancelled the shell's current foreground execution scope and forced
the controller to exit, but it did not terminate members that carried the
session identity without belonging to that temporary scope. An idle shell had
no active scope, and a descendant created outside a foreground command could
therefore survive. PPID changes made no difference because the old teardown
did not use the persistent session membership.

## Ownership model

Each tab owns one `PtySession { id, generation, master, control }`. The PTY
broker records the creator PID and its kernel publication generation. The
shell receives a delegated slave capability and binds itself with
`BIND_FOREGROUND_OWNER`. The kernel then records:

```
tab -> PTY id/generation -> authenticated shell controller
    -> inherited broker/session/generation membership
    -> optional temporary foreground execution scope
```

The session tuple is copied when a process is spawned, so it survives exec,
ordinary reparenting, and intermediate parent exit. A numeric PID, process
name, PPID, or session number without the broker publication and generation is
not an authority.

## Creation and close

Tab creation is the existing bounded state machine: create the PTY, set its
mode, delegate the slave, spawn `/bin/sshl<tab-id>`, then register that shell
as the session foreground controller. A tab close removes the tab's input
route, closes its control-authorized PTY session, and cancels any in-flight
spawn targeted at that tab. Closing the last tab requests application/window
close after the same session teardown has started.

`CLOSE_SESSION` is idempotent at the broker boundary. It invalidates the
generation-qualified PTY handles, clears the kernel TTY geometry, releases
the broker's endpoint authorities, and requests full session termination.
Foreground ETX (`Ctrl+C`) remains a separate operation: it interrupts only
the active foreground scope and leaves the shell and other session members
alive.

## Full session termination

The trusted PTY broker calls the kernel session operation with the exact
broker publication, PTY session ID, generation, and hangup signal. The kernel
first cancels active foreground scopes, then marks every live process carrying
that exact session tuple for forced exit. It wakes blocked members and
cancels pending IPC calls through the existing generation-qualified IPC path.
The scheduler performs descriptor, address-space, IPC, and shared-resource
reclamation through its normal reaper. No PPID scan or arbitrary PID kill is
used for session ownership.

Detached processes are not provided by the current native shell language.
Adding an intentional detach operation would require a separate authenticated
capability and explicit policy; a process cannot detach merely because its
parent exits.

## Crash recovery

The PTY broker polls its endpoint with a 50 ms maintenance deadline. For every
live session it checks the creator PID and captured process publication
generation using the trusted generation-qualified liveness syscall. If the
terminal application dies without sending `CLOSE_SESSION`, the broker performs
the same full session termination, releases deferred PTY waiters, clears TTY
geometry, and destroys the session slot. PID reuse cannot satisfy this check.

## Security boundaries

Control, master, and slave operations continue to require their role-specific
opaque capability, caller PID, caller generation, PTY ID, and PTY generation.
The kernel session operation is callable only through the trusted PTY broker
path. The generation-qualified liveness extension is also restricted to that
broker. Stale tab handles, stale broker publications, reused PIDs, other PTY
sessions, legacy TTY sessions, and unrelated services remain outside the
operation's match set.

## Verification

Host evidence for this change:

- the production foreground-state model passes the idle, reparented-member,
  stale-identity, and cross-session tests;
- `pty_server` host tests pass (11 tests);
- `sunlight-terminal` host tests pass (28 tests);
- native-target checks pass for `sunlight-ipc`, `sunlight-kernel`,
  `pty_server`, and `sunlight-terminal`.

The native QEMU matrix still needs a fresh run against this revision for
interactive tab close, application death, and 1/2/4 CPU resource baselines.
Until those runs are captured, this document does not claim a measured GUI
latency, heap baseline, or complete crash-recovery result in a live guest.

## Known limitations

The graphical client still uses a bounded synchronous PTY close call; it is
not an unbounded event-loop wait, but a delayed broker reply can consume that
call's timeout budget. The shell's supported language does not include an
intentional detached background process, and complete POSIX job-control
semantics remain outside this lifecycle contract. Helios tasks inherit native
session membership and forced teardown, while Helios signal-handler semantics
remain distinct from native cancellation.
