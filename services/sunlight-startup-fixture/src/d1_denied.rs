//! Exercises denial using a deliberately passed manager SEND token.
#![no_std]
#![no_main]
use sunlight_ipc::{self as ipc, demand as d, CapabilityToken, IpcMsg};
#[no_mangle]
pub extern "C" fn _start() -> ! {
    assert!(ipc::nameserver_lookup_timeout("sunlightd", 500).is_none());
    let ep = ipc::endpoint_create();
    assert!(ipc::nameserver_register("d1-denied", ep));
    let msg = ipc::ipc_recv(ep);
    let manager = CapabilityToken(msg.words[0]);
    let rejected = matches!(
        ipc::ipc_call_timeout(
            manager,
            d::request(d::ACTIVATE, "d1-fixture").unwrap(),
            1_000
        ),
        Err(ipc::IpcCallError::InvalidCapability)
    );
    let legacy_rejected = matches!(
        ipc::ipc_call_timeout(manager, d::request(1, "d1-fixture").unwrap(), 1_000),
        Err(ipc::IpcCallError::InvalidCapability)
    );
    let lease_rejected = matches!(
        ipc::ipc_call_timeout(
            manager,
            IpcMsg::with_label(d::LEASE_ACQUIRE)
                .word(0, 1)
                .word(1, 1)
                .word(2, 1)
                .word(3, 1000),
            1000
        ),
        Err(ipc::IpcCallError::InvalidCapability)
    );
    let account_rejected = matches!(
        ipc::ipc_call_timeout(
            manager,
            d::request(d::IDLE_ACCOUNT, "d2-fixture").unwrap(),
            1000
        ),
        Err(ipc::IpcCallError::InvalidCapability)
    );
    let rejected = rejected && legacy_rejected && lease_rejected && account_rejected;
    ipc::ipc_reply(IpcMsg::with_label(d::OK).word(0, rejected as u64));
    sunlight_libc::exit(if rejected { 0 } else { 99 })
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    ipc::debug_log("[D1-TEST] denied fixture FAIL\n");
    sunlight_libc::exit(99)
}
