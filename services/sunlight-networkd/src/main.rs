//! networkd v0 — SunlightOS network management daemon.
//!
//! Small, reliable, userspace service that owns interface model, IP config policy,
//! priority, and default-route selection. It cooperates with deviced for discovery
//! and leaves actual frame/IP stack execution to net_server (for v0).
//!
//! Philosophy: no huge NetworkManager; split Wi-Fi/VPN/resolved later.

#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, Ordering};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_call_timeout, ipc_recv, ipc_reply_and_wait,
    monotonic_millis, nameserver_lookup_timeout, nameserver_register, pack_ipv4, pack_short_name,
    unpack_ipv4, AdminState, DevicedMsg, DnsSource, IfaceSummary, InterfaceId, InterfaceKind,
    IpConfigMode, IpcMsg, LinkState, NetOp, NetworkdMsg, ResolvedMsg,
};

const MAX_IFACES: usize = 8;
const NET_GETIP_TIMEOUT_MS: u64 = 20;
static ETHERNET_VISIBLE_LOGGED: AtomicBool = AtomicBool::new(false);

/// Exponential backoff steps (ms) when deviced is unavailable.
/// 1 s → 2 s → 5 s → 10 s → 30 s → 60 s (cap)
const BACKOFF_STEPS_MS: [u64; 6] = [1_000, 2_000, 5_000, 10_000, 30_000, 60_000];

/// How long to wait before re-checking deviced after a successful discovery.
/// Devices do not hot-plug after boot in this platform.
const DISCOVERY_STABLE_INTERVAL_MS: u64 = 30_000;

/// Metrics and rate-limit state for deviced discovery.
struct DiscoveryCache {
    /// Count of net devices found in the last successful run.
    last_net_count: usize,
    /// Driver IDs from the last successful run (for change detection).
    last_driver_ids: [u64; MAX_IFACES],
    /// Was deviced reachable on the last attempt?
    deviced_was_available: bool,
    /// How many consecutive failures since the last success.
    consecutive_failures: u32,
    /// monotonic_millis() timestamp: do not attempt discovery before this.
    next_retry_ms: u64,
    // --- observable metrics ---
    pub ipc_calls_total: u64,
    pub success_total: u64,
    pub timeout_total: u64,
    pub current_backoff_ms: u64,
}

impl DiscoveryCache {
    const fn new() -> Self {
        Self {
            last_net_count: 0,
            last_driver_ids: [0u64; MAX_IFACES],
            deviced_was_available: false,
            consecutive_failures: 0,
            next_retry_ms: 0,
            ipc_calls_total: 0,
            success_total: 0,
            timeout_total: 0,
            current_backoff_ms: 0,
        }
    }

    fn backoff_ms(&self) -> u64 {
        let idx = (self.consecutive_failures as usize).min(BACKOFF_STEPS_MS.len() - 1);
        BACKOFF_STEPS_MS[idx]
    }
}

#[macro_export]
macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

#[derive(Clone, Copy)]
struct InterfaceRecord {
    id: InterfaceId,
    name: [u8; 8], // fixed short name storage
    kind: InterfaceKind,
    driver_name: u64,
    driver_id: u64,
    mac: [u8; 6],
    admin: AdminState,
    link: LinkState,
    mode: IpConfigMode,
    addr: [u8; 4],
    prefix: u8,
    gw: [u8; 4],
    dns: [[u8; 4]; 2],
    priority: i32,
    auto_connect: bool,
    rx_bytes: u64,
    tx_bytes: u64,
    occupied: bool,
}

impl InterfaceRecord {
    const fn empty() -> Self {
        Self {
            id: 0,
            name: [0; 8],
            kind: InterfaceKind::Unknown,
            driver_name: 0,
            driver_id: 0,
            mac: [0; 6],
            admin: AdminState::Disabled,
            link: LinkState::Unknown,
            mode: IpConfigMode::None,
            addr: [0; 4],
            prefix: 0,
            gw: [0; 4],
            dns: [[0; 4]; 2],
            priority: 0,
            auto_connect: false,
            rx_bytes: 0,
            tx_bytes: 0,
            occupied: false,
        }
    }

    fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(8);
        // SAFETY: we control the bytes; ASCII from construction
        core::str::from_utf8(&self.name[..len]).unwrap_or("???")
    }
}

struct NetworkManager {
    ifaces: [InterfaceRecord; MAX_IFACES],
    next_id: InterfaceId,
    discovery: DiscoveryCache,
}

impl NetworkManager {
    const fn new() -> Self {
        Self {
            ifaces: [InterfaceRecord::empty(); MAX_IFACES],
            next_id: 1,
            discovery: DiscoveryCache::new(),
        }
    }

    fn seed_loopback(&mut self) {
        if self.find_by_name("lo").is_some() {
            return;
        }
        if let Some(slot) = self.free_slot() {
            let mut rec = InterfaceRecord::empty();
            rec.id = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            rec.name[..2].copy_from_slice(b"lo");
            rec.kind = InterfaceKind::Loopback;
            rec.admin = AdminState::Enabled;
            rec.link = LinkState::Up;
            rec.mode = IpConfigMode::Static;
            rec.addr = [127, 0, 0, 1];
            rec.prefix = 8;
            rec.priority = 32767; // conventional "low" for display; ineligibility via kind check
            rec.auto_connect = true;
            rec.occupied = true;
            self.ifaces[slot] = rec;
        }
    }

    fn free_slot(&self) -> Option<usize> {
        self.ifaces.iter().position(|i| !i.occupied)
    }

    fn find_by_name(&self, name: &str) -> Option<usize> {
        self.ifaces
            .iter()
            .position(|i| i.occupied && i.name_str() == name)
    }

    #[allow(dead_code)]
    fn find_by_id(&self, id: InterfaceId) -> Option<usize> {
        self.ifaces.iter().position(|i| i.occupied && i.id == id)
    }

