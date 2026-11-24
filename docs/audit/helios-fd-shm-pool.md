# Fixed-size Helios shared pools

## Original TCG failure

Before changing the backing, booted the existing
`target/fd-shared-mmap/guest.iso` with TCG, two vCPUs, and the original
2048 MiB, RNG and serial options. The ISO hash is in
`target/fd-shared-mmap/replay-iso.sha256`. The original
`serial-tcg-failure.log` was left intact; the replay is
`serial-tcg-replay.log`.

The failure was native PID 711's final `waitpid(712)` assertion, after
the stage exchange and after Helios PID 712 read the native update.
Helios records exit code 0 at `process_mark_finished` (line 572), followed
by resource reap (line 575). Slot 4, which held PID 712, is reused for
`resolved` PID 722 at line 595, before PID 711 reports its wait failure
at line 817. The syscall looks up the child by PID and PPID on every call
and returns -1 when the row is gone; it has no separate exit receipt.
Slot reuse is consistent with losing the exit observation. The original
log did not record the individual syscall returns or whether the parent
parked before the child finished. The archive demonstrates coordination and
aliasing up to that wait; the final post-exit read was not reached.

The unchanged replay observed the Helios update and exit at lines
388–390, and passed the native post-exit read at line 400, before slot 4
was reused at line 496. No wait condition or timeout was relaxed.

The instrumented KVM regression reproduced the same sequence and recorded
the actual wait result: PID 711 parked in `waitpid`, PID 712 exited with
code 0 and was reaped, slot 4 was reused by PID 713, and PID 711's next
`waitpid` returned `no-child` (-1). See
`target/mmap-regression-kvm/serial.log`, lines 433–458. This establishes
the lost exit observation in the reproduced run. The original TCG log
has the same exit, reap, slot reuse and assertion ordering, but no
instrumented return. The pool fixture uses explicit transfer and release
stages; it neither changes the old wait assertion nor repairs waitpid.

## Client and backing choice

The target is the freestanding x86-64 Linux client in
[`fd-shm-pool.h`](../../tools/tests/fd-shm-pool.h). It tests the
`wl_shm.create_pool` request with a known, already-bound object ID. The
native fixture parses the 16-byte request and obtains the FD from ancillary
data. This exercises the pool wire request without registry setup, surface
creation, a Wayland library, or a compositor.

Run `python3 tools/tests/measure-fd-shm-pool.py` on Linux with GDB and
a C compiler. It builds both variants of the same client, records syscall
stops in `target/fd-shm-pool/host-{memfd,temp}.log`, and checks the request,
transferred FD, mapping contents, and a cross-process update.

| Operation | memfd variant | Temporary file variant |
|---|---|---|
| Create | `memfd_create(319, MFD_CLOEXEC)` | `openat(257, O_RDWR\|O_CREAT\|O_EXCL\|O_CLOEXEC, 0600)`, `unlinkat(263)` |
| Size and map | `ftruncate(77)`, `mmap(9, MAP_SHARED)` | Same |
| Deliver and release | `sendmsg(46, SCM_RIGHTS)`, `close(3)`, `munmap(11)` | Same |

The harness also observes `socket(41)`, `connect(42)`, the fixture's
`write(1)`/`read(0)` stage exchange, socket close, and `exit(60)`.
Numbers are Linux x86-64 syscall IDs. Regular Helios file operations use
the VFS route, which supplies neither the backing frames nor a shared page
cache for this mapping. A temporary file would require that additional work
and a writable directory. The memfd route gives the FD a kernel-owned
backing without adding a filesystem path. The original global-page endpoint
remains separate for the existing regression.

## Contract

- Creation grants read/write mapping and initial sizing rights to the fresh
  descriptor. Duplication and SCM_RIGHTS preserve the open description and
  its rights; native receipt sets close-on-exec. No name lookup can reopen
  or discover a pool. The name is validated but is not retained.
- `memfd_create` accepts only flags 0 or `MFD_CLOEXEC`. Unsupported flags
  or a name longer than 249 bytes give `EINVAL`; inaccessible name bytes
  give `EFAULT`. A full descriptor table gives `EMFILE`.
- `ftruncate` sets a positive logical size once. Same-size calls succeed;
  changing the established size returns `EPERM`. Zero and negative sizes
  give `EINVAL`, sizes over 1 MiB give `EFBIG`, a missing FD gives
  `EBADF`, and an unsupported FD or insufficient rights give
  `EOPNOTSUPP` or `EACCES`, respectively.
