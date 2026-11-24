# Helios to native Unix stream descriptor transfer

This page records the original single-byte global-page fixture.
[Fixed-size Helios shared pools](helios-fd-shm-pool.md) describes the
memfd support and the extension to 1–16 payload bytes per marked FD.

Linux x86-64 syscall 46 accepts a connected native-listener stream, a
56-byte `msghdr`, one `iovec` of length one, and one aligned
`SOL_SOCKET/SCM_RIGHTS` control record with `cmsg_len=20` and
`msg_controllen=20` or 24. No destination address is accepted. Only
`MSG_DONTWAIT` and `MSG_NOSIGNAL` are accepted; admission is always
nonblocking. Unsupported shapes, flags, ancillary types and object kinds
return `EOPNOTSUPP`; malformed control length or alignment returns
`EINVAL`; bad user pointers return `EFAULT`, missing FDs `EBADF`.
Control lengths above 24 are rejected as unsupported. The input
`msg_flags` field is ignored; flags come from the third syscall argument.
The endpoints must have stream read or write rights for their respective
operations, or the syscall returns `EACCES`. The bounded send path
returns `EPIPE` without generating a signal.

Only the existing fixed shared page is transferable. The ring owns a
reference to the open description with the original access rights. Closing
or reusing the sender FD does not affect it. The marked byte and reference
enter the ring together; a full ring or another pending reference returns
`EAGAIN`, and a closed receiver returns `EPIPE`. A failed send publishes
neither byte nor reference.

Ordinary reads stop before the marked byte and return `EOPNOTSUPP` at
that byte. Native `unix_stream_try_recv_fd` delivers the byte and one
close-on-exec descriptor together (syscall 185, stream FD in RDI; success
packs byte in bits 32–39 and FD in bits 0–31). There is no userspace
output buffer to truncate. An empty live stream returns `EAGAIN`;
ordinary data at the head or EOF returns `ENOMSG`. `EMFILE` leaves
both the byte and reference available for retry. Success consumes both
once. Sender close preserves queued data; final receiver close, including
closing an unaccepted connection, drops its queued reference.

The Helios syscall translation routes Linux 46 to `sys_linux_sendmsg`,
which snapshots the user headers and payload before acquiring the scheduler
lock. `FdTable::retain_shared_page` then retains the open description.
`pipe::send_right` publishes the reference and its byte under the same
lock used for stream I/O. `sys_native_unix_recv_fd` calls
`pipe::recv_right`, which installs the description in the recipient's
FD table before consuming the marked byte. Both operations refresh stream
readiness and wake existing pipe waiters after successful I/O.

Run from the repository root:

```sh
SUNLIGHT_TEST_ACCEL=kvm bash tools/test_fd_shared_mmap.sh scm-rights
bash tools/test_fd_shared_mmap.sh
bash tools/test_native_unix_listener.sh
```

The transfer runner builds two statically linked freestanding ELFs, stamps
the Linux ELF for Helios, and runs
`cargo build --package sunlight-kernel --features fd_shared_mmap_test`
with `SUNLIGHT_FD_SHARED_SERVER` and `SUNLIGHT_FD_SHARED_CLIENT` pointing
to those ELFs. It packages the resulting kernel with
`bash tools/make_hybrid_iso.sh target/x86_64-unknown-none/debug/sunlight-kernel target/native-scm-rights/guest.iso`.
The successful transfer boot used:

```sh
qemu-system-x86_64 -accel kvm -m 2048 -smp 2 \
  -device virtio-rng-pci,disable-modern=on \
  -cdrom target/native-scm-rights/guest.iso -display none -monitor none \
  -serial file:target/native-scm-rights/serial.log -no-reboot
```

The native receiver only mmaps FDs returned by the transfer syscall. A
separate stream carries coordination bytes so the receiver accepts the
transfer stream only after Helios closes and reuses the original sender FD.
The guest checks that ordinary reads stop at the marked byte, a second
pending send publishes nothing, and an exhausted receiver table can retry
without losing or redelivering the FD. Invalid FD lookup precedes reference
acquisition, and the receiver checks that invalid sends leave its stream
empty. For the discard case, Helios queues a final FD and closes its FD
and stream before the native receiver closes without consuming the marker.

From `target/native-scm-rights/serial.log`:

```text
[SCM-RIGHTS] invalid FD and malformed control rejected
[FD-SHARED] pid=712 fd=5 object=shared-page frame=0x1a356000 va=0x300000000
[FD-SHARED] pid=711 fd=255 object=shared-page frame=0x1a356000 va=0x300000000
[SCM-RIGHTS] sender FD closed before delivery; EMFILE retry preserved byte and FD
[FD-SHARED] pid=711 fd=6 object=shared-page frame=0x1a356000 va=0x300001000
[SCM-RIGHTS] observed mapping update and rejected writable mmap on read-only FD
[SCM-RIGHTS] dropped queued reference on receiver close
[SCM-RIGHTS] PASS
```

The same transfer ISO also reached both the cleanup and PASS markers under
TCG (`target/native-scm-rights/serial-tcg.log`).

The unchanged listener scenario passed under TCG
(`target/native-unix-listener/serial.log`). The original mmap scenario
exchanged both patterns under TCG but failed its final wait for Helios,
even though the kernel logged Helios exit code 0
(`target/fd-shared-mmap/serial-tcg-failure.log`). The same mmap ISO passed
under KVM, including the final read after Helios exit
(`target/fd-shared-mmap/serial-kvm.log`). The cause of the TCG wait outcome
was not established.

The fixed boot-owned page lets the fixture prove transfer ordering,
descriptor rights, and aliasing. It cannot prove reclamation or
last-reference ownership of dynamic backing, or full `wl_shm`
compatibility. Linux recvmsg, Smithay, SGP, independent multi-page
buffers, Chromium, and SunApps are outside scope.
