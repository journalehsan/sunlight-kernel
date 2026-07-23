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
const SIGKILL: u32 = 9;
const SIGTERM: u32 = 15;
const STOP_GRACE_MS: u64 = 3_000;
const FAILURE_UNKNOWN: u32 = 0;
const FAILURE_SPAWN: u32 = 1;
const FAILURE_IDENTITY: u32 = 2;
const FAILURE_STARTUP: u32 = 3;
const FAILURE_RESTART_LIMIT: u32 = 4;

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
    let fd = libc::open_with_flags(ENABLED_STATE_TMP_PATH, O_WRONLY | O_CREAT | O_TRUNC)
        .map_err(|_| "open-temp")?;
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

fn pack_path_words(msg: &mut IpcMsg, path: &str) {
    let bytes = path.as_bytes();
    for i in 0..4 {
        let mut word = 0u64;
        for j in 0..8 {
            let idx = i * 8 + j;
            if idx < bytes.len() {
                word |= (bytes[idx] as u64) << (j * 8);
            }
        }
        msg.words[i] = word;
    }
    msg.word_count = 4;
}

fn lookup_user_credentials(username: &str) -> Option<(u32, u32)> {
    let vfs = nameserver_lookup("vfs")?;
    let mut msg = IpcMsg::with_label(sunlight_ipc::VfsMsg::GETPWNAM);
    pack_path_words(&mut msg, username);
    let reply = ipc_call(vfs, msg);
    if reply.label != sunlight_ipc::VfsMsg::REPLY || reply.words[0] != 0 {
        return None;
    }
    Some((reply.words[1] as u32, reply.words[2] as u32))
}

