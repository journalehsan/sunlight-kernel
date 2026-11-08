# Rappid Rabbit internal start page

Rappid Rabbit includes a start page in its application binary. Open it with
`rappid:startpage`; `rappid://startpage` is an alias. Both spellings resolve to
the canonical address `rappid:startpage` in the address bar.

The HTML and stylesheet are compiled from `rappid-rabbit/assets/`. Rabbit
recognizes the internal scheme before HTTP URL parsing, so internal documents
do not enter `sunlight-fetch`. Unknown `rappid:` addresses show an internal
page error.

Golden Fish parses the compiled start-page DOM once and retains it in a
`OnceLock`. Each navigation clones that immutable DOM and builds styles and a
render scene for the active viewport. The retained scene is not shared across
viewports because its layout geometry depends on the viewport.

When Rabbit starts without an explicit URL, it opens `rappid:startpage`.

## Certificate information

Click the site indicator at the left of the address field to open the native
certificate card. Verified HTTPS uses a green lock; certificate rejection uses
a warning indicator. HTTP reports **Not Secure**. Transport failures and pending
navigation report unavailable details rather than claiming successful TLS.
The card shows the verified hostname, certificate subject and issuer, UTC
validity dates, signature algorithm, and public key size when recognized.
**View Full Chain** expands the available certificates in root-most-first order.
Scroll inside the card for additional rows. Escape, an outside click, focus
loss, or a new navigation dismisses it.

`sunlight-cert` is a dependency-free `no_std + alloc` metadata library. Its
bounded DER reader extracts display fields only; rustls continues to validate
hostnames, dates, signatures, and trust. Unknown algorithms are shown using
OID bytes, unsupported key sizes remain unavailable, and malformed metadata
never enables TLS or turns a rejected connection into a secure one.

Data flows from the live TLS session through `TcpHandle`,
`sunlight_fetch::backend::RequestResult`, and Rabbit's `NavigationOutput`.
Certificate state follows the existing navigation generation checks and is
cleared before each navigation. Subresource responses cannot replace the main
document's certificate. The panel renders backend values and performs no
certificate lookup or verification.

On native SunlightOS, `sunlight-tls` retains DER for the anchors loaded from
`tls.ca.idx` / `tls.ca.*` in `sunlight-kv`. After a successful handshake it checks
for a KV anchor that independently verifies the peer chain with the same
crypto provider, hostname, and clock. That anchor can be included with a
**Trusted anchor** indicator. Other entries say **TLS supplied**: servers can
send extra certificates, so presence alone does not imply individual trust or
membership in the selected validation path. The KV schema and existing policy
that indexed certificates are trust anchors remain unchanged; do not install
an intermediate there unless it should be trusted as an anchor.

When no usable KV anchors exist, native TLS falls back to `webpki-roots`, as
the host fetch implementation does. These anchors do not contain complete
root DER, so the card explicitly reports unavailable root details. No root
metadata is synthesized. Certificate rejection keeps the error message;
rejected-session leaf metadata may be unavailable because the session is
never admitted. No certificate bypass is added.

The additive IPC operation `TLS_CERT_DER` (`0x5409`) accepts a session ID in
word 0, certificate index in word 1, and a caller-owned shared page in cap 0.
It returns the DER length in word 0 (zero marks the end), and word 1 is one
for a separately verified KV anchor. Peer entries are leaf-first; the optional
anchor follows them. Transfers are bounded to 4096 bytes and 16 queries per
snapshot. The daemon checks page capacity and releases its mapping after the
copy. Older daemons or oversized certificates leave metadata unavailable while
the connection status still comes from the successful TLS transport.

Rust callers constructing `RequestResult` must initialize `cert_info` and
`connection_security`. Exhaustive `FetchError` matches must handle
`TlsCertInvalid(String)`. No persistent-data migration is required; deploy the
updated TLS daemon to enable native certificate details.

Focused host checks:

```sh
cargo test -p sunlight-cert -p sunlight-ui --lib --target x86_64-unknown-linux-gnu
cargo test -p rappid-rabbit --bin rappid-rabbit --target x86_64-unknown-linux-gnu
cargo test -p sunlight-fetch --test certificate_session --target x86_64-unknown-linux-gnu
cargo check -p sunlight-fetch --lib --target x86_64-unknown-linux-gnu
cargo check -p sunlight-tls --no-default-features --features sunlightos
```

Use the platform-specific workspace procedure in `docs/workspace-checks.md`
for full workspace checks; native binaries and host binaries require distinct
targets and feature selections.
