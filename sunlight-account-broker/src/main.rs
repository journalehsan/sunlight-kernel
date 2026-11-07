//! Native broker: metadata management backed by live-session KV. Third-party
//! token access remains denied; provider configuration is installed trusted code.
#![no_std]
#![no_main]
extern crate alloc;
#[cfg(any(feature = "native-mock", feature = "native-provider-test"))]
mod native_flow;
mod native_net;
mod native_store;
#[cfg(any(feature = "native-mock", feature = "native-provider-test"))]
use alloc::vec::Vec;

use linked_list_allocator::LockedHeap;
use sunlight_account_broker::{
    transport::VerifiedTls, Broker, DenyAccess, Entropy, Error, Provider, Result,
    UnavailableDurableKv,
};
use sunlight_ipc::{
    debug_log, endpoint_create, ipc_call_timeout, ipc_recv_timeout, ipc_reply, monotonic_millis,
    nameserver_lookup, nameserver_register, IpcMsg,
};

const STATUS: u64 = 0xAB00;
const START: u64 = 0xAB01;
const COMPLETE: u64 = 0xAB02;
const REQUEST_ACCESS: u64 = 0xAB03;
const DISCONNECT: u64 = 0xAB04;
const REPLY: u64 = 0xABFF;
const DENIED: u64 = 1;
const CALLBACK_UNAVAILABLE: u64 = 2;
const UNSUPPORTED: u64 = 3;

#[global_allocator]
static ALLOCATOR: LockedHeap = LockedHeap::empty();
static mut HEAP: [u8; 1024 * 1024] = [0; 1024 * 1024];

struct NativeEntropy;
impl Entropy for NativeEntropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<()> {
        if sunlight_libc::getrandom(bytes, 0) == bytes.len() as isize {
            Ok(())
        } else {
            Err(Error::Entropy)
        }
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    debug_log("[ACCOUNT-BROKER] panic");
    sunlight_libc::exit(101)
}

#[no_mangle]
pub unsafe extern "C" fn _start(argc: u64, argv: *const *const u8) -> ! {
    ALLOCATOR
        .lock()
        .init(core::ptr::addr_of_mut!(HEAP) as *mut u8, 1024 * 1024);
    #[cfg(feature = "native-mock")]
    if argc == 4 && !argv.is_null() {
        let args: Vec<&str> = (1..4)
            .filter_map(|i| {
                let p = *argv.add(i);
                if p.is_null() {
                    return None;
                }
                let mut n = 0;
                while n < 32 && *p.add(n) != 0 {
                    n += 1;
                }
                core::str::from_utf8(core::slice::from_raw_parts(p, n)).ok()
            })
            .collect();
        if args.len() == 3 && args[0] == "--mock" {
            sunlight_libc::exit(mock_client(
                args[1].parse().unwrap_or(0),
                args[2].parse().unwrap_or(99),
            ));
        }
    }
    if argc == 2 && !argv.is_null() {
        let arg = *argv.add(1);
        for (flag, action) in [
            (b"--storage-seed\0".as_slice(), 0),
            (b"--storage-check\0".as_slice(), 1),
            (b"--storage-delete\0".as_slice(), 2),
            (b"--storage-absent\0".as_slice(), 3),
        ] {
            if !arg.is_null() && (0..flag.len()).all(|i| *arg.add(i) == flag[i]) {
                sunlight_libc::exit(if storage_probe(action).is_ok() { 0 } else { 1 });
            }
        }
        #[cfg(feature = "native-provider-test")]
        if !arg.is_null()
            && (0..b"--real-provider\0".len()).all(|i| *arg.add(i) == b"--real-provider\0"[i])
        {
            sunlight_libc::exit(provider_client());
        }
        if !arg.is_null()
            && (0..b"--loopback-probe\0".len())
                .all(|index| *arg.add(index) == b"--loopback-probe\0"[index])
        {
            let result = native_net::probe();
            if result.is_ok() {
                debug_log("[ACCOUNT-BROKER] native loopback ephemeral probe PASS");
            }
            sunlight_libc::exit(if result.is_ok() { 0 } else { 1 });
        }
        if !arg.is_null() && (0..8).all(|index| *arg.add(index) == b"--probe\0"[index]) {
            sunlight_libc::exit(probe());
        }
    }
    run()
}

