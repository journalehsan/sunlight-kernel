# Phase 5.x Implementation Plan

## Overview
Replace all Phase 5.0-5.7 stubs with real, functional networking and utilities implementations.

**Status:** Planning complete ✓
- Roadmap written: `docs/phase5x-roadmap.md`
- Test expectations created: `tools/tests/phase5x_{0,1,2,3,4,5,6}.expected`
- Test infrastructure updated: `tools/test.sh` recognizes phase5x.0-5.x.6
- Ready to begin implementation

---

## Phase Breakdown

### Phase 5.x.0: Real DHCP
**Status:** Ready to implement
**File:** `sunlight-net/src/dhcp.rs`
**Key:** smoltcp::socket::dhcpv4::Socket
**Gate:** `[DHCP] OK`

### Phase 5.x.1: Real DNS
**Status:** Ready to implement  
**File:** `sunlight-net/src/dns.rs`
**Key:** smoltcp::socket::dns::Socket
**Gate:** `[DNS]  OK`

### Phase 5.x.2: Real TCP Sockets
**Status:** Ready to implement
**File:** `sunlight-net/src/tcp.rs`
**Key:** smoltcp::socket::tcp::Socket
**Gate:** `[TCP]  OK`

### Phase 5.x.3: Real ICMP Ping (M3 MILESTONE!)
**Status:** Ready to implement
**File:** `sunlight-net/src/icmp.rs`
**Key:** smoltcp::socket::icmpv4::Socket
**Gate:** `[M3]   ping 8.8.8.8: SUCCESS 🌐`

### Phase 5.x.4: Real TLS
**Status:** Ready to implement
**File:** `sunlight-net/src/tls.rs`
**Key:** rustls::ClientConnection + webpki-roots
**Gate:** `[TLS]  Handshake OK`

### Phase 5.x.5: sunlight-utils
**Status:** Ready to implement
**Crate:** New `sunlight-utils/`
**Commands:** ls, cat, cp, mv, rm, mkdir, grep, find, etc.
**Gate:** `[UTIL] OK`

### Phase 5.x.6: sunlight-net-utils
**Status:** Ready to implement
**Crate:** New `sunlight-net-utils/`
**Commands:** ping, ifconfig, wget, curl, dig, etc.
**Gate:** `[NET]  OK`

---

## Development Strategy

### Per Phase:
1. Implement real functionality (no stubs)
2. Run `./tools/test.sh phase5x.X`
3. Fix compilation/logic errors
4. Verify gate passes
5. Run regression tests on all prior phases
6. Proceed to next phase

### Testing Approach:
```bash
# After each phase:
./tools/test.sh phase4.5   # No regression
./tools/test.sh phase5.0   # No regression  
./tools/test.sh phase5.7   # No regression
./tools/test.sh phase5x.0  # New gate passes
```

---

## Critical Implementation Notes

### DHCP (Phase 5.x.0)
- Use `smoltcp::socket::dhcpv4::Socket`, not `DhcpClient`
- Must poll with `iface.poll(now, device, sockets)`
- Handle `DhcpEvent::Configured` and `DhcpEvent::Deconfigured`
- Apply routes via `iface.routes_mut().add_default_ipv4_route(gw)`
- QEMU user-net returns: 10.0.2.15/24, gw 10.0.2.2, dns 10.0.2.3

### DNS (Phase 5.x.1)
- Use `smoltcp::socket::dns::Socket`
- Start query with `socket.start_query(...)`
- Poll `socket.get_query_result(query_handle)`
- Return first IPv4 address from `query_result`

### TCP (Phase 5.x.2)
- Use `smoltcp::socket::tcp::Socket` with buffers
- `socket.connect(...)` starts 3-way handshake
- Poll until `socket.is_active()`
- Read/write via `socket.recv()` / `socket.send()`

### ICMP (Phase 5.x.3) - M3!
- Use `smoltcp::socket::icmpv4::Socket`
- Send via `socket.send_slice(&echo_request, target)`
- Receive via `socket.recv()`
- Measure RTT with `Instant::now() - send_time`
- **This is the M3 milestone**

### TLS (Phase 5.x.4)
- Add to Cargo.toml: `rustls = { version = "0.23", default-features = false, features = ["ring", "tls12"] }`
- Add to Cargo.toml: `webpki-roots = "0.26"`
- Use `rustls::ClientConnection` with root CA store
- Wrap TCP socket to handle TLS record layer

### sunlight-utils (Phase 5.x.5)
- No external crates! Only `sunlight-ipc`
- Dispatcher on argv[0]
- All file ops use VFS IPC (not std::fs)
- Commands: ls, cat, cp, mv, rm, mkdir, find, grep, wc, etc.

### sunlight-net-utils (Phase 5.x.6)
- No external crates! Only `sunlight-ipc`
- Dispatcher on argv[0]
- All network ops use net_server IPC
- Commands: ping, ifconfig, wget, curl, dig, etc.

---

## Safety & Constraints

✅ No confirmation prompts between phases
✅ No stubs — real functionality only
✅ SAFETY: comments on all unsafe blocks
✅ No regressions on Phase 3.x/4.x/5.0-5.7
✅ Run gates after each phase
✅ Stop only on blocking errors

---

## Success Criteria

```
Phase 5.x.0: ✓ DHCP real lease from 10.0.2.15
Phase 5.x.1: ✓ DNS resolves google.com → 142.250.x.x
Phase 5.x.2: ✓ TCP connects to 93.184.216.34:80
Phase 5.x.3: ✓ ping 8.8.8.8 returns 4/4 packets (M3!)
Phase 5.x.4: ✓ TLS handshake completes
Phase 5.x.5: ✓ sunlight-utils registered
Phase 5.x.6: ✓ sunlight-net-utils registered
Phase 3.x:   ✓ No regressions
Phase 4.x:   ✓ No regressions
Phase 5.0-7: ✓ No regressions
```

---

## Ready to Begin
All planning complete. Begin Phase 5.x.0 implementation.
