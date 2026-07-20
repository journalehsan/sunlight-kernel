//! sunlightd - SunlightOS service supervisor daemon
//! Reads .service and .socket unit files and manages process lifecycle

#![no_std]
#![cfg_attr(not(test), no_main)]

extern crate alloc;
#[cfg(test)]
extern crate std;

#[cfg(not(test))]
struct BumpAllocator;

#[cfg(not(test))]
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

#[cfg(not(test))]
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
    debug_log, endpoint_create, ipc_call, ipc_reply_and_try_recv, monotonic_millis,
    nameserver_lookup, nameserver_register, CapabilityToken, IpcMsg, SpawnRequest,
};
use sunlight_libc::{self as libc, Errno, O_CREAT, O_RDONLY, O_TRUNC, O_WRONLY};
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

const ENABLED_STATE_PATH: &[u8] = b"/state/sunlightd/enabled-services";
const ENABLED_STATE_TMP_PATH: &[u8] = b"/state/sunlightd/enabled-services.tmp";
const ENABLED_STATE_VERSION: &str = "v1";
const ENABLED_STATE_MAX_BYTES: usize = 2048;

struct ServiceTable {
    services: [Option<ServiceEntry>; MAX_UNITS],
    count: usize,
}

struct BootStartup {
    pending: [bool; MAX_UNITS],
    remaining: usize,
    completion_logged: bool,
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

    fn find_by_unit_id(&self, unit_id: &str) -> Option<usize> {
        for i in 0..self.count {
            if let Some(ref entry) = self.services[i] {
                if service_unit_id(entry) == unit_id {
                    return Some(i);
                }
            }
        }
        None
    }
}

impl BootStartup {
    fn new(services: &ServiceTable) -> Self {
        let mut pending = [false; MAX_UNITS];
        let mut remaining = 0usize;
        for idx in 0..services.count {
            if let Some(entry) = services.get(idx) {
                if entry.enabled {
                    pending[idx] = true;
                    remaining += 1;
                }
            }
        }
        Self {
            pending,
            remaining,
            completion_logged: false,
        }
    }

    fn is_pending(&self, idx: usize) -> bool {
        idx < self.pending.len() && self.pending[idx]
    }

    fn mark_done(&mut self, idx: usize) {
        if self.is_pending(idx) {
            self.pending[idx] = false;
            self.remaining = self.remaining.saturating_sub(1);
        }
    }