fn probe() -> u64 {
    let Some(cap) = nameserver_lookup("sunlight-account-broker") else {
        return 1;
    };
    for (label, expected) in [
        (STATUS, CALLBACK_UNAVAILABLE),
        (START, CALLBACK_UNAVAILABLE),
        (COMPLETE, DENIED),
        (REQUEST_ACCESS, DENIED),
        (sunlight_ipc::online_accounts::LIST_ACCOUNTS, DENIED),
        (DISCONNECT, DENIED),
    ] {
        let Ok(reply) = ipc_call_timeout(cap, IpcMsg::with_label(label), 2000) else {
            return 2;
        };
        if reply.label != REPLY || reply.words[0] != expected {
            return 3;
        }
    }
    debug_log("[ACCOUNT-BROKER] native IPC probe PASS: provider gated; access denied");
    0
}

fn run() -> ! {
    // Placeholder configuration cannot be used to navigate or exchange. A
    // provider must be installed in trusted code along with a native callback.
    let mut broker = Broker::new(
        Provider {
            id: "unconfigured",
            authorization_endpoint: "https://unconfigured.invalid/authorize",
            token_endpoint: "https://unconfigured.invalid/token",
            client_id: "unconfigured",
            allowed_scopes: &[],
        },
        UnavailableDurableKv,
        NativeEntropy,
        VerifiedTls,
        DenyAccess,
    )
    .unwrap();
    let endpoint = endpoint_create();
    if !nameserver_register("sunlight-account-broker", endpoint) {
        debug_log("[ACCOUNT-BROKER] registration failed");
        sunlight_libc::exit(1);
    }
    debug_log(
        "[ACCOUNT-BROKER] native service ready; production provider unconfigured; live-session KV storage ready",
    );
    #[cfg(any(feature = "native-mock", feature = "native-provider-test"))]
    let mut flows: Vec<native_flow::Flow> = Vec::new();
    #[cfg(any(feature = "native-mock", feature = "native-provider-test"))]
    let mut next_flow = 1u64;
    loop {
        #[cfg(any(feature = "native-mock", feature = "native-provider-test"))]
        for flow in &mut flows {
            flow.tick();
        }
        broker.expire_sessions(monotonic_millis() / 1000);
        let Some(msg) = ipc_recv_timeout(endpoint, 20) else {
            continue;
        };
        // No app-controlled URL, provider, identity, secret, or callback SHM is
        // accepted. In particular Complete is not a public code-injection API.
        use sunlight_ipc::online_accounts as api;
        let caller = sunlight_ipc::account_broker_caller_role(msg.badge);
        if api::allows(caller, msg.badge, msg.label) {
            let caller = caller.unwrap();
            match msg.label {
                api::STATUS => {
                    let mode = if cfg!(feature = "native-mock") {
                        1
                    } else if cfg!(feature = "native-provider-test")
                        && sunlight_account_broker::installed_provider::microsoft().is_ok()
                    {
                        2
                    } else {
                        0
                    };
                    ipc_reply(IpcMsg::with_label(REPLY).word(0, 0).word(1, mode));
                    continue;
                }
                api::START => {
                    #[cfg(any(feature = "native-mock", feature = "native-provider-test"))]
                    {
                        flows.retain(|f| f.status == 0);
                        let result = if flows.len() >= 8 {
                            Err(Error::Capacity)
                        } else {
                            #[cfg(feature = "native-mock")]
                            let result = native_flow::Flow::start(next_flow, msg.badge, 8443, 11);
                            #[cfg(all(
                                not(feature = "native-mock"),
                                feature = "native-provider-test"
                            ))]
                            let result = native_flow::Flow::start_provider(next_flow, msg.badge);
                            result
                        };
                        match result {
                            Ok(mut flow) => {
                                flow.manager = Some(caller);
                                let id = flow.id;
                                flows.push(flow);
                                next_flow += 1;
                                ipc_reply(IpcMsg::with_label(REPLY).word(0, 0).word(1, id));
                            }
                            Err(_) => ipc_reply(IpcMsg::with_label(REPLY).word(0, 6)),
                        }
                        continue;
                    }
                    #[cfg(not(any(feature = "native-mock", feature = "native-provider-test")))]
                    {
                        ipc_reply(IpcMsg::with_label(REPLY).word(0, CALLBACK_UNAVAILABLE));
                        continue;
                    }
                }
                api::LIST_ACCOUNTS | api::DISCONNECT => {
                    let result = manage_accounts(&msg, caller.uid);
                    ipc_reply(IpcMsg::with_label(REPLY).word(
                        0,
                        if result.is_ok() {
                            0
                        } else {
                            api::STORAGE_UNAVAILABLE
                        },
                    ));
                    continue;
                }
                api::REQUEST_ACCESS => {
                    // Eligibility only, never a token or a third-party access grant.
                    ipc_reply(IpcMsg::with_label(REPLY).word(0, 0));
                    continue;
                }
                _ => {}
            }
        }
        #[cfg(feature = "native-mock")]
        if msg.label == 0xAB10 {
            flows.retain(|f| f.status == 0);
            let result = if flows.len() >= 8 {
                Err(Error::Capacity)
            } else {
                native_flow::Flow::start(next_flow, msg.badge, msg.words[0] as u16, msg.words[1])
            };
            match result {
                Ok(flow) => {
                    let id = flow.id;
                    flows.push(flow);
                    next_flow += 1;
                    ipc_reply(IpcMsg::with_label(REPLY).word(0, 0).word(1, id));
                }
                Err(_) => ipc_reply(IpcMsg::with_label(REPLY).word(0, 6)),
            }
            continue;
        }
        #[cfg(feature = "native-provider-test")]
        if msg.label == 0xAB11 {
            flows.retain(|f| f.status == 0);
            let result = if flows.len() >= 8 {
                Err(Error::Capacity)
            } else {
                native_flow::Flow::start_provider(next_flow, msg.badge)
            };
            match result {
                Ok(flow) => {
                    let id = flow.id;
                    flows.push(flow);
                    next_flow += 1;
                    ipc_reply(IpcMsg::with_label(REPLY).word(0, 0).word(1, id));
                }
                Err(_) => ipc_reply(IpcMsg::with_label(REPLY).word(0, 6)),
            }
            continue;
        }
        #[cfg(any(feature = "native-mock", feature = "native-provider-test"))]
        if matches!(msg.label, 0xAB05 | 0xAB06 | 0xAB07 | 0xAB08) {
            if let Some(flow) = flows.iter_mut().find(|f| f.id == msg.words[0]) {
                if msg.label == 0xAB08 && flow.browser == msg.badge {
                    flow.release_url_page();
                    ipc_reply(IpcMsg::with_label(REPLY).word(0, 0));
                    continue;
                }
                if msg.label == 0xAB05 && flow.browser == msg.badge {
                    if let Some(url) = flow.url.take() {
                        if let Ok((ptr, token)) = sunlight_ipc::shm_alloc() {
                            unsafe {
                                core::ptr::copy_nonoverlapping(url.as_ptr(), ptr, url.len());
                            }
                            ipc_reply(
                                IpcMsg::with_label(REPLY)
                                    .word(0, 0)
                                    .word(1, url.len() as u64)
                                    .with_cap(0, token),
                            );
                            flow.url_page = Some(token);
                            continue;
                        }
                    }
                } else if flow.owner == msg.badge
                    && flow.manager.map_or(true, |manager| caller == Some(manager))
                    && matches!(msg.label, 0xAB06 | 0xAB07)
                {
                    if msg.label == 0xAB07 {
                        flow.cancel();
                    }
                    ipc_reply(IpcMsg::with_label(REPLY).word(0, 0).word(1, flow.status));
                    if flow.status != 0 {
                        flow.close_browser();
                    }
                    continue;
                }
            }
            ipc_reply(IpcMsg::with_label(REPLY).word(0, DENIED));
            continue;
        }
        let status = match msg.label {
            STATUS | START => CALLBACK_UNAVAILABLE,
            REQUEST_ACCESS => {
                let result = broker.request_access(msg.badge, "", &[], monotonic_millis() / 1000);
                debug_assert!(matches!(result, Err(Error::Unauthorized)));
                DENIED
            }
            COMPLETE | DISCONNECT | api::LIST_ACCOUNTS => DENIED,
            _ => UNSUPPORTED,
        };
        ipc_reply(IpcMsg::with_label(REPLY).word(0, status));
    }
}

