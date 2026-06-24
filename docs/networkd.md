# networkd v0

`networkd` is the first userspace network management daemon for SunlightOS. It is a small, service-oriented control plane responsible for:

- Discovering network-capable devices (via `deviced`)
- Maintaining a simple model of network interfaces (loopback + ethernet/virtio today)
- Exposing administrative and IP configuration policy (enable/disable, DHCP vs static, priority, auto-connect)
- Selecting the default route / gateway by priority among eligible interfaces
- Recording DNS servers learned from config (for future `resolved` handoff)

## What networkd manages (v0)

- Interface presence and basic metadata (name, kind, driver linkage)
- Admin state (enabled/disabled)
- Link state (approximated from driver state for v0)
- IPv4 configuration mode + values (DHCP or static address/prefix/gateway/dns)
- Per-interface priority for default-route selection
- Which interface currently provides the default route

## What networkd does NOT manage (by design)

- Raw frame TX/RX and the smoltcp stack (owned by `net_server`)
- Wi-Fi association, scanning, credentials (future dedicated Wi-Fi service)
- VPN tunnels and crypto (future VPN service)
- DNS caching, split-horizon, DoT/DoH (future `resolved`)
- 802.1X, captive portals, proxies, firewall rules, advanced routing metrics
- Persistent on-disk configuration (v0 is in-memory; later via KV/sm)

## Relationship to deviced

`networkd` is a **consumer** of `deviced`. At startup and on REFRESH it calls LIST_DEVICES and maps devices with `DeviceKind::Network` (or drivers carrying the `NETWORK` capability bit) into interfaces.

- If `deviced` is absent or slow, `networkd` logs and continues with only the loopback interface. No crash.
- Drivers (today: the virtio path inside `net_server`) still register themselves with `deviced` exactly as before.

## Relationship to net_server ("net")

`net_server` remains the executor that owns the live interface, sockets, and frame proxy syscalls (`NetTx`/`NetRx`).

- `networkd` provides the *desired* configuration and policy.
- For v0, `net_server` consults `networkd` (best-effort, with timeout) when answering `NetOp::GETIP` and when clients ask for current addressing. If `networkd` is unavailable or returns no useful data, `net_server` falls back to the previous QEMU user-net defaults. Existing downloads, TLS, and file serving continue to work.
- Reconfiguring the live smoltcp `Interface` IP addrs/routes from a `networkctl` change is intentionally deferred (would require coordinated restart of sockets or a clean "reconfigure" path). The model + CLI + IPC are the v0 deliverable.

## Interface model (v0)

See the types in `ipc/src/lib.rs` (`InterfaceKind`, `LinkState`, `AdminState`, `IpConfigMode`, `IfaceSummary`) and the internal `InterfaceRecord` in networkd.

Default priorities (wired first):

- Loopback: ineligible for default route (priority -1)
- Ethernet / VirtioNet: 100
- (Wireless later: ~80)
- (Tunnel/VPN later: often 200 when up)

Loopback is always present as `lo` with 127.0.0.1/8 once `networkd` starts.

## IPC protocol

Registered as `"networkd"`.

Core operations (register IPC, 4 words):

- LIST_INTERFACES / GET_INTERFACE
- ENABLE/DISABLE_INTERFACE
- SET_DHCP
- SET_STATIC_IPV4 (addr, gw, prefix packed)
- SET_PRIORITY, SET_AUTO_CONNECT
- GET_DEFAULT_ROUTE
- REFRESH

Replies use `NetworkdMsg::REPLY` with a compact `IfaceSummary` (or error codes).

See `NetworkdMsg` in `ipc/src/lib.rs` and the implementation in `services/sunlight-networkd`.

## CLI: networkctl

Minimal v0 surface (more commands are implemented):

```
networkctl list
networkctl status [iface]
networkctl up <iface>
networkctl down <iface>
networkctl dhcp <iface>
networkctl static <iface> <a.b.c.d/prefix> [gw <gw>] [dns <list>]
networkctl priority <iface> <n>
networkctl json
networkctl refresh
```

Example:

```
IFACE  KIND        ADMIN    LINK      MODE   ADDRESS         GATEWAY     PRIO  DEF
lo     Loopback    enabled  carrier   static 127.0.0.1/8     -             -1  no
eth0   VirtioNet   enabled  carrier   dhcp   10.0.2.15/24    10.0.2.2     100  yes
```

## Configuration

v0 keeps everything in RAM inside the daemon. A future revision can persist `InterfaceConfig` entries via `sunlight-kv` or StateFS. The in-memory shape already mirrors the suggested `NetworkdConfig` / `InterfaceConfig`.

## Future hooks (documented, not implemented)

- 802.1X / wired auth pluggable hooks
- Wi-Fi service integration (separate binary + capability set)
- VPN service (tunnels appear as high-priority interfaces)
- `resolved` handoff for per-iface + split DNS
- Per-profile configuration
- Captive portal detection + remediation
- Proxy environment variables
- Metric-based + policy routing
- Firewall / nftables or native packet filter integration
- Hotplug events from deviced v1 (shm path)

## v0 limitations & non-goals

- Not a NetworkManager clone.
- No persistent config.
- DHCP client lives in `sunlight-net` (smoltcp); full integration of live DHCP reacquisition under networkd control is future work.
- Only IPv4 for v0.
- No kernel changes required.

## Validation expectations

- `cargo check -p sunlight-networkd`
- `cargo check -p sunlight-net-server`
- Boot still succeeds with only loopback when no net device present.
- `networkctl list` shows `lo` and any discovered ethN.
- `networkctl status eth0` reflects mode/address/gw when set.
- Existing `fetch`, TLS, and `solar` networking paths continue to function (via fallback in net_server).
- `deviced` absence does not crash networkd.

This is the clean foundation for all future SunlightOS networking services.
