# Native descriptor replacement identity (dup2/dup3)

This document closes the `dup2` limitation recorded in the 2026-11-12/13
native TCP epoll reports. It defines what a descriptor replacement does to
the native TCP transport attachment and to kernel epoll registrations. It
does not add Linux open-file-description epoll semantics and it does not add
`dup`/`fork` transport attachment transfer.

Evidence: `docs/audit/probes/native-foundation/results-2026-11-13-native-dup2-replacement-identity/`.

## Reachability through the C ABI

The native C surface has **no descriptor duplication entry point**. The
shipped archive exports `close`, `read`, `write`, `epoll_create1`,
`epoll_ctl` and `epoll_wait` for this area, but no `dup`, `dup2` or `dup3`,
and `sunlight-libc/include/unistd.h` declares none of them. A C program that
declares `dup2` and calls it fails at link time (`undefined symbol: dup2`);
the negative probe is retained in the evidence directory.

Replacement is nevertheless reachable, and is therefore specified:

- the guest syscall ABI exposes `dup2` (46) and Linux `dup3`, so a native
  program that issues the syscall directly replaces a descriptor without
  passing through this crate;
- `sunlight_libc::dup2(Fd, Fd)` exists as the internal native Rust entry
  point;
- the Linux personality maps Linux `dup2`/`dup3` onto the same kernel
  replacement path (no userspace transport map applies there).

For a bridged TCP descriptor every one of those paths is **fail closed**.
There is no supported way to transfer a transport attachment to a duplicate,
and none is introduced here.

## Ownership and lifetime contract

| Layer | Owner | Identity |
| --- | --- | --- |
| Guest descriptor number | kernel fd table | index in the process descriptor table |
| Descriptor-table entry | kernel fd table | open object handle + shared description |
| Inet endpoint | `kernel/src/process/inet_socket.rs` | pool slot plus monotonic generation |
| Epoll registration | `kernel/src/process/epoll.rs` | epoll instance + fd key + fresh token + optional external net identity + raw 8-byte data |
| Transport attachment | `sunlight-libc/src/net_bridge.rs` | fd key → (`net_server` socket id, kernel endpoint generation) |

The kernel endpoint generation is the only identity that both layers can
observe. It changes whenever a slot is reused, so it distinguishes "the same
fd number" from "the same endpoint".

### Replacement ordering (kernel)

`sys_dup2` and Linux `sys_linux_dup3` run under `SCHEDULER`, which is also
the lock used by `close`, `epoll_ctl` and readiness collection:

1. Validate exactly as `FdTable::dup2`/`dup3` does (`old_fd == new_fd`
   short-circuit, `new_fd` in range, source present). Nothing is destroyed
   yet, so an invalid source or invalid target leaves the target's
   registration and attachment untouched and returns `EBADF`.
2. `epoll::detach_fd(new_fd, process)` removes the replaced fd key from every
   epoll instance held by that descriptor table. This is the linearization
   point for registration identity: a registration describes the descriptor
   identity that existed before the call, and a replaced identity has none.
3. `FdTable::dup2`/`dup3` installs the source handle at `new_fd` and returns
   the displaced handle, which is released after dropping `SCHEDULER`.

Consequences: after a successful replacement `EPOLL_CTL_DEL` of the target
reports `ENOENT`, and a later `EPOLL_CTL_ADD` assigns a fresh token. No event
carrying the old token or the old `event.data` can be produced for the new
identity, whether the new object is local or a different inet endpoint.

### Replacement ordering (libc transport attachment)

`sunlight_libc::dup2` owns the attachment map:

1. `dup2(fd, fd)` is forwarded to the kernel unchanged; the map is not
   touched.
2. The target's attachment is removed and captured.
3. The kernel replacement runs. If it fails, the captured attachment is
   re-installed, because the kernel rejects the call before replacing
   anything.
4. On success the captured `net_server` socket is closed. The duplicate is
   left **without** a transport attachment, matching the documented `dup`
   rule that attachments are not copied.

The attachment is detached before the syscall so the failure window is fail
closed (`ENOTCONN`), never "still routed to the replaced endpoint".

### Bridge map invariant

A mapping is valid only while the descriptor still names the kernel inet
endpoint recorded when the mapping was installed:

- `install` records the kernel endpoint generation (queried before taking the
  table lock);
- `lookup` returns the attachment only when the live generation still
  matches, so a rebound fd number cannot read, write, poll or register
  against the replaced endpoint;
- `close` reads the recorded attachment without the identity check and then
  clears it, so the displaced endpoint is still released exactly once;
- `socket()` clears any mapping for a reused number, and `close` clears the
  mapping it consumes.

The identity query is a non-blocking syscall (the existing mode-3 generation
query of the native epoll bridge), performed only when a mapping exists. No
lock is held across it, and no lock is held across a `NetOp` IPC.

## Supported behaviour

- Local descriptors keep ordinary local semantics after any replacement.
- A replacement that displaces a local descriptor leaves the new descriptor
  usable for its new local object.
- The source descriptor of a replacement keeps its own identity, transport
  attachment and readiness.
- `dup2(fd, fd)` is a no-op; `dup2` with an invalid source or an
  out-of-range target returns `EBADF` and destroys nothing.
- Repeated replacements are allowed; each replacement invalidates the fd key
  registration and each later `ADD` gets a fresh token.
- Closing a replaced descriptor releases the attachment recorded for it, so
  `net_server` socket/listener/stream counters return to zero.

## Explicitly unsupported

- **No C duplication API.** `dup`, `dup2` and `dup3` remain undeclared and
  unexported for C. `dup()` does not copy a transport attachment, so a
  duplicate of a bridged stream fails closed.
- **No Linux open-file-description epoll lifetime.** Registrations are keyed
  by fd number. Linux keeps an interest on the old description after
  `dup2`; this implementation ends the interest instead, and does not
  promise events for an old description that another descriptor keeps open.
  `dup`/`fork`/`exec` attachment transfer remains unsupported.
- **No atomicity between concurrent replacement and an in-flight bridged
  operation on the same fd.** The kernel replacement is atomic with respect
  to registration mutation and collection. A userspace operation that
  resolved the attachment before the replacement may complete against it;
  every later operation fails closed. This is the same class of
  non-atomicity as concurrent `close`, which was already unspecified.
- **Linux-personality AF_UNIX `connect` handle replacement**
  (`FdTable::replace_handle`) is a different, non-TCP replacement path; it
  is not covered by this contract and is recorded as an audit finding.
- `install_at`/`install_from` during spawn target a freshly constructed child
  descriptor table whose only entries are the standard streams, so no
  registration can be displaced there.