    /// Discover network-capable devices from deviced.
    ///
    /// Uses a cache + exponential backoff so that:
    /// - Repeated calls after a stable discovery are no-ops until DISCOVERY_STABLE_INTERVAL_MS.
    /// - Failures back off exponentially (1 s → 2 s → 5 s → 10 s → 30 s → 60 s).
    /// - Logs only on meaningful state changes, not on every tick.
    fn discover_from_deviced(&mut self, force: bool) {
        let now = monotonic_millis();

        // Skip if still inside the backoff/stable window.
        if !force && now < self.discovery.next_retry_ms {
            return;
        }

        self.discovery.ipc_calls_total += 1;

        let Some(cap) = nameserver_lookup_timeout("deviced", 80) else {
            self.discovery.timeout_total += 1;
            // Log only on the first failure (deviced was available → now gone).
            if self.discovery.deviced_was_available || self.discovery.consecutive_failures == 0 {
                let backoff = self.discovery.backoff_ms();
                serial_println!("[NETD] deviced unavailable; next retry in {}ms", backoff);
                self.discovery.current_backoff_ms = backoff;
            }
            self.discovery.deviced_was_available = false;
            self.discovery.consecutive_failures =
                self.discovery.consecutive_failures.saturating_add(1);
            let backoff = self.discovery.backoff_ms();
            self.discovery.current_backoff_ms = backoff;
            self.discovery.next_retry_ms = now + backoff;
            return;
        };

        // Enumerate devices.
        let mut idx = 0usize;
        let mut found_net = 0usize;
        let mut new_driver_ids = [0u64; MAX_IFACES];
        let backend_info = sunlight_ipc::net_device_info();

        loop {
            let reply = ipc_call(
                cap,
                IpcMsg::with_label(DevicedMsg::LIST_DEVICES).word(0, idx as u64),
            );
            if reply.label != DevicedMsg::REPLY {
                break;
            }
            idx += 1;

            let name_u64 = reply.words[1];
            let driver_id = reply.words[2];
            let packed = reply.words[3];
            let kind = (packed & 0xff) as u64;
            let state = ((packed >> 8) & 0xff) as u64;

            // Only Network kind (DeviceKind::Network == 2).
            if kind != 2 {
                continue;
            }

            let reported_kind = if name_u64 == pack_short_name("vmxnet3") {
                InterfaceKind::Vmxnet3
            } else if name_u64 == pack_short_name("virtio") {
                InterfaceKind::VirtioNet
            } else {
                InterfaceKind::Ethernet
            };
            if let Some(info) = backend_info {
                if info.publishable()
                    && info.backend.map(|backend| backend.interface_kind()) != Some(reported_kind)
                {
                    // deviced can retain a registration from an older backend;
                    // the kernel active slot is authoritative.
                    continue;
                }
            }

            if found_net < MAX_IFACES {
                new_driver_ids[found_net] = driver_id;
            }

            let base = b"eth";
            let mut name_buf = [0u8; 8];
            name_buf[..3].copy_from_slice(base);
            let n = found_net;
            if n < 10 {
                name_buf[3] = b'0' + n as u8;
            } else {
                name_buf[3] = b'x';
            }

            let existing = self.ifaces.iter().position(|i| {
                i.occupied && (i.driver_id == driver_id || i.driver_name == name_u64)
            });

            let slot = if let Some(s) = existing {
                s
            } else if let Some(s) = self.free_slot() {
                s
            } else {
                continue;
            };

            let mut rec = if existing.is_some() {
                self.ifaces[slot]
            } else {
                let mut r = InterfaceRecord::empty();
                r.id = self.next_id;
                self.next_id = self.next_id.wrapping_add(1).max(1);
                r
            };

            rec.name = name_buf;
            rec.kind = reported_kind;
            rec.driver_name = name_u64;
            rec.driver_id = driver_id;
            rec.admin = AdminState::Enabled;
            if let Some(info) = backend_info {
                if info.present
                    && info.backend.map(|backend| backend.interface_kind()) == Some(rec.kind)
                {
                    rec.mac = info.mac;
                    rec.link = if info.link_up {
                        LinkState::Carrier
                    } else {
                        LinkState::NoCarrier
                    };
                } else {
                    rec.link = LinkState::NoCarrier;
                }
            } else {
                rec.link = if state == 2 {
                    LinkState::Carrier
                } else {
                    LinkState::NoCarrier
                };
            }
            if existing.is_none() {
                rec.mode = IpConfigMode::Dhcp;
                rec.priority = 100;
                rec.auto_connect = true;
            }
            rec.occupied = true;

            self.ifaces[slot] = rec;
            if existing.is_none() {
                serial_println!(
                    "[NETWORKD] published interface {} kind={}",
                    self.ifaces[slot].name_str(),
                    self.ifaces[slot].kind.label()
                );
            }
            found_net += 1;
        }

        // A forced refresh must not depend on deviced having observed a
        // backend that initialized after networkd startup. Publish directly
        // from the same authoritative kernel query used by the frame proxy.
        if let Some(info) = backend_info.filter(|info| info.publishable()) {
            let active_kind = info
                .backend
                .expect("publishable backend has a kind")
                .interface_kind();
            for record in &mut self.ifaces {
                if record.occupied
                    && record.kind != InterfaceKind::Loopback
                    && record.kind != active_kind
                {
                    *record = InterfaceRecord::empty();
                }
            }
            if found_net == 0 {
                let existing = self
                    .ifaces
                    .iter()
                    .position(|record| record.occupied && record.kind == active_kind);
                if let Some(slot) = existing.or_else(|| self.free_slot()) {
                    let is_new = existing.is_none();
                    let mut record = if is_new {
                        let mut record = InterfaceRecord::empty();
                        record.id = self.next_id;
                        self.next_id = self.next_id.wrapping_add(1).max(1);
                        record.mode = IpConfigMode::Dhcp;
                        record.priority = 100;
                        record.auto_connect = true;
                        record
                    } else {
                        self.ifaces[slot]
                    };
                    record.name = *b"eth0\0\0\0\0";
                    record.kind = active_kind;
                    record.driver_name = pack_short_name(
                        info.backend
                            .expect("publishable backend has a kind")
                            .driver_name(),
                    );
                    record.driver_id = 0;
                    record.admin = AdminState::Enabled;
                    record.link = if info.link_up {
                        LinkState::Carrier
                    } else {
                        LinkState::NoCarrier
                    };
                    record.mac = info.mac;
                    record.occupied = true;
                    self.ifaces[slot] = record;
                    found_net = 1;
                    new_driver_ids[0] = 0;
                    if is_new {
                        serial_println!(
                            "[NETWORKD] published interface eth0 kind={}",
                            active_kind.label()
                        );
                    }
                }
            }
            let _ = sunlight_ipc::net_mark_backend_event(
                sunlight_ipc::NetBackendEvent::InterfacePublished,
            );
        }

        // Detect what changed to decide what to log.
        let was_unavailable =
            !self.discovery.deviced_was_available && self.discovery.consecutive_failures > 0;
        let count_changed = found_net != self.discovery.last_net_count;
        let ids_changed = new_driver_ids[..found_net.min(MAX_IFACES)]
            != self.discovery.last_driver_ids[..found_net.min(MAX_IFACES)];

        if was_unavailable {
            serial_println!(
                "[NETD] deviced available again (was down for {} failure(s))",
                self.discovery.consecutive_failures
            );
        }
        if found_net > 0 && (count_changed || ids_changed || was_unavailable) {
            serial_println!(
                "[NETD] discovered {} network device(s) via deviced",
                found_net
            );
        }

        // Update cache state.
        self.discovery.last_net_count = found_net;
        self.discovery.last_driver_ids = new_driver_ids;
        self.discovery.deviced_was_available = true;
        self.discovery.consecutive_failures = 0;
        self.discovery.current_backoff_ms = 0;
        self.discovery.success_total += 1;
        // Suppress re-discovery until the stable interval elapses.
        self.discovery.next_retry_ms = now + DISCOVERY_STABLE_INTERVAL_MS;
    }

