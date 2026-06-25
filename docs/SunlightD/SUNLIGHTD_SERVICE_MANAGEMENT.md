# SunlightD Service Management

**Last updated:** 2026-06-26
**Status:** In-memory service control working. Persistence is a future item.

---

## Overview

SunlightD (`sunlightd`) is the Ring-3 service supervisor for SunlightOS. It sits in
the third tier of the boot stack, below the kernel and init/initd layers that it must
never touch.

```
Tier 1 – Kernel-spawned (Ring 0 → Ring 3, fixed PIDs)
  init (pid 1)  ·  timer_server (pid 2)  ·  vfs_server (pid 3)
  tty_server (pid 4)  ·  net_server (pid 5)

Tier 2 – Init-spawned early services
  sunlightd itself is spawned here (currently by init after net_server)

Tier 3 – SunlightD-managed services  ← this document
  timezone_service · niced · gcd · uac_service · sunlight-sm
  sunlight-kv · rand_service · sunlight-tls · solar (disabled by default)
```

SunlightD is the correct place to add, remove, enable, or disable optional
user-space services. **Never move Tier 1 or Tier 2 services into SunlightD
without understanding the full boot dependency chain.**

---

## Current Service Registry

Services are defined as inline unit strings in `services/sunlightd/src/main.rs`
inside `load_units()`. There are no on-disk unit files yet.

| Service | Binary | Default Enabled | Why |
|---|---|:---:|---|
| timezone_service | `/sbin/timezone_service` | ✓ | System clock / TZ lookups |
| niced | `/sbin/niced` | ✓ | Process nice-priority management |
| gcd | `/sbin/gcd` | ✓ | Generic control daemon |
| uac_service | `/sbin/uac_service` | ✓ | User Access Control broker |
| sunlight-sm | `/sbin/sunlight-sm` | ✓ | Storage Manager (whitelist writes) |
| sunlight-kv | `/sbin/sunlight-kv` | ✓ | Persistent key-value store |
| rand_service | `/sbin/rand_service` | ✓ | ChaCha20 CSPRNG — **required by TLS** |
| sunlight-tls | `/sbin/sunlight-tls` | ✓ | TLS over IPC (rustls) |
| solar | `/sbin/solar` | ✗ | HTTP server — optional, saves ~RAM |

> **Never disable** `rand_service`, `sunlight-kv`, or `sunlight-tls` by default.
> They are required for TLS, certificate handling, and secure networking. The
> `rand_service` in particular is not just convenient — TLS handshakes pull
> randomness from it via the getrandom IPC path.

---

## IPC Protocol

All control goes through the SunlightD nameserver endpoint registered as
`"sunlightd"`. The IPC transport carries **words[0..4] only** (32 bytes).
Words[4..7] are silently dropped by the kernel ABI — this is a confirmed
hardware constraint, not a software bug.

### Opcodes

| Op | Label | Direction | Description |
|---|---|---|---|
| Start | 1 | ctl → d | Spawn a service (even if disabled) |
| Stop | 2 | ctl → d | SIGTERM + mark stopped |
| Restart | 3 | ctl → d | Stop then Start |
| Reload | 4 | ctl → d | No-op (unit files are compiled-in) |
| Enable | 5 | ctl → d | Set `enabled = true` |
| Disable | 6 | ctl → d | Set `enabled = false` |
| Status | 10 | ctl → d | Query a single service |
| List | 11 | ctl → d | Paginated service enumeration |
| GetLog | 20 | ctl → d | Stub — not implemented |

### Reply Codes

| Label | Meaning |
|---|---|
| 1 (`REPLY_OK`) | Operation succeeded |
| 2 (`REPLY_NOP`) | Already in desired state (enable/disable no-op) |
| 0xff (`REPLY_ERR`) | Not found or spawn failed |

### Unit Name Encoding (request)

Service names are packed little-endian into `words[0..4]`, one byte per bit-lane,
NUL-terminated. Maximum 32 bytes (covers all current service names).

### List Entry Encoding (reply, words[0..4])

```
words[0]  bits  0–31   total service count (u32)
          bits 32–39   service state (u8):  0=stopped 1=starting 2=running
                                            3=failed  4=restarting
          bit  40      enabled flag (0/1)
          bits 48–55   restart count, clamped to u8
words[1]  bits  0–31   PID (0 when not running)
words[2]  bytes 0–7    service name, little-endian
words[3]  bytes 8–15   service name continued
```

### Status Reply Encoding (words[0..4])

```
words[0]  state (u32)
words[1]  pid   (u32)
words[2]  restarts (u32 low) | enabled (bit 32)
words[3]  started_at (u64, or 0 if not tracked)
```

---

## sunlightctl Reference

`sunlightctl` is the CLI client for SunlightD, installed at `/bin/sunlightctl`.

```
sunlightctl list
sunlightctl status <service>
sunlightctl start <service>
sunlightctl stop <service>
sunlightctl restart <service>
sunlightctl reboot <service>           # alias for restart
sunlightctl enable [--now] <service>
sunlightctl disable [--now] <service>
```

The `--now` flag can appear before or after the service name:
```
sunlightctl enable --now solar
sunlightctl enable solar --now         # also accepted
```

### Example: Solar HTTP Server

Solar is disabled by default. To use it:

```
sunlightctl start solar                # one-off start, stays disabled
sunlightctl enable --now solar         # enable + start immediately
sunlightctl disable --now solar        # disable + stop immediately
```

