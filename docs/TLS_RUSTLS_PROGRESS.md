# Real rustls TLS for SunlightOS — Progress & Handoff

_Last updated: 2026-06-17_

## Goal

Replace the stub `sunlight-tls` (which rejected all HTTPS) with a **real rustls
TLS endpoint**, so `fetch https://api.sampleapis.com` works. Certificates (root
CAs) are stored in **sunlight-kv** as the trust store ("store center").

## Design (approved)

**Design B — daemon owns socket + crypto.** `sunlight-tls` opens its own TCP
socket via `net_server` and runs the full rustls handshake/IO loop. Clients
(`fetch`) exchange only **plaintext** with the daemon over IPC + shared memory.

```
fetch ──TLS_CONNECT(ip,port,sni)──▶ sunlight-tls ──socket──▶ net_server
fetch ──TLS_SEND(plaintext via shm)─▶ rustls encrypts ─▶ net
fetch ◀─TLS_RECV(plaintext via shm)── rustls decrypts ◀─ net
```

**Trust scope:** curated minimal. Embedded root = **GTS Root R4** (DER at
`sunlight-tls/src/roots/gts_root_r4.der`, 525 B, self-signed, valid to 2036).
`api.sampleapis.com` chains: leaf ← GTS WE1 ← GTS Root R4 (Cloudflare/Google).

## Status: Phases 0–5 DONE & compiling; Phase 6 (QEMU) BLOCKED

| Phase | What | State |
|---|---|---|
| 0 | Prove rustls builds for `x86_64-unknown-none` | ✅ done (recipe below) |
| 1 | Real freeing heap in sunlight-tls | ✅ `linked_list_allocator`, 8 MiB |
| 2 | sunlight-kv shm large-value transport | ✅ `KV_PUT_SHM`/`KV_GET_SHM` |
| 3 | sunlight-tls real rustls endpoint (unbuffered API) | ✅ compiles |
| 4 | certificatectl + trust seeding | ✅ compiles |
| 5 | fetch drops guard, talks to daemon | ✅ compiles |
| 6 | Full image build + QEMU end-to-end | ⛔ **daemon hangs at boot** (see below) |

The full image **builds** (`bash tools/build.sh`, ISO ~13 MB) and boots; the TLS
daemon starts but hangs before serving (details below).

## Phase-0 build recipe (CRITICAL — also in memory `rustls_no_std_recipe`)

`x86_64-unknown-none` disables SSE; RustCrypto SIMD backends emit 128-bit
intrinsics LLVM can't lower (`LLVM ERROR: Do not know how to split...`). Fix =
force software backends via RUSTFLAGS. `tools/build.sh` sets `TLS_RUSTFLAGS`:

```
--cfg aes_force_soft --cfg polyval_force_soft --cfg poly1305_force_soft
--cfg chacha20_force_soft --cfg curve25519_dalek_backend="serial"
```

Deps (sunlight-tls/Cargo.toml, all `default-features=false`):
- `rustls 0.23` features `["tls12","logging"]` (no std, no ring)
- `rustls-rustcrypto 0.0.2-alpha` features `["alloc","tls12","logging"]`
- `sha2 0.10` features `["force-soft","oid"]` (force-soft is a FEATURE here)
- `getrandom 0.2` features `["custom"]` → `register_custom_getrandom!` → `sunlight_libc::getrandom`
- `linked_list_allocator 0.10`

rustls no_std uses the **unbuffered** API (buffered `read_tls`/`write_tls` need
`std::io`): `UnbufferedClientConnection` + `process_tls_records()` state machine
(`EncodeTlsData`/`TransmitTlsData`/`BlockedHandshake`/`WriteTraffic`/`ReadTraffic`).
Config via `ClientConfig::builder_with_details(provider, Arc<dyn TimeProvider>)`;
custom `TimeProvider` over `get_time_utc()`.

## IPC protocol

sunlight-tls (labels in `sunlight-tls/src/main.rs` + `sunlight-fetch/src/ipc.rs`):
- `TLS_CONNECT 0x5401`: word0=packed ipv4, word1=port, words[2..]=SNI → reply word0=sid
- `TLS_SEND 0x5402`: word0=sid, word1=len, caps[0]=shm(plaintext) → reply word0=0
- `TLS_RECV 0x5403`: word0=sid → reply word0=len, word1=eof, caps[0]=shm(plaintext)
- `TLS_CLOSE 0x5405`, `TLS_INSTALL 0x5406`, `TLS_LIST 0x5407`, `TLS_REPLY 0x54FF`, `TLS_ERROR 0x54EE`

sunlight-kv (new, `sunlight-kv/src/main.rs`):
- `KV_PUT_SHM 0x4B06`: word0=value_len, words[2..]=key, caps[0]=shm(value) → KV_REPLY
- `KV_GET_SHM 0x4B07`: words[2..]=key → KV_VALUE word0=len + caps[0]=shm(value)

Producer allocs the shm page, fills it, sends `caps[0]=token`; consumer maps +
copies + frees (mirrors VFS `DATA_SHARED`). One page (≤4096 B) per value.

## ⛔ Current blocker (where to resume)