    fn count(&self) -> u16 {
        self.ifaces.iter().filter(|i| i.occupied).count() as u16
    }

    fn iface_to_summary(&self, idx: usize) -> IfaceSummary {
        let i = &self.ifaces[idx];
        IfaceSummary {
            id: i.id,
            name: pack_short_name(i.name_str()),
            kind: i.kind,
            admin: i.admin,
            link: i.link,
            mode: i.mode,
            addr: i.addr,
            prefix: i.prefix,
            gw: i.gw,
            dns: i.dns[0],
            priority: i.priority,
            is_default: self.is_default_route(idx),
            total: self.count(),
        }
    }

    fn is_default_route(&self, idx: usize) -> bool {
        if !self.ifaces[idx].occupied {
            return false;
        }
        let i = &self.ifaces[idx];
        if i.kind == InterfaceKind::Loopback || i.priority < 0 {
            return false;
        }
        if i.admin != AdminState::Enabled
            || (i.link != LinkState::Up && i.link != LinkState::Carrier)
        {
            return false;
        }
        // has a gateway?
        i.gw != [0, 0, 0, 0]
    }

    /// Small default route decision per v0.1 rules.
    /// Returns the slot index of the chosen interface, if any.
    fn compute_default_route(&self) -> Option<usize> {
        choose_default_slot(&self.ifaces)
    }

    #[allow(dead_code)]
    fn choose_default_id(&self) -> Option<InterfaceId> {
        self.compute_default_route()
            .map(|slot| self.ifaces[slot].id)
    }

    fn reply_summary(&self, slot: usize) -> IpcMsg {
        let s = self.iface_to_summary(slot);
        sunlight_ipc::pack_iface_summary(&s)
    }

