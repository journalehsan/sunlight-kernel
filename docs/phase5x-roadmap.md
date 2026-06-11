# SunlightOS — Phase 5.x: Real Networking + Utilities
## Claude Code Prompt
## Model: Best available (1M context preferred)

---

## Constraints (read before starting)

- Do NOT ask for confirmation between sub-phases
- Do NOT present options at the end
- Implement all sub-phases in order, run gate tests after each
- Stop only when all gates pass OR a blocking error occurs
- If blocked: explain root cause clearly and stop
- Do NOT break any Phase 3.x, 4.x, or 5.0-5.7 gates
- `SAFETY:` comment on every unsafe block
- Every gate must test REAL functionality, not stubs

---

## Current State

```
✓ Phase 5.0  virtio-net PCI driver + device init
✓ Phase 5.1  smoltcp Device trait + net_server service
✓ Phase 5.2  DHCP/DNS stubs (serial log only)
✓ Phase 5.3  Socket IPC interface (placeholders)
✓ Phase 5.4  Linux socket syscalls (Helios stubs)
✓ Phase 5.5  TLS stub (serial log only)
✓ Phase 5.6  btrfs read-only stub
✓ Phase 5.7  NVMe driver stub

⚠️  ALL ABOVE ARE STUBS — gates pass but no real functionality
```

**Key crates (existing):**
- `sunlight-net`: VirtioNet, smoltcp integration (stubs)
- `services/net_server`: IPC endpoint, placeholder handlers
- `sunlight-ipc`: Message passing ABI
- `sunlight-fs`: VFS trait, RamFs

---

## Phase 5.x.0 — Real DHCP

**Goal:** Acquire actual IP lease from QEMU user-net via smoltcp DhcpSocket.

### Implementation

**File:** `sunlight-net/src/dhcp.rs`

Replace stub with real smoltcp DhcpSocket:
- Create DhcpSocket and add to SocketSet
- Poll Interface + DhcpSocket for events
- On `DhcpEvent::Configured`: extract IP, gateway, DNS servers
- Update Interface IP addresses and routes
- Timeout after 10 seconds
- Return `DhcpConfig` struct with actual values

**Expected output:**
```
[DHCP] Sending DISCOVER...
[DHCP] Got OFFER from 10.0.2.2
[DHCP] Sending REQUEST...
[DHCP] Lease acquired: 10.0.2.15/24
[DHCP] Gateway: 10.0.2.2
[DHCP] DNS: 10.0.2.3
[DHCP] OK
```

**Serial gate line:**
```
[DHCP] Lease acquired: 10.0.2.15/24
```

---

## Phase 5.x.1 — Real DNS

**Goal:** Resolve hostnames (google.com, example.com) via QEMU DNS server.

### Implementation

**File:** `sunlight-net/src/dns.rs`

Replace stub with real smoltcp DnsSocket:
- Create DnsSocket with DNS server IP from DHCP
- Start DNS query for hostname
- Poll until resolved or 5-second timeout
- Parse A record and return Ipv4Address
- Handle NXDOMAIN, timeouts, format errors

**Expected output:**
```
[DNS]  Querying 10.0.2.3 for google.com...
[DNS]  google.com → 142.250.185.46
[DNS]  OK
```

**Serial gate line:**
```
[DNS]  google.com → 142.250.185.46
```

---

## Phase 5.x.2 — Real TCP Sockets

**Goal:** Establish TCP connections to remote servers.

### Implementation

**File:** `sunlight-net/src/tcp.rs`

Real TcpSocket wrapper:
- `TcpConnection::connect()` — allocate buffers, bind ephemeral port, connect
- `send()` — queue data to transmit buffer
- `recv()` — read from receive buffer
- `close()` — graceful shutdown
- Timeout: 5 seconds for SYN-ACK

**Wire to net_server IPC:**
- Handle `NetOp::CONNECT` — create connection, return socket_id
- Handle `NetOp::SEND` — write to socket
- Handle `NetOp::RECV` — read from socket
- Handle `NetOp::CLOSE` — close connection

**Expected output:**
```
[TCP]  Connecting to example.com:80...
[TCP]  Connected (local 49152, remote 93.184.216.34:80)
[TCP]  OK
```

**Serial gate line:**
```
[TCP]  Connected (local 49152, remote 93.184.216.34:80)
```

---

## Phase 5.x.3 — Real ICMP Ping (M3 MILESTONE)

