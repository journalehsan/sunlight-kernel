//! resolved v0 — SunlightOS DNS resolver configuration service.
//!
//! Owns the system DNS server list and search domains.
//! Generates /etc/resolv.conf compatibility view.
//! networkd pushes DHCP/static DNS servers here via UpdateFromNetworkd.
//!
//! v0 policy (simple, no DoT/DoH/split):
//!   1. If manual servers set, use them.
//!   2. Else if DHCP/static servers from networkd, use them (mark Dhcp/Static).
//!   3. Else use system default (OpenDNS).
//!
//! /etc/resolv.conf is a generated facade. Direct writes are not supported
//! for unprivileged processes in v0.

#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register,
    pack_dns_server_summary, pack_ipv4, pack_short_name, unpack_ipv4, DnsServerSummary, DnsSource,
    IpcMsg, ResolvedMsg,
};

const MAX_SERVERS: usize = 8;
const SYSTEM_DEFAULT_DNS: [[u8; 4]; 2] = [[208, 67, 222, 222], [208, 67, 220, 220]];

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
struct DnsServerRec {
    addr: [u8; 4],
    source: DnsSource,
    iface: [u8; 8], // packed name or zeros
    priority: i32,
    occupied: bool,
}

impl DnsServerRec {
    const fn empty() -> Self {
        Self {
            addr: [0; 4],
            source: DnsSource::Unknown,
            iface: [0; 8],
            priority: 0,
            occupied: false,
        }
    }
    fn iface_str(&self) -> &str {
        let len = self.iface.iter().position(|&b| b == 0).unwrap_or(8);
        core::str::from_utf8(&self.iface[..len]).unwrap_or("")
    }
}

struct ResolverState {
    servers: [DnsServerRec; MAX_SERVERS],
    _search: [u8; 64], // simple single search domain for v0
    generated_at_ms: u64,
    manual_override: bool, // if true, ignore lower sources until cleared
}

impl ResolverState {
    const fn new() -> Self {
        Self {
            servers: [DnsServerRec::empty(); MAX_SERVERS],
            _search: [0; 64],
            generated_at_ms: 0,
            manual_override: false,
        }
    }

    fn seed_system_default(&mut self) {
        self.clear_all_internal();
        for &addr in &SYSTEM_DEFAULT_DNS {
            self.add_server_internal(addr, DnsSource::SystemDefault, b"", 100);
        }
        self.manual_override = false;
        self.generated_at_ms = sunlight_ipc::monotonic_millis();
        serial_println!("[RESOLVED] seeded system default OpenDNS");
    }

    fn clear_all_internal(&mut self) {
        for s in &mut self.servers {
            *s = DnsServerRec::empty();
        }
    }

    fn count(&self) -> usize {
        self.servers.iter().filter(|s| s.occupied).count()
    }

    fn add_server_internal(
        &mut self,
        addr: [u8; 4],
        source: DnsSource,
        iface: &[u8],
        prio: i32,
    ) -> bool {
        if addr == [0, 0, 0, 0] {
            return false;
        }
        // de-dup
        for s in &mut self.servers {
            if s.occupied && s.addr == addr {
                s.source = source;
                s.priority = prio;
                if !iface.is_empty() {
                    s.iface = [0; 8];
                    let n = iface.len().min(8);
                    s.iface[..n].copy_from_slice(&iface[..n]);
                }
                return true;
            }
        }
        if let Some(slot) = self.servers.iter().position(|s| !s.occupied) {
            self.servers[slot].addr = addr;
            self.servers[slot].source = source;
            self.servers[slot].priority = prio;
            self.servers[slot].occupied = true;
            if !iface.is_empty() {
                let n = iface.len().min(8);
                self.servers[slot].iface[..n].copy_from_slice(&iface[..n]);
            }
            true
        } else {
            false
        }
    }