    fn list_one(&self, requested: usize) -> IpcMsg {
        let mut seen = 0usize;
        for (i, rec) in self.ifaces.iter().enumerate() {
            if rec.occupied {
                if seen == requested {
                    if self.ifaces[i].kind != InterfaceKind::Loopback
                        && !ETHERNET_VISIBLE_LOGGED.swap(true, Ordering::AcqRel)
                    {
                        serial_println!(
                            "[NETWORKD] interface visible to networkctl name={}",
                            self.ifaces[i].name_str()
                        );
                    }
                    return self.reply_summary(i);
                }
                seen += 1;
            }
        }
        IpcMsg::with_label(NetworkdMsg::ERROR)
            .word(0, NetworkdMsg::ERR_NOT_FOUND)
            .word(1, self.count() as u64)
    }

    fn get_by_key(&self, key: u64) -> IpcMsg {
        // key can be id or name_u64
        for (i, rec) in self.ifaces.iter().enumerate() {
            if rec.occupied && (rec.id == key || pack_short_name(rec.name_str()) == key) {
                return self.reply_summary(i);
            }
        }
        IpcMsg::with_label(NetworkdMsg::ERROR).word(0, NetworkdMsg::ERR_NOT_FOUND)
    }

    fn set_admin(&mut self, key: u64, enabled: bool) -> IpcMsg {
        if let Some(slot) = self.find_slot_by_key(key) {
            self.ifaces[slot].admin = if enabled {
                AdminState::Enabled
            } else {
                AdminState::Disabled
            };
            // If we just disabled the current default, recompute is implicit on queries.
            self.reply_summary(slot)
        } else {
            err_not_found()
        }
    }

    fn find_slot_by_key(&self, key: u64) -> Option<usize> {
        for (i, rec) in self.ifaces.iter().enumerate() {
            if rec.occupied && (rec.id == key || pack_short_name(rec.name_str()) == key) {
                return Some(i);
            }
        }
        None
    }

    fn set_dhcp(&mut self, key: u64) -> IpcMsg {
        if let Some(slot) = self.find_slot_by_key(key) {
            self.ifaces[slot].mode = IpConfigMode::Dhcp;
            // Clear; future DHCP client or live ingest via net GETIP may repopulate.
            self.ifaces[slot].addr = [0; 4];
            self.ifaces[slot].prefix = 0;
            self.ifaces[slot].gw = [0; 4];
            // keep existing dns[ ] for now; they may be stale until re-acquire/ingest
            serial_println!(
                "[NETD] {} set to dhcp (policy recorded)",
                self.ifaces[slot].name_str()
            );
            self.reply_summary(slot)
        } else {
            err_not_found()
        }
    }

    fn set_static(
        &mut self,
        key: u64,
        addr: [u8; 4],
        prefix: u8,
        gw: [u8; 4],
        dns0: [u8; 4],
        dns1: [u8; 4],
    ) -> IpcMsg {
        if let Some(slot) = self.find_slot_by_key(key) {
            self.ifaces[slot].mode = IpConfigMode::Static;
            self.ifaces[slot].addr = addr;
            self.ifaces[slot].prefix = prefix;
            self.ifaces[slot].gw = gw;
            self.ifaces[slot].dns = [dns0, dns1];
            serial_println!(
                "[NETD] {} static {}.{}.{}.{}/{} gw {}.{}.{}.{}",
                self.ifaces[slot].name_str(),
                addr[0],
                addr[1],
                addr[2],
                addr[3],
                prefix,
                gw[0],
                gw[1],
                gw[2],
                gw[3]
            );
            self.handoff_dns_to_resolved(slot, DnsSource::Static);
            self.reply_summary(slot)
        } else {
            err_not_found()
        }
    }

    fn set_priority(&mut self, key: u64, prio: i32) -> IpcMsg {
        if let Some(slot) = self.find_slot_by_key(key) {
            self.ifaces[slot].priority = prio;
            self.reply_summary(slot)
        } else {
            err_not_found()
        }
    }

    fn set_auto_connect(&mut self, key: u64, ac: bool) -> IpcMsg {
        if let Some(slot) = self.find_slot_by_key(key) {
            self.ifaces[slot].auto_connect = ac;
            self.reply_summary(slot)
        } else {
            err_not_found()
        }
    }

