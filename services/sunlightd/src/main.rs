//! sunlightd - SunlightOS service supervisor daemon
//! Reads .service and .socket unit files and manages process lifecycle

#![no_std]
#![no_main]

extern crate alloc;

struct BumpAllocator;

unsafe impl core::alloc::GlobalAlloc for BumpAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        static mut HEAP: [u8; 65536] = [0; 65536];
        static mut NEXT: usize = 0;
        let start = NEXT;
        let align = layout.align();
        let aligned = (start + align - 1) & !(align - 1);
        let end = aligned + layout.size();
        if end > HEAP.len() {
            return core::ptr::null_mut();
        }
        NEXT = end;
        HEAP.as_mut_ptr().add(aligned)
    }
    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: core::alloc::Layout) {}
}

#[global_allocator]
static BUMP: BumpAllocator = BumpAllocator;

mod graph;
mod ipc;
mod journal;
mod socket_act;
mod supervisor;
mod unit;

use graph::DepGraph;
use ipc::{extract_unit_name, ListEntry, StatusReply, SunlightdOp};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait, nameserver_lookup,
    nameserver_register, CapabilityToken, IpcMsg, SpawnRequest,
};
use supervisor::{ServiceEntry, ServiceState};
use unit::{parse_service_unit, ServiceUnit, SocketUnit, MAX_UNITS};

macro_rules! serial_println {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let mut buf = heapless::String::<256>::new();
        let _ = write!(&mut buf, $($arg)*);
        debug_log(&buf);
    }};
}

struct ServiceTable {
    services: [Option<ServiceEntry>; MAX_UNITS],
    count: usize,
}

impl ServiceTable {
    fn new() -> Self {
        Self {
            services: [const { None }; MAX_UNITS],
            count: 0,
        }
    }

    fn add(&mut self, unit: ServiceUnit) -> Result<usize, &'static str> {
        if self.count >= MAX_UNITS {
            return Err("Service table full");
        }
        let idx = self.count;
        self.services[idx] = Some(ServiceEntry::new(unit));
        self.count += 1;
        Ok(idx)
    }

    /// Find service index by binary name extracted from ExecStart path.
    fn find_by_name(&self, name: &str) -> Option<usize> {
        for i in 0..self.count {
            if let Some(ref entry) = self.services[i] {
                let bin = binary_name_of(&entry.unit.exec_start);
                if bin == name {
                    return Some(i);
                }
            }
        }
        None
    }

    fn get_mut(&mut self, idx: usize) -> Option<&mut ServiceEntry> {
        if idx < self.count {
            self.services[idx].as_mut()
        } else {
            None
        }
    }

    fn get(&self, idx: usize) -> Option<&ServiceEntry> {
        if idx < self.count {
            self.services[idx].as_ref()
        } else {
            None
        }
    }
}

/// Extract binary name from an ExecStart path like "/sbin/niced" → "niced".
fn binary_name_of(exec_start: &str) -> &str {
    if let Some(pos) = exec_start.rfind('/') {
        &exec_start[(pos + 1)..]
    } else {
        exec_start
    }
}