- The first sizing eagerly allocates zeroed pages, rounded up to 4 KiB.
  The guest-wide limits are 128 objects and 4096 live pages (16 MiB),
  including objects retained by a map or a queue. There is no per-user
  reservation. Allocation or quota failure gives `ENOMEM`; partial
  frame allocation is rolled back.
- `MAP_SHARED` takes a zero address hint, positive length, and page-aligned
  offset; offset plus length must fit inside the logical size. The last
  page is mapped at page granularity. Read-only and read/write views are
  supported, always NX. Wrong ranges and protections give `EINVAL`,
  insufficient FD rights give `EACCES`, and an unsupported FD or missing
  FD gives `ENODEV` or `EBADF`, respectively. Native mmap reports its
  generic failure result. Whole-view `munmap` removes the PTEs and waits
  for invalidation before dropping the mapping reference. Partial unmap,
  permission changes, and replacement are protected. Address-space
  teardown also drops mapping references after detaching the leaves. Pool
  views use the existing shared address range; unmap does not recycle
  addresses within that range.
- The last open-description reference makes the backing eligible for
  reclaim. The PMM drain returns its frames and releases the quota.
  Descriptors, queued SCM_RIGHTS, and mappings each hold references.
  Closing a receiver with an unread right releases the queued reference.

The transfer accepts one FD, one control record, one iovec of 1–16 bytes
and at most one pending marked record per stream. Native receipt consumes
the marked byte and the FD; ordinary reads consume the remaining bytes.
Pool resize, sealing, byte I/O and metadata for memfds, general Linux
recvmsg, batched FD sends and full Wayland client behavior are outside the
contract. No Wayland server, Smithay, SGP, Chromium, or SunApp was added.

## Guest runs

```sh
SUNLIGHT_TEST_ACCEL=kvm bash tools/test_fd_shared_mmap.sh pool
SUNLIGHT_TEST_ACCEL=tcg bash tools/test_fd_shared_mmap.sh pool
SUNLIGHT_TEST_ACCEL=kvm SUNLIGHT_TEST_OUTPUT="$PWD/target/scm-rights-regression-kvm" bash tools/test_fd_shared_mmap.sh scm-rights
SUNLIGHT_TEST_ACCEL=tcg SUNLIGHT_TEST_OUTPUT="$PWD/target/scm-rights-regression-tcg" bash tools/test_fd_shared_mmap.sh scm-rights
SUNLIGHT_TEST_ACCEL=kvm SUNLIGHT_TEST_OUTPUT="$PWD/target/mmap-regression-kvm" bash tools/test_fd_shared_mmap.sh
SUNLIGHT_TEST_ACCEL=tcg SUNLIGHT_TEST_OUTPUT="$PWD/target/mmap-regression-tcg" bash tools/test_fd_shared_mmap.sh
SUNLIGHT_TEST_ACCEL=tcg bash tools/test_native_unix_listener.sh
```

The pool runner compiles two freestanding ELFs, stamps the Helios ELF,
sets `SUNLIGHT_FD_SHARED_{SERVER,CLIENT}`, runs
`cargo build --package sunlight-kernel --features fd_shared_mmap_test`,
and invokes `tools/make_hybrid_iso.sh` with the resulting kernel.
It boots QEMU with `-accel kvm` or `-accel tcg`, `-m 2048 -smp 2`,
`-device virtio-rng-pci,disable-modern=on`, and the pool ISO as `-cdrom`.
Guest serial and QEMU logs are `target/fd-shm-pool/serial-{kvm,tcg}.log`
and `qemu-{kvm,tcg}.log` in the same directory.

The guest maps 8192- and 4096-byte objects, writes and checks their
distinct patterns across the 4096-byte boundary, and maps a second-page
alias using a nonzero offset. Both sender FDs close before their delivery;
only the received FDs are used to map. After the receiver closes its FDs,
both mappings still see updates and retain independent contents. The runner
requires the PMM reclaim markers for both normal releases (three total
frames) before the discard case starts. For the latter it requires
the receiver-close marker and a second reclaim to zero live frames for
the discarded two-page backing. A process returning zero is insufficient
to pass this check.

| Two-vCPU fixture | KVM | TCG |
|---|---|---|
| Independent pool transfer and reclaim | Pass | Pass |
| Original single-FD SCM_RIGHTS | Pass | Pass |
| Original global-page mmap | Failed at final wait: `no-child` after slot reuse | Pass, including final read |
| Native Unix listener | Pass | Pass |

The original SCM_RIGHTS and global-page regression logs are in the output
directories named in the commands. The listener TCG log is
`target/native-unix-listener/serial.log`; its KVM replay used the same ISO
and boot options with `-accel kvm`, `serial-kvm.log` and `qemu-kvm.log`.
