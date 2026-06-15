# sunlight-fetch — TODO

Track what works today and what still needs to land for a complete SunlightOS downloader.

## Done

| Area | Status |
|------|--------|
| CLI parsing | URL, `-o`, `-T`, `-d`, `-c`, `--help` |
| HTTP/1.1 client | GET/POST, header parse, redirect follow (301/302/303/307/308) |
| URL parser | `http://` and `https://`, query strings in path, filename inference |
| Linux host build | `host-linux` feature — DNS/TCP/TLS via std + rustls |
| SunlightOS integration | Embedded in kernel, `/bin/fetch` + `/usr/bin/fetch` ramfs stubs |
| SunlightOS HTTP | DNS via `NetOp::RESOLVE`, TCP via `NetOp::CONNECT/SEND/RECV/CLOSE` |
| net_server TCP | Real smoltcp sockets (was IPC stub) |
| Atomic writes (host) | `file.part` → `rename` |
| Progress bar | Single-line stderr TUI |

## Missing / Next

### P0 — HTTPS on SunlightOS (blocks Edge download)

Linux host has full TLS. SunlightOS returns:

```text
HTTPS not yet supported in SunlightOS fetch (use http:// URLs for now)
```

Needed:

- [ ] `rustls` + `ring` on `x86_64-unknown-none` (no_std)
- [ ] `getrandom` custom provider (e.g. `rdrand` on x86_64)
- [ ] Rustflags: `-C target-feature=+sse,+sse2` for ring
- [ ] TLS over IPC TCP: handshake via `read_tls` / `write_tls` / `process_new_packets`
- [ ] Smoke test: `fetch https://example.com` inside QEMU shell

### P1 — VFS / filesystem

- [ ] Kernel `rename` syscall (SunlightOS uses read-back + rewrite workaround)
- [ ] Write directly to output path without `.part` fallback once rename exists
- [ ] Large-file streaming (write chunks to disk instead of buffering full body in RAM)
- [ ] `fetch -o -` (stdout) support

### P1 — Parallel chunked downloads (`-c N`)

CLI accepts `-c` (default 16) but only single-stream download runs today.

- [ ] Probe `Accept-Ranges: bytes` + `Content-Length`
- [ ] Split into N `Range:` requests when supported
- [ ] Fallback to single stream when Range unsupported
- [ ] Chunk integrity checks + reassembly
- [ ] Cooperative parallelism (fits SunBurst scheduler — no threads yet)

### P2 — IPC / networking performance

Current TCP IPC moves **48 bytes per message** (`IPC_MAX_WORDS` packing). Works but slow for 100MB+ files.

- [ ] Use shared-memory grants (`ShmAlloc` / `ShmMap`) for SEND/RECV bulk transfer
- [ ] Or raise chunk size with a dedicated net_server bulk op
- [ ] Keep-alive / connection reuse (today: `Connection: close`)

### P2 — HTTP protocol edge cases

- [ ] `Transfer-Encoding: chunked` when `Content-Length` absent
- [ ] Relative redirect locations (`../path`, `./file`)
- [ ] Cookie / auth headers (optional `-H` flag?)
- [ ] POST body from stdin (`-d -`)
- [ ] Timeout and retry policy

### P2 — SunlightOS polish

- [ ] Capability acquisition (`net`, `vfs_write`) — currently assumed
- [ ] Ctrl+C / interrupt handling via signal or TTY hook
- [ ] Boot test gate: `tools/tests/phase5x_7.expected` + inject `fetch http://example.com`
- [ ] Man page `man/fetch.1`
- [ ] Deprecate or wire `/bin/wget` and `/bin/curl` stubs to `fetch`?

### P3 — no_std cleanup

- [ ] Gate all `std` usage behind `host-linux`; lib should be fully `alloc` on OS
- [ ] Shared `platform` module instead of `cfg` branches in `ipc.rs` / `downloader.rs`
- [ ] Reduce binary size (currently ~78KB release, target was ~4KB — acceptable for now)

## Build reference

```bash
# Linux host (HTTPS works)
cd sunlight-fetch
cargo build --release
./target/x86_64-unknown-linux-gnu/release/fetch https://example.com

# SunlightOS (HTTP only until TLS lands)
cd ..
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
  cargo build --package sunlight-fetch --features sunlightos --no-default-features --release
./tools/build.sh
```

## Architecture notes

```
fetch (userland, no_std)
  ├── cli / http / downloader / progress  (shared lib)
  └── ipc.rs
        ├── host-linux  → std::net + rustls
        └── sunlightos  → sunlight-ipc → net_server
                              ├── RESOLVE (DNS)
                              └── CONNECT/SEND/RECV/CLOSE (smoltcp TCP)
```

Kernel embeds `target/x86_64-unknown-none/release/fetch` via `include_bytes!`; ramfs exposes stub `#!/sunlight/fetch` at `/bin/fetch`.