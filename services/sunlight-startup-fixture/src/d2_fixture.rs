//! Isolated cooperative service. No disk writes. All work is completed before
//! replying; a closed barrier explicitly rejects later direct operations.
#![no_std]
#![no_main]
mod d1_log;
use sunlight_ipc::{self as ipc, demand as d, IpcMsg};

fn report(manager: ipc::CapabilityToken, generation: u64, busy: u64, dirty: u64) {
    // Shorter than the manager's shutdown budget. If a quiesce call crosses
    // this report, cancellation lets the primary loop receive that request.
    for _ in 0..3 {
        if ipc::ipc_call_timeout(
            manager,
            IpcMsg::with_label(d::IDLE_REPORT)
                .word(0, generation)
                .word(1, busy)
                .word(2, dirty),
            100,
        )
        .is_ok()
        {
            return;
        }
    }
    // A lost declaration is conservative: existing clients can still work,
    // and the next completed operation can report idle again. No idle poller.
}
#[no_mangle]
pub extern "C" fn _start() -> ! {
    use sunlight_libc::mman::*;
    let private = mmap(
        core::ptr::null_mut(),
        1024 * 1024,
        PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS,
        -1,
        0,
    )
    .unwrap();
    for offset in (0..1024 * 1024).step_by(4096) {
        unsafe {
            private.add(offset).write_volatile(0x5a);
        }
    }
    let (shared, token) = ipc::shm_alloc().unwrap();
    unsafe {
        shared.write_volatile(0xa5);
    }
    let sleep_ep = ipc::endpoint_create();
    let manager = ipc::nameserver_lookup_timeout("sunlightd", 1000).unwrap();
    let hello = ipc::ipc_call_timeout(manager, IpcMsg::with_label(d::HELLO), 1000).unwrap();
    assert_eq!(hello.label, d::OK);
    let (generation, manager_pid) = (hello.words[0], hello.words[1]);
    assert!(manager_pid != 0);
    let ep = ipc::endpoint_create();
    let name = env!("CARGO_BIN_NAME");
    if name != "d2-fixture" { let _ = ipc::ipc_recv_timeout(sleep_ep, 120); }
    assert!(ipc::nameserver_register(name, ep));
    let readiness = match ipc::ipc_call_timeout(
        manager, IpcMsg::with_label(d::READY).word(0, generation), 1000,
    ) {
        Ok(reply) => reply,
        #[cfg(feature = "recovery-test")]
        Err(_) => {
            d1_log::log(format_args!("[D36R-TEST] manager loss before readiness: explicit fixture exit"));
            sunlight_libc::exit(42);
        }
        #[cfg(not(feature = "recovery-test"))]
        Err(error) => panic!("readiness transport: {:?}", error),
    };
    if name != "d2-fixture" && readiness.label == d::ERROR {
        // The dependency may die during local initialization. Failed readiness
        // is an expected bounded startup failure; never open admission.
        assert!(matches!(readiness.words[0], d::STALE | d::DEPENDENCY));
        sunlight_libc::exit(42);
    }
    assert_eq!(readiness.label, d::OK);
    let (mut busy, mut dirty, mut closed, mut nonce, mut mode, mut completed) =
        (0, 0, false, 0, 0, 0);
    report(manager, generation, busy, dirty);
    loop {
        let msg = ipc::ipc_recv(ep);
        let mut exit = false;
        let mut deferred = None;
        let mut should_report = false;
        let reply = if matches!(msg.label, d::QUIESCE | d::COMMIT_EXIT) {
            if msg.badge != manager_pid || msg.word_count != 2 || msg.words[0] != generation {
                d::error(d::STALE)
            } else if msg.label == d::QUIESCE {
                if busy != 0 || dirty != 0 {
                    d::error(d::BUSY)
                } else if mode == 2 {
                    deferred = Some(ipc::ipc_defer_reply().unwrap());
                    let _ = ipc::ipc_recv_timeout(sleep_ep, 1500);
                    d::error(d::BUSY)
                } else {
                    closed = true;
                    nonce = msg.words[1];
                    IpcMsg::with_label(d::DRAINED)
                        .word(0, generation)
                        .word(
                            1,
                            if mode == 1 {
                                nonce.saturating_sub(1)
                            } else {
                                nonce
                            },
                        )
                        .word(2, 0)
                        .word(3, 0)
                }
            } else if !closed || nonce != msg.words[1] || busy != 0 || dirty != 0 {
                d::error(d::STALE)
            } else {
                exit = mode != 4;
                IpcMsg::with_label(d::OK).word(0, generation).word(1, nonce)
            }
        } else if msg.label == 5 {
            // Recovery/crash injection is available only in this synthetic
            // process. It must never discard simulated persistent state.
            assert_eq!(dirty, 0);
            exit = true;
            IpcMsg::with_label(d::OK)
        } else if closed {
            d::error(d::STOPPED)
        } else {
            match msg.label {
                1 => IpcMsg::with_label(d::OK)
                    .word(0, generation)
                    .word(1, ipc::current_process_generation())
                    .word(2, completed),
                2 => {
                    deferred = Some(ipc::ipc_defer_reply().unwrap());
                    let _ = ipc::ipc_recv_timeout(sleep_ep, msg.words[0].clamp(1, 300));
                    completed += 1;
                    should_report = true;
                    IpcMsg::with_label(d::OK).word(0, completed)
                }
                3 => {
                    busy = msg.words[0];
                    dirty = msg.words[1];
                    should_report = true;
                    IpcMsg::with_label(d::OK)
                }
                4 => IpcMsg::with_label(d::OK).with_cap(0, token),
                6 => {
                    mode = msg.words[0];
                    IpcMsg::with_label(d::OK)
                }
                _ => d::error(d::NOT_FOUND),
            }
        };
        if let Some(token) = deferred {
            // A caller can time out while accepted work drains. Its expired
            // completion token must not crash the service or leak work state.
            let _ = ipc::ipc_complete_deferred_reply(token, reply);
        } else {
            ipc::ipc_reply(reply);
        }
        if exit {
            if closed {
                let _ = ipc::ipc_recv_timeout(sleep_ep, 200);
            }
            sunlight_libc::exit(0);
        }
        if should_report {
            report(manager, generation, busy, dirty);
        }
    }
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    d1_log::log(format_args!("[D2-TEST] fixture FAIL {}", info));
    sunlight_libc::exit(99)
}
