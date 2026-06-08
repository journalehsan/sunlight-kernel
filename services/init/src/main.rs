#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, CapabilityToken, InitMsg, IpcMsg,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[ init] SunlightOS init process started");
    debug_log("[ init] Waiting for system services to register...");

    let ep = endpoint_create();
    debug_log("[init] Name server: listening");

    let mut registry = [RegistryEntry::empty(); 32];
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
