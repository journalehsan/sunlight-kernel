#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call, ipc_recv, ipc_reply_and_wait, CapabilityToken, InitMsg,
    IpcMsg, SpawnMsg,
};

/// Base servers that init launches via the kernel spawn capability once it
/// holds the spawn token. These need no privileged memory setup (unlike
/// vfs_server/tty_server, which the kernel spawns directly). sunlightd in turn
/// launches the user-level daemons (timezone_service, niced, gcd).
/// deviced starts before drivers so registration normally succeeds. Drivers
/// still treat deviced as optional and continue if it is unavailable.
/// sunlight-kbd and sunlight-mouse are spawned AFTER timer_server (so IPC is stable)
/// but BEFORE tty_server (which depends on input routing).
const INIT_SERVICES: [&str; 8] = [
    "/sbin/timer_server",
    "/sbin/deviced",
    "/sbin/networkd",
    "/sbin/resolved",
    "/sbin/sunlight-kbd",
    "/sbin/sunlight-mouse",
    "/sbin/net_server",
    "/sbin/sunlightd",
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

    // Register the kernel spawn endpoint if token was passed.
    if spawn_token != 0 {
        let name = name_to_u64("spawn");
        registry_insert(&mut registry, name, CapabilityToken(spawn_token));
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
            InitMsg::REGISTER => {
                registry_insert(&mut registry, msg.words[0], CapabilityToken(msg.words[1]));
                IpcMsg::with_label(InitMsg::GRANT)
            }
            InitMsg::LOOKUP => match registry_find(&registry, msg.words[0]) {
                Some(cap) => IpcMsg::with_label(InitMsg::GRANT).word(0, cap.0),
                None => IpcMsg::with_label(InitMsg::DENY),
            },
            _ => IpcMsg::with_label(InitMsg::DENY),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}

#[derive(Clone, Copy)]
struct RegistryEntry {
    name: u64,
    cap: CapabilityToken,
}

impl RegistryEntry {
    const fn empty() -> Self {
        Self {
            name: 0,
            cap: CapabilityToken::INVALID,
        }
    }
}

fn registry_insert(registry: &mut [RegistryEntry; 32], name: u64, cap: CapabilityToken) {
    for entry in registry.iter_mut() {
        if entry.name == name || entry.name == 0 {
            entry.name = name;
            entry.cap = cap;
            return;
        }
    }
}

fn registry_find(registry: &[RegistryEntry; 32], name: u64) -> Option<CapabilityToken> {
    registry
        .iter()
        .find(|entry| entry.name == name && entry.cap != CapabilityToken::INVALID)
        .map(|entry| entry.cap)
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