fn capability_summary(mask: u64) -> heapless::String<192> {
    let mut out = heapless::String::<192>::new();
    let mut wrote_any = false;
    for name in sunlight_ipc::service_capability_mask_to_names(mask) {
        if wrote_any {
            let _ = out.push(',');
        }
        let _ = out.push_str(name);
        wrote_any = true;
    }
    if !wrote_any {
        let _ = out.push_str("none");
    }
    out
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
Capability=network
Capability=vfs
Capability=kv-store
Capability=logging
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
Capability=time-sync
Capability=vfs
Capability=service-lifecycle
Capability=logging
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
Capability=service-lifecycle
Capability=scheduler-control
Capability=logging
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
Capability=scheduler-control
Capability=logging
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
Capability=authentication
Capability=vfs
Capability=logging
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
Capability=logging
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
Capability=storage-admin
Capability=logging
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
Capability=kv-store
Capability=logging
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
Capability=logging
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
Capability=logging
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
Capability=network
Capability=kv-store
Capability=secure-random
Capability=time-sync
Capability=logging
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(tls_service.as_bytes()) {
        let _ = services.add(unit);
    }

    // secret_store_test.service exercises the generic private-secret storage
    // contract. It is intentionally disabled and has the narrow host-key
    // administration capability rather than broad filesystem write access.
    let secret_store_test_service = r#"[Unit]
Description=SunlightOS Secret Storage Regression Service
After=rand_service.service
Requires=rand_service.service

[Service]
Type=oneshot
ExecStart=/sbin/secret_store_test
Restart=no
User=root
Capability=host-key-admin
Capability=secure-random
Capability=logging
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=sunlight.target
"#;
    if let Ok(unit) = parse_service_unit(secret_store_test_service.as_bytes()) {
        if let Ok(idx) = services.add(unit) {
            if let Some(entry) = services.get_mut(idx) {
                entry.enabled = false;
            }
        }
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
Capability=logging
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

fn spawn_named_with_identity(
    spawn_cap: CapabilityToken,
    path: &str,
    name: &str,
    uid: u32,
    gid: u32,
    capability_mask: u64,
) -> Result<u32, &'static str> {
    use sunlight_ipc::SpawnMsg;

    let req = SpawnRequest::new(path, name)
        .with_identity(uid, gid)
        .with_service_caps(capability_mask);
    let mut msg = IpcMsg::empty();
    req.pack_into(&mut msg);

    let reply = ipc_call(spawn_cap, msg);
    if reply.label == SpawnMsg::REPLY {
        Ok(reply.words[0] as u32)
    } else {
        Err("Spawn failed")
    }
}

fn entry_pid(entry: &ServiceEntry) -> Option<u32> {
    match entry.state {
        ServiceState::Starting { pid, .. } | ServiceState::Running { pid, .. } => Some(pid),
        _ => None,
    }
}

fn spawn_service_at(
    services: &mut ServiceTable,
    idx: usize,
    spawn_cap: CapabilityToken,
) -> Result<(), u32> {
    let Some(unit) = services.get(idx).map(|entry| entry.unit.clone()) else {
        return Err(FAILURE_SPAWN);
    };
    let mut path = heapless::String::<256>::new();
    let _ = path.push_str(&unit.exec_start);
    let bin = binary_name_of(&path);
    let mut name_buf = heapless::String::<64>::new();
    let _ = name_buf.push_str(bin);
    let Some((uid, gid)) = lookup_user_credentials(unit.user.as_str()) else {
        serial_println!(
            "[SUNLIGHTD] failed to resolve User={} for {}",
            unit.user,
            name_buf
        );
        if let Some(entry) = services.get_mut(idx) {
            entry.last_status_detail = FAILURE_IDENTITY;
            entry.mark_failed(-1, monotonic_millis());
        }
        return Err(FAILURE_IDENTITY);
    };
    serial_println!(
        "[SUNLIGHTD] capability profile {} uid={} gid={} caps={}",
        name_buf,
        uid,
        gid,
        capability_summary(unit.capability_mask)
    );
    match spawn_named_with_identity(spawn_cap, &path, &name_buf, uid, gid, unit.capability_mask) {
        Ok(pid) => {
            let now = monotonic_millis();
            if let Some(entry) = services.get_mut(idx) {
                entry.mark_starting(pid, now);
            }
            serial_println!("[SUNLIGHTD] spawned {} pid={}", name_buf, pid);
            if let Some(entry) = services.get(idx) {
                if !matches!(
                    entry.state,
                    ServiceState::Starting {
                        needs_ready: true,
                        ..
                    }
                ) {
                    if let Some(entry) = services.get_mut(idx) {
                        entry.mark_running(pid, now);
                    }
                }
            }
            if name_buf.as_str() == "timezone_service" {
                serial_println!("[SUNLIGHTD] timezone.service: running (pid={})", pid);
                serial_println!("[SunlightOS] timezone OK");
            }
            Ok(())
        }
        Err(e) => {
            serial_println!("[SUNLIGHTD] failed to spawn {}: {}", name_buf, e);
            if let Some(entry) = services.get_mut(idx) {
                entry.last_status_detail = FAILURE_SPAWN;
                entry.mark_failed(-1, monotonic_millis());
            }
            Err(FAILURE_SPAWN)
        }
    }
}

fn begin_stop(entry: &mut ServiceEntry, restart_after_stop: bool) {
    entry.stop_requested = true;
    entry.restart_after_stop = restart_after_stop;
}

fn finish_stop_or_restart(
    services: &mut ServiceTable,
    idx: usize,
    spawn_cap: CapabilityToken,
    exit_code: u64,
) {
    let now = monotonic_millis();
    let mut should_restart = false;
    if let Some(entry) = services.get_mut(idx) {
        if entry.stop_requested {
            if entry.restart_after_stop {
                entry.mark_restarting(now, now);
                should_restart = true;
            } else {
                entry.mark_stopped();
            }
        } else if entry.should_restart(exit_code as i32) {
            if entry.check_restart_limit(now) {
                entry.last_status_detail = FAILURE_RESTART_LIMIT;
                entry.mark_failed(exit_code as i32, now);
            } else {
                entry.mark_restarting(now, now);
                should_restart = true;
            }
        } else {
            entry.last_status_detail = FAILURE_UNKNOWN;
            entry.mark_failed(exit_code as i32, now);
        }
    }
    if should_restart {
        let _ = spawn_service_at(services, idx, spawn_cap);
    }
}

fn stop_service(
    services: &mut ServiceTable,
    idx: usize,
    spawn_cap: CapabilityToken,
    restart_after_stop: bool,
) -> Result<(), &'static str> {
    let pid = services.get(idx).and_then(entry_pid);
    let Some(pid) = pid else {
        if let Some(entry) = services.get_mut(idx) {
            if restart_after_stop {
                entry.mark_restarting(monotonic_millis(), monotonic_millis());
            } else {
                entry.mark_stopped();
            }
        }
        if restart_after_stop {
            return spawn_service_at(services, idx, spawn_cap).map_err(|_| "spawn");
        }
        return Ok(());
    };

    let Some(entry) = services.get_mut(idx) else {
        return Err("not-found");
    };
    begin_stop(entry, restart_after_stop);
    let _ = libc::kill(pid as u64, SIGTERM);
    let start = monotonic_millis();
    loop {
        match libc::try_waitpid(pid as u64) {
            Ok(Some(code)) => {
                finish_stop_or_restart(services, idx, spawn_cap, code);
                return Ok(());
            }
            Ok(None) => {
                if monotonic_millis().saturating_sub(start) >= STOP_GRACE_MS {
                    let _ = libc::kill(pid as u64, SIGKILL);
                    match libc::waitpid(pid as u64) {
                        Ok(code) => {
                            finish_stop_or_restart(services, idx, spawn_cap, code);
                            return Ok(());
                        }
                        Err(_) => {
                            if let Some(entry) = services.get_mut(idx) {
                                entry.mark_stopped();
                            }
                            return Err("waitpid");
                        }
                    }
                }
                sunlight_ipc::process_yield();
            }
            Err(_) => {
                finish_stop_or_restart(services, idx, spawn_cap, 0);
                return Ok(());
            }
        }
    }
}

fn poll_service_exits(services: &mut ServiceTable, spawn_cap: CapabilityToken) {
    for idx in 0..services.count {
        let pid = services.get(idx).and_then(entry_pid);
        let Some(pid) = pid else {
            continue;
        };
        match libc::try_waitpid(pid as u64) {
            Ok(Some(code)) => finish_stop_or_restart(services, idx, spawn_cap, code),
            Ok(None) => {}
            Err(_) => finish_stop_or_restart(services, idx, spawn_cap, 0),
        }
    }
}