**Goal:** Send ICMP echo requests and measure RTT. **THIS IS M3!**

### Implementation

**File:** `sunlight-net/src/icmp.rs`

Real ICMP socket:
- Create IcmpSocket and bind
- Send echo request (ICMP type 8)
- Poll for echo reply (ICMP type 0)
- Measure round-trip time
- Print "64 bytes from X: icmp_seq=N time=XYZms"
- Exit code 0 if any reply received

**Integration:**
- Call from sunshell `ping` command via net_server IPC
- Or standalone test in net_server during boot

**Expected output:**
```
[PING] Sending 4 ICMP echo requests to 8.8.8.8...
64 bytes from 8.8.8.8: icmp_seq=0 time=12ms
64 bytes from 8.8.8.8: icmp_seq=1 time=13ms
64 bytes from 8.8.8.8: icmp_seq=2 time=12ms
64 bytes from 8.8.8.8: icmp_seq=3 time=14ms
--- 8.8.8.8 ping statistics ---
4 packets transmitted, 4 received, 0% loss, avg time=12.75ms
[M3]   ping 8.8.8.8: SUCCESS 🌐
```

**Serial gate line:**
```
[M3]   ping 8.8.8.8: SUCCESS 🌐
```

---

## Phase 5.x.4 — Real TLS Handshake

**Goal:** HTTPS connection with TLS 1.2/1.3, root CA validation.

### Implementation

**File:** `sunlight-net/src/tls.rs`

Real rustls integration:
- DNS resolve hostname
- TCP connect to host:443
- rustls ClientConnection setup
- TLS handshake (record layer via TCP)
- Validate server certificate (webpki-roots)
- Print "Handshake OK: example.com (TLSv1.3)"

**Dependencies:**
```toml
rustls = { version = "0.23", default-features = false, features = ["ring", "tls12"] }
webpki-roots = "0.26"
```

**Expected output:**
```
[TLS]  Connecting to example.com:443...
[TLS]  Handshake with example.com...
[TLS]  Handshake OK: example.com (TLSv1.3)
```

**Serial gate line:**
```
[TLS]  Handshake OK: example.com (TLSv1.3)
```

---

## Phase 5.x.5 — sunlight-utils v0.1

**Goal:** Core file system utilities (ls, cat, cp, grep, find, etc.).

### Implementation

**New crate:** `sunlight-utils`

Busybox-style dispatcher — argv[0] determines command:
- `ls` — list directory with permissions, size, date
- `cat` — concatenate files to stdout
- `cp` — copy file (no recursion in v0.1)
- `mv` — move/rename file
- `rm` — delete file
- `mkdir` — create directory
- `rmdir` — remove empty directory
- `touch` — create/update file
- `chmod` — change permissions
- `chown` — change owner
- `find` — search for files by name/type
- `grep` — pattern search in files (substring only, no regex)
- `wc` — word/line/byte count
- `head` — print first N lines
- `tail` — print last N lines
- `sort` — sort lines
- `uniq` — deduplicate lines
- `cut` — extract columns
- `date` — print current date
- `id` — print user/group IDs
- `whoami` — print current user

All commands use VFS IPC (not std::fs).

**Install in RamFs:**
```
/bin/ls → /usr/bin/sunlight-utils
/bin/cat → /usr/bin/sunlight-utils
/bin/grep → /usr/bin/sunlight-utils
... etc
```

**Expected output:**
```
[UTIL] sunlight-utils v0.1 loaded (ls, cat, grep, find, ...)
[UTIL] Commands available: ls cat cp mv rm mkdir rmdir touch chmod find grep wc head tail sort uniq cut date id whoami
[UTIL] OK
```

**Serial gate line:**
```
[UTIL] Commands available: ls cat cp mv rm mkdir...
```

---

## Phase 5.x.6 — sunlight-net-utils v0.1

**Goal:** Network utilities (ping, ifconfig, wget, curl, dig, etc.).

### Implementation

**New crate:** `sunlight-net-utils`

Busybox-style dispatcher:
- `ping` — ICMP echo test (uses real ICMP socket)
- `ifconfig` — show interface config (IP, MAC, gateway)
- `ip` — alias for ifconfig
- `wget` — HTTP GET to file
- `curl` — HTTP/HTTPS to stdout
- `dig` — DNS query tool
- `nslookup` — DNS lookup (simpler dig)
- `hostname` — get/set hostname
- `netstat` — connection statistics
- `ss` — socket statistics
- `traceroute` — trace route to host (basic ICMP TTL)

