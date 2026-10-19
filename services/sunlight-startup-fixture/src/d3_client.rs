//! Ordinary UID 1000, activation-only profile and a deliberately passed manager
//! token. Neither PID claims nor possession of a token grant management rights.
#![no_std]
#![no_main]
use sunlight_ipc::{self as ipc, demand as d, CapabilityToken, IpcMsg};
extern "C" fn activating(arg: *mut u8) -> *mut u8 {
    let _ = ipc::ipc_call_timeout(
        CapabilityToken(arg as u64),
        d::request(d::ACTIVATE, "d3-parent").unwrap(),
        8000,
    );
    core::ptr::null_mut()
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    assert_eq!(sunlight_libc::getuid(), 1000);
    assert!(ipc::nameserver_lookup_timeout("sunlightd", 100).is_none());
    let ep = ipc::endpoint_create();
    assert!(ipc::nameserver_register("d3-client", ep));
    let request = ipc::ipc_recv(ep);
    let manager = CapabilityToken(request.words[0]);
    if request.label == 2 {
        let _ = ipc::ipc_defer_reply().unwrap();
        unsafe {
            sunlight_libc::thread::spawn(activating, manager.0 as *mut u8).unwrap();
        }
        let delay = ipc::endpoint_create();
        let _ = ipc::ipc_recv_timeout(delay, 80);
        // Existing native std ExitGroup ABI: kill the blocked caller thread
        // too, so this actually cancels the manager's deferred waiter.
        unsafe {
            sunlight_libc::sys::syscall1(157, 0);
        }
        sunlight_libc::exit(99);
    }
    for name in ["d3-leaf", "d2-fixture", "vfs", "sunlightd"] {
        assert!(matches!(
            ipc::ipc_call_timeout(manager, d::request(d::ACTIVATE, name).unwrap(), 500),
            Err(ipc::IpcCallError::InvalidCapability)
        ));
    }
    assert!(matches!(
        ipc::ipc_call_timeout(manager, d::request(d::DISCOVER, "d3-parent").unwrap(), 500),
        Err(ipc::IpcCallError::InvalidCapability)
    ));
    for op in [1, 2, 3, d::LEASE_ACQUIRE, d::IDLE_ACCOUNT] {
        assert!(matches!(
            ipc::ipc_call_timeout(manager, d::request(op, "d3-parent").unwrap(), 500),
            Err(ipc::IpcCallError::InvalidCapability)
        ));
    }
    // Kernel badge wins over any forged readiness identity in the payload.
    assert_eq!(
        ipc::ipc_call_timeout(manager, IpcMsg::with_label(d::READY).word(0, 1), 500)
            .unwrap()
            .label,
        d::ERROR
    );
    let r = ipc::ipc_call_timeout(manager, d::request(d::ACTIVATE, "d3-parent").unwrap(), 8000)
        .unwrap();
    assert_eq!(r.label, d::OK);
    assert!(ipc::nameserver_lookup_timeout("d3-parent", 100).is_none());
    assert!(ipc::nameserver_lookup_timeout("d3-leaf", 100).is_none());
    ipc::ipc_reply(IpcMsg::with_label(d::OK).word(0, r.words[0]));
    sunlight_libc::exit(0)
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    ipc::debug_log("[D3-TEST] ordinary client FAIL\n");
    sunlight_libc::exit(99)
}
