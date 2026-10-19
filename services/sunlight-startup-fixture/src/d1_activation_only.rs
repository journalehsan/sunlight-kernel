//! Lifecycle authority alone must not grant synthetic service invocation.
#![no_std]
#![no_main]
mod d1_log;
use sunlight_ipc::{self as ipc, demand as d, IpcMsg};
#[no_mangle]
pub extern "C" fn _start() -> ! {
    let ep = ipc::endpoint_create();
    assert!(ipc::nameserver_register("d1-activate-only", ep));
    let _ = ipc::ipc_recv(ep);
    let manager = ipc::nameserver_lookup_timeout("sunlightd", 500).unwrap();
    let reply = ipc::ipc_call_timeout(
        manager,
        d::request(d::ACTIVATE, "d1-fixture").unwrap(),
        2_000,
    )
    .unwrap();
    let allowed =
        reply.label == d::OK && ipc::nameserver_lookup_timeout("d1-fixture", 500).is_none();
    let after_denial = ipc::ipc_call_timeout(manager,
        d::request(d::DISCOVER, "d1-fixture").unwrap(), 500).unwrap();
    let allowed = allowed && after_denial.label == d::OK;
    ipc::ipc_reply(IpcMsg::with_label(d::OK).word(0, allowed as u64));
    sunlight_libc::exit(if allowed { 0 } else { 99 })
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    d1_log::log(format_args!("[D1-TEST] activation-only FAIL {}", info));
    sunlight_libc::exit(99)
}