    fn refresh(&mut self) -> IpcMsg {
        // Re-seed lo and re-discover (idempotent)
        self.seed_loopback();
        self.discover_from_deviced(true);
        // Ingest live addressing from the executing net stack (least invasive source of truth).
        // This populates addr/gw/prefix/dns for display and default route without duplicating config.
        self.ingest_live_from_net();
        let count = self.count();
        serial_println!("[NETWORKD] refresh complete interfaces={}", count);
        IpcMsg::with_label(NetworkdMsg::REPLY)
            .word(0, 0)
            .word(1, count as u64)
    }

    /// Query the 'net' service (if present) via its NetOp::GETIP and apply observed
    /// IPv4 config to the first non-loopback interface (typically eth0). This lets
    /// networkd reflect the actual stack state served to clients (DHCP or static).
    /// Degrades gracefully if net is absent or returns zeros.
    fn ingest_live_from_net(&mut self) {
        let Some(net_cap) = nameserver_lookup_timeout("net", 60) else {
            self.seed_qemu_dns_for_first_network_iface();
            return;
        };
        // Never block indefinitely here: request-time refreshes can be triggered by
        // clients already in a call path that involves net_server, so an unbounded
        // GETIP can form a circular wait (networkd -> net -> networkd).
        let Ok(reply) = ipc_call_timeout(
            net_cap,
            IpcMsg::with_label(NetOp::GETIP),
            NET_GETIP_TIMEOUT_MS,
        ) else {
            self.seed_qemu_dns_for_first_network_iface();
            return;
        };
        if reply.label != NetOp::GETIP {
            self.seed_qemu_dns_for_first_network_iface();
            return;
        }
        // net_server uses its own le byte-order packing for GETIP words:
        // w0=addr_le, w1=prefix, w2=gw_le, w3=dns_le
        let addr = [
            (reply.words[0] & 0xff) as u8,
            ((reply.words[0] >> 8) & 0xff) as u8,
            ((reply.words[0] >> 16) & 0xff) as u8,
            ((reply.words[0] >> 24) & 0xff) as u8,
        ];
        let prefix = (reply.words[1] & 0xff) as u8;
        let gw = [
            (reply.words[2] & 0xff) as u8,
            ((reply.words[2] >> 8) & 0xff) as u8,
            ((reply.words[2] >> 16) & 0xff) as u8,
            ((reply.words[2] >> 24) & 0xff) as u8,
        ];
        let dns = [
            (reply.words[3] & 0xff) as u8,
            ((reply.words[3] >> 8) & 0xff) as u8,
            ((reply.words[3] >> 16) & 0xff) as u8,
            ((reply.words[3] >> 24) & 0xff) as u8,
        ];

        if addr == [0, 0, 0, 0] {
            self.seed_qemu_dns_for_first_network_iface();
            return; // nothing useful yet (e.g. before stack up); degrade ok
        }

        // Apply to the first eligible non-lo interface (stable by discovery order)
        if let Some(slot) = self
            .ifaces
            .iter()
            .position(|i| i.occupied && i.kind != InterfaceKind::Loopback)
        {
            // Preserve mode intent (Dhcp vs Static) if already set; only fill numbers.
            // If mode is None, treat observed as DHCP (common for QEMU path).
            if self.ifaces[slot].mode == IpConfigMode::None {
                self.ifaces[slot].mode = IpConfigMode::Dhcp;
            }
            self.ifaces[slot].addr = addr;
            self.ifaces[slot].prefix = if prefix == 0 { 24 } else { prefix };
            self.ifaces[slot].gw = gw;
            if dns != [0, 0, 0, 0] {
                self.ifaces[slot].dns[0] = dns;
            }
            serial_println!(
                "[NETD] ingested live ip for {}: {}.{}.{}.{}/{} gw {}.{}.{}.{}",
                self.ifaces[slot].name_str(),
                addr[0],
                addr[1],
                addr[2],
                addr[3],
                self.ifaces[slot].prefix,
                gw[0],
                gw[1],
                gw[2],
                gw[3]
            );
            let source = if self.ifaces[slot].mode == IpConfigMode::Static {
                DnsSource::Static
            } else {
                DnsSource::Dhcp
            };
            self.handoff_dns_to_resolved(slot, source);
        }
    }

