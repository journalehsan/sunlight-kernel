//! networkd v0 — SunlightOS network management daemon.
//!
//! Small, reliable, userspace service that owns interface model, IP config policy,
//! priority, and default-route selection. It cooperates with deviced for discovery
//! and leaves actual frame/IP stack execution to net_server (for v0).
//!
//! Philosophy: no huge NetworkManager; split Wi-Fi/VPN/resolved later.

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait,
    nameserver_lookup_timeout, nameserver_register, pack_ipv4, pack_short_name, unpack_ipv4, AdminState,
    DevicedMsg, IfaceSummary, InterfaceId, InterfaceKind, IpConfigMode, IpcMsg,
    LinkState, NetworkdMsg,
};

const MAX_IFACES: usize = 8;

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
}

impl NetworkManager {
    const fn new() -> Self {
        Self {
            ifaces: [InterfaceRecord::empty(); MAX_IFACES],
            next_id: 1,
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
            rec.priority = -1; // not eligible
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
        self.ifaces
            .iter()
            .position(|i| i.occupied && i.id == id)
    }

    /// Discover network capable devices from deviced (best effort, never panic).
    fn discover_from_deviced(&mut self) {
        let Some(cap) = nameserver_lookup_timeout("deviced", 80) else {
            serial_println!("[NETD] deviced unavailable; skipping discovery");
            return;
        };

        // Enumerate devices
        let mut idx = 0usize;
        let mut found_net = 0usize;
        loop {
            let reply = ipc_call(
                cap,
                IpcMsg::with_label(DevicedMsg::LIST_DEVICES).word(0, idx as u64),
            );
            if reply.label != DevicedMsg::REPLY {
                break;
            }
            idx += 1;

            // Decode device record (see deviced reply_device_summary)
            let dev_id = reply.words[0];
            let name_u64 = reply.words[1];
            let driver_id = reply.words[2];
            let packed = reply.words[3];
            let kind = (packed & 0xff) as u64; // DeviceKind
            let state = ((packed >> 8) & 0xff) as u64;

            // Only Network kind (DeviceKind::Network == 2)
            if kind != 2 {
                continue;
            }

            // Map to a friendly name. For v0: ethN for discovered net devices.
            let base = b"eth";
            let mut name_buf = [0u8; 8];
            name_buf[..3].copy_from_slice(base);
            // simple index: eth0, eth1... (skip if lo collision)
            let n = found_net;
            if n < 10 {
                name_buf[3] = b'0' + n as u8;
            } else {
                // fall back
                name_buf[3] = b'x';
            }

            // If we already have an iface with this driver_id or similar name, update
            let existing = self
                .ifaces
                .iter()
                .position(|i| i.occupied && (i.driver_id == driver_id || i.driver_name == name_u64));

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
            rec.kind = InterfaceKind::VirtioNet; // Virtio is primary net today
            rec.driver_name = name_u64;
            rec.driver_id = driver_id;
            rec.admin = AdminState::Enabled;
            rec.link = if state == 2 { LinkState::Carrier } else { LinkState::NoCarrier };
            if !existing.is_some() {
                rec.mode = IpConfigMode::Dhcp;
                rec.priority = 100;
                rec.auto_connect = true;
            }
            rec.occupied = true;

            self.ifaces[slot] = rec;
            found_net += 1;
        }

        if found_net > 0 {
            serial_println!("[NETD] discovered {} network device(s) via deviced", found_net);
        }
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
        if i.admin != AdminState::Enabled || (i.link != LinkState::Up && i.link != LinkState::Carrier) {
            return false;
        }
        // has a gateway?
        i.gw != [0, 0, 0, 0]
    }

    fn compute_default_route(&self) -> Option<usize> {
        let mut best: Option<(i32, usize)> = None;
        for (slot, iface) in self.ifaces.iter().enumerate() {
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
            if let Some((bp, _)) = best {
                if iface.priority > bp {
                    best = Some((iface.priority, slot));
                }
            } else {
                best = Some((iface.priority, slot));
            }
        }
        best.map(|(_, s)| s)
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
                    return self.reply_summary(i);
                }
                seen += 1;
            }
        }
        IpcMsg::with_label(NetworkdMsg::ERROR).word(0, NetworkdMsg::ERR_NOT_FOUND).word(1, self.count() as u64)
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
            self.ifaces[slot].admin = if enabled { AdminState::Enabled } else { AdminState::Disabled };
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
            // Clear static fields; real acquisition would be triggered here in future.
            self.ifaces[slot].addr = [0; 4];
            self.ifaces[slot].prefix = 0;
            self.ifaces[slot].gw = [0; 4];
            serial_println!("[NETD] {} set to dhcp (policy recorded)", self.ifaces[slot].name_str());
            self.reply_summary(slot)
        } else {
            err_not_found()
        }
    }

    fn set_static(&mut self, key: u64, addr: [u8; 4], prefix: u8, gw: [u8; 4]) -> IpcMsg {
        if let Some(slot) = self.find_slot_by_key(key) {
            self.ifaces[slot].mode = IpConfigMode::Static;
            self.ifaces[slot].addr = addr;
            self.ifaces[slot].prefix = prefix;
            self.ifaces[slot].gw = gw;
            serial_println!(
                "[NETD] {} static {}.{}.{}.{}/{} gw {}.{}.{}.{}",
                self.ifaces[slot].name_str(),
                addr[0], addr[1], addr[2], addr[3], prefix,
                gw[0], gw[1], gw[2], gw[3]
            );
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
        self.discover_from_deviced();
        IpcMsg::with_label(NetworkdMsg::REPLY).word(0, 0).word(1, self.count() as u64)
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
}

fn err_not_found() -> IpcMsg {
    IpcMsg::with_label(NetworkdMsg::ERROR).word(0, NetworkdMsg::ERR_NOT_FOUND)
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
    mgr.discover_from_deviced();

    // If we discovered something with DHCP intent, note it (actual stack still in net_server v0)
    for iface in mgr.ifaces.iter() {
        if iface.occupied && iface.kind != InterfaceKind::Loopback {
            serial_println!(
                "[NETD] iface {} kind={:?} admin={:?} link={:?} mode={:?} prio={}",
                iface.name_str(), iface.kind, iface.admin, iface.link, iface.mode, iface.priority
            );
        }
    }

    let mut msg = ipc_recv(ep);
    loop {
        let reply = match msg.label {
            NetworkdMsg::LIST_INTERFACES => {
                let _ = mgr.refresh();
                mgr.list_one(msg.words[0] as usize)
            }
            NetworkdMsg::GET_INTERFACE => {
                let _ = mgr.refresh();
                mgr.get_by_key(msg.words[0])
            }
            NetworkdMsg::ENABLE_INTERFACE => mgr.set_admin(msg.words[0], true),
            NetworkdMsg::DISABLE_INTERFACE => mgr.set_admin(msg.words[0], false),
            NetworkdMsg::SET_DHCP => mgr.set_dhcp(msg.words[0]),
            NetworkdMsg::SET_STATIC_IPV4 => {
                // word0=key, word1=addr_packed, word2=prefix + gw_packed high?
                // Protocol: for simplicity, word1=addr, word2= (prefix<<24 | gw_packed low? Use two words.
                // Better compact: addr in w1, (prefix u8 | gw[4] packed somehow). For v0 use:
                // w1 = addr_packed, w2 = gw_packed, w3 = prefix
                let key = msg.words[0];
                let addr = unpack_ipv4(msg.words[1]);
                let gw = unpack_ipv4(msg.words[2]);
                let prefix = (msg.words[3] & 0xff) as u8;
                mgr.set_static(key, addr, prefix, gw)
            }
            NetworkdMsg::SET_PRIORITY => {
                let key = msg.words[0];
                let prio = msg.words[1] as i32; // sign extend? caller sends i32 as u64 bits
                mgr.set_priority(key, prio)
            }
            NetworkdMsg::SET_AUTO_CONNECT => {
                let key = msg.words[0];
                let ac = msg.words[1] != 0;
                mgr.set_auto_connect(key, ac)
            }
            NetworkdMsg::GET_DEFAULT_ROUTE => {
                let _ = mgr.refresh();
                mgr.get_default_route_reply()
            }
            NetworkdMsg::REFRESH => mgr.refresh(),
            _ => err_bad(),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}
