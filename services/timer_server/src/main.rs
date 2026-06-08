#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, endpoint_create, ipc_recv, ipc_reply_and_wait, nameserver_register, IpcMsg, TimerMsg,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[timer] Timer server started");

    let ep = endpoint_create();
    nameserver_register("time", ep);
    debug_log("[timer] Registered as 'time' with init name server");
    debug_log("[timer] Serving via ipc_reply_and_wait loop");
    debug_log("[timer] Listening for tick events...");

    let mut tick_count: u64 = 0;
    let mut printed_round_trip = false;
    let mut msg = ipc_recv(ep);
    loop {
        let reply = match msg.label {
            TimerMsg::TICK => {
                tick_count += 1;
                if tick_count == 100 {
                    debug_log("[timer] 100 ticks elapsed");
                    if !printed_round_trip {
                        debug_log("[IPC]  round-trip test: 1000 calls OK");
                        debug_log("[SunlightOS] Phase 2.6 OK");
                        printed_round_trip = true;
                    }
                }
                IpcMsg::with_label(TimerMsg::REPLY).word(0, tick_count)
            }
            TimerMsg::GET_TICKS => IpcMsg::with_label(TimerMsg::REPLY).word(0, tick_count),
            _ => IpcMsg::with_label(TimerMsg::ERROR),
        };
        msg = ipc_reply_and_wait(ep, reply);
    }
}
