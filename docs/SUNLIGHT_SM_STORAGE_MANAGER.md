# SUNLIGHT_SM_STORAGE_MANAGER

## Why sunlight-sm exists

SunlightOS is moving to an immutable-by-policy root filesystem. Normal services must not write to `/etc`, `/var`, `/usr`, `/boot` etc. Writable areas for apps are `/tmp` and `/home`.

Persistent system state (e.g. sunlight-kv store, TLS material) must go through a controlled, auditable path.

`sunlight-sm` ("Storage Manager", registered as name `"sm"`) is the focused, minimal service that owns writes to an explicit static whitelist. It solves the immediate protected-write problem without requiring full VFS capability delegation, OP_DELEGATE tokens, or runas elevation.

## Why simpler than full delegated write tokens (this bite)

- No changes to core capability broker or VFS token model.
- No new ABI for minting broad "full access" tokens.
- sm itself is trusted; it enforces narrow directory prefix whitelist internally.
- Other services (only sunlight-kv for now) make explicit IPC requests; sm checks every operation.
- Fast to implement and less likely to destabilize TLS/KV bringup.

## Current whitelist (static, prefix match after normalization)

- `/var/lib/sunlight-kv/`
- `/var/lib/sunlight/tls/`
- `/var/lib/sunlight/`

Rules enforced:
- Must be absolute.
- No `..` components or traversal tricks.
- Empty/relative denied.
- Only prefix match on the table (narrow dirs preferred).

Denied example log: `[SM][DENY] op=write path=/etc/passwd reason=not-whitelisted`

Allowed: `[SM][ALLOW] op=write path=/var/lib/sunlight/kv.store len=...`

## IPC operations (SmMsg in ipc/src/lib.rs)

- `OP_SM_WRITE_FILE` (1)
- `OP_SM_MKDIR_ALL` (2)
- `OP_SM_REMOVE` (3)
- `OP_SM_READ_FILE` (4)

Reply labels: `REPLY_OK=1`, `REPLY_ERR=0xff`

`words[0]` on error: 0=OK, 1=DENIED, 2=INVALID_PATH, 3=PAYLOAD_TOO_LARGE, 4=NOT_FOUND, 5=IO_ERROR, 6=UNSUPPORTED

Payloads for content use shared-memory grant (shm_alloc + cap) to stay within current IpcMsg limits. Large writes >~4k return PAYLOAD_TOO_LARGE.

Batch (5) skipped in this bite (IPC size).

## Write behavior

- Parents created (best-effort mkdir -p).
- Content write from offset 0 (replace/truncate semantics available via ramfs).
- Atomic rename+flush not yet wired in base FS for services; logged as `atomic=false reason=...`
- Remove clears content (full unlink not yet in base user libc path).

## sunlight-kv integration

sunlight-kv (sunlightos feature) now:
- Calls sm for mkdir of its protected dirs.
- All log append records are sent via sm (read-modify-write of the store log via sm_read + sm_write).
- On sm unavailable: clear error (no silent direct write).
- Logs: `[KV][SM] lookup ok`, `[KV][SM] mkdir ...`, `[KV][SM] write ... ok=...`

If sm missing the KV will surface storage errors instead of succeeding on protected paths.

## Known limitations (this bite)

- Payloads limited to one shm page (~4k path+data). Larger KV records would need chunking (future).
- Remove is best-effort (content clear); no directory rmdir/unlink syscall surface yet.
- No per-caller ACLs inside sm (whitelist + nameserver reachability is the gate).
- No integration with UAC runas/delegation (explicit non-goal).
- sm and kv use direct libc VFS calls (as trusted services); future bites will layer real caps.

## Future

When full VFS write capability delegation lands, sm can be evolved (or replaced) to accept minted narrow write tokens instead of (or in addition to) its internal whitelist. The IPC surface can stay source-compatible.

See also: docs/PHASE5_STORAGE_ARCHITECTURE.md, docs/TLS_RUSTLS_PROGRESS.md, docs/FILESYSTEM_SECURITY.md