#[cfg(feature = "native-mock")]
fn mock_client(port: u16, mode: u64) -> u64 {
    let Some(cap) = nameserver_lookup("sunlight-account-broker") else {
        return 1;
    };
    let Ok(reply) = ipc_call_timeout(
        cap,
        IpcMsg::with_label(0xAB10)
            .word(0, port as u64)
            .word(1, mode),
        5000,
    ) else {
        return 2;
    };
    if reply.words[0] != 0 {
        return 3;
    }
    let id = reply.words[1];
    let Ok(denied) = ipc_call_timeout(cap, IpcMsg::with_label(0xAB05).word(0, id), 2000) else {
        return 7;
    };
    if denied.words[0] != DENIED || denied.caps[0] != sunlight_ipc::CapabilityToken::INVALID {
        return 8;
    }
    debug_log("[ACCOUNT-BROKER] URL claim rejected for unlaunched caller");
    let start = monotonic_millis();
    let mut cancelled = false;
    while monotonic_millis() - start < 130_000 {
        if mode == 4 && !cancelled && monotonic_millis() - start > 500 {
            let _ = ipc_call_timeout(cap, IpcMsg::with_label(0xAB07).word(0, id), 2000);
            cancelled = true;
        }
        if let Ok(reply) = ipc_call_timeout(cap, IpcMsg::with_label(0xAB06).word(0, id), 30_000) {
            if reply.words[0] != 0 {
                return 4;
            }
            if reply.words[1] != 0 {
                let expected = match mode {
                    0 | 2 => 1,
                    1 => 2,
                    3 | 7 | 9 => 3,
                    4 => 4,
                    5 | 6 => 5,
                    8 => 6,
                    _ => 99,
                };
                return if reply.words[1] == expected {
                    debug_log("[ACCOUNT-BROKER] native mock expected outcome PASS");
                    0
                } else {
                    5
                };
            }
        }
        sunlight_ipc::process_yield();
    }
    6
}

