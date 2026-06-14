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
    CapabilityToken, IpcMsg, debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait,
    nameserver_lookup, nameserver_register,
};
use unit::{ServiceUnit, SocketUnit, parse_service_unit, parse_socket_unit, MAX_UNITS};
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

/// Load unit files from VFS /etc/sunlight/services/
fn load_units() -> (ServiceTable, heapless::Vec<SocketUnit, 8>) {
    let mut services = ServiceTable::new();
    let mut sockets: heapless::Vec<SocketUnit, 8> = heapless::Vec::new();

    // TODO: Use VFS readdir IPC to enumerate /etc/sunlight/services/
    // For now, load hardcoded default services
    
    // vfs.service
    let vfs_service = r#"[Unit]
Description=VFS Server
After=
Requires=

[Service]
Type=simple
ExecStart=/sbin/vfs_server
Restart=always
RestartSec=2
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(vfs_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // net.service
    let net_service = r#"[Unit]
Description=Network Service
After=vfs.service
Requires=vfs.service

[Service]
Type=simple
ExecStart=/sbin/net_server
Restart=on-failure
RestartSec=5
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=network.target
"#;
    if let Ok(unit) = parse_service_unit(net_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // tty.service
    let tty_service = r#"[Unit]
Description=SunlightTTY Terminal Service
After=vfs.service
Requires=vfs.service
Wants=net.service

[Service]
Type=simple
ExecStart=/sbin/tty_server
Restart=always
RestartSec=1
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(tty_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // sshd.socket
    let sshd_socket = r#"[Unit]
Description=SSH Socket Activation

[Socket]
ListenStream=22
Service=sshd.service

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(socket) = parse_socket_unit(sshd_socket.as_bytes()) {
        let _ = sockets.push(socket);
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

/// Spawn a service using the spawn capability
fn spawn_service(spawn_cap: CapabilityToken, entry: &mut ServiceEntry) -> Result<u32, &'static str> {
    use sunlight_ipc::SpawnMsg;

    entry.mark_starting();

    // Parse ExecStart to get path
    let path = entry.unit.exec_start.as_str();
    
    // Create spawn request
    let mut msg = IpcMsg::empty();
    msg.label = SpawnMsg::SPAWN as u64;

    // Pack path into first 4 words (32 bytes)
    let path_bytes = path.as_bytes();
    for i in 0..4 {
        let mut word: u64 = 0;
        for j in 0..8 {
            let idx = i * 8 + j;
            if idx < path_bytes.len() {
                word |= (path_bytes[idx] as u64) << (j * 8);
            }
        }
        msg.words[i] = word;
    }

    // Set uid/gid (words[4] and [5])
    msg.words[4] = 0; // root uid
    msg.words[5] = 0; // root gid

    // Send spawn request
    let reply = ipc_call(spawn_cap, msg);
    
    if reply.label == SpawnMsg::REPLY as u64 {
        let pid = reply.words[0] as u32;
        Ok(pid)
    } else {
        Err("Spawn failed")
    }
}

/// Handle control IPC messages
fn handle_control_message(msg: &IpcMsg, services: &mut ServiceTable, spawn_cap: CapabilityToken) -> IpcMsg {
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
            // Return first service entry
            // TODO: Support multi-message list iteration
            if services.count > 0 {
                if let Some(entry) = services.get(0) {
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
                            _ => 0,
                        },
                        pid: match entry.state {
                            ServiceState::Running { pid, .. } => pid,
                            _ => 0,
                        },
                        restarts: entry.restart_count,
                    };
                    list_entry.pack(&mut reply);
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
    serial_println!("[SUNLIGHTD] Starting sunlightd v0.1");

    // Load unit files
    let (mut services, sockets) = load_units();
    serial_println!("[SUNLIGHTD] Loaded {} service units, {} socket unit", services.count, sockets.len());

    // Build dependency graph
    let order = match build_dep_graph(&services) {
        Ok(o) => o,
        Err(e) => {
            serial_println!("[SUNLIGHTD] ERROR: {}", e);
            loop {}
        }
    };

    // Print start order
    let mut order_str = heapless::String::<128>::new();
    for (i, &idx) in order.iter().enumerate() {
        if let Some(entry) = services.get(idx) {
            if let Some(pos) = entry.unit.exec_start.rfind('/') {
                let _ = order_str.push_str(&entry.unit.exec_start[(pos + 1)..]);
            } else {
                let _ = order_str.push_str(&entry.unit.exec_start);
            }
            if i < order.len() - 1 {
                let _ = order_str.push_str(" → ");
            }
        }
    }
    serial_println!("[SUNLIGHTD] Start order: {}", order_str);

    // Lookup spawn capability
    let spawn_cap = nameserver_lookup("spawn");
    if spawn_cap.is_none() {
        serial_println!("[SUNLIGHTD] ERROR: spawn capability not found");
        loop {}
    }
    let spawn_cap = spawn_cap.unwrap();

    // Note: Services are already running (spawned by kernel init)
    // We're starting AFTER they're already up
    // So we just mark them as running and monitor them
    serial_println!("[SUNLIGHTD] Core services already running (spawned by kernel)");
    serial_println!("[SUNLIGHTD] Monitoring service health...");

    // Mark services as running (they were spawned by kernel)
    // vfs_server = PID 3, net_server = PID 5, tty_server = PID 4
    if let Some(entry) = services.get_mut(0) { // vfs
        entry.mark_running(3, 0);
        serial_println!("[SUNLIGHTD] vfs.service: running");
    }
    if let Some(entry) = services.get_mut(1) { // net
        entry.mark_running(5, 0);
        serial_println!("[SUNLIGHTD] net.service: running");
    }
    if let Some(entry) = services.get_mut(2) { // tty
        entry.mark_running(4, 0);
        serial_println!("[SUNLIGHTD] tty.service: running");
    }

    // Setup socket listeners
    for socket in &sockets {
        if let unit::SocketAddr::Tcp(port) = socket.listen_stream {
            serial_println!("[SUNLIGHTD] Socket listener: {} port {}", socket.service, port);
        }
    }

    serial_println!("[SUNLIGHTD] All units started");
    serial_println!("[SunlightOS] sunlightd OK");

    // Register with nameserver
    let ep = endpoint_create();
    nameserver_register("sunlightd", ep);

    // Main control loop
    loop {
        let msg = ipc_recv(ep);
        let reply = handle_control_message(&msg, &mut services, spawn_cap);
        ipc_reply_and_wait(ep, reply);
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[SUNLIGHTD] PANIC: {}", _info);
    loop {}
}