/// Load unit files for services managed by sunlightd.
/// kernel → init → sunlightd; vfs/net/tty are managed by kernel/init, not here.
fn load_units() -> (ServiceTable, heapless::Vec<SocketUnit, 8>) {
    let mut services = ServiceTable::new();
    let sockets: heapless::Vec<SocketUnit, 8> = heapless::Vec::new();

    // solar.service — disabled by default; start with: sunlightctl start solar
    let solar_service = r#"[Unit]
Description=Solar HTTP Server
After=net_server

[Service]
Type=simple
ExecStart=/sbin/solar
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(solar_service.as_bytes()) {
        if let Ok(idx) = services.add(unit) {
            if let Some(entry) = services.get_mut(idx) {
                entry.enabled = false;
            }
        }
    }

    // timezone_service.service
    let tz_service = r#"[Unit]
Description=SunlightOS Timezone Service

[Service]
Type=simple
ExecStart=/sbin/timezone_service
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(tz_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // niced.service
    let niced_service = r#"[Unit]
Description=SunlightOS Nice Priority Daemon

[Service]
Type=simple
ExecStart=/sbin/niced
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(niced_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // gcd.service
    let gcd_service = r#"[Unit]
Description=SunlightOS Generic Control Daemon

[Service]
Type=simple
ExecStart=/sbin/gcd
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(gcd_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // uac_service.service
    let uac_service = r#"[Unit]
Description=SunlightOS User Access Control Service

[Service]
Type=simple
ExecStart=/sbin/uac_service
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(uac_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // sunlight-sm.service - Storage Manager for controlled writes to protected paths (whitelist)
    let sm_service = r#"[Unit]
Description=SunlightOS Storage Manager (controlled persistent writes)
After=uac_service.service

[Service]
Type=simple
ExecStart=/sbin/sunlight-sm
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(sm_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // sunlight-kv.service - persistent key-value storage (append-only log backend)
    // NOTE: sunlight-kv now delegates protected writes to sunlight-sm ("sm")
    let kv_service = r#"[Unit]
Description=SunlightOS Key-Value Storage Daemon
After=sm.service

[Service]
Type=simple
ExecStart=/sbin/sunlight-kv
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(kv_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // rand_service.service - ChaCha20 CSPRNG (libc crypto getrandom routes here).
    // MUST start before sunlight-tls: TLS handshakes pull randomness from it.
    let rand_service = r#"[Unit]
Description=SunlightOS Random Service

[Service]
Type=simple
ExecStart=/sbin/rand_service
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(rand_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // sunlight-tls.service - TLS service (rustls over IPC, certs via sunlight-kv,
    // handshake randomness via rand_service). Starts after kv + rand_service.
    let tls_service = r#"[Unit]
Description=SunlightOS TLS Service

[Service]
Type=simple
ExecStart=/sbin/sunlight-tls
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(tls_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // sunlight-thumbd.service — disabled: thumbnail pre-warming panics on
    // malformed simg data and leaves the process in a zombie-Ready loop.
    let thumbd_service = r#"[Unit]
Description=SunlightOS Thumbnail Daemon

[Service]
Type=simple
ExecStart=/sbin/sunlight-thumbd
Restart=no
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(thumbd_service.as_bytes()) {
        if let Ok(idx) = services.add(unit) {
            if let Some(entry) = services.get_mut(idx) {
                entry.enabled = false;
            }
        }
    }

    (services, sockets)
}

/// Build dependency graph and return topological order
fn build_dep_graph(
    services: &ServiceTable,
) -> Result<heapless::Vec<usize, MAX_UNITS>, &'static str> {
    let mut graph = DepGraph::new();

    for i in 0..services.count {
        if let Some(ref entry) = services.services[i] {
            let mut unit_name = heapless::String::<64>::new();
            let _ = unit_name.push_str(binary_name_of(&entry.unit.exec_start));
            let _ = unit_name.push_str(".service");
            graph.add_unit(&unit_name).map_err(|_| "Graph add failed")?;
        }
    }

    for i in 0..services.count {
        if let Some(ref entry) = services.services[i] {
            let mut unit_name = heapless::String::<64>::new();
            let _ = unit_name.push_str(binary_name_of(&entry.unit.exec_start));
            let _ = unit_name.push_str(".service");

            for dep in &entry.unit.after {
                let _ = graph.add_edge(dep, &unit_name);
            }
        }
    }

    graph
        .topological_order()
        .map_err(|_| "Topological sort failed")
}

/// Spawn a named daemon via the kernel spawn capability.
fn spawn_named(spawn_cap: CapabilityToken, path: &str, name: &str) -> Result<u32, &'static str> {
    use sunlight_ipc::SpawnMsg;

    let req = SpawnRequest::new(path, name);
    let mut msg = IpcMsg::empty();
    req.pack_into(&mut msg);

    let reply = ipc_call(spawn_cap, msg);
    if reply.label == SpawnMsg::REPLY {
        Ok(reply.words[0] as u32)
    } else {
        Err("Spawn failed")
    }
}

fn wait_for_spawn_cap() -> CapabilityToken {
    loop {
        if let Some(cap) = nameserver_lookup("spawn") {
            return cap;
        }
        sunlight_ipc::process_yield();
    }
}

/// Reply label codes for control operations.
const REPLY_OK: u64 = 1;
const REPLY_NOP: u64 = 2; // already in desired state (enable/disable no-op)
const REPLY_ERR: u64 = 0xff;

/// Handle control IPC messages
fn handle_control_message(
    msg: &IpcMsg,
    services: &mut ServiceTable,
    spawn_cap: CapabilityToken,
) -> IpcMsg {
    let mut reply = IpcMsg::empty();

    let op = match SunlightdOp::from_u32(msg.label as u32) {
        Some(op) => op,
        None => {
            reply.label = REPLY_ERR;
            return reply;
        }
    };

    match op {
        SunlightdOp::Start => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                let already_running = matches!(
                    services.get(idx).map(|e| &e.state),
                    Some(ServiceState::Running { .. })
                );
                if already_running {
                    reply.label = REPLY_OK;
                } else {
                    // Clone path before mutable borrow
                    let path = services.get(idx).map(|e| {
                        let mut p = heapless::String::<256>::new();
                        let _ = p.push_str(&e.unit.exec_start);
                        p
                    });
                    if let Some(path) = path {
                        let bin = binary_name_of(&path);
                        let mut name_buf = heapless::String::<64>::new();
                        let _ = name_buf.push_str(bin);
                        match spawn_named(spawn_cap, &path, &name_buf) {
                            Ok(pid) => {
                                if let Some(entry) = services.get_mut(idx) {
                                    entry.mark_running(pid, 0);
                                }
                                reply.label = REPLY_OK;
                            }
                            Err(_) => {
                                reply.label = REPLY_ERR;
                            }
                        }
                    } else {
                        reply.label = REPLY_ERR;
                    }
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::Stop => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                if let Some(entry) = services.get_mut(idx) {
                    match entry.state {
                        ServiceState::Running { pid, .. } => {
                            sunlight_ipc::kill(pid as u64, 15); // SIGTERM
                            entry.mark_stopped();
                            reply.label = REPLY_OK;
                        }
                        _ => {
                            // Not running; still report success
                            reply.label = REPLY_OK;
                        }
                    }
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::Restart => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                // Stop if running
                if let Some(entry) = services.get_mut(idx) {
                    if let ServiceState::Running { pid, .. } = entry.state {
                        sunlight_ipc::kill(pid as u64, 15);
                        entry.mark_stopped();
                    }
                }
                // Clone path before mutable borrow
                let path = services.get(idx).map(|e| {
                    let mut p = heapless::String::<256>::new();
                    let _ = p.push_str(&e.unit.exec_start);
                    p
                });
                if let Some(path) = path {
                    let bin = binary_name_of(&path);
                    let mut name_buf = heapless::String::<64>::new();
                    let _ = name_buf.push_str(bin);
                    match spawn_named(spawn_cap, &path, &name_buf) {
                        Ok(pid) => {
                            if let Some(entry) = services.get_mut(idx) {
                                entry.mark_running(pid, 0);
                            }
                            reply.label = REPLY_OK;
                        }
                        Err(_) => {
                            reply.label = REPLY_ERR;
                        }
                    }
                } else {
                    reply.label = REPLY_ERR;
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::Reload => {
            // Unit files are compiled-in; reload is a no-op but not an error.
            reply.label = REPLY_OK;
        }

        SunlightdOp::Enable => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                if let Some(entry) = services.get_mut(idx) {
                    if entry.enabled {
                        reply.label = REPLY_NOP;
                    } else {
                        entry.enabled = true;
                        reply.label = REPLY_OK;
                    }
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::Disable => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                if let Some(entry) = services.get_mut(idx) {
                    if !entry.enabled {
                        reply.label = REPLY_NOP;
                    } else {
                        entry.enabled = false;
                        reply.label = REPLY_OK;
                    }
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::Status => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                if let Some(entry) = services.get(idx) {
                    let status = match entry.state {
                        ServiceState::Stopped => StatusReply {
                            state: 0,
                            pid: 0,
                            restarts: entry.restart_count,
                            started_at: 0,
                            enabled: entry.enabled,
                        },
                        ServiceState::Starting => StatusReply {
                            state: 1,
                            pid: 0,
                            restarts: entry.restart_count,
                            started_at: 0,
                            enabled: entry.enabled,
                        },
                        ServiceState::Running { pid, started_at } => StatusReply {
                            state: 2,
                            pid,
                            restarts: entry.restart_count,
                            started_at,
                            enabled: entry.enabled,
                        },
                        ServiceState::Failed {
                            exit_code,
                            crashed_at,
                            restarts,
                        } => StatusReply {
                            state: 3,
                            pid: exit_code as u32,
                            restarts,
                            started_at: crashed_at,
                            enabled: entry.enabled,
                        },
                        ServiceState::Restarting { at } => StatusReply {
                            state: 4,
                            pid: 0,
                            restarts: entry.restart_count,
                            started_at: at,
                            enabled: entry.enabled,
                        },
                    };
                    status.pack(&mut reply);
                    reply.label = REPLY_OK;
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::List => {
            // words[0] from client = requested index
            let idx = msg.words[0] as usize;
            if idx < services.count {
                if let Some(entry) = services.get(idx) {
                    let mut name = heapless::String::<64>::new();
                    let _ = name.push_str(binary_name_of(&entry.unit.exec_start));
                    let list_entry = ListEntry {
                        name,
                        state: match entry.state {
                            ServiceState::Running { .. } => 2,
                            ServiceState::Starting => 1,
                            ServiceState::Failed { .. } => 3,
                            ServiceState::Restarting { .. } => 4,
                            _ => 0,
                        },
                        pid: match entry.state {
                            ServiceState::Running { pid, .. } => pid,
                            _ => 0,
                        },
                        restarts: entry.restart_count,
                        enabled: entry.enabled,
                    };
                    list_entry.pack(&mut reply, services.count);
                    reply.label = REPLY_OK;
                }
            }
            // label stays 0 when idx >= count → client stops iterating
        }

        SunlightdOp::GetLog => {
            reply.label = REPLY_ERR; // not yet implemented
        }
    }

    reply
}

#[no_mangle]
fn _start() -> ! {
    sunlight_ipc::debug_log("[SUNLIGHTD] main() reached\n");
    serial_println!("[SUNLIGHTD] Starting sunlightd v0.2");

    // Register self FIRST (before any other IPC lookups, to avoid deadlock)
    let ep = endpoint_create();
    nameserver_register("sunlightd", ep);
    serial_println!("[SUNLIGHTD] Registered as 'sunlightd'");

    // Load unit files
    let (mut services, _sockets) = load_units();
    serial_println!("[SUNLIGHTD] Loaded {} service units", services.count);

    // Build dependency graph (validates graph; start order is fixed below)
    let _order = match build_dep_graph(&services) {
        Ok(o) => o,
        Err(e) => {
            serial_println!("[SUNLIGHTD] ERROR: {}", e);
            loop {}
        }
    };

    serial_println!("[SUNLIGHTD] Start order: timezone_service → niced → gcd → uac_service → sm → sunlight-kv → rand_service → sunlight-tls");
    serial_println!("[SunlightOS] sunlightd OK");

    // Spawn enabled services owned by sunlightd (kernel/init own vfs/net/tty — not our job).
    // Order matters: sm before kv, rand_service before sunlight-tls.
    // solar is disabled by default and intentionally omitted from auto-start.
    let spawn_cap = wait_for_spawn_cap();
    if spawn_cap != sunlight_ipc::CapabilityToken(0) {
        let managed: &[(&str, &str)] = &[
            ("/sbin/timezone_service", "timezone_service"),
            ("/sbin/niced", "niced"),
            ("/sbin/gcd", "gcd"),
            ("/sbin/uac_service", "uac_service"),
            ("/sbin/sunlight-sm", "sunlight-sm"),
            ("/sbin/sunlight-kv", "sunlight-kv"),
            ("/sbin/rand_service", "rand_service"),
            ("/sbin/sunlight-tls", "sunlight-tls"),
            ("/sbin/solar", "solar"),
            ("/sbin/sunlight-thumbd", "sunlight-thumbd"),
        ];
        for &(path, name) in managed {
            // Check enabled state before spawning
            let enabled = services
                .find_by_name(name)
                .and_then(|i| services.get(i))
                .map(|e| e.enabled)
                .unwrap_or(true);

            if !enabled {
                serial_println!("[SUNLIGHTD] {} is disabled, skipping auto-start", name);
                continue;
            }

            match spawn_named(spawn_cap, path, name) {
                Ok(pid) => {
                    serial_println!("[SUNLIGHTD] spawned {} pid={}", name, pid);
                    // Update service table state using name-based lookup (not index)
                    if let Some(idx) = services.find_by_name(name) {
                        if let Some(entry) = services.get_mut(idx) {
                            entry.mark_running(pid, 0);
                        }
                    }
                    if name == "timezone_service" {
                        serial_println!("[SUNLIGHTD] timezone.service: running (pid={})", pid);
                        serial_println!("[SunlightOS] timezone OK");
                    }
                }
                Err(e) => serial_println!("[SUNLIGHTD] failed to spawn {}: {}", name, e),
            }
        }
    }

    // Main control loop.
    let mut msg = ipc_recv(ep);
    loop {
        let reply = handle_control_message(&msg, &mut services, spawn_cap);
        msg = ipc_reply_and_wait(ep, reply);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[SUNLIGHTD] PANIC: {}", _info);
    loop {}
}
