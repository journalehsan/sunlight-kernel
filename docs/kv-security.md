# sunlight-kv Security Model

This document describes how `sunlight-kv` authenticates callers, evaluates
per-key ACLs, isolates sensitive namespaces, and preserves compatibility with
existing consumers (TLS trust anchors, system service configs, daemon state).

It covers both build modes:

- **SunlightOS mode** (`--features sunlightos --no-default-features`,
  `x86_64-unknown-none`): the daemon runs as an embedded service inside the
  kernel; identity is resolved from kernel-authoritative IPC metadata.
- **Host mode** (`--features host`): the daemon runs as a normal Unix process;
  identity is resolved from `SO_PEERCRED` on the accepted socket.

## 1. Caller identity

### 1.1 The bug this fixes

Historically every operation handler in SunlightOS mode hardcoded the caller
string `"root"`. Because `sunlight-kv` is on the `UserSession` IPC allow-list,
any unprivileged process could read, mutate or delete arbitrary keys and bypass
every ACL. Identity must never come from the request payload: it is derived
from the message envelope and the kernel process table.

### 1.2 Extraction mechanism (SunlightOS mode)

1. The kernel stamps the **real sender PID** into `IpcMsg.badge`
   (`kernel/src/ipc/mod.rs`). The client cannot forge it; the daemon reads
   `msg.badge` and treats it as authoritative.
2. The daemon calls `sunlight_ipc::kv_caller_credentials(badge)`, which issues
   `SunlightSyscall::KvGetCallerCredentials` (108). This is a narrow syscall in
   the same family as `PtyGetCredentials` (103): the kernel refuses it unless
   `current_process().trusted_kv_service` is set.
3. The kernel (`sys_kv_get_caller_credentials`) looks up the target PID, echoes
   it in `r8`, returns the address-space **generation** in `r9`, the real
   `uid | (gid << 32)` in `rax`, and the **service-identity key** in `r10`.
4. `resolve_caller(badge)` maps the result to a [`Caller`]:

| Kernel result                                   | `Caller`               |
| ----------------------------------------------- | ---------------------- |
| `service_identity != 0`                         | `Caller::Service(key)` |
| `uid == 0`, no service key                      | `Caller::Root`         |
| any other `uid`                                 | `Caller::User(uid)`    |
| syscall denied / PID not live                   | `Caller::Unknown`      |

The service-identity key is `sunlight_ipc::name_to_u64(name)` (FNV-1a 64) of the
canonical service name. It is **stamped by the kernel** from the exact embedded
executable path at spawn/exec time (`trusted_service_identity_for_path`), and
only for a root-launched embedded daemon image. A user process can never obtain
a key, even if it registers the same nameserver name or is named `sunlight-tls`.

The `trusted_kv_service` marker (which gates the credential syscall itself) is
likewise granted only to the root-launched embedded `/sbin/sunlight-kv` image.

### 1.3 Extraction mechanism (host mode)

`resolve_caller_identity` calls `getsockopt(SO_PEERCRED)` on each accepted
connection: `uid == 0` becomes `Caller::Root`, otherwise `Caller::User(uid)`.
This mirrors the kernel semantics for native testing without the kernel
provenance machinery.

### 1.4 Why UID alone is insufficient

Every system daemon (`sunlight-tls`, `sunlight-sm`, `sunlightd`, …) and the
`uac_service` auth broker run as `User=root` (uid 0). UID alone therefore cannot
distinguish the TLS service from an arbitrary root process. The kernel-provenance
**service key** supplies that distinction: `Caller::Service(TLS)` is derived from
the embedded path, not from anything the caller claims.

## 2. Caller kinds and their authority

```rust
pub enum Caller {
    Root,                 // uid 0, or trusted kernel service with no narrower role
    Service(u64),         // authenticated system daemon, by stable service key
    User(u32),            // regular session application
    Unknown,              // identity could not be resolved (deny by default)
}
```

- `Caller::Root` and `Caller::Service(_)` are *privileged*: they bypass the
  per-key read/write lists (the legacy `"root"` admin bypass), but still pass
  the namespace gate.
- `Caller::User(uid)` is subject to both the namespace gate and the per-key ACL.
- `Caller::Unknown` is denied everything except world-readable public keys.

Named service keys defined in `sunlight_ipc::service_identity`:

| Constant          | Service name           | Notable authority                         |
| ----------------- | ---------------------- | ----------------------------------------- |
| `TLS`             | `sunlight-tls`         | may write public `tls.*` trust anchors    |
| `AUTH_BROKER`     | `sunlight-uac`         | may read any `account:<uid>:*` / `vault:*`|
| `KV`              | `sunlight-kv`          | the daemon itself                         |
| `STORAGE_MANAGER` | `sunlight-sm`          | persistence backend                       |
| `SERVICE_MANAGER` | `sunlightd`            | service supervisor                        |
| `SESSION_MANAGER` | `sunlight-sessiond`    | desktop session manager                   |

## 3. Namespaces

A key's namespace is derived from its prefix (`acl::classify`):

| Prefix                | Namespace           | Reachable by                                        |
| --------------------- | ------------------- | --------------------------------------------------- |
| `account:<uid>:*`     | `Account(uid)`      | that user, `Root`, the auth broker                  |
| `vault:*`             | `Vault`             | `Root`, the auth broker                             |
| `tls.` `pki.` `ca.` `pub.` | `Public`       | any caller (read); `Root`/`Service`/TLS (write)     |
| anything else         | `General`           | governed solely by the per-key ACL                  |

