# Native listener for Helios AF_UNIX streams

The Linux client uses the existing `socket(AF_UNIX, SOCK_STREAM, 0)`
and `connect(sockaddr_un)` translation. The kernel resolves the name in
its internal Unix socket table. Both connected descriptors use the same
two pipe rings as `NativeSocketPair`; existing read, write and close
operate on those descriptors. Native IPC endpoints are a separate transport.

The native API has two operations: register a listening descriptor in one
call, and try to accept one stream. It does not expose Linux dispatch IDs,
an intermediate bound state, peer addresses or ancillary data.

Registration uses a byte name under the reserved prefix
`/sunlight/native-unix/`, with a nonempty suffix, no NUL bytes and at most
107 bytes total. The name is matched byte for byte. There is no VFS node,
path normalization, directory permission check or unlink operation.

Only a native process with kernel UID 0 can register. Linux bind cannot
claim this prefix even while a name is vacant. A second authorized registrar
cannot replace a live registration. UID 0 is the trusted registration
authority; this API does not distinguish between different UID-zero native
services. The listener descriptor owns the registration. Its final close
frees the name and drops queued streams; already accepted streams retain
their own descriptor lifetime.

The registrar selects one allowed UID. The kernel compares it with the
connecting process's current UID before queue admission. A mismatch returns
`ECONNREFUSED`, as does an absent listener. There is no UID-zero connect
exception. Processes with the same UID share access across sessions and seats;
this is UID isolation. No user-supplied peer identity or socket name is proof
of authorization.

The queue is bounded at 128. Try-accept returns `EAGAIN` when the queue is
empty; a full queue returns `EAGAIN` to connect. The listener and accepted
descriptors are close-on-exec. The API returns an FD or a negative errno.

The public Rust wrappers are `sunlight_libc::socket::unix_stream_listen`
and `unix_stream_try_accept`. At the native syscall boundary,
`NativeUnixListen` (183) takes the name pointer, byte length and allowed UID
in RDI, RSI and RDX. `NativeUnixAccept` (184) takes the listener FD in RDI.
Native dispatch rejects the internal Linux socket handler numbers 1050–1058.
The fixture uses the documented native syscall ABI directly.

Run from the repository root:

```sh
SUNLIGHT_TEST_ACCEL=kvm bash tools/test_native_unix_listener.sh
```

Omit the environment override to use TCG. The runner builds separate static
native and Helios ELFs, compiles the debug fixture kernel, packages a Limine
ISO, and boots it in QEMU. It requires the client and server serial markers
and saves `build.log`, `pack.log`, `qemu.log` and `serial.log` in
`target/native-unix-listener/`.

This change adds no dependency or third-party license.

The two-vCPU KVM run passed. The native process received `Q` and returned
`R`. The UID 1001 client observed `ECONNREFUSED` while the listener was
live, and the native server observed `EAGAIN` on its still-empty queue.
After closing the listener, the UID 1000 client observed `ECONNREFUSED`
on a new connection and reported it over the already accepted stream.
The server then registered the same name again. The fixture also checked
duplicate registration, nonroot native registration and Linux bind denial.
