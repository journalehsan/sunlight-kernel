# deviced v0

`deviced` is the first SunlightOS device/driver manager. It is a ring-3
service, not a kernel subsystem, and registers with init's nameserver as
`deviced`.

## What v0 Does

- Tracks registered userspace drivers in a small fixed-size in-memory table.
- Tracks simple logical devices derived from driver registrations.
- Exposes register, state update, heartbeat, list, get, fail, and unregister
  operations over the existing register IPC path.
- Provides `devicectl` for basic inspection from the shell.

The v0 protocol is deliberately compact because current register IPC transports
four `u64` words. Driver names are short packed names, and capabilities are a
bitmask rendered by clients.

## Driver Registration

Drivers look up `deviced` through the nameserver and send
`DevicedMsg::REGISTER_DRIVER` with:

- short driver name
- current pid
- `DriverKind`
- initial `DriverState`
- capability bitmask

Registration is best-effort. If `deviced` is absent or slow, drivers log a
warning and continue booting. Current v0 registrations are:

- `keyboard`: `Keyboard`, `input|keyboard`
- `mouse`: `Mouse`, `input|pointer|relative-motion`
- `virtio`: `Virtio`, `virtio|bus|net`

## v0 Limits

- No hardware discovery or hotplug.
- No automatic driver restart.
- No dependency graph.
- No sandbox or permission policy enforcement.
- No long strings or device paths on the wire.

## Planned Extensions

- Hotplug events and dynamic device discovery.
- Dependency graph and ordered driver startup.
- Restart policy integrated with `sunlightd` or a dedicated driver supervisor.
- Driver sandbox policy and permission/capability model.
- Shared-memory protocol for richer metadata and long paths.
- Integration points for `networkd`, `powerd`, display server, and storage
  manager.