#[cfg(feature = "native-provider-test")]
fn provider_client() -> u64 {
    let Some(cap) = nameserver_lookup("sunlight-account-broker") else {
        return 1;
    };
    let Ok(reply) = ipc_call_timeout(cap, IpcMsg::with_label(0xAB11), 5000) else {
        return 2;
    };
    if reply.words[0] != 0 {
        return 3;
    }
    let until = monotonic_millis() + 130_000;
    while monotonic_millis() < until {
        if let Ok(status) = ipc_call_timeout(
            cap,
            IpcMsg::with_label(0xAB06).word(0, reply.words[1]),
            30_000,
        ) {
            if status.words[0] != 0 {
                return 4;
            }
            if status.words[1] != 0 {
                return if status.words[1] == 1 { 0 } else { 5 };
            }
        }
        sunlight_ipc::process_yield();
    }
    let _ = ipc_call_timeout(
        cap,
        IpcMsg::with_label(0xAB07).word(0, reply.words[1]),
        2000,
    );
    6
}

fn manage_accounts(msg: &IpcMsg, uid: u32) -> Result<()> {
    use sunlight_account_broker::{storage, KvClient};
    use sunlight_ipc::online_accounts as api;
    let ptr = sunlight_ipc::shm_map(msg.caps[0]).map_err(|_| Error::Kv)?;
    let result = (|| {
        if msg.label == api::DISCONNECT {
            let raw = unsafe { core::slice::from_raw_parts(ptr, 48) };
            let handle = core::str::from_utf8(raw).map_err(|_| Error::NotFound)?;
            native_store::disconnect(uid, handle)?;
            debug_log("[ONLINE-ACCOUNTS] disconnected; record and index removed");
        } else {
            let mut kv = native_store::LiveKv(uid);
            let handles = storage::read_index(&mut kv)?;
            let mut snapshot = api::Snapshot::EMPTY;
            for handle in handles {
                let raw = kv.get(&storage::record_key(&handle))?;
                let provider = if raw.contains("\"provider\":\"native-mock\"") {
                    "native-mock"
                } else {
                    "microsoft-consumers"
                };
                let meta =
                    storage::read_metadata(&mut kv, provider, &handle, monotonic_millis() / 1000)?;
                let account = &mut snapshot.accounts[snapshot.count as usize];
                account.id.copy_from_slice(handle.as_bytes());
                account.identifier[..meta.identifier.len()]
                    .copy_from_slice(meta.identifier.as_bytes());
                account.provider[..meta.provider.len()].copy_from_slice(meta.provider.as_bytes());
                account.active = u64::from(meta.status == storage::AccountStatus::Connected);
                snapshot.count += 1;
            }
            unsafe {
                core::ptr::write_unaligned(ptr.cast::<api::Snapshot>(), snapshot);
            }
        }
        Ok(())
    })();
    let _ = sunlight_ipc::shm_free(msg.caps[0]);
    result
}

fn storage_probe(action: u64) -> Result<()> {
    use sunlight_account_broker::KvClient;
    let mut kv = native_store::LiveKv(u32::MAX);
    for n in [1024, 4096] {
        let key = alloc::format!("account-broker:probe-{n}");
        let value = alloc::string::String::from("x").repeat(n);
        match action {
            0 => kv.put(&key, &value)?,
            1 => {
                if *kv.get(&key)? != value {
                    return Err(Error::Kv);
                }
            }
            2 => kv.delete(&key)?,
            _ => {
                if kv.get(&key).err() != Some(Error::NotFound) {
                    return Err(Error::Kv);
                }
            }
        }
    }
    if sunlight_libc::stat(b"/var/lib/sunlight/kv.store")
        .map_or(true, |stat| stat.mode & 0o777 != 0o600)
    {
        return Err(Error::Kv);
    }
    if sunlight_libc::open(b"/var/lib/sunlight-kv/broker.store").is_ok() {
        return Err(Error::Unauthorized);
    }
    debug_log("[ONLINE-ACCOUNTS] native 1K/4K storage probe PASS; shared log mode=0600; backing-file access denied");
    Ok(())
}
