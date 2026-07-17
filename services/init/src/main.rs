#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait,
    nameserver_endpoint_is_live, nameserver_note_diagnostic, CapabilityToken, InitMsg, InitStatus,
    IpcMsg, NameserverDiagnosticEvent, SpawnMsg,
};

/// Base servers that init launches via the kernel spawn capability once it
/// holds the spawn token. These need no privileged memory setup (unlike
/// vfs_server/tty_server, which the kernel spawns directly). sunlightd in turn
/// launches the user-level daemons (timezone_service, niced, gcd).
///
/// Spawn returns immediately, so init effectively queues these services and
/// then services their nameserver traffic. Keep `sunlightd` early enough that
/// its own daemon tree can start overlapping the rest of init's boot work.
/// deviced starts before drivers so registration normally succeeds. Drivers
/// still treat deviced as optional and continue if it is unavailable.
/// sunlight-kbd and sunlight-mouse are spawned AFTER timer_server (so IPC is stable)
/// but BEFORE tty_server (which depends on input routing).
const INIT_SERVICES: [&str; 11] = [
    "/sbin/timer_server",
    "/sbin/deviced",
    "/sbin/sunlightd",
    "/sbin/networkd",
    "/sbin/resolved",
    "/sbin/powerd",
    "/sbin/sunlight-kbd",
    "/sbin/sunlight-mouse",
    "/sbin/sunlight-display",
    // PTY broker used by sunlight-libc::openpty() for tty/session spawning.
    "/sbin/pty_server",
    "/sbin/net_server",
];