    fn is_complete(&self) -> bool {
        self.remaining == 0
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

fn service_unit_id(entry: &ServiceEntry) -> heapless::String<64> {
    let mut unit_name = heapless::String::<64>::new();
    let _ = unit_name.push_str(binary_name_of(&entry.unit.exec_start));
    let _ = unit_name.push_str(".service");
    unit_name
}

fn collect_enabled_state(services: &ServiceTable) -> heapless::String<ENABLED_STATE_MAX_BYTES> {
    let mut content = heapless::String::<ENABLED_STATE_MAX_BYTES>::new();
    let _ = content.push_str(ENABLED_STATE_VERSION);
    let _ = content.push('\n');
    for idx in 0..services.count {
        let Some(entry) = services.get(idx) else {
            continue;
        };
        let unit_id = service_unit_id(entry);
        let _ = content.push_str(unit_id.as_str());
        let _ = content.push('=');
        let _ = content.push_str(if entry.enabled { "1" } else { "0" });
        let _ = content.push('\n');
    }
    content
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PersistStateError {
    InvalidUtf8,
    MissingVersion,
    UnsupportedVersion,
    MalformedRecord,
    InvalidStateValue,
}

fn apply_enabled_state_from_str(
    services: &mut ServiceTable,
    content: &str,
) -> Result<(), PersistStateError> {
    let mut lines = content.lines();
    let Some(version) = lines.next() else {
        return Err(PersistStateError::MissingVersion);
    };
    if version != ENABLED_STATE_VERSION {
        return Err(PersistStateError::UnsupportedVersion);
    }

    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((unit_id, state)) = line.split_once('=') else {
            return Err(PersistStateError::MalformedRecord);
        };
        let enabled = match state {
            "0" => false,
            "1" => true,
            _ => return Err(PersistStateError::InvalidStateValue),
        };
        if let Some(idx) = services.find_by_unit_id(unit_id) {
            if let Some(entry) = services.get_mut(idx) {
                entry.enabled = enabled;
            }
        } else {
            serial_println!("[SUNLIGHTD] Ignoring unknown service state '{}'", unit_id);
        }
    }

    Ok(())
}

fn load_persisted_enabled_state(services: &mut ServiceTable) {
    let fd = match libc::open_with_flags(ENABLED_STATE_PATH, O_RDONLY) {
        Ok(fd) => fd,
        Err(Errno::Failed) => return,
        Err(err) => {
            serial_println!("[SUNLIGHTD] enabled-state open failed: {:?}", err);
            return;
        }
    };

    let mut buf = [0u8; ENABLED_STATE_MAX_BYTES];
    let mut total = 0usize;
    let result = loop {
        if total == buf.len() {
            break Err(PersistStateError::MalformedRecord);
        }
        match libc::read(fd, &mut buf[total..]) {
            Ok(0) => break Ok(total),
            Ok(n) => total += n,
            Err(_) => break Err(PersistStateError::MalformedRecord),
        }
    };
    let _ = libc::close(fd);

    let Ok(total) = result else {
        serial_println!("[SUNLIGHTD] enabled-state rejected: unreadable");
        return;
    };
    let text = match core::str::from_utf8(&buf[..total]) {
        Ok(text) => text,
        Err(_) => {
            serial_println!("[SUNLIGHTD] enabled-state rejected: invalid utf8");
            return;
        }
    };

    if let Err(err) = apply_enabled_state_from_str(services, text) {
        serial_println!("[SUNLIGHTD] enabled-state rejected: {:?}", err);
    } else {
        serial_println!("[SUNLIGHTD] loaded persisted enabled state");
    }
}

fn persist_enabled_state(services: &ServiceTable) -> Result<(), &'static str> {
    let content = collect_enabled_state(services);
    let fd =
        libc::open_with_flags(ENABLED_STATE_TMP_PATH, O_WRONLY | O_CREAT | O_TRUNC).map_err(
            |_| "open-temp",
        )?;
    libc::chmod(ENABLED_STATE_TMP_PATH, 0o600).map_err(|_| "chmod-temp")?;
    libc::write_all(fd, content.as_bytes()).map_err(|_| "write-temp")?;
    libc::close(fd).map_err(|_| "close-temp")?;
    libc::rename(ENABLED_STATE_TMP_PATH, ENABLED_STATE_PATH).map_err(|_| "rename-temp")?;
    Ok(())
}

fn normalize_dep_unit_name(dep: &str) -> heapless::String<64> {
    let mut unit_name = heapless::String::<64>::new();
    if dep.contains('.') {
        let _ = unit_name.push_str(dep);
    } else {
        let _ = unit_name.push_str(dep);
        let _ = unit_name.push_str(".service");
    }
    unit_name
}

/// Load unit files for services managed by sunlightd.
/// kernel → init → sunlightd; vfs/net/tty are managed by kernel/init, not here.
fn load_units() -> (ServiceTable, heapless::Vec<SocketUnit, 8>) {
    let mut services = ServiceTable::new();
    let sockets: heapless::Vec<SocketUnit, 8> = heapless::Vec::new();

    // solar.service — disabled by default; start with: sunlightctl start solar
    let solar_service = r#"[Unit]
Description=Solar HTTP Server
After=net_server.service

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
Requires=uac_service.service

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
After=sunlight-sm.service
Requires=sunlight-sm.service

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

    let clipd_service = r#"[Unit]
Description=SunlightOS Clipboard Service
After=sunlight-kv.service
Requires=sunlight-kv.service

[Service]
Type=simple
ExecStart=/sbin/sunlight-clipd
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(clipd_service.as_bytes()) {
        let _ = services.add(unit);
    }

    let dialogd_service = r#"[Unit]
Description=SunlightOS Dialog Host
After=sunlight-display.service

[Service]
Type=simple
ExecStart=/sbin/sunlight-dialogd
Restart=on-failure
RestartSec=3
User=root
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(dialogd_service.as_bytes()) {
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
After=sunlight-kv.service rand_service.service net_server.service networkd.service resolved.service
Requires=sunlight-kv.service rand_service.service net_server.service

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
            let unit_name = service_unit_id(entry);
            graph.add_unit(&unit_name).map_err(|_| "Graph add failed")?;
        }
    }

