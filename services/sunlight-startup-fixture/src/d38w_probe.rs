//! Isolated D3.8-W receive/request checks; never launched in ordinary boots.
#![no_std]
#![no_main]
mod d1_log;

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use sunlight_ipc::{self as ipc, CapabilityToken, IpcError, IpcMsg, SessionMsg, SESSION_ENDPOINT};

static DIALOG: AtomicU64 = AtomicU64::new(0);
static DONE: AtomicUsize = AtomicUsize::new(0);

fn pause(ms: u64) {
    let endpoint = ipc::endpoint_create();
    assert_eq!(
        ipc::ipc_recv_timeout_result(endpoint, ms).err(),
        Some(IpcError::DeadlineExpired)
    );
    assert!(ipc::endpoint_destroy(endpoint));
}

extern "C" fn dialog_client(_: *mut u8) -> *mut u8 {
    let cap = CapabilityToken(DIALOG.load(Ordering::Acquire));
    // SHOW_DIALOG without a shared request page must reply with a protocol
    // error. Exercise the actual server and its return to blocking receive.
    for _ in 0..8 {
        let reply =
            ipc::ipc_call_timeout(cap, IpcMsg::with_label(0xD201), 1000).expect("dialog reply");
        assert_eq!(reply.label, 0xD2FE);
    }
    DONE.fetch_add(1, Ordering::Release);
    core::ptr::null_mut()
}

fn stop_start(name: &str, endpoint_name: &str, old: CapabilityToken) {
    let manager = ipc::nameserver_lookup_timeout("sunlightd", 2000).expect("manager");
    let mut request = IpcMsg::with_label(2);
    for (i, byte) in name.bytes().enumerate() {
        request.words[i / 8] |= (byte as u64) << (8 * (i % 8));
    }
    request.word_count = 4;
    assert_eq!(
        ipc::ipc_call_timeout(manager, request, 5000)
            .expect("stop")
            .label,
        1
    );
    assert!(ipc::ipc_call_timeout(old, IpcMsg::empty(), 1000).is_err());
    request.label = 1;
    assert_eq!(
        ipc::ipc_call_timeout(manager, request, 5000)
            .expect("start")
            .label,
        1
    );
    // Lookup's timeout bounds one RPC; it does not wait for registration.
    // A Start reply confirms spawn, so explicitly wait for endpoint readiness.
    let until = ipc::monotonic_millis().saturating_add(5000);
    let fresh = loop {
        if let Some(cap) = ipc::nameserver_lookup_timeout(endpoint_name, 100) {
            break cap;
        }
        assert!(ipc::monotonic_millis() < until, "new endpoint readiness");
        pause(10);
    };
    assert_ne!(old, fresh);
    let reply = ipc::ipc_call_timeout(fresh, IpcMsg::empty(), 1000).expect("fresh request");
    assert!(reply.label != 0);
    d1_log::log(format_args!(
        "[D38W] {} blocked stop, stale endpoint, owner restart, next request PASS\n",
        name
    ));
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    pause(15_000);
    let ep = ipc::endpoint_create();
    assert!(ipc::endpoint_destroy(ep));
    assert!(matches!(
        ipc::ipc_recv_result(ep),
        Err(IpcError::InvalidCapability | IpcError::EndpointNotFound)
    ));
    d1_log::log(format_args!(
        "[D38W] checked timeout and closed endpoint PASS\n"
    ));

    let session = ipc::nameserver_lookup_timeout(SESSION_ENDPOINT, 2000).expect("sessiond");
    let mut latencies = [0u64; 32];
    for elapsed in &mut latencies {
        pause(100); // Give the service time to enter its idle receive again.
        let start = ipc::monotonic_millis();
        let reply = ipc::ipc_call_timeout(
            session,
            IpcMsg::with_label(SessionMsg::SESSION_GET_HEALTH),
            1000,
        )
        .expect("session reply");
        *elapsed = ipc::monotonic_millis().saturating_sub(start);
        assert_eq!(reply.label, SessionMsg::REPLY);
    }
    latencies.sort_unstable();
    d1_log::log(format_args!(
        "[D38W] session requests=32 clock_resolution_ms=10 p50_ms={} p95_ms={} p99_ms={} PASS\n",
        latencies[15], latencies[30], latencies[31]
    ));
    let clip = ipc::nameserver_lookup_timeout("clipd", 2000).expect("clipd");
    for _ in 0..8 {
        let reply = ipc::ipc_call_timeout(
            clip,
            IpcMsg::with_label(ipc::ClipMsg::GET_CLIPBOARD_SUMMARY),
            1000,
        )
        .expect("clip reply");
        assert_eq!(reply.label, ipc::ClipMsg::REPLY);
    }
    d1_log::log(format_args!(
        "[D38W] clipboard repeated summary requests PASS\n"
    ));
    DIALOG.store(
        ipc::nameserver_lookup_timeout("dialogd", 2000)
            .expect("dialogd")
            .0,
        Ordering::Release,
    );
    // SAFETY: neither worker dereferences its null argument; shared data are atomic.
    let a = unsafe { sunlight_libc::thread::spawn(dialog_client, core::ptr::null_mut()) }
        .expect("client a");
    let b = unsafe { sunlight_libc::thread::spawn(dialog_client, core::ptr::null_mut()) }
        .expect("client b");
    for _ in 0..200 {
        if !ipc::process_is_alive(a.tid as u64) && !ipc::process_is_alive(b.tid as u64) {
            break;
        }
        pause(10);
    }
    assert!(!ipc::process_is_alive(a.tid as u64));
    assert!(!ipc::process_is_alive(b.tid as u64));
    assert_eq!(DONE.load(Ordering::Acquire), 2);
    d1_log::log(format_args!(
        "[D38W] dialog concurrent malformed requests PASS\n"
    ));
    pause(1000);
    stop_start("sunlight-sessiond", SESSION_ENDPOINT, session);
    pause(100);
    stop_start(
        "sunlight-dialogd",
        "dialogd",
        CapabilityToken(DIALOG.load(Ordering::Acquire)),
    );
    d1_log::log(format_args!("[D38W] FINAL PASS\n"));
    ipc::ProcessExit::exit(0)
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    d1_log::log(format_args!("[D38W] FAIL {info}\n"));
    ipc::ProcessExit::exit(1)
}