fn find_service_idx_by_pid(services: &ServiceTable, pid: u32) -> Option<usize> {
    for idx in 0..services.count {
        let Some(entry) = services.get(idx) else {
            continue;
        };
        if entry_pid(entry) == Some(pid) {
            return Some(idx);
        }
    }
    None
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
        let _ = spawn_service_at(services, idx, spawn_cap);

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
                    Some(ServiceState::Running { .. } | ServiceState::Starting { .. })
                );
                if already_running {
                    reply.label = REPLY_OK;
                } else {
                    reply.label = if spawn_service_at(services, idx, spawn_cap).is_ok() {
                        REPLY_OK
                    } else {
                        REPLY_ERR
                    };
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::Stop => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                reply.label = if stop_service(services, idx, spawn_cap, false).is_ok() {
                    REPLY_OK
                } else {
                    REPLY_ERR
                };
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::Restart => {
            let unit_name = extract_unit_name(msg);
            if let Some(idx) = services.find_by_name(&unit_name) {
                reply.label = if stop_service(services, idx, spawn_cap, true).is_ok() {
                    REPLY_OK
                } else {
                    REPLY_ERR
                };
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
                if services
                    .get(idx)
                    .map(|entry| entry.enabled)
                    .unwrap_or(false)
                {
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

        SunlightdOp::NotifyReady => {
            let pid = msg.badge as u32;
            if let Some(idx) = find_service_idx_by_pid(services, pid) {
                let next = services.get(idx).and_then(|entry| match entry.state {
                    ServiceState::Starting {
                        pid, started_at, ..
                    } => Some((pid, started_at)),
                    ServiceState::Running { pid, started_at } => Some((pid, started_at)),
                    _ => None,
                });
                if let Some((pid, started_at)) = next {
                    if let Some(entry) = services.get_mut(idx) {
                        entry.mark_running(pid, started_at);
                    }
                    reply.label = REPLY_OK;
                } else {
                    reply.label = REPLY_ERR;
                }
            } else {
                reply.label = REPLY_ERR;
            }
        }

        SunlightdOp::NotifyFailed => {
            let pid = msg.badge as u32;
            if let Some(idx) = find_service_idx_by_pid(services, pid) {
                let exit_code = msg.words[0] as i32;
                let detail = (msg.words[1] as u32).max(FAILURE_STARTUP);
                if let Some(entry) = services.get_mut(idx) {
                    entry.last_status_detail = detail;
                    entry.mark_failed(exit_code, monotonic_millis());
                }
                reply.label = REPLY_OK;
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
                            detail: entry.last_status_detail,
                        },
                        ServiceState::Starting {
                            pid, started_at, ..
                        } => StatusReply {
                            state: 1,
                            pid,
                            restarts: entry.restart_count,
                            started_at,
                            enabled: entry.enabled,
                            detail: entry.last_status_detail,
                        },
                        ServiceState::Running { pid, started_at } => StatusReply {
                            state: 2,
                            pid,
                            restarts: entry.restart_count,
                            started_at,
                            enabled: entry.enabled,
                            detail: entry.last_status_detail,
                        },
                        ServiceState::Failed {
                            exit_code,
                            crashed_at,
                            restarts,
                        } => StatusReply {
                            state: 3,
                            pid: 0,
                            restarts,
                            started_at: crashed_at,
                            enabled: entry.enabled,
                            detail: if entry.last_status_detail == 0 {
                                exit_code as u32
                            } else {
                                entry.last_status_detail
                            },
                        },
                        ServiceState::Restarting { at } => StatusReply {
                            state: 4,
                            pid: 0,
                            restarts: entry.restart_count,
                            started_at: at,
                            enabled: entry.enabled,
                            detail: entry.last_status_detail,
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
                            ServiceState::Starting { .. } => 1,
                            ServiceState::Failed { .. } => 3,
                            ServiceState::Restarting { .. } => 4,
                            _ => 0,
                        },
                        pid: match entry.state {
                            ServiceState::Running { pid, .. }
                            | ServiceState::Starting { pid, .. } => pid,
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
        poll_service_exits(&mut services, spawn_cap);
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
    fn tls_service_can_resolve_secure_random_provider() {
        let services = load_for_test();
        let idx = services
            .find_by_name("sunlight-tls")
            .expect("sunlight-tls service");
        let mask = services.get(idx).unwrap().unit.capability_mask;

        assert!(mask & sunlight_ipc::ServiceCapability::SecureRandom.bit() != 0);
        assert!(sunlight_ipc::service_capability_allows_hashed_name(
            mask,
            sunlight_ipc::name_to_u64("rand")
        ));
    }

    #[test]
    fn persisted_state_overrides_defaults() {
        let mut services = load_for_test();
        apply_enabled_state_from_str(
            &mut services,
            "v1\nsolar.service=1\nsunlight-tls.service=0\n",
        )
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