Rules:

- A user process may only reach its **own** `account:<uid>:*` namespace.
  Cross-user token reads are strictly denied.
- `vault:*` is privileged-only: `Root` or an explicitly authorised auth broker.
- `Public` keys are world-readable but writable only by `Root`/`Service`/TLS.

The namespace gate is applied **before** the per-key ACL for `Get`, `Set`,
`Delete`, and `Scan`. `Set`/`Delete` additionally consult
`namespace_allows_write` (a user cannot create keys in another user's namespace
even though the per-key ACL is checked independently).

## 4. Per-key ACL

Every record stores an ACL: `{ owner, read: [identity…], write: [identity…] }`.

- `owner` has implicit read and write.
- `read`/`write` list explicit grants by ACL identity string.
- The reserved value `*` (`acl::WORLD`) in `read` grants read to every caller.

ACL identity strings:

| Caller            | Identity string   |
| ----------------- | ----------------- |
| `Caller::Root`    | `root`            |
| `Caller::User(n)` | `uid:<n>`         |
| `Caller::Service` | `service:<key>`   |
| `Caller::Unknown` | `unknown`         |

The on-disk layout stays the legacy three-field `{owner, read, write}` shape so
existing stores load unchanged; the only extension is the `*` sentinel inside
`read`.

### 4.1 Public keys and legacy migration

TLS trust anchors and CA bundles must remain broadly readable while their writes
stay privileged. Two layers enforce this:

1. **On write**, keys in a public namespace receive `Acl::public_read`, and any
   existing public record is upgraded with `Acl::ensure_public_read` so a legacy
   `root`-only record gains world-read on its next write.
2. **On read** (`acl::read_allowed`), public-namespace keys are world-readable by
   policy regardless of the stored ACL. This means a *legacy* store created
   before this model existed — where `tls.ca.*` may still carry a `root`-only
   ACL — stays readable by every caller **immediately after upgrade**, without a
   rewrite-on-first-read migration or any handshake regression.

## 5. TLS and system-daemon compatibility

`sunlight-tls` is a KV client (`KV_GET_SHM` 0x4B07 / `KV_PUT_SHM` 0x4B06) that
stores compact `tls.ca.*` keys and the `tls.ca.idx` index. Under this model:

- The TLS service is classified `Caller::Service(TLS_SERVICE)`, so it may write
  public trust anchors (and any key its per-key ACL permits).
- Any client, including a user process or `Caller::Unknown`, may read
  `tls.ca.*` because those keys are in the public namespace.
- Handshake lookup and CA seed/rotate paths use the same `kv_get_shm` /
  `kv_put_shm` calls as before; no protocol change is required.

System service configs and daemon state stored by root-owned daemons
(`sunlightd`, `sunlight-sm`, `uac_service`, …) are written under
`Caller::Root` or their `Caller::Service` key and remain readable by those
services. User-shell calculator history (`calc.hist.idx`, `calc.h.<id>`) stays
`General`-namespace, owner-scoped to the invoking uid; it is intentionally **not**
public.

## 6. Persistence and performance safeguards

- **Record bounds.** `validate_record_bounds` rejects a key/value/ACL whose
  framing cannot fit the storage-manager page before anything is committed to a
  shared-memory frame, and rejects lengths that would truncate to `u32`.
- **Batched flush.** `sm_append_records` coalesces up to `PERSIST_BATCH` (16)
  queued records into a single read-modify-write, amortising the O(log size)
  full-store rewrite across all mutations in the batch instead of paying it once
  per put/delete.
- **No silent dirty-state loss.** On a failed flush the batch is *requeued at the
  front* of the pending queue (not dropped), `VOLATILE_ONLY` is set, and an
  explicit diagnostic is emitted. In volatile mode the in-memory store remains
  authoritative and correct; dirty records are retained for the process
  lifetime rather than discarded.

## 7. Error mapping

`KvError` mirrors the wire reason codes:

| `KvError`          | Meaning                                        |
| ------------------ | ---------------------------------------------- |
| `PermissionDenied` | Namespace or per-key ACL denied the operation  |
| `NotFound`         | Key absent (or tombstoned)                     |
| `InvalidRequest`   | Empty key, oversized record, malformed frame   |
| `Internal`         | Persistence/storage failure                    |

The host daemon maps `StorageError::PermissionDenied` to `PERMISSION_DENIED`,
so unauthorized reads and mutations are reported explicitly rather than as
generic errors.

## 8. Verification gates

- **Unit tests** (`cargo test -p sunlight-kv --target x86_64-unknown-linux-gnu`):
  - `Caller::User(1000)` reading/writing a `Caller::User(1001)` or root key
    returns `PermissionDenied` (`user_cross_namespace_denied`,
    `scan_filters_unauthorized_keys`).
  - Public keys are world-readable but not user-writable
    (`public_tls_key_readable_by_world_writable_only_by_root`).
  - Legacy `root`-only public records stay world-readable
    (`legacy_public_key_stays_world_readable`).
- **Regression** (`python3 tools/check_workspace.py`): checks every workspace
  library/binary across host, std, and freestanding feature graphs.
- **TLS handshake**: requires a QEMU run in which `sunlight-tls` seeds and loads
  its anchors and completes a handshake. Compilation and host tests are *not*
  evidence of a working handshake; record the serial log explicitly.
