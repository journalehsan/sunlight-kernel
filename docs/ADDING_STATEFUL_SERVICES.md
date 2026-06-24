# Adding a State-Writing Service

How to let a new OS service persist files on the otherwise-immutable root,
without UAC prompts and without a capability broker. This complements
[`FILESYSTEM_SECURITY.md`](./FILESYSTEM_SECURITY.md), which describes the overall
write policy.

## The model in one sentence

The kernel identifies a process by its **kernel-assigned name** and lets a
*service* write only under its **own** `/state/<service-name>` (or
`/var/lib/<service-name>`) directory — everyone else is denied, and no UAC or
token exchange is involved.

This works because the name is assigned by the kernel at spawn from the ELF path,
so a process cannot impersonate another service. The kernel *is* the
authorization chokepoint; there is no runtime broker in this path.

### Authorization flow

```
service calls SYS_OPEN/SYS_MKDIR (write)
        │
        ▼
kernel sys_open / sys_mkdir            kernel/src/arch/x86_64/syscall.rs
        │   resolves the caller to an Actor
        ▼
current_fs_actor() → Actor::Service{ name }   (name-based)
        │
        ▼
sunlight_fs::can_write(actor, path, op)         sunlight-fs/src/policy.rs
        │   service_decision(): allow iff
        │   service_state_owner(path) == name
        ▼
allow  →  reason = AllowedServiceState
```

`service_state_owner("/state/sunlight-kv/kv.store")` returns `"sunlight-kv"`; if
that equals the calling service's name, the write is allowed.

## Checklist: add a new stateful service `my-svc`

Three changes are required. The path segment, the initramfs dir, and the actor
name **must all be the exact same string** as the process name.

### 1. Pre-create the state directory in the image

Add a `RamEntry::dir` to `INITRAMFS` in `sunlight-fs/src/ramfs.rs` (next to the
existing `/state/sunlight-kv` etc.):

```rust
RamEntry::dir("/state/my-svc", 0, 0, mode::DIR_700),
```

`DIR_700` (root-only) is fine because services run as root; the *policy* (not the
unix mode) is what authorizes the write. Pre-creating avoids needing to `mkdir`
the immutable parents (`/state`, `/var/lib`) at runtime.

### 2. Map the process name to a Service actor

In `current_fs_actor()` in `kernel/src/arch/x86_64/syscall.rs`, add an arm to the
name match (this is also factored into `fs_actor_for`):

```rust
"my-svc" => sunlight_fs::Actor::Service { name: "my-svc" },
```

**Without this arm the process is treated as a normal `User` and denied.** The
literal must be `'static` (it appears in the `Actor<'static>` returned).

### 3. Open/create the file correctly in the service

Follow the `sunlight-kv` pattern (`sunlight-kv/src/main.rs`,
`open_or_create_store`). Plain `libc::open` is **read-only and does not create**:

```rust
const STORE: &[u8] = b"/state/my-svc/data";

fn open_store() -> Option<Fd> {
    // One fd for both read (recovery) and append → must be O_RDWR.
    if let Ok(fd) = libc::open_with_flags(STORE, libc::O_RDWR) {
        return Some(fd);          // existing file, opened in place (not truncated)
    }
    let _ = libc::create(STORE);  // first boot: create, then reopen read/write
    libc::open_with_flags(STORE, libc::O_RDWR).ok()
}
```

Never pass `O_CREAT` to an already-existing data file you want to keep — the
kernel's create path goes through `create_file` and will not preserve prior
contents.

## What this does NOT grant

- Writing another service's state (`/state/other-svc`) — denied.
- Writing protected/immutable paths (`/etc`, `/bin`, `/boot`, `/sbin`, `/kernel`,
  `/services`, `/proc`, `/sys`) — denied even for services.
- A process that is **not** mapped in step 2 — treated as a `User`, so it only
  gets `/tmp` and its `/home/<user>` (and `/root`, `/home` for root).

If a service genuinely needs to write outside its own state (e.g. editing
`/etc/resolv.conf`), that is a *different* problem — relocate the file under the
service's `/state` (or `/run`), or add an explicit, reviewed policy exception in
`sunlight-fs/src/policy.rs`. Do not widen `service_state_owner`.

### Special case: generated compatibility files

Some legacy paths are compatibility views rather than normal writable state.
`/etc/resolv.conf` is the current example:

- `resolved` owns DNS state in memory.
- `vfs_server` materializes `/etc/resolv.conf` from `resolved` via
  `ResolvedMsg::RENDER_RESOLV_CONF`.
- `sunlight-fs/src/ramfs.rs` still seeds a small `/etc/resolv.conf` entry so
  `stat`, `ls`, and early `cat` work before `vfs_server` refreshes it.
- User tools must call `resolvectl`; direct writes to `/etc/resolv.conf` remain
  denied in v0.

Use this pattern only for reviewed compatibility facades. Do not make `/etc`
broadly writable for a service.

## Relationship to the capability broker (currently dormant)

The kernel has a capability-mint path (`GrantCapability` syscall →
`capability::sys_grant_capability`, gated on a process named `capability-broker`)
and the write syscalls consult per-process scoped capabilities as an *additive*
fallback (`vfs_allows_for_pid`). **No `capability-broker` service is running
today**, so nothing mints capabilities and this path is inert. Plain service
state (the mechanism above) does **not** need it.

> Note: `FILESYSTEM_SECURITY.md` still states that `sunlightd` starts
> `/sbin/capability-broker` before stateful services. That broker service was
> rolled back; it is not started. Treat the broker sections there as a design
> sketch for a *future* bite, not current behavior.

A real broker would only be worth building if a service needed a write grant that
identity-based policy can't express — e.g. a user process getting a *temporary,
UAC-approved* scoped write outside its home. The intended shape is
`requester -> capability-broker -> sunlight_uac -> decision -> minted capability`,
where the minted token is scoped (subject, rights, path) and validated through
`sunlight_fs::broker_mint_fs_capability`.

## Build gotchas

- **`include_bytes!` staleness.** The kernel embeds service ELFs via
  `include_bytes!` in `kernel/src/main.rs`. Cargo may not rebuild the kernel when
  only a service binary changed, leaving a stale embedded copy. Force it:
  `touch kernel/src/main.rs` before building the kernel.
- **QEMU disk lock.** If a run is killed (e.g. by `timeout`), QEMU can leave the
  qcow2 write-lock held and the next run fails with "Failed to get write lock".
  Clear it with `pkill -9 -f qemu-system-x86_64`.

## Verify

Boot (`tools/run.sh --no-display`) and look for the service writing its own state:

```
[SUNLIGHT-FS] decision actor=Service { name: "my-svc" } op=Create path=/state/my-svc/data result=allow reason=AllowedServiceState
[SUNLIGHT-FS] decision actor=Service { name: "my-svc" } op=Write  path=/state/my-svc/data result=allow reason=AllowedServiceState
```

A `result=deny reason=DeniedMissingCapability` on a `/state/...` path almost
always means step 2 (the actor-name mapping) is missing or the name doesn't match
the directory segment.
