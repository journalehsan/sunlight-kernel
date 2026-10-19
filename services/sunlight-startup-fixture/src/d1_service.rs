//! Synthetic initialized service with deliberately delayed, verified readiness.
use crate::d1_log;
use sunlight_ipc::{self as ipc, demand as d, IpcMsg};
const NAME: &str = env!("CARGO_BIN_NAME");

#[no_mangle]
pub extern "C" fn _start() -> ! {
    let sleep_ep = ipc::endpoint_create();
    if NAME == "d1-crash" {
        sunlight_libc::exit(42);
    }
    if NAME == "d1-timeout" {
        loop {
            let _ = ipc::ipc_recv_timeout(sleep_ep, 10_000);
        }
    }
    let manager = ipc::nameserver_lookup_timeout("sunlightd", 1_000).expect("manager");
    let hello = ipc::ipc_call_timeout(manager, IpcMsg::with_label(d::HELLO), 1_000).expect("hello");
    assert_eq!(hello.label, d::OK);
    let generation = hello.words[0];
    // Keeps several real clients concurrently blocked on a cold start even UP.
    let _ = ipc::ipc_recv_timeout(sleep_ep, 250);
    let ep = ipc::endpoint_create();
    let premature = ipc::ipc_call_timeout(
        manager,
        IpcMsg::with_label(d::READY).word(0, generation),
        1_000,
    )
    .expect("premature");
    assert_eq!(premature.label, d::ERROR);
    assert!(ipc::nameserver_register(NAME, ep));
    // An earlier activation generation cannot satisfy this attempt.
    let stale = ipc::ipc_call_timeout(
        manager,
        IpcMsg::with_label(d::READY).word(0, generation.saturating_sub(1)),
        1_000,
    )
    .expect("stale reply");
    assert_eq!(stale.label, d::ERROR);
    let ready = ipc::ipc_call_timeout(
        manager,
        IpcMsg::with_label(d::READY).word(0, generation),
        1_000,
    )
    .expect("ready reply");
    assert_eq!(ready.label, d::OK);
    let mut reply = IpcMsg::empty();
    loop {
        if let Some(msg) = ipc::ipc_reply_and_recv_timeout(ep, reply, 10_000) {
            if msg.label == 3 {
                assert!(ipc::endpoint_destroy(ep));
                // Consume the now-stale current reply target before waiting
                // on a different endpoint (the existing kernel IPC contract).
                ipc::ipc_reply(IpcMsg::with_label(d::OK));
                loop {
                    let _ = ipc::ipc_recv_timeout(sleep_ep, 10_000);
                }
            }
            if msg.label == 2 {
                sunlight_libc::exit(23);
            }
            reply = IpcMsg::with_label(d::OK)
                .word(0, generation)
                .word(1, ipc::current_process_generation());
        } else {
            reply = IpcMsg::empty();
        }
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    d1_log::log(format_args!("[D1-TEST] fixture FAIL {}", info));
    sunlight_libc::exit(99)
}