    fn set_servers_from_list(&mut self, addrs: &[[u8; 4]], source: DnsSource, iface: &[u8]) {
        self.clear_all_internal();
        self.manual_override = matches!(source, DnsSource::Manual);
        for &a in addrs {
            if a != [0, 0, 0, 0] {
                let _ = self.add_server_internal(
                    a,
                    source,
                    iface,
                    if source == DnsSource::Manual {
                        1000
                    } else {
                        100
                    },
                );
            }
        }
        self.generated_at_ms = sunlight_ipc::monotonic_millis();
    }

    fn effective_servers(&self) -> heapless::Vec<DnsServerRec, MAX_SERVERS> {
        let mut out: heapless::Vec<DnsServerRec, MAX_SERVERS> = heapless::Vec::new();
        if self.manual_override {
            for s in &self.servers {
                if s.occupied && matches!(s.source, DnsSource::Manual) {
                    let _ = out.push(*s);
                }
            }
        }
        if out.is_empty() {
            for s in &self.servers {
                if s.occupied {
                    let _ = out.push(*s);
                }
            }
        }
        // If still empty, caller should have seeded; return whatever we have.
        out
    }

    fn first_addr(&self) -> [u8; 4] {
        let eff = self.effective_servers();
        if let Some(s) = eff.first() {
            s.addr
        } else {
            SYSTEM_DEFAULT_DNS[0]
        }
    }
}

fn err_bad() -> IpcMsg {
    IpcMsg::with_label(ResolvedMsg::ERROR).word(0, ResolvedMsg::ERR_BAD_REQUEST)
}

