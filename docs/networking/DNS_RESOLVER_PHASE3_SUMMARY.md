# DNS Resolver — Phase 3.0-3.4 (COMPLETE, live)

## Status: ✅ Fully completed and live

The `net_server` dynamic DNS resolver now resolves both local (`/etc/hosts`)
and external (upstream DNS over UDP) hostnames, with TTL caching and
hot-reloadable hosts entries.

## Resolver chain

```text
RESOLVE(hostname)
  -> /etc/hosts (HostsTable)
  -> TTL cache (DnsCache, BTreeMap<String, CacheEntry>)
  -> upstream DNS-over-UDP (8.8.8.8:53, hand-written RFC 1035 codec)
  -> (mDNS .local stub reserved for a future phase)
```

- `ResolverChain::resolve_local()` checks hosts then cache.
- On a cache/hosts miss, `RESOLVE` falls through to
  `sunlight_net::dns::upstream::query_a()`, which builds a UDP query with the
  hand-written `wire::DnsPacket` codec, sends it, and polls for a reply
  (one retry on timeout, `POLL_TIMEOUT_TICKS = 2000`, yielding via
  `sunlight_ipc::process_yield()` between polls).
- A successful upstream answer is inserted into the cache via
  `chain.cache_insert(hostname, ip, ttl, now)` using the record's real TTL.
- Any failure (timeout, no route, malformed reply, interface not yet up)
  returns `None` -> RESOLVE replies with `word(0, 0)` (NXDOMAIN), never panics.

## Phase 3.4: kernel frame-proxy device

`net_server` runs in ring-3 and cannot do port I/O, so the kernel keeps the
`VirtioNet` device alive after boot (`kernel::NET_DEVICE`) and exposes two
syscalls gated to pid 5 (`net_server`):

- `NetTx = 90` — send a raw Ethernet frame
- `NetRx = 91` — receive the next queued frame (0 bytes if none)

`sunlight-net::ProxyNetDevice` implements smoltcp's `Device` trait purely in
terms of these two syscalls. `net_server` brings up a smoltcp `Interface` over
`ProxyNetDevice` at startup (10.0.2.15/24, gateway 10.0.2.2 — matching
`NetOp::GETIP`), and `upstream::query_a()` runs its UDP socket over that
interface/device pair exactly like `dhcp::acquire_lease` / `icmp::ping`.

## New IPC opcode

- `NetOp::RELOAD_HOSTS = 12` — re-reads `/etc/hosts` from the VFS and
  atomically swaps the resolver chain's hosts table.

## Files changed/added

- `sunlight-net/src/dns/{mod,wire,cache,upstream}.rs` — RFC 1035 codec, TTL
  cache, resolver chain, upstream UDP query (all generic over `Device`)
- `sunlight-net/src/proxy_device.rs` — `ProxyNetDevice`/`ProxyRxToken`/`ProxyTxToken`
- `sunlight-net/src/netop.rs` — `RELOAD_HOSTS = 12`
- `ipc/src/lib.rs` — `NetTx`/`NetRx` syscalls (90/91) + `net_tx`/`net_rx` wrappers
- `kernel/src/main.rs` — `NET_DEVICE` static keeps `VirtioNet` alive after boot
- `kernel/src/arch/x86_64/syscall.rs` — `sys_net_tx`/`sys_net_rx`, pid==5-gated
- `services/net_server/src/main.rs` — resolver chain + smoltcp interface wiring,
  `RESOLVE` and `RELOAD_HOSTS` handlers

## Verification

- `cargo build -p sunlight-net -p sunlight-net-server` — clean (warnings only)
- `cargo build -p sunlight-kernel` — clean
- `cargo test --target x86_64-unknown-linux-gnu --lib` (sunlight-net) — 15/15 passed
- Manual: boot QEMU, from a shell run `ping example.com` (or any RESOLVE
  client) twice — first request takes the upstream UDP path and logs a
  cache insert; second request is served from `DnsCache` with no network
  round trip.

## Known limitations / follow-ups

- `.local` mDNS resolution remains a stub (`ResolverChain::is_mdns_name`)
  for a future phase.
- A single in-flight upstream query at a time (static UDP socket buffers in
  `upstream.rs`) — sufficient for net_server's current synchronous IPC model.
