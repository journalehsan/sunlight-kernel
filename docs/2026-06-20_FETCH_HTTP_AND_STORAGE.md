# Fetch over HTTP — the final "I/O error" and how storage works

**Date:** 2026-06-20
**Component:** `sunlight-fetch`, `sunlight-libc`, kernel `sys_open`, `sunlight-fs`
**Status:** ✅ `fetch http://example.com` now connects, gets `200 OK`, and saves a file.

This is the last fix in the chain that took SunlightOS from "can't ping" to a
working HTTP downloader. For completeness the full chain was:

1. **Checksum capabilities** (`sunlight-net/src/proxy_device.rs`) —
   `ChecksumCapabilities::ignored()` → `default()`. `ignored()` told smoltcp to
   skip checksums (assume hardware offload); with no offload, every outbound
   IPv4 packet shipped with a zero/invalid checksum and QEMU slirp dropped it.
   ARP (no checksum) worked, so the link looked half-up. Fixed → ping/DNS/TLS
   packets actually leave. (commit `c6278c1`)
2. **Stale `/etc/hosts`** — `example.com` was hard-pinned to the retired
   `93.184.216.34`, which black-holes SYNs. Removed so it resolves live via
   upstream DNS. (commit `4f1ab48`)
3. **The file write** — this document.

---

## The symptom

`fetch http://example.com` got all the way through the network path:

```
tx (SYN) → rx (SYN-ACK) → tx (ACK)        # handshake OK
tx … proto=06 dst_port=0050               # HTTP request out
rx n=917 … proto=06 src_port=0050         # HTTP 200 response + body in
[SCHED] FINISHED process pid=18 name='fetch'
```

…and then printed `fetch: error: I/O error` at the very end. The data was
fetched correctly; the failure was entirely in writing the result to disk.

## Root cause — two compounding bugs

### 1. `open()` instead of `create()` (no `O_CREAT`)

`sunlight-fetch`'s no_std writer opened the output file with `libc::open()`:

```rust
let fd = libc::open(path.as_bytes())?;   // SYS_OPEN with flags = 0
```

`libc::open()` passes `flags = 0` — **no `O_WRONLY`, no `O_CREAT`**. The kernel
`sys_open` only creates a file when `O_CREAT (0x40)` is set, so opening a
*non-existent* `index.html.part` read-only failed immediately. fetch could
never create any output file.

Fix: use `libc::create()`, which passes `O_WRONLY | O_CREAT`:

```rust
let fd = libc::create(abs.as_bytes())?;  // SYS_OPEN with O_WRONLY | O_CREAT
```

### 2. A relative filename on an immutable root

`fetch <url>` with no `-o` defaults the output to a name inferred from the URL —
for `http://example.com` that is the **relative** `index.html`. SunlightOS has
**no current-working-directory concept**: the kernel passes the path straight to
the VFS with no CWD prefixing. A bare `index.html` therefore resolves against
`/`, which is immutable (see below), and the write policy denies it.

Fix: anchor relative output names under root's home (`/root`), which is
writable. Absolute `-o /path/...` names are passed through untouched.

```rust
fn resolve_out_path(path: &str) -> String {
    if path.starts_with('/') { String::from(path) }
    else { format!("/root/{path}") }     // e.g. index.html → /root/index.html
}
```

Result: `fetch http://example.com` saves to **`/root/index.html`**.

---

## How storage works in SunlightOS

Understanding why a bare filename failed requires the storage model.

### Filesystem layout

* The root filesystem is an in-memory **ramfs** mounted at `/`, seeded from the
  build-time image (`sunlight-fs/…`). Other mounts (e.g. zram) attach under it
  per `/etc/fstab`.
* There is **no working directory**. Userland passes absolute paths; the kernel
  does not maintain or prepend a CWD. A relative path is taken verbatim and so
  effectively lands at the root.

### The write policy (`sunlight-fs/src/policy.rs`)

Every create/write goes through `can_write(actor, path, op, …)` in the kernel
*before* the VFS is touched. The model is **immutable-root with writable home
trees**:

| Path                         | root (uid 0)            | a user (uid N)            |
|------------------------------|-------------------------|---------------------------|
| `/`, `/etc`, `/bin`, `/dev`… | ❌ `DeniedImmutableRoot` | ❌ `DeniedImmutableRoot`   |
| `/root` and below            | ✅ allowed               | ❌                         |
| `/home/<name>` and below     | ✅ allowed               | ✅ only their own `<name>` |
| `/var/lib/...`               | via capability grant    | via capability grant      |

This is why system services that need persistence (e.g. `sunlight-kv`) go
through the storage manager / capability broker to write under `/var/lib`,
while ordinary programs simply write into a home tree.

### The open/create path

```
userland: libc::create(path)               // O_WRONLY | O_CREAT
   └─ SYS_OPEN(path, flags, mode)
        └─ kernel sys_open:
             wants_create = flags & 0x40
             can_write(actor, path, Create)        // policy gate (immutable root)
             if wants_create: vfs.create_file(path) // ramfs: create, or open if exists
             register fd with WRITE rights
   └─ libc::write(fd, bytes)  →  SYS_WRITE  →  vfs.write(handle, off, buf)
   └─ libc::close(fd)
```

`ramfs::create_file` validates the path is absolute, requires the parent to be
an existing directory, and—if the file already exists—opens it instead of
erroring (it does **not** truncate; a shorter rewrite can leave a stale tail).

### Atomic write (and a current limitation)

`fetch` writes to `<name>.part` then "renames" to `<name>`. SunlightOS has no
`rename`/`unlink` syscall yet, so on no_std the rename is emulated by reading the
`.part` back and rewriting the final file. Consequence: a stray
`/root/index.html.part` is left behind. Cosmetic; to be cleaned up when an
`unlink` syscall lands.

---

## Known follow-ups (not blocking)

* **Chunked transfer decoding.** example.com (Cloudflare) replies with
  `Transfer-Encoding: chunked` and no `Content-Length`. The body reader reads
  until the peer closes, so the file saves — but the saved bytes still contain
  the chunk-size framing lines. Needs de-chunking in `read_body_full`.
* **`unlink` syscall** to make the `.part` → final rename clean.
* **HTTPS/TLS** end-to-end, now unblocked by the working TCP + checksum path.

## Files touched

* `sunlight-fetch/src/downloader.rs` — `create()` instead of `open()`;
  `resolve_out_path()` anchors relative names under `/root`.