fn err_not_found() -> IpcMsg {
    IpcMsg::with_label(ResolvedMsg::ERROR).word(0, ResolvedMsg::ERR_NOT_FOUND)
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[RESOLVED] PANIC");
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    serial_println!("[RESOLVED] resolved v0 starting");

    let ep = endpoint_create();
    nameserver_register("resolved", ep);
    serial_println!("[RESOLVED] registered as 'resolved'");

    let mut state = ResolverState::new();
    state.seed_system_default();

    let mut msg = ipc_recv(ep);
    loop {
        let reply = match msg.label {
            ResolvedMsg::GET_CONFIG => {
                let n = state.count() as u64;
                let first = state.first_addr();
                IpcMsg::with_label(ResolvedMsg::REPLY)
                    .word(0, n)
                    .word(1, pack_ipv4(first))
                    .word(2, if state.manual_override { 1 } else { 0 })
            }
            ResolvedMsg::GET_SERVER => {
                let idx = msg.words[0] as usize;
                let mut seen = 0usize;
                let mut rep: Option<IpcMsg> = None;
                for s in &state.servers {
                    if s.occupied {
                        if seen == idx {
                            let sum = DnsServerSummary {
                                address: s.addr,
                                source: s.source,
                                priority: s.priority,
                                iface: pack_short_name(s.iface_str()),
                                total: state.count() as u16,
                            };
                            rep = Some(pack_dns_server_summary(&sum));
                            break;
                        }
                        seen += 1;
                    }
                }
                rep.unwrap_or_else(err_not_found)
            }
            ResolvedMsg::SET_SERVERS => {
                // w0.. up to 4 addrs packed (caller packs via pack_ipv4)
                let mut addrs: [[u8; 4]; 4] = [[0; 4]; 4];
                let mut n = 0usize;
                for i in 0..4 {
                    if msg.word_count > i as u32 {
                        let a = unpack_ipv4(msg.words[i]);
                        if a != [0, 0, 0, 0] {
                            addrs[n] = a;
                            n += 1;
                        }
                    }
                }
                state.set_servers_from_list(&addrs[..n], DnsSource::Manual, b"");
                serial_println!("[RESOLVED] SET_SERVERS manual count={}", n);
                IpcMsg::with_label(ResolvedMsg::REPLY).word(0, n as u64)
            }
            ResolvedMsg::ADD_SERVER => {
                let addr = unpack_ipv4(msg.words[0]);
                let src = DnsSource::from_u64(msg.words[1]);
                let ok = state.add_server_internal(
                    addr,
                    src,
                    b"",
                    if src == DnsSource::Manual { 1000 } else { 100 },
                );
                if ok {
                    state.manual_override = matches!(src, DnsSource::Manual);
                }
                IpcMsg::with_label(ResolvedMsg::REPLY).word(0, if ok { 1 } else { 0 })
            }
            ResolvedMsg::CLEAR_SERVERS => {
                state.clear_all_internal();
                state.manual_override = false;
                state.seed_system_default();
                IpcMsg::with_label(ResolvedMsg::REPLY).word(0, 0)
            }
            ResolvedMsg::USE_SYSTEM_DEFAULT => {
                state.seed_system_default();
                IpcMsg::with_label(ResolvedMsg::REPLY).word(0, 2)
            }
            ResolvedMsg::UPDATE_FROM_NETWORKD => {
                // w0 = iface name packed (u64), w1 = source tag, w2..wN = dns addrs packed.
                // For compatibility with early callers, an invalid/zero source tag is treated as Dhcp.
                let iface_u = msg.words[0];
                let mut iface_buf = [0u8; 8];
                for i in 0..8 {
                    iface_buf[i] = ((iface_u >> (i * 8)) & 0xff) as u8;
                }
                let source_tag = DnsSource::from_u64(msg.words[1]);
                let old_layout_first_addr = unpack_ipv4(msg.words[1]);
                let old_layout = msg.word_count > 1
                    && source_tag == DnsSource::Unknown
                    && old_layout_first_addr != [0, 0, 0, 0];
                let source = match source_tag {
                    DnsSource::Static => DnsSource::Static,
                    DnsSource::Vpn => DnsSource::Vpn,
                    DnsSource::Manual => DnsSource::Manual,
                    DnsSource::SystemDefault => DnsSource::SystemDefault,
                    _ => DnsSource::Dhcp,
                };
                let mut addrs: [[u8; 4]; 4] = [[0; 4]; 4];
                let mut n = 0usize;
                let first_addr_word = if old_layout { 1 } else { 2 };
                for i in first_addr_word..(first_addr_word + 4) {
                    if msg.word_count > i as u32 {
                        let a = unpack_ipv4(msg.words[i]);
                        if a != [0, 0, 0, 0] {
                            addrs[n] = a;
                            n += 1;
                        }
                    }
                }
                if !state.manual_override && n > 0 {
                    // DHCP/Static from networkd take precedence over default.
                    state.set_servers_from_list(&addrs[..n], source, &iface_buf);
                    let iface_len = iface_buf.iter().position(|&b| b == 0).unwrap_or(8);
                    serial_println!(
                        "[RESOLVED] UPDATE_FROM_NETWORKD iface={} source={:?} count={}",
                        core::str::from_utf8(&iface_buf[..iface_len]).unwrap_or("?"),
                        source,
                        n
                    );
                } else if state.manual_override {
                    serial_println!("[RESOLVED] UPDATE_FROM_NETWORKD ignored (manual override)");
                }
                IpcMsg::with_label(ResolvedMsg::REPLY).word(0, n as u64)
            }
            ResolvedMsg::RENDER_RESOLV_CONF => {
                // Return up to 3 servers packed for vfs/CLI to format.
                // w0=count, w1=addr0, w2=addr1, w3=addr2
                let eff = state.effective_servers();
                let mut r = IpcMsg::with_label(ResolvedMsg::REPLY).word(0, eff.len() as u64);
                for (i, s) in eff.iter().take(3).enumerate() {
                    r = r.word(1 + i, pack_ipv4(s.addr));
                }
                r
            }
            ResolvedMsg::RESOLVE_NAME => {
                // v0 stub: do not implement full resolve here. net owns real DNS.
                // Return 0 to signal "use net path".
                IpcMsg::with_label(ResolvedMsg::REPLY).word(0, 0)
            }
            ResolvedMsg::FLUSH_CACHE => {
                // v0 stub
                IpcMsg::with_label(ResolvedMsg::REPLY).word(0, 1)
            }
            _ => err_bad(),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}