All commands use net_server IPC.

**Install in RamFs:**
```
/bin/ping → /usr/bin/sunlight-net-utils
/bin/ifconfig → /usr/bin/sunlight-net-utils
/bin/wget → /usr/bin/sunlight-net-utils
... etc
```

**Expected output:**
```
[NET]  sunlight-net-utils v0.1 loaded (ping, ifconfig, wget, curl, ...)
[NET]  Commands available: ping ifconfig wget curl dig hostname netstat
[NET]  OK
```

**Serial gate line:**
```
[NET]  Commands available: ping ifconfig wget curl dig hostname netstat
```

---

## Test Expectations & Gates

```
./tools/test.sh phase5.x.0   → [DHCP] Lease acquired: 10.0.2.15/24
./tools/test.sh phase5.x.1   → [DNS]  google.com → 142.250.x.x
./tools/test.sh phase5.x.2   → [TCP]  Connected (local XXXXX, remote X.X.X.X:80)
./tools/test.sh phase5.x.3   → [M3]   ping 8.8.8.8: SUCCESS 🌐
./tools/test.sh phase5.x.4   → [TLS]  Handshake OK: example.com (TLSv1.3)
./tools/test.sh phase5.x.5   → [UTIL] Commands available: ls cat cp mv...
./tools/test.sh phase5.x.6   → [NET]  Commands available: ping ifconfig wget...
```

---

## Regression Testing

After each phase, verify:
```bash
./tools/test.sh phase4.5     # Phase 4.5 still passes
./tools/test.sh phase5.0     # Phase 5.0 still passes
./tools/test.sh phase5.1     # Phase 5.1 still passes
... etc (all previous phases)
```

---

## Dependencies to Add

```toml
# sunlight-net/Cargo.toml
smoltcp = { version = "0.11", ... socket-dhcpv4, socket-dns, socket-icmpv4 ... }
rustls = { version = "0.23", default-features = false, features = ["ring", "tls12"] }
webpki-roots = "0.26"

# New crates:
# sunlight-utils (ZERO external deps — only sunlight-ipc)
# sunlight-net-utils (ZERO external deps — only sunlight-ipc)
```

---

## Workspace Members

Add to root `Cargo.toml`:
```toml
members = [
    # ... existing ...
    "sunlight-utils",       # new
    "sunlight-net-utils",   # new
]
```

---

## Implementation Order

1. ✅ Create roadmap (this file)
2. ✅ Create test expectation files (phase5x_0.expected ... phase5x_6.expected)
3. Real DHCP — implement, test phase5.x.0
4. Real DNS — implement, test phase5.x.1
5. Real TCP — implement, test phase5.x.2
6. Real ICMP ping — implement, test phase5.x.3 (M3!)
7. Real TLS — implement, test phase5.x.4
8. sunlight-utils — implement, test phase5.x.5
9. sunlight-net-utils — implement, test phase5.x.6
10. Verify no regressions on Phase 3.x/4.x/5.0-5.7
11. docs/PHASE_5X_SUMMARY.md

---

## Success Criteria

```
✓ Phase 5.x.0 gate PASSED  (DHCP real lease from 10.0.2.2)
✓ Phase 5.x.1 gate PASSED  (DNS resolves google.com)
✓ Phase 5.x.2 gate PASSED  (TCP connects to 93.184.216.34:80)
✓ Phase 5.x.3 gate PASSED  (ping 8.8.8.8 sends 4, receives 4) — M3 MILESTONE
✓ Phase 5.x.4 gate PASSED  (TLS handshake completes, validates certs)
✓ Phase 5.x.5 gate PASSED  (sunlight-utils commands registered)
✓ Phase 5.x.6 gate PASSED  (sunlight-net-utils commands registered)
✓ No regressions on Phase 3.x/4.x/5.0-5.7
```

---

## M3 Milestone

**Definition:** User can run `ping google.com` from SunlightOS shell and receive replies.

**Confirmation:**
```
root@sunlightos:~# ping google.com
PING google.com (142.250.185.46) 56 bytes of data
64 bytes from 142.250.185.46: icmp_seq=0 time=12ms
64 bytes from 142.250.185.46: icmp_seq=1 time=13ms
64 bytes from 142.250.185.46: icmp_seq=2 time=12ms
^C
--- google.com ping statistics ---
3 packets transmitted, 3 received, 0% loss

[M3]   ping google.com: SUCCESS 🌐
```