### Example: list output

```
NAME               STATE      ENABLED    PID
solar              stopped    disabled   -
timezone_service   running    enabled    5
niced              running    enabled    6
gcd                running    enabled    7
uac_service        running    enabled    8
sunlight-sm        running    enabled    9
sunlight-kv        running    enabled    10
rand_service       running    enabled    11
sunlight-tls       running    enabled    12
```

---

## Source Layout

```
services/sunlightd/
  src/
    main.rs        — load_units(), boot spawn loop, IPC dispatch
    ipc.rs         — SunlightdOp enum, StatusReply, ListEntry, encoding helpers
    supervisor.rs  — ServiceEntry, ServiceState, enabled field
    unit.rs        — ServiceUnit / SocketUnit parsers (INI-style unit files)
    graph.rs       — Kahn's topological sort for dependency ordering
    journal.rs     — stub (future log capture)
    socket_act.rs  — stub (future socket activation)

services/sunlightctl/
  src/
    main.rs        — all commands, argc/argv parsing, IPC calls
```

---

## What Is Already Working (In-Memory)

- [x] Service table with `enabled: bool` per entry
- [x] Solar disabled by default, skipped in boot spawn loop
- [x] `start` — spawns via kernel `spawn` capability
- [x] `stop` — sends SIGTERM, marks service stopped
- [x] `restart` — stop + start
- [x] `enable` / `disable` — flips `enabled` flag, returns NOP if already in state
- [x] `enable --now` / `disable --now`
- [x] `list` with name / state / enabled / PID columns
- [x] `status` with enabled field
- [x] `reboot` alias for restart
- [x] IPC wire format uses only words[0..4] (transport-safe)
- [x] `find_by_name` used in spawn loop (fixes old index-mismatch bug)

---

## Known Limitations & Future Work

### P1 — Persistence

Enabled/disabled state is in-memory only. After a reboot, or if sunlightd is
restarted, all services revert to their compiled-in defaults (solar disabled,
everything else enabled).

**Future:** Write enable/disable decisions to the KV store (`sunlight-kv`) under
a `sunlightd/enabled/<name>` namespace. Read them back at `load_units()` time to
override compiled-in defaults. This requires sunlight-kv to be running before
sunlightd reads state — which means sunlightd must either start kv first and
then re-read, or use a two-phase init.

### P1 — Stop Reliability

`Stop` sends SIGTERM and immediately marks the service as `Stopped`. If the
kernel's signal delivery is asynchronous, there is a window where the process is
still alive but sunlightd considers it stopped. A subsequent `Start` could race
with the dying process.

**Future:** After SIGTERM, poll `waitpid(pid, WNOHANG)` in a short retry loop
before marking stopped.

### P2 — Restart-after-stop Race

`Restart` is implemented as stop + spawn with no wait between them. If the binary
holds a nameserver slot, the new instance might fail to register until the old one
exits.

**Future:** Same waitpid fix as stop reliability above.

### P2 — Reload

`Reload` currently returns `REPLY_OK` immediately (no-op). Unit definitions are
compiled-in, so there is nothing to reload from disk yet.

**Future:** When on-disk unit files are supported, `Reload` should re-parse them
and diff against the running table without restarting already-running services.

### P2 — Restart Monitoring

SunlightD tracks `restart_count` and `ServiceState` but does not yet watch for
process exit. There is no background loop polling `waitpid` for managed PIDs.
Failed services stay in `Running` state until manually queried.

**Future:** Add a periodic check loop (or signal-based child reaping) in the main
IPC loop, between `ipc_reply_and_wait` calls.

### P3 — Socket Activation

`socket_act.rs` and `journal.rs` are stubs. Socket-activated services (e.g. a
future `sshd`) would need a socket listener registered before the service starts.

### P3 — Boot Profiles

The memory target is < 100 MB for the graphical desktop. Today only Solar is
disabled. Future non-graphical mode (e.g. headless server) might disable
display-related services.

**Future:** Add an optional boot profile field to each unit (`Profile = graphical
| headless | minimal`). SunlightD reads the active profile from KV at startup and
skips services not in that profile.

### P3 — Unit Files on Disk

Unit strings are currently hardcoded in `load_units()`. Moving them to
`/etc/sunlightd/` (or `/lib/sunlightd/`) would allow packages to drop their own
unit files without recompiling sunlightd.

### P3 — GetLog (label 20)

The `GetLog` opcode is wired in the opcode table but returns `REPLY_ERR`. Future
implementation would stream journal output over SHM to `sunlightctl logs <service>`.

---

## Adding a New SunlightD-Managed Service

Follow `docs/ADDING_A_BINARY.md` spots 1–4 and 6, then add a unit string in
`load_units()`. Set `entry.enabled = false` after `services.add()` if the service
should be opt-in. Do not touch spot 5 (RamFS stub) for daemons — only `/bin`
commands need the shell PATH stub.

Quick checklist:

```
1. Add crate to root Cargo.toml [members]
2. Add build line to tools/build.sh and tools/test.sh
3. Add include_bytes! static in kernel/src/main.rs
4. Add arm in kernel/src/process/spawn.rs embedded_bytes_for_path
5. Add unit string in services/sunlightd/src/main.rs load_units()
6. (optional) Set entry.enabled = false for opt-in services
```