    fn seed_qemu_dns_for_first_network_iface(&mut self) {
        if let Some(slot) = self
            .ifaces
            .iter()
            .position(|i| i.occupied && i.kind != InterfaceKind::Loopback)
        {
            if self.ifaces[slot].dns[0] == [0, 0, 0, 0] {
                self.ifaces[slot].dns[0] = [10, 0, 2, 3];
            }
            if self.ifaces[slot].mode == IpConfigMode::None {
                self.ifaces[slot].mode = IpConfigMode::Dhcp;
            }
            self.handoff_dns_to_resolved(slot, DnsSource::Dhcp);
        }
    }

    fn get_default_route_reply(&self) -> IpcMsg {
        if let Some(slot) = self.compute_default_route() {
            let i = &self.ifaces[slot];
            IpcMsg::with_label(NetworkdMsg::REPLY)
                .word(0, i.id)
                .word(1, pack_short_name(i.name_str()))
                .word(2, pack_ipv4(i.addr))
                .word(3, pack_ipv4(i.gw))
        } else {
            IpcMsg::with_label(NetworkdMsg::REPLY).word(0, 0) // id=0 means none
        }
    }

    /// Best-effort handoff of per-iface DNS to resolved.
    /// If resolved is absent, log and continue (graceful).
    fn handoff_dns_to_resolved(&self, slot: usize, source: DnsSource) {
        if !self.ifaces[slot].occupied {
            return;
        }
        let i = &self.ifaces[slot];
        if i.dns[0] == [0, 0, 0, 0] {
            return;
        }
        let Some(res_cap) = nameserver_lookup_timeout("resolved", 40) else {
            // resolved not up yet or unavailable; networkd keeps working
            return;
        };
        let iface_p = pack_short_name(i.name_str());
        let mut msg = IpcMsg::with_label(ResolvedMsg::UPDATE_FROM_NETWORKD)
            .word(0, iface_p)
            .word(1, source as u64)
            .word(2, pack_ipv4(i.dns[0]));
        if i.dns[1] != [0, 0, 0, 0] {
            msg = msg.word(3, pack_ipv4(i.dns[1]));
        }
        let _ = ipc_call(res_cap, msg);
        // fire-and-forget; no hard dep
    }
}

fn err_not_found() -> IpcMsg {
    IpcMsg::with_label(NetworkdMsg::ERROR).word(0, NetworkdMsg::ERR_NOT_FOUND)
}

/// choose_default_interface selects the best default route provider from a slice of records.
/// v0.1 rules:
/// - Loopback never chosen
/// - Disabled interfaces never chosen
/// - No carrier never chosen
/// - No gateway normally not chosen
/// - Highest priority wins
/// - On priority tie, prefer lower id (or stable discovery order via slot)
#[allow(dead_code)]
fn choose_default_interface(interfaces: &[InterfaceRecord]) -> Option<InterfaceId> {
    choose_default_slot(interfaces).map(|slot| interfaces[slot].id)
}

fn choose_default_slot(ifaces: &[InterfaceRecord]) -> Option<usize> {
    let mut best: Option<(i32, InterfaceId, usize)> = None; // (prio, id, slot)
    for (slot, iface) in ifaces.iter().enumerate() {
        if !iface.occupied {
            continue;
        }
        if iface.kind == InterfaceKind::Loopback || iface.priority < 0 {
            continue;
        }
        if iface.admin != AdminState::Enabled {
            continue;
        }
        if iface.link != LinkState::Up && iface.link != LinkState::Carrier {
            continue;
        }
        if iface.gw == [0, 0, 0, 0] {
            continue;
        }
        let candidate = (iface.priority, iface.id, slot);
        match best {
            None => best = Some(candidate),
            Some((bp, bid, _bslot)) => {
                if iface.priority > bp || (iface.priority == bp && iface.id < bid) {
                    best = Some(candidate);
                }
            }
        }
    }
    best.map(|(_, _, s)| s)
}