    for i in 0..services.count {
        if let Some(ref entry) = services.services[i] {
            let unit_name = service_unit_id(entry);

            for dep in &entry.unit.after {
                let dep_name = normalize_dep_unit_name(dep);
                let _ = graph.add_edge(&dep_name, &unit_name);
            }
            for dep in &entry.unit.requires {
                let dep_name = normalize_dep_unit_name(dep);
                let _ = graph.add_edge(&dep_name, &unit_name);
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

fn dep_unit_to_ready_name(dep: &str) -> &str {
    let dep = dep.strip_suffix(".service").unwrap_or(dep);
    match dep {
        "timezone_service" => "tz",
        "uac_service" => "uac",
        "sunlight-sm" => "sm",
        "rand_service" => "rand",
        "net_server" => "net",
        "sunlight-thumbd" => "thumbd",
        other => other,
    }
}

fn unit_is_enabled(services: &ServiceTable, dep: &str) -> bool {
    let dep_service = dep.strip_suffix(".service").unwrap_or(dep);
    services
        .find_by_name(dep_service)
        .and_then(|idx| services.get(idx))
        .map(|entry| entry.enabled)
        .unwrap_or(true)
}

fn deps_ready(services: &ServiceTable, unit: &ServiceUnit) -> bool {
    for dep in &unit.requires {
        if nameserver_lookup(dep_unit_to_ready_name(dep)).is_none() {
            return false;
        }
    }
    for dep in &unit.after {
        if !unit_is_enabled(services, dep) {
            continue;
        }
        if nameserver_lookup(dep_unit_to_ready_name(dep)).is_none() {
            return false;
        }
    }
    true
}

fn autostart_services(
    services: &mut ServiceTable,
    startup: &mut BootStartup,
    spawn_cap: CapabilityToken,
) {
    if startup.is_complete() {
        if !startup.completion_logged {
            serial_println!("[SUNLIGHTD] Autostart queue drained");
            startup.completion_logged = true;
        }
        return;
    }

    for idx in 0..services.count {
        if !startup.is_pending(idx) {
            continue;
        }

        let Some(entry) = services.get(idx) else {
            startup.mark_done(idx);
            continue;
        };

        if !entry.enabled {
            startup.mark_done(idx);
            continue;
        }

        if !deps_ready(services, &entry.unit) {
            continue;
        }

        let mut path = heapless::String::<256>::new();
        let _ = path.push_str(&entry.unit.exec_start);
        let bin = binary_name_of(&path);
        let mut name_buf = heapless::String::<64>::new();
        let _ = name_buf.push_str(bin);

        if let Some(entry) = services.get_mut(idx) {
            entry.mark_starting();
        }

        match spawn_named(spawn_cap, &path, &name_buf) {
            Ok(pid) => {
                let started_at = monotonic_millis();
                serial_println!("[SUNLIGHTD] spawned {} pid={}", name_buf, pid);
                if let Some(entry) = services.get_mut(idx) {
                    entry.mark_running(pid, started_at);
                }
                if name_buf.as_str() == "timezone_service" {
                    serial_println!("[SUNLIGHTD] timezone.service: running (pid={})", pid);
                    serial_println!("[SunlightOS] timezone OK");
                }
            }
            Err(e) => {
                serial_println!("[SUNLIGHTD] failed to spawn {}: {}", name_buf, e);
                if let Some(entry) = services.get_mut(idx) {
                    entry.mark_failed(-1, monotonic_millis());
                }
            }
        }

        startup.mark_done(idx);
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
                                    entry.mark_running(pid, monotonic_millis());
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
                                entry.mark_running(pid, monotonic_millis());
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
                if services.get(idx).map(|entry| entry.enabled).unwrap_or(false) {
                    reply.label = REPLY_NOP;
                } else {
                    if let Some(entry) = services.get_mut(idx) {
                        entry.enabled = true;
                    }
                    match persist_enabled_state(services) {
                        Ok(()) => reply.label = REPLY_OK,
                        Err(err) => {
                            if let Some(entry) = services.get_mut(idx) {
                                entry.enabled = false;
                            }
                            serial_println!(
                                "[SUNLIGHTD] enable persist failed for {}: {}",
                                unit_name,
                                err
                            );
                            reply.label = REPLY_ERR;
                        }
                    }
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::Disable => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                if !services.get(idx).map(|entry| entry.enabled).unwrap_or(true) {
                    reply.label = REPLY_NOP;
                } else {
                    if let Some(entry) = services.get_mut(idx) {
                        entry.enabled = false;
                    }
                    match persist_enabled_state(services) {
                        Ok(()) => reply.label = REPLY_OK,
                        Err(err) => {
                            if let Some(entry) = services.get_mut(idx) {
                                entry.enabled = true;
                            }
                            serial_println!(
                                "[SUNLIGHTD] disable persist failed for {}: {}",
                                unit_name,
                                err
                            );
                            reply.label = REPLY_ERR;
                        }
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

#[cfg(not(test))]
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
    load_persisted_enabled_state(&mut services);
    serial_println!("[SUNLIGHTD] Loaded {} service units", services.count);

    // Build dependency graph (validates the declarative dependency metadata)
    let _order = match build_dep_graph(&services) {
        Ok(o) => o,
        Err(e) => {
            serial_println!("[SUNLIGHTD] ERROR: {}", e);
            loop {}
        }
    };

    serial_println!("[SUNLIGHTD] Autostart: parallel where possible, gated by service readiness");
    serial_println!("[SUNLIGHTD] TLS waits for rand_service, sunlight-kv, and the network stack");
    serial_println!("[SunlightOS] sunlightd OK");

    // Spawn enabled services owned by sunlightd (kernel/init own vfs/net/tty — not our job).
    // Independent services can launch together; dependent services wait until
    // their providers have registered with the nameserver.
    let spawn_cap = wait_for_spawn_cap();
    let mut startup = BootStartup::new(&services);
    autostart_services(&mut services, &mut startup, spawn_cap);

    // Main control loop. Non-blocking receive lets boot autostart keep
    // progressing while dependencies register.
    let mut reply = IpcMsg::empty();
    loop {
        autostart_services(&mut services, &mut startup, spawn_cap);
        match ipc_reply_and_try_recv(ep, reply) {
            Some(msg) => {
                reply = handle_control_message(&msg, &mut services, spawn_cap);
            }
            None => {
                reply = IpcMsg::empty();
                sunlight_ipc::process_yield();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_for_test() -> ServiceTable {
        let (services, _) = load_units();
        services
    }

    #[test]
    fn compiled_default_for_solar_is_disabled() {
        let services = load_for_test();
        let idx = services.find_by_name("solar").expect("solar service");
        assert!(!services.get(idx).unwrap().enabled);
    }

    #[test]
    fn persisted_state_overrides_defaults() {
        let mut services = load_for_test();
        apply_enabled_state_from_str(&mut services, "v1\nsolar.service=1\nsunlight-tls.service=0\n")
            .expect("valid state");

        let solar = services.find_by_name("solar").unwrap();
        let tls = services.find_by_name("sunlight-tls").unwrap();
        assert!(services.get(solar).unwrap().enabled);
        assert!(!services.get(tls).unwrap().enabled);
    }

    #[test]
    fn malformed_state_fails_closed() {
        let mut services = load_for_test();
        let solar = services.find_by_name("solar").unwrap();
        let original = services.get(solar).unwrap().enabled;

        let err = apply_enabled_state_from_str(&mut services, "v1\nsolar.service=enabled\n");
        assert_eq!(err, Err(PersistStateError::InvalidStateValue));
        assert_eq!(services.get(solar).unwrap().enabled, original);
    }

    #[test]
    fn unknown_services_are_ignored() {
        let mut services = load_for_test();
        apply_enabled_state_from_str(&mut services, "v1\nfuture-ssh.service=1\n")
            .expect("unknown entries should be ignored");
        let solar = services.find_by_name("solar").unwrap();
        assert!(!services.get(solar).unwrap().enabled);
    }

    #[test]
    fn serialized_state_is_versioned_and_unit_scoped() {
        let services = load_for_test();
        let content = collect_enabled_state(&services);
        assert!(content.starts_with("v1\n"));
        assert!(content.contains("solar.service=0\n"));
        assert!(content.contains("sunlight-tls.service=1\n"));
    }
}

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    serial_println!("[SUNLIGHTD] PANIC: {}", _info);
    loop {}
}