The daemon registers then **hangs on the very first kv call**. Boot serial:
```
[SUNLIGHT-TLS] Starting sunlight-tls (real rustls)
[SUNLIGHT-TLS] Registered as 'sunlight-tls'
[SUNLIGHT-TLS] dbg: build_root_store start
[SUNLIGHT-TLS] dbg: seed get index...      ← hangs here, nothing after
```
i.e. `kv_get_shm("tls/ca/index")` → `ipc_call(kv_cap, KV_GET_SHM)` never returns.
(Debug markers are currently in `seed_trust_if_empty`/`build_root_store`/`rebuild_config`.)

### Confirmed fact
`kernel/src/ipc/message.rs:83-89`: the register-IPC ABI transmits **only
words[0..3] (r8,r9,r10,r12) + caps[0..1] (r13,r14)**. Words 4–7 are dropped
(matches memory `register_ipc_four_word_limit`). Our control messages pack
key/SNI into words[2..7]; only words[2],[3] (16 B) actually cross.
- `"tls/ca/index"` (12 B) fits words[2,3] → GET key survives, so truncation
  alone does **not** explain the GET hang.
- BUT `"tls/ca/gts-root-r4"` (18 B) and `"api.sampleapis.com"` SNI (18 B) do
  **not** fit 16 B → PUT-key and SNI are truncated and **must be repacked**
  regardless.

### Ranked hypotheses / next experiment
1. **First, confirm kv receives the request.** Add `debug_log` at the top of the
   `KV_GET_SHM` and `KV_PUT_SHM` arms in `sunlight-kv/src/main.rs`. Rebuild
   (`bash tools/build.sh`), boot (command below).
   - If kv logs it → the kv handler hangs/faults (look at `shm_alloc`/`shm_map`).
   - If kv does **not** log it → IPC routing/deadlock: tls↔kv (likely a stale/bad
     `sunlight-kv` cap from `nameserver_lookup`, or a scheduler/IPC-reply issue).
     Note: at runtime nothing else had exercised cross-process calls *to* kv, so
     this may be a latent kv IPC bug, not TLS-specific.
2. **Repack control messages to the 4-word limit.** Keys/SNI must fit words[1..3]
   (24 B) or travel via shm. Suggested: word0=meta(len/flags), words[1..3]=key/SNI
   (≤24 B covers our names). Update both sides (tls + kv + certificatectl + fetch).
3. **Verify request-direction shm cap transfer.** VFS precedent passes the shm cap
   in the *reply* (server→client). We pass it in a *request* (client→server, for
   `KV_PUT_SHM` and `TLS_SEND`). Confirm the kernel delivers caps[0] on inbound
   requests and that `shm_map` works for a receiver that got the cap that way.
   (The hanging GET carries no cap, so this is about PUT/SEND, not the current hang.)

## Files changed
- `sunlight-tls/Cargo.toml` — rustls stack deps
- `sunlight-tls/src/main.rs` — full rewrite: LockedHeap, custom getrandom/TimeProvider,
  net client, unbuffered rustls connect/send/recv loops, kv trust load + self-seed
- `sunlight-tls/src/roots/gts_root_r4.der` — embedded curated root (NEW)
- `sunlight-kv/src/main.rs` — `KV_PUT_SHM`/`KV_GET_SHM` handlers + shm imports
- `certificatectl/src/main.rs` — real `install ca <name> <path>` (VFS read → shm → daemon) + `list`
- `sunlight-fetch/src/ipc.rs` — removed HTTPS reject guard; `OsConnection::Tls{session_id,rx}`;
  `tls_connect`/`tls_send`/`tls_recv` over shm (replaces feed/get_write/get_plain)
- `tools/build.sh` — `TLS_RUSTFLAGS` (force-soft cfgs) for the sunlight-tls build

## Build & verify commands

```bash
# Full build (services → kernel → ISO)
bash tools/build.sh

# Headless boot, capture serial (35s), inspect TLS daemon
rm -f /tmp/tls_boot.log
timeout 35 qemu-system-x86_64 -cdrom target/sunlightos.iso -m 2048 -vga std \
  -no-reboot -display none -serial file:/tmp/tls_boot.log \
  -netdev user,id=net0 -device virtio-net-pci,disable-modern=on,netdev=net0
grep -nE "SUNLIGHT-TLS|SUNLIGHT-KV|trust|seed|client config|PANIC" /tmp/tls_boot.log

# Build a single service quickly (no kernel/ISO):
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
  cargo build --package sunlight-kv --features sunlightos --no-default-features --release
# (sunlight-tls additionally needs the TLS_RUSTFLAGS force-soft cfgs — see tools/build.sh)
```

### End-to-end goal (once boot hang fixed)
Boot reaches login (root). In the shell run `fetch` against `https://api.sampleapis.com`.
Expect serial `[SUNLIGHT-TLS] hs_OK ...`, no `HTTPS rejected`, and a real 200 + JSON.
Note: login + shell input is via emulated **PS/2 keyboard**, not serial — driving
`fetch` automatically needs QMP `sendkey` or an autorun hook (not yet set up).

## Verification cleanup
- Remove the `dbg:` markers in `sunlight-tls/src/main.rs` once the hang is fixed.
