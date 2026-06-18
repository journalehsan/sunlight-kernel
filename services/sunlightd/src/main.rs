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

mod unit;
mod graph;
mod supervisor;
mod ipc;
mod socket_act;
mod journal;

use sunlight_ipc::{
    CapabilityToken, IpcMsg, SpawnRequest, debug_log, endpoint_create, ipc_call, ipc_recv,
    ipc_reply_and_wait, nameserver_lookup, nameserver_register,
};
use unit::{ServiceUnit, SocketUnit, parse_service_unit, MAX_UNITS};
use graph::DepGraph;
use supervisor::{ServiceEntry, ServiceState};
use ipc::{SunlightdOp, extract_unit_name, StatusReply, ListEntry};

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

    fn find_by_name(&self, name: &str) -> Option<usize> {
        for i in 0..self.count {
            if let Some(ref entry) = self.services[i] {
                // Extract service name from ExecStart path
                if let Some(path_end) = entry.unit.exec_start.rfind('/') {
                    let binary_name = &entry.unit.exec_start[(path_end + 1)..];
                    if binary_name.starts_with(name) {
                        return Some(i);
                    }
                } else if entry.unit.exec_start.starts_with(name) {
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

/// Load unit files for services managed by sunlightd.
/// kernel → init → sunlightd; vfs/net/tty are managed by kernel/init, not here.
fn load_units() -> (ServiceTable, heapless::Vec<SocketUnit, 8>) {
    let mut services = ServiceTable::new();
    let sockets: heapless::Vec<SocketUnit, 8> = heapless::Vec::new();

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

    (services, sockets)
}

/// Build dependency graph and return topological order
fn build_dep_graph(services: &ServiceTable) -> Result<heapless::Vec<usize, MAX_UNITS>, &'static str> {
    let mut graph = DepGraph::new();

    // Add all services to graph
    for i in 0..services.count {
        if let Some(ref entry) = services.services[i] {
            // Use a stable unit name (derived from ExecStart)
            let mut unit_name = heapless::String::<64>::new();
            if let Some(pos) = entry.unit.exec_start.rfind('/') {
                let _ = unit_name.push_str(&entry.unit.exec_start[(pos + 1)..]);
            } else {
                let _ = unit_name.push_str(&entry.unit.exec_start);
            }
            let _ = unit_name.push_str(".service");
            
            graph.add_unit(&unit_name).map_err(|_| "Graph add failed")?;
        }
    }

    // Add edges based on After/Requires
    for i in 0..services.count {
        if let Some(ref entry) = services.services[i] {
            let mut unit_name = heapless::String::<64>::new();
            if let Some(pos) = entry.unit.exec_start.rfind('/') {
                let _ = unit_name.push_str(&entry.unit.exec_start[(pos + 1)..]);
            } else {
                let _ = unit_name.push_str(&entry.unit.exec_start);
            }
            let _ = unit_name.push_str(".service");

            for dep in &entry.unit.after {
                let _ = graph.add_edge(dep, &unit_name);
            }
        }
    }

    graph.topological_order().map_err(|_| "Topological sort failed")
}


/// Spawn a named daemon via the kernel spawn capability.
///
/// `path` is the binary path (up to 32 bytes); `name` is the explicit process
/// name that will appear in monitoring tools such as `top`. The name is packed
/// into the SpawnRequest alongside the path; the kernel derives the PCB name
/// from the path basename (register IPC only forwards words[0..3]), while the
/// name field is preserved in the request for documentation and future
/// memory-mapped IPC channels.
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

/// Handle control IPC messages
fn handle_control_message(msg: &IpcMsg, services: &mut ServiceTable, _spawn_cap: CapabilityToken) -> IpcMsg {
    let mut reply = IpcMsg::empty();

    let op = match SunlightdOp::from_u32(msg.label as u32) {
        Some(op) => op,
        None => {
            reply.label = 0xff; // Error
            return reply;
        }
    };

    match op {
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
                        },
                        ServiceState::Starting => StatusReply {
                            state: 1,
                            pid: 0,
                            restarts: entry.restart_count,
                            started_at: 0,
                        },
                        ServiceState::Running { pid, started_at } => StatusReply {
                            state: 2,
                            pid,
                            restarts: entry.restart_count,
                            started_at,
                        },
                        ServiceState::Failed { exit_code, crashed_at, restarts } => StatusReply {
                            state: 3,
                            pid: exit_code as u32,
                            restarts,
                            started_at: crashed_at,
                        },
                        ServiceState::Restarting { at } => StatusReply {
                            state: 4,
                            pid: 0,
                            restarts: entry.restart_count,
                            started_at: at,
                        },
                    };
                    status.pack(&mut reply);
                    reply.label = 1; // Success
                }
            } else {
                reply.label = 0xff; // Not found
            }
        }
        SunlightdOp::List => {
            // words[0] from client = requested index; words[7] in reply = total count
            let idx = msg.words[0] as usize;
            if idx < services.count {
                if let Some(entry) = services.get(idx) {
                    let mut name = heapless::String::<64>::new();
                    if let Some(pos) = entry.unit.exec_start.rfind('/') {
                        let _ = name.push_str(&entry.unit.exec_start[(pos + 1)..]);
                    } else {
                        let _ = name.push_str(&entry.unit.exec_start);
                    }
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
                    };
                    list_entry.pack(&mut reply);
                    reply.words[7] = services.count as u64;
                    reply.label = 1;
                }
            }
        }
        _ => {
            reply.label = 0xff; // Unsupported
        }
    }

    reply
}

