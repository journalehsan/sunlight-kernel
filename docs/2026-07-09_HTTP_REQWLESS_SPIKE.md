# HTTP Layer Spike: reqwless and sunlight-http

**Date:** 2026-07-09  
**Scope:** Small investigation only. No production integration of reqwless.  
**Focus:** Sunlight-fetch HTTP, transport, reusability, reqwless fit, and `sunlight-http` facade proposal.  
**Constraints observed:** No kernel/scheduler/WM changes; no browser engines; HTTPS only if safe+explicit (current policy: clean `UnsupportedHttps` error if backend lacks it).

## Summary

- Sunlight-fetch implements its own minimal HTTP/1.1 (no external crates).
- The code is **not reusable as a library** in its current packaging.
- The pure HTTP message types (`HttpRequest`/`HttpResponse`/`ParsedUrl`) are easily extractable.
- A skeleton `sunlight-http` crate was created defining the recommended stable API surface.
- **reqwless is not a clear win for the first MVP**; it brings significant costs and model mismatches.
- Current Sunlight transport can implement *sync* `embedded-io` traits with modest wrappers, but reqwless's higher-level API is built on `embedded-io-async` + `embedded-nal-async` (async traits + executor requirement).
- TLS/HTTPS risk: reqwless's no_std TLS options have weaker verification stories than the existing `sunlight-tls` daemon (rustls). Do not route https through reqwless without explicit safe policy.

## 1. Where Sunlight-fetch currently implements HTTP

- **Protocol types & parsing** (`sunlight-fetch/src/http.rs`):
  - `ParsedUrl` (scheme/host/port/path, inference, host header).
  - `HttpRequest` + `serialize()` (builds wire bytes, always `Connection: close`).
  - `HttpResponse::parse()` (finds `\r\n\r\n`, parses status + lowercased headers).
  - Helpers: `content_length`, `accepts_ranges`, custom ASCII lowercase.
- **Orchestration** (`downloader.rs`):
  - `execute_download` routes http vs https.
  - `fetch_with_redirects` (up to 10 redirects, Location handling, absolute/relative).
  - `execute_get` / `execute_post`, progress, atomic write (`.part` then rename emulation).
  - Chunked note in TODO (current body reader works until close; chunked framing not stripped).
- **Transport** (`ipc.rs`):
  - Dual implementation behind `TcpHandle`:
    - `host-linux`: `std::net::TcpStream` + rustls (when https).
    - `sunlightos`: IPC to `net_server` (SOCKET/CONNECT/SEND_SHM/RECV_SHM/CLOSE via `NetOp`) and `sunlight-tls` daemon (TLS_CONNECT/SEND/RECV/CLOSE).
  - `http_request_impl` does the header read loop + initial body split.
  - `read_body_full` handles Content-Length or read-until-close.
- DNS is via `NetOp::RESOLVE` (or std `to_socket_addrs` on host).

No `embedded-*` traits or reqwless anywhere.

## 2. Is Sunlight-fetch reusable as a library today?

No.

- It is declared as both bin and lib, but the public API is incidental.
- The main useful entry (`execute_download`) is file-oriented CLI logic (output paths, progress TUI, atomic write, interrupt flag).
- HTTP, DNS, TLS policy, redirects, and VFS write are tightly coupled.
- No clean "take a request, give me a response + body handle" surface intended for other apps.
- Other apps (future Rappid Rabbit etc.) would need to duplicate or factor.

## 3. Can its HTTP code be extracted into `sunlight-http`?

Yes, the protocol bits are cleanly separable.

- `http.rs` (URL, request serialize, response header parse) has **no** dependencies on sunlight-net, ipc, or libc.
- Body reading, redirect loop, progress, and file I/O are higher.
- A skeleton `sunlight-http` crate now exists with equivalent (and slightly cleaned) types:
  - `ParsedUrl`, `HttpRequest`, `HttpResponse`
  - `HttpError` (focused: `InvalidUrl`, `Transport`, `Protocol`, `Status`, `UnsupportedHttps`, `Other`)
  - `get(url)` stub shape (real impls supplied by backends)
- Extraction would be a small follow-up: move the logic, make fetch (and future clients) depend on `sunlight-http`, map errors at the boundary.

## 4. Can reqwless sit behind a sunlight-http adapter?

Technically possible for the *HTTP framing/state machine*, with caveats:

- Low-level request builder + writer lives over `embedded_io_async::Write`.
- High-level `HttpClient<T, D>` where `T: embedded_nal_async::TcpConnect`, `D: Dns`.
- The connection objects must impl `embedded_io_async::{Read, Write}`.

**Adapter surface would look like** (future):

```rust
// sunlight-http (or a sunlight-http-reqwless feature)
use reqwless::client::HttpClient;
use sunlight_http::{HttpRequest, HttpResponse, HttpError};

pub struct SunlightTransport { /* wraps TcpHandle or equivalent */ }

impl embedded_io_async::ErrorType for SunlightTransport { type Error = ...; }
impl embedded_io_async::Read for ... { async fn read... }
impl embedded_io_async::Write for ... { async fn write... }

impl embedded_nal_async::TcpConnect for SunlightStack { ... }
impl embedded_nal_async::Dns for SunlightStack { ... }

pub async fn reqwless_get(...) -> Result<HttpResponse, HttpError> {
    let client = HttpClient::new(transport, dns);
    // ...
}
```

