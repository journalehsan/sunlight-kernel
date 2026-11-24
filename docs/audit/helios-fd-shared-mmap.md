# FD-backed shared page for Helios and native processes

For independent multi-page pools and the TCG replay, see
[Fixed-size Helios shared pools](helios-fd-shm-pool.md). This page records
the original global-page fixture.

The MVP exposes one zero-initialized, fixed 4096-byte kernel memory object at
`/dev/sunlight-shared-page`. It is a virtual open endpoint for UID 0 callers
on the kernel file route. Regular VFS descriptors resolve to filesystem
handles for byte I/O; they do not supply a stable, shared physical frame.
The existing shared-memory allocator supplies frames through capability tokens
rather than FDs. The minimal bridge is a named FD object that uses the existing
shared page-table mapper. It requires no regular-file page cache, new allocator
syscall, descriptor transfer, or changes to the Unix listener.

Both ABIs independently open the same path. Native `sys_open` and Linux
`sys_linux_openat` (also used for Linux `open`) reach
[`open_resolved_path`](../../kernel/src/arch/x86_64/syscall.rs), which installs
a local FD with a [`FileHandle::SHARED_PAGE`](../../kernel/src/process/fd_table.rs)
and access rights derived from `O_RDONLY` or `O_RDWR`. The only optional
open flag is `O_CLOEXEC`. The object is mmap-only.

[`translate_mmap_flags`](../../compat-linux/src/lib.rs) accepts exact
`MAP_SHARED`; `sys_linux_mmap` and native mmap both reach
[`map_shared_fd`](../../kernel/src/process/mmap.rs). It checks the descriptor,
handle kind and rights before selecting the object frame.
[`map_shared_region_protected`](../../kernel/src/process/address_space.rs)
installs that frame directly in the caller's address space with NX permissions.
The first successful map initializes the page; the kernel owns the frame for
the rest of the boot. Closing descriptors or tearing down either address space
does not free or clear the object. The frame is retained across subsequent
opens. Views use the existing owner-managed policy: userspace cannot change
permissions, replace or explicitly unmap the view; address-space teardown
removes it. There is no resize operation.

The accepted map has address 0, length 4096, offset 0, and protection
`PROT_READ` or `PROT_READ | PROT_WRITE`. Helios returns `EACCES` for
insufficient rights, `EINVAL` for invalid arguments, `EBADF` for a missing
descriptor, and `ENODEV` for an unsupported descriptor type. Native mmap
uses its existing generic `-1` failure result. Validation rejects requests
before installing any PTE or ledger entry.

Run from the repository root:

```sh
bash tools/test_fd_shared_mmap.sh
bash tools/test_native_unix_listener.sh
```

The first runner compiles two static ELFs from
[`tools/tests/fd-shared-mmap.c`](../../tools/tests/fd-shared-mmap.c), stamps the
Helios ELF, runs `cargo build --package sunlight-kernel --features fd_shared_mmap_test`
with the fixture ELF paths, and packages the kernel with
`bash tools/make_hybrid_iso.sh`. Both runners boot QEMU with
`-accel tcg -m 2048 -smp 2 -device virtio-rng-pci,disable-modern=on`,
their respective ISO as `-cdrom`, and serial output sent to a file.

The two-vCPU guest run passed. The kernel logged the same object and physical
frame for native PID 711 and Helios PID 712, with separate local FDs. Both
rejected writable mmap through a read-only FD, misaligned and out-of-bounds
offsets, and an oversized range. Their ledger counts remained at four before
the first successful map, which used the initial shared VA. Both closed their
backing FDs before exchanging patterns. The Unix listener carried only one-byte
stage messages. Helios verified the native update, exited, and the native
parent waited for its zero exit status before verifying the final pattern.
The unchanged listener regression also passed.

Guest logs: `target/fd-shared-mmap/serial.log` and
`target/native-unix-listener/serial.log`.