/// Spawn a service by absolute path using the kernel spawn capability.
fn spawn_service(spawn_cap: CapabilityToken, path: &str) -> bool {
    let mut msg = IpcMsg::with_label(SpawnMsg::SPAWN);
    // Pack the path into the first 4 words (32 bytes max), little-endian.
    let path_bytes = path.as_bytes();
    let mut i = 0;
    while i < 4 {
        let mut word: u64 = 0;
        let mut j = 0;
        while j < 8 {
            let idx = i * 8 + j;
            if idx < path_bytes.len() {
                word |= (path_bytes[idx] as u64) << (j * 8);
            }
            j += 1;
        }
        msg.words[i] = word;
        i += 1;
    }
    let reply = ipc_call(spawn_cap, msg);
    reply.label == SpawnMsg::REPLY
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start(spawn_token: u64) -> ! {
    debug_log("[ init] SunlightOS init process started");
    debug_log("[ init] Waiting for system services to register...");

    let ep = endpoint_create();
    debug_log("[init] Name server: listening");
    if name_to_u64("sunlight-kv") != name_to_u64("sunlight-tls") {
        debug_log("[init] nameserver long-name keys OK");
    } else {
        debug_log("[init] ERROR: nameserver long-name key collision");
    }

    let mut registry = [RegistryEntry::empty(); 32];
    run_registry_self_tests();

    // Register the kernel spawn endpoint if token was passed.
    if spawn_token != 0 {
        let name = name_to_u64("spawn");
        let _ = registry_insert_kernel(&mut registry, name, CapabilityToken(spawn_token));
        debug_log("[init] Registered kernel spawn endpoint");

        // Launch the base servers that do not need privileged kernel memory
        // setup. The spawn syscall is handled inline by the kernel and returns
        // immediately, so these are queued before we enter the name-server loop
        // below — their own register/lookup IPC is then serviced by that loop.
        let spawn_cap = CapabilityToken(spawn_token);
        for path in INIT_SERVICES.iter() {
            if spawn_service(spawn_cap, path) {
                debug_log("[init] launched base service");
            } else {
                debug_log("[init] FAILED to launch base service");
            }
        }
    }

    let mut msg = ipc_recv(ep);
    loop {
        let reply = match msg.label {
            InitMsg::REGISTER => register_service(
                &mut registry,
                msg.words[0],
                CapabilityToken(msg.words[1]),
                msg.words[2] as u32,
                msg.badge as usize,
            ),
            InitMsg::LOOKUP => lookup_service(&mut registry, msg.words[0]),
            _ => IpcMsg::with_label(InitMsg::DENY),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct RegistryEntry {
    name: u64,
    public_cap: CapabilityToken,
    endpoint_id: u32,
    owner_pid: usize,
    kernel_entry: bool,
}

impl RegistryEntry {
    const fn empty() -> Self {
        Self {
            name: 0,
            public_cap: CapabilityToken::INVALID,
            endpoint_id: 0,
            owner_pid: 0,
            kernel_entry: false,
        }
    }

    fn is_empty(self) -> bool {
        self.name == 0
    }

    fn is_live(self) -> bool {
        self.kernel_entry || nameserver_endpoint_is_live(self.public_cap, self.endpoint_id)
    }
}

fn registry_insert_kernel(
    registry: &mut [RegistryEntry; 32],
    name: u64,
    cap: CapabilityToken,
) -> bool {
    for entry in registry.iter_mut() {
        if entry.is_empty() {
            *entry = RegistryEntry {
                name,
                public_cap: cap,
                endpoint_id: 0,
                owner_pid: 0,
                kernel_entry: true,
            };
            return true;
        }
    }
    false
}

fn register_service(
    registry: &mut [RegistryEntry; 32],
    name: u64,
    public_cap: CapabilityToken,
    endpoint_id: u32,
    owner_pid: usize,
) -> IpcMsg {
    if name == 0 || public_cap == CapabilityToken::INVALID || owner_pid == 0 {
        return deny(InitStatus::INVALID_REGISTRATION);
    }

    if let Some(index) = registry.iter().position(|entry| entry.name == name) {
        let current = registry[index];
        return register_existing(
            registry,
            index,
            current.is_live(),
            true,
            name,
            public_cap,
            endpoint_id,
            owner_pid,
        );
    }

    register_new(registry, true, name, public_cap, endpoint_id, owner_pid)
}

fn register_new(
    registry: &mut [RegistryEntry; 32],
    emit_diagnostics: bool,
    name: u64,
    public_cap: CapabilityToken,
    endpoint_id: u32,
    owner_pid: usize,
) -> IpcMsg {
    let Some(slot) = registry.iter_mut().find(|entry| entry.is_empty()) else {
        if emit_diagnostics {
            nameserver_note_diagnostic(NameserverDiagnosticEvent::REGISTRY_FULL);
        }
        return deny(InitStatus::REGISTRY_FULL);
    };
    *slot = RegistryEntry {
        name,
        public_cap,
        endpoint_id,
        owner_pid,
        kernel_entry: false,
    };
    grant()
}

fn register_existing(
    registry: &mut [RegistryEntry; 32],
    index: usize,
    current_is_live: bool,
    emit_diagnostics: bool,
    name: u64,
    public_cap: CapabilityToken,
    endpoint_id: u32,
    owner_pid: usize,
) -> IpcMsg {
    let current = registry[index];
    if current.kernel_entry {
        if emit_diagnostics {
            nameserver_note_diagnostic(NameserverDiagnosticEvent::LIVE_CONFLICT);
        }
        return deny(InitStatus::LIVE_NAME_CONFLICT);
    }
    if current_is_live {
        if current.owner_pid == owner_pid
            && current.endpoint_id == endpoint_id
            && current.public_cap == public_cap
        {
            return grant();
        }
        if emit_diagnostics {
            nameserver_note_diagnostic(NameserverDiagnosticEvent::LIVE_CONFLICT);
        }
        return deny(InitStatus::LIVE_NAME_CONFLICT);
    }

    if emit_diagnostics {
        nameserver_note_diagnostic(NameserverDiagnosticEvent::STALE_REMOVAL);
    }
    registry[index] = RegistryEntry {
        name,
        public_cap,
        endpoint_id,
        owner_pid,
        kernel_entry: false,
    };
    if emit_diagnostics {
        nameserver_note_diagnostic(NameserverDiagnosticEvent::DEAD_REPLACEMENT);
    }
    grant()
}

fn lookup_existing(
    registry: &mut [RegistryEntry; 32],
    index: usize,
    entry_is_live: bool,
    emit_diagnostics: bool,
) -> IpcMsg {
    let entry = registry[index];
    if !entry_is_live {
        registry[index] = RegistryEntry::empty();
        if emit_diagnostics {
            nameserver_note_diagnostic(NameserverDiagnosticEvent::STALE_REMOVAL);
            nameserver_note_diagnostic(NameserverDiagnosticEvent::STALE_LOOKUP);
        }
        return deny(InitStatus::NOT_FOUND);
    }
    IpcMsg::with_label(InitMsg::GRANT)
        .word(0, entry.public_cap.0)
        .word(1, InitStatus::OK)
}

fn lookup_service(registry: &mut [RegistryEntry; 32], name: u64) -> IpcMsg {
    let Some(index) = registry.iter().position(|entry| entry.name == name) else {
        return deny(InitStatus::NOT_FOUND);
    };
    let is_live = registry[index].is_live();
    lookup_existing(registry, index, is_live, true)
}

fn run_registry_self_tests() {
    let name = name_to_u64("registry-self-test");
    let old_cap = CapabilityToken(0x1111);
    let new_cap = CapabilityToken(0x2222);
    let mut registry = [RegistryEntry::empty(); 32];
    registry[0] = RegistryEntry {
        name,
        public_cap: old_cap,
        endpoint_id: 10,
        owner_pid: 20,
        kernel_entry: false,
    };

    let idempotent = register_existing(&mut registry, 0, true, false, name, old_cap, 10, 20).label
        == InitMsg::GRANT;
    let conflict = register_existing(&mut registry, 0, true, false, name, new_cap, 11, 21);
    let conflict_preserved = conflict.label == InitMsg::DENY
        && conflict.words[0] == InitStatus::LIVE_NAME_CONFLICT
        && registry[0].public_cap == old_cap;
    let replaced = register_existing(&mut registry, 0, false, false, name, new_cap, 11, 21);
    let dead_replaced = replaced.label == InitMsg::GRANT
        && registry[0].public_cap == new_cap
        && registry[0].endpoint_id == 11
        && registry[0].owner_pid == 21;
    let stale_lookup = lookup_existing(&mut registry, 0, false, false);
    let stale_removed =
        stale_lookup.label == InitMsg::DENY && registry[0] == RegistryEntry::empty();

    let mut full = [RegistryEntry::empty(); 32];
    for (index, entry) in full.iter_mut().enumerate() {
        *entry = RegistryEntry {
            name: (index + 1) as u64,
            public_cap: CapabilityToken((index + 1) as u64),
            endpoint_id: (index + 1) as u32,
            owner_pid: index + 1,
            kernel_entry: true,
        };
    }
    let before = full;
    let full_reply = register_new(&mut full, false, 99, CapabilityToken(99), 99, 99);
    let full_preserved = full_reply.label == InitMsg::DENY
        && full_reply.words[0] == InitStatus::REGISTRY_FULL
        && full == before;

    if idempotent && conflict_preserved && dead_replaced && stale_removed && full_preserved {
        debug_log("[init] nameserver registry integrity tests: OK");
    } else {
        debug_log("[init] nameserver registry integrity tests: UNEXPECTED");
    }
}

fn grant() -> IpcMsg {
    IpcMsg::with_label(InitMsg::GRANT).word(0, InitStatus::OK)
}

fn deny(status: u64) -> IpcMsg {
    IpcMsg::with_label(InitMsg::DENY).word(0, status)
}

fn name_to_u64(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash = 0xcbf29ce484222325u64;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        i += 1;
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}