fn err_bad() -> IpcMsg {
    IpcMsg::with_label(NetworkdMsg::ERROR).word(0, NetworkdMsg::ERR_BAD_REQUEST)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[NETD] PANIC");
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[NETD] networkd v0 starting");

    let ep = endpoint_create();
    nameserver_register("networkd", ep);
    serial_println!("[NETD] registered as 'networkd'");

    let mut mgr = NetworkManager::new();
    mgr.seed_loopback();
    mgr.discover_from_deviced(true);
    mgr.ingest_live_from_net();

    // If we discovered something with DHCP intent, note it (actual stack still in net_server v0)
    for iface in mgr.ifaces.iter() {
        if iface.occupied && iface.kind != InterfaceKind::Loopback {
            serial_println!(
                "[NETD] iface {} kind={:?} admin={:?} link={:?} mode={:?} prio={}",
                iface.name_str(),
                iface.kind,
                iface.admin,
                iface.link,
                iface.mode,
                iface.priority
            );
        }
    }

    let mut msg = ipc_recv(ep);
    loop {
        let reply = match msg.label {
            // Read-only queries: serve cached state — no discovery refresh.
            // GET_DEFAULT_ROUTE is the hot path (net_server calls it on every GETIP);
            // triggering discover_from_deviced() there caused circular IPC churn.
            NetworkdMsg::LIST_INTERFACES => mgr.list_one(msg.words[0] as usize),
            NetworkdMsg::GET_INTERFACE => mgr.get_by_key(msg.words[0]),
            NetworkdMsg::GET_DEFAULT_ROUTE => mgr.get_default_route_reply(),

            // State-changing operations: ingest live IP data (cheap, no discovery).
            NetworkdMsg::ENABLE_INTERFACE => {
                mgr.ingest_live_from_net();
                mgr.set_admin(msg.words[0], true)
            }
            NetworkdMsg::DISABLE_INTERFACE => {
                mgr.ingest_live_from_net();
                mgr.set_admin(msg.words[0], false)
            }
            NetworkdMsg::SET_DHCP => {
                mgr.ingest_live_from_net();
                mgr.set_dhcp(msg.words[0])
            }
            NetworkdMsg::SET_STATIC_IPV4 => {
                // w0=key, w1=addr_p, w2=gw_p, w3=prefix (low), w4=dns0_p (opt), w5=dns1_p (opt)
                let key = msg.words[0];
                let addr = unpack_ipv4(msg.words[1]);
                let gw = unpack_ipv4(msg.words[2]);
                let prefix = (msg.words[3] & 0xff) as u8;
                let dns0 = if msg.word_count > 4 {
                    unpack_ipv4(msg.words[4])
                } else {
                    [0u8; 4]
                };
                let dns1 = if msg.word_count > 5 {
                    unpack_ipv4(msg.words[5])
                } else {
                    [0u8; 4]
                };
                mgr.set_static(key, addr, prefix, gw, dns0, dns1)
            }
            NetworkdMsg::SET_PRIORITY => {
                let key = msg.words[0];
                let prio = msg.words[1] as i32;
                mgr.set_priority(key, prio)
            }
            NetworkdMsg::SET_AUTO_CONNECT => {
                let key = msg.words[0];
                let ac = msg.words[1] != 0;
                mgr.set_auto_connect(key, ac)
            }

            // Explicit refresh: run full discovery (cache/backoff still apply internally).
            NetworkdMsg::REFRESH => {
                let r = mgr.refresh();
                serial_println!(
                    "[NETD] metrics: ipc_calls={} success={} timeout={} backoff_ms={}",
                    mgr.discovery.ipc_calls_total,
                    mgr.discovery.success_total,
                    mgr.discovery.timeout_total,
                    mgr.discovery.current_backoff_ms,
                );
                r
            }

            _ => err_bad(),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}