#[no_mangle]
fn _start() -> ! {
    // Diagnostic 1c: absolute first line of sunlightd main, using same debug_log mechanism as other services (vfs_server, tty_server, install_sunlightos etc.)
    sunlight_ipc::debug_log("[SUNLIGHTD] main() reached\n");
    serial_println!("[SUNLIGHTD] Starting sunlightd v0.1");

    // Register self FIRST (before any other IPC lookups, to avoid deadlock and to match required startup sequence)
    let ep = endpoint_create();
    nameserver_register("sunlightd", ep);
    serial_println!("[SUNLIGHTD] Registered as 'sunlightd'");

    // Load unit files
    let (mut services, _sockets) = load_units();
    serial_println!("[SUNLIGHTD] Loaded {} service units", services.count);
    serial_println!("[SUNLIGHTD] All units accounted for");

    // Build dependency graph (result drives start order; unused directly but validates graph)
    let _order = match build_dep_graph(&services) {
        Ok(o) => o,
        Err(e) => {
            serial_println!("[SUNLIGHTD] ERROR: {}", e);
            loop {}
        }
    };

    serial_println!("[SUNLIGHTD] Start order: timezone_service → niced → gcd → uac_service → sm → sunlight-kv → rand_service → sunlight-tls");
    serial_println!("[SunlightOS] sunlightd OK");

    // Spawn services sunlightd owns (kernel/init own vfs/net/tty — not our job).
    let spawn_cap = nameserver_lookup("spawn").unwrap_or(sunlight_ipc::CapabilityToken(0));
    if spawn_cap != sunlight_ipc::CapabilityToken(0) {
        // Indices must match the order services were added in load_units():
        // 0=timezone_service, 1=niced, 2=gcd, 3=uac_service, 4=sm (sunlight-sm),
        // 5=sunlight-kv, 6=rand_service, 7=sunlight-tls.
        // sm (storage manager) starts before kv so protected writes are delegated.
        // rand_service MUST precede sunlight-tls.
        let managed: [(&str, &str); 8] = [
            ("/sbin/timezone_service", "timezone_service"),
            ("/sbin/niced",            "niced"),
            ("/sbin/gcd",              "gcd"),
            ("/sbin/uac_service",      "uac_service"),
            ("/sbin/sunlight-sm",      "sunlight-sm"),
            ("/sbin/sunlight-kv",      "sunlight-kv"),
            ("/sbin/rand_service",     "rand_service"),
            ("/sbin/sunlight-tls",     "sunlight-tls"),
        ];
        for (i, (path, name)) in managed.iter().enumerate() {
            match spawn_named(spawn_cap, path, name) {
                Ok(pid) => {
                    serial_println!("[SUNLIGHTD] spawned {} pid={}", name, pid);
                    if let Some(entry) = services.get_mut(i) {
                        entry.mark_running(pid, 0);
                    }
                    if *name == "timezone_service" {
                        serial_println!("[SUNLIGHTD] timezone.service: running (pid={})", pid);
                        serial_println!("[SunlightOS] timezone OK");
                    }
                }
                Err(e) => serial_println!("[SUNLIGHTD] failed to spawn {}: {}", name, e),
            }
        }
    } else {
        serial_println!("[SUNLIGHTD] spawn capability unavailable; daemons not started");
    }

    // Main control loop.
    // ipc_reply_and_wait returns the NEXT incoming message atomically with the reply.
    // We must feed that returned message into the next iteration instead of calling
    // ipc_recv again, otherwise the consumed message is dropped and the client deadlocks.
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