Problems today:
- Sunlight userland is **sync + explicit yield**, no async runtime/executor.
- `embedded-io-async` async fn traits require polling.
- Even plain TCP would need an adapter that turns our blocking/yield recv into an async read (possible with a trivial single-task executor, but new machinery).
- TLS: we would probably bypass reqwless's TLS entirely and present a plaintext view (the daemon already terminated TLS), defeating part of the value.

## 5. What transport traits would SunlightOS need to implement?

For reqwless high-level client:

- `embedded-nal-async::TcpConnect` (async fn returning a connection)
- `embedded-nal-async::Dns`
- Connections must satisfy `embedded_io_async::Read + Write + ErrorType`

For a hypothetical *sync-only* path (if someone forked or used lower level):
- Plain `embedded_io::Read + Write` is a close match to the existing `read_some`/`send_all` on `OsConnection`.

Current code already has the right shape (`&mut [u8]` read, slice write, owned chunks via SHM). Wrapping `TcpHandle` would be straightforward for sync `embedded-io`.

## 6. Would reqwless help more than a small custom HTTP/1.1 GET for the MVP?

**No, for the first MVP.**

Current custom implementation:
- Tiny (one file + small orchestrator).
- Zero extra crates for the protocol.
- Already handles GET/POST, redirects, range fallback, progress, host-linux + sunlightos.
- Works with the existing TLS daemon architecture.

reqwless costs (even `default-features = false`):
- Brings `p256`, `ecdsa`, `elliptic-curve`, `sha2`, `hkdf`, `rand_chacha`, `pkcs8`, `heapless`, `nourl`, `buffered-io`, `httparse`, etc. (crypto bloat for a client that may never use reqwless TLS).
- Requires async traits + executor.
- HTTP features (chunked support, more methods, header handling) are nice-to-have but the custom code + TODO already covers what fetch needs.
- Duplicate TLS story risk.

**When reqwless might help later**:
- We adopt an async model in userland (or a tiny HTTP executor).
- We want a richer client with less custom code (auth, cookies? forms?).
- Multiple backends behind the same `sunlight-http` surface (e.g. "host std", "smoltcp direct", "reqwless", "future native").

For now: keep custom small, extract types into `sunlight-http`, evaluate reqwless again when async or richer client requirements appear.

## 7. Risks around TLS/HTTPS

- **Current architecture is safe-by-construction for the client**: `sunlight-tls` daemon owns the TCP socket + rustls handshake + cert validation + roots (from kv + built-in). Fetch/similar only ever see plaintext bytes over IPC+SHM. Certificate errors and expiry are reported cleanly (`TlsHandshakeFailed`, `TlsCertExpired`).
- **reqwless TLS path**:
  - `embedded-tls` (default when feature on): "NOTE: TLS verification is not supported in no_std environments for `embedded-tls`."
  - `mbedtls-rs` / esp-mbedtls: different stack, hardware accel focus, TLS 1.2/1.3.
  - Both would be a second TLS implementation in the system.
- Policy: if a backend does not support https, it must return a clean `UnsupportedHttps` (or equivalent). The `sunlight-http::HttpError::UnsupportedHttps` variant exists for this.
- Do not enable reqwless TLS features for SunlightOS backends without an explicit review of verification, root store handling, and time source.

Recommendation: keep https termination in `sunlight-tls`. A future `sunlight-http` adapter over reqwless should only be used for the HTTP layer over a *pre-established plaintext transport*.

## Created / Proposed `sunlight-http` Crate

Location: `sunlight-http/`

Stable internal API (as recommended):

```rust
pub struct HttpRequest { method, path, host, headers, body }
pub struct HttpResponse { status_code, status_text, headers, header_len }
pub enum HttpError { InvalidUrl, Transport, Protocol, Status, UnsupportedHttps, Other }
pub fn get(url: &str) -> Result<HttpResponse, HttpError>; // shape / stub
```

Features:
- Always `#![no_std]` + `alloc`.
- Pure (no sunlight-net, no tls crates).
- Backends are out of tree / provided by consumers.

`cargo check --package sunlight-http` passes.

Future steps (outside this spike):
- Move `http.rs` logic into sunlight-http (or re-export).
- Make sunlight-fetch (and later clients) depend on it.
- Define a small `HttpTransport` trait or backend injection point.
- Only then consider an optional `reqwless` feature behind a clear async + dep cost gate.

## Verification Performed

- `cargo check --package sunlight-http` — clean.
- `cargo check --package sunlight-fetch --features host-linux --target x86_64-unknown-linux-gnu` — clean (pre-existing).
- No changes to kernel, drivers, services (except the net-facing types were only inspected), WM, or unrelated apps.
- No new HTTPS implementation.
- reqwless was probed via isolated temp crate + registry inspection (no permanent dep added).

## Conclusion

Create `sunlight-http` as the stable seam. Keep the proven custom implementation for the MVP. Revisit reqwless only when:
- an async substrate exists, or
- the custom HTTP code becomes a maintenance burden, and
- we accept the extra crypto surface or can scope it behind a "rich client" feature that still funnels TLS through the existing daemon.

The patch for this spike is intentionally small and reversible.
